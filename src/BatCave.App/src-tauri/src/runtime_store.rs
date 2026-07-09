use std::{
    cmp::Ordering,
    collections::{HashMap, VecDeque},
    env, fs,
    path::PathBuf,
    sync::Mutex,
    time::Instant,
};

use serde::de::DeserializeOwned;

use crate::{
    atomic_json::{write_json_atomic, AtomicJsonErrorLabels},
    contracts::{
        MetricQuality, ProcessFocusMode, ProcessSample, ProcessViewRow, ProcessViewRowKind,
        RuntimeHealth, RuntimeQuery, RuntimeSettings, RuntimeSnapshot, RuntimeWarning, SortColumn,
        SortDirection, SystemMemoryAccounting, SystemMetricsSnapshot, WarmCache,
    },
    elevation::ElevatedHelperClient,
    telemetry::{now_ms, TelemetryCollector},
};

const SETTINGS_FILE: &str = "settings.json";
const WARM_CACHE_FILE: &str = "warm-cache.json";
const DIAGNOSTICS_FILE: &str = "diagnostics.jsonl";
const MAX_WARNINGS: usize = 16;
const WARM_CACHE_WRITE_INTERVAL_TICKS: u64 = 10;
const APP_CPU_DEGRADE_PCT: f64 = 6.0;
const APP_RSS_DEGRADE_BYTES: u64 = 350 * 1024 * 1024;
const PERSISTENCE_JSON_ERRORS: AtomicJsonErrorLabels = AtomicJsonErrorLabels {
    write_failed: "persistence_write_failed",
    serialize_failed: "persistence_serialize_failed",
    replace_failed: "persistence_replace_failed",
    rename_failed: "persistence_rename_failed",
    serialize_error_includes_path: true,
};

pub struct RuntimeState {
    store: Mutex<RuntimeStore>,
}

impl RuntimeState {
    pub fn new() -> Self {
        Self {
            store: Mutex::new(RuntimeStore::new()),
        }
    }

    pub fn snapshot(&self) -> Result<RuntimeSnapshot, String> {
        self.with_store(|store| store.snapshot())
    }

    pub fn refresh_now(&self) -> Result<RuntimeSnapshot, String> {
        self.with_store(|store| store.refresh_now())
    }

    pub fn pause(&self) -> Result<RuntimeSnapshot, String> {
        self.with_store(|store| store.set_paused(true))
    }

    pub fn resume(&self) -> Result<RuntimeSnapshot, String> {
        self.with_store(|store| store.set_paused(false))
    }

    pub fn set_admin_mode(&self, enabled: bool) -> Result<RuntimeSnapshot, String> {
        self.with_store(|store| store.set_admin_mode(enabled))
    }

    pub fn set_query(&self, query: RuntimeQuery) -> Result<RuntimeSnapshot, String> {
        self.with_store(|store| store.set_query(query))
    }

    pub fn has_process_exe(&self, exe: &str) -> Result<bool, String> {
        self.with_store(|store| store.has_process_exe(exe))
    }

    fn with_store<T>(&self, action: impl FnOnce(&mut RuntimeStore) -> T) -> Result<T, String> {
        let mut store = self
            .store
            .lock()
            .map_err(|_| "runtime store lock is poisoned".to_string())?;
        Ok(action(&mut store))
    }
}

struct RuntimeStore {
    collector: TelemetryCollector,
    base_dir: PathBuf,
    settings: RuntimeSettings,
    elevated: Option<ElevatedHelperClient>,
    snapshot: RuntimeSnapshot,
    warnings: VecDeque<RuntimeWarning>,
    previous_totals: Option<TelemetryTotals>,
    previous_processes: Vec<ProcessSample>,
    live_process_snapshot: bool,
    tick_p95: P95Window,
    sort_p95: P95Window,
    jitter_p95: P95Window,
    last_tick_at: Option<Instant>,
    dropped_ticks: u64,
    seq: u64,
}

impl RuntimeStore {
    fn new() -> Self {
        Self::from_base_dir(default_base_dir())
    }

    fn from_base_dir(base_dir: PathBuf) -> Self {
        ElevatedHelperClient::remove_stale_artifacts(&base_dir);
        let mut warnings = VecDeque::new();
        let settings =
            read_json::<RuntimeSettings>(&base_dir.join(SETTINGS_FILE)).unwrap_or_else(|error| {
                if let Some(message) = error {
                    push_warning(&mut warnings, 0, "persistence", message);
                }
                RuntimeSettings::default()
            });
        let settings_requested_admin = settings.admin_mode_requested;
        let mut warm_cache = read_json::<WarmCache>(&base_dir.join(WARM_CACHE_FILE))
            .unwrap_or_else(|error| {
                if let Some(message) = error {
                    push_warning(&mut warnings, 0, "persistence", message);
                }
                WarmCache {
                    seq: 0,
                    rows: Vec::new(),
                }
            });
        let seq = warm_cache.seq;
        let settings = normalize_settings(settings);
        if settings_requested_admin {
            warm_cache.rows.clear();
            let _ = fs::remove_file(base_dir.join(WARM_CACHE_FILE));
        }
        let snapshot = build_snapshot(
            seq,
            now_ms(),
            &settings,
            RuntimeHealth::default(),
            empty_system(),
            shape_rows(&warm_cache.rows, &settings.query),
            warm_cache.rows.len(),
            warnings.iter().cloned().collect(),
        );

        Self {
            collector: TelemetryCollector::new(),
            base_dir,
            settings,
            elevated: None,
            snapshot,
            warnings,
            previous_totals: None,
            previous_processes: warm_cache.rows,
            live_process_snapshot: false,
            tick_p95: P95Window::new(120),
            sort_p95: P95Window::new(120),
            jitter_p95: P95Window::new(120),
            last_tick_at: None,
            dropped_ticks: 0,
            seq,
        }
    }

    fn snapshot(&mut self) -> RuntimeSnapshot {
        if self.settings.paused {
            return self.snapshot.clone();
        }

        self.tick()
    }

    fn refresh_now(&mut self) -> RuntimeSnapshot {
        self.tick()
    }

    fn set_paused(&mut self, paused: bool) -> RuntimeSnapshot {
        self.settings.paused = paused;
        self.persist_settings();
        self.publish_snapshot_only(None);
        self.snapshot.clone()
    }

    fn set_admin_mode(&mut self, enabled: bool) -> RuntimeSnapshot {
        let was_admin_enabled = self.settings.admin_mode_enabled;
        self.settings.admin_mode_requested = enabled;
        self.settings.admin_mode_enabled = false;
        if enabled {
            if self.elevated.is_none() {
                match ElevatedHelperClient::start(&self.base_dir) {
                    Ok(client) => {
                        self.elevated = Some(client);
                    }
                    Err(error) => {
                        self.add_warning("admin_mode", error);
                    }
                }
            }
        } else {
            if let Some(client) = self.elevated.take() {
                client.stop();
            }
            if was_admin_enabled {
                self.previous_processes.clear();
                self.previous_totals = None;
            }
            self.purge_warm_cache();
            ElevatedHelperClient::remove_stale_artifacts(&self.base_dir);
        }
        self.persist_settings();
        self.publish_snapshot_only(None);
        self.snapshot.clone()
    }

    fn set_query(&mut self, query: RuntimeQuery) -> RuntimeSnapshot {
        self.settings.query = normalize_query(query);
        self.persist_settings();
        self.publish_snapshot_only(None);
        self.snapshot.clone()
    }

    fn has_process_exe(&mut self, exe: &str) -> bool {
        let exe = exe.trim();
        self.live_process_snapshot
            && !exe.is_empty()
            && self
                .snapshot
                .processes
                .iter()
                .any(|process| process.exe.eq_ignore_ascii_case(exe))
    }

    fn tick(&mut self) -> RuntimeSnapshot {
        self.seq = self.seq.saturating_add(1);
        let tick_started = Instant::now();
        let previous_tick_at = self.last_tick_at.replace(tick_started);
        let sample = match self.collector.collect() {
            Ok(sample) => sample,
            Err(error) => {
                self.add_warning("collector", error);
                self.publish_snapshot_only(None);
                return self.snapshot.clone();
            }
        };
        let tick_ms = tick_started.elapsed().as_secs_f64() * 1000.0;
        self.tick_p95.add(tick_ms);

        if let Some(previous) = previous_tick_at {
            let elapsed_ms = tick_started.duration_since(previous).as_secs_f64() * 1000.0;
            let jitter = (elapsed_ms - 1000.0).abs();
            self.jitter_p95.add(jitter);
            if jitter > 1500.0 {
                self.dropped_ticks = self.dropped_ticks.saturating_add((jitter / 1000.0) as u64);
            }
        }

        let sample_ts_ms = now_ms();
        for warning in &sample.warnings {
            self.add_warning("collector", warning.clone());
        }

        let elapsed_seconds = self
            .previous_totals
            .as_ref()
            .and_then(|previous| {
                let delta_ms = sample_ts_ms.saturating_sub(previous.ts_ms);
                (delta_ms > 0).then_some(delta_ms as f64 / 1000.0)
            })
            .unwrap_or(1.0)
            .max(0.5);
        let mut system = sample.system;
        let mut sample_processes = sample.processes;
        if self.settings.admin_mode_requested {
            let mut admin_warning = None;
            let mut stop_elevated = false;
            if let Some(elevated) = &mut self.elevated {
                match elevated.poll_rows() {
                    Ok(Some(rows)) => {
                        sample_processes = rows;
                        self.settings.admin_mode_enabled = true;
                    }
                    Ok(None) => {
                        self.settings.admin_mode_enabled = false;
                    }
                    Err(error) => {
                        self.settings.admin_mode_enabled = false;
                        admin_warning = Some(error);
                        stop_elevated = true;
                    }
                }
            } else {
                self.settings.admin_mode_enabled = false;
            }
            if stop_elevated {
                if let Some(client) = self.elevated.take() {
                    client.stop();
                }
            }
            if let Some(error) = admin_warning {
                self.add_warning("admin_mode", error);
            }
        } else {
            self.settings.admin_mode_enabled = false;
        }
        if let Some(previous) = &self.previous_totals {
            if system.disk_read_bps == 0 && system.disk_write_bps == 0 {
                system.disk_read_bps = byte_rate(
                    system.disk_read_total_bytes,
                    previous.disk_read_total_bytes,
                    elapsed_seconds,
                );
                system.disk_write_bps = byte_rate(
                    system.disk_write_total_bytes,
                    previous.disk_write_total_bytes,
                    elapsed_seconds,
                );
            }
            system.network_received_bps = byte_rate(
                system.network_received_total_bytes,
                previous.network_received_total_bytes,
                elapsed_seconds,
            );
            system.network_transmitted_bps = byte_rate(
                system.network_transmitted_total_bytes,
                previous.network_transmitted_total_bytes,
                elapsed_seconds,
            );
        }

        let processes =
            add_process_rates(sample_processes, &self.previous_processes, elapsed_seconds);
        add_process_memory_accounting(&mut system, &processes);
        self.previous_processes = processes;
        self.live_process_snapshot = true;
        self.previous_totals = Some(TelemetryTotals::from_system(&system, sample_ts_ms));

        let sort_started = Instant::now();
        let rows = shape_rows(&self.previous_processes, &self.settings.query);
        let sort_ms = sort_started.elapsed().as_secs_f64() * 1000.0;
        self.sort_p95.add(sort_ms);

        let app_metrics = current_app_metrics(&self.previous_processes);
        let health = self.build_health(
            sample.latency_ms,
            app_metrics.cpu_percent,
            app_metrics.rss_bytes,
        );
        self.snapshot = build_snapshot(
            self.seq,
            now_ms(),
            &self.settings,
            health,
            system,
            rows,
            self.previous_processes.len(),
            self.warnings.iter().cloned().collect(),
        );

        if self.seq % WARM_CACHE_WRITE_INTERVAL_TICKS == 0 {
            self.persist_warm_cache();
        }

        self.snapshot.clone()
    }

    fn publish_snapshot_only(&mut self, warning: Option<&str>) {
        self.seq = self.seq.saturating_add(1);
        if let Some(message) = warning {
            self.add_warning("runtime", message.to_string());
        }

        let app_metrics = current_app_metrics(&self.previous_processes);
        let health = self.build_health(0, app_metrics.cpu_percent, app_metrics.rss_bytes);
        let rows = shape_rows(&self.previous_processes, &self.settings.query);
        self.snapshot = build_snapshot(
            self.seq,
            now_ms(),
            &self.settings,
            health,
            self.snapshot.system.clone(),
            rows,
            self.previous_processes.len(),
            self.warnings.iter().cloned().collect(),
        );
    }

    fn build_health(
        &self,
        latency_ms: u64,
        app_cpu_percent: f64,
        app_rss_bytes: u64,
    ) -> RuntimeHealth {
        let cpu_degraded = app_cpu_percent >= APP_CPU_DEGRADE_PCT;
        let rss_degraded = app_rss_bytes >= APP_RSS_DEGRADE_BYTES;
        let warning_degraded_count = self
            .warnings
            .iter()
            .filter(|warning| warning_degrades_health(&warning.category))
            .count();
        let warning_degraded = warning_degraded_count > 0;
        let last_warning = self.warnings.back().map(|warning| warning.message.clone());
        let status_summary = if self.settings.paused {
            "Paused.".to_string()
        } else if self.settings.admin_mode_requested && !self.settings.admin_mode_enabled {
            "Standard access: admin mode requested but elevation is inactive.".to_string()
        } else if warning_degraded {
            format!("Collector warnings present: {warning_degraded_count} retained.")
        } else if cpu_degraded && rss_degraded {
            "Degraded: app CPU and RSS above budget.".to_string()
        } else if cpu_degraded {
            "Degraded: app CPU above budget.".to_string()
        } else if rss_degraded {
            "Degraded: app RSS above budget.".to_string()
        } else {
            "Healthy.".to_string()
        };

        RuntimeHealth {
            tick_count: self.seq,
            snapshot_latency_ms: latency_ms,
            degraded: cpu_degraded || rss_degraded || warning_degraded,
            collector_warnings: self.warnings.len(),
            runtime_loop_enabled: true,
            runtime_loop_running: !self.settings.paused,
            status_summary,
            updated_at_ms: now_ms(),
            tick_p95_ms: round1(self.tick_p95.value()),
            sort_p95_ms: round1(self.sort_p95.value()),
            jitter_p95_ms: round1(self.jitter_p95.value()),
            dropped_ticks: self.dropped_ticks,
            app_cpu_percent: round1(app_cpu_percent),
            app_rss_bytes,
            last_warning,
        }
    }

    fn add_warning(&mut self, category: &str, message: String) {
        push_warning(&mut self.warnings, self.seq, category, message.clone());
        self.append_diagnostic(category, &message);
    }

    fn persist_settings(&mut self) {
        if let Err(error) = write_json_atomic(
            &self.base_dir.join(SETTINGS_FILE),
            &self.settings,
            PERSISTENCE_JSON_ERRORS,
        ) {
            self.add_warning("persistence", error);
        }
    }

    fn persist_warm_cache(&mut self) {
        if self.settings.admin_mode_enabled {
            return;
        }
        let cache = WarmCache {
            seq: self.seq,
            rows: self.previous_processes.clone(),
        };
        if let Err(error) = write_json_atomic(
            &self.base_dir.join(WARM_CACHE_FILE),
            &cache,
            PERSISTENCE_JSON_ERRORS,
        ) {
            self.add_warning("persistence", error);
        }
    }

    fn purge_warm_cache(&mut self) {
        let path = self.base_dir.join(WARM_CACHE_FILE);
        match fs::remove_file(&path) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => self.add_warning(
                "persistence",
                format!(
                    "persistence_remove_failed path={} error={}",
                    path.display(),
                    error
                ),
            ),
        }
    }

    fn append_diagnostic(&self, category: &str, message: &str) {
        let path = self.base_dir.join(DIAGNOSTICS_FILE);
        let payload = serde_json::json!({
            "ts_ms": now_ms(),
            "category": category,
            "payload": { "message": message },
        });
        let _ = fs::create_dir_all(&self.base_dir);
        let _ = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .and_then(|mut file| {
                use std::io::Write;
                writeln!(file, "{payload}")
            });
    }
}

fn warning_degrades_health(category: &str) -> bool {
    matches!(category, "collector" | "admin_mode" | "persistence")
}

#[derive(Debug, Clone)]
struct TelemetryTotals {
    ts_ms: u64,
    disk_read_total_bytes: u64,
    disk_write_total_bytes: u64,
    network_received_total_bytes: u64,
    network_transmitted_total_bytes: u64,
}

impl TelemetryTotals {
    fn from_system(system: &SystemMetricsSnapshot, ts_ms: u64) -> Self {
        Self {
            ts_ms,
            disk_read_total_bytes: system.disk_read_total_bytes,
            disk_write_total_bytes: system.disk_write_total_bytes,
            network_received_total_bytes: system.network_received_total_bytes,
            network_transmitted_total_bytes: system.network_transmitted_total_bytes,
        }
    }
}

struct P95Window {
    values: VecDeque<f64>,
    capacity: usize,
}

#[derive(Debug, Clone, Copy, Default)]
struct CurrentAppMetrics {
    cpu_percent: f64,
    rss_bytes: u64,
}

impl P95Window {
    fn new(capacity: usize) -> Self {
        Self {
            values: VecDeque::with_capacity(capacity),
            capacity,
        }
    }

    fn add(&mut self, value: f64) {
        if self.values.len() == self.capacity {
            self.values.pop_front();
        }
        self.values.push_back(value.max(0.0));
    }

    fn value(&self) -> f64 {
        if self.values.is_empty() {
            return 0.0;
        }

        let mut values = self.values.iter().copied().collect::<Vec<_>>();
        values.sort_by(|left, right| left.partial_cmp(right).unwrap_or(Ordering::Equal));
        let index = ((values.len() as f64 * 0.95).ceil() as usize).saturating_sub(1);
        values[index.min(values.len() - 1)]
    }
}

fn build_snapshot(
    seq: u64,
    ts_ms: u64,
    settings: &RuntimeSettings,
    health: RuntimeHealth,
    system: SystemMetricsSnapshot,
    processes: Vec<ProcessSample>,
    total_process_count: usize,
    warnings: Vec<RuntimeWarning>,
) -> RuntimeSnapshot {
    let process_view_rows = shape_process_view(&processes, &settings.query);
    RuntimeSnapshot {
        event_kind: "runtime_snapshot".to_string(),
        seq,
        ts_ms,
        source: "tauri_runtime".to_string(),
        settings: settings.clone(),
        health,
        system,
        processes,
        process_view_rows,
        total_process_count,
        warnings,
    }
}

fn add_process_rates(
    mut processes: Vec<ProcessSample>,
    previous_processes: &[ProcessSample],
    elapsed_seconds: f64,
) -> Vec<ProcessSample> {
    for process in &mut processes {
        if let Some(previous) = previous_processes.iter().find(|candidate| {
            candidate.pid == process.pid && candidate.start_time_ms == process.start_time_ms
        }) {
            process.disk_read_bps = byte_rate(
                process.disk_read_total_bytes,
                previous.disk_read_total_bytes,
                elapsed_seconds,
            );
            process.disk_write_bps = byte_rate(
                process.disk_write_total_bytes,
                previous.disk_write_total_bytes,
                elapsed_seconds,
            );
            process.other_io_bps =
                match (process.other_io_total_bytes, previous.other_io_total_bytes) {
                    (Some(current), Some(previous)) => {
                        Some(byte_rate(current, previous, elapsed_seconds))
                    }
                    _ => process.other_io_bps,
                };
        }
    }

    processes
}

fn shape_rows(processes: &[ProcessSample], query: &RuntimeQuery) -> Vec<ProcessSample> {
    let needle = query.filter_text.trim().to_lowercase();
    let mut rows = processes
        .iter()
        .filter(|process| {
            needle.is_empty()
                || process.name.to_lowercase().contains(&needle)
                || process.pid.contains(&needle)
                || process.exe.to_lowercase().contains(&needle)
        })
        .filter(|process| matches_focus_mode(process, query.focus_mode))
        .cloned()
        .collect::<Vec<_>>();

    rows.sort_by(|left, right| compare_process(left, right, query));
    rows.truncate(query.limit.max(1));
    rows
}

#[derive(Debug, Clone)]
struct ProcessIdentity {
    icon_kind: &'static str,
    category: &'static str,
    is_child: bool,
}

#[derive(Debug, Clone)]
struct ProcessAppGroup {
    key: String,
    label: String,
    category: String,
    icon_kind: String,
    representative: ProcessSample,
    processes: Vec<ProcessSample>,
    cpu_percent: f64,
    memory_bytes: u64,
    io_bps: u64,
    network_bps: u64,
    threads: u64,
}

fn shape_process_view(processes: &[ProcessSample], query: &RuntimeQuery) -> Vec<ProcessViewRow> {
    let mut groups = Vec::<ProcessAppGroup>::new();
    let mut group_indexes = HashMap::<String, usize>::new();

    for process in processes {
        let key = process_app_key(process);
        let index = if let Some(index) = group_indexes.get(&key) {
            *index
        } else {
            let identity = process_identity(process);
            let index = groups.len();
            group_indexes.insert(key.clone(), index);
            groups.push(ProcessAppGroup {
                key,
                label: process_app_label(process),
                category: identity.category.to_string(),
                icon_kind: identity.icon_kind.to_string(),
                representative: process.clone(),
                processes: Vec::new(),
                cpu_percent: 0.0,
                memory_bytes: 0,
                io_bps: 0,
                network_bps: 0,
                threads: 0,
            });
            index
        };

        let group = &mut groups[index];
        group.cpu_percent += process.cpu_percent;
        group.memory_bytes = group.memory_bytes.saturating_add(process.memory_bytes);
        group.io_bps = group.io_bps.saturating_add(process_io_rate(process));
        group.network_bps = group
            .network_bps
            .saturating_add(process_network_rate(process));
        group.threads = group.threads.saturating_add(process.threads as u64);
        group.processes.push(process.clone());
    }

    groups.sort_by(|left, right| compare_process_group(left, right, query));

    let mut rows = Vec::with_capacity(processes.len() + groups.len());
    for group in groups {
        let grouped = group.processes.len() > 1;
        let group_count = group.processes.len();
        if grouped {
            rows.push(ProcessViewRow {
                kind: ProcessViewRowKind::Group,
                process: None,
                representative: Some(group.representative.clone()),
                group_key: Some(group.key.clone()),
                group_label: Some(group.label.clone()),
                group_category: Some(group.category.clone()),
                group_count,
                icon_kind: group.icon_kind.clone(),
                is_child: false,
                is_grouped: true,
                attention_label: attention_label(
                    group.cpu_percent,
                    group.memory_bytes,
                    group.io_bps,
                ),
                cpu_percent: round1(group.cpu_percent),
                memory_bytes: group.memory_bytes,
                io_bps: group.io_bps,
                network_bps: group.network_bps,
                threads: group.threads,
            });
        }

        for process in group.processes {
            let identity = process_identity(&process);
            rows.push(ProcessViewRow {
                kind: ProcessViewRowKind::Process,
                process: Some(process.clone()),
                representative: None,
                group_key: Some(group.key.clone()),
                group_label: Some(group.label.clone()),
                group_category: Some(group.category.clone()),
                group_count,
                icon_kind: identity.icon_kind.to_string(),
                is_child: identity.is_child,
                is_grouped: grouped,
                attention_label: attention_label(
                    process.cpu_percent,
                    process.memory_bytes,
                    process_io_rate(&process),
                ),
                cpu_percent: process.cpu_percent,
                memory_bytes: process.memory_bytes,
                io_bps: process_io_rate(&process),
                network_bps: process_network_rate(&process),
                threads: process.threads as u64,
            });
        }
    }

    rows
}

fn compare_process(left: &ProcessSample, right: &ProcessSample, query: &RuntimeQuery) -> Ordering {
    let ordering = match query.sort_column {
        SortColumn::Name => left.name.to_lowercase().cmp(&right.name.to_lowercase()),
        SortColumn::Pid => left.pid.cmp(&right.pid),
        SortColumn::MemoryBytes => left.memory_bytes.cmp(&right.memory_bytes),
        SortColumn::DiskBps => left
            .disk_read_bps
            .saturating_add(left.disk_write_bps)
            .cmp(&right.disk_read_bps.saturating_add(right.disk_write_bps)),
        SortColumn::NetworkBps => process_network_rate(left).cmp(&process_network_rate(right)),
        SortColumn::Threads => left.threads.cmp(&right.threads),
        SortColumn::Handles => left.handles.cmp(&right.handles),
        SortColumn::StartTimeMs => left.start_time_ms.cmp(&right.start_time_ms),
        SortColumn::Attention => process_attention_score(left)
            .partial_cmp(&process_attention_score(right))
            .unwrap_or(Ordering::Equal),
        SortColumn::CpuPct => left
            .cpu_percent
            .partial_cmp(&right.cpu_percent)
            .unwrap_or(Ordering::Equal),
    };

    let directed = match query.sort_direction {
        SortDirection::Asc => ordering,
        SortDirection::Desc => ordering.reverse(),
    };

    directed.then_with(|| left.name.to_lowercase().cmp(&right.name.to_lowercase()))
}

fn compare_process_group(
    left: &ProcessAppGroup,
    right: &ProcessAppGroup,
    query: &RuntimeQuery,
) -> Ordering {
    let ordering = match query.sort_column {
        SortColumn::Name => left.label.to_lowercase().cmp(&right.label.to_lowercase()),
        SortColumn::Pid => left.representative.pid.cmp(&right.representative.pid),
        SortColumn::MemoryBytes => left.memory_bytes.cmp(&right.memory_bytes),
        SortColumn::DiskBps => left.io_bps.cmp(&right.io_bps),
        SortColumn::NetworkBps => left.network_bps.cmp(&right.network_bps),
        SortColumn::Threads => left.threads.cmp(&right.threads),
        SortColumn::Handles => left
            .representative
            .handles
            .cmp(&right.representative.handles),
        SortColumn::StartTimeMs => left
            .representative
            .start_time_ms
            .cmp(&right.representative.start_time_ms),
        SortColumn::Attention => group_attention_score(left)
            .partial_cmp(&group_attention_score(right))
            .unwrap_or(Ordering::Equal),
        SortColumn::CpuPct => left
            .cpu_percent
            .partial_cmp(&right.cpu_percent)
            .unwrap_or(Ordering::Equal),
    };

    let directed = match query.sort_direction {
        SortDirection::Asc => ordering,
        SortDirection::Desc => ordering.reverse(),
    };

    directed.then_with(|| left.label.to_lowercase().cmp(&right.label.to_lowercase()))
}

fn matches_focus_mode(process: &ProcessSample, focus_mode: ProcessFocusMode) -> bool {
    match focus_mode {
        ProcessFocusMode::All => true,
        ProcessFocusMode::Active => process.cpu_percent >= 1.0,
        ProcessFocusMode::Io => process_io_rate(process) > 0,
    }
}

fn process_io_rate(process: &ProcessSample) -> u64 {
    process
        .disk_read_bps
        .saturating_add(process.disk_write_bps)
        .saturating_add(process.other_io_bps.unwrap_or_default())
}

fn process_network_rate(process: &ProcessSample) -> u64 {
    process
        .network_received_bps
        .unwrap_or_default()
        .saturating_add(process.network_transmitted_bps.unwrap_or_default())
}

fn process_identity(process: &ProcessSample) -> ProcessIdentity {
    let haystack = format!("{} {}", process.name, process.exe).to_lowercase();
    let name = process.name.to_lowercase();
    let is_child = name.starts_with("--")
        || haystack.contains("--type=")
        || haystack.contains("renderer")
        || haystack.contains("gpu-process")
        || haystack.contains("utility");

    if haystack.contains("batcave") {
        return ProcessIdentity {
            icon_kind: "batcave",
            category: "BatCave",
            is_child,
        };
    }

    if matches_any(
        &haystack,
        &["chrome", "msedge", "firefox", "brave", "browser"],
    ) {
        return ProcessIdentity {
            icon_kind: "browser",
            category: "Browsers",
            is_child,
        };
    }

    if matches_any(
        &haystack,
        &["code.exe", "visual studio code", "\\code\\", "/code/"],
    ) {
        return ProcessIdentity {
            icon_kind: "code",
            category: "Developer tools",
            is_child,
        };
    }

    if matches_any(&haystack, &["node", "npm", "deno", "bun.exe"]) {
        return ProcessIdentity {
            icon_kind: "node",
            category: "Runtimes",
            is_child,
        };
    }

    if haystack.contains("docker") {
        return ProcessIdentity {
            icon_kind: "container",
            category: "Containers",
            is_child,
        };
    }

    if matches_any(
        &haystack,
        &["postgres", "mysql", "redis", "sqlserver", "mariadb"],
    ) {
        return ProcessIdentity {
            icon_kind: "database",
            category: "Databases",
            is_child,
        };
    }

    if matches_any(&haystack, &["slack", "teams", "discord", "zoom"]) {
        return ProcessIdentity {
            icon_kind: "chat",
            category: "Communication",
            is_child,
        };
    }

    if matches_any(&haystack, &["spotify", "vlc", "media player"]) {
        return ProcessIdentity {
            icon_kind: "media",
            category: "Media",
            is_child,
        };
    }

    if matches_any(&haystack, &["dropbox", "onedrive", "googledrive"]) {
        return ProcessIdentity {
            icon_kind: "sync",
            category: "Sync",
            is_child,
        };
    }

    if matches_any(&haystack, &["nvidia", "amd", "radeon", "intel graphics"]) {
        return ProcessIdentity {
            icon_kind: "gpu",
            category: "GPU",
            is_child,
        };
    }

    if matches_any(
        &haystack,
        &[
            "applicationframehost",
            "conhost",
            "ctfmon",
            "dwm",
            "explorer.exe",
            "phoneexperiencehost",
            "searchindexer",
            "securityhealthservice",
            "shellexperiencehost",
            "sihost",
            "startmenuexperiencehost",
            "svchost",
            "textinputhost",
            "widgetservice",
            "windows",
        ],
    ) {
        return ProcessIdentity {
            icon_kind: "windows",
            category: "Windows",
            is_child,
        };
    }

    ProcessIdentity {
        icon_kind: "process",
        category: "Processes",
        is_child,
    }
}

fn process_app_key(process: &ProcessSample) -> String {
    let executable_name = normalized_process_name(executable_file_name(&process.exe));
    let process_name = normalized_process_name(&process.name);
    let key = if !executable_name.trim().is_empty() {
        executable_name
    } else if !process_name.trim().is_empty() {
        process_name
    } else {
        format!("pid:{}", process.pid)
    };

    key.to_lowercase()
}

fn process_app_label(process: &ProcessSample) -> String {
    normalized_process_name(&process.name)
}

fn executable_file_name(path: &str) -> &str {
    let trimmed = path.trim();
    trimmed
        .rsplit(|candidate| candidate == '\\' || candidate == '/')
        .next()
        .filter(|file_name| !file_name.is_empty())
        .unwrap_or(trimmed)
}

fn normalized_process_name(name: &str) -> String {
    let lower = name.to_lowercase();
    if !lower.ends_with(".exe") {
        return name.to_string();
    }

    let extension_start = name.len().saturating_sub(4);
    let stem = &name[..extension_start];
    if let Some((base, suffix)) = stem.rsplit_once('-') {
        if !suffix.is_empty() && suffix.chars().all(|character| character.is_ascii_digit()) {
            return format!("{base}.exe");
        }
    }

    name.to_string()
}

fn matches_any(haystack: &str, needles: &[&str]) -> bool {
    needles.iter().any(|needle| haystack.contains(needle))
}

fn attention_label(cpu_percent: f64, memory_bytes: u64, io_bps: u64) -> String {
    if cpu_percent >= 20.0 {
        return "CPU lead".to_string();
    }

    if memory_bytes >= 900 * 1024 * 1024 {
        return "memory lead".to_string();
    }

    if io_bps >= 500 * 1024 {
        return "I/O lead".to_string();
    }

    "steady".to_string()
}

fn normalize_settings(settings: RuntimeSettings) -> RuntimeSettings {
    RuntimeSettings {
        query: normalize_query(settings.query),
        admin_mode_requested: false,
        admin_mode_enabled: false,
        metric_window_seconds: settings.metric_window_seconds.clamp(15, 600),
        paused: settings.paused,
    }
}

fn process_attention_score(process: &ProcessSample) -> f64 {
    let mut score = process.cpu_percent * 3.0;
    score += (process.memory_bytes as f64 / (128.0 * 1024.0 * 1024.0)).min(20.0);
    let io_bps = process_io_rate(process);
    score += (io_bps as f64 / (512.0 * 1024.0)).min(20.0);
    let network_bps = process_network_rate(process);
    score += (network_bps as f64 / (1024.0 * 1024.0)).min(20.0);
    if process.access_state != crate::contracts::AccessState::Full {
        score += 12.0;
    }

    score
}

fn group_attention_score(group: &ProcessAppGroup) -> f64 {
    let mut score = group.cpu_percent * 3.0;
    score += (group.memory_bytes as f64 / (128.0 * 1024.0 * 1024.0)).min(20.0);
    score += (group.io_bps as f64 / (512.0 * 1024.0)).min(20.0);
    score += (group.network_bps as f64 / (1024.0 * 1024.0)).min(20.0);
    if group
        .processes
        .iter()
        .any(|process| process.access_state != crate::contracts::AccessState::Full)
    {
        score += 12.0;
    }

    score
}

fn normalize_query(query: RuntimeQuery) -> RuntimeQuery {
    RuntimeQuery {
        filter_text: query.filter_text.trim().to_string(),
        focus_mode: query.focus_mode,
        sort_column: query.sort_column,
        sort_direction: query.sort_direction,
        limit: query.limit.clamp(25, 20_000),
    }
}

fn push_warning(
    warnings: &mut VecDeque<RuntimeWarning>,
    seq: u64,
    category: &str,
    message: String,
) {
    warnings.push_back(RuntimeWarning {
        seq,
        ts_ms: now_ms(),
        category: category.to_string(),
        message,
    });
    while warnings.len() > MAX_WARNINGS {
        warnings.pop_front();
    }
}

fn empty_system() -> SystemMetricsSnapshot {
    SystemMetricsSnapshot {
        cpu_percent: 0.0,
        kernel_cpu_percent: 0.0,
        logical_cpu_percent: Vec::new(),
        memory_used_bytes: 0,
        memory_total_bytes: 0,
        memory_available_bytes: None,
        swap_used_bytes: 0,
        swap_total_bytes: 0,
        process_count: 0,
        disk_read_total_bytes: 0,
        disk_write_total_bytes: 0,
        disk_read_bps: 0,
        disk_write_bps: 0,
        network_received_total_bytes: 0,
        network_transmitted_total_bytes: 0,
        network_received_bps: 0,
        network_transmitted_bps: 0,
        memory_accounting: None,
        quality: None,
    }
}

fn add_process_memory_accounting(system: &mut SystemMetricsSnapshot, processes: &[ProcessSample]) {
    let process_working_set_bytes = processes
        .iter()
        .filter(|process| process_memory_is_reported(process))
        .fold(0_u64, |total, process| {
            total.saturating_add(process.memory_bytes)
        });
    let process_private_bytes = processes
        .iter()
        .filter(|process| process_memory_is_reported(process))
        .fold(0_u64, |total, process| {
            total.saturating_add(process.private_bytes)
        });
    let denied_process_count = processes
        .iter()
        .filter(|process| process.access_state == crate::contracts::AccessState::Denied)
        .count();
    let partial_process_count = processes
        .iter()
        .filter(|process| process.access_state == crate::contracts::AccessState::Partial)
        .count();

    let accounting = system
        .memory_accounting
        .get_or_insert_with(SystemMemoryAccounting::default);
    accounting.process_working_set_bytes = process_working_set_bytes;
    accounting.process_private_bytes = process_private_bytes;
    accounting.denied_process_count = denied_process_count;
    accounting.partial_process_count = partial_process_count;
    accounting.unattributed_bytes = Some(
        system
            .memory_used_bytes
            .saturating_sub(process_working_set_bytes),
    );
}

fn process_memory_is_reported(process: &ProcessSample) -> bool {
    process
        .quality
        .as_ref()
        .and_then(|quality| quality.memory.as_ref())
        .map(|memory| memory.quality != MetricQuality::Unavailable)
        .unwrap_or(true)
}

fn current_app_metrics(processes: &[ProcessSample]) -> CurrentAppMetrics {
    let current_pid = std::process::id().to_string();
    processes
        .iter()
        .find(|process| process.pid == current_pid)
        .map(|process| CurrentAppMetrics {
            cpu_percent: process.cpu_percent,
            rss_bytes: process.memory_bytes,
        })
        .unwrap_or_default()
}

fn byte_rate(current: u64, previous: u64, elapsed_seconds: f64) -> u64 {
    if current < previous {
        return 0;
    }

    ((current - previous) as f64 / elapsed_seconds.max(0.001)).round() as u64
}

fn read_json<T: DeserializeOwned>(path: &PathBuf) -> Result<T, Option<String>> {
    if !path.exists() {
        return Err(None);
    }

    let payload = fs::read_to_string(path).map_err(|error| {
        Some(format!(
            "persistence_load_failed path={} error={}: {}",
            path.display(),
            std::any::type_name::<T>(),
            error
        ))
    })?;
    serde_json::from_str(&payload).map_err(|error| {
        Some(format!(
            "persistence_parse_failed path={} error={}",
            path.display(),
            error
        ))
    })
}

fn default_base_dir() -> PathBuf {
    platform_data_dir()
        .unwrap_or_else(env::temp_dir)
        .join("BatCaveMonitor")
}

fn platform_data_dir() -> Option<PathBuf> {
    #[cfg(windows)]
    {
        env::var_os("LOCALAPPDATA")
            .or_else(|| env::var_os("APPDATA"))
            .map(PathBuf::from)
    }

    #[cfg(target_os = "macos")]
    {
        env::var_os("HOME")
            .map(PathBuf::from)
            .map(|home| home.join("Library").join("Application Support"))
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    {
        env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .or_else(|| env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/share")))
    }

    #[cfg(not(any(windows, unix)))]
    {
        None
    }
}

fn round1(value: f64) -> f64 {
    (value * 10.0).round() / 10.0
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contracts::{AccessState, MetricQualityInfo, MetricSource, ProcessMetricQuality};

    #[test]
    fn shape_rows_filters_sorts_and_limits_processes() {
        let rows = shape_rows(
            &[
                sample("10", "Alpha", 1.0),
                sample("20", "Code", 3.0),
                sample("30", "Codex", 2.0),
            ],
            &RuntimeQuery {
                filter_text: "code".to_string(),
                focus_mode: ProcessFocusMode::All,
                sort_column: SortColumn::Name,
                sort_direction: SortDirection::Asc,
                limit: 1,
            },
        );

        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].name, "Code");
    }

    #[test]
    fn attention_sort_uses_attention_score_not_cpu_only() {
        let mut limited = sample("10", "Limited", 0.2);
        limited.access_state = AccessState::Partial;
        let rows = shape_rows(
            &[
                sample("20", "Quiet", 0.1),
                limited,
                sample("30", "Busy", 44.4),
            ],
            &RuntimeQuery {
                sort_column: SortColumn::Attention,
                sort_direction: SortDirection::Desc,
                ..RuntimeQuery::default()
            },
        );

        assert_eq!(rows[0].name, "Busy");
        assert_eq!(rows[1].name, "Limited");
    }

    #[test]
    fn shape_rows_applies_focus_mode_in_runtime_query() {
        let mut idle_io = sample("20", "IdleIo", 0.0);
        idle_io.disk_read_bps = 2048;
        let active = sample("30", "Active", 2.0);
        let rows = shape_rows(
            &[sample("10", "Idle", 0.0), active.clone(), idle_io.clone()],
            &RuntimeQuery {
                focus_mode: ProcessFocusMode::Active,
                sort_column: SortColumn::Name,
                sort_direction: SortDirection::Asc,
                ..RuntimeQuery::default()
            },
        );

        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].name, "Active");

        let io_rows = shape_rows(
            &[sample("10", "Idle", 0.0), active, idle_io],
            &RuntimeQuery {
                focus_mode: ProcessFocusMode::Io,
                ..RuntimeQuery::default()
            },
        );

        assert_eq!(io_rows.len(), 1);
        assert_eq!(io_rows[0].name, "IdleIo");
    }

    #[test]
    fn process_view_groups_suffixed_app_processes() {
        let mut first = sample("10", "SearchIndexer-211.exe", 12.0);
        first.exe = "C:\\Windows\\System32\\SearchIndexer-211.exe".to_string();
        first.disk_read_bps = 256;
        first.threads = 3;
        let mut second = sample("20", "SearchIndexer-223.exe", 8.0);
        second.exe = "C:\\Windows\\System32\\SearchIndexer-223.exe".to_string();
        second.disk_write_bps = 512;
        second.threads = 5;

        let rows = shape_process_view(&[first, second], &RuntimeQuery::default());

        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0].kind, ProcessViewRowKind::Group);
        assert_eq!(rows[0].group_label.as_deref(), Some("SearchIndexer.exe"));
        assert_eq!(rows[0].group_category.as_deref(), Some("Windows"));
        assert_eq!(rows[0].group_count, 2);
        assert_eq!(rows[0].cpu_percent, 20.0);
        assert_eq!(rows[0].io_bps, 768);
        assert_eq!(rows[0].threads, 8);
        assert!(rows[1].is_grouped);
        assert_eq!(rows[1].group_key, rows[0].group_key);
    }

    #[test]
    fn process_view_uses_process_name_key_when_exe_is_missing() {
        let mut first = sample("10", "Memory Compression", 12.0);
        first.exe = String::new();
        let mut second = sample("20", "Memory Compression", 8.0);
        second.exe = " ".to_string();

        let rows = shape_process_view(&[first, second], &RuntimeQuery::default());

        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0].kind, ProcessViewRowKind::Group);
        assert_eq!(rows[0].group_key.as_deref(), Some("memory compression"));
        assert!(rows[1].is_grouped);
        assert_eq!(rows[1].group_key, rows[0].group_key);
        assert_eq!(rows[2].group_key, rows[0].group_key);
    }

    #[test]
    fn process_view_keeps_singletons_as_process_rows() {
        let rows = shape_process_view(&[sample("10", "Code.exe", 12.0)], &RuntimeQuery::default());

        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].kind, ProcessViewRowKind::Process);
        assert!(!rows[0].is_grouped);
        assert_eq!(rows[0].group_label.as_deref(), Some("Code.exe"));
    }

    #[test]
    fn shape_rows_sorts_by_network_rate() {
        let quiet = sample("10", "Quiet", 1.0);
        let mut network_busy = sample("20", "NetworkBusy", 1.0);
        network_busy.network_received_bps = Some(8_000);
        network_busy.network_transmitted_bps = Some(2_000);

        let rows = shape_rows(
            &[quiet, network_busy],
            &RuntimeQuery {
                sort_column: SortColumn::NetworkBps,
                sort_direction: SortDirection::Desc,
                ..RuntimeQuery::default()
            },
        );

        assert_eq!(rows[0].name, "NetworkBusy");
    }

    #[test]
    fn process_view_sorts_group_rows_by_visible_aggregate() {
        let singleton = sample("10", "Code.exe", 70.0);
        let mut first = sample("20", "SearchIndexer-001.exe", 40.0);
        first.exe = "C:\\Windows\\System32\\SearchIndexer-001.exe".to_string();
        let mut second = sample("30", "SearchIndexer-002.exe", 35.0);
        second.exe = "C:\\Windows\\System32\\SearchIndexer-002.exe".to_string();

        let rows = shape_process_view(
            &[singleton, first, second],
            &RuntimeQuery {
                sort_column: SortColumn::CpuPct,
                sort_direction: SortDirection::Desc,
                ..RuntimeQuery::default()
            },
        );

        assert_eq!(rows[0].kind, ProcessViewRowKind::Group);
        assert_eq!(rows[0].group_label.as_deref(), Some("SearchIndexer.exe"));
        assert_eq!(rows[0].cpu_percent, 75.0);
    }

    #[test]
    fn process_view_sorts_cpu_values_numerically() {
        let larger = sample("10", "OneTwentyFour.exe", 124.0);
        let smaller = sample("20", "TwentyFive.exe", 25.0);

        let descending = shape_process_view(
            &[smaller.clone(), larger.clone()],
            &RuntimeQuery {
                sort_column: SortColumn::CpuPct,
                sort_direction: SortDirection::Desc,
                ..RuntimeQuery::default()
            },
        );
        let ascending = shape_process_view(
            &[larger, smaller],
            &RuntimeQuery {
                sort_column: SortColumn::CpuPct,
                sort_direction: SortDirection::Asc,
                ..RuntimeQuery::default()
            },
        );

        assert_eq!(descending[0].cpu_percent, 124.0);
        assert_eq!(descending[1].cpu_percent, 25.0);
        assert_eq!(ascending[0].cpu_percent, 25.0);
        assert_eq!(ascending[1].cpu_percent, 124.0);
    }

    #[test]
    fn process_rates_use_pid_and_start_time_identity() {
        let mut previous = sample("10", "Stable", 1.0);
        previous.start_time_ms = 100;
        previous.disk_read_total_bytes = 100;
        previous.disk_write_total_bytes = 50;
        previous.other_io_total_bytes = Some(10);

        let mut current = previous.clone();
        current.disk_read_total_bytes = 600;
        current.disk_write_total_bytes = 250;
        current.other_io_total_bytes = Some(110);

        let updated = add_process_rates(vec![current], &[previous], 1.0);

        assert_eq!(updated[0].disk_read_bps, 500);
        assert_eq!(updated[0].disk_write_bps, 200);
        assert_eq!(updated[0].other_io_bps, Some(100));

        let mut restarted_previous = sample("10", "Stable", 1.0);
        restarted_previous.start_time_ms = 1;
        let restarted = add_process_rates(
            vec![sample("10", "Stable", 1.0)],
            &[restarted_previous],
            1.0,
        );

        assert_eq!(restarted[0].disk_read_bps, 0);
        assert_eq!(restarted[0].other_io_bps, None);
    }

    #[test]
    fn process_memory_accounting_excludes_unavailable_placeholder_memory() {
        let mut system = empty_system();
        system.memory_used_bytes = 1_000;
        system.memory_accounting = Some(SystemMemoryAccounting {
            kernel_paged_pool_bytes: Some(128),
            ..SystemMemoryAccounting::default()
        });
        let mut reported = sample("10", "Reported", 1.0);
        reported.memory_bytes = 400;
        reported.private_bytes = 200;
        let mut blocked = sample("20", "Blocked", 1.0);
        blocked.access_state = AccessState::Denied;
        blocked.memory_bytes = 900;
        blocked.private_bytes = 800;
        blocked.quality = Some(ProcessMetricQuality {
            memory: Some(MetricQualityInfo::new(
                MetricQuality::Unavailable,
                MetricSource::DirectApi,
            )),
            ..ProcessMetricQuality::default()
        });
        let mut partial = sample("30", "Partial", 1.0);
        partial.access_state = AccessState::Partial;
        partial.memory_bytes = 100;
        partial.private_bytes = 50;

        add_process_memory_accounting(&mut system, &[reported, blocked, partial]);

        let accounting = system.memory_accounting.expect("accounting exists");
        assert_eq!(accounting.process_working_set_bytes, 500);
        assert_eq!(accounting.process_private_bytes, 250);
        assert_eq!(accounting.denied_process_count, 1);
        assert_eq!(accounting.partial_process_count, 1);
        assert_eq!(accounting.unattributed_bytes, Some(500));
        assert_eq!(accounting.kernel_paged_pool_bytes, Some(128));
    }

    #[test]
    fn attention_score_counts_network_rates() {
        let quiet = sample("10", "Quiet", 0.1);
        let mut network_busy = sample("20", "NetworkBusy", 0.1);
        network_busy.network_received_bps = Some(8 * 1024 * 1024);
        network_busy.network_transmitted_bps = Some(2 * 1024 * 1024);

        assert!(process_attention_score(&network_busy) > process_attention_score(&quiet));
    }

    #[test]
    fn settings_normalization_clears_effective_admin_and_clamps_ranges() {
        let settings = normalize_settings(RuntimeSettings {
            admin_mode_requested: true,
            admin_mode_enabled: true,
            metric_window_seconds: 1,
            query: RuntimeQuery {
                filter_text: "  code  ".to_string(),
                focus_mode: ProcessFocusMode::Active,
                limit: usize::MAX,
                ..RuntimeQuery::default()
            },
            paused: false,
        });

        assert!(!settings.admin_mode_requested);
        assert!(!settings.admin_mode_enabled);
        assert_eq!(settings.metric_window_seconds, 15);
        assert_eq!(settings.query.filter_text, "code");
        assert_eq!(settings.query.focus_mode, ProcessFocusMode::Active);
        assert_eq!(settings.query.limit, 20_000);
    }

    #[test]
    fn process_icon_allowlist_requires_live_snapshot() {
        let mut store = RuntimeStore::new();
        let trusted = sample("10", "Trusted", 1.0);
        let exe = trusted.exe.clone();
        store.settings = RuntimeSettings::default();
        store.snapshot = build_snapshot(
            1,
            now_ms(),
            &store.settings,
            RuntimeHealth::default(),
            empty_system(),
            vec![trusted],
            1,
            Vec::new(),
        );

        assert!(!store.has_process_exe(&exe));

        store.live_process_snapshot = true;

        assert!(store.has_process_exe(&exe));
    }

    #[test]
    fn persisted_admin_request_is_session_only_and_drops_warm_cache() {
        let base_dir = runtime_test_dir("admin-startup");
        let helper_dir = base_dir.join("elevated-helper");
        fs::create_dir_all(&base_dir).expect("test dir exists");
        fs::create_dir_all(&helper_dir).expect("helper dir exists");
        fs::write(
            base_dir.join(SETTINGS_FILE),
            serde_json::to_string(&RuntimeSettings {
                admin_mode_requested: true,
                ..RuntimeSettings::default()
            })
            .expect("settings serialize"),
        )
        .expect("settings fixture writes");
        fs::write(
            base_dir.join(WARM_CACHE_FILE),
            serde_json::to_string(&WarmCache {
                seq: 7,
                rows: vec![sample("10", "Elevated", 0.0)],
            })
            .expect("cache serializes"),
        )
        .expect("cache fixture writes");
        fs::write(helper_dir.join("snapshot.json"), "{}").expect("snapshot fixture writes");
        fs::write(helper_dir.join("snapshot.json.tmp"), "{}").expect("temp fixture writes");
        fs::write(helper_dir.join("stop.signal"), "stop").expect("stop fixture writes");

        let store = RuntimeStore::from_base_dir(base_dir.clone());

        assert!(!store.settings.admin_mode_requested);
        assert!(store.previous_processes.is_empty());
        assert!(store.snapshot.processes.is_empty());
        assert!(!base_dir.join(WARM_CACHE_FILE).exists());
        assert!(!helper_dir.join("snapshot.json").exists());
        assert!(!helper_dir.join("snapshot.json.tmp").exists());
        assert!(!helper_dir.join("stop.signal").exists());
        let _ = fs::remove_dir_all(base_dir);
    }

    #[test]
    fn disabling_admin_mode_purges_cached_rows_and_warm_cache() {
        let base_dir = runtime_test_dir("admin-disable");
        let helper_dir = base_dir.join("elevated-helper");
        fs::create_dir_all(&base_dir).expect("test dir exists");
        fs::create_dir_all(&helper_dir).expect("helper dir exists");
        fs::write(base_dir.join(WARM_CACHE_FILE), "{}").expect("cache fixture writes");
        fs::write(helper_dir.join("snapshot.json"), "{}").expect("snapshot fixture writes");
        fs::write(helper_dir.join("snapshot.json.tmp"), "{}").expect("temp fixture writes");
        fs::write(helper_dir.join("stop.signal"), "stop").expect("stop fixture writes");
        let mut store = RuntimeStore::from_base_dir(base_dir.clone());
        store.settings.admin_mode_requested = true;
        store.settings.admin_mode_enabled = true;
        store.previous_processes = vec![sample("10", "Elevated", 0.0)];
        fs::write(helper_dir.join("snapshot.json"), "{}").expect("snapshot fixture rewrites");
        fs::write(helper_dir.join("snapshot.json.tmp"), "{}").expect("temp fixture rewrites");
        fs::write(helper_dir.join("stop.signal"), "stop").expect("stop fixture rewrites");

        let snapshot = store.set_admin_mode(false);

        assert!(!snapshot.settings.admin_mode_requested);
        assert!(store.previous_processes.is_empty());
        assert!(snapshot.processes.is_empty());
        assert!(!base_dir.join(WARM_CACHE_FILE).exists());
        assert!(!helper_dir.join("snapshot.json").exists());
        assert!(!helper_dir.join("snapshot.json.tmp").exists());
        assert!(!helper_dir.join("stop.signal").exists());
        let _ = fs::remove_dir_all(base_dir);
    }

    #[test]
    fn admin_pending_status_does_not_degrade_without_warning() {
        let base_dir = runtime_test_dir("admin-pending");
        let mut store = RuntimeStore::from_base_dir(base_dir.clone());
        store.settings.admin_mode_requested = true;
        store.settings.admin_mode_enabled = false;
        store.warnings.clear();

        let health = store.build_health(0, 0.0, 0);

        assert!(!health.degraded);
        assert_eq!(health.collector_warnings, 0);
        assert!(health.status_summary.contains("elevation is inactive"));
        let _ = fs::remove_dir_all(base_dir);
    }

    #[test]
    fn warm_cache_is_not_written_while_admin_mode_is_enabled() {
        let base_dir = runtime_test_dir("admin-cache-skip");
        fs::create_dir_all(&base_dir).expect("test dir exists");
        let mut store = RuntimeStore::from_base_dir(base_dir.clone());
        store.settings.admin_mode_enabled = true;
        store.previous_processes = vec![sample("10", "Elevated", 0.0)];
        store.seq = 10;

        store.persist_warm_cache();

        assert!(!base_dir.join(WARM_CACHE_FILE).exists());
        let _ = fs::remove_dir_all(base_dir);
    }

    #[test]
    fn collector_warning_marks_health_degraded() {
        let mut store = RuntimeStore::new();
        let base_dir = std::env::temp_dir().join(format!(
            "batcave-runtime-health-warning-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&base_dir);
        store.base_dir = base_dir.clone();
        store.settings = RuntimeSettings::default();
        store.warnings.clear();
        store.add_warning(
            "collector",
            "network_attribution_failed:access_denied".to_string(),
        );

        let health = store.build_health(3, 0.2, 64 * 1024 * 1024);

        assert!(health.degraded);
        assert_eq!(health.collector_warnings, 1);
        assert!(health.status_summary.contains("Collector warnings"));

        let _ = fs::remove_dir_all(&base_dir);
    }

    #[test]
    fn corrupt_json_returns_persistence_parse_warning() {
        let path = std::env::temp_dir().join(format!(
            "batcave-corrupt-settings-{}.json",
            std::process::id()
        ));
        fs::write(&path, "{not-json").expect("corrupt fixture writes");

        let error = read_json::<RuntimeSettings>(&path).expect_err("corrupt json fails");

        fs::remove_file(&path).expect("corrupt fixture cleanup");
        assert!(error
            .expect("parse warning exists")
            .contains("persistence_parse_failed"));
    }

    fn runtime_test_dir(name: &str) -> PathBuf {
        let path =
            std::env::temp_dir().join(format!("batcave-runtime-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&path);
        path
    }

    fn sample(pid: &str, name: &str, cpu: f64) -> ProcessSample {
        ProcessSample {
            pid: pid.to_string(),
            parent_pid: None,
            start_time_ms: 1_700_000_000_000,
            name: name.to_string(),
            exe: format!("C:\\Program Files\\{name}.exe"),
            status: "running".to_string(),
            cpu_percent: cpu,
            kernel_cpu_percent: None,
            memory_bytes: 64 * 1024 * 1024,
            private_bytes: 32 * 1024 * 1024,
            virtual_memory_bytes: 128 * 1024 * 1024,
            disk_read_total_bytes: 0,
            disk_write_total_bytes: 0,
            other_io_total_bytes: None,
            disk_read_bps: 0,
            disk_write_bps: 0,
            other_io_bps: None,
            network_received_bps: None,
            network_transmitted_bps: None,
            threads: 1,
            handles: 1,
            access_state: AccessState::Full,
            quality: None,
        }
    }
}
