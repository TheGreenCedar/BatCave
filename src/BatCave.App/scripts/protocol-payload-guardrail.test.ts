import { RUNTIME_PROTOCOL_VERSION } from "../src/lib/generated/runtime-protocol-v4.ts";
import assert from "node:assert/strict";
import test from "node:test";
import {
  assertGuardrail,
  buildEquivalentV2,
  buildGuardrailEvidence,
  buildProductionV4,
  groupRowCount,
  loadProductionFixture,
  processRowCount,
} from "./protocol-payload-guardrail.ts";

test("the checked production envelope scales to the 5,000-row gate mechanically", () => {
  const { envelope } = loadProductionFixture();
  const scaled = buildProductionV4(envelope);
  const payload = scaled.event.payload;
  const processes = payload.workloads.filter((row) => row.kind === "process");
  const groups = payload.workloads.filter((row) => row.kind === "group");

  assert.equal(scaled.protocol_version, RUNTIME_PROTOCOL_VERSION);
  assert.equal(scaled.event.kind, "runtime_snapshot");
  const overviewGroups = payload.overview_workloads.filter((row) => row.kind === "group");
  const overviewProcesses = payload.overview_workloads.filter((row) => row.kind === "process");
  assert.equal(overviewGroups.length, 20);
  assert.equal(overviewProcesses.length, 200);
  const scaledIds = new Set(payload.workloads.map((row) => row.detail.stable_id));
  assert.ok(payload.overview_workloads.every((row) => scaledIds.has(row.detail.stable_id)));
  assert.deepEqual(
    new Set(overviewGroups.flatMap((row) => row.detail.member_ids)),
    new Set(overviewProcesses.map((row) => row.detail.stable_id)),
  );
  assert.equal(processes.length, processRowCount);
  assert.equal(groups.length, groupRowCount);
  assert.equal(payload.total_process_count, processRowCount);
  assert.equal(payload.visible_process_count, processRowCount);
  assert.equal(new Set(processes.map((row) => row.detail.stable_id)).size, processRowCount);
  assert.equal(groups[0].detail.member_ids.length, 10);
  assert.deepEqual(
    groups[0].detail.member_ids,
    processes.slice(0, 10).map((row) => row.detail.stable_id),
  );
  assert.ok(payload.descriptors.every((descriptor) => !("value" in descriptor)));
  assert.ok(processes.every((row) => row.detail.metrics.every((metric) => metric.length === 5)));
});

test("the equivalent v2 baseline preserves full rows and duplicated view payloads", () => {
  const baseline = buildEquivalentV2();

  assert.equal(baseline.event_kind, "runtime_snapshot");
  assert.equal(baseline.processes.length, processRowCount);
  assert.equal(baseline.process_view_rows.length, 4_100);
  assert.equal(
    baseline.process_view_rows.filter((row) => row.kind === "group").length,
    groupRowCount,
  );
  assert.ok(baseline.processes[0].quality.cpu);
  assert.ok(baseline.process_view_rows.some((row) => row.process?.quality));
  assert.ok(baseline.system.memory_accounting);
});

test("the production payload passes size and same-run JSON timing budgets", () => {
  const evidence = buildGuardrailEvidence(undefined, 21);

  assertGuardrail(evidence);
  assert.ok(evidence.comparison.size_reduction_percent >= 50);
  assert.ok(evidence.comparison.stringify_p95_ratio <= 3);
  assert.ok(evidence.comparison.parse_p95_ratio <= 3);
});
