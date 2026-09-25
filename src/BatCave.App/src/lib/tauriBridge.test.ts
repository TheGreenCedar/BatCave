/// <reference types="node" />

import assert from "node:assert/strict";
import test from "node:test";

import { decodeSystemHistoryPoints, readSystemHistory } from "./tauriBridge.ts";

const system = {
  cpu_percent: 10,
  kernel_cpu_percent: 4,
  logical_cpu_percent: [10, 12],
  memory_used_bytes: 1_000,
  memory_total_bytes: 4_000,
  process_count: 3,
  disk_read_total_bytes: 1,
  disk_write_total_bytes: 2,
  disk_read_bps: 3,
  disk_write_bps: 4,
  network_received_total_bytes: 5,
  network_transmitted_total_bytes: 6,
  network_received_bps: 7,
  network_transmitted_bps: 8,
};

test("decodeSystemHistoryPoints accepts a well-formed wire payload", () => {
  const points = decodeSystemHistoryPoints([
    { sample_seq: 2, sampled_at_ms: 2_000, system },
    {
      sample_seq: 3,
      sampled_at_ms: 3_000,
      system: {
        ...system,
        swap_used_bytes: 10,
        quality: { cpu: { quality: "partial", limitation_code: "held_value" } },
      },
    },
  ]);
  assert.equal(points.length, 2);
  assert.equal(points[0].sample_seq, 2);
  assert.equal(points[1].system.swap_used_bytes, 10);
});

test("decodeSystemHistoryPoints rejects malformed payloads", () => {
  for (const bad of [
    "nope",
    [{ sampled_at_ms: 1, system }],
    [{ sample_seq: 1, sampled_at_ms: 1, system: null }],
    [{ sample_seq: 1, sampled_at_ms: 1, system: { ...system, cpu_percent: "x" } }],
    [{ sample_seq: 1, sampled_at_ms: 1, system: { ...system, logical_cpu_percent: [1, "x"] } }],
    [
      {
        sample_seq: 1,
        sampled_at_ms: 1,
        system: { ...system, quality: { cpu: { quality: "bogus" } } },
      },
    ],
  ]) {
    assert.throws(() => decodeSystemHistoryPoints(bad), /not recognized/);
  }
});

test("readSystemHistory invokes get_system_history with the after cursor", async () => {
  const calls: Array<{ command: string; args: unknown }> = [];
  const invoke = <T>(command: string, args?: Record<string, unknown>): Promise<T> => {
    calls.push({ command, args });
    return Promise.resolve([{ sample_seq: 6, sampled_at_ms: 6_000, system }] as T);
  };
  const points = await readSystemHistory(invoke, 5);
  assert.deepEqual(calls, [{ command: "get_system_history", args: { afterSampleSeq: 5 } }]);
  assert.equal(points[0].sample_seq, 6);
});

test("readSystemHistory propagates malformed native results", async () => {
  const invoke = <T>(): Promise<T> => Promise.resolve([{ sample_seq: "x" }] as T);
  await assert.rejects(() => readSystemHistory(invoke, 0), /not recognized/);
});
