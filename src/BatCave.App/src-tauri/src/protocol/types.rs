use serde::{Deserialize, Serialize};
#[cfg(test)]
use ts_rs::TS;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(test, derive(TS))]
#[serde(rename_all = "snake_case")]
pub struct ProtocolEnvelope {
    pub protocol_version: u16,
    pub compatibility: Compatibility,
    pub event: ProtocolEvent,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(test, derive(TS))]
#[serde(rename_all = "snake_case")]
pub struct Compatibility {
    pub minimum_reader_version: u16,
    pub breaking: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(test, derive(TS))]
#[serde(tag = "kind", content = "payload", rename_all = "snake_case")]
pub enum ProtocolEvent {
    RuntimeSnapshot(Box<RuntimeSnapshotPayloadV4>),
    ProtocolMismatch(ProtocolMismatchPayload),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(test, derive(TS))]
#[serde(rename_all = "snake_case")]
pub struct ProtocolMismatchPayload {
    pub reason: ProtocolMismatchReason,
    pub writer_version: u16,
    pub minimum_reader_version: u16,
    pub message: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[cfg_attr(test, derive(TS))]
#[serde(rename_all = "snake_case")]
pub enum ProtocolMismatchReason {
    LegacyWriter,
    ReaderTooOld,
    BreakingWriter,
    MalformedPayload,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(test, derive(TS))]
#[serde(rename_all = "snake_case")]
pub struct RuntimeSnapshotPayloadV4 {
    #[cfg_attr(test, ts(type = "number"))]
    pub publication_seq: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub published_at_ms: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub sample_seq: u64,
    #[cfg_attr(test, ts(type = "number | null"))]
    pub sampled_at_ms: Option<u64>,
    pub source: String,
    pub environment: RuntimeEnvironmentV4,
    pub privileged_collection: RuntimePrivilegedCollectionV4,
    pub settings: RuntimeSettingsV4,
    pub health: RuntimeHealthV4,
    pub persistence: Option<RuntimePersistenceV4>,
    pub descriptors: Vec<MeasurementDescriptor>,
    pub quality_codes: Vec<MetricQualityV4>,
    pub limitations: Vec<LimitationEntry>,
    pub system: SystemDetailV4,
    pub workloads: Vec<WorkloadDetailV4>,
    pub overview_workloads: Vec<WorkloadDetailV4>,
    pub contributors: Vec<ProcessContributorV4>,
    pub total_process_count: u32,
    pub visible_process_count: u32,
    pub warnings: Vec<RuntimeWarningV4>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(test, derive(TS))]
#[serde(rename_all = "snake_case")]
pub struct RuntimeEnvironmentV4 {
    pub platform: RuntimePlatformV4,
    pub architecture: RuntimeArchitectureV4,
    pub process_elevation: RuntimeProcessElevationV4,
    pub install_kind: RuntimeInstallKindV4,
    pub data_directory: Option<String>,
    pub release_identity: RuntimeReleaseIdentityV4,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(TS))]
#[serde(rename_all = "snake_case")]
pub struct RuntimeReleaseIdentityV4 {
    pub app_version: String,
    pub source_commit_sha: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(TS))]
#[serde(rename_all = "snake_case")]
pub enum RuntimeArchitectureV4 {
    X86_64,
    Aarch64,
    X86,
    Unknown,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[cfg_attr(test, derive(TS))]
#[serde(rename_all = "snake_case")]
pub enum RuntimePlatformV4 {
    Windows,
    Linux,
    Macos,
    Fixture,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[cfg_attr(test, derive(TS))]
#[serde(rename_all = "snake_case")]
pub enum RuntimeProcessElevationV4 {
    Unknown,
    Standard,
    Elevated,
    NotApplicable,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[cfg_attr(test, derive(TS))]
#[serde(rename_all = "snake_case")]
pub enum RuntimeInstallKindV4 {
    Unknown,
    Nsis,
    Appimage,
    Deb,
    Dmg,
    AppBundle,
    Portable,
    Development,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(test, derive(TS))]
#[serde(rename_all = "snake_case")]
pub struct RuntimePrivilegedCollectionV4 {
    pub state: PrivilegedCollectionStateV4,
    pub source: PrivilegedCollectionSourceV4,
    pub preference: PrivilegedCollectionPreferenceV4,
    pub standard_fallback_process_etw_disabled: bool,
    pub detail: Option<String>,
    #[cfg_attr(test, ts(type = "number | null"))]
    pub last_success_at_ms: Option<u64>,
    pub collector_service: Option<CollectorServiceStatusV4>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[cfg_attr(test, derive(TS))]
#[serde(rename_all = "snake_case")]
pub enum PrivilegedCollectionStateV4 {
    Unavailable,
    StandardOnly,
    Connecting,
    Active,
    Recovering,
    Failed,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[cfg_attr(test, derive(TS))]
#[serde(rename_all = "snake_case")]
pub enum PrivilegedCollectionSourceV4 {
    None,
    LocalProcess,
    CollectorService,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(TS))]
#[serde(rename_all = "snake_case")]
pub enum PrivilegedCollectionPreferenceV4 {
    StandardOnly,
    BestAvailable,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(test, derive(TS))]
#[serde(rename_all = "snake_case")]
pub struct CollectorServiceStatusV4 {
    pub state: CollectorServiceStateV4,
    pub release_identity: Option<RuntimeReleaseIdentityV4>,
    pub service_version: Option<String>,
    pub negotiated_protocol_version: Option<u16>,
    pub minimum_desktop_version: Option<String>,
    pub instance_id: Option<String>,
    #[cfg_attr(test, ts(type = "number | null"))]
    pub last_connected_at_ms: Option<u64>,
    pub detail: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(TS))]
#[serde(rename_all = "snake_case")]
pub enum CollectorServiceStateV4 {
    NotInstalled,
    Stopped,
    Connecting,
    Recovering,
    Active,
    Incompatible,
    Unauthorized,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(test, derive(TS))]
#[serde(rename_all = "snake_case")]
pub struct RuntimeSettingsV4 {
    pub query: RuntimeQueryV4,
    pub metric_window_seconds: u32,
    pub effective_sample_interval_ms: u32,
    pub collection_paused: bool,
    pub ui_preferences: Option<RuntimeUiPreferencesV4>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(test, derive(TS))]
#[serde(rename_all = "snake_case")]
pub struct RuntimeUiPreferencesV4 {
    pub theme: String,
    pub history_point_limit: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(test, derive(TS))]
#[serde(rename_all = "snake_case")]
pub struct RuntimeQueryInputV4 {
    pub filter_text: String,
    pub focus_mode: ProcessFocusModeV4,
    pub sort_column: SortColumnV4,
    pub sort_direction: SortDirectionV4,
    pub limit: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(test, derive(TS))]
#[serde(rename_all = "snake_case")]
pub struct RuntimeQueryV4 {
    pub filter_text: String,
    pub focus_mode: ProcessFocusModeV4,
    pub sort_column: SortColumnV4,
    pub sort_direction: SortDirectionV4,
    pub limit: u32,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[cfg_attr(test, derive(TS))]
#[serde(rename_all = "snake_case")]
pub enum ProcessFocusModeV4 {
    All,
    Attention,
    Io,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[cfg_attr(test, derive(TS))]
#[serde(rename_all = "snake_case")]
pub enum SortColumnV4 {
    Attention,
    Name,
    Pid,
    CpuPct,
    MemoryBytes,
    IoBps,
    NetworkBps,
    Threads,
    Handles,
    StartTimeMs,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[cfg_attr(test, derive(TS))]
#[serde(rename_all = "snake_case")]
pub enum SortDirectionV4 {
    Asc,
    Desc,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(test, derive(TS))]
#[serde(rename_all = "snake_case")]
pub struct RuntimeHealthV4 {
    pub engine_state: Option<RuntimeEngineStateV4>,
    pub collector_state: Option<RuntimeCollectorStateV4>,
    pub degraded: bool,
    pub freshness: crate::contracts::RuntimeFreshness,
    pub reason_codes: Vec<crate::contracts::RuntimeHealthReason>,
    pub status_summary: String,
    #[cfg_attr(test, ts(type = "number"))]
    pub evaluated_at_ms: u64,
    #[cfg_attr(test, ts(type = "number | null"))]
    pub last_heartbeat_at_ms: Option<u64>,
    #[cfg_attr(test, ts(type = "number | null"))]
    pub heartbeat_age_ms: Option<u64>,
    #[cfg_attr(test, ts(type = "number"))]
    pub publication_age_ms: u64,
    #[cfg_attr(test, ts(type = "number | null"))]
    pub sample_age_ms: Option<u64>,
    #[cfg_attr(test, ts(type = "number | null"))]
    pub deadline_misses: Option<u64>,
    pub deadline_lateness_p95_ms: Option<f64>,
    pub collection_latency_ms: Option<f64>,
    pub collection_p95_ms: Option<f64>,
    pub publication_latency_ms: Option<f64>,
    pub publication_p95_ms: Option<f64>,
    pub collector_warning_count: u32,
    pub app_cpu_percent: f64,
    #[cfg_attr(test, ts(type = "number"))]
    pub app_rss_bytes: u64,
    pub last_warning: Option<String>,
    pub fatal_error: Option<RuntimeFatalErrorV4>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(TS))]
#[serde(rename_all = "snake_case")]
pub enum RuntimeEngineStateV4 {
    Starting,
    Running,
    Paused,
    Fatal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(TS))]
#[serde(rename_all = "snake_case")]
pub enum RuntimeCollectorStateV4 {
    Healthy,
    Limited,
    Unavailable,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(test, derive(TS))]
#[serde(rename_all = "snake_case")]
pub struct RuntimeFatalErrorV4 {
    pub code: String,
    pub message: String,
    #[cfg_attr(test, ts(type = "number"))]
    pub occurred_at_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(test, derive(TS))]
#[serde(rename_all = "snake_case")]
pub struct RuntimePersistenceV4 {
    pub state: RuntimePersistenceStateV4,
    pub roots: Vec<RuntimePersistenceRootV4>,
    pub components: Vec<RuntimePersistenceComponentV4>,
    #[cfg_attr(test, ts(type = "number"))]
    pub suppressed_diagnostic_events: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(TS))]
#[serde(rename_all = "snake_case")]
pub enum RuntimePersistenceStateV4 {
    Healthy,
    Degraded,
    Unavailable,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(test, derive(TS))]
#[serde(rename_all = "snake_case")]
pub struct RuntimePersistenceRootV4 {
    pub owner: RuntimePersistenceOwnerV4,
    pub directory: Option<String>,
    pub permission_state: RuntimePersistencePermissionStateV4,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(test, derive(TS))]
#[serde(rename_all = "snake_case")]
pub enum RuntimePersistenceOwnerV4 {
    CurrentUser,
    CollectorService,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(TS))]
#[serde(rename_all = "snake_case")]
pub enum RuntimePersistencePermissionStateV4 {
    Verified,
    Invalid,
    Unavailable,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(test, derive(TS))]
#[serde(rename_all = "snake_case")]
pub struct RuntimePersistenceComponentV4 {
    pub owner: RuntimePersistenceOwnerV4,
    pub kind: RuntimePersistenceKindV4,
    pub state: RuntimePersistenceStateV4,
    pub durability: RuntimePersistenceDurabilityV4,
    #[cfg_attr(test, ts(type = "number | null"))]
    pub last_success_at_ms: Option<u64>,
    pub active_failure: Option<RuntimePersistenceFailureV4>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(test, derive(TS))]
#[serde(rename_all = "snake_case")]
pub enum RuntimePersistenceKindV4 {
    Settings,
    WarmCache,
    Diagnostics,
    ServiceState,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(TS))]
#[serde(rename_all = "snake_case")]
pub enum RuntimePersistenceDurabilityV4 {
    Durable,
    NotWritten,
    SessionOnly,
    NotApplicable,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(test, derive(TS))]
#[serde(rename_all = "snake_case")]
pub struct RuntimePersistenceFailureV4 {
    pub code: String,
    pub operation: RuntimePersistenceOperationV4,
    #[cfg_attr(test, ts(type = "number"))]
    pub occurred_at_ms: u64,
    pub retryable: bool,
    pub summary: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(TS))]
#[serde(rename_all = "snake_case")]
pub enum RuntimePersistenceOperationV4 {
    ResolveRoot,
    Create,
    Load,
    Parse,
    Migrate,
    Serialize,
    Write,
    Sync,
    Replace,
    Rotate,
    Remove,
    Permissions,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(test, derive(TS))]
#[serde(rename_all = "snake_case")]
pub struct MeasurementDescriptor {
    pub id: u16,
    pub semantic: MetricSemantic,
    pub scope: MetricScope,
    pub unit: MetricUnit,
    pub interval_ms: Option<u32>,
    pub network_scope: Option<NetworkScopeV4>,
    pub source: MetricSourceV4,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(TS))]
#[serde(rename_all = "snake_case")]
pub enum NetworkScopeV4 {
    NonLoopbackInterfaceAggregate,
    AllInterfaceAggregate,
    IpSocketPayload,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[cfg_attr(test, derive(TS))]
#[serde(rename_all = "snake_case")]
pub enum MetricSemantic {
    CpuUsage,
    KernelCpuUsage,
    LogicalCpuUsage,
    ResidentMemory,
    PrivateMemory,
    VirtualMemory,
    MemoryUsed,
    MemoryCapacity,
    MemoryAvailable,
    SwapUsed,
    SwapCapacity,
    ProcessWorkingSetMemory,
    ProcessPrivateMemory,
    DeniedProcessCount,
    PartialProcessCount,
    CommitUsed,
    CommitLimit,
    SystemCache,
    KernelMemory,
    KernelPagedPool,
    KernelNonpagedPool,
    KernelPoolBytes,
    KernelPoolAllocations,
    KernelPoolFrees,
    PhysicalDiskReadTotal,
    PhysicalDiskWriteTotal,
    PhysicalDiskReadRate,
    PhysicalDiskWriteRate,
    ReadIoTotal,
    WriteIoTotal,
    OtherIoTotal,
    ReadIoRate,
    WriteIoRate,
    OtherIoRate,
    ReadWriteIoRate,
    NetworkReceiveTotal,
    NetworkTransmitTotal,
    NetworkReceiveRate,
    NetworkTransmitRate,
    NetworkRate,
    ProcessCount,
    ThreadCount,
    HandleCount,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[cfg_attr(test, derive(TS))]
#[serde(rename_all = "snake_case")]
pub enum MetricScope {
    System,
    Process,
    Group,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[cfg_attr(test, derive(TS))]
#[serde(rename_all = "snake_case")]
pub enum MetricUnit {
    PercentOneCore,
    PercentSystem,
    Bytes,
    BytesPerSecond,
    Count,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[cfg_attr(test, derive(TS))]
#[serde(rename_all = "snake_case")]
pub enum MetricSourceV4 {
    Unknown,
    DirectApi,
    Libproc,
    Iokit,
    Pdh,
    InterfaceAggregate,
    ProcessAggregate,
    Sysinfo,
    Runtime,
    Etw,
    Nstat,
    Procfs,
    Ebpf,
    Fixture,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(TS))]
#[serde(rename_all = "snake_case")]
pub enum MetricQualityV4 {
    Native,
    Estimated,
    Held,
    Partial,
    Unavailable,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(test, derive(TS))]
pub struct MetricObservation(
    pub u16,
    pub Option<f64>,
    pub u8,
    #[cfg_attr(test, ts(type = "number | null"))] pub Option<u64>,
    pub Option<u16>,
);

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(test, derive(TS))]
#[serde(rename_all = "snake_case")]
pub struct LimitationEntry {
    pub code: LimitationCode,
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(TS))]
#[serde(rename_all = "snake_case")]
pub enum LimitationCode {
    UnsupportedMetric,
    AccessDenied,
    AuthorizationScope,
    PartialCoverage,
    PendingBaseline,
    HeldValue,
    CollectorFailure,
    DataLoss,
    MissingMetadata,
    GroupPartialCoverage,
    NumericRange,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(test, derive(TS))]
#[serde(rename_all = "snake_case")]
pub struct SystemDetailV4 {
    pub stable_id: String,
    pub metrics: Vec<MetricObservation>,
    pub logical_cpus: Vec<LogicalCpuDetailV4>,
    pub kernel_pool_tags: Vec<KernelPoolTagDetailV4>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(test, derive(TS))]
#[serde(rename_all = "snake_case")]
pub struct LogicalCpuDetailV4 {
    pub stable_id: String,
    pub index: u16,
    pub metrics: Vec<MetricObservation>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(test, derive(TS))]
#[serde(rename_all = "snake_case")]
pub struct KernelPoolTagDetailV4 {
    pub stable_id: String,
    pub tag: String,
    pub kind: KernelPoolKindV4,
    pub driver_candidates: Vec<String>,
    pub driver_candidates_pending: bool,
    pub metrics: Vec<MetricObservation>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[cfg_attr(test, derive(TS))]
#[serde(rename_all = "snake_case")]
pub enum KernelPoolKindV4 {
    Paged,
    Nonpaged,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(test, derive(TS))]
#[serde(tag = "kind", content = "detail", rename_all = "snake_case")]
pub enum WorkloadDetailV4 {
    Process(ProcessDetailV4),
    Group(GroupDetailV4),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(test, derive(TS))]
#[serde(rename_all = "snake_case")]
pub struct ProcessDetailV4 {
    pub stable_id: String,
    pub identity_stability: ProcessIdentityStabilityV4,
    pub pid: String,
    pub parent_pid: Option<String>,
    pub parent_process_id: Option<String>,
    #[cfg_attr(test, ts(type = "number | null"))]
    pub start_time_ms: Option<u64>,
    pub display_name: String,
    pub executable: String,
    pub status: String,
    pub access_state: AccessStateV4,
    pub presentation: ProcessPresentationV4,
    pub metrics: Vec<MetricObservation>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[cfg_attr(test, derive(TS))]
#[serde(rename_all = "snake_case")]
pub enum ProcessIdentityStabilityV4 {
    Stable,
    Publication,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[cfg_attr(test, derive(TS))]
#[serde(rename_all = "snake_case")]
pub enum AccessStateV4 {
    Full,
    Partial,
    Denied,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(test, derive(TS))]
#[serde(rename_all = "snake_case")]
pub struct ProcessPresentationV4 {
    pub group_id: Option<String>,
    pub group_key: String,
    pub group_label: String,
    pub group_category: String,
    pub group_count: u32,
    pub icon_kind: String,
    pub is_child: bool,
    pub is_grouped: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(test, derive(TS))]
#[serde(rename_all = "snake_case")]
pub struct GroupDetailV4 {
    pub stable_id: String,
    pub group_key: String,
    pub label: String,
    pub category: String,
    pub member_ids: Vec<String>,
    pub icon_kind: String,
    pub icon_source: Option<String>,
    pub example_label: Option<String>,
    pub metrics: Vec<MetricObservation>,
    pub coverage: Vec<GroupMetricCoverageV4>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(test, derive(TS))]
#[serde(rename_all = "snake_case")]
pub struct GroupMetricCoverageV4 {
    pub descriptor_index: u16,
    pub available_contributors: u32,
    pub total_contributors: u32,
    pub limitation_index: Option<u16>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(test, derive(TS))]
#[serde(rename_all = "snake_case")]
pub struct ProcessContributorV4 {
    pub metric: ContributorMetricV4,
    pub process_id: Option<String>,
    pub display_name: Option<String>,
    pub name_ambiguous: bool,
    pub available_contributors: u32,
    pub total_contributors: u32,
    pub quality_code: u8,
    pub source: MetricSourceV4,
    pub limitation_index: Option<u16>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(test, derive(TS))]
#[serde(rename_all = "snake_case")]
pub enum ContributorMetricV4 {
    Cpu,
    Memory,
    Io,
    Network,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(test, derive(TS))]
#[serde(rename_all = "snake_case")]
pub struct RuntimeWarningV4 {
    pub key: String,
    #[cfg_attr(test, ts(type = "number"))]
    pub publication_seq: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub occurred_at_ms: u64,
    pub category: String,
    pub message: String,
}
