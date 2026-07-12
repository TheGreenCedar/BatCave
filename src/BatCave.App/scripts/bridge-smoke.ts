import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import {
  getRuntimeProcessIcons,
  readNativeSnapshot,
  refreshRuntime,
  setRuntimePaused,
  setRuntimeProcessQuery,
  setRuntimeSampleInterval,
  type RuntimeInvoke,
} from "../src/lib/tauriBridge.ts";

const canonicalSnapshot = JSON.parse(
  readFileSync(new URL("./fixtures/runtime-snapshot.v2.json", import.meta.url), "utf8"),
);
assert.equal(canonicalSnapshot.publication_seq, 42);
assert.equal(canonicalSnapshot.seq, undefined);
assert.deepEqual(
  canonicalSnapshot.system.memory_accounting.kernel_pool_tags[0].driver_candidates,
  [],
);

function snapshot(seq: number) {
  return {
    event_kind: "runtime_snapshot",
    publication_seq: seq,
    published_at_ms: seq,
    sample_seq: seq,
    sampled_at_ms: seq || null,
    source: "batcave_runtime",
    environment: {
      platform: "windows",
      admin_mode_available: true,
      install_kind: "nsis",
      data_directory: "C:\\Users\\test\\BatCaveMonitor",
    },
    settings: {
      query: {
        filter_text: "",
        focus_mode: "all",
        sort_column: "attention",
        sort_direction: "desc",
        limit: 5000,
      },
      admin_mode_requested: false,
      admin_mode_enabled: false,
      metric_window_seconds: 60,
      sample_interval_ms: 1000,
      paused: false,
    },
    health: {
      tick_count: 0,
      snapshot_latency_ms: 0,
      degraded: false,
      collector_warnings: 0,
      runtime_loop_enabled: true,
      runtime_loop_running: true,
      status_summary: "ok",
      updated_at_ms: seq,
      tick_p95_ms: 0,
      sort_p95_ms: 0,
      jitter_p95_ms: 0,
      dropped_ticks: 0,
      app_cpu_percent: 0,
      app_rss_bytes: 0,
      last_warning: null,
    },
    system: {
      cpu_percent: 0,
      kernel_cpu_percent: 0,
      logical_cpu_percent: [],
      memory_used_bytes: 0,
      memory_total_bytes: 1,
      memory_available_bytes: 1,
      process_count: 0,
      disk_read_total_bytes: 0,
      disk_write_total_bytes: 0,
      disk_read_bps: 0,
      disk_write_bps: 0,
      network_received_total_bytes: 0,
      network_transmitted_total_bytes: 0,
      network_received_bps: 0,
      network_transmitted_bps: 0,
    },
    process_contributors: { cpu: null, memory: null, disk: null, network: null },
    processes: [],
    process_view_rows: [],
    total_process_count: 0,
    warnings: [],
  };
}

const calls: Array<{ command: string; args?: Record<string, unknown> }> = [];
const invoke: RuntimeInvoke = async (command, args) => {
  calls.push({ command, args });
  if (command === "get_process_icons") {
    throw new Error("access denied");
  }

  return snapshot(calls.length);
};

const successfulRead = await readNativeSnapshot(
  async (command) => {
    assert.equal(command, "get_snapshot");
    return snapshot(7);
  },
  {
    currentSnapshot: snapshot(99),
    emptySnapshot: () => snapshot(0),
    hasNativeSnapshot: false,
  },
);
assert.equal(successfulRead.ok, true);
assert.equal(successfulRead.error, "");
assert.equal(successfulRead.snapshot.publication_seq, 7);

await setRuntimePaused(invoke, true);
await setRuntimePaused(invoke, false);
await refreshRuntime(invoke);
await setRuntimeSampleInterval(invoke, 2000);
await setRuntimeProcessQuery(invoke, {
  filter_text: "chrome",
  focus_mode: "io",
  sort_column: "network_bps",
  sort_direction: "desc",
  limit: 25,
});
assert.deepEqual(await getRuntimeProcessIcons(invoke, ["C:\\Windows\\explorer.exe"]), {});

assert.deepEqual(
  calls.map((call) => [call.command, call.args]),
  [
    ["pause_runtime", undefined],
    ["resume_runtime", undefined],
    ["refresh_now", undefined],
    ["set_sample_interval", { sampleIntervalMs: 2000 }],
    [
      "set_process_query",
      {
        query: {
          filter_text: "chrome",
          focus_mode: "io",
          sort_column: "network_bps",
          sort_direction: "desc",
          limit: 25,
        },
      },
    ],
    ["get_process_icons", { exes: ["C:\\Windows\\explorer.exe"] }],
  ],
);

const emptySnapshot = snapshot(0);
const failedRead = await readNativeSnapshot(
  async () => {
    throw new Error("native down");
  },
  {
    currentSnapshot: snapshot(99),
    emptySnapshot: (message) => ({
      ...emptySnapshot,
      health: { ...emptySnapshot.health, status_summary: message },
    }),
    hasNativeSnapshot: false,
  },
);
assert.equal(failedRead.ok, false);
assert.equal(failedRead.error, "native down");
assert.equal(failedRead.snapshot.health.status_summary, "native down");

const previous = snapshot(42);
const heldRead = await readNativeSnapshot(
  async () => {
    throw "still down";
  },
  {
    currentSnapshot: previous,
    emptySnapshot: () => emptySnapshot,
    hasNativeSnapshot: true,
  },
);
assert.equal(heldRead.snapshot, previous);
assert.equal(heldRead.error, "still down");

console.log("bridge smoke passed");
