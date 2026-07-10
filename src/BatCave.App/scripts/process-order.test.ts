import assert from "node:assert/strict";
import test from "node:test";
import {
  hasSameProcessOrder,
  processViewRowKey,
  shouldStabilizeProcessOrder,
  stabilizeProcessRows,
  windowProcessViewRows,
} from "../src/lib/process.ts";
import type { ProcessViewRow } from "../src/lib/types.ts";

function row(pid: string, cpuPercent: number, startTimeMs = 0): ProcessViewRow {
  return {
    kind: "process",
    process: {
      pid,
      parent_pid: null,
      start_time_ms: startTimeMs,
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

test("processViewRowKey treats PID reuse as a new process", () => {
  assert.notEqual(processViewRowKey(row("42", 1, 100)), processViewRowKey(row("42", 1, 200)));
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

  assert.deepEqual(stable.map(processViewRowKey), ["process:1:0", "process:2:0", "process:3:0"]);
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

test("result window counts collapsed groups instead of their hidden children", () => {
  const firstGroup = groupRows("first", 4);
  const secondGroup = groupRows("second", 3);
  const thirdGroup = groupRows("third", 2);

  const windowed = windowProcessViewRows([...firstGroup, ...secondGroup, ...thirdGroup], 2);

  assert.deepEqual(
    windowed.filter((value) => value.kind === "group").map((value) => value.group_key),
    ["first", "second"],
  );
  assert.equal(windowed.length, firstGroup.length + secondGroup.length);
  assert.equal(
    windowed.some((value) => value.group_key === "third"),
    false,
  );
});

function groupRows(groupKey: string, childCount: number): ProcessViewRow[] {
  const children = Array.from({ length: childCount }, (_, index) => ({
    ...row(`${groupKey}-${index}`, childCount - index),
    group_key: groupKey,
    group_label: `${groupKey}.exe`,
    group_count: childCount,
    is_grouped: true,
  }));
  const representative = children[0].process;

  return [
    {
      ...children[0],
      kind: "group",
      process: undefined,
      representative,
      is_child: false,
    },
    ...children,
  ];
}
