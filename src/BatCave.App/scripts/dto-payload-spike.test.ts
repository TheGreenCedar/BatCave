import assert from "node:assert/strict";
import test from "node:test";
import { buildEvidence, buildPayload, strategies } from "./dto-payload-spike.ts";

test("each strategy preserves the requested row count and explicit contract version", () => {
  for (const strategy of strategies) {
    const payload = buildPayload(strategy, 17);
    assert.equal(payload.processes.length, 17);
    assert.equal(payload.contract_version, strategy === "current_v2" ? 2 : 3);
  }
});

test("shared descriptors avoid repeated metadata overhead", () => {
  const evidence = buildEvidence();

  for (const result of evidence.results) {
    const { strategies: measured } = result;
    assert.ok(measured.per_value_metadata_v3.bytes > measured.per_family_metadata_v3.bytes);
    assert.ok(measured.per_family_metadata_v3.bytes > measured.shared_descriptor_catalog_v3.bytes);
    assert.ok(measured.shared_descriptor_catalog_v3.bytes > measured.current_v2.bytes);
  }
});
