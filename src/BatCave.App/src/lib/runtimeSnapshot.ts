import type { RuntimeQuery, RuntimeSnapshot } from "./types";

export function makeDefaultRuntimeQuery(): RuntimeQuery {
  return {
    filter_text: "",
    focus_mode: "all",
    sort_column: "attention",
    sort_direction: "desc",
    limit: 5000,
  };
}

export function makeEmptySnapshot(
  statusSummary = "Waiting for native telemetry.",
): RuntimeSnapshot {
  const now = Date.now();
  const platform =
    typeof navigator !== "undefined" && navigator.platform.toLocaleLowerCase().includes("linux")
      ? "linux"
      : "windows";

  return {
    event_kind: "runtime_snapshot",
    publication_seq: 0,
    published_at_ms: now,
    sample_seq: 0,
    sampled_at_ms: null,
    source: "tauri_runtime",
    environment: {
      platform,
      admin_mode_available: false,
      data_directory: null,
    },
    settings: {
      query: makeDefaultRuntimeQuery(),
      admin_mode_requested: false,
      admin_mode_enabled: false,
      metric_window_seconds: 60,
      paused: false,
    },
    health: {
      tick_count: 0,
      snapshot_latency_ms: 0,
      degraded: true,
      collector_warnings: statusSummary ? 1 : 0,
      runtime_loop_enabled: true,
      runtime_loop_running: false,
      status_summary: statusSummary,
      updated_at_ms: now,
      tick_p95_ms: 0,
      sort_p95_ms: 0,
      jitter_p95_ms: 0,
      dropped_ticks: 0,
      app_cpu_percent: 0,
      app_rss_bytes: 0,
      last_warning: statusSummary,
    },
    system: {
      cpu_percent: 0,
      kernel_cpu_percent: 0,
      logical_cpu_percent: [],
      memory_used_bytes: 0,
      memory_total_bytes: 0,
      memory_available_bytes: 0,
      process_count: 0,
      disk_read_total_bytes: 0,
      disk_write_total_bytes: 0,
      disk_read_bps: 0,
      disk_write_bps: 0,
      network_received_total_bytes: 0,
      network_transmitted_total_bytes: 0,
      network_received_bps: 0,
      network_transmitted_bps: 0,
      quality: {
        cpu: { quality: "unavailable", source: "runtime", message: statusSummary },
        kernel_cpu: { quality: "unavailable", source: "runtime", message: statusSummary },
        logical_cpu: { quality: "unavailable", source: "runtime", message: statusSummary },
        memory: { quality: "unavailable", source: "runtime", message: statusSummary },
        swap: { quality: "unavailable", source: "runtime", message: statusSummary },
        disk: { quality: "unavailable", source: "runtime", message: statusSummary },
        network: { quality: "unavailable", source: "runtime", message: statusSummary },
      },
    },
    processes: [],
    process_view_rows: [],
    total_process_count: 0,
    warnings: [],
  };
}

export function hasNewRuntimeSample(
  current: Pick<RuntimeSnapshot, "sample_seq">,
  incoming: Pick<RuntimeSnapshot, "sample_seq">,
): boolean {
  return incoming.sample_seq > current.sample_seq;
}
