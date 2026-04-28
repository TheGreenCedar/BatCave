export interface RuntimeSnapshot {
  event_kind: "runtime_snapshot";
  seq: number;
  ts_ms: number;
  source: RuntimeTelemetrySource;
  settings: RuntimeSettings;
  health: RuntimeHealth;
  system: SystemMetricsSnapshot;
  processes: ProcessSample[];
  total_process_count: number;
  warnings: RuntimeWarning[];
}

export type RuntimeTelemetrySource =
  | "tauri_runtime"
  | "tauri_sysinfo"
  | "batcave_runtime"
  | "fixture";

export type MetricQuality = "native" | "estimated" | "held" | "partial" | "unavailable";
export type MetricSource =
  | "direct_api"
  | "pdh"
  | "interface_aggregate"
  | "process_aggregate"
  | "sysinfo"
  | "runtime"
  | "etw"
  | "procfs"
  | "ebpf"
  | "fixture";

export interface MetricQualityInfo {
  quality: MetricQuality;
  source?: MetricSource;
  updated_at_ms?: number;
  age_ms?: number;
  message?: string;
}

export interface RuntimeHealth {
  tick_count: number;
  snapshot_latency_ms: number;
  degraded: boolean;
  collector_warnings: number;
  runtime_loop_enabled: boolean;
  runtime_loop_running: boolean;
  status_summary: string;
  updated_at_ms: number;
  tick_p95_ms: number;
  sort_p95_ms: number;
  jitter_p95_ms: number;
  dropped_ticks: number;
  app_cpu_percent: number;
  app_rss_bytes: number;
  last_warning: string | null;
}

export interface SystemMetricsSnapshot {
  cpu_percent: number;
  kernel_cpu_percent: number;
  logical_cpu_percent: number[];
  memory_used_bytes: number;
  memory_total_bytes: number;
  memory_available_bytes?: number;
  swap_used_bytes: number;
  swap_total_bytes: number;
  process_count: number;
  disk_read_total_bytes: number;
  disk_write_total_bytes: number;
  disk_read_bps: number;
  disk_write_bps: number;
  network_received_total_bytes: number;
  network_transmitted_total_bytes: number;
  network_received_bps: number;
  network_transmitted_bps: number;
  quality?: SystemMetricQuality;
}

export interface SystemMetricQuality {
  cpu?: MetricQualityInfo;
  kernel_cpu?: MetricQualityInfo;
  logical_cpu?: MetricQualityInfo;
  memory?: MetricQualityInfo;
  swap?: MetricQualityInfo;
  disk?: MetricQualityInfo;
  network?: MetricQualityInfo;
}

export interface ProcessSample {
  pid: string;
  parent_pid: string | null;
  start_time_ms: number;
  name: string;
  exe: string;
  status: string;
  cpu_percent: number;
  kernel_cpu_percent?: number;
  memory_bytes: number;
  private_bytes: number;
  virtual_memory_bytes: number;
  disk_read_total_bytes: number;
  disk_write_total_bytes: number;
  other_io_total_bytes?: number;
  disk_read_bps: number;
  disk_write_bps: number;
  other_io_bps?: number;
  network_received_bps?: number;
  network_transmitted_bps?: number;
  threads: number;
  handles: number;
  access_state: AccessState;
  quality?: ProcessMetricQuality;
}

export interface ProcessMetricQuality {
  cpu?: MetricQualityInfo;
  memory?: MetricQualityInfo;
  disk?: MetricQualityInfo;
  other_io?: MetricQualityInfo;
  network?: MetricQualityInfo;
  threads?: MetricQualityInfo;
  handles?: MetricQualityInfo;
}

export type AccessState = "full" | "partial" | "denied";
export type SortColumn =
  | "attention"
  | "name"
  | "pid"
  | "cpu_pct"
  | "memory_bytes"
  | "disk_bps"
  | "threads"
  | "handles"
  | "start_time_ms";
export type SortDirection = "asc" | "desc";

export interface RuntimeQuery {
  filter_text: string;
  sort_column: SortColumn;
  sort_direction: SortDirection;
  limit: number;
}

export interface RuntimeSettings {
  query: RuntimeQuery;
  admin_mode_requested: boolean;
  admin_mode_enabled: boolean;
  metric_window_seconds: number;
  paused: boolean;
}

export interface RuntimeWarning {
  seq: number;
  ts_ms: number;
  category: string;
  message: string;
}

export interface TrendState {
  cpu: number[];
  memory: number[];
  swap: number[];
  diskRead: number[];
  diskWrite: number[];
  netRx: number[];
  netTx: number[];
  cores: number[][];
}
