use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct RuntimeSnapshot {
    pub event_kind: &'static str,
    pub seq: u64,
    pub ts_ms: u64,
    pub source: &'static str,
    pub health: RuntimeHealth,
    pub system: SystemMetricsSnapshot,
    pub processes: Vec<ProcessSample>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct RuntimeHealth {
    pub tick_count: u64,
    pub snapshot_latency_ms: u64,
    pub degraded: bool,
    pub collector_warnings: usize,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct SystemMetricsSnapshot {
    pub cpu_percent: f64,
    pub kernel_cpu_percent: f64,
    pub logical_cpu_percent: Vec<f64>,
    pub memory_used_bytes: u64,
    pub memory_total_bytes: u64,
    pub swap_used_bytes: u64,
    pub swap_total_bytes: u64,
    pub process_count: usize,
    pub disk_read_total_bytes: u64,
    pub disk_write_total_bytes: u64,
    pub network_received_total_bytes: u64,
    pub network_transmitted_total_bytes: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct ProcessSample {
    pub pid: String,
    pub parent_pid: Option<String>,
    pub name: String,
    pub exe: String,
    pub status: String,
    pub cpu_percent: f64,
    pub memory_bytes: u64,
    pub virtual_memory_bytes: u64,
    pub disk_read_total_bytes: u64,
    pub disk_write_total_bytes: u64,
}
