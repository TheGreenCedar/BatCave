import assert from "node:assert/strict";
import test from "node:test";
import { makeFixtureSnapshot } from "../src/lib/fixtures.ts";
import { startRuntimePolling, type PollScheduler } from "../src/lib/runtimePolling.ts";
import { StableUpdateController } from "../src/lib/stableUpdate.ts";
import {
  combineSeries,
  emptyProcessTrendState,
  emptyTrendState,
  initialWorkloadTrend,
  nextSystemHistory,
  nextWorkloadTrend,
  processRatesFromSamples,
  trimProcessHistory,
  trimSystemHistory,
} from "../src/lib/telemetryHistory.ts";
import type { InstallableUpdateResource } from "../src/lib/updateLifecycle.ts";

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
  assert.deepEqual(scheduler.cleared, [2]);
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
  private readonly closeError: Error | null;
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
  const controller = new StableUpdateController(async () => update);
  const states: string[] = [];

  await controller.check("appimage", (state) => states.push(`${state.status}:${state.message}`));
  await controller.install((state) => states.push(`${state.status}:${state.message}`));

  assert.deepEqual(states, [
    "checking:Checking the stable release channel…",
    "available:Version 2.0.0 is available.",
    "installing:Downloading and verifying the signed update…",
    "installing:Installing the verified update. BatCave will close when installation begins.",
  ]);
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
