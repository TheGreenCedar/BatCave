import assert from "node:assert/strict";
import test from "node:test";
import { makeFixtureSnapshot } from "../src/lib/fixtures.ts";
import {
  buildOverviewStatus,
  OverviewRanking,
  leadingOverviewRows,
  overviewQualityLabel,
  overviewMetricValue,
} from "../src/lib/overview.ts";
import { makeEmptySnapshot } from "../src/lib/runtimeSnapshot.ts";
import {
  buildTelemetryPresentation,
  metricPresentation,
} from "../src/lib/telemetryPresentation.ts";

test("overview reports measured utilization without diagnosing machine health", () => {
  const snapshot = makeFixtureSnapshot(1, undefined, "macos");
  const status = buildOverviewStatus(snapshot, "live", 0);
  assert.match(status.headline, /CPU|Memory/);
  assert.doesNotMatch(status.headline + status.summary, /normal|pressure|healthy|unusual/i);
  assert.equal(status.attention, null);
});

test("the primary resource follows explicit selection, independently of utilization and quality", () => {
  const snapshot = makeFixtureSnapshot(1, undefined, "windows");
  snapshot.system.cpu_percent = 20;
  snapshot.system.memory_used_bytes = snapshot.system.memory_total_bytes * 0.91;
  const memory = buildOverviewStatus(snapshot, "live", 0, "memory");
  assert.equal(memory.primaryResource, "memory");
  assert.match(memory.headline, /91%/);
  assert.equal(buildOverviewStatus(snapshot, "live", 0).primaryResource, "cpu");
  snapshot.system.quality!.memory = { quality: "held" };
  const held = buildOverviewStatus(snapshot, "live", 0, "memory");
  assert.equal(held.primaryResource, "memory");
  assert.match(held.headline, /pending/);
});

test("degraded collection and persistence never imply monitor overhead", () => {
  const snapshot = makeFixtureSnapshot(1, undefined, "macos");
  snapshot.health.degraded = true;
  snapshot.health.collector_state = "limited";
  snapshot.health.status_summary = "One collector failed";
  const telemetry = buildTelemetryPresentation(snapshot, "live");
  assert.equal(telemetry.label, "Collection limited");
  const status = buildOverviewStatus(snapshot, "live", 0);
  assert.match(status.attention!.title, /limited/i);
  assert.doesNotMatch(
    status.headline + status.summary + status.attention!.detail,
    /overhead|budget|more resources/,
  );
  snapshot.health.collector_state = "healthy";
  snapshot.persistence = {
    state: "degraded",
    roots: [],
    components: [],
    suppressed_diagnostic_events: 0,
  };
  assert.equal(buildTelemetryPresentation(snapshot, "live").label, "Local storage limited");
});

test("startup, stale samples, pause, and collector failure share one freshness state", () => {
  const snapshot = makeFixtureSnapshot(1, undefined, "macos");
  assert.equal(buildTelemetryPresentation(makeEmptySnapshot(), "starting").state, "starting");
  assert.equal(buildTelemetryPresentation(snapshot, "paused").state, "paused");
  snapshot.health.freshness = "stale";
  assert.equal(buildTelemetryPresentation(snapshot, "live").state, "stale");
  snapshot.health.freshness = "paused";
  assert.equal(buildTelemetryPresentation(snapshot, "paused").state, "paused");
  snapshot.health.collector_state = "unavailable";
  snapshot.health.freshness = "stale";
  assert.equal(buildTelemetryPresentation(snapshot, "live").state, "stale");
  snapshot.health.engine_state = "fatal";
  assert.equal(buildTelemetryPresentation(snapshot, "paused").tone, "danger");
});

test("metric presentation fails closed and preserves real zero plus freshness", () => {
  for (const quality of [
    undefined,
    { quality: "held" as const },
    { quality: "unavailable" as const },
  ]) {
    assert.equal(metricPresentation(quality, "live", true).canDisplay, false);
  }
  assert.equal(metricPresentation({ quality: "native" }, "starting", false).label, "No sample");
  assert.equal(metricPresentation({ quality: "native" }, "stale", true).label, "Stale");
  assert.equal(metricPresentation({ quality: "estimated" }, "live", true).label, "Estimated");
  assert.equal(overviewQualityLabel({ quality: "native" }, "live", true), null);
  assert.equal(overviewQualityLabel({ quality: "partial" }, "live", true), "Limited");
  const row = makeFixtureSnapshot(1, undefined, "macos").process_view_rows.find(
    (row) => row.kind === "process",
  )!;
  if (row.kind !== "process") throw new Error("expected process");
  row.detail.process.cpu_percent = 0;
  row.detail.process.quality = { cpu: { quality: "native" } };
  assert.equal(overviewMetricValue(row, "cpu"), "0%");
  assert.equal(overviewMetricValue(row, "network"), "Quality not reported");
  row.detail.process.quality.cpu = { quality: "held" };
  assert.equal(overviewMetricValue(row, "cpu"), "Pending");
});

test("Overview ranks its own complete rows by the selected resource without grouped children", () => {
  const rows = makeFixtureSnapshot(1, undefined, "macos").process_view_rows;
  const leading = leadingOverviewRows(rows, "memory", 5);
  assert.ok(leading.length <= 5);
  assert.equal(
    leading.some((row) => row.kind === "process" && row.is_grouped),
    false,
  );
  assert.equal(new Set(leading.map((row) => row.detail.workload_id)).size, leading.length);
  const values = leading.map((row) =>
    row.kind === "group" ? row.detail.memory_bytes : row.detail.process.memory_bytes,
  );
  assert.deepEqual(
    values,
    [...values].sort((left, right) => right - left),
  );
  assert.deepEqual(leadingOverviewRows(rows, "memory", 0), []);
});

test("unavailable or held resource measurements cannot enter the Overview ranking", () => {
  const rows = makeFixtureSnapshot(1, undefined, "macos").process_view_rows;
  for (const row of rows) {
    if (row.kind === "group") row.detail.quality.network = { quality: "unavailable" };
    else
      row.detail.process.quality = { ...row.detail.process.quality, network: { quality: "held" } };
  }
  assert.deepEqual(leadingOverviewRows(rows, "network", 5), []);
});

test("monitor overhead is claimed only for an explicit runtime budget reason", () => {
  const snapshot = makeFixtureSnapshot(1, undefined, "macos");
  snapshot.health.reason_codes = ["runtime_cpu_budget"];
  snapshot.health.degraded = true;
  assert.equal(buildTelemetryPresentation(snapshot, "live").label, "Monitor resource warning");
  assert.match(buildTelemetryPresentation(snapshot, "live").detail, /own CPU/);
  snapshot.health.reason_codes = ["runtime_memory_budget"];
  assert.match(buildTelemetryPresentation(snapshot, "live").detail, /own memory/);
  snapshot.health.reason_codes = ["cadence_missed"];
  assert.equal(buildTelemetryPresentation(snapshot, "live").label, "Sampling delayed");
  assert.doesNotMatch(buildTelemetryPresentation(snapshot, "live").detail, /CPU|memory/);
});

test("paused before the first sample stays paused without inventing a measurement", () => {
  const snapshot = makeEmptySnapshot();
  snapshot.settings.paused = true;
  snapshot.health.freshness = "paused";
  snapshot.health.engine_state = "paused";
  const presentation = buildTelemetryPresentation(snapshot, "starting");
  assert.equal(presentation.state, "paused");
  assert.match(presentation.detail, /before the first sample/);
  assert.equal(
    metricPresentation({ quality: "native" }, presentation.state, false).canDisplay,
    false,
  );
});

test("Overview order holds fresh identities through pointer and keyboard interaction, then releases", () => {
  const initial = makeFixtureSnapshot(1, undefined, "macos").overview_rows.slice(0, 2);
  assert.equal(initial.length, 2);
  const incoming = structuredClone([...initial].reverse());
  const ranking = new OverviewRanking();
  assert.deepEqual(ranking.update("cpu", initial), initial);
  ranking.setInteraction("pointer", true);
  ranking.setInteraction("focus", true);
  const held = ranking.update("cpu", incoming);
  assert.deepEqual(
    held.map((row) => row.detail.workload_id),
    initial.map((row) => row.detail.workload_id),
  );
  assert.equal(held[0], incoming[1]);
  assert.deepEqual(ranking.setInteraction("pointer", false), held);
  assert.deepEqual(ranking.setInteraction("focus", false), incoming);
  ranking.setInteraction("pointer", true);
  ranking.update("cpu", initial);
  assert.deepEqual(ranking.update("memory", incoming), incoming);
  assert.deepEqual(ranking.update("memory", []), []);
  assert.deepEqual(ranking.setInteraction("pointer", false), []);
});
