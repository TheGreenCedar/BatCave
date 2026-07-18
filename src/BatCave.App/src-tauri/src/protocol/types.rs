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
    RuntimeSnapshot(Box<RuntimeSnapshotPayloadV3>),
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
pub struct RuntimeSnapshotPayloadV3 {
    #[cfg_attr(test, ts(type = "number"))]
    pub publication_seq: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub published_at_ms: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub sample_seq: u64,
    #[cfg_attr(test, ts(type = "number | null"))]
    pub sampled_at_ms: Option<u64>,
    pub source: String,
    pub environment: RuntimeEnvironmentV3,
    pub privileged_collection: RuntimePrivilegedCollectionV3,
    pub settings: RuntimeSettingsV3,
    pub health: RuntimeHealthV3,
    pub persistence: Option<RuntimePersistenceV3>,
    pub descriptors: Vec<MeasurementDescriptor>,
    pub quality_codes: Vec<MetricQualityV3>,
    pub limitations: Vec<LimitationEntry>,
    pub system: SystemDetailV3,
    pub workloads: Vec<WorkloadDetailV3>,
    pub contributors: Vec<ProcessContributorV3>,
    pub total_process_count: u32,
    pub visible_process_count: u32,
    pub warnings: Vec<RuntimeWarningV3>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(test, derive(TS))]
#[serde(rename_all = "snake_case")]
pub struct RuntimeEnvironmentV3 {
    pub platform: RuntimePlatformV3,
    pub architecture: RuntimeArchitectureV3,
    pub process_elevation: RuntimeProcessElevationV3,
    pub install_kind: RuntimeInstallKindV3,
    pub data_directory: Option<String>,
    pub release_identity: RuntimeReleaseIdentityV3,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(TS))]
#[serde(rename_all = "snake_case")]
pub struct RuntimeReleaseIdentityV3 {
    pub app_version: String,
    pub source_commit_sha: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(TS))]
#[serde(rename_all = "snake_case")]
pub enum RuntimeArchitectureV3 {
    X86_64,
    Aarch64,
    X86,
    Unknown,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[cfg_attr(test, derive(TS))]
#[serde(rename_all = "snake_case")]
pub enum RuntimePlatformV3 {
    Windows,
    Linux,
    Macos,
    Fixture,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[cfg_attr(test, derive(TS))]
#[serde(rename_all = "snake_case")]
pub enum RuntimeProcessElevationV3 {
    Unknown,
    Standard,
    Elevated,
    NotApplicable,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[cfg_attr(test, derive(TS))]
#[serde(rename_all = "snake_case")]
pub enum RuntimeInstallKindV3 {
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
pub struct RuntimePrivilegedCollectionV3 {
    pub state: PrivilegedCollectionStateV3,
    pub source: PrivilegedCollectionSourceV3,
    pub preference: PrivilegedCollectionPreferenceV3,
    pub standard_fallback_process_etw_disabled: bool,
    pub detail: Option<String>,
    #[cfg_attr(test, ts(type = "number | null"))]
    pub last_success_at_ms: Option<u64>,
    pub collector_service: Option<CollectorServiceStatusV3>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[cfg_attr(test, derive(TS))]
#[serde(rename_all = "snake_case")]
pub enum PrivilegedCollectionStateV3 {
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
pub enum PrivilegedCollectionSourceV3 {
    None,
    LocalProcess,
    CollectorService,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(TS))]
#[serde(rename_all = "snake_case")]
pub enum PrivilegedCollectionPreferenceV3 {
    StandardOnly,
    BestAvailable,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(test, derive(TS))]
#[serde(rename_all = "snake_case")]
pub struct CollectorServiceStatusV3 {
    pub state: CollectorServiceStateV3,
    pub release_identity: Option<RuntimeReleaseIdentityV3>,
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
pub enum CollectorServiceStateV3 {
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
pub struct RuntimeSettingsV3 {
    pub query: RuntimeQueryV3,
    pub metric_window_seconds: u32,
    pub effective_sample_interval_ms: u32,
    pub collection_paused: bool,
    pub ui_preferences: Option<RuntimeUiPreferencesV3>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(test, derive(TS))]
#[serde(rename_all = "snake_case")]
pub struct RuntimeUiPreferencesV3 {
    pub theme: String,
    pub history_point_limit: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(test, derive(TS))]
#[serde(rename_all = "snake_case")]
pub struct RuntimeQueryInputV3 {
    pub filter_text: String,
    pub focus_mode: ProcessFocusModeV3,
    pub sort_column: SortColumnV3,
    pub sort_direction: SortDirectionV3,
    pub limit: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(test, derive(TS))]
#[serde(rename_all = "snake_case")]
pub struct RuntimeQueryV3 {
    pub filter_text: String,
    pub focus_mode: ProcessFocusModeV3,
    pub sort_column: SortColumnV3,
    pub sort_direction: SortDirectionV3,
    pub limit: u32,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[cfg_attr(test, derive(TS))]
#[serde(rename_all = "snake_case")]
pub enum ProcessFocusModeV3 {
    All,
    Attention,
    Io,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[cfg_attr(test, derive(TS))]
#[serde(rename_all = "snake_case")]
pub enum SortColumnV3 {
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
pub enum SortDirectionV3 {
    Asc,
    Desc,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(test, derive(TS))]
#[serde(rename_all = "snake_case")]
pub struct RuntimeHealthV3 {
    pub engine_state: Option<RuntimeEngineStateV3>,
    pub collector_state: Option<RuntimeCollectorStateV3>,
    pub degraded: bool,
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
    pub fatal_error: Option<RuntimeFatalErrorV3>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(TS))]
#[serde(rename_all = "snake_case")]
pub enum RuntimeEngineStateV3 {
    Starting,
    Running,
    Paused,
    Fatal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(TS))]
#[serde(rename_all = "snake_case")]
pub enum RuntimeCollectorStateV3 {
    Healthy,
    Limited,
    Unavailable,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(test, derive(TS))]
#[serde(rename_all = "snake_case")]
pub struct RuntimeFatalErrorV3 {
    pub code: String,
    pub message: String,
    #[cfg_attr(test, ts(type = "number"))]
    pub occurred_at_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(test, derive(TS))]
#[serde(rename_all = "snake_case")]
pub struct RuntimePersistenceV3 {
    pub state: RuntimePersistenceStateV3,
    pub roots: Vec<RuntimePersistenceRootV3>,
    pub components: Vec<RuntimePersistenceComponentV3>,
    #[cfg_attr(test, ts(type = "number"))]
    pub suppressed_diagnostic_events: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(TS))]
#[serde(rename_all = "snake_case")]
pub enum RuntimePersistenceStateV3 {
    Healthy,
    Degraded,
    Unavailable,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(test, derive(TS))]
#[serde(rename_all = "snake_case")]
pub struct RuntimePersistenceRootV3 {
    pub owner: RuntimePersistenceOwnerV3,
    pub directory: Option<String>,
    pub permission_state: RuntimePersistencePermissionStateV3,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(test, derive(TS))]
#[serde(rename_all = "snake_case")]
pub enum RuntimePersistenceOwnerV3 {
    CurrentUser,
    CollectorService,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(TS))]
#[serde(rename_all = "snake_case")]
pub enum RuntimePersistencePermissionStateV3 {
    Verified,
    Invalid,
    Unavailable,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(test, derive(TS))]
#[serde(rename_all = "snake_case")]
pub struct RuntimePersistenceComponentV3 {
    pub owner: RuntimePersistenceOwnerV3,
    pub kind: RuntimePersistenceKindV3,
    pub state: RuntimePersistenceStateV3,
    pub durability: RuntimePersistenceDurabilityV3,
    #[cfg_attr(test, ts(type = "number | null"))]
    pub last_success_at_ms: Option<u64>,
    pub active_failure: Option<RuntimePersistenceFailureV3>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(test, derive(TS))]
#[serde(rename_all = "snake_case")]
pub enum RuntimePersistenceKindV3 {
    Settings,
    WarmCache,
    Diagnostics,
    ServiceState,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(TS))]
#[serde(rename_all = "snake_case")]
pub enum RuntimePersistenceDurabilityV3 {
    Durable,
    NotWritten,
    SessionOnly,
    NotApplicable,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(test, derive(TS))]
#[serde(rename_all = "snake_case")]
pub struct RuntimePersistenceFailureV3 {
    pub code: String,
    pub operation: RuntimePersistenceOperationV3,
    #[cfg_attr(test, ts(type = "number"))]
    pub occurred_at_ms: u64,
    pub retryable: bool,
    pub summary: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(TS))]
#[serde(rename_all = "snake_case")]
pub enum RuntimePersistenceOperationV3 {
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
    pub network_scope: Option<NetworkScopeV3>,
    pub source: MetricSourceV3,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(TS))]
#[serde(rename_all = "snake_case")]
pub enum NetworkScopeV3 {
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
pub enum MetricSourceV3 {
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
pub enum MetricQualityV3 {
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
pub struct SystemDetailV3 {
    pub stable_id: String,
    pub metrics: Vec<MetricObservation>,
    pub logical_cpus: Vec<LogicalCpuDetailV3>,
    pub kernel_pool_tags: Vec<KernelPoolTagDetailV3>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(test, derive(TS))]
#[serde(rename_all = "snake_case")]
pub struct LogicalCpuDetailV3 {
    pub stable_id: String,
    pub index: u16,
    pub metrics: Vec<MetricObservation>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(test, derive(TS))]
#[serde(rename_all = "snake_case")]
pub struct KernelPoolTagDetailV3 {
    pub stable_id: String,
    pub tag: String,
    pub kind: KernelPoolKindV3,
    pub driver_candidates: Vec<String>,
    pub driver_candidates_pending: bool,
    pub metrics: Vec<MetricObservation>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[cfg_attr(test, derive(TS))]
#[serde(rename_all = "snake_case")]
pub enum KernelPoolKindV3 {
    Paged,
    Nonpaged,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(test, derive(TS))]
#[serde(tag = "kind", content = "detail", rename_all = "snake_case")]
pub enum WorkloadDetailV3 {
    Process(ProcessDetailV3),
    Group(GroupDetailV3),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(test, derive(TS))]
#[serde(rename_all = "snake_case")]
pub struct ProcessDetailV3 {
    pub stable_id: String,
    pub identity_stability: ProcessIdentityStabilityV3,
    pub pid: String,
    pub parent_pid: Option<String>,
    pub parent_process_id: Option<String>,
    #[cfg_attr(test, ts(type = "number | null"))]
    pub start_time_ms: Option<u64>,
    pub display_name: String,
    pub executable: String,
    pub status: String,
    pub access_state: AccessStateV3,
    pub presentation: ProcessPresentationV3,
    pub metrics: Vec<MetricObservation>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[cfg_attr(test, derive(TS))]
#[serde(rename_all = "snake_case")]
pub enum ProcessIdentityStabilityV3 {
    Stable,
    Publication,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[cfg_attr(test, derive(TS))]
#[serde(rename_all = "snake_case")]
pub enum AccessStateV3 {
    Full,
    Partial,
    Denied,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(test, derive(TS))]
#[serde(rename_all = "snake_case")]
pub struct ProcessPresentationV3 {
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
pub struct GroupDetailV3 {
    pub stable_id: String,
    pub group_key: String,
    pub label: String,
    pub category: String,
    pub member_ids: Vec<String>,
    pub icon_kind: String,
    pub icon_source: Option<String>,
    pub example_label: Option<String>,
    pub metrics: Vec<MetricObservation>,
    pub coverage: Vec<GroupMetricCoverageV3>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(test, derive(TS))]
#[serde(rename_all = "snake_case")]
pub struct GroupMetricCoverageV3 {
    pub descriptor_index: u16,
    pub available_contributors: u32,
    pub total_contributors: u32,
    pub limitation_index: Option<u16>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(test, derive(TS))]
#[serde(rename_all = "snake_case")]
pub struct ProcessContributorV3 {
    pub metric: ContributorMetricV3,
    pub process_id: Option<String>,
    pub display_name: Option<String>,
    pub name_ambiguous: bool,
    pub available_contributors: u32,
    pub total_contributors: u32,
    pub quality_code: u8,
    pub source: MetricSourceV3,
    pub limitation_index: Option<u16>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(test, derive(TS))]
#[serde(rename_all = "snake_case")]
pub enum ContributorMetricV3 {
    Cpu,
    Memory,
    Io,
    Network,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(test, derive(TS))]
#[serde(rename_all = "snake_case")]
pub struct RuntimeWarningV3 {
    pub key: String,
    #[cfg_attr(test, ts(type = "number"))]
    pub publication_seq: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub occurred_at_ms: u64,
    pub category: String,
    pub message: String,
}
