import type { RuntimePlatform, RuntimeQuery, RuntimeSnapshot } from "./types";

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
  const platform = browserPlatform();

  return {
    event_kind: "runtime_snapshot",
    publication_seq: 0,
    published_at_ms: now,
    sample_seq: 0,
    sampled_at_ms: null,
    source: "tauri_runtime",
    environment: {
      platform,
      architecture: "unknown",
      admin_mode_available: false,
      process_elevation: "not_applicable",
      install_kind: "portable",
      data_directory: null,
      release_identity: { app_version: "development", source_commit_sha: null },
    },
    admin_mode: {
      state: "unavailable",
      source: "none",
      detail: null,
      last_success_at_ms: null,
    },
    settings: {
      query: makeDefaultRuntimeQuery(),
      admin_mode_requested: false,
      admin_mode_enabled: false,
      metric_window_seconds: 60,
      sample_interval_ms: 1000,
      paused: false,
    },
    health: {
      engine_state: null,
      collector_state: null,
      degraded: true,
      status_summary: statusSummary,
      evaluated_at_ms: now,
      last_heartbeat_at_ms: null,
      heartbeat_age_ms: null,
      publication_age_ms: 0,
      sample_age_ms: null,
      deadline_misses: null,
      deadline_lateness_p95_ms: null,
      collection_latency_ms: null,
      collection_p95_ms: null,
      publication_latency_ms: null,
      publication_p95_ms: null,
      collector_warning_count: statusSummary ? 1 : 0,
      app_cpu_percent: 0,
      app_rss_bytes: 0,
      last_warning: statusSummary,
      fatal_error: null,
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
    process_contributors: {
      cpu: null,
      cpu_process_id: null,
      cpu_coverage: { available: 0, total: 0 },
      cpu_name_ambiguous: false,
      memory: null,
      memory_process_id: null,
      memory_coverage: { available: 0, total: 0 },
      memory_name_ambiguous: false,
      io: null,
      io_process_id: null,
      io_coverage: { available: 0, total: 0 },
      io_name_ambiguous: false,
      network: null,
      network_process_id: null,
      network_coverage: { available: 0, total: 0 },
      network_name_ambiguous: false,
    },
    processes: [],
    process_view_rows: [],
    total_process_count: 0,
    warnings: [],
  };
}

function browserPlatform(): RuntimePlatform {
  if (typeof navigator === "undefined") return "windows";
  const value = `${navigator.platform} ${navigator.userAgent}`.toLocaleLowerCase();
  if (value.includes("mac")) return "macos";
  if (value.includes("linux")) return "linux";
  return "windows";
}

export function hasNewRuntimeSample(
  current: Pick<RuntimeSnapshot, "sample_seq">,
  incoming: Pick<RuntimeSnapshot, "sample_seq">,
): boolean {
  return incoming.sample_seq > current.sample_seq;
}

export function shouldApplyRuntimePublication(
  current: Pick<RuntimeSnapshot, "publication_seq">,
  incoming: Pick<RuntimeSnapshot, "publication_seq">,
): boolean {
  return incoming.publication_seq >= current.publication_seq;
}

export function shouldPollRuntime(paused: boolean, nativeRuntime: boolean): boolean {
  return !paused || nativeRuntime;
}
