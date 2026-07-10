import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import { compareProcessSamples, processNeedsAttention } from "../src/lib/process.ts";
import { currentDiagnosticIssues, uniqueWarningCount } from "../src/lib/diagnostics.ts";
import { qualityGuidance } from "../src/lib/format.ts";
import { hasNewRuntimeSample, makeDefaultRuntimeQuery } from "../src/lib/runtimeSnapshot.ts";
import { summarizeProcessContributors, systemPressureHeadline } from "../src/lib/systemPressure.ts";
import type { ProcessSample, RuntimeAdminModeStatus, RuntimeWarning } from "../src/lib/types.ts";

const canonicalSnapshot = JSON.parse(
  readFileSync(new URL("./fixtures/runtime-snapshot.v2.json", import.meta.url), "utf8"),
);
const themeCss = readFileSync(new URL("../src/styles/themes.css", import.meta.url), "utf8");

function process(overrides: Partial<ProcessSample> = {}): ProcessSample {
  return {
    pid: "1",
    parent_pid: null,
    start_time_ms: 1,
    name: "quiet.exe",
    exe: "C:\\quiet.exe",
    status: "idle",
    cpu_percent: 0,
    memory_bytes: 1,
    private_bytes: 1,
    disk_read_total_bytes: 0,
    disk_write_total_bytes: 0,
    disk_read_bps: 0,
    disk_write_bps: 0,
    other_io_bps: 0,
    network_received_bps: 0,
    network_transmitted_bps: 0,
    threads: 1,
    handles: 1,
    access_state: "full",
    ...overrides,
  };
}

test("control publications do not look like new telemetry samples", () => {
  assert.equal(hasNewRuntimeSample({ sample_seq: 7 }, { sample_seq: 7 }), false);
  assert.equal(hasNewRuntimeSample({ sample_seq: 7 }, { sample_seq: 8 }), true);
});

test("shared fixture exposes the preview environment and stable empty arrays", () => {
  assert.deepEqual(canonicalSnapshot.environment, {
    platform: "windows",
    admin_mode_available: true,
    data_directory: "C:\\Users\\test\\BatCaveMonitor",
  });
  assert.deepEqual(
    canonicalSnapshot.system.memory_accounting.kernel_pool_tags[0].driver_candidates,
    [],
  );
  assert.equal(canonicalSnapshot.seq, undefined);
  assert.equal(canonicalSnapshot.ts_ms, undefined);
  assert.deepEqual(canonicalSnapshot.admin_mode, {
    state: "off",
    detail: null,
    last_success_at_ms: null,
  });
});

test("diagnostics render one limitation per stable key with the current admin action", () => {
  const warnings: RuntimeWarning[] = [
    warning("collector.network_attribution", "network_attribution_failed: access denied", 1),
    warning("collector.network_attribution", "network_attribution_failed: retry failed", 2),
  ];

  assert.deepEqual(
    currentDiagnosticIssues(warnings, adminMode("off"), true).map((issue) => [
      issue.key,
      issue.action,
    ]),
    [["collector.network_attribution", "enable"]],
  );
  assert.equal(
    currentDiagnosticIssues(warnings, adminMode("requesting"), true)[0].action,
    "cancel",
  );
  assert.equal(currentDiagnosticIssues(warnings, adminMode("failed"), true)[0].action, "retry");
  assert.equal(currentDiagnosticIssues(warnings, adminMode("active"), true)[0].action, null);
  assert.equal(uniqueWarningCount(warnings), 1);
});

test("native metrics omit empty quality guidance", () => {
  assert.deepEqual(
    qualityGuidance({
      cpu: { quality: "native", source: "direct_api" },
      disk: { quality: "native", source: "pdh" },
      network: { quality: "native", source: "interface_aggregate" },
    }),
    [],
  );
  assert.deepEqual(
    qualityGuidance({ network: { quality: "unavailable", message: "ETW access denied" } }),
    ["ETW access denied"],
  );
});

test("attention includes each scored resource and limited access", () => {
  const quiet = process();

  assert.equal(processNeedsAttention(quiet), false);
  assert.equal(processNeedsAttention(process({ cpu_percent: 1 })), true);
  assert.equal(processNeedsAttention(process({ memory_bytes: 900 * 1024 * 1024 })), true);
  assert.equal(processNeedsAttention(process({ disk_read_bps: 500 * 1024 })), true);
  assert.equal(processNeedsAttention(process({ network_received_bps: 1024 * 1024 })), true);
  assert.equal(processNeedsAttention(process({ access_state: "partial" })), true);
});

test("fixture comparator honors network sorting", () => {
  const query = {
    ...makeDefaultRuntimeQuery(),
    sort_column: "network_bps" as const,
    sort_direction: "desc" as const,
  };
  const rows = [
    process({ name: "low", network_received_bps: 1 }),
    process({ name: "high", network_received_bps: 100 }),
  ].sort((left, right) => compareProcessSamples(left, right, query));

  assert.deepEqual(
    rows.map((row) => row.name),
    ["high", "low"],
  );
});

test("pressure attribution follows the dominant resource, not display order", () => {
  const processes = [
    process({ name: "CPU first", disk_read_bps: 1 }),
    process({ name: "Disk winner", disk_read_bps: 60 * 1024 * 1024 }),
  ];
  const contributors = summarizeProcessContributors(processes);

  assert.match(systemPressureHeadline(10, 20, 60 * 1024 * 1024, 0, contributors), /Disk winner/);
  assert.equal(systemPressureHeadline(10, 20, 0, 0, contributors), "System is steady.");
});

test("search and focus cannot change the headline contributor", () => {
  const cpuWinner = process({ name: "CPU winner", cpu_percent: 80 });
  const visibleIo = process({ name: "Visible I/O", disk_read_bps: 1024 });
  const allProcesses = [cpuWinner, visibleIo];
  const searchedRows = allProcesses.filter((candidate) => candidate.name.includes("Visible"));
  const focusedRows = allProcesses.filter(
    (candidate) => candidate.disk_read_bps + candidate.disk_write_bps > 0,
  );
  const contributors = summarizeProcessContributors(allProcesses);

  assert.deepEqual(
    searchedRows.map((candidate) => candidate.name),
    ["Visible I/O"],
  );
  assert.deepEqual(
    focusedRows.map((candidate) => candidate.name),
    ["Visible I/O"],
  );
  assert.equal(
    systemPressureHeadline(90, 0, 0, 0, contributors),
    "High CPU pressure - CPU winner is the top activity.",
  );
});

test("all theme text and focus colors meet contrast floors", () => {
  const blocks = [
    themeCss.match(/\.app-shell,\s*:root\s*\{([^}]*)\}/s)?.[1],
    ...[...themeCss.matchAll(/\.app-shell\[data-theme="[^"]+"\]\s*\{([^}]*)\}/gs)].map(
      (match) => match[1],
    ),
  ];

  assert.equal(blocks.length, 4);
  for (const block of blocks) {
    assert.ok(block);
    const variables = Object.fromEntries(
      [...block.matchAll(/--([\w-]+):\s*(#[\da-f]{6})/gi)].map((match) => [match[1], match[2]]),
    );
    for (const surface of ["surface-0", "surface-1", "surface-2", "surface-3"]) {
      assert.ok(contrast(variables["text-subtle"], variables[surface]) >= 4.5);
      assert.ok(contrast(variables.accent, variables[surface]) >= 3);
    }
  }
});

function contrast(left: string, right: string): number {
  const [lighter, darker] = [luminance(left), luminance(right)].sort((a, b) => b - a);
  return (lighter + 0.05) / (darker + 0.05);
}

function luminance(color: string): number {
  const channels = [1, 3, 5].map(
    (index) => Number.parseInt(color.slice(index, index + 2), 16) / 255,
  );
  const [red, green, blue] = channels.map((value) =>
    value <= 0.03928 ? value / 12.92 : ((value + 0.055) / 1.055) ** 2.4,
  );
  return 0.2126 * red + 0.7152 * green + 0.0722 * blue;
}

function warning(key: string, message: string, publicationSeq: number): RuntimeWarning {
  return {
    key,
    message,
    publication_seq: publicationSeq,
    occurred_at_ms: publicationSeq,
    category: "collector",
  };
}

function adminMode(state: RuntimeAdminModeStatus["state"]): RuntimeAdminModeStatus {
  return { state, detail: null, last_success_at_ms: null };
}
