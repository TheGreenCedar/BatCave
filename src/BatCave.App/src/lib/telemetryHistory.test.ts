/// <reference types="node" />

import assert from "node:assert/strict";
import test from "node:test";

import {
  emptyTrendState,
  historyGapPoints,
  nextSystemHistory,
  replaySystemHistory,
} from "./telemetryHistory.ts";
import type { SystemHistoryPoint, SystemMetricsSnapshot } from "./types.ts";

function system(
  cpu: number,
  overrides: Partial<SystemMetricsSnapshot> = {},
): SystemMetricsSnapshot {
  return {
    cpu_percent: cpu,
    kernel_cpu_percent: cpu / 2,
    logical_cpu_percent: [cpu, cpu + 1],
    memory_used_bytes: 1_000 + cpu,
    memory_total_bytes: 4_000,
    swap_used_bytes: 100,
    swap_total_bytes: 200,
    process_count: 10,
    disk_read_total_bytes: 0,
    disk_write_total_bytes: 0,
    disk_read_bps: cpu * 10,
    disk_write_bps: cpu * 20,
    network_received_total_bytes: 0,
    network_transmitted_total_bytes: 0,
    network_received_bps: cpu * 30,
    network_transmitted_bps: cpu * 40,
    quality: {
      cpu: { quality: "native" },
      kernel_cpu: { quality: "native" },
      logical_cpu: { quality: "native" },
      memory: { quality: "native" },
      swap: { quality: "native" },
      disk: { quality: "native" },
      network: { quality: "native" },
    },
    ...overrides,
  };
}

function point(seq: number, cpu: number): SystemHistoryPoint {
  return { sample_seq: seq, sampled_at_ms: seq * 1_000, system: system(cpu) };
}

test("replaySystemHistory equals folding snapshots one by one", () => {
  const points = [point(1, 10), point(2, 20), point(3, 30)];
  const limit = 72;

  const replayed = replaySystemHistory(emptyTrendState(), points, limit);
  const folded = points.reduce(
    (history, p) => nextSystemHistory(history, { system: p.system }, limit),
    emptyTrendState(),
  );

  assert.deepEqual(replayed, folded);
  assert.deepEqual(replayed.cpu, [10, 20, 30]);
  assert.deepEqual(replayed.cores[0], [10, 20, 30]);
  assert.deepEqual(replayed.cores[1], [11, 21, 31]);
});

test("replaySystemHistory trims to the point limit like live ingestion", () => {
  const points = [point(1, 1), point(2, 2), point(3, 3), point(4, 4)];
  const replayed = replaySystemHistory(emptyTrendState(), points, 3);
  assert.deepEqual(replayed.cpu, [2, 3, 4]);
});

test("replaySystemHistory preserves quality-gated metrics", () => {
  const unavailable = point(1, 10);
  unavailable.system.quality = {
    cpu: { quality: "unavailable" },
  };
  const replayed = replaySystemHistory(emptyTrendState(), [unavailable], 72);
  assert.deepEqual(replayed.cpu, []);
});

test("historyGapPoints keeps only the missing seqs before the live snapshot", () => {
  const points = [point(4, 1), point(5, 2), point(6, 3), point(7, 4), point(8, 5)];
  assert.deepEqual(
    historyGapPoints(points, 4, 8).map((p) => p.sample_seq),
    [5, 6, 7],
  );
  // A truncated ring (points 4 and 5 lost) still returns what it retained.
  assert.deepEqual(
    historyGapPoints(points.slice(2), 4, 8).map((p) => p.sample_seq),
    [6, 7],
  );
  assert.deepEqual(historyGapPoints(points, 4, 5), []);
});
