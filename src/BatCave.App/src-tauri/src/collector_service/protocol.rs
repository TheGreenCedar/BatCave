use serde::{Deserialize, Serialize};

use crate::contracts::{
    AccessState, KernelPoolKind, MetricLimitationCode, MetricQuality, MetricSource,
    RuntimeCollectorState,
};

pub(crate) const COLLECTOR_SERVICE_PROTOCOL_VERSION: u16 = 1;
pub(crate) const COLLECTOR_SNAPSHOT_SCHEMA_VERSION: u16 = 1;
pub(crate) const COLLECTOR_SERVICE_NAME: &str = "BatCaveCollector";
pub(crate) const MAX_FRAME_BYTES: usize = 8 * 1024 * 1024;
pub(crate) const MAX_PROCESS_COUNT: usize = 5_000;
pub(crate) const MAX_STRING_BYTES: usize = 32 * 1024;
pub(crate) const MAX_CLIENTS: usize = 8;
pub(crate) const MAX_WARNINGS: usize = 64;
pub(crate) const MAX_LOGICAL_CPUS: usize = 1_024;
pub(crate) const MAX_KERNEL_POOL_TAGS: usize = 4_096;
pub(crate) const MAX_DRIVER_CANDIDATES_PER_TAG: usize = 256;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub(crate) struct ReleaseIdentityV1 {
    pub app_version: String,
    pub source_commit_sha: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub(crate) struct ClientRequestV1 {
    pub protocol_version: u16,
    pub request_id: u64,
    pub operation: ClientOperationV1,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    content = "payload",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub(crate) enum ClientOperationV1 {
    Negotiate(NegotiateRequestV1),
    ServiceIdentity,
    LatestSnapshot(LatestSnapshotRequestV1),
    Ping(PingV1),
    Disconnect,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub(crate) struct NegotiateRequestV1 {
    pub minimum_protocol_version: u16,
    pub maximum_protocol_version: u16,
    pub desktop_release: ReleaseIdentityV1,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub(crate) struct LatestSnapshotRequestV1 {
    pub after_sample_seq: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub(crate) struct PingV1 {
    pub nonce: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub(crate) struct ServiceResponseV1 {
    pub protocol_version: u16,
    pub request_id: u64,
    pub outcome: ServiceOutcomeV1,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    content = "payload",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub(crate) enum ServiceOutcomeV1 {
    Negotiated(NegotiatedV1),
    ServiceIdentity(ServiceIdentityV1),
    LatestSnapshot(LatestSnapshotV1),
    Pong(PingV1),
    Disconnected,
    Error(ServiceFailureV1),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub(crate) struct NegotiatedV1 {
    pub negotiated_protocol_version: u16,
    pub service: ServiceIdentityV1,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub(crate) struct ServiceIdentityV1 {
    pub service_name: String,
    pub service_version: String,
    pub release: ReleaseIdentityV1,
    pub instance_id: String,
    pub protocol_version: u16,
    pub minimum_desktop_version: String,
    pub limits: ServiceLimitsV1,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub(crate) struct ServiceLimitsV1 {
    pub maximum_frame_bytes: u32,
    pub maximum_process_count: u32,
    pub maximum_string_bytes: u32,
    pub maximum_clients: u16,
}

impl ServiceLimitsV1 {
    pub(crate) fn contract() -> Self {
        Self {
            maximum_frame_bytes: MAX_FRAME_BYTES as u32,
            maximum_process_count: MAX_PROCESS_COUNT as u32,
            maximum_string_bytes: MAX_STRING_BYTES as u32,
            maximum_clients: MAX_CLIENTS as u16,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    content = "payload",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub(crate) enum LatestSnapshotV1 {
    Snapshot(Box<CollectorSnapshotV1>),
    Unchanged(UnchangedSnapshotV1),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub(crate) struct CollectorSnapshotV1 {
    pub snapshot_schema_version: u16,
    pub service_instance_id: String,
    pub sample_seq: u64,
    pub sampled_at_ms: u64,
    pub collection_latency_ms: u64,
    pub collector_state: RuntimeCollectorState,
    pub system: CollectorSystemV1,
    pub processes: Vec<CollectorProcessV1>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub(crate) struct CollectorSystemV1 {
    pub cpu_percent: f64,
    pub kernel_cpu_percent: f64,
    pub logical_cpu_percent: Vec<f64>,
    pub memory_used_bytes: u64,
    pub memory_total_bytes: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub memory_available_bytes: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub swap_used_bytes: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub swap_total_bytes: Option<u64>,
    pub process_count: u32,
    pub disk_read_total_bytes: u64,
    pub disk_write_total_bytes: u64,
    pub disk_read_bps: u64,
    pub disk_write_bps: u64,
    pub network_received_total_bytes: u64,
    pub network_transmitted_total_bytes: u64,
    pub network_received_bps: u64,
    pub network_transmitted_bps: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub memory_accounting: Option<CollectorMemoryAccountingV1>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quality: Option<CollectorSystemQualityV1>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub(crate) struct CollectorMemoryAccountingV1 {
    pub process_working_set_bytes: u64,
    pub process_private_bytes: u64,
    pub denied_process_count: u32,
    pub partial_process_count: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub commit_used_bytes: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub commit_limit_bytes: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub system_cache_bytes: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kernel_total_bytes: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kernel_paged_pool_bytes: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kernel_nonpaged_pool_bytes: Option<u64>,
    pub kernel_pool_tags: Vec<CollectorKernelPoolTagV1>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub(crate) struct CollectorKernelPoolTagV1 {
    pub tag: String,
    pub kind: KernelPoolKind,
    pub bytes: u64,
    pub allocations: u64,
    pub frees: u64,
    pub driver_candidates: Vec<String>,
    pub driver_candidates_pending: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub(crate) struct CollectorProcessV1 {
    pub pid: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_pid: Option<String>,
    pub start_time_ms: u64,
    pub name: String,
    pub exe: String,
    pub status: String,
    pub cpu_percent: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kernel_cpu_percent: Option<f64>,
    pub memory_bytes: u64,
    pub private_bytes: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub virtual_memory_bytes: Option<u64>,
    pub io_read_total_bytes: u64,
    pub io_write_total_bytes: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub other_io_total_bytes: Option<u64>,
    pub io_read_bps: u64,
    pub io_write_bps: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub other_io_bps: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub network_received_bps: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub network_transmitted_bps: Option<u64>,
    pub threads: u32,
    pub handles: u32,
    pub access_state: AccessState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quality: Option<CollectorProcessQualityV1>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub(crate) struct CollectorMetricQualityV1 {
    pub quality: MetricQuality,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<MetricSource>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated_at_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub age_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limitation_code: Option<MetricLimitationCode>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub(crate) struct CollectorSystemQualityV1 {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cpu: Option<CollectorMetricQualityV1>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kernel_cpu: Option<CollectorMetricQualityV1>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub logical_cpu: Option<CollectorMetricQualityV1>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub memory: Option<CollectorMetricQualityV1>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub swap: Option<CollectorMetricQualityV1>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub disk: Option<CollectorMetricQualityV1>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub network: Option<CollectorMetricQualityV1>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub(crate) struct CollectorProcessQualityV1 {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cpu: Option<CollectorMetricQualityV1>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub memory: Option<CollectorMetricQualityV1>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub io: Option<CollectorMetricQualityV1>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub other_io: Option<CollectorMetricQualityV1>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub network: Option<CollectorMetricQualityV1>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub threads: Option<CollectorMetricQualityV1>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub handles: Option<CollectorMetricQualityV1>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub(crate) struct UnchangedSnapshotV1 {
    pub service_instance_id: String,
    pub sample_seq: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub(crate) struct ServiceFailureV1 {
    pub code: ServiceFailureCodeV1,
    pub detail: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ServiceFailureCodeV1 {
    Incompatible,
    Unauthorized,
    Malformed,
    Oversized,
    StaleSequence,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ContractFailure {
    pub code: ServiceFailureCodeV1,
    pub detail: String,
}

impl ContractFailure {
    pub(crate) fn new(code: ServiceFailureCodeV1, detail: impl Into<String>) -> Self {
        Self {
            code,
            detail: detail.into(),
        }
    }

    pub(crate) fn response(&self) -> ServiceFailureV1 {
        ServiceFailureV1 {
            code: self.code,
            detail: bounded_failure_detail(&self.detail),
        }
    }
}

impl std::fmt::Display for ContractFailure {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{:?}:{}", self.code, self.detail)
    }
}

impl std::error::Error for ContractFailure {}

pub(crate) fn decode_request(payload: &[u8]) -> Result<ClientRequestV1, ContractFailure> {
    if payload.is_empty() {
        return Err(malformed("collector_service_request_empty"));
    }
    if payload.len() > MAX_FRAME_BYTES {
        return Err(oversized("collector_service_request_frame_too_large"));
    }
    let request = serde_json::from_slice::<ClientRequestV1>(payload)
        .map_err(|_| malformed("collector_service_request_malformed"))?;
    validate_request(&request)?;
    Ok(request)
}

pub(crate) fn decode_response(payload: &[u8]) -> Result<ServiceResponseV1, ContractFailure> {
    if payload.is_empty() {
        return Err(malformed("collector_service_response_empty"));
    }
    if payload.len() > MAX_FRAME_BYTES {
        return Err(oversized("collector_service_response_frame_too_large"));
    }
    let response = serde_json::from_slice::<ServiceResponseV1>(payload)
        .map_err(|_| malformed("collector_service_response_malformed"))?;
    validate_response(&response)?;
    Ok(response)
}

pub(crate) fn validate_request(request: &ClientRequestV1) -> Result<(), ContractFailure> {
    if request.request_id == 0 {
        return Err(malformed("collector_service_request_id_zero"));
    }
    match &request.operation {
        ClientOperationV1::Negotiate(hello) => {
            if hello.minimum_protocol_version == 0
                || hello.minimum_protocol_version > hello.maximum_protocol_version
            {
                return Err(malformed("collector_service_protocol_range_invalid"));
            }
            if request.protocol_version < hello.minimum_protocol_version
                || request.protocol_version > hello.maximum_protocol_version
            {
                return Err(malformed("collector_service_writer_version_outside_range"));
            }
            validate_release_identity(&hello.desktop_release)?;
        }
        _ if request.protocol_version != COLLECTOR_SERVICE_PROTOCOL_VERSION => {
            return Err(incompatible("collector_service_protocol_incompatible"));
        }
        ClientOperationV1::LatestSnapshot(request) => {
            if request.after_sample_seq == Some(u64::MAX) {
                return Err(malformed("collector_service_sample_sequence_exhausted"));
            }
        }
        ClientOperationV1::ServiceIdentity
        | ClientOperationV1::Ping(_)
        | ClientOperationV1::Disconnect => {}
    }
    Ok(())
}

pub(crate) fn validate_response(response: &ServiceResponseV1) -> Result<(), ContractFailure> {
    if response.protocol_version != COLLECTOR_SERVICE_PROTOCOL_VERSION {
        return Err(incompatible(
            "collector_service_response_protocol_incompatible",
        ));
    }
    if response.request_id == 0 {
        return Err(malformed("collector_service_response_request_id_zero"));
    }
    match &response.outcome {
        ServiceOutcomeV1::Negotiated(negotiated) => {
            if negotiated.negotiated_protocol_version != COLLECTOR_SERVICE_PROTOCOL_VERSION {
                return Err(incompatible("collector_service_negotiated_version_invalid"));
            }
            validate_service_identity(&negotiated.service)?;
        }
        ServiceOutcomeV1::ServiceIdentity(identity) => validate_service_identity(identity)?,
        ServiceOutcomeV1::LatestSnapshot(snapshot) => validate_latest_snapshot(snapshot)?,
        ServiceOutcomeV1::Pong(_) | ServiceOutcomeV1::Disconnected => {}
        ServiceOutcomeV1::Error(failure) => {
            validate_string(&failure.detail, "collector_service_failure_detail")?;
            if failure.detail.trim().is_empty() {
                return Err(malformed("collector_service_failure_detail_empty"));
            }
        }
    }
    Ok(())
}

pub(crate) fn negotiate_protocol(hello: &NegotiateRequestV1) -> Result<u16, ContractFailure> {
    if hello.minimum_protocol_version <= COLLECTOR_SERVICE_PROTOCOL_VERSION
        && hello.maximum_protocol_version >= COLLECTOR_SERVICE_PROTOCOL_VERSION
    {
        Ok(COLLECTOR_SERVICE_PROTOCOL_VERSION)
    } else {
        Err(incompatible("collector_service_protocol_incompatible"))
    }
}

pub(crate) fn validate_service_identity(
    identity: &ServiceIdentityV1,
) -> Result<(), ContractFailure> {
    if identity.service_name != COLLECTOR_SERVICE_NAME {
        return Err(malformed("collector_service_name_invalid"));
    }
    validate_nonempty_string(&identity.service_version, "collector_service_version")?;
    validate_nonempty_string(&identity.instance_id, "collector_service_instance_id")?;
    validate_nonempty_string(
        &identity.minimum_desktop_version,
        "collector_service_minimum_desktop_version",
    )?;
    validate_release_identity(&identity.release)?;
    if identity.protocol_version != COLLECTOR_SERVICE_PROTOCOL_VERSION {
        return Err(incompatible(
            "collector_service_identity_protocol_incompatible",
        ));
    }
    if identity.limits != ServiceLimitsV1::contract() {
        return Err(malformed("collector_service_limits_mismatch"));
    }
    Ok(())
}

pub(crate) fn validate_release_identity(
    identity: &ReleaseIdentityV1,
) -> Result<(), ContractFailure> {
    validate_nonempty_string(&identity.app_version, "collector_service_release_version")?;
    if let Some(commit) = &identity.source_commit_sha {
        if commit.len() != 40 || !commit.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(malformed("collector_service_release_commit_invalid"));
        }
    }
    Ok(())
}

pub(crate) fn validate_latest_snapshot(latest: &LatestSnapshotV1) -> Result<(), ContractFailure> {
    match latest {
        LatestSnapshotV1::Snapshot(snapshot) => validate_snapshot(snapshot),
        LatestSnapshotV1::Unchanged(snapshot) => {
            validate_nonempty_string(
                &snapshot.service_instance_id,
                "collector_service_snapshot_instance_id",
            )?;
            if snapshot.sample_seq == 0 {
                return Err(malformed("collector_service_sample_sequence_zero"));
            }
            Ok(())
        }
    }
}

pub(crate) fn validate_snapshot(snapshot: &CollectorSnapshotV1) -> Result<(), ContractFailure> {
    if snapshot.snapshot_schema_version != COLLECTOR_SNAPSHOT_SCHEMA_VERSION {
        return Err(incompatible(
            "collector_service_snapshot_schema_incompatible",
        ));
    }
    validate_nonempty_string(
        &snapshot.service_instance_id,
        "collector_service_snapshot_instance_id",
    )?;
    if snapshot.sample_seq == 0 {
        return Err(malformed("collector_service_sample_sequence_zero"));
    }
    if snapshot.processes.len() > MAX_PROCESS_COUNT {
        return Err(oversized("collector_service_process_limit_exceeded"));
    }
    if usize::try_from(snapshot.system.process_count).ok() != Some(snapshot.processes.len()) {
        return Err(malformed("collector_service_process_count_mismatch"));
    }
    if snapshot.system.logical_cpu_percent.len() > MAX_LOGICAL_CPUS {
        return Err(oversized("collector_service_logical_cpu_limit_exceeded"));
    }
    if snapshot.warnings.len() > MAX_WARNINGS {
        return Err(oversized("collector_service_warning_limit_exceeded"));
    }
    validate_system(&snapshot.system)?;
    for process in &snapshot.processes {
        validate_process(process)?;
    }
    for warning in &snapshot.warnings {
        validate_string(warning, "collector_service_warning")?;
    }
    Ok(())
}

fn validate_system(system: &CollectorSystemV1) -> Result<(), ContractFailure> {
    for value in std::iter::once(system.cpu_percent)
        .chain(std::iter::once(system.kernel_cpu_percent))
        .chain(system.logical_cpu_percent.iter().copied())
    {
        validate_nonnegative_number(value, "collector_service_system_percent")?;
    }
    if let Some(accounting) = &system.memory_accounting {
        if accounting.kernel_pool_tags.len() > MAX_KERNEL_POOL_TAGS {
            return Err(oversized("collector_service_kernel_pool_limit_exceeded"));
        }
        for tag in &accounting.kernel_pool_tags {
            validate_string(&tag.tag, "collector_service_kernel_pool_tag")?;
            if tag.driver_candidates.len() > MAX_DRIVER_CANDIDATES_PER_TAG {
                return Err(oversized(
                    "collector_service_driver_candidate_limit_exceeded",
                ));
            }
            for candidate in &tag.driver_candidates {
                validate_string(candidate, "collector_service_driver_candidate")?;
            }
        }
    }
    if let Some(quality) = &system.quality {
        validate_system_quality(quality)?;
    }
    Ok(())
}

fn validate_process(process: &CollectorProcessV1) -> Result<(), ContractFailure> {
    validate_nonempty_string(&process.pid, "collector_service_process_pid")?;
    if let Some(parent_pid) = &process.parent_pid {
        validate_nonempty_string(parent_pid, "collector_service_process_parent_pid")?;
    }
    validate_nonempty_string(&process.name, "collector_service_process_name")?;
    validate_string(&process.exe, "collector_service_process_exe")?;
    validate_nonempty_string(&process.status, "collector_service_process_status")?;
    validate_nonnegative_number(process.cpu_percent, "collector_service_process_cpu")?;
    if let Some(value) = process.kernel_cpu_percent {
        validate_nonnegative_number(value, "collector_service_process_kernel_cpu")?;
    }
    if let Some(quality) = &process.quality {
        validate_process_quality(quality)?;
    }
    Ok(())
}

fn validate_system_quality(quality: &CollectorSystemQualityV1) -> Result<(), ContractFailure> {
    for value in [
        quality.cpu.as_ref(),
        quality.kernel_cpu.as_ref(),
        quality.logical_cpu.as_ref(),
        quality.memory.as_ref(),
        quality.swap.as_ref(),
        quality.disk.as_ref(),
        quality.network.as_ref(),
    ]
    .into_iter()
    .flatten()
    {
        validate_metric_quality(value)?;
    }
    Ok(())
}

fn validate_process_quality(quality: &CollectorProcessQualityV1) -> Result<(), ContractFailure> {
    for value in [
        quality.cpu.as_ref(),
        quality.memory.as_ref(),
        quality.io.as_ref(),
        quality.other_io.as_ref(),
        quality.network.as_ref(),
        quality.threads.as_ref(),
        quality.handles.as_ref(),
    ]
    .into_iter()
    .flatten()
    {
        validate_metric_quality(value)?;
    }
    Ok(())
}

fn validate_metric_quality(quality: &CollectorMetricQualityV1) -> Result<(), ContractFailure> {
    if let Some(message) = &quality.message {
        validate_nonempty_string(message, "collector_service_quality_message")?;
    }
    Ok(())
}

fn validate_nonnegative_number(value: f64, field: &str) -> Result<(), ContractFailure> {
    if !value.is_finite() || value < 0.0 {
        return Err(malformed(format!("{field}_invalid")));
    }
    Ok(())
}

fn validate_nonempty_string(value: &str, field: &str) -> Result<(), ContractFailure> {
    validate_string(value, field)?;
    if value.trim().is_empty() {
        return Err(malformed(format!("{field}_empty")));
    }
    Ok(())
}

fn validate_string(value: &str, field: &str) -> Result<(), ContractFailure> {
    if value.len() > MAX_STRING_BYTES {
        return Err(oversized(format!("{field}_too_large")));
    }
    Ok(())
}

fn bounded_failure_detail(value: &str) -> String {
    if value.len() <= MAX_STRING_BYTES {
        return value.to_string();
    }
    let mut end = MAX_STRING_BYTES;
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    value[..end].to_string()
}

pub(crate) fn incompatible(detail: impl Into<String>) -> ContractFailure {
    ContractFailure::new(ServiceFailureCodeV1::Incompatible, detail)
}

pub(crate) fn unauthorized(detail: impl Into<String>) -> ContractFailure {
    ContractFailure::new(ServiceFailureCodeV1::Unauthorized, detail)
}

pub(crate) fn malformed(detail: impl Into<String>) -> ContractFailure {
    ContractFailure::new(ServiceFailureCodeV1::Malformed, detail)
}

pub(crate) fn oversized(detail: impl Into<String>) -> ContractFailure {
    ContractFailure::new(ServiceFailureCodeV1::Oversized, detail)
}

pub(crate) fn stale_sequence(detail: impl Into<String>) -> ContractFailure {
    ContractFailure::new(ServiceFailureCodeV1::StaleSequence, detail)
}

#[derive(Debug, Clone)]
pub(crate) struct SnapshotSequence {
    service_instance_id: String,
    last_sample_seq: Option<u64>,
}

impl SnapshotSequence {
    pub(crate) fn new(service: &ServiceIdentityV1) -> Result<Self, ContractFailure> {
        validate_service_identity(service)?;
        Ok(Self {
            service_instance_id: service.instance_id.clone(),
            last_sample_seq: None,
        })
    }

    pub(crate) fn accept(
        &mut self,
        requested_after: Option<u64>,
        latest: &LatestSnapshotV1,
    ) -> Result<(), ContractFailure> {
        validate_latest_snapshot(latest)?;
        match latest {
            LatestSnapshotV1::Snapshot(snapshot) => {
                self.require_instance(&snapshot.service_instance_id)?;
                if requested_after.is_some_and(|after| snapshot.sample_seq <= after)
                    || self
                        .last_sample_seq
                        .is_some_and(|previous| snapshot.sample_seq <= previous)
                {
                    return Err(stale_sequence("collector_service_sample_sequence_stale"));
                }
                self.last_sample_seq = Some(snapshot.sample_seq);
            }
            LatestSnapshotV1::Unchanged(snapshot) => {
                self.require_instance(&snapshot.service_instance_id)?;
                if requested_after != Some(snapshot.sample_seq)
                    || self.last_sample_seq != Some(snapshot.sample_seq)
                {
                    return Err(stale_sequence(
                        "collector_service_unchanged_sequence_invalid",
                    ));
                }
            }
        }
        Ok(())
    }

    fn require_instance(&self, instance_id: &str) -> Result<(), ContractFailure> {
        if instance_id != self.service_instance_id {
            return Err(stale_sequence(
                "collector_service_snapshot_instance_mismatch",
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NEGOTIATE_REQUEST: &str =
        include_str!("../fixtures/collector-service-v1/negotiate-request.json");
    const NEGOTIATE_RESPONSE: &str =
        include_str!("../fixtures/collector-service-v1/negotiate-response.json");
    const SNAPSHOT_RESPONSE: &str =
        include_str!("../fixtures/collector-service-v1/snapshot-response.json");

    #[test]
    fn checked_protocol_fixtures_round_trip_exactly() {
        let request = decode_request(NEGOTIATE_REQUEST.as_bytes()).unwrap();
        let response = decode_response(NEGOTIATE_RESPONSE.as_bytes()).unwrap();
        let snapshot = decode_response(SNAPSHOT_RESPONSE.as_bytes()).unwrap();

        assert_eq!(
            serde_json::to_value(request).unwrap(),
            serde_json::from_str::<serde_json::Value>(NEGOTIATE_REQUEST).unwrap()
        );
        assert_eq!(
            serde_json::to_value(response).unwrap(),
            serde_json::from_str::<serde_json::Value>(NEGOTIATE_RESPONSE).unwrap()
        );
        assert_eq!(
            serde_json::to_value(snapshot).unwrap(),
            serde_json::from_str::<serde_json::Value>(SNAPSHOT_RESPONSE).unwrap()
        );
    }

    #[test]
    fn unknown_and_mutating_operations_are_structurally_rejected() {
        for kind in [
            "set_query",
            "set_sample_interval",
            "pause",
            "resume",
            "read_file",
            "write_file",
            "launch_process",
            "run_command",
            "stop_service",
        ] {
            let payload = serde_json::json!({
                "protocol_version": 1,
                "request_id": 1,
                "operation": { "kind": kind, "payload": {} }
            });
            assert_eq!(
                decode_request(&serde_json::to_vec(&payload).unwrap())
                    .unwrap_err()
                    .code,
                ServiceFailureCodeV1::Malformed,
                "operation {kind} must be rejected"
            );
        }
    }

    #[test]
    fn client_cannot_claim_transport_process_or_session_identity() {
        let mut payload = serde_json::from_str::<serde_json::Value>(NEGOTIATE_REQUEST).unwrap();
        let hello = payload["operation"]["payload"]
            .as_object_mut()
            .expect("negotiation payload is an object");
        hello.insert("process_id".to_string(), serde_json::json!(1234));
        hello.insert("session_id".to_string(), serde_json::json!(2));
        assert_eq!(
            decode_request(&serde_json::to_vec(&payload).unwrap())
                .unwrap_err()
                .code,
            ServiceFailureCodeV1::Malformed
        );
    }

    #[test]
    fn snapshot_wire_types_reject_nested_unknown_fields() {
        let mut payload = serde_json::from_str::<serde_json::Value>(SNAPSHOT_RESPONSE).unwrap();
        let process = payload["outcome"]["payload"]["payload"]["processes"][0]
            .as_object_mut()
            .expect("snapshot process is an object");
        process.insert("run_command".to_string(), serde_json::json!("whoami"));

        assert_eq!(
            decode_response(&serde_json::to_vec(&payload).unwrap())
                .unwrap_err()
                .code,
            ServiceFailureCodeV1::Malformed
        );
    }

    #[test]
    fn incompatible_ranges_and_versions_are_structured_failures() {
        let hello = NegotiateRequestV1 {
            minimum_protocol_version: 2,
            maximum_protocol_version: 3,
            desktop_release: release(),
        };
        let failure = negotiate_protocol(&hello).unwrap_err();
        assert_eq!(failure.code, ServiceFailureCodeV1::Incompatible);
        assert_eq!(failure.response().code, ServiceFailureCodeV1::Incompatible);

        let request = ClientRequestV1 {
            protocol_version: 2,
            request_id: 1,
            operation: ClientOperationV1::Ping(PingV1 { nonce: 1 }),
        };
        assert_eq!(
            validate_request(&request).unwrap_err().code,
            ServiceFailureCodeV1::Incompatible
        );
    }

    #[test]
    fn frame_process_string_and_client_limits_are_explicit_contract_values() {
        assert_eq!(ServiceLimitsV1::contract().maximum_frame_bytes, 8_388_608);
        assert_eq!(ServiceLimitsV1::contract().maximum_process_count, 5_000);
        assert_eq!(ServiceLimitsV1::contract().maximum_string_bytes, 32_768);
        assert_eq!(ServiceLimitsV1::contract().maximum_clients, 8);

        let mut identity = service_identity("instance-1");
        validate_service_identity(&identity).unwrap();
        identity.limits.maximum_clients += 1;
        assert_eq!(
            validate_service_identity(&identity).unwrap_err().code,
            ServiceFailureCodeV1::Malformed
        );
    }

    #[test]
    fn process_and_string_bounds_fail_closed() {
        let mut snapshot = sample_snapshot("instance-1", 1);
        snapshot.processes = vec![sample_process(); MAX_PROCESS_COUNT + 1];
        snapshot.system.process_count = snapshot.processes.len() as u32;
        assert_eq!(
            validate_snapshot(&snapshot).unwrap_err().code,
            ServiceFailureCodeV1::Oversized
        );

        let mut snapshot = sample_snapshot("instance-1", 1);
        snapshot.processes[0].name = "x".repeat(MAX_STRING_BYTES + 1);
        assert_eq!(
            validate_snapshot(&snapshot).unwrap_err().code,
            ServiceFailureCodeV1::Oversized
        );

        let mut snapshot = sample_snapshot("instance-1", 1);
        snapshot.system.process_count = 99;
        assert_eq!(
            validate_snapshot(&snapshot).unwrap_err().code,
            ServiceFailureCodeV1::Malformed
        );
    }

    #[test]
    fn nonfinite_or_negative_measurements_are_rejected() {
        let mut snapshot = sample_snapshot("instance-1", 1);
        snapshot.system.cpu_percent = f64::INFINITY;
        assert_eq!(
            validate_snapshot(&snapshot).unwrap_err().code,
            ServiceFailureCodeV1::Malformed
        );

        let mut snapshot = sample_snapshot("instance-1", 1);
        snapshot.processes[0].cpu_percent = -1.0;
        assert_eq!(
            validate_snapshot(&snapshot).unwrap_err().code,
            ServiceFailureCodeV1::Malformed
        );
    }

    #[test]
    fn sample_sequence_rejects_duplicates_regressions_and_old_instances() {
        let service = service_identity("instance-1");
        let mut sequence = SnapshotSequence::new(&service).unwrap();
        sequence
            .accept(
                None,
                &LatestSnapshotV1::Snapshot(Box::new(sample_snapshot("instance-1", 4))),
            )
            .unwrap();

        for sequence_value in [4, 3] {
            assert_eq!(
                sequence
                    .accept(
                        Some(4),
                        &LatestSnapshotV1::Snapshot(Box::new(sample_snapshot(
                            "instance-1",
                            sequence_value,
                        ))),
                    )
                    .unwrap_err()
                    .code,
                ServiceFailureCodeV1::StaleSequence
            );
        }
        assert_eq!(
            sequence
                .accept(
                    Some(4),
                    &LatestSnapshotV1::Snapshot(Box::new(sample_snapshot("old-instance", 5))),
                )
                .unwrap_err()
                .code,
            ServiceFailureCodeV1::StaleSequence
        );

        sequence
            .accept(
                Some(4),
                &LatestSnapshotV1::Snapshot(Box::new(sample_snapshot("instance-1", 5))),
            )
            .unwrap();
    }

    #[test]
    fn unchanged_response_is_valid_only_for_the_exact_observed_sequence() {
        let service = service_identity("instance-1");
        let mut sequence = SnapshotSequence::new(&service).unwrap();
        sequence
            .accept(
                None,
                &LatestSnapshotV1::Snapshot(Box::new(sample_snapshot("instance-1", 4))),
            )
            .unwrap();
        sequence
            .accept(
                Some(4),
                &LatestSnapshotV1::Unchanged(UnchangedSnapshotV1 {
                    service_instance_id: "instance-1".to_string(),
                    sample_seq: 4,
                }),
            )
            .unwrap();

        assert_eq!(
            sequence
                .accept(
                    Some(3),
                    &LatestSnapshotV1::Unchanged(UnchangedSnapshotV1 {
                        service_instance_id: "instance-1".to_string(),
                        sample_seq: 3,
                    }),
                )
                .unwrap_err()
                .code,
            ServiceFailureCodeV1::StaleSequence
        );
    }

    #[test]
    fn oversized_failure_details_are_truncated_on_a_utf8_boundary() {
        let detail = "🦇".repeat(MAX_STRING_BYTES);
        let response = ContractFailure::new(ServiceFailureCodeV1::Malformed, detail).response();
        assert!(response.detail.len() <= MAX_STRING_BYTES);
        assert!(response.detail.is_char_boundary(response.detail.len()));
    }

    fn service_identity(instance_id: &str) -> ServiceIdentityV1 {
        ServiceIdentityV1 {
            service_name: COLLECTOR_SERVICE_NAME.to_string(),
            service_version: "0.2.0-rc.2".to_string(),
            release: release(),
            instance_id: instance_id.to_string(),
            protocol_version: COLLECTOR_SERVICE_PROTOCOL_VERSION,
            minimum_desktop_version: "0.2.0-rc.2".to_string(),
            limits: ServiceLimitsV1::contract(),
        }
    }

    fn release() -> ReleaseIdentityV1 {
        ReleaseIdentityV1 {
            app_version: "0.2.0-rc.2".to_string(),
            source_commit_sha: Some("a".repeat(40)),
        }
    }

    fn sample_snapshot(instance_id: &str, sample_seq: u64) -> CollectorSnapshotV1 {
        CollectorSnapshotV1 {
            snapshot_schema_version: COLLECTOR_SNAPSHOT_SCHEMA_VERSION,
            service_instance_id: instance_id.to_string(),
            sample_seq,
            sampled_at_ms: 1_700_000_000_000,
            collection_latency_ms: 10,
            collector_state: RuntimeCollectorState::Healthy,
            system: system(1),
            processes: vec![sample_process()],
            warnings: Vec::new(),
        }
    }

    fn system(process_count: u32) -> CollectorSystemV1 {
        CollectorSystemV1 {
            cpu_percent: 10.0,
            kernel_cpu_percent: 2.0,
            logical_cpu_percent: vec![10.0, 11.0],
            memory_used_bytes: 1_000,
            memory_total_bytes: 2_000,
            memory_available_bytes: Some(1_000),
            swap_used_bytes: Some(0),
            swap_total_bytes: Some(0),
            process_count,
            disk_read_total_bytes: 10,
            disk_write_total_bytes: 20,
            disk_read_bps: 1,
            disk_write_bps: 2,
            network_received_total_bytes: 30,
            network_transmitted_total_bytes: 40,
            network_received_bps: 3,
            network_transmitted_bps: 4,
            memory_accounting: None,
            quality: None,
        }
    }

    fn sample_process() -> CollectorProcessV1 {
        CollectorProcessV1 {
            pid: "1234".to_string(),
            parent_pid: Some("1".to_string()),
            start_time_ms: 1_700_000_000_000,
            name: "batcave-monitor".to_string(),
            exe: r"C:\Program Files\BatCave Monitor\batcave-monitor.exe".to_string(),
            status: "Run".to_string(),
            cpu_percent: 1.0,
            kernel_cpu_percent: Some(0.2),
            memory_bytes: 100,
            private_bytes: 90,
            virtual_memory_bytes: Some(200),
            io_read_total_bytes: 10,
            io_write_total_bytes: 20,
            other_io_total_bytes: Some(5),
            io_read_bps: 1,
            io_write_bps: 2,
            other_io_bps: Some(1),
            network_received_bps: Some(3),
            network_transmitted_bps: Some(4),
            threads: 5,
            handles: 6,
            access_state: AccessState::Full,
            quality: None,
        }
    }
}
