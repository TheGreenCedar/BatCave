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
  settleProcessRanking,
  advanceProcessRanking,
  rankingNearTie,
  ProcessInteraction,
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

test("only active Explore interaction holds ranking, passive selection does not", () => {
  assert.equal(shouldHoldProcessOrder({ view: "explore", interacting: true }), true);
  assert.equal(shouldHoldProcessOrder({ view: "explore", interacting: false }), false);
  assert.equal(shouldHoldProcessOrder({ view: "overview", interacting: true }), false);
});

test("pointer exit retains a keyboard interaction until focus also leaves", () => {
  const interaction = new ProcessInteraction();
  assert.equal(interaction.set("pointer", true), true);
  assert.equal(interaction.set("focus", true), true);
  assert.equal(interaction.set("pointer", false), true);
  assert.equal(interaction.set("focus", false), false);
  assert.equal(interaction.set("pointer", true), true);
  assert.equal(interaction.set("focus", false), true);
  assert.equal(interaction.set("pointer", false), false);
});

test("ranking releases on pointer exit and navigation while retaining fresh values and identities", () => {
  const initial = [row("1", 20), row("2", 10)];
  const next = [row("2", 90), row("1", 40)];
  const held = advanceProcessRanking(initial, next, true);
  assert.equal(held.updateAvailable, true);
  assert.deepEqual(held.rows.map(processViewRowKey), initial.map(processViewRowKey));
  assert.deepEqual(
    held.rows.map((row) => processViewRowMetrics(row).cpuPercent),
    [40, 90],
  );
  const released = advanceProcessRanking(held.rows, next, false);
  assert.deepEqual(released.rows, next);
  assert.equal(released.updateAvailable, false);
  const overview = advanceProcessRanking(
    held.rows,
    [],
    shouldHoldProcessOrder({ view: "overview", interacting: true }),
  );
  assert.deepEqual(overview.rows, []);
  assert.equal(overview.updateAvailable, false);
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

test("selection follows a backend-owned group identity through presentation updates", () => {
  const selected = "group:visual studio code";
  const enrichedRows = groupRows("visual studio code", 2);
  const group = enrichedRows[0];
  if (group.kind !== "group") throw new Error("expected group row");
  group.detail.label = "Visual Studio Code";
  group.icon_source = "C:\\Program Files\\Microsoft VS Code\\Code.exe";

  assert.equal(group.detail.workload_id, selected);
  assert.equal(reconcileWorkloadSelection(enrichedRows, selected), selected);
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

test("result window clears a selection that no longer has a visible row", () => {
  const rows = Array.from({ length: 181 }, (_, index) => row(`process-${index}`, 1));
  const selected = processViewRowKey(rows.at(-1)!);

  const prepared = prepareProcessViewRows(rows, selected, 180);

  assert.equal(prepared.rows.length, 180);
  assert.equal(prepared.selection, "");
  assert.equal(reconcileWorkloadSelection(prepared.rows, selected), "");
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

test("settleProcessRanking keeps an adjacent swap stable inside the settle interval", () => {
  const current = [row("1", 10), row("2", 9), row("3", 8)];
  const incoming = [row("2", 10), row("1", 9), row("3", 8)];
  const settled = settleProcessRanking(current, incoming, 5_000, 1_000);
  assert.deepEqual(settled.rows.map(processViewRowKey), current.map(processViewRowKey));
  assert.equal(settled.rows[0], incoming[1]);
  assert.equal(settled.settledAt, 1_000);
});

test("settleProcessRanking adopts the incoming order after the settle interval", () => {
  const current = [row("1", 10), row("2", 9), row("3", 8)];
  const incoming = [row("2", 10), row("1", 9), row("3", 8)];
  const settled = settleProcessRanking(current, incoming, 11_000, 1_000);
  assert.deepEqual(settled.rows.map(processViewRowKey), incoming.map(processViewRowKey));
  assert.equal(settled.settledAt, 11_000);
});

test("settleProcessRanking adopts displacement beyond one row immediately", () => {
  const current = [row("1", 10), row("2", 9), row("3", 8)];
  const incoming = [row("3", 10), row("1", 9), row("2", 8)];
  const settled = settleProcessRanking(current, incoming, 5_000, 1_000);
  assert.deepEqual(settled.rows.map(processViewRowKey), incoming.map(processViewRowKey));
  assert.equal(settled.settledAt, 5_000);
});

test("settleProcessRanking inserts a new row at its incoming position without reordering", () => {
  const current = [row("1", 10), row("2", 9), row("3", 8)];
  const incoming = [row("1", 10), row("4", 9.5), row("2", 9), row("3", 8)];
  const settled = settleProcessRanking(current, incoming, 5_000, 1_000);
  assert.deepEqual(settled.rows.map(processViewRowKey), [
    "process:1:0",
    "process:4:0",
    "process:2:0",
    "process:3:0",
  ]);
  assert.equal(settled.rows[0], incoming[0]);
  assert.equal(settled.settledAt, 1_000);
});

test("settleProcessRanking drops a removed row without reordering the rest", () => {
  const current = [row("1", 10), row("2", 9), row("3", 8)];
  const incoming = [row("1", 10), row("3", 8)];
  const settled = settleProcessRanking(current, incoming, 5_000, 1_000);
  assert.deepEqual(settled.rows.map(processViewRowKey), ["process:1:0", "process:3:0"]);
  assert.equal(settled.settledAt, 1_000);
});

test("settleProcessRanking measures displacement only among common rows", () => {
  const current = [row("1", 10), row("2", 9), row("3", 8), row("4", 7)];
  const incoming = [row("2", 9), row("1", 10), row("4", 7)];
  const settled = settleProcessRanking(current, incoming, 5_000, 1_000);
  assert.deepEqual(settled.rows.map(processViewRowKey), [
    "process:1:0",
    "process:2:0",
    "process:4:0",
  ]);
  assert.equal(settled.settledAt, 1_000);
});

const cpuNearTie = (a: ProcessViewRow, b: ProcessViewRow) =>
  rankingNearTie(2)(processViewRowMetrics(a).cpuPercent, processViewRowMetrics(b).cpuPercent);

test("settleProcessRanking re-sorts a large inversion immediately when near-tie is provided", () => {
  const quiet = row("1", 23);
  const busy = row("2", 182);
  const settled = settleProcessRanking(
    [quiet, busy],
    [busy, quiet],
    5_000,
    1_000,
    10_000,
    cpuNearTie,
  );
  assert.deepEqual(settled.rows.map(processViewRowKey), ["process:2:0", "process:1:0"]);
  assert.equal(settled.settledAt, 5_000);
});

test("settleProcessRanking holds a near-tie inversion inside the settle interval", () => {
  const a = row("1", 20.0);
  const b = row("2", 21.0);
  const settled = settleProcessRanking([a, b], [b, a], 5_000, 1_000, 10_000, cpuNearTie);
  assert.deepEqual(settled.rows.map(processViewRowKey), ["process:1:0", "process:2:0"]);
  assert.equal(settled.settledAt, 1_000);
});

test("settleProcessRanking applies the near-tie floor to memory-sized values", () => {
  const mib = 1024 * 1024;
  const memoryNearTie = (a: ProcessViewRow, b: ProcessViewRow) =>
    rankingNearTie(32 * mib)(
      processViewRowMetrics(a).memoryBytes,
      processViewRowMetrics(b).memoryBytes,
    );
  const a = row("1", 0);
  if (a.kind === "process") a.detail.process.memory_bytes = 1024 * mib;
  const b = row("2", 0);
  if (b.kind === "process") b.detail.process.memory_bytes = 1024 * mib + 20 * mib;
  const held = settleProcessRanking([a, b], [b, a], 5_000, 1_000, 10_000, memoryNearTie);
  assert.deepEqual(held.rows.map(processViewRowKey), ["process:1:0", "process:2:0"]);

  const far = row("3", 0);
  if (far.kind === "process") far.detail.process.memory_bytes = 1024 * mib + 400 * mib;
  const adopted = settleProcessRanking([a, far], [far, a], 5_000, 1_000, 10_000, memoryNearTie);
  assert.deepEqual(adopted.rows.map(processViewRowKey), ["process:3:0", "process:1:0"]);
  assert.equal(adopted.settledAt, 5_000);
});

test("settleProcessRanking without a near-tie keeps the previous one-position behavior", () => {
  const quiet = row("1", 23);
  const busy = row("2", 182);
  const settled = settleProcessRanking([quiet, busy], [busy, quiet], 5_000, 1_000);
  assert.deepEqual(settled.rows.map(processViewRowKey), ["process:1:0", "process:2:0"]);
  assert.equal(settled.settledAt, 1_000);
});
