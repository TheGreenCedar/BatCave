import assert from "node:assert/strict";
import test from "node:test";
import { buildEvidence, buildPayload, strategies } from "./dto-payload-spike.ts";

test("the baseline preserves the real v2 envelope and duplicated view rows", () => {
  const payload = buildPayload("current_v2", 100);

  assert.equal(payload.processes.length, 100);
  assert.equal(payload.process_view_rows.length, 82);
  assert.equal(payload.total_process_count, 100);
  assert.equal(payload.event_kind, "runtime_snapshot");
  assert.ok(payload.processes[0].quality.cpu);
  assert.equal(typeof payload.processes[0].disk_read_total_bytes, "number");
  assert.ok(payload.process_view_rows.some((row) => row.kind === "group"));
  assert.ok(payload.process_view_rows.some((row) => row.process?.quality));
  assert.ok(payload.system.memory_accounting);
  assert.ok(payload.system.quality.disk);
});

test("candidate strategies preserve process, group, system, and sparse limitation data", () => {
  for (const strategy of strategies.filter((candidate) => candidate !== "current_v2")) {
    const payload = buildPayload(strategy, 100);
    assert.equal(payload.protocol_version, 3);
    assert.equal(payload.process_row_count, 100);
    assert.equal(payload.group_row_count, 2);
    assert.equal(payload.workloads.filter((row) => row.kind === "process").length, 100);
    assert.equal(payload.workloads.filter((row) => row.kind === "group").length, 2);
    assert.ok(payload.system);
  }

  const shared = buildPayload("shared_descriptor_catalog_v3", 100);
  assert.ok(shared.descriptors.some((descriptor) => descriptor.scope === "system"));
  assert.ok(shared.descriptors.some((descriptor) => descriptor.scope === "process"));
  assert.ok(shared.descriptors.some((descriptor) => descriptor.scope === "group"));
  assert.ok(shared.descriptors.every((descriptor) => !("family" in descriptor)));
  assert.ok(
    shared.descriptors.some(
      (descriptor) =>
        descriptor.scope === "system" && descriptor.semantic === "process_working_set_memory",
    ),
  );
  assert.equal(shared.limitations.length, 2);
  assert.deepEqual(shared.system.limitation_indexes, [1]);
  assert.ok(!("memory_accounting" in shared.system));
  assert.ok(
    shared.workloads.some((row) => row.detail.metrics?.some((observation) => observation[4] === 0)),
  );
  assert.ok(
    shared.workloads.some(
      (row) => row.kind === "group" && row.detail.coverage.limitation_indexes?.[0] === 0,
    ),
  );
});

test("shared descriptors avoid repeated metadata overhead", () => {
  const evidence = buildEvidence();

  for (const result of evidence.results) {
    const { strategies: measured } = result;
    assert.ok(measured.per_value_metadata_v3.bytes > measured.per_family_metadata_v3.bytes);
    assert.ok(measured.per_family_metadata_v3.bytes > measured.shared_descriptor_catalog_v3.bytes);
    assert.ok(
      measured.shared_descriptor_catalog_v3.relative_to_current_percent <= -50,
      `shared descriptor payload missed the 50% reduction budget at ${result.process_row_count} rows`,
    );
  }
});
