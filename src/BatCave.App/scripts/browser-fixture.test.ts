import assert from "node:assert/strict";
import { existsSync, readFileSync } from "node:fs";
import test from "node:test";
import type { ProtocolEnvelope } from "../src/lib/generated/runtime-protocol-v3.ts";
import { makeFixtureSnapshot } from "../src/lib/fixtures.ts";
import { processViewRowKey, processViewRowMetrics } from "../src/lib/process.ts";
import { makeDefaultRuntimeQuery } from "../src/lib/runtimeSnapshot.ts";
import type { RuntimePlatform, RuntimeQuery } from "../src/lib/types.ts";

const canonicalPrefix = [
  "group:batcave.app.exe",
  "process:1234:1699999999000",
  "process:1235:1699999999001",
];
const browserFixturePaths = {
  windows: "../src-tauri/src/fixtures/runtime-protocol-v3/browser-windows.json",
  linux: "../src-tauri/src/fixtures/runtime-protocol-v3/browser-linux.json",
  macos: "../src-tauri/src/fixtures/runtime-protocol-v3/browser-macos.json",
} as const;

for (const platform of Object.keys(browserFixturePaths) as Array<
  Exclude<RuntimePlatform, "fixture">
>) {
  test(`${platform} browser fixture preserves checked workload identity and density`, () => {
    const envelope = fixture(browserFixturePaths[platform]);
    if (envelope.event.kind !== "runtime_snapshot") throw new Error("expected snapshot fixture");
    const payload = envelope.event.payload;
    const workloadIds = payload.workloads.map((workload) => workload.detail.stable_id);
    const snapshot = makeFixtureSnapshot(0, makeDefaultRuntimeQuery(), platform, "compact");
    const adaptedIds = snapshot.process_view_rows.map(processViewRowKey);

    assert.deepEqual(workloadIds.slice(0, canonicalPrefix.length), canonicalPrefix);
    const canonicalGroup = payload.workloads[0];
    assert.equal(canonicalGroup.kind, "group");
    if (canonicalGroup.kind !== "group") throw new Error("expected canonical group");
    assert.deepEqual(canonicalGroup.detail.member_ids, canonicalPrefix.slice(1));
    assert.deepEqual(adaptedIds, workloadIds);
    assert.equal(new Set(workloadIds).size, workloadIds.length);
    assert.equal(payload.visible_process_count, 48);
    assert.equal(snapshot.total_process_count, 48);
    assert.equal(snapshot.process_view_rows.length, 49);
    assert.ok(snapshot.process_view_rows.some((row) => row.kind === "group"));
    assert.equal(
      snapshot.process_view_rows.filter((row) => row.kind === "process" && !row.is_grouped).length,
      46,
    );
    assert.deepEqual(snapshot.warnings, []);

    const processIds = new Set(
      payload.workloads
        .filter((workload) => workload.kind === "process")
        .map((workload) => workload.detail.stable_id),
    );
    for (const workload of payload.workloads) {
      if (workload.kind === "group") {
        assert.ok(workload.detail.member_ids.every((memberId) => processIds.has(memberId)));
      }
    }

    if (platform === "macos") {
      assert.ok(
        snapshot.processes.every((process) => process.quality?.network?.quality === "unavailable"),
      );
    } else if (platform === "linux") {
      assert.ok(
        snapshot.processes.every((process) => process.quality?.memory?.quality === "partial"),
      );
    } else {
      assert.ok(
        snapshot.processes.every((process) => process.quality?.cpu?.quality === "estimated"),
      );
    }
  });
}

test("browser fixture ticks animate values and simulate publication order without changing identity", () => {
  const query = makeDefaultRuntimeQuery();
  const first = makeFixtureSnapshot(8, query, "macos", "compact");
  const repeated = makeFixtureSnapshot(8, query, "macos", "compact");
  const next = makeFixtureSnapshot(9, query, "macos", "compact");
  const firstIds = first.process_view_rows.map(processViewRowKey);
  const nextIds = next.process_view_rows.map(processViewRowKey);

  assert.deepEqual(repeated, first);
  assert.deepEqual(firstIds.slice(0, canonicalPrefix.length), canonicalPrefix);
  assert.deepEqual(nextIds.slice(0, canonicalPrefix.length), canonicalPrefix);
  assert.deepEqual(new Set(nextIds), new Set(firstIds));
  assert.notDeepEqual(nextIds, firstIds);
  assert.notEqual(
    processViewRowMetrics(next.process_view_rows[0]).cpuPercent,
    processViewRowMetrics(first.process_view_rows[0]).cpuPercent,
  );
  assert.equal(next.publication_seq, 9);
  assert.equal(next.sample_seq, 9);
  assert.equal(next.published_at_ms - first.published_at_ms, 1_000);
});

test("browser fixture queries are echoed without reshaping checked workloads", () => {
  const baselineQuery = makeDefaultRuntimeQuery();
  const hostileQuery: RuntimeQuery = {
    filter_text: "does-not-exist",
    focus_mode: "io",
    sort_column: "name",
    sort_direction: "asc",
    limit: 1,
  };
  const baseline = makeFixtureSnapshot(4, baselineQuery, "windows", "compact");
  const mutated = makeFixtureSnapshot(4, hostileQuery, "windows", "compact");

  assert.deepEqual(
    mutated.process_view_rows.map(processViewRowKey),
    baseline.process_view_rows.map(processViewRowKey),
  );
  assert.deepEqual(mutated.settings.query, hostileQuery);
});

test("normal macOS fixture keeps the full result-window density independent of query limits", () => {
  const dense = makeFixtureSnapshot(0, makeDefaultRuntimeQuery(), "macos");
  const compact = makeFixtureSnapshot(0, makeDefaultRuntimeQuery(), "macos", "compact");
  const rankedRows = dense.process_view_rows.filter(
    (row) => row.kind === "group" || !row.is_grouped,
  );

  assert.equal(dense.total_process_count, 182);
  assert.equal(dense.process_view_rows.length, 183);
  assert.equal(rankedRows.length, 181);
  assert.equal(compact.total_process_count, 48);
  assert.equal(compact.process_view_rows.length, 49);
});

test("browser fixture source contains no duplicate workload shaper", () => {
  const fixturesSource = source("../src/lib/fixtures.ts");
  const processSource = source("../src/lib/process.ts");
  const fixtureProtocolSource = source("../src/lib/protocol/fixtureProtocol.ts");

  for (const obsolete of [
    "shapeProcessView",
    "compareProcessSamples",
    "compareProcessGroups",
    "processGroupKey",
    "normalizedProcessName",
    "summarizeProcessContributors",
  ]) {
    assert.doesNotMatch(fixturesSource, new RegExp(obsolete, "u"));
    assert.doesNotMatch(processSource, new RegExp(obsolete, "u"));
  }
  assert.doesNotMatch(fixtureProtocolSource, /roundTripFixtureSnapshot/u);
  assert.doesNotMatch(fixturesSource, /\.sort\(|\.filter\(|\.flatMap\(/u);
  assert.equal(existsSync(new URL("../src/lib/systemPressure.ts", import.meta.url)), false);
});

function fixture(relativePath: string): ProtocolEnvelope {
  return JSON.parse(readFileSync(new URL(relativePath, import.meta.url), "utf8"));
}

function source(relativePath: string): string {
  return readFileSync(new URL(relativePath, import.meta.url), "utf8");
}
