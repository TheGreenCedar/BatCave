import assert from "node:assert/strict";
import test from "node:test";

import { makeFixtureSnapshot } from "../src/lib/fixtures.ts";
import {
  buildOverviewStatus,
  leadingOverviewRows,
  overviewQualityLabel,
} from "../src/lib/overview.ts";
import { makeEmptySnapshot } from "../src/lib/runtimeSnapshot.ts";

test("healthy overview stays quiet and uses platform language", () => {
  const snapshot = makeFixtureSnapshot(1, undefined, "macos");
  const status = buildOverviewStatus(snapshot, "live", 0);

  assert.equal(status.headline, "Your Mac is running normally.");
  assert.equal(status.tone, "healthy");
  assert.equal(status.attention, null);
});

test("CPU and memory pressure create a single useful attention state", () => {
  const cpuSnapshot = makeFixtureSnapshot(1, undefined, "windows");
  cpuSnapshot.system.cpu_percent = 91;
  const cpuStatus = buildOverviewStatus(cpuSnapshot, "live", 0);
  assert.equal(cpuStatus.headline, "Your PC is under pressure.");
  assert.equal(cpuStatus.pressuredResource, "cpu");
  assert.equal(cpuStatus.attention?.title, "CPU needs attention");

  const memorySnapshot = makeFixtureSnapshot(1, undefined, "linux");
  memorySnapshot.system.memory_used_bytes = memorySnapshot.system.memory_total_bytes * 0.9;
  const memoryStatus = buildOverviewStatus(memorySnapshot, "live", 0);
  assert.equal(memoryStatus.pressuredResource, "memory");
  assert.equal(memoryStatus.attention?.title, "Memory needs attention");
});

test("stale and paused collection explain why values are not live", () => {
  const snapshot = makeFixtureSnapshot(1, undefined, "macos");

  const stale = buildOverviewStatus(snapshot, "stale", 0);
  assert.equal(stale.tone, "danger");
  assert.equal(stale.attention?.title, "Data is stale");

  const paused = buildOverviewStatus(snapshot, "paused", 0);
  assert.equal(paused.tone, "warning");
  assert.equal(paused.attention?.title, "Live updates are paused");
});

test("limited and degraded states remain visible without collector jargon", () => {
  const limitedSnapshot = makeFixtureSnapshot(1, undefined, "macos");
  const limited = buildOverviewStatus(limitedSnapshot, "live", 2);
  assert.equal(limited.attention?.title, "2 data limitations");

  const degradedSnapshot = makeFixtureSnapshot(1, undefined, "macos");
  degradedSnapshot.health.degraded = true;
  const degraded = buildOverviewStatus(degradedSnapshot, "live", 0);
  assert.equal(degraded.attention?.title, "Monitor overhead is elevated");
  assert.doesNotMatch(degraded.summary, /collector|provenance|lifecycle/i);
});

test("empty state waits for the first sample", () => {
  const status = buildOverviewStatus(makeEmptySnapshot(), "starting", 0);
  assert.equal(status.headline, "BatCave is getting ready.");
  assert.equal(status.attention, null);
});

test("leading workloads are capped and do not duplicate grouped children", () => {
  const rows = makeFixtureSnapshot(1, undefined, "macos").process_view_rows;
  const leading = leadingOverviewRows(rows, 5);

  assert.ok(leading.length <= 5);
  assert.equal(
    leading.some((row) => row.kind === "process" && row.is_grouped),
    false,
  );
  assert.equal(new Set(leading.map((row) => row.detail.workload_id)).size, leading.length);
});

test("quality labels stay silent for normal native values", () => {
  assert.equal(overviewQualityLabel({ quality: "native" }, "live", true), null);
  assert.equal(overviewQualityLabel({ quality: "estimated" }, "live", true), "Estimated");
  assert.equal(overviewQualityLabel({ quality: "partial" }, "live", true), "Limited");
  assert.equal(overviewQualityLabel({ quality: "native" }, "stale", true), "Stale");
  assert.equal(overviewQualityLabel(undefined, "live", false), "No sample");
});
