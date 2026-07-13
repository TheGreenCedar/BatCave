export interface RuntimeSnapshot {
  event_kind: "runtime_snapshot";
  publication_seq: number;
  published_at_ms: number;
  sample_seq: number;
  sampled_at_ms: number | null;
  source: RuntimeTelemetrySource;
  environment: RuntimeEnvironment;
  admin_mode: RuntimeAdminModeStatus;
  settings: RuntimeSettings;
  health: RuntimeHealth;
  system: SystemMetricsSnapshot;
  process_contributors: ProcessContributorSummary;
  processes: ProcessSample[];
  process_view_rows: ProcessViewRow[];
  total_process_count: number;
  warnings: RuntimeWarning[];
}

export interface ProcessContributorSummary {
  cpu: string | null;
  memory: string | null;
  io: string | null;
  network: string | null;
}

export interface RuntimeEnvironment {
  platform: RuntimePlatform;
  admin_mode_available: boolean;
  install_kind: RuntimeInstallKind;
  data_directory: string | null;
}

export type RuntimePlatform = "windows" | "linux" | "macos" | "fixture";
export type RuntimeInstallKind = "nsis" | "appimage" | "deb" | "dmg" | "portable";

export type RuntimeAdminModeState =
  | "unavailable"
  | "off"
  | "requesting"
  | "active"
  | "recovering"
  | "failed";

export interface RuntimeAdminModeStatus {
  state: RuntimeAdminModeState;
  detail: string | null;
  last_success_at_ms: number | null;
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
  swap_used_bytes?: number;
  swap_total_bytes?: number;
  process_count: number;
  disk_read_total_bytes: number;
  disk_write_total_bytes: number;
  disk_read_bps: number;
  disk_write_bps: number;
  network_received_total_bytes: number;
  network_transmitted_total_bytes: number;
  network_received_bps: number;
  network_transmitted_bps: number;
  memory_accounting?: SystemMemoryAccounting;
  quality?: SystemMetricQuality;
}

export interface SystemMemoryAccounting {
  process_working_set_bytes: number;
  process_private_bytes: number;
  denied_process_count: number;
  partial_process_count: number;
  commit_used_bytes?: number;
  commit_limit_bytes?: number;
  system_cache_bytes?: number;
  kernel_total_bytes?: number;
  kernel_paged_pool_bytes?: number;
  kernel_nonpaged_pool_bytes?: number;
  kernel_pool_tags?: KernelPoolTag[];
}

export type KernelPoolKind = "paged" | "nonpaged";

export interface KernelPoolTag {
  tag: string;
  kind: KernelPoolKind;
  bytes: number;
  allocations: number;
  frees: number;
  driver_candidates: string[];
  driver_candidates_pending?: boolean;
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
  virtual_memory_bytes?: number;
  io_read_total_bytes: number;
  io_write_total_bytes: number;
  other_io_total_bytes?: number;
  io_read_bps: number;
  io_write_bps: number;
  other_io_bps?: number;
  network_received_bps?: number;
  network_transmitted_bps?: number;
  threads: number;
  handles: number;
  access_state: AccessState;
  quality?: ProcessMetricQuality;
}

export type ProcessViewRowKind = "group" | "process";

export interface ProcessViewRow {
  kind: ProcessViewRowKind;
  process?: ProcessSample;
  representative?: ProcessSample;
  group_key?: string;
  group_label?: string;
  group_category?: string;
  group_count: number;
  icon_kind: string;
  is_child: boolean;
  is_grouped: boolean;
  attention_label: string;
  cpu_percent: number;
  memory_bytes: number;
  io_bps: number;
  network_bps: number;
  threads: number;
}

export interface ProcessMetricQuality {
  cpu?: MetricQualityInfo;
  memory?: MetricQualityInfo;
  io?: MetricQualityInfo;
  other_io?: MetricQualityInfo;
  network?: MetricQualityInfo;
  threads?: MetricQualityInfo;
  handles?: MetricQualityInfo;
}

export type AccessState = "full" | "partial" | "denied";
export type ProcessFocusMode = "all" | "attention" | "io";
export type SortColumn =
  | "attention"
  | "name"
  | "pid"
  | "cpu_pct"
  | "memory_bytes"
  | "io_bps"
  | "network_bps"
  | "threads"
  | "handles"
  | "start_time_ms";
export type SortDirection = "asc" | "desc";

export interface RuntimeQuery {
  filter_text: string;
  focus_mode: ProcessFocusMode;
  sort_column: SortColumn;
  sort_direction: SortDirection;
  limit: number;
}

export interface RuntimeSettings {
  query: RuntimeQuery;
  admin_mode_requested: boolean;
  admin_mode_enabled: boolean;
  metric_window_seconds: number;
  sample_interval_ms: number;
  paused: boolean;
}

export interface RuntimeWarning {
  key: string;
  publication_seq: number;
  occurred_at_ms: number;
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
