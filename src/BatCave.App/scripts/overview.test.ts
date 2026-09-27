import assert from "node:assert/strict";
import test from "node:test";
import { makeFixtureSnapshot } from "../src/lib/fixtures.ts";
import { buildOverviewContributor } from "../src/lib/cockpit.ts";
import {
  buildOverviewStatus,
  OverviewRanking,
  leadingOverviewRows,
  overviewQualityLabel,
  overviewMetricValue,
} from "../src/lib/overview.ts";
import { makeEmptySnapshot } from "../src/lib/runtimeSnapshot.ts";
import {
  applyCollectionHysteresis,
  buildTelemetryPresentation,
  createCollectionHysteresis,
  metricPresentation,
} from "../src/lib/telemetryPresentation.ts";
import type { ProcessViewRow } from "../src/lib/types.ts";

function withMetric(
  row: ProcessViewRow,
  resource: "cpu" | "memory",
  value: number,
): ProcessViewRow {
  const clone = structuredClone(row);
  if (resource === "cpu") {
    if (clone.kind === "group") clone.detail.cpu_percent = value;
    else clone.detail.process.cpu_percent = value;
  } else {
    if (clone.kind === "group") clone.detail.memory_bytes = value;
    else clone.detail.process.memory_bytes = value;
  }
  return clone;
}

test("overview reports measured utilization without diagnosing machine health", () => {
  const snapshot = makeFixtureSnapshot(1, undefined, "macos");
  const status = buildOverviewStatus(snapshot, "live", 0);
  assert.match(status.headline, /CPU|Memory/);
  assert.doesNotMatch(status.headline + status.summary, /normal|pressure|healthy|unusual/i);
  assert.equal(status.attention.tone, "healthy");
  assert.equal(status.attention.title, "Monitoring");
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

test("a failed first poll reports monitoring unavailable instead of starting", () => {
  const empty = makeEmptySnapshot("protocol_contributor_identity_invalid");
  const failed = buildTelemetryPresentation(empty, "stale");
  assert.equal(failed.state, "stale");
  assert.equal(failed.tone, "danger");
  assert.equal(failed.label, "Monitoring unavailable");

  // A later successful poll recovers to the live presentation.
  const recovered = buildTelemetryPresentation(makeFixtureSnapshot(1, undefined, "macos"), "live");
  assert.equal(recovered.state, "live");
  assert.equal(recovered.tone, "healthy");
});

function limitedPresentation() {
  const snapshot = makeFixtureSnapshot(1, undefined, "macos");
  snapshot.health.reason_codes = ["collector_limited"];
  return buildTelemetryPresentation(snapshot, "live");
}

test("collection-limited hysteresis engages after 3 and clears after 5", () => {
  const hysteresis = createCollectionHysteresis();
  assert.equal(hysteresis(true), false, "first limited sample");
  assert.equal(hysteresis(true), false, "second limited sample");
  assert.equal(hysteresis(true), true, "third consecutive limited sample latches");
  assert.equal(hysteresis(true), true, "stays latched");

  for (let index = 0; index < 4; index += 1) {
    assert.equal(hysteresis(false), true, `clear sample ${index + 1} keeps the latch`);
  }
  assert.equal(hysteresis(false), false, "fifth clear sample releases");

  // A single limited sample after release does not re-engage.
  assert.equal(hysteresis(true), false);
});

test("hysteresis gates only the limited presentation; danger passes through", () => {
  const limited = limitedPresentation();
  assert.equal(limited.label, "Collection limited");

  const unlatched = applyCollectionHysteresis(limited, false);
  assert.equal(unlatched.tone, "healthy");
  assert.equal(unlatched.label, "Monitoring");
  assert.equal(unlatched.state, "live");

  const healthy = buildTelemetryPresentation(makeFixtureSnapshot(1, undefined, "macos"), "live");
  const stillLimited = applyCollectionHysteresis(healthy, true);
  assert.equal(stillLimited.label, "Collection limited");
  assert.equal(stillLimited.tone, "warning");

  const stale = buildTelemetryPresentation(makeEmptySnapshot("failed"), "stale");
  const passthrough = applyCollectionHysteresis(stale, true);
  assert.equal(passthrough.label, "Monitoring unavailable");
  assert.equal(passthrough.tone, "danger");
});

test("collector-limited and limitation counts stay chip-only; danger keeps the banner", () => {
  const limited = makeFixtureSnapshot(1, undefined, "macos");
  limited.health.reason_codes = ["collector_limited"];
  limited.health.collector_state = "limited";
  const limitedStatus = buildOverviewStatus(limited, "live", 4);
  assert.equal(limitedStatus.attention.tone, "healthy");

  const calm = buildOverviewStatus(makeFixtureSnapshot(1, undefined, "macos"), "live", 3);
  assert.equal(calm.attention.tone, "healthy");

  const stale = makeEmptySnapshot("protocol failure");
  stale.health.freshness = "stale";
  const staleStatus = buildOverviewStatus(stale, "stale", 0);
  assert.equal(staleStatus.attention.tone, "danger");
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
  assert.equal(metricPresentation({ quality: "estimated" }, "live", true).label, "Current");
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
  assert.deepEqual(
    leading.map((row) => row.detail.workload_id),
    [
      "process:2180:1699999819000",
      "process:2179:1699999820000",
      "process:2178:1699999821000",
      "process:2177:1699999822000",
      "process:2176:1699999823000",
    ],
  );
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

function groupedContributorFixture() {
  const snapshot = makeFixtureSnapshot(8, undefined, "windows", "compact");
  const group = snapshot.overview_rows.find((row) => row.kind === "group")!;
  const members = snapshot.overview_rows.filter((row) => row.kind === "process" && row.is_grouped);
  const competitor = snapshot.overview_rows.find(
    (row) => row.kind === "process" && !row.is_grouped,
  )!;
  assert.equal(group.kind, "group");
  assert.equal(competitor.kind, "process");
  if (group.kind !== "group" || competitor.kind !== "process")
    throw new Error("Expected fixture rows");
  assert.equal(members.length, 2);
  members.forEach((row, index) => {
    if (row.kind !== "process") throw new Error("Expected process member");
    row.detail.process.cpu_percent = index === 0 ? 45 : 35;
    row.detail.process.memory_bytes = (index === 0 ? 96 : 64) * 1024 ** 2;
    row.detail.process.network_received_bps = (index === 0 ? 14 : 10) * 1024;
    row.detail.process.network_transmitted_bps = 0;
  });
  group.detail.label = "BatCave";
  group.detail.cpu_percent = 80;
  group.detail.memory_bytes = 160 * 1024 ** 2;
  group.detail.network_bps = 24 * 1024;
  group.detail.quality.cpu =
    group.detail.quality.memory =
    group.detail.quality.network =
      { quality: "native" };
  group.detail.coverage.cpu =
    group.detail.coverage.memory =
    group.detail.coverage.network =
      { available: 2, total: 2 };
  competitor.detail.process.cpu_percent = 72;
  competitor.detail.process.memory_bytes = 128 * 1024 ** 2;
  competitor.detail.process.network_received_bps = 20 * 1024;
  competitor.detail.process.network_transmitted_bps = 0;
  competitor.detail.process.quality = {
    cpu: { quality: "native" },
    memory: { quality: "native" },
    network: { quality: "native" },
  };
  snapshot.overview_rows = [competitor, ...members, group, group];
  return { snapshot, group, members, competitor };
}

test("Overview hero selects the distinct-member aggregate and preserves process contributor facts", () => {
  const { snapshot, group } = groupedContributorFixture();
  const contributors = structuredClone(snapshot.process_contributors);
  assert.equal(buildOverviewContributor(snapshot, "cpu", "live").row, group);
  assert.equal(
    buildOverviewContributor(snapshot, "cpu", "live").statusLabel,
    "2 processes · 80% of one core",
  );
  assert.equal(
    buildOverviewContributor(snapshot, "memory", "live").statusLabel,
    "2 processes · 160 MB resident memory",
  );
  assert.equal(
    buildOverviewContributor(snapshot, "network", "live").statusLabel,
    "2 processes · 24 KB/s process traffic",
  );
  for (const resource of ["cpu", "memory", "network"] as const) {
    assert.equal(
      buildOverviewContributor(snapshot, resource, "live").row?.detail.workload_id,
      leadingOverviewRows(snapshot.overview_rows, resource, 1)[0].detail.workload_id,
    );
  }
  // Explore's filtered rows and raw process-scoped contributor identity cannot change the hero scope.
  snapshot.process_view_rows = [];
  snapshot.processes = [];
  assert.equal(buildOverviewContributor(snapshot, "cpu", "live").row, group);
  assert.deepEqual(snapshot.process_contributors, contributors);
});

test("Overview hero retains aggregate partial coverage and excludes held, unavailable, and zero coverage", () => {
  const { snapshot, group, competitor } = groupedContributorFixture();
  group.detail.cpu_percent = 45;
  group.detail.quality.cpu = { quality: "partial" };
  group.detail.coverage.cpu = { available: 1, total: 2 };
  competitor.detail.process.cpu_percent = 40;
  assert.equal(buildOverviewContributor(snapshot, "cpu", "live").row, group);
  assert.equal(
    buildOverviewContributor(snapshot, "cpu", "live").statusLabel,
    "2 processes · 45% of one core · 1/2 · limited",
  );
  for (const quality of ["held", "unavailable"] as const) {
    group.detail.quality.cpu = { quality };
    assert.equal(buildOverviewContributor(snapshot, "cpu", "live").row, competitor);
  }
  group.detail.quality.cpu = { quality: "native" };
  group.detail.coverage.cpu = { available: 0, total: 2 };
  assert.equal(buildOverviewContributor(snapshot, "cpu", "live").row, competitor);
  competitor.detail.process.quality!.cpu = { quality: "unavailable" };
  assert.deepEqual(buildOverviewContributor(snapshot, "cpu", "live"), {
    row: null,
    statusLabel: "No available workload attribution for this resource",
  });
});

test("Overview hero admits only the current opaque group scope after removal and restart", () => {
  const { snapshot, group, members, competitor } = groupedContributorFixture();
  group.detail.workload_id = `group:scope:${"a".repeat(64)}`;
  const previousId = group.detail.workload_id;
  assert.equal(
    buildOverviewContributor(snapshot, "cpu", "live").row?.detail.workload_id,
    previousId,
  );
  snapshot.overview_rows = [competitor, ...members];
  assert.equal(buildOverviewContributor(snapshot, "cpu", "live").row, competitor);
  const restarted = structuredClone(group);
  restarted.detail.workload_id = `group:scope:${"b".repeat(64)}`;
  restarted.detail.cpu_percent = 35;
  restarted.detail.process_count = 1;
  restarted.detail.coverage.cpu = { available: 1, total: 1 };
  snapshot.overview_rows = [restarted];
  const current = buildOverviewContributor(snapshot, "cpu", "live");
  assert.equal(current.row, restarted);
  assert.notEqual(current.row?.detail.workload_id, previousId);
  assert.equal(current.statusLabel, "1 process · 35% of one core");
  // Frontend selection uses the backend scope as published; it does not manufacture or retain an old scope.
});

test("Overview hero never treats aggregate process I/O as physical disk attribution", () => {
  const { snapshot, group } = groupedContributorFixture();
  group.detail.io_bps = 1024 ** 3;
  snapshot.system.disk_read_bps = 1024;
  snapshot.system.disk_write_bps = 0;
  assert.deepEqual(buildOverviewContributor(snapshot, "disk", "live"), {
    row: null,
    statusLabel: "No compatible process attribution",
  });
  assert.equal(leadingOverviewRows(snapshot.overview_rows, "disk", 1)[0], group);
  assert.equal(
    buildOverviewStatus(snapshot, "live", 0, "disk").headline,
    "Physical disk throughput is 1.0 KB/s.",
  );
});

test("Overview hero waits for a system sample and uses the published rows while paused", () => {
  const { snapshot, group } = groupedContributorFixture();
  assert.equal(buildOverviewContributor(snapshot, "cpu", "paused").row, group);
  assert.equal(buildOverviewContributor(snapshot, "cpu", "stale").row, group);
  snapshot.sampled_at_ms = null;
  assert.equal(buildOverviewContributor(snapshot, "cpu", "live").row, null);
  assert.equal(buildOverviewContributor(makeEmptySnapshot(), "cpu", "starting").row, null);
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
  // While interacting, an emptied incoming list keeps the last rows as ghosts.
  assert.deepEqual(
    ranking.update("memory", []).map((row) => row.detail.workload_id),
    incoming.map((row) => row.detail.workload_id),
  );
  // Releasing the interaction drops the ghosts.
  assert.deepEqual(ranking.setInteraction("pointer", false), []);
});

test("Overview ranking keeps a near-tie swap stable inside the settle interval", () => {
  const initial = makeFixtureSnapshot(1, undefined, "macos")
    .overview_rows.slice(0, 3)
    .map((row, index) => withMetric(row, "cpu", 20 + index * 0.5));
  const swapped = [initial[1], initial[0], initial[2]];
  const ranking = new OverviewRanking();
  assert.deepEqual(ranking.update("cpu", initial), initial);
  assert.deepEqual(
    ranking.update("cpu", swapped).map((row) => row.detail.workload_id),
    initial.map((row) => row.detail.workload_id),
  );
});

test("Overview ranking adopts a large inversion immediately", () => {
  const [first, second] = makeFixtureSnapshot(1, undefined, "macos").overview_rows.slice(0, 2);
  const quiet = withMetric(first, "cpu", 23);
  const busy = withMetric(second, "cpu", 182);
  const ranking = new OverviewRanking();
  assert.deepEqual(ranking.update("cpu", [quiet, busy]), [quiet, busy]);
  // A quiet row outranking a much busier one is not a tie; re-sort now.
  assert.deepEqual(ranking.update("cpu", [busy, quiet]), [busy, quiet]);
});

test("Overview ranking holds memory inversions only within the near-tie floor", () => {
  const rows = makeFixtureSnapshot(1, undefined, "macos")
    .overview_rows.slice(0, 2)
    .map((row) => structuredClone(row));
  const gib = 1024 * 1024 * 1024;
  const mib = 1024 * 1024;
  const base = rows.map((row, index) => withMetric(row, "memory", gib + index * 20 * mib));
  const ranking = new OverviewRanking();
  assert.deepEqual(ranking.update("memory", base), base);
  // 20 MiB is inside the 32 MiB floor for a ~1 GiB row: held.
  assert.deepEqual(
    ranking.update("memory", [base[1], base[0]]).map((row) => row.detail.workload_id),
    base.map((row) => row.detail.workload_id),
  );

  const far = rows.map((row, index) => withMetric(row, "memory", gib + index * 400 * mib));
  const distant = new OverviewRanking();
  distant.update("memory", far);
  assert.deepEqual(distant.update("memory", [far[1], far[0]]), [far[1], far[0]]);
});

test("OverviewRanking reports an available update while held and applies it on request", () => {
  const rows = makeFixtureSnapshot(1, undefined, "macos")
    .overview_rows.slice(0, 3)
    .map((row, index) => withMetric(row, "cpu", 40 - index));
  const reordered = [rows[1], rows[2], rows[0]];
  const ranking = new OverviewRanking();
  ranking.update("cpu", rows);
  assert.equal(ranking.updateAvailable, false);

  // A settle-hold without interaction stays silent.
  ranking.update("cpu", reordered);
  assert.equal(ranking.updateAvailable, false);

  ranking.setInteraction("pointer", true);
  const held = ranking.update("cpu", reordered);
  assert.equal(ranking.updateAvailable, true);
  assert.deepEqual(
    held.map((row) => row.detail.workload_id),
    rows.map((row) => row.detail.workload_id),
  );

  assert.deepEqual(ranking.applyUpdate(), reordered);
  assert.equal(ranking.updateAvailable, false);
  assert.deepEqual(
    ranking.update("cpu", reordered).map((row) => row.detail.workload_id),
    reordered.map((row) => row.detail.workload_id),
  );
});
