import assert from "node:assert/strict";
import test from "node:test";
import {
  hasSameProcessOrder,
  processViewRowKey,
  shouldStabilizeProcessOrder,
  stabilizeProcessRows,
} from "../src/lib/process.ts";
import type { ProcessViewRow } from "../src/lib/types.ts";

function row(pid: string, cpuPercent: number): ProcessViewRow {
  return {
    kind: "process",
    process: {
      pid,
      parent_pid: null,
      start_time_ms: 0,
      name: `${pid}.exe`,
      exe: `C:\\${pid}.exe`,
      status: "running",
      cpu_percent: cpuPercent,
      memory_bytes: 0,
      private_bytes: 0,
      virtual_memory_bytes: 0,
      disk_read_total_bytes: 0,
      disk_write_total_bytes: 0,
      disk_read_bps: 0,
      disk_write_bps: 0,
      threads: 1,
      handles: 1,
      access_state: "full",
    },
    group_count: 1,
    icon_kind: "process",
    is_child: false,
    is_grouped: false,
    attention_label: "steady",
    cpu_percent: cpuPercent,
    memory_bytes: 0,
    io_bps: 0,
    network_bps: 0,
    threads: 1,
  };
}

test("processViewRowKey keeps process identity independent of live values", () => {
  assert.equal(processViewRowKey(row("42", 1)), processViewRowKey(row("42", 99)));
});

test("hasSameProcessOrder detects a live reorder", () => {
  assert.equal(
    hasSameProcessOrder([row("1", 10), row("2", 20)], [row("2", 30), row("1", 40)]),
    false,
  );
});

test("stabilizeProcessRows updates values without moving rows under the user", () => {
  const stable = stabilizeProcessRows(
    [row("1", 10), row("2", 20)],
    [row("2", 88), row("1", 77), row("3", 66)],
  );

  assert.deepEqual(stable.map(processViewRowKey), ["process:1", "process:2", "process:3"]);
  assert.deepEqual(
    stable.map((value) => value.cpu_percent),
    [77, 88, 66],
  );
});

test("only the default attention ranking holds live row order", () => {
  assert.equal(shouldStabilizeProcessOrder("attention"), true);
  assert.equal(shouldStabilizeProcessOrder("cpu"), false);
  assert.equal(shouldStabilizeProcessOrder("memory"), false);
  assert.equal(shouldStabilizeProcessOrder("io"), false);
  assert.equal(shouldStabilizeProcessOrder("network"), false);
  assert.equal(shouldStabilizeProcessOrder("name"), false);
});
