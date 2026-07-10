use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct RuntimeSnapshot {
    pub event_kind: String,
    pub publication_seq: u64,
    pub published_at_ms: u64,
    pub sample_seq: u64,
    pub sampled_at_ms: Option<u64>,
    pub source: String,
    pub environment: RuntimeEnvironment,
    pub admin_mode: RuntimeAdminModeStatus,
    pub settings: RuntimeSettings,
    pub health: RuntimeHealth,
    pub system: SystemMetricsSnapshot,
    pub process_contributors: ProcessContributorSummary,
    pub processes: Vec<ProcessSample>,
    pub process_view_rows: Vec<ProcessViewRow>,
    pub total_process_count: usize,
    pub warnings: Vec<RuntimeWarning>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ProcessContributorSummary {
    pub cpu: Option<String>,
    pub memory: Option<String>,
    pub disk: Option<String>,
    pub network: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct RuntimeEnvironment {
    pub platform: RuntimePlatform,
    pub admin_mode_available: bool,
    pub data_directory: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct RuntimeAdminModeStatus {
    pub state: RuntimeAdminModeState,
    pub detail: Option<String>,
    pub last_success_at_ms: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeAdminModeState {
    Unavailable,
    Off,
    Requesting,
    Active,
    Recovering,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimePlatform {
    Windows,
    Linux,
    Fixture,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct RuntimeHealth {
    pub tick_count: u64,
    pub snapshot_latency_ms: u64,
    pub degraded: bool,
    pub collector_warnings: usize,
    pub runtime_loop_enabled: bool,
    pub runtime_loop_running: bool,
    pub status_summary: String,
    pub updated_at_ms: u64,
    pub tick_p95_ms: f64,
    pub sort_p95_ms: f64,
    pub jitter_p95_ms: f64,
    pub dropped_ticks: u64,
    pub app_cpu_percent: f64,
    pub app_rss_bytes: u64,
    pub last_warning: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct SystemMetricsSnapshot {
    pub cpu_percent: f64,
    pub kernel_cpu_percent: f64,
    pub logical_cpu_percent: Vec<f64>,
    pub memory_used_bytes: u64,
    pub memory_total_bytes: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub memory_available_bytes: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub swap_used_bytes: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub swap_total_bytes: Option<u64>,
    pub process_count: usize,
    pub disk_read_total_bytes: u64,
    pub disk_write_total_bytes: u64,
    pub disk_read_bps: u64,
    pub disk_write_bps: u64,
    pub network_received_total_bytes: u64,
    pub network_transmitted_total_bytes: u64,
    pub network_received_bps: u64,
    pub network_transmitted_bps: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub memory_accounting: Option<SystemMemoryAccounting>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quality: Option<SystemMetricQuality>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct SystemMemoryAccounting {
    pub process_working_set_bytes: u64,
    pub process_private_bytes: u64,
    pub denied_process_count: usize,
    pub partial_process_count: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unattributed_bytes: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub commit_used_bytes: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub commit_limit_bytes: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system_cache_bytes: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kernel_total_bytes: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kernel_paged_pool_bytes: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kernel_nonpaged_pool_bytes: Option<u64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub kernel_pool_tags: Vec<KernelPoolTag>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct KernelPoolTag {
    pub tag: String,
    pub kind: KernelPoolKind,
    pub bytes: u64,
    pub allocations: u64,
    pub frees: u64,
    #[serde(default)]
    pub driver_candidates: Vec<String>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub driver_candidates_pending: bool,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KernelPoolKind {
    #[default]
    Paged,
    Nonpaged,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ProcessSample {
    pub pid: String,
    pub parent_pid: Option<String>,
    pub start_time_ms: u64,
    pub name: String,
    pub exe: String,
    pub status: String,
    pub cpu_percent: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kernel_cpu_percent: Option<f64>,
    pub memory_bytes: u64,
    pub private_bytes: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub virtual_memory_bytes: Option<u64>,
    pub disk_read_total_bytes: u64,
    pub disk_write_total_bytes: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub other_io_total_bytes: Option<u64>,
    pub disk_read_bps: u64,
    pub disk_write_bps: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub other_io_bps: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub network_received_bps: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub network_transmitted_bps: Option<u64>,
    pub threads: u32,
    pub handles: u32,
    pub access_state: AccessState,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quality: Option<ProcessMetricQuality>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ProcessViewRow {
    pub kind: ProcessViewRowKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub process: Option<ProcessSample>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub representative: Option<ProcessSample>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub group_key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub group_label: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub group_category: Option<String>,
    pub group_count: usize,
    pub icon_kind: String,
    pub is_child: bool,
    pub is_grouped: bool,
    pub attention_label: String,
    pub cpu_percent: f64,
    pub memory_bytes: u64,
    pub io_bps: u64,
    pub network_bps: u64,
    pub threads: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProcessViewRowKind {
    Group,
    Process,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct MetricQualityInfo {
    pub quality: MetricQuality,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<MetricSource>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub updated_at_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub age_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

impl MetricQualityInfo {
    pub fn new(quality: MetricQuality, source: MetricSource) -> Self {
        Self {
            quality,
            source: Some(source),
            updated_at_ms: None,
            age_ms: None,
            message: None,
        }
    }

    pub fn with_message(mut self, message: &str) -> Self {
        self.message = Some(message.to_string());
        self
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub struct SystemMetricQuality {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cpu: Option<MetricQualityInfo>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kernel_cpu: Option<MetricQualityInfo>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub logical_cpu: Option<MetricQualityInfo>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub memory: Option<MetricQualityInfo>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub swap: Option<MetricQualityInfo>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub disk: Option<MetricQualityInfo>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub network: Option<MetricQualityInfo>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub struct ProcessMetricQuality {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cpu: Option<MetricQualityInfo>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub memory: Option<MetricQualityInfo>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub disk: Option<MetricQualityInfo>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub other_io: Option<MetricQualityInfo>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub network: Option<MetricQualityInfo>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub threads: Option<MetricQualityInfo>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub handles: Option<MetricQualityInfo>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MetricQuality {
    Native,
    Estimated,
    Held,
    Partial,
    Unavailable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MetricSource {
    DirectApi,
    Pdh,
    InterfaceAggregate,
    ProcessAggregate,
    Sysinfo,
    Runtime,
    Etw,
    Procfs,
    Ebpf,
    Fixture,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct RuntimeWarning {
    pub key: String,
    pub publication_seq: u64,
    pub occurred_at_ms: u64,
    pub category: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct RuntimeSettings {
    #[serde(default)]
    pub query: RuntimeQuery,
    #[serde(default)]
    pub admin_mode_requested: bool,
    #[serde(default)]
    pub admin_mode_enabled: bool,
    #[serde(default = "default_metric_window_seconds")]
    pub metric_window_seconds: u32,
    #[serde(default)]
    pub paused: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct RuntimeQuery {
    #[serde(default)]
    pub filter_text: String,
    #[serde(default)]
    pub focus_mode: ProcessFocusMode,
    #[serde(default)]
    pub sort_column: SortColumn,
    #[serde(default)]
    pub sort_direction: SortDirection,
    #[serde(default = "default_query_limit")]
    pub limit: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProcessFocusMode {
    All,
    Attention,
    Io,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AccessState {
    Full,
    Partial,
    Denied,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SortColumn {
    Attention,
    Name,
    Pid,
    CpuPct,
    MemoryBytes,
    DiskBps,
    NetworkBps,
    Threads,
    Handles,
    StartTimeMs,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SortDirection {
    Asc,
    Desc,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct WarmCache {
    pub seq: u64,
    #[serde(alias = "processes")]
    pub rows: Vec<ProcessSample>,
}

impl Default for RuntimeSettings {
    fn default() -> Self {
        Self {
            query: RuntimeQuery::default(),
            admin_mode_requested: false,
            admin_mode_enabled: false,
            metric_window_seconds: default_metric_window_seconds(),
            paused: false,
        }
    }
}

impl Default for RuntimeQuery {
    fn default() -> Self {
        Self {
            filter_text: String::new(),
            focus_mode: ProcessFocusMode::All,
            sort_column: SortColumn::Attention,
            sort_direction: SortDirection::Desc,
            limit: default_query_limit(),
        }
    }
}

impl Default for ProcessFocusMode {
    fn default() -> Self {
        Self::All
    }
}

impl Default for SortColumn {
    fn default() -> Self {
        Self::Attention
    }
}

impl Default for SortDirection {
    fn default() -> Self {
        Self::Desc
    }
}

fn default_metric_window_seconds() -> u32 {
    60
}

fn default_query_limit() -> usize {
    5000
}

fn is_false(value: &bool) -> bool {
    !*value
}

impl Default for RuntimeHealth {
    fn default() -> Self {
        Self {
            tick_count: 0,
            snapshot_latency_ms: 0,
            degraded: false,
            collector_warnings: 0,
            runtime_loop_enabled: true,
            runtime_loop_running: false,
            status_summary: "Runtime starting.".to_string(),
            updated_at_ms: 0,
            tick_p95_ms: 0.0,
            sort_p95_ms: 0.0,
            jitter_p95_ms: 0.0,
            dropped_ticks: 0,
            app_cpu_percent: 0.0,
            app_rss_bytes: 0,
            last_warning: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn runtime_snapshot_serializes_current_snake_case_wire_shape() {
        let snapshot = RuntimeSnapshot {
            event_kind: "snapshot".to_string(),
            publication_seq: 42,
            published_at_ms: 1_700_000_000_123,
            sample_seq: 40,
            sampled_at_ms: Some(1_700_000_000_000),
            source: "rust_runtime".to_string(),
            environment: RuntimeEnvironment {
                platform: RuntimePlatform::Windows,
                admin_mode_available: true,
                data_directory: Some("C:\\Users\\test\\BatCaveMonitor".to_string()),
            },
            admin_mode: RuntimeAdminModeStatus {
                state: RuntimeAdminModeState::Requesting,
                detail: None,
                last_success_at_ms: None,
            },
            settings: RuntimeSettings {
                query: RuntimeQuery {
                    filter_text: "bat".to_string(),
                    focus_mode: ProcessFocusMode::Attention,
                    sort_column: SortColumn::MemoryBytes,
                    sort_direction: SortDirection::Asc,
                    limit: 25,
                },
                admin_mode_requested: true,
                admin_mode_enabled: false,
                metric_window_seconds: 30,
                paused: true,
            },
            health: RuntimeHealth {
                tick_count: 7,
                snapshot_latency_ms: 11,
                degraded: true,
                collector_warnings: 2,
                runtime_loop_enabled: true,
                runtime_loop_running: true,
                status_summary: "Collecting with warnings.".to_string(),
                updated_at_ms: 1_700_000_000_456,
                tick_p95_ms: 12.5,
                sort_p95_ms: 3.25,
                jitter_p95_ms: 1.75,
                dropped_ticks: 1,
                app_cpu_percent: 4.5,
                app_rss_bytes: 123_456,
                last_warning: Some("partial process access".to_string()),
            },
            system: SystemMetricsSnapshot {
                cpu_percent: 13.5,
                kernel_cpu_percent: 2.5,
                logical_cpu_percent: vec![10.0, 17.0],
                memory_used_bytes: 8_000,
                memory_total_bytes: 16_000,
                memory_available_bytes: Some(8_000),
                swap_used_bytes: Some(1_000),
                swap_total_bytes: Some(2_000),
                process_count: 99,
                disk_read_total_bytes: 1_000_000,
                disk_write_total_bytes: 2_000_000,
                disk_read_bps: 300,
                disk_write_bps: 400,
                network_received_total_bytes: 3_000_000,
                network_transmitted_total_bytes: 4_000_000,
                network_received_bps: 500,
                network_transmitted_bps: 600,
                memory_accounting: Some(SystemMemoryAccounting {
                    process_working_set_bytes: 6_000,
                    process_private_bytes: 3_000,
                    denied_process_count: 2,
                    partial_process_count: 3,
                    unattributed_bytes: Some(2_000),
                    commit_used_bytes: Some(11_000),
                    commit_limit_bytes: Some(32_000),
                    system_cache_bytes: Some(1_500),
                    kernel_total_bytes: Some(1_200),
                    kernel_paged_pool_bytes: Some(700),
                    kernel_nonpaged_pool_bytes: Some(500),
                    kernel_pool_tags: vec![KernelPoolTag {
                        tag: "Leak".to_string(),
                        kind: KernelPoolKind::Nonpaged,
                        bytes: 384,
                        allocations: 12,
                        frees: 2,
                        driver_candidates: vec!["leaky.sys".to_string()],
                        driver_candidates_pending: false,
                    }],
                }),
                quality: Some(SystemMetricQuality {
                    cpu: Some(MetricQualityInfo::new(
                        MetricQuality::Native,
                        MetricSource::DirectApi,
                    )),
                    kernel_cpu: Some(MetricQualityInfo::new(
                        MetricQuality::Native,
                        MetricSource::DirectApi,
                    )),
                    logical_cpu: Some(MetricQualityInfo::new(
                        MetricQuality::Estimated,
                        MetricSource::Sysinfo,
                    )),
                    memory: Some(MetricQualityInfo::new(
                        MetricQuality::Native,
                        MetricSource::DirectApi,
                    )),
                    swap: Some(MetricQualityInfo::new(
                        MetricQuality::Native,
                        MetricSource::DirectApi,
                    )),
                    disk: Some(MetricQualityInfo::new(
                        MetricQuality::Partial,
                        MetricSource::ProcessAggregate,
                    )),
                    network: Some(MetricQualityInfo::new(
                        MetricQuality::Native,
                        MetricSource::InterfaceAggregate,
                    )),
                }),
            },
            process_contributors: ProcessContributorSummary {
                cpu: Some("BatCave.App".to_string()),
                memory: Some("BatCave.App".to_string()),
                disk: Some("BatCave.App".to_string()),
                network: None,
            },
            processes: vec![sample_process()],
            process_view_rows: vec![sample_process_view_row()],
            total_process_count: 1,
            warnings: vec![RuntimeWarning {
                key: "collector.network_attribution".to_string(),
                publication_seq: 41,
                occurred_at_ms: 1_700_000_000_400,
                category: "collector".to_string(),
                message: "partial process access".to_string(),
            }],
        };

        let actual = serde_json::to_value(snapshot).expect("snapshot serializes");

        let mut expected: serde_json::Value = serde_json::from_str(
            r#"{
                "event_kind": "snapshot",
                "publication_seq": 42,
                "published_at_ms": 1700000000123,
                "sample_seq": 40,
                "sampled_at_ms": 1700000000000,
                "source": "rust_runtime",
                "environment": {
                    "platform": "windows",
                    "admin_mode_available": true,
                    "data_directory": "C:\\Users\\test\\BatCaveMonitor"
                },
                "admin_mode": {
                    "state": "requesting",
                    "detail": null,
                    "last_success_at_ms": null
                },
                "settings": {
                    "query": {
                        "filter_text": "bat",
                        "focus_mode": "attention",
                        "sort_column": "memory_bytes",
                        "sort_direction": "asc",
                        "limit": 25
                    },
                    "admin_mode_requested": true,
                    "admin_mode_enabled": false,
                    "metric_window_seconds": 30,
                    "paused": true
                },
                "health": {
                    "tick_count": 7,
                    "snapshot_latency_ms": 11,
                    "degraded": true,
                    "collector_warnings": 2,
                    "runtime_loop_enabled": true,
                    "runtime_loop_running": true,
                    "status_summary": "Collecting with warnings.",
                    "updated_at_ms": 1700000000456,
                    "tick_p95_ms": 12.5,
                    "sort_p95_ms": 3.25,
                    "jitter_p95_ms": 1.75,
                    "dropped_ticks": 1,
                    "app_cpu_percent": 4.5,
                    "app_rss_bytes": 123456,
                    "last_warning": "partial process access"
                },
                "system": {
                    "cpu_percent": 13.5,
                    "kernel_cpu_percent": 2.5,
                    "logical_cpu_percent": [10.0, 17.0],
                    "memory_used_bytes": 8000,
                    "memory_total_bytes": 16000,
                    "memory_available_bytes": 8000,
                    "swap_used_bytes": 1000,
                    "swap_total_bytes": 2000,
                    "process_count": 99,
                    "disk_read_total_bytes": 1000000,
                    "disk_write_total_bytes": 2000000,
                    "disk_read_bps": 300,
                    "disk_write_bps": 400,
                    "network_received_total_bytes": 3000000,
                    "network_transmitted_total_bytes": 4000000,
                    "network_received_bps": 500,
                    "network_transmitted_bps": 600,
                    "memory_accounting": {
                        "process_working_set_bytes": 6000,
                        "process_private_bytes": 3000,
                        "denied_process_count": 2,
                        "partial_process_count": 3,
                        "unattributed_bytes": 2000,
                        "commit_used_bytes": 11000,
                        "commit_limit_bytes": 32000,
                        "system_cache_bytes": 1500,
                        "kernel_total_bytes": 1200,
                        "kernel_paged_pool_bytes": 700,
                        "kernel_nonpaged_pool_bytes": 500,
                        "kernel_pool_tags": [{
                            "tag": "Leak",
                            "kind": "nonpaged",
                            "bytes": 384,
                            "allocations": 12,
                            "frees": 2,
                            "driver_candidates": ["leaky.sys"]
                        }]
                    },
                    "quality": {
                        "cpu": { "quality": "native", "source": "direct_api" },
                        "kernel_cpu": { "quality": "native", "source": "direct_api" },
                        "logical_cpu": { "quality": "estimated", "source": "sysinfo" },
                        "memory": { "quality": "native", "source": "direct_api" },
                        "swap": { "quality": "native", "source": "direct_api" },
                        "disk": { "quality": "partial", "source": "process_aggregate" },
                        "network": { "quality": "native", "source": "interface_aggregate" }
                    }
                },
                "process_contributors": {
                    "cpu": "BatCave.App",
                    "memory": "BatCave.App",
                    "disk": "BatCave.App",
                    "network": null
                },
                "processes": [],
                "process_view_rows": [],
                "total_process_count": 1,
                "warnings": [{
                    "key": "collector.network_attribution",
                    "publication_seq": 41,
                    "occurred_at_ms": 1700000000400,
                    "category": "collector",
                    "message": "partial process access"
                }]
            }"#,
        )
        .expect("expected JSON parses");
        expected["processes"] = json!([sample_process_json()]);
        expected["process_view_rows"] = json!([sample_process_view_row_json()]);

        assert_eq!(actual, expected);
        assert!(actual.get("seq").is_none());
        assert!(actual.get("ts_ms").is_none());
    }

    #[test]
    fn kernel_pool_tag_always_serializes_driver_candidates() {
        let tag = KernelPoolTag {
            tag: "None".to_string(),
            kind: KernelPoolKind::Paged,
            bytes: 0,
            allocations: 0,
            frees: 0,
            driver_candidates: Vec::new(),
            driver_candidates_pending: false,
        };

        let actual = serde_json::to_value(tag).expect("tag serializes");

        assert_eq!(actual["driver_candidates"], json!([]));
    }

    #[test]
    fn shared_runtime_snapshot_fixture_round_trips_through_rust() {
        let expected: serde_json::Value = serde_json::from_str(include_str!(
            "../../scripts/fixtures/runtime-snapshot.v2.json"
        ))
        .expect("shared fixture parses");
        let snapshot: RuntimeSnapshot =
            serde_json::from_value(expected.clone()).expect("shared fixture matches Rust contract");

        assert_eq!(
            serde_json::to_value(snapshot).expect("snapshot serializes"),
            expected
        );
    }

    #[test]
    fn runtime_settings_serializes_sort_and_direction_as_snake_case() {
        let settings = RuntimeSettings {
            query: RuntimeQuery {
                filter_text: String::new(),
                focus_mode: ProcessFocusMode::All,
                sort_column: SortColumn::MemoryBytes,
                sort_direction: SortDirection::Asc,
                limit: 100,
            },
            admin_mode_requested: false,
            admin_mode_enabled: false,
            metric_window_seconds: 15,
            paused: false,
        };

        let actual = serde_json::to_value(settings).expect("settings serializes");

        assert_eq!(
            actual,
            json!({
                "query": {
                    "filter_text": "",
                    "focus_mode": "all",
                    "sort_column": "memory_bytes",
                    "sort_direction": "asc",
                    "limit": 100
                },
                "admin_mode_requested": false,
                "admin_mode_enabled": false,
                "metric_window_seconds": 15,
                "paused": false
            })
        );
    }

    #[test]
    fn runtime_settings_accepts_minimal_persisted_json() {
        let settings: RuntimeSettings = serde_json::from_value(json!({
            "query": {
                "filter_text": "code",
                "sort_column": "cpu_pct",
                "sort_direction": "desc"
            }
        }))
        .expect("minimal persisted settings deserialize");

        assert_eq!(settings.query.filter_text, "code");
        assert_eq!(settings.query.focus_mode, ProcessFocusMode::All);
        assert_eq!(settings.query.sort_column, SortColumn::CpuPct);
        assert_eq!(settings.query.sort_direction, SortDirection::Desc);
        assert_eq!(settings.query.limit, 5000);
        assert!(!settings.admin_mode_requested);
        assert!(!settings.admin_mode_enabled);
        assert_eq!(settings.metric_window_seconds, 60);
        assert!(!settings.paused);
    }

    #[test]
    fn runtime_settings_default_to_attention_triage() {
        let fresh_settings = RuntimeSettings::default();
        assert_eq!(fresh_settings.query.focus_mode, ProcessFocusMode::All);
        assert_eq!(fresh_settings.query.sort_column, SortColumn::Attention);
        assert_eq!(fresh_settings.query.sort_direction, SortDirection::Desc);

        let persisted_settings: RuntimeSettings =
            serde_json::from_value(json!({})).expect("empty persisted settings deserialize");
        assert_eq!(persisted_settings.query.sort_column, SortColumn::Attention);
        assert_eq!(persisted_settings.query.sort_direction, SortDirection::Desc);

        let partial_query_settings: RuntimeSettings = serde_json::from_value(json!({
            "query": {
                "filter_text": "code"
            }
        }))
        .expect("partial persisted query deserializes");
        assert_eq!(partial_query_settings.query.filter_text, "code");
        assert_eq!(
            partial_query_settings.query.focus_mode,
            ProcessFocusMode::All
        );
        assert_eq!(
            partial_query_settings.query.sort_column,
            SortColumn::Attention
        );
        assert_eq!(
            partial_query_settings.query.sort_direction,
            SortDirection::Desc
        );
    }

    #[test]
    fn warm_cache_accepts_legacy_processes_alias_for_rows() {
        let cache: WarmCache = serde_json::from_value(json!({
            "seq": 9,
            "processes": [sample_process_json()]
        }))
        .expect("legacy warm cache deserializes");

        assert_eq!(cache.seq, 9);
        assert_eq!(cache.rows.len(), 1);
        assert_eq!(cache.rows[0].pid, "1234");
        assert_eq!(cache.rows[0].access_state, AccessState::Partial);
    }

    #[test]
    fn runtime_health_default_starts_waiting_for_runtime_loop() {
        let health = RuntimeHealth::default();

        assert_eq!(health.status_summary, "Runtime starting.");
        assert!(health.runtime_loop_enabled);
        assert!(!health.runtime_loop_running);
        assert!(!health.degraded);
        assert_eq!(health.collector_warnings, 0);
        assert_eq!(health.tick_count, 0);
        assert_eq!(health.last_warning, None);
    }

    fn sample_process() -> ProcessSample {
        serde_json::from_value(sample_process_json()).expect("sample process is valid")
    }

    fn sample_process_view_row() -> ProcessViewRow {
        serde_json::from_value(sample_process_view_row_json()).expect("sample view row is valid")
    }

    fn sample_process_view_row_json() -> serde_json::Value {
        json!({
            "kind": "process",
            "process": sample_process_json(),
            "group_key": "batcave.app.exe",
            "group_label": "BatCave.App.exe",
            "group_category": "BatCave",
            "group_count": 1,
            "icon_kind": "batcave",
            "is_child": false,
            "is_grouped": false,
            "attention_label": "steady",
            "cpu_percent": 8.25,
            "memory_bytes": 65_536,
            "io_bps": 24,
            "network_bps": 0,
            "threads": 9
        })
    }

    fn sample_process_json() -> serde_json::Value {
        json!({
            "pid": "1234",
            "parent_pid": "1000",
            "start_time_ms": 1_699_999_999_000_u64,
            "name": "BatCave.App",
            "exe": "C:\\Program Files\\BatCave\\BatCave.App.exe",
            "status": "run",
            "cpu_percent": 8.25,
            "kernel_cpu_percent": 1.5,
            "memory_bytes": 65_536,
            "private_bytes": 32_768,
            "virtual_memory_bytes": 131_072,
            "disk_read_total_bytes": 123,
            "disk_write_total_bytes": 456,
            "other_io_total_bytes": 789,
            "disk_read_bps": 7,
            "disk_write_bps": 8,
            "other_io_bps": 9,
            "network_received_bps": 0,
            "network_transmitted_bps": 0,
            "threads": 9,
            "handles": 10,
            "access_state": "partial",
            "quality": {
                "cpu": { "quality": "estimated", "source": "sysinfo" },
                "memory": { "quality": "native", "source": "direct_api" },
                "disk": { "quality": "native", "source": "direct_api" },
                "other_io": { "quality": "native", "source": "direct_api" },
                "network": { "quality": "unavailable", "source": "etw", "message": "Waiting for ETW network attribution." },
                "threads": { "quality": "native", "source": "direct_api" },
                "handles": { "quality": "partial", "source": "direct_api" }
            }
        })
    }
}
