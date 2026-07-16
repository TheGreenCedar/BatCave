import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import {
  getRuntimeProcessIcons,
  readNativeSnapshot,
  refreshRuntime,
  runtimeMutationAllowed,
  setRuntimePaused,
  setRuntimeProcessQuery,
  setRuntimeSampleInterval,
  setRuntimeUiPreferences,
  type RuntimeInvoke,
} from "../src/lib/tauriBridge.ts";
import { encodeFixtureSnapshot } from "../src/lib/protocol/fixtureProtocol.ts";
import { runtimeSurfaceMode } from "../src/lib/runtimeMode.ts";

assert.equal(runtimeSurfaceMode(true, false), "native");
assert.equal(runtimeSurfaceMode(false, true), "fixture");
assert.equal(runtimeSurfaceMode(false, false), "unavailable");

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
      architecture: "unknown",
      admin_mode_available: true,
      process_elevation: "standard",
      install_kind: "nsis",
      data_directory: "C:\\Users\\test\\BatCaveMonitor",
      release_identity: { app_version: "development", source_commit_sha: null },
    },
    admin_mode: {
      state: "off",
      source: "none",
      detail: null,
      last_success_at_ms: null,
    },
    settings: {
      query: {
        filter_text: "",
        focus_mode: "all",
        sort_column: "attention",
        sort_direction: "desc",
        limit: 5000,
      },
      metric_window_seconds: 60,
      sample_interval_ms: 1000,
      paused: false,
    },
    health: {
      engine_state: null,
      collector_state: null,
      degraded: false,
      status_summary: "ok",
      evaluated_at_ms: seq,
      last_heartbeat_at_ms: null,
      heartbeat_age_ms: null,
      publication_age_ms: 0,
      sample_age_ms: 0,
      deadline_misses: null,
      deadline_lateness_p95_ms: null,
      collection_latency_ms: null,
      collection_p95_ms: null,
      publication_latency_ms: null,
      publication_p95_ms: null,
      collector_warning_count: 0,
      app_cpu_percent: 0,
      app_rss_bytes: 0,
      last_warning: null,
      fatal_error: null,
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

const calls: Array<{ command: string; args?: Record<string, unknown> }> = [];
const invoke: RuntimeInvoke = async (command, args) => {
  calls.push({ command, args });
  if (command === "get_process_icons") {
    throw new Error("access denied");
  }

  return encodeFixtureSnapshot(snapshot(calls.length));
};

const successfulRead = await readNativeSnapshot(
  async (command) => {
    assert.equal(command, "get_snapshot");
    return encodeFixtureSnapshot(snapshot(7));
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
await setRuntimeProcessQuery(
  invoke,
  {
    filter_text: "chrome",
    focus_mode: "attention",
    sort_column: "attention",
    sort_direction: "desc",
    limit: 25,
  },
  "runtime_only",
);
await setRuntimeUiPreferences(invoke, { theme: "ember", history_point_limit: 180 });
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
        persist: true,
      },
    ],
    [
      "set_process_query",
      {
        query: {
          filter_text: "chrome",
          focus_mode: "attention",
          sort_column: "attention",
          sort_direction: "desc",
          limit: 25,
        },
        persist: false,
      },
    ],
    ["set_ui_preferences", { preferences: { theme: "ember", history_point_limit: 180 } }],
    ["get_process_icons", { exes: ["C:\\Windows\\explorer.exe"] }],
  ],
);

const orderedCalls: string[] = [];
const orderedResolutions: Array<(value: unknown) => void> = [];
const orderedInvoke: RuntimeInvoke = <T>(command: string) =>
  new Promise<T>((resolve) => {
    orderedCalls.push(command);
    orderedResolutions.push((value) => resolve(value as T));
  });
const orderedMutations = [
  setRuntimePaused(orderedInvoke, true),
  setRuntimePaused(orderedInvoke, false),
  setRuntimeSampleInterval(orderedInvoke, 500),
  setRuntimeProcessQuery(
    orderedInvoke,
    {
      filter_text: "newest",
      focus_mode: "attention",
      sort_column: "attention",
      sort_direction: "desc",
      limit: 25,
    },
    "user_mutation",
  ),
  setRuntimeUiPreferences(orderedInvoke, { theme: "daylight", history_point_limit: 360 }),
];
const orderedCommands = [
  "pause_runtime",
  "resume_runtime",
  "set_sample_interval",
  "set_process_query",
  "set_ui_preferences",
];
await Promise.resolve();
assert.deepEqual(orderedCalls, orderedCommands.slice(0, 1));
for (let index = 0; index < orderedMutations.length; index += 1) {
  orderedResolutions[index](encodeFixtureSnapshot(snapshot(100 + index)));
  await orderedMutations[index];
  await Promise.resolve();
  assert.deepEqual(
    orderedCalls,
    orderedCommands.slice(0, Math.min(index + 2, orderedCommands.length)),
  );
}

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

const incompatible = JSON.parse(
  readFileSync(
    new URL("../src-tauri/src/fixtures/runtime-protocol-v3/incompatible.json", import.meta.url),
    "utf8",
  ),
);
const mismatchRead = await readNativeSnapshot(async () => incompatible, {
  currentSnapshot: previous,
  emptySnapshot: (message) => ({
    ...emptySnapshot,
    health: { ...emptySnapshot.health, status_summary: message },
  }),
  hasNativeSnapshot: true,
});
assert.equal(mismatchRead.ok, false);
assert.notEqual(mismatchRead.snapshot, previous);
assert.equal(mismatchRead.snapshot.publication_seq, 0);
assert.equal(mismatchRead.mismatch?.reason, "reader_too_old");

let blockedMutationCalls = 0;
if (runtimeMutationAllowed(mismatchRead.mismatch)) {
  blockedMutationCalls += 1;
}
assert.equal(blockedMutationCalls, 0);

console.log("bridge smoke passed");
