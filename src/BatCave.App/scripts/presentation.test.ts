import assert from "node:assert/strict";
import test from "node:test";
import { buildPressureBrief } from "../src/lib/cockpit.ts";
import { metricQualityLabel, metricQualityShortLabel } from "../src/lib/format.ts";
import {
  processIdentity,
  processRowSecondaryLabel,
  sortAriaValue,
  sortButtonLabel,
  type ProcessColumn,
} from "../src/lib/process.ts";
import { makeEmptySnapshot } from "../src/lib/runtimeSnapshot.ts";
import type { MetricQualityInfo, ProcessSample, RuntimeSnapshot } from "../src/lib/types.ts";

test("pressure brief preserves full copy while exposing a semantic headline prefix", () => {
  const brief = buildPressureBrief(pressureSnapshot("Code.exe"), 20, 0, 0);

  assert.equal(brief.headlinePrefix, "High CPU pressure");
  assert.equal(brief.headline, "High CPU pressure — Visual Studio Code is the leading workload.");
  assert.equal(
    brief.headline.slice(brief.headlinePrefix.length),
    " — Visual Studio Code is the leading workload.",
  );
});

test("pressure brief uses terminal punctuation when there is no contributor", () => {
  const brief = buildPressureBrief(pressureSnapshot(null), 20, 0, 0);

  assert.equal(brief.headlinePrefix, "High CPU pressure");
  assert.equal(brief.headline, "High CPU pressure.");
  assert.equal(brief.leadingWorkload, null);
});

test("compact metric quality labels keep the full source-aware label available", () => {
  const qualities: Array<[MetricQualityInfo["quality"], string]> = [
    ["native", "Native"],
    ["partial", "Partial"],
    ["estimated", "Estimated"],
    ["held", "Held"],
    ["unavailable", "Unavailable"],
  ];

  for (const [quality, label] of qualities) {
    assert.equal(metricQualityShortLabel({ quality }, "Measured"), label);
  }

  const partial: MetricQualityInfo = { quality: "partial", source: "process_aggregate" };
  assert.equal(metricQualityLabel(partial, "Aggregate"), "Partial / process aggregate");
  assert.equal(metricQualityShortLabel(partial, "Aggregate"), "Partial");
  assert.equal(metricQualityShortLabel(undefined, "Aggregate"), "Aggregate");
  assert.equal(
    metricQualityShortLabel(
      { quality: "future_quality" } as unknown as MetricQualityInfo,
      "Measured",
    ),
    "Measured",
  );
});

test("unavailable metrics remain explicit in compact and detailed labels", () => {
  const unavailable: MetricQualityInfo = {
    quality: "unavailable",
    source: "runtime",
    message: "No trustworthy sample",
  };

  assert.equal(metricQualityShortLabel(unavailable, "Aggregate"), "Unavailable");
  assert.equal(metricQualityLabel(unavailable, "Aggregate"), "Unavailable / runtime");
});

test("sort helpers expose state and the next accessible action", () => {
  const cpuColumn: ProcessColumn = { key: "cpu", label: "CPU", metric: true };

  assert.equal(sortAriaValue("cpu", "cpu", "desc"), "descending");
  assert.equal(sortAriaValue("memory", "cpu", "desc"), "none");
  assert.equal(
    sortButtonLabel(cpuColumn, "cpu", "desc"),
    "CPU, sorted descending. Sort ascending.",
  );
  assert.equal(
    sortButtonLabel({ key: "name", label: "Workload" }, "cpu", "desc"),
    "Sort by Workload ascending.",
  );
});

test("process fallback icon categories are deterministic", () => {
  const safari = process({ name: "Safari", exe: "/Applications/Safari.app/Contents/MacOS/Safari" });
  const unknown = process({ name: "launch-helper", exe: "/usr/libexec/launch-helper" });

  assert.deepEqual(processIdentity(safari), {
    icon: "browser",
    group: "Browsers",
    isChild: false,
  });
  assert.deepEqual(processIdentity(unknown), processIdentity({ ...unknown }));
  assert.deepEqual(processIdentity(unknown), {
    icon: "process",
    group: "Processes",
    isChild: false,
  });
});

test("process row secondary labels keep only useful hierarchy", () => {
  const sample = process({ pid: "99", name: "Codex (Renderer)" });
  const standalone = processRow({ process: sample, group_category: "Processes" });
  const categorized = processRow({ process: sample, group_category: "Browsers" });
  const child = processRow({ process: sample, group_category: "Processes", is_grouped: true });
  const group = processRow({
    kind: "group",
    process: undefined,
    representative: sample,
    group_count: 4,
    is_grouped: true,
  });

  assert.equal(processRowSecondaryLabel(standalone), null);
  assert.equal(processRowSecondaryLabel(categorized), "Browsers");
  assert.equal(processRowSecondaryLabel(child), "PID 99");
  assert.equal(processRowSecondaryLabel(group), "4");
});

function pressureSnapshot(contributor: string | null): RuntimeSnapshot {
  const snapshot = makeEmptySnapshot("");
  snapshot.health.degraded = false;
  snapshot.system.cpu_percent = 90;
  snapshot.system.memory_total_bytes = 100;
  snapshot.system.memory_used_bytes = 20;
  snapshot.system.quality = {
    cpu: { quality: "native", source: "direct_api" },
    memory: { quality: "native", source: "sysinfo" },
    disk: { quality: "native", source: "process_aggregate" },
    network: { quality: "native", source: "interface_aggregate" },
  };
  snapshot.process_contributors.cpu = contributor;
  snapshot.processes = contributor ? [process({ name: contributor })] : [];
  return snapshot;
}

function process(overrides: Partial<ProcessSample> = {}): ProcessSample {
  return {
    pid: "42",
    parent_pid: null,
    start_time_ms: 1,
    name: "process",
    exe: "/usr/bin/process",
    status: "running",
    cpu_percent: 40,
    memory_bytes: 20,
    private_bytes: 20,
    disk_read_total_bytes: 0,
    disk_write_total_bytes: 0,
    disk_read_bps: 0,
    disk_write_bps: 0,
    threads: 1,
    handles: 1,
    access_state: "full",
    ...overrides,
  };
}

function processRow(overrides: Partial<import("../src/lib/types.ts").ProcessViewRow> = {}) {
  return {
    kind: "process" as const,
    process: process(),
    group_count: 1,
    icon_kind: "process",
    is_child: false,
    is_grouped: false,
    attention_label: "Normal",
    cpu_percent: 0,
    memory_bytes: 0,
    io_bps: 0,
    network_bps: 0,
    threads: 1,
    ...overrides,
  };
}
