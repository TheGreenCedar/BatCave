export interface RuntimeSnapshot {
  event_kind: "runtime_snapshot";
  seq: number;
  ts_ms: number;
  source: "tauri_sysinfo" | "fixture";
  health: RuntimeHealth;
  system: SystemMetricsSnapshot;
  processes: ProcessSample[];
  warnings: string[];
}

export interface RuntimeHealth {
  tick_count: number;
  snapshot_latency_ms: number;
  degraded: boolean;
  collector_warnings: number;
}

export interface SystemMetricsSnapshot {
  cpu_percent: number;
  kernel_cpu_percent: number;
  logical_cpu_percent: number[];
  memory_used_bytes: number;
  memory_total_bytes: number;
  swap_used_bytes: number;
  swap_total_bytes: number;
  process_count: number;
  disk_read_total_bytes: number;
  disk_write_total_bytes: number;
  network_received_total_bytes: number;
  network_transmitted_total_bytes: number;
}

export interface ProcessSample {
  pid: string;
  parent_pid: string | null;
  name: string;
  exe: string;
  status: string;
  cpu_percent: number;
  memory_bytes: number;
  virtual_memory_bytes: number;
  disk_read_total_bytes: number;
  disk_write_total_bytes: number;
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
