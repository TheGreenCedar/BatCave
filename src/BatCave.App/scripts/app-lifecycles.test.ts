import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { stripTypeScriptTypes } from "node:module";
import test from "node:test";
import { runInNewContext } from "node:vm";
import { makeFixtureSnapshot } from "../src/lib/fixtures.ts";
import {
  advanceProcessRanking,
  countWorkloadRows,
  defaultSortDirection,
  nextSortDirection,
  prepareProcessViewRows,
  processViewRowKey,
  sortColumnForKey,
  sortKeyForColumn,
} from "../src/lib/process.ts";
import { AcceptedRuntimeControls } from "../src/lib/runtimeControls.ts";
import {
  makeDefaultRuntimeQuery,
  shouldApplyRuntimePublication,
} from "../src/lib/runtimeSnapshot.ts";
import { startRuntimePolling, type PollScheduler } from "../src/lib/runtimePolling.ts";
import { StableUpdateController } from "../src/lib/stableUpdate.ts";
import {
  combineSeries,
  emptyProcessTrendState,
  emptyTrendState,
  initialWorkloadTrend,
  historyGapPoints,
  nextSystemHistory,
  nextWorkloadTrend,
  processRatesFromSamples,
  replaySystemHistory,
  trimProcessHistory,
  trimSystemHistory,
} from "../src/lib/telemetryHistory.ts";
import type { RuntimeQuery, RuntimeSnapshot, SystemHistoryPoint } from "../src/lib/types.ts";
import type { InstallableUpdateResource } from "../src/lib/updateLifecycle.ts";
import {
  decodeRuntimeSnapshot,
  observeAcceptedRuntimePublication,
} from "../src/lib/tauriBridge.ts";

test("publication observations exclude discarded and duplicate responses and retain transport timing", () => {
  const wire: unknown = JSON.parse(
    readFileSync(
      new URL(
        "../src-tauri/src/fixtures/runtime-protocol-v4/windows-standard.json",
        import.meta.url,
      ),
      "utf8",
    ),
  );
  const observations: Array<{ sequence: number; elapsed: number }> = [];
  const observe = (snapshot: ReturnType<typeof decodeRuntimeSnapshot>, elapsed: number) => {
    observations.push({ sequence: snapshot.publication_seq, elapsed });
  };
  const discarded = decodeRuntimeSnapshot(wire, 120);
  const duplicate = decodeRuntimeSnapshot(wire, 240);
  const accepted = decodeRuntimeSnapshot(wire, 360);
  assert.deepEqual(observations, [], "decoding alone never observes a publication");
  observeAcceptedRuntimePublication(
    { publication_seq: discarded.publication_seq + 1 },
    discarded,
    observe,
  );
  observeAcceptedRuntimePublication(
    { publication_seq: duplicate.publication_seq },
    duplicate,
    observe,
  );
  assert.deepEqual(observations, [], "older and equal publications cannot be recorded as painted");
  observeAcceptedRuntimePublication(
    { publication_seq: accepted.publication_seq - 1 },
    accepted,
    observe,
  );
  assert.equal(observations.length, 1);
  assert.equal(observations[0]?.sequence, accepted.publication_seq);
  assert.ok(
    observations[0] && observations[0].elapsed >= 360,
    "transport and decode time survive acceptance",
  );
  observeAcceptedRuntimePublication(
    { publication_seq: accepted.publication_seq - 1 },
    accepted,
    observe,
  );
  assert.equal(observations.length, 1, "timing metadata is consumed once");
  assert.equal(
    JSON.stringify(accepted).includes("transportElapsed"),
    false,
    "timing remains outside the protocol payload",
  );
});

class ManualScheduler implements PollScheduler {
  private nextId = 1;
  readonly scheduled: Array<{ id: number; callback: () => void; delayMs: number }> = [];
  readonly cleared: number[] = [];

  setTimeout(callback: () => void, delayMs: number): number {
    const id = this.nextId++;
    this.scheduled.push({ id, callback, delayMs });
    return id;
  }

  clearTimeout(timeoutId: number): void {
    this.cleared.push(timeoutId);
  }

  readonly intervals: Array<{ id: number; callback: () => void; intervalMs: number }> = [];
  readonly clearedIntervals: number[] = [];

  setInterval(callback: () => void, intervalMs: number): number {
    const id = this.nextId++;
    this.intervals.push({ id, callback, intervalMs });
    return id;
  }

  clearInterval(intervalId: number): void {
    this.clearedIntervals.push(intervalId);
  }

  async runNext(): Promise<void> {
    const task = this.scheduled.shift();
    assert.ok(task);
    task.callback();
    await new Promise<void>((resolve) => setImmediate(resolve));
  }
}

test("runtime polling reads the current cadence only after each poll completes", async () => {
  const scheduler = new ManualScheduler();
  const events: string[] = [];
  let intervalMs = 500;
  const stop = startRuntimePolling({
    initialDelayMs: 120,
    intervalMs: () => intervalMs,
    poll: async () => {
      events.push("poll");
      intervalMs = 2_000;
    },
    scheduler,
  });

  assert.equal(scheduler.scheduled[0]?.delayMs, 120);
  await scheduler.runNext();
  assert.deepEqual(events, ["poll"]);
  assert.equal(scheduler.scheduled[0]?.delayMs, 2_000);

  stop();
  assert.deepEqual(scheduler.cleared, [3]);
  assert.equal(scheduler.intervals.length, 1, "watchdog armed");
  assert.equal(scheduler.clearedIntervals.length, 1, "watchdog cleared on dispose");
});

test("runtime polling does not reschedule after disposal during an in-flight poll", async () => {
  const scheduler = new ManualScheduler();
  let releasePoll: (() => void) | undefined;
  const stop = startRuntimePolling({
    initialDelayMs: 120,
    intervalMs: () => 1_000,
    poll: () => new Promise<void>((resolve) => (releasePoll = resolve)),
    scheduler,
  });

  const initial = scheduler.scheduled.shift();
  assert.ok(initial);
  initial.callback();
  stop();
  releasePoll?.();
  await new Promise<void>((resolve) => setImmediate(resolve));

  assert.equal(scheduler.scheduled.length, 0);
  assert.deepEqual(scheduler.cleared, [initial.id]);
});

class FixtureUpdate implements InstallableUpdateResource {
  readonly version = "2.0.0";
  closeCalls = 0;
  installCalls = 0;
  closeError: Error | null;
  private readonly installError: Error | null;

  constructor(closeError: Error | null = null, installError: Error | null = null) {
    this.closeError = closeError;
    this.installError = installError;
  }

  async close(): Promise<void> {
    this.closeCalls += 1;
    if (this.closeError) throw this.closeError;
  }

  async downloadAndInstall(): Promise<void> {
    this.installCalls += 1;
    if (this.installError) throw this.installError;
  }
}

test("stable update controller exposes check and install transitions", async () => {
  const update = new FixtureUpdate();
  let checks = 0;
  const controller = new StableUpdateController(async () => {
    checks += 1;
    return update;
  });
  const states: string[] = [];

  await controller.check("appimage", (state) => states.push(`${state.status}:${state.message}`));
  await controller.install((state) => states.push(`${state.status}:${state.message}`));

  assert.deepEqual(states, [
    "checking:Checking the stable release channel…",
    "available:Version 2.0.0 is available.",
    "installing:Downloading, verifying, and installing the update…",
    "restart_required:Version 2.0.0 is installed. Close and reopen BatCave to use it.",
  ]);
  const completed = controller.state();
  assert.equal(completed.status, "restart_required");
  await controller.check("appimage", () => assert.fail("completed update cannot check again"));
  await controller.install(() => assert.fail("completed update cannot install again"));
  assert.deepEqual(controller.state(), completed);
  assert.equal(checks, 1);
  assert.equal(update.installCalls, 1);
  assert.equal(update.closeCalls, 1);
});

test("stable update controller retains a cleanup-failed handle for retry", async () => {
  const update = new FixtureUpdate(new Error("resource table unavailable"));
  let checks = 0;
  const controller = new StableUpdateController(async () => {
    checks += 1;
    return update;
  });
  const messages: string[] = [];

  await controller.check("appimage", (state) => messages.push(state.message));
  await controller.check("appimage", (state) => messages.push(state.message));

  assert.equal(checks, 1);
  assert.equal(update.closeCalls, 1);
  assert.equal(
    messages.at(-1),
    "Unable to check for updates. Monitoring remains available offline.",
  );
  update.closeError = null;
  await controller.check("appimage", (state) => messages.push(state.message));
  assert.equal(update.closeCalls, 2, "retry closes the retained handle before replacement");
  assert.equal(checks, 2);
  assert.equal(controller.state().status, "available");
});

test("stable update controller retries cleanup after installation and release both fail", async () => {
  const update = new FixtureUpdate(
    new Error("resource table unavailable"),
    new Error("signature rejected"),
  );
  let checks = 0;
  const controller = new StableUpdateController(async () => {
    checks += 1;
    return update;
  });
  await controller.check("appimage", () => {});
  await controller.install(() => {});
  assert.equal(controller.state().status, "error");
  assert.equal(
    controller.state().message,
    "Update failed and its local selection could not be released. Retry will clean it up before checking again.",
  );
  assert.equal(update.closeCalls, 1);
  update.closeError = null;
  await controller.check("appimage", () => {});
  assert.equal(update.closeCalls, 2);
  assert.equal(checks, 2);
  assert.equal(controller.state().status, "available");
});

test("Debian update checks remain package-manager guidance without invoking the updater", async () => {
  let checks = 0;
  const controller = new StableUpdateController<FixtureUpdate>(async () => {
    checks += 1;
    return null;
  });
  const states: string[] = [];

  await controller.check("deb", (state) => states.push(`${state.status}:${state.message}`));

  assert.equal(checks, 0);
  assert.deepEqual(states, [
    "current:Debian packages update through your package manager or a downloaded .deb release.",
  ]);
});

test("system history advances only through quality-aware bounded series", () => {
  const snapshot = makeFixtureSnapshot(8);
  const first = nextSystemHistory(emptyTrendState(), snapshot, 2);
  const second = nextSystemHistory(first, snapshot, 2);
  const third = nextSystemHistory(second, snapshot, 2);

  assert.equal(third.cpu.length, 2);
  assert.equal(third.memory.length, 2);
  assert.equal(third.diskRead.length, 2);
  assert.equal(third.netRx.length, 2);
  assert.equal(third.cores.length, snapshot.system.logical_cpu_percent.length);

  snapshot.system.quality!.cpu = {
    quality: "unavailable",
    reason: "collector unavailable",
  };
  assert.deepEqual(nextSystemHistory(third, snapshot, 2).cpu, []);
});

test("workload history uses accepted process rates and clears unpublishable group metrics", () => {
  const snapshot = makeFixtureSnapshot(8);
  const processWorkload = snapshot.process_view_rows.find(
    (row) => row.kind === "process" && !row.is_grouped,
  )?.detail;
  assert.equal(processWorkload?.kind, "process");
  if (!processWorkload || processWorkload.kind !== "process") return;

  const rates = processRatesFromSamples(snapshot.processes);
  const first = initialWorkloadTrend(processWorkload, snapshot.system.memory_total_bytes, rates, 2);
  const next = nextWorkloadTrend(
    first,
    processWorkload,
    snapshot.system.memory_total_bytes,
    rates,
    2,
  );
  assert.deepEqual(next.readRate, [
    processWorkload.process.io_read_bps,
    processWorkload.process.io_read_bps,
  ]);

  const group = snapshot.process_view_rows.find((row) => row.kind === "group")?.detail;
  assert.equal(group?.kind, "group");
  if (!group || group.kind !== "group") return;
  group.quality.network = { quality: "unavailable", reason: "not sampled" };
  group.coverage.network = { available: 0, total: group.process_count };
  const groupTrend = nextWorkloadTrend(
    { ...emptyProcessTrendState(), networkRate: [99] },
    group,
    snapshot.system.memory_total_bytes,
    rates,
    2,
  );
  assert.deepEqual(groupTrend.networkRate, []);
});

test("history trimming and aligned combined series preserve chart semantics", () => {
  const system = {
    ...emptyTrendState(),
    cpu: [1, 2, 3],
    cores: [
      [1, 2, 3],
      [4, 5, 6],
    ],
  };
  const process = {
    ...emptyProcessTrendState(),
    readRate: [1, 2, 3],
    writeRate: [4, 5, 6],
  };

  assert.deepEqual(trimSystemHistory(system, 2).cpu, [2, 3]);
  assert.deepEqual(trimSystemHistory(system, 2).cores, [
    [2, 3],
    [5, 6],
  ]);
  assert.deepEqual(trimProcessHistory(process, 2).readRate, [2, 3]);
  assert.deepEqual(combineSeries([2], [10, 20]), [10, 22]);
});

const appSource = readFileSync(new URL("../src/App.svelte", import.meta.url), "utf8");

function loadAppFunctions(context: object, names: string[]): void {
  const functions = names.map((name) => {
    const match = appSource.match(
      new RegExp(
        `  (?:async )?function ${name}\\([\\s\\S]*?(?=\\n  (?:async )?function |\\n</script>)`,
      ),
    );
    assert.ok(match, `App.svelte function ${name} exists`);
    return match[0];
  });
  runInNewContext(stripTypeScriptTypes(functions.join("\n")), context);
}

function deferred<T>() {
  let release: (value: T) => void = () => assert.fail("promise resolver was not initialized");
  const promise = new Promise<T>((resolve) => {
    release = resolve;
  });
  return { promise, resolve: (value: T) => release(value) };
}

function historySnapshot(seq: number): RuntimeSnapshot {
  const snapshot = makeFixtureSnapshot(seq);
  snapshot.sample_seq = seq;
  snapshot.system.cpu_percent = seq;
  return snapshot;
}

function historyBackfillHarness() {
  const result = deferred<SystemHistoryPoint[]>();
  const snapshot = historySnapshot(10);
  const context = {
    snapshot,
    history: nextSystemHistory(emptyTrendState(), snapshot, 72),
    historyPointLimit: 72,
    historyGeneration: 0,
    lastHistorySampleSeq: 10,
    runtimeMode: () => "native",
    invoke: () => {},
    readSystemHistory: () => result.promise,
    historyGapPoints,
    replaySystemHistory,
    emptyTrendState,
    backfillHistoryGap: async (_snapshot: RuntimeSnapshot) => {},
    resetHistory: () => {},
  };
  loadAppFunctions(context, ["backfillHistoryGap", "resetHistory"]);
  const resolve = () =>
    result.resolve(
      [11, 12].map((seq) => ({
        sample_seq: seq,
        sampled_at_ms: seq * 1_000,
        system: historySnapshot(seq).system,
      })),
    );
  return { context, resolve };
}

test("delayed system history cannot replay behind a newer accepted sample", async () => {
  const { context, resolve } = historyBackfillHarness();
  const backfill = context.backfillHistoryGap(historySnapshot(13));
  context.snapshot = historySnapshot(14);
  context.history = nextSystemHistory(context.history, context.snapshot, 72);
  context.lastHistorySampleSeq = 14;
  resolve();
  await backfill;
  assert.deepEqual(context.history.cpu, [10, 14]);
  assert.equal(context.lastHistorySampleSeq, 14);
});

test("delayed system history cannot restore points after reset at the same sample", async () => {
  const { context, resolve } = historyBackfillHarness();
  const backfill = context.backfillHistoryGap(historySnapshot(13));
  context.resetHistory();
  resolve();
  await backfill;
  assert.deepEqual(context.history.cpu, []);
  assert.equal(context.lastHistorySampleSeq, 10);
});

test("delayed system history only replays points newer than the current cursor", async () => {
  const { context, resolve } = historyBackfillHarness();
  const backfill = context.backfillHistoryGap(historySnapshot(13));
  context.snapshot = historySnapshot(11);
  context.history = nextSystemHistory(context.history, context.snapshot, 72);
  context.lastHistorySampleSeq = 11;
  resolve();
  await backfill;
  assert.deepEqual(context.history.cpu, [10, 11, 12]);
  assert.equal(context.lastHistorySampleSeq, 12);
});

function queryRankingHarness() {
  const result = deferred<RuntimeSnapshot>();
  const initial = makeFixtureSnapshot(8);
  const rows = initial.process_view_rows.filter((row) => row.kind === "process").slice(0, 2);
  assert.equal(rows.length, 2);
  const scheduler = new ManualScheduler();
  const context = {
    protocolMismatch: null,
    runtimeMutationAllowed: () => true,
    runtimeMode: () => "native",
    invoke: () => {},
    setRuntimeProcessQuery: () => result.promise,
    refreshRuntime: () => result.promise,
    runtimeQueryRequestSeq: 0,
    acceptedRuntimeControls: new AcceptedRuntimeControls(makeDefaultRuntimeQuery(), 1000),
    searchText: "",
    focusMode: "all",
    sortKey: "attention",
    sortDirection: "desc",
    searchDebounceId: undefined,
    window: scheduler,
    commandErrorSurface: "global",
    commandError: "",
    runtimeCommandError: () => "query failed",
    snapshot: { ...initial, process_view_rows: rows },
    displayProcessRows: rows,
    pendingProcessRows: null,
    rankingUpdateAvailable: false,
    exitedRowKeys: new Set<string>(),
    selectedWorkloadId: "",
    matchedWorkloadCount: 2,
    shouldHoldRanking: () => true,
    countWorkloadRows,
    prepareProcessViewRows,
    advanceProcessRanking,
    defaultSortDirection,
    nextSortDirection,
    sortColumnForKey,
    sortKeyForColumn,
    makeDefaultRuntimeQuery,
    ingest: (snapshot: RuntimeSnapshot) => {
      if (!shouldApplyRuntimePublication(context.snapshot, snapshot)) return;
      context.snapshot = snapshot;
      context.acceptedRuntimeControls.observe(snapshot);
      context.updateProcessRows(snapshot.process_view_rows);
    },
    applyNativeSnapshot: (snapshot: RuntimeSnapshot) => context.ingest(snapshot),
    updateProcessRows: (_rows: RuntimeSnapshot["process_view_rows"]) => {},
    applyPendingRanking: () => {},
    restoreRuntimeQueryControls: (_query: RuntimeQuery) => {},
    currentRuntimeQuery: makeDefaultRuntimeQuery,
    syncRuntimeQuery: async (_query?: RuntimeQuery) => {},
    setSearchText: (_value: string) => {},
    setSortKey: (_value: string) => {},
    setFocusMode: (_value: string) => {},
    refreshNow: async () => {},
  };
  loadAppFunctions(context, [
    "setSearchText",
    "setSortKey",
    "setFocusMode",
    "flushRuntimeQuery",
    "currentRuntimeQuery",
    "syncRuntimeQuery",
    "restoreRuntimeQueryControls",
    "updateProcessRows",
    "applyPendingRanking",
    "refreshNow",
  ]);
  const response = {
    ...initial,
    publication_seq: initial.publication_seq + 2,
    process_view_rows: rows.slice(1),
  };
  return { context, scheduler, result, rows, response };
}

test("search adoption survives a poll during debounce and another while the query is pending", async () => {
  const { context, scheduler, result, rows, response } = queryRankingHarness();
  context.setSearchText("query");
  context.ingest({
    ...response,
    publication_seq: response.publication_seq - 1,
    process_view_rows: rows,
  });
  const debounce = scheduler.scheduled.shift();
  assert.ok(debounce);
  debounce.callback();
  context.ingest({ ...response, publication_seq: response.publication_seq + 1 });
  assert.equal(context.displayProcessRows.length, 2, "polling holds rows during interaction");
  result.resolve(response);
  await new Promise<void>((resolve) => setImmediate(resolve));
  assert.deepEqual(
    context.displayProcessRows.map(processViewRowKey),
    response.process_view_rows.map(processViewRowKey),
    "the acknowledged query adopts the newer poll even when its own publication is older",
  );
  assert.equal(context.rankingUpdateAvailable, false);
});

test("sort and focus query acknowledgements replace held rows after intervening polls", async () => {
  for (const change of [
    (context: ReturnType<typeof queryRankingHarness>["context"]) => context.setSortKey("cpu"),
    (context: ReturnType<typeof queryRankingHarness>["context"]) =>
      context.setFocusMode("attention"),
  ]) {
    const { context, result, response } = queryRankingHarness();
    change(context);
    context.ingest({ ...response, publication_seq: response.publication_seq - 1 });
    result.resolve(response);
    await new Promise<void>((resolve) => setImmediate(resolve));
    assert.deepEqual(
      context.displayProcessRows.map(processViewRowKey),
      response.process_view_rows.map(processViewRowKey),
    );
    assert.equal(context.pendingProcessRows, null);
  }
});

test("explicit refresh adopts its accepted ranking while the workload table is held", async () => {
  const { context, result, response } = queryRankingHarness();
  const refresh = context.refreshNow();
  context.ingest({ ...response, publication_seq: response.publication_seq - 1 });
  result.resolve(response);
  await refresh;
  assert.deepEqual(
    context.displayProcessRows.map(processViewRowKey),
    response.process_view_rows.map(processViewRowKey),
  );
  assert.equal(context.pendingProcessRows, null);
});
