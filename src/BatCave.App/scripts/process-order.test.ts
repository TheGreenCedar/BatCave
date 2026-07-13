import assert from "node:assert/strict";
import test from "node:test";
import {
  hasSameProcessOrder,
  processViewRowKey,
  processViewRowMetrics,
  prepareProcessViewRows,
  reconcileWorkloadSelection,
  selectedWorkloadDetail,
  shouldHoldProcessOrder,
  shouldStabilizeProcessOrder,
  stabilizeProcessRows,
  windowProcessViewRows,
  workloadSelectionHighlightsRow,
  workloadSelectionMatchesRow,
} from "../src/lib/process.ts";
import type { ProcessViewRow } from "../src/lib/types.ts";

function row(pid: string, cpuPercent: number, startTimeMs = 0): ProcessViewRow {
  return {
    kind: "process",
    detail: {
      kind: "process",
      workload_id: `process:${pid}:${startTimeMs}`,
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
        io_read_total_bytes: 0,
        io_write_total_bytes: 0,
        io_read_bps: 0,
        io_write_bps: 0,
        threads: 1,
        handles: 1,
        access_state: "full",
      },
      io_bps: 0,
      network_bps: 0,
    },
    group_key: `${pid}.exe`,
    group_label: `${pid}.exe`,
    group_category: "Processes",
    group_count: 1,
    icon_kind: "process",
    is_child: false,
    is_grouped: false,
    attention_label: "steady",
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
    stable.map((value) => processViewRowMetrics(value).cpuPercent),
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

test("pointer or keyboard interaction holds any active sort target", () => {
  assert.equal(shouldHoldProcessOrder("cpu", true, 0, false), true);
  assert.equal(shouldHoldProcessOrder("name", true, 0, false), true);
  assert.equal(shouldHoldProcessOrder("cpu", false, 0, true), false);
  assert.equal(shouldHoldProcessOrder("attention", false, 0, true), true);
});

test("selection follows identity through reorder and clears on disappearance or PID reuse", () => {
  const selected = processViewRowKey(row("42", 1, 100));
  const reordered = [row("7", 90), row("42", 3, 100)];
  const replacement = [row("42", 3, 200)];

  assert.equal(reconcileWorkloadSelection(reordered, selected), selected);
  assert.equal(selectedWorkloadDetail(reordered, selected)?.kind, "process");
  assert.equal(reconcileWorkloadSelection([], selected), "");
  assert.equal(reconcileWorkloadSelection(replacement, selected), "");
});

test("result window counts collapsed groups instead of their hidden children", () => {
  const firstGroup = groupRows("first", 4);
  const secondGroup = groupRows("second", 3);
  const thirdGroup = groupRows("third", 2);

  const windowed = windowProcessViewRows([...firstGroup, ...secondGroup, ...thirdGroup], 2);

  assert.deepEqual(
    windowed.filter((value) => value.kind === "group").map((value) => value.detail.group_key),
    ["first", "second"],
  );
  assert.equal(windowed.length, firstGroup.length + secondGroup.length);
  assert.equal(
    windowed.some(
      (value) => (value.kind === "group" ? value.detail.group_key : value.group_key) === "third",
    ),
    false,
  );
});

test("visible workload budgeting preserves later identities behind a large collapsed group", () => {
  const largeGroup = groupRows("large", 220);
  const laterRows = Array.from({ length: 179 }, (_, index) => row(`later-${index}`, 1));
  const selected = processViewRowKey(laterRows.at(-1)!);
  const rawRows = [...largeGroup, ...laterRows];

  const prepared = prepareProcessViewRows(rawRows, selected, 180);

  assert.equal(rawRows.indexOf(laterRows.at(-1)!), 399);
  assert.equal(prepared.selection, selected);
  assert.equal(reconcileWorkloadSelection(prepared.rows, selected), selected);
  assert.equal(
    prepared.rows.filter((candidate) => candidate.kind === "group" || !candidate.is_grouped).length,
    180,
  );
  assert.equal(
    prepared.rows.filter(
      (candidate) => candidate.kind === "process" && candidate.group_key === "large",
    ).length,
    220,
  );
});

test("child selection highlights its group without pressing the group inspection action", () => {
  const rows = groupRows("workers", 2);
  const group = rows[0];
  const child = rows[1];
  const selection = processViewRowKey(child);

  assert.equal(workloadSelectionHighlightsRow(rows, group, selection), true);
  assert.equal(workloadSelectionMatchesRow(group, selection), false);
  assert.equal(workloadSelectionMatchesRow(child, selection), true);
});

function groupRows(groupKey: string, childCount: number): ProcessViewRow[] {
  const children = Array.from({ length: childCount }, (_, index) => ({
    ...row(`${groupKey}-${index}`, childCount - index),
    group_key: groupKey,
    group_label: `${groupKey}.exe`,
    group_count: childCount,
    is_grouped: true,
  }));
  return [
    {
      kind: "group",
      detail: {
        kind: "group",
        workload_id: `group:${groupKey}`,
        group_key: groupKey,
        label: `${groupKey}.exe`,
        category: "Processes",
        process_count: childCount,
        cpu_percent: childCount,
        memory_bytes: 0,
        io_bps: 0,
        network_bps: 0,
        threads: childCount,
        quality: groupQuality(),
        coverage: groupCoverage(childCount),
      },
      icon_kind: "process",
      attention_label: "steady",
    },
    ...children,
  ];
}

function groupQuality() {
  const quality = { quality: "native" as const, source: "process_aggregate" as const };
  const unavailable = { quality: "unavailable" as const, source: "process_aggregate" as const };
  return {
    cpu: quality,
    memory: quality,
    io: quality,
    other_io: unavailable,
    network: quality,
    threads: quality,
  };
}

function groupCoverage(total: number) {
  const coverage = { available: total, total };
  return {
    cpu: coverage,
    memory: coverage,
    io: coverage,
    other_io: { available: 0, total },
    network: coverage,
    threads: coverage,
  };
}
