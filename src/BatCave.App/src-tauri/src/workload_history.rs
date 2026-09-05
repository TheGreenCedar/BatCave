//! Runtime-owned inspection history. Publications share this archive; readers copy one selection.
use crate::{
    contracts::{ProcessSample, ProcessViewRow, RuntimeSnapshot},
    protocol::{
        catalog::{CatalogBuilder, QUALITY_CODES},
        encode::encode_workloads,
        types::{
            LimitationEntry, MeasurementDescriptor, MetricQualityV4, MetricSemantic,
            MetricSourceV4, NetworkScopeV4, WorkloadDetailV4,
        },
    },
};
use serde::Serialize;
use std::{
    collections::{HashMap, HashSet, VecDeque},
    mem::size_of,
    sync::{Arc, Mutex},
};

pub const HISTORY_BUDGET_BYTES: usize = 64 * 1024 * 1024;
const MAX_POINTS: usize = 360;
const REPLY_BUDGET_BYTES: usize = 8 * 1024 * 1024;
const MAX_TOMBSTONES: usize = 1_024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum InspectionStatus {
    Current,
    Exited,
    Evicted,
    Unknown,
}

#[derive(Debug, Clone, Serialize)]
pub struct InspectionCatalog {
    pub sample_seq: u64,
    pub sampled_at_ms: u64,
    pub published_at_ms: u64,
    pub descriptors: Vec<MeasurementDescriptor>,
    pub quality_codes: Vec<MetricQualityV4>,
    pub limitations: Vec<LimitationEntry>,
    pub workloads: Vec<WorkloadDetailV4>,
}

#[derive(Debug, Clone, Copy, Serialize)]
pub struct HistoryObservation {
    pub value: Option<f64>,
    pub quality: MetricQualityV4,
    pub source: MetricSourceV4,
    pub network_scope: Option<NetworkScopeV4>,
    pub available: u32,
    pub total: u32,
}

#[derive(Debug, Clone, Copy, Serialize)]
pub struct WorkloadHistoryPoint {
    pub sample_seq: u64,
    pub sampled_at_ms: u64,
    pub interval_ms: u32,
    pub gap_before: bool,
    pub cpu: HistoryObservation,
    pub memory: HistoryObservation,
    pub io: HistoryObservation,
    pub network: HistoryObservation,
}

#[derive(Debug, Serialize)]
pub struct WorkloadInspection {
    pub response_token: String,
    pub inspection_version: u16,
    pub runtime_protocol_version: u16,
    pub stable_id: String,
    pub publication_seq: u64,
    pub sample_seq: u64,
    pub status: InspectionStatus,
    pub catalog: Option<InspectionCatalog>,
    pub retained_points: u16,
    pub history_truncated: bool,
    pub history: Vec<WorkloadHistoryPoint>,
}

struct Entry {
    bundle_id: u64,
    last_sample_seq: u64,
    history: VecDeque<WorkloadHistoryPoint>,
    truncated: bool,
}
struct Bundle {
    catalog: InspectionCatalog,
    references: usize,
    bytes: usize,
}

pub struct WorkloadArchive {
    entries: HashMap<String, Entry>,
    bundles: HashMap<u64, Bundle>,
    evicted: VecDeque<String>,
    budget: usize,
    allocated: usize,
    next_bundle: u64,
    publication_seq: u64,
    sample_seq: u64,
    last_sampled_at_ms: Option<u64>,
    failed_sample: bool,
    ingress_reserved: usize,
    replies: Arc<Mutex<ResponseCredits>>,
}
impl Default for WorkloadArchive {
    fn default() -> Self {
        Self::with_budget(HISTORY_BUDGET_BYTES)
    }
}
impl WorkloadArchive {
    fn with_budget(budget: usize) -> Self {
        Self {
            entries: HashMap::new(),
            bundles: HashMap::new(),
            evicted: VecDeque::new(),
            budget,
            allocated: 0,
            next_bundle: 0,
            publication_seq: 0,
            sample_seq: 0,
            last_sampled_at_ms: None,
            failed_sample: false,
            ingress_reserved: 0,
            replies: Arc::new(Mutex::new(ResponseCredits::default())),
        }
    }
    /// Admit all archive-owned shaping allocations before the caller clones a single process.
    pub fn begin_ingress(&mut self, processes: &[ProcessSample]) -> Result<(), String> {
        let max_label = processes
            .iter()
            .map(|process| process.name.len().saturating_add(process.exe.len()))
            .max()
            .unwrap_or_default();
        // Four simultaneous ProcessSample representations, row boxes, grouping/hash maps,
        // normalization buffers, repeated group labels, and one encoded family. Source strings
        // are counted directly without allocating normalized strings or a JSON buffer.
        let fixed = processes.len().saturating_mul(
            size_of::<ProcessSample>()
                .saturating_mul(4)
                .saturating_add(size_of::<ProcessViewRow>() * 4)
                .saturating_add(4096)
                .saturating_add(max_label.saturating_mul(8)),
        );
        let strings = processes
            .iter()
            .map(process_text_bytes)
            .sum::<usize>()
            .saturating_mul(8);
        let required = fixed
            .saturating_add(strings)
            .saturating_add(self.budget / 8);
        let reply_reserve = REPLY_BUDGET_BYTES.min(self.budget / 8);
        if required > self.budget.saturating_sub(reply_reserve) {
            self.failed_sample = true;
            return Err("workload_inspection_ingress_budget_exceeded".into());
        }
        self.ingress_reserved = required;
        self.enforce_budget();
        Ok(())
    }
    pub fn end_ingress(&mut self) {
        self.ingress_reserved = 0;
    }
    pub fn observe(
        &mut self,
        snapshot: &RuntimeSnapshot,
        rows: &[ProcessViewRow],
        live: bool,
    ) -> Result<(), String> {
        let new_sample =
            live && snapshot.sampled_at_ms.is_some() && snapshot.sample_seq > self.sample_seq;
        if new_sample {
            self.failed_sample = true;
        }
        let result = self.observe_inner(snapshot, rows, live);
        if new_sample && result.is_ok() {
            self.failed_sample = false;
        }
        result
    }
    fn observe_inner(
        &mut self,
        snapshot: &RuntimeSnapshot,
        rows: &[ProcessViewRow],
        live: bool,
    ) -> Result<(), String> {
        self.publication_seq = snapshot.publication_seq;
        let Some(sampled_at_ms) = snapshot.sampled_at_ms else {
            return Ok(());
        };
        if !live || snapshot.sample_seq <= self.sample_seq {
            return Ok(());
        }
        if self
            .last_sampled_at_ms
            .is_some_and(|last| sampled_at_ms < last)
        {
            return Err("workload_inspection_clock_regressed".into());
        }
        self.last_sampled_at_ms = Some(sampled_at_ms);
        self.sample_seq = snapshot.sample_seq;
        // Encode one disjoint family at a time; never materialize a second complete workload catalog.
        let mut members: HashMap<&str, Vec<usize>> = HashMap::new();
        for (index, row) in rows.iter().enumerate() {
            if let ProcessViewRow::Process {
                group_key,
                is_grouped: true,
                ..
            } = row
            {
                members.entry(group_key).or_default().push(index);
            }
        }
        for row in rows {
            if matches!(
                row,
                ProcessViewRow::Process {
                    is_grouped: true,
                    ..
                }
            ) {
                continue;
            }
            let mut catalog = CatalogBuilder::new(snapshot.settings.sample_interval_ms)?;
            let workloads = match row {
                ProcessViewRow::Group { detail, .. } => {
                    let indices = members
                        .get(detail.group_key.as_str())
                        .map(Vec::as_slice)
                        .unwrap_or_default();
                    let bytes =
                        indices
                            .iter()
                            .try_fold(serialized_size_bound(row)?, |bytes, index| {
                                serialized_size_bound(&rows[*index])
                                    .map(|next| bytes.saturating_add(next))
                            })?;
                    if bytes.saturating_mul(4) > self.budget / 8 {
                        let id = row_id(row, snapshot.sample_seq);
                        self.evict_id(&id);
                        self.remember_evicted(id);
                        for index in indices {
                            let id = row_id(&rows[*index], snapshot.sample_seq);
                            self.evict_id(&id);
                            self.remember_evicted(id);
                        }
                        self.enforce_budget();
                        continue;
                    }
                    let mut family = vec![row.clone()];
                    if let Some(indices) = members.get(detail.group_key.as_str()) {
                        family.extend(indices.iter().map(|index| rows[*index].clone()));
                    }
                    encode_workloads(
                        &family,
                        snapshot.sample_seq,
                        Some(sampled_at_ms),
                        snapshot.environment.platform,
                        &mut catalog,
                    )?
                }
                ProcessViewRow::Process {
                    is_grouped: false, ..
                } => {
                    if serialized_size_bound(row)?.saturating_mul(4) > self.budget / 8 {
                        let id = row_id(row, snapshot.sample_seq);
                        self.evict_id(&id);
                        self.remember_evicted(id);
                        self.enforce_budget();
                        continue;
                    }
                    encode_workloads(
                        std::slice::from_ref(row),
                        snapshot.sample_seq,
                        Some(sampled_at_ms),
                        snapshot.environment.platform,
                        &mut catalog,
                    )?
                }
                ProcessViewRow::Process {
                    is_grouped: true, ..
                } => continue,
            };
            self.insert_family(workloads, catalog, snapshot, sampled_at_ms)?;
        }
        self.enforce_budget();
        Ok(())
    }
    fn insert_family(
        &mut self,
        mut workloads: Vec<WorkloadDetailV4>,
        source: CatalogBuilder,
        snapshot: &RuntimeSnapshot,
        sampled_at_ms: u64,
    ) -> Result<(), String> {
        let ids: HashSet<String> = workloads
            .iter()
            .map(|row| stable_id(row).to_string())
            .collect();
        for row in &mut workloads {
            if let WorkloadDetailV4::Process(detail) = row {
                if detail
                    .parent_process_id
                    .as_ref()
                    .is_some_and(|id| !ids.contains(id))
                {
                    detail.parent_process_id = None;
                }
            }
        }
        // The encoder sees exactly one family, so its catalog already contains only that
        // family's descriptors and limitations. Move it without cloning or remapping indices.
        let catalog = InspectionCatalog {
            sample_seq: snapshot.sample_seq,
            sampled_at_ms,
            published_at_ms: snapshot.published_at_ms,
            descriptors: source.descriptors,
            quality_codes: QUALITY_CODES.to_vec(),
            limitations: source.limitations,
            workloads,
        };
        // This deliberately overcounts small values and allocator overhead. Shared family rows are counted once.
        let bytes = serialized_size_bound(&catalog)?
            .max(catalog_heap_bytes(&catalog))
            .saturating_mul(4)
            .saturating_add(size_of::<Bundle>() + 512);
        if bytes > self.budget / 8 {
            for id in ids {
                self.evict_id(&id);
                self.remember_evicted(id);
            }
            self.enforce_budget();
            return Ok(());
        }
        self.next_bundle += 1;
        let bundle_id = self.next_bundle;
        let points: Vec<_> = catalog
            .workloads
            .iter()
            .map(|row| {
                (
                    stable_id(row).to_string(),
                    history_point(row, &catalog, snapshot.settings.sample_interval_ms),
                )
            })
            .collect();
        self.allocated += bytes;
        self.bundles.insert(
            bundle_id,
            Bundle {
                catalog,
                references: points.len(),
                bytes,
            },
        );
        for (id, mut point) in points {
            let mut entry = if let Some(mut previous) = self.entries.remove(&id) {
                self.allocated -= entry_heap(&id, &previous);
                self.release_bundle(previous.bundle_id);
                point.gap_before = previous.history.back().is_none_or(|last| {
                    last.sample_seq + 1 != point.sample_seq
                        || last.interval_ms != point.interval_ms
                        || scope_changed(last, &point)
                        || point.sampled_at_ms <= last.sampled_at_ms
                        || point.sampled_at_ms - last.sampled_at_ms
                            >= u64::from(last.interval_ms) * 2
                });
                previous.bundle_id = bundle_id;
                previous.last_sample_seq = point.sample_seq;
                previous
            } else {
                Entry {
                    bundle_id,
                    last_sample_seq: point.sample_seq,
                    history: VecDeque::new(),
                    truncated: self.evicted.contains(&id),
                }
            };
            if entry.history.len() == MAX_POINTS {
                entry.history.pop_front();
                entry.truncated = true;
            }
            entry.history.push_back(point);
            // VecDeque may reserve beyond 360 while growing. Account its actual allocation.
            self.allocated += entry_heap(&id, &entry);
            self.entries.insert(id.clone(), entry);
            if let Some(index) = self.evicted.iter().position(|old| old == &id) {
                self.evicted.remove(index);
            }
        }
        self.enforce_budget();
        Ok(())
    }
    fn release_bundle(&mut self, id: u64) {
        if let Some(bundle) = self.bundles.get_mut(&id) {
            bundle.references -= 1;
            if bundle.references == 0 {
                if let Some(bundle) = self.bundles.remove(&id) {
                    self.allocated -= bundle.bytes;
                }
            }
        }
    }
    fn evict_id(&mut self, id: &str) {
        if let Some((key, entry)) = self.entries.remove_entry(id) {
            self.allocated -= entry_heap(&key, &entry);
            self.release_bundle(entry.bundle_id);
            self.remember_evicted(key);
        }
    }
    fn remember_evicted(&mut self, id: String) {
        if !self.evicted.contains(&id) {
            self.evicted.push_back(id);
        }
        if self.evicted.len() > MAX_TOMBSTONES {
            self.evicted.pop_front();
        }
    }
    fn retained_bytes(&self) -> usize {
        self.allocated
            + self.entries.capacity() * (size_of::<(String, Entry)>() + 32)
            + self.bundles.capacity() * (size_of::<(u64, Bundle)>() + 32)
            + self.evicted.capacity() * size_of::<String>()
            + self.evicted.iter().map(String::capacity).sum::<usize>()
    }
    fn enforce_budget(&mut self) {
        // Reply admission is shared across all readers; ingress reserves its complete peak before shaping.
        while self.retained_bytes()
            > self
                .budget
                .saturating_sub(REPLY_BUDGET_BYTES.min(self.budget / 8))
                .saturating_sub(self.ingress_reserved)
        {
            let oldest = self
                .entries
                .iter()
                .min_by_key(|(_, entry)| entry.last_sample_seq)
                .map(|(id, _)| id.clone());
            if let Some(id) = oldest {
                self.evict_id(&id);
            } else if self.evicted.pop_front().is_none() {
                break;
            }
            if self.entries.len() * 4 < self.entries.capacity() {
                self.entries.shrink_to_fit();
            }
            if self.bundles.len() * 4 < self.bundles.capacity() {
                self.bundles.shrink_to_fit();
            }
            if self.evicted.len() * 4 < self.evicted.capacity() {
                self.evicted.shrink_to_fit();
            }
        }
    }
    pub fn acknowledge(&self, token: &str) -> Result<(), String> {
        let id = token
            .strip_prefix("inspection:")
            .and_then(|id| id.parse::<u64>().ok())
            .filter(|id| format!("inspection:{id}") == token)
            .ok_or_else(|| "workload_inspection_response_token_invalid".to_string())?;
        let mut replies = self
            .replies
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(bytes) = replies.pending.remove(&id) {
            replies.used -= bytes;
        }
        if replies.pending.len() * 4 < replies.pending.capacity() {
            replies.pending.shrink_to_fit();
        }
        Ok(())
    }
    pub fn wants_sample(&self, sample_seq: u64) -> bool {
        sample_seq > self.sample_seq
    }
    pub fn inspect_published(
        &self,
        id: &str,
        limit: u16,
        publication_seq: u64,
    ) -> Result<WorkloadInspection, String> {
        if self.publication_seq != publication_seq {
            return Err("workload_inspection_publication_pending".into());
        }
        self.inspect(id, limit)
    }
    pub fn inspect(&self, id: &str, point_limit: u16) -> Result<WorkloadInspection, String> {
        if self.failed_sample {
            return Err("workload_inspection_sample_failed".into());
        }
        if ![30, 72, 180, 360].contains(&point_limit) || id.is_empty() || id.len() > 1024 {
            return Err("workload_inspection_request_invalid".into());
        }
        let entry = self.entries.get(id);
        let status = match entry {
            Some(entry) if entry.last_sample_seq == self.sample_seq => InspectionStatus::Current,
            Some(_) => InspectionStatus::Exited,
            None if self.evicted.iter().any(|key| key == id) => InspectionStatus::Evicted,
            None => InspectionStatus::Unknown,
        };
        let catalog_bytes = entry
            .and_then(|entry| self.bundles.get(&entry.bundle_id))
            .map(|bundle| bundle.bytes)
            .unwrap_or_default();
        let reply_bytes = catalog_bytes
            .saturating_add(
                entry
                    .map(|entry| entry.history.len().min(point_limit as usize))
                    .unwrap_or_default()
                    * 4096,
            )
            .saturating_add(id.len() * 4)
            .saturating_add(512);
        let response_token = ResponseCredits::reserve(
            &self.replies,
            reply_bytes,
            REPLY_BUDGET_BYTES.min(self.budget / 8),
        )?;
        let history = entry
            .map(|entry| {
                entry
                    .history
                    .iter()
                    .skip(entry.history.len().saturating_sub(point_limit as usize))
                    .copied()
                    .collect()
            })
            .unwrap_or_default();
        let catalog = entry
            .and_then(|entry| self.bundles.get(&entry.bundle_id))
            .map(|bundle| bundle.catalog.clone());
        Ok(WorkloadInspection {
            response_token,
            inspection_version: 1,
            runtime_protocol_version: crate::protocol::RUNTIME_PROTOCOL_VERSION,
            stable_id: id.into(),
            publication_seq: self.publication_seq,
            sample_seq: self.sample_seq,
            status,
            catalog,
            retained_points: entry
                .map(|entry| entry.history.len() as u16)
                .unwrap_or_default(),
            history_truncated: entry
                .is_some_and(|entry| entry.truncated || entry.history.len() > point_limit as usize),
            history,
        })
    }
}
#[derive(Debug, Default)]
struct ResponseCredits {
    used: usize,
    next_id: u64,
    pending: HashMap<u64, usize>,
}
impl ResponseCredits {
    fn reserve(pool: &Arc<Mutex<Self>>, bytes: usize, budget: usize) -> Result<String, String> {
        let mut pool = pool
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let next = pool
            .used
            .checked_add(bytes)
            .filter(|next| *next <= budget)
            .ok_or_else(|| "workload_inspection_reply_budget_exceeded".to_string())?;
        pool.next_id = pool
            .next_id
            .checked_add(1)
            .ok_or_else(|| "workload_inspection_response_tokens_exhausted".to_string())?;
        let id = pool.next_id;
        pool.pending.insert(id, bytes);
        pool.used = next;
        Ok(format!("inspection:{id}"))
    }
}

fn process_text_bytes(process: &ProcessSample) -> usize {
    let messages = process
        .quality
        .as_ref()
        .map(|quality| {
            [
                &quality.cpu,
                &quality.memory,
                &quality.io,
                &quality.other_io,
                &quality.network,
                &quality.threads,
                &quality.handles,
            ]
            .into_iter()
            .flatten()
            .map(|quality| {
                quality
                    .message
                    .as_ref()
                    .map(String::len)
                    .unwrap_or_default()
            })
            .sum::<usize>()
        })
        .unwrap_or_default();
    process
        .pid
        .len()
        .saturating_add(
            process
                .parent_pid
                .as_ref()
                .map(String::len)
                .unwrap_or_default(),
        )
        .saturating_add(process.name.len())
        .saturating_add(process.exe.len())
        .saturating_add(process.status.len())
        .saturating_add(messages)
}
fn row_id(row: &ProcessViewRow, sample_seq: u64) -> String {
    match row {
        ProcessViewRow::Group { detail, .. } => detail.workload_id.clone(),
        ProcessViewRow::Process { detail, .. } => {
            if detail.process.start_time_ms == 0 {
                format!("process:{}:publication:{sample_seq}", detail.process.pid)
            } else {
                format!(
                    "process:{}:{}",
                    detail.process.pid, detail.process.start_time_ms
                )
            }
        }
    }
}
fn serialized_size_bound(value: &impl Serialize) -> Result<usize, String> {
    value
        .serialize(size_bound::JsonSize)
        .map_err(|error| error.to_string())
}

fn catalog_heap_bytes(catalog: &InspectionCatalog) -> usize {
    fn vector<T>(values: &Vec<T>) -> usize {
        values.capacity().saturating_mul(size_of::<T>())
    }
    fn optional(value: &Option<String>) -> usize {
        value.as_ref().map(String::capacity).unwrap_or_default()
    }
    let mut bytes = size_of::<InspectionCatalog>()
        + vector(&catalog.descriptors)
        + vector(&catalog.quality_codes)
        + vector(&catalog.limitations)
        + vector(&catalog.workloads);
    bytes += catalog
        .limitations
        .iter()
        .map(|entry| entry.message.capacity())
        .sum::<usize>();
    for row in &catalog.workloads {
        bytes += match row {
            WorkloadDetailV4::Process(detail) => {
                let presentation = &detail.presentation;
                [
                    &detail.stable_id,
                    &detail.pid,
                    &detail.display_name,
                    &detail.executable,
                    &detail.status,
                    &presentation.group_key,
                    &presentation.group_label,
                    &presentation.group_category,
                    &presentation.icon_kind,
                ]
                .into_iter()
                .map(String::capacity)
                .sum::<usize>()
                    + optional(&detail.parent_pid)
                    + optional(&detail.parent_process_id)
                    + optional(&presentation.group_id)
                    + vector(&detail.metrics)
            }
            WorkloadDetailV4::Group(detail) => {
                [
                    &detail.stable_id,
                    &detail.group_key,
                    &detail.label,
                    &detail.category,
                    &detail.icon_kind,
                ]
                .into_iter()
                .chain(detail.member_ids.iter())
                .map(String::capacity)
                .sum::<usize>()
                    + optional(&detail.icon_source)
                    + optional(&detail.example_label)
                    + vector(&detail.member_ids)
                    + vector(&detail.metrics)
                    + vector(&detail.coverage)
            }
        };
    }
    bytes
}

// Only used for conservative allocation admission, never for the wire protocol. Walking the
// derived Serialize shape covers future fields without formatting numbers or building JSON.
mod size_bound {
    use serde::{ser, Serialize};

    pub(super) struct JsonSize;
    pub(super) struct Container(usize);
    type Result<T> = std::result::Result<T, serde_json::Error>;

    fn string(value: &str) -> usize {
        value.bytes().fold(2usize, |bytes, byte| {
            bytes.saturating_add(match byte {
                0..=0x1f => 6,
                b'"' | b'\\' => 2,
                _ => 1,
            })
        })
    }
    macro_rules! primitive {
        ($name:ident, $kind:ty, $bound:expr) => {
            fn $name(self, _: $kind) -> Result<usize> {
                Ok($bound)
            }
        };
    }
    impl ser::Serializer for JsonSize {
        type Ok = usize;
        type Error = serde_json::Error;
        type SerializeSeq = Container;
        type SerializeTuple = Container;
        type SerializeTupleStruct = Container;
        type SerializeTupleVariant = Container;
        type SerializeMap = Container;
        type SerializeStruct = Container;
        type SerializeStructVariant = Container;
        primitive!(serialize_bool, bool, 5);
        primitive!(serialize_i8, i8, 4);
        primitive!(serialize_i16, i16, 6);
        primitive!(serialize_i32, i32, 11);
        primitive!(serialize_i64, i64, 20);
        primitive!(serialize_i128, i128, 40);
        primitive!(serialize_u8, u8, 3);
        primitive!(serialize_u16, u16, 5);
        primitive!(serialize_u32, u32, 10);
        primitive!(serialize_u64, u64, 20);
        primitive!(serialize_u128, u128, 39);
        primitive!(serialize_f32, f32, 24);
        primitive!(serialize_f64, f64, 24);
        primitive!(serialize_char, char, 8);
        fn serialize_str(self, value: &str) -> Result<usize> {
            Ok(string(value))
        }
        fn serialize_bytes(self, value: &[u8]) -> Result<usize> {
            Ok(value.len().saturating_mul(4).saturating_add(2))
        }
        fn serialize_none(self) -> Result<usize> {
            Ok(4)
        }
        fn serialize_some<T: ?Sized + Serialize>(self, value: &T) -> Result<usize> {
            value.serialize(self)
        }
        fn serialize_unit(self) -> Result<usize> {
            Ok(4)
        }
        fn serialize_unit_struct(self, _: &'static str) -> Result<usize> {
            Ok(4)
        }
        fn serialize_unit_variant(
            self,
            _: &'static str,
            _: u32,
            variant: &'static str,
        ) -> Result<usize> {
            Ok(string(variant))
        }
        fn serialize_newtype_struct<T: ?Sized + Serialize>(
            self,
            _: &'static str,
            value: &T,
        ) -> Result<usize> {
            value.serialize(self)
        }
        fn serialize_newtype_variant<T: ?Sized + Serialize>(
            self,
            _: &'static str,
            _: u32,
            variant: &'static str,
            value: &T,
        ) -> Result<usize> {
            Ok(string(variant)
                .saturating_add(value.serialize(self)?)
                .saturating_add(3))
        }
        fn serialize_seq(self, _: Option<usize>) -> Result<Container> {
            Ok(Container(2))
        }
        fn serialize_tuple(self, _: usize) -> Result<Container> {
            Ok(Container(2))
        }
        fn serialize_tuple_struct(self, _: &'static str, _: usize) -> Result<Container> {
            Ok(Container(2))
        }
        fn serialize_tuple_variant(
            self,
            _: &'static str,
            _: u32,
            variant: &'static str,
            _: usize,
        ) -> Result<Container> {
            Ok(Container(string(variant).saturating_add(5)))
        }
        fn serialize_map(self, _: Option<usize>) -> Result<Container> {
            Ok(Container(2))
        }
        fn serialize_struct(self, _: &'static str, _: usize) -> Result<Container> {
            Ok(Container(2))
        }
        fn serialize_struct_variant(
            self,
            _: &'static str,
            _: u32,
            variant: &'static str,
            _: usize,
        ) -> Result<Container> {
            Ok(Container(string(variant).saturating_add(5)))
        }
        fn collect_str<T: ?Sized + std::fmt::Display>(self, value: &T) -> Result<usize> {
            struct DisplaySize(usize);
            impl std::fmt::Write for DisplaySize {
                fn write_str(&mut self, value: &str) -> std::fmt::Result {
                    self.0 = self.0.saturating_add(string(value).saturating_sub(2));
                    Ok(())
                }
            }
            let mut count = DisplaySize(2);
            std::fmt::write(&mut count, format_args!("{value}")).map_err(ser::Error::custom)?;
            Ok(count.0)
        }
    }
    impl Container {
        fn item<T: ?Sized + Serialize>(&mut self, value: &T) -> Result<()> {
            self.0 = self
                .0
                .saturating_add(value.serialize(JsonSize)?)
                .saturating_add(1);
            Ok(())
        }
    }
    macro_rules! sequence {
        ($trait:ident, $method:ident) => {
            impl ser::$trait for Container {
                type Ok = usize;
                type Error = serde_json::Error;
                fn $method<T: ?Sized + Serialize>(&mut self, value: &T) -> Result<()> {
                    self.item(value)
                }
                fn end(self) -> Result<usize> {
                    Ok(self.0)
                }
            }
        };
    }
    sequence!(SerializeSeq, serialize_element);
    sequence!(SerializeTuple, serialize_element);
    sequence!(SerializeTupleStruct, serialize_field);
    sequence!(SerializeTupleVariant, serialize_field);
    impl ser::SerializeMap for Container {
        type Ok = usize;
        type Error = serde_json::Error;
        fn serialize_key<T: ?Sized + Serialize>(&mut self, key: &T) -> Result<()> {
            // JSON map keys are quoted even when their Serde representation is numeric.
            self.0 = self.0.saturating_add(2);
            self.item(key)
        }
        fn serialize_value<T: ?Sized + Serialize>(&mut self, value: &T) -> Result<()> {
            self.item(value)
        }
        fn end(self) -> Result<usize> {
            Ok(self.0)
        }
    }
    macro_rules! record {
        ($trait:ident) => {
            impl ser::$trait for Container {
                type Ok = usize;
                type Error = serde_json::Error;
                fn serialize_field<T: ?Sized + Serialize>(
                    &mut self,
                    key: &'static str,
                    value: &T,
                ) -> Result<()> {
                    self.0 = self.0.saturating_add(string(key)).saturating_add(1);
                    self.item(value)
                }
                fn end(self) -> Result<usize> {
                    Ok(self.0)
                }
            }
        };
    }
    record!(SerializeStruct);
    record!(SerializeStructVariant);
}
fn entry_heap(id: &String, entry: &Entry) -> usize {
    id.capacity() + entry.history.capacity() * size_of::<WorkloadHistoryPoint>()
}
fn stable_id(row: &WorkloadDetailV4) -> &str {
    match row {
        WorkloadDetailV4::Process(d) => &d.stable_id,
        WorkloadDetailV4::Group(d) => &d.stable_id,
    }
}
fn history_point(
    row: &WorkloadDetailV4,
    catalog: &InspectionCatalog,
    interval_ms: u32,
) -> WorkloadHistoryPoint {
    let observation = |semantics: &[MetricSemantic]| {
        let (metrics, coverage, total) = match row {
            WorkloadDetailV4::Process(d) => (&d.metrics, None, 1),
            WorkloadDetailV4::Group(d) => {
                (&d.metrics, Some(&d.coverage), d.member_ids.len() as u32)
            }
        };
        let mut result = HistoryObservation {
            value: Some(0.0),
            quality: MetricQualityV4::Native,
            source: MetricSourceV4::Unknown,
            network_scope: None,
            available: total,
            total,
        };
        for semantic in semantics {
            let Some(metric) = metrics
                .iter()
                .find(|metric| catalog.descriptors[metric.0 as usize].semantic == *semantic)
            else {
                result.value = None;
                result.quality = MetricQualityV4::Unavailable;
                result.available = 0;
                continue;
            };
            let descriptor = &catalog.descriptors[metric.0 as usize];
            let quality = catalog.quality_codes[metric.2 as usize];
            let count = coverage
                .and_then(|coverage| {
                    coverage
                        .iter()
                        .find(|count| count.descriptor_index == metric.0)
                })
                .map(|count| count.available_contributors)
                .unwrap_or(if metric.1.is_some() { total } else { 0 });
            result.available = result.available.min(count);
            if quality_rank(quality) > quality_rank(result.quality) {
                result.quality = quality;
            }
            result.source = descriptor.source;
            result.network_scope = descriptor.network_scope;
            result.value = match (result.value, metric.1) {
                (Some(left), Some(right))
                    if !matches!(
                        quality,
                        MetricQualityV4::Held | MetricQualityV4::Unavailable
                    ) && descriptor.source != MetricSourceV4::Unknown
                        && count > 0 =>
                {
                    Some(left + right)
                }
                _ => None,
            };
        }
        result
    };
    let group = matches!(row, WorkloadDetailV4::Group(_));
    WorkloadHistoryPoint {
        sample_seq: catalog.sample_seq,
        sampled_at_ms: catalog.sampled_at_ms,
        interval_ms,
        gap_before: true,
        cpu: observation(&[MetricSemantic::CpuUsage]),
        memory: observation(&[MetricSemantic::ResidentMemory]),
        io: observation(if group {
            &[MetricSemantic::ReadWriteIoRate]
        } else {
            &[MetricSemantic::ReadIoRate, MetricSemantic::WriteIoRate]
        }),
        network: observation(if group {
            &[MetricSemantic::NetworkRate]
        } else {
            &[
                MetricSemantic::NetworkReceiveRate,
                MetricSemantic::NetworkTransmitRate,
            ]
        }),
    }
}
fn scope_changed(left: &WorkloadHistoryPoint, right: &WorkloadHistoryPoint) -> bool {
    [
        (left.cpu, right.cpu),
        (left.memory, right.memory),
        (left.io, right.io),
        (left.network, right.network),
    ]
    .iter()
    .any(|(left, right)| {
        left.source != right.source
            || left.network_scope != right.network_scope
            || left.available != right.available
            || left.total != right.total
    })
}
fn quality_rank(quality: MetricQualityV4) -> u8 {
    match quality {
        MetricQualityV4::Native => 0,
        MetricQualityV4::Estimated => 1,
        MetricQualityV4::Partial => 2,
        MetricQualityV4::Held => 3,
        MetricQualityV4::Unavailable => 4,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> RuntimeSnapshot {
        crate::protocol::test_runtime_snapshot()
    }
    #[test]
    fn allocation_size_bound_covers_hostile_json_shapes_and_catalog_spare_capacity() {
        fn covers(value: &impl Serialize) {
            assert!(
                serialized_size_bound(value).unwrap() >= serde_json::to_vec(value).unwrap().len()
            );
        }
        #[derive(Serialize)]
        enum Variant {
            Unit,
            Newtype(String),
            Tuple(i64, String),
            Record { value: u128 },
        }
        #[derive(Serialize)]
        struct Tuple(u8, u16);
        struct Bytes<'a>(&'a [u8]);
        impl Serialize for Bytes<'_> {
            fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                serializer.serialize_bytes(self.0)
            }
        }
        struct Display<'a>(&'a str);
        impl Serialize for Display<'_> {
            fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                serializer.collect_str(self.0)
            }
        }
        let text = (0..=127).map(char::from).collect::<String>() + "é漢字🦇\\\"";
        covers(&text);
        covers(&Display(&text));
        covers(&Bytes(text.as_bytes()));
        covers(&vec![text.clone(); 32]);
        covers(&(None::<u32>, Some(0u32), (), true, false, '\0', '🦇'));
        covers(&(
            i8::MIN,
            i16::MIN,
            i32::MIN,
            i64::MIN,
            i128::MIN,
            u8::MAX,
            u16::MAX,
            u32::MAX,
            u64::MAX,
            u128::MAX,
        ));
        covers(&[
            f64::MIN,
            f64::MAX,
            f64::MIN_POSITIVE,
            f64::EPSILON,
            f64::from_bits(1),
            -0.0,
            f64::NAN,
            f64::INFINITY,
        ]);
        covers(&[f32::MIN, f32::MAX, f32::from_bits(1)]);
        covers(&Tuple(1, 2));
        covers(&Variant::Unit);
        covers(&Variant::Newtype(text.clone()));
        covers(&Variant::Tuple(i64::MIN, text.clone()));
        covers(&Variant::Record { value: u128::MAX });
        covers(&std::collections::BTreeMap::from([(
            i64::MIN,
            text.clone(),
        )]));
        covers(&std::collections::BTreeMap::from([(text.clone(), text)]));
        let mut archive = WorkloadArchive::default();
        let snapshot = fixture();
        for row in &snapshot.process_view_rows {
            covers(row);
        }
        archive
            .observe(&snapshot, &snapshot.process_view_rows, true)
            .unwrap();
        for bundle in archive.bundles.values() {
            covers(&bundle.catalog);
            assert!(bundle.bytes >= catalog_heap_bytes(&bundle.catalog) * 4);
        }
        let mut catalog = archive.bundles.values().next().unwrap().catalog.clone();
        let before = catalog_heap_bytes(&catalog);
        catalog.workloads.reserve(10_000);
        let first = &mut catalog.workloads[0];
        let text = match first {
            WorkloadDetailV4::Process(detail) => &mut detail.display_name,
            WorkloadDetailV4::Group(detail) => &mut detail.label,
        };
        text.reserve(100_000);
        assert!(
            catalog_heap_bytes(&catalog)
                >= before + 100_000 + 9_000 * size_of::<WorkloadDetailV4>()
        );
    }
    #[test]
    #[ignore = "bounded archive performance probe; run explicitly with --ignored --nocapture"]
    fn archive_publication_cost_probe() {
        use std::time::{Duration, Instant};
        for count in [128, 512, 1_024] {
            let mut snapshot = fixture();
            let processes = (0..count)
                .map(|index| {
                    let mut process = snapshot.processes[0].clone();
                    process.pid = (10_000 + index).to_string();
                    process.parent_pid = None;
                    process.start_time_ms = 1_000 + index;
                    process.name = format!("process-{index}");
                    process.exe = format!("/usr/bin/process-{index}");
                    process
                })
                .collect::<Vec<_>>();
            let mut archive = WorkloadArchive::default();
            let mut shaping = Duration::ZERO;
            let mut observation = Duration::ZERO;
            let mut accounting = Duration::ZERO;
            for _ in 0..3 {
                snapshot.sample_seq += 1;
                snapshot.publication_seq += 1;
                snapshot.sampled_at_ms = snapshot.sampled_at_ms.map(|time| time + 1_000);
                archive.begin_ingress(&processes).unwrap();
                let started = Instant::now();
                let rows = crate::runtime_store::shape_full_process_view(&processes);
                shaping += started.elapsed();
                let started = Instant::now();
                archive.observe(&snapshot, &rows, true).unwrap();
                observation += started.elapsed();
                assert!(
                    archive.retained_bytes() + REPLY_BUDGET_BYTES + archive.ingress_reserved
                        <= HISTORY_BUDGET_BYTES
                );
                let started = Instant::now();
                for row in &rows {
                    std::hint::black_box(serialized_size_bound(row).unwrap());
                }
                for bundle in archive.bundles.values() {
                    std::hint::black_box(serialized_size_bound(&bundle.catalog).unwrap());
                }
                accounting += started.elapsed();
                drop(rows);
                archive.end_ingress();
                assert!(archive.retained_bytes() + REPLY_BUDGET_BYTES <= HISTORY_BUDGET_BYTES);
            }
            eprintln!("archive processes={count} shaping_mean_ms={:.2} observation_mean_ms={:.2} accounting_only_mean_ms={:.2} retained_bytes={} identities={}", shaping.as_secs_f64() * 1_000.0 / 3.0, observation.as_secs_f64() * 1_000.0 / 3.0, accounting.as_secs_f64()*1_000.0/3.0, archive.retained_bytes(), archive.entries.len());
        }
    }
    #[test]
    fn selection_history_survives_filters_switches_and_exit() {
        let mut archive = WorkloadArchive::default();
        let mut snapshot = fixture();
        let rows = snapshot.process_view_rows.clone();
        archive.observe(&snapshot, &rows, true).unwrap();
        let id = archive
            .entries
            .keys()
            .find(|id| id.starts_with("process:"))
            .unwrap()
            .clone();
        snapshot.process_view_rows.clear();
        snapshot.publication_seq += 1;
        archive.observe(&snapshot, &rows, true).unwrap();
        assert_eq!(archive.inspect(&id, 72).unwrap().history.len(), 1);
        snapshot.sample_seq += 1;
        snapshot.publication_seq += 1;
        snapshot.sampled_at_ms = snapshot.sampled_at_ms.map(|time| time + 1_000);
        snapshot.published_at_ms += 1_000;
        archive.observe(&snapshot, &[], true).unwrap();
        let exited = archive.inspect(&id, 72).unwrap();
        assert_eq!(exited.status, InspectionStatus::Exited);
        assert!(exited.catalog.is_some());
        assert_eq!(exited.history.len(), 1);
    }
    #[test]
    fn bounded_archive_marks_evicted_and_never_fabricates_unknown() {
        let mut archive = WorkloadArchive::with_budget(24_000);
        let snapshot = fixture();
        archive
            .observe(&snapshot, &snapshot.process_view_rows, true)
            .unwrap();
        assert!(archive.retained_bytes() <= 24_000);
        let unknown = archive.inspect("process:99999:1", 72).unwrap();
        assert_eq!(unknown.status, InspectionStatus::Unknown);
        assert!(unknown.catalog.is_none());
        assert!(unknown.history.is_empty());
        assert!(archive.inspect("x", 73).is_err());
    }
    #[test]
    fn timestamps_cadence_gaps_quality_and_zero_survive_retention() {
        let mut archive = WorkloadArchive::default();
        let mut snapshot = fixture();
        let rows = snapshot.process_view_rows.clone();
        let first = snapshot.sampled_at_ms.unwrap();
        for index in 0..365 {
            snapshot.sample_seq += 1;
            snapshot.publication_seq += 1;
            snapshot.sampled_at_ms =
                Some(first + index * 1_000 + if index >= 5 { 5_000 } else { 0 });
            snapshot.published_at_ms = snapshot.sampled_at_ms.unwrap();
            archive.observe(&snapshot, &rows, true).unwrap();
        }
        let id = archive.entries.keys().next().unwrap();
        let result = archive.inspect(id, 360).unwrap();
        assert_eq!(result.history.len(), 360);
        assert!(result
            .history
            .windows(2)
            .all(|pair| pair[0].sampled_at_ms < pair[1].sampled_at_ms));
        assert!(result.history[0].gap_before);
        for limit in [30, 72, 180, 360] {
            assert_eq!(
                archive.inspect(id, limit).unwrap().history.len(),
                limit as usize
            );
        }
        assert!(archive.retained_bytes() <= HISTORY_BUDGET_BYTES);
    }
    #[test]
    fn response_credits_survive_serialization_and_drop_until_delivery_acknowledged() {
        let mut archive = WorkloadArchive::default();
        let snapshot = fixture();
        archive
            .observe(&snapshot, &snapshot.process_view_rows, true)
            .unwrap();
        let id = archive.entries.keys().next().unwrap().clone();
        let mut replies = Vec::new();
        while let Ok(reply) = archive.inspect(&id, 360) {
            let encoded = serde_json::to_vec(&reply).unwrap();
            let token = reply.response_token.clone();
            let credit = archive.replies.lock().unwrap().pending[&token
                .strip_prefix("inspection:")
                .unwrap()
                .parse::<u64>()
                .unwrap()];
            assert!(encoded.capacity() < credit);
            replies.push(token);
        }
        assert!(!replies.is_empty());
        assert!(archive.replies.lock().unwrap().used <= REPLY_BUDGET_BYTES);
        assert!(archive.inspect(&id, 360).is_err());
        for token in replies {
            archive.acknowledge(&token).unwrap();
            archive.acknowledge(&token).unwrap();
        }
        assert_eq!(archive.replies.lock().unwrap().used, 0);
        assert!(archive.inspect(&id, 360).is_ok());
    }
    #[test]
    fn ingress_is_admitted_before_shape_and_publication_fence_hides_future_samples() {
        let mut archive = WorkloadArchive::default();
        let snapshot = fixture();
        let mut process = snapshot.processes[0].clone();
        process.exe = "x".repeat(8 * 1024 * 1024);
        assert!(archive.begin_ingress(&[process]).is_err());
        assert!(archive.entries.is_empty());
        archive
            .observe(&snapshot, &snapshot.process_view_rows, true)
            .unwrap();
        let id = archive.entries.keys().next().unwrap();
        assert!(archive
            .inspect_published(id, 72, snapshot.publication_seq - 1)
            .is_err());
        assert!(archive
            .inspect_published(id, 72, snapshot.publication_seq)
            .is_ok());
    }
    #[test]
    fn duplicate_clock_samples_are_separate_segments_and_regression_fails_closed() {
        let mut archive = WorkloadArchive::default();
        let mut snapshot = fixture();
        archive
            .observe(&snapshot, &snapshot.process_view_rows, true)
            .unwrap();
        snapshot.sample_seq += 1;
        snapshot.publication_seq += 1;
        archive
            .observe(&snapshot, &snapshot.process_view_rows, true)
            .unwrap();
        let id = archive.entries.keys().next().unwrap().clone();
        let result = archive.inspect(&id, 72).unwrap();
        assert_eq!(result.history.len(), 2);
        assert!(result.history[1].gap_before);
        assert_eq!(
            result.history[0].sampled_at_ms,
            result.history[1].sampled_at_ms
        );
        snapshot.sample_seq += 1;
        snapshot.publication_seq += 1;
        snapshot.sampled_at_ms = snapshot.sampled_at_ms.map(|time| time - 1);
        assert!(archive
            .observe(&snapshot, &snapshot.process_view_rows, true)
            .is_err());
        assert!(archive.inspect(&id, 72).is_err());
    }
    #[test]
    fn inspection_wire_fixture_is_current() {
        let mut archive = WorkloadArchive::default();
        let snapshot = fixture();
        archive
            .observe(&snapshot, &snapshot.process_view_rows, true)
            .unwrap();
        let id = archive
            .entries
            .keys()
            .filter(|id| id.starts_with("group:"))
            .min()
            .unwrap();
        let response = archive.inspect(id, 72).unwrap();
        let json = serde_json::to_string_pretty(&response).unwrap();
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/src/fixtures/workload-inspection-v1.json"
        );
        if std::env::var_os("BATCAVE_UPDATE_PROTOCOL_GOLDENS").is_some() {
            std::fs::write(path, &json).unwrap();
        } else {
            assert_eq!(
                json,
                std::fs::read_to_string(path).expect("regenerate inspection wire fixture")
            );
        }
    }
    #[test]
    fn unknown_start_and_restarted_process_never_reuse_history() {
        let mut archive = WorkloadArchive::default();
        let mut snapshot = fixture();
        let mut process = snapshot.processes[0].clone();
        process.start_time_ms = 0;
        let rows = crate::runtime_store::shape_full_process_view(&[process]);
        archive.observe(&snapshot, &rows, true).unwrap();
        let first_id = archive.entries.keys().next().unwrap().clone();
        snapshot.sample_seq += 1;
        snapshot.publication_seq += 1;
        snapshot.sampled_at_ms = snapshot.sampled_at_ms.map(|time| time + 1000);
        snapshot.published_at_ms += 1000;
        archive.observe(&snapshot, &rows, true).unwrap();
        assert_eq!(
            archive.inspect(&first_id, 72).unwrap().status,
            InspectionStatus::Exited
        );
        assert_eq!(archive.entries.len(), 2);
        assert!(archive
            .entries
            .values()
            .all(|entry| entry.history.len() == 1));
    }
    #[test]
    fn warm_cache_and_repeated_publications_do_not_create_history() {
        let mut archive = WorkloadArchive::default();
        let snapshot = fixture();
        archive
            .observe(&snapshot, &snapshot.process_view_rows, false)
            .unwrap();
        assert!(archive.entries.is_empty());
    }
}
