import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import { buildResourceBrief } from "../src/lib/cockpit.ts";
import {
  displayMetricValue,
  displayGroupMetricValue,
  displayProcessMetricValue,
  formatOptionalRate,
  formatPercent,
  formatRate,
  groupFindingLabel,
  groupMetricCanDisplay,
  logicalCpuMetricQuality,
  metricQualityLabel,
  metricQualityShortLabel,
  nextProcessMetricHistory,
  processActivityLabel,
  processBytesLabel,
  processFindingLabel,
  processPrivateMemoryValue,
  processTrustLabel,
} from "../src/lib/format.ts";
import { nextMetricHistory, resourceHistoryWindowLabel } from "../src/lib/history.ts";
import {
  nextSortDirection,
  processIdentity,
  processRowSecondaryLabel,
  sortAriaValue,
  sortButtonLabel,
  sortDirectionButtonLabel,
  type ProcessColumn,
} from "../src/lib/process.ts";
import { makeEmptySnapshot } from "../src/lib/runtimeSnapshot.ts";
import type { MetricQualityInfo, ProcessSample, RuntimeSnapshot } from "../src/lib/types.ts";

test("selected resource brief keeps machine and process CPU scopes explicit", () => {
  const brief = buildResourceBrief(
    resourceSnapshot("Code.exe"),
    "cpu",
    { memoryPercent: 20, diskRate: 0, networkRate: 0 },
    "live",
  );

  assert.equal(brief.headline, "Machine-total CPU is 90%.");
  assert.equal(brief.leadingWorkload, "Visual Studio Code");
  assert.equal(brief.contributorStatusLabel, "40% of one core");
  assert.match(brief.attributionLabel, /one-core-equivalent/);
  assert.equal(brief.stateLabel, "Current");
  assert.equal(brief.confidence, "High");
});

test("contributor quality gates publication and limits overview confidence", () => {
  const estimated = resourceSnapshot("Estimated worker");
  estimated.process_contributors.cpu_quality = {
    quality: "estimated",
    source: "sysinfo",
  };
  const estimatedBrief = buildResourceBrief(
    estimated,
    "cpu",
    { memoryPercent: 20, diskRate: 0, networkRate: 0 },
    "live",
  );
  assert.equal(estimatedBrief.leadingWorkload, "Estimated worker");
  assert.match(estimatedBrief.contributorStatusLabel, /Estimated attribution/);
  assert.equal(estimatedBrief.confidence, "Limited");

  const unavailable = resourceSnapshot("Blocked worker");
  unavailable.process_contributors.cpu_quality = {
    quality: "unavailable",
    source: "runtime",
  };
  const unavailableBrief = buildResourceBrief(
    unavailable,
    "cpu",
    { memoryPercent: 20, diskRate: 0, networkRate: 0 },
    "live",
  );
  assert.equal(unavailableBrief.leadingWorkload, null);
  assert.equal(unavailableBrief.confidence, "Limited");
});

test("full-sample contributor ambiguity survives a query that retains one matching row", () => {
  const snapshot = resourceSnapshot("worker");
  snapshot.process_contributors.cpu_name_ambiguous = true;
  snapshot.processes = [process({ pid: "1", name: "worker" })];

  const brief = buildResourceBrief(
    snapshot,
    "cpu",
    { memoryPercent: 20, diskRate: 0, networkRate: 0 },
    "live",
  );

  assert.equal(brief.leadingWorkload, "worker");
  assert.equal(brief.contributorNameAmbiguous, true);
  assert.match(brief.contributorStatusLabel, /ambiguous across the full process sample/);
  assert.equal(brief.confidence, "Limited");
});

test("legacy contributor summaries without ambiguity truth do not lend a visible row value", () => {
  const snapshot = resourceSnapshot("worker");
  delete (snapshot.process_contributors as Partial<typeof snapshot.process_contributors>)
    .cpu_name_ambiguous;

  const brief = buildResourceBrief(
    snapshot,
    "cpu",
    { memoryPercent: 20, diskRate: 0, networkRate: 0 },
    "live",
  );

  assert.equal(brief.contributorNameAmbiguous, true);
  assert.match(brief.contributorStatusLabel, /ambiguous across the full process sample/);
});

test("physical disk summary rejects process I/O as compatible attribution", () => {
  const snapshot = resourceSnapshot(null);
  snapshot.process_contributors.io = "I/O winner";
  const brief = buildResourceBrief(
    snapshot,
    "disk",
    { memoryPercent: 20, diskRate: 4_096, networkRate: 0 },
    "live",
  );

  assert.equal(brief.headline, "Physical disk throughput is 4.0 KB/s.");
  assert.equal(brief.leadingWorkload, null);
  assert.equal(brief.contributorStatusLabel, "No compatible process attribution");
  assert.equal(brief.confidence, "High");
  assert.match(brief.attributionLabel, /not used as physical-disk attribution/);
});

test("missing contributor quality limits zero-activity attribution without hiding its status", () => {
  const snapshot = resourceSnapshot(null);
  snapshot.system.cpu_percent = 0;
  snapshot.system.process_count = 1;
  snapshot.total_process_count = 1;
  snapshot.processes = [process({ cpu_percent: 0 })];

  const brief = buildResourceBrief(
    snapshot,
    "cpu",
    { memoryPercent: 20, diskRate: 0, networkRate: 0 },
    "live",
  );

  assert.equal(snapshot.system.quality?.cpu?.quality, "native");
  assert.equal(snapshot.process_contributors.cpu_quality, undefined);
  assert.equal(brief.leadingWorkload, null);
  assert.equal(brief.confidence, "Limited");
  assert.equal(brief.contributorStatusLabel, "Attribution quality not reported");
});

test("incomplete zero-activity coverage limits contributor confidence", () => {
  const snapshot = resourceSnapshot(null);
  snapshot.system.cpu_percent = 0;
  snapshot.system.process_count = 2;
  snapshot.total_process_count = 2;
  snapshot.process_contributors.cpu_quality = { quality: "unavailable", source: "runtime" };

  const brief = buildResourceBrief(
    snapshot,
    "cpu",
    { memoryPercent: 20, diskRate: 0, networkRate: 0 },
    "live",
  );

  assert.equal(brief.leadingWorkload, null);
  assert.equal(brief.confidence, "Limited");
  assert.equal(brief.contributorStatusLabel, "Process attribution unavailable");
});

test("blocked process coverage suppresses a positive visible contributor", () => {
  const snapshot = resourceSnapshot(null);
  snapshot.system.cpu_percent = 25;
  snapshot.system.process_count = 2;
  snapshot.total_process_count = 2;
  snapshot.process_contributors.cpu_quality = { quality: "held", source: "runtime" };
  snapshot.processes = [
    process({ name: "Native visible", cpu_percent: 25, quality: { cpu: { quality: "native" } } }),
    process({ name: "Held placeholder", cpu_percent: 0, quality: { cpu: { quality: "held" } } }),
  ];

  const brief = buildResourceBrief(
    snapshot,
    "cpu",
    { memoryPercent: 20, diskRate: 0, networkRate: 0 },
    "live",
  );

  assert.equal(brief.leadingWorkload, null);
  assert.equal(brief.confidence, "Limited");
  assert.equal(brief.contributorStatusLabel, "Process attribution pending");
});

test("held system CPU suppresses its contributor without claiming no activity", () => {
  const snapshot = resourceSnapshot("worker");
  snapshot.system.cpu_percent = 0;
  snapshot.system.process_count = 1;
  snapshot.total_process_count = 1;
  snapshot.system.quality!.cpu = { quality: "held", source: "direct_api" };
  snapshot.processes = [process({ name: "worker", cpu_percent: 25 })];

  const brief = buildResourceBrief(
    snapshot,
    "cpu",
    { memoryPercent: 20, diskRate: 0, networkRate: 0 },
    "live",
  );

  assert.equal(brief.headline, "Machine-total CPU has no trusted sample.");
  assert.equal(brief.leadingWorkload, null);
  assert.equal(brief.contributorStatusLabel, "Process attribution pending");
  assert.doesNotMatch(brief.contributorStatusLabel, /no process activity/i);
});

test("Linux first system sample keeps CPU disk network and core history pending", () => {
  const snapshot = resourceSnapshot(null);
  snapshot.system.cpu_percent = 73;
  snapshot.system.disk_read_bps = 8 * 1024 * 1024;
  snapshot.system.network_received_bps = 4 * 1024 * 1024;
  snapshot.system.logical_cpu_percent = [90, 40];
  snapshot.system.quality!.cpu = { quality: "held", source: "procfs" };
  snapshot.system.quality!.logical_cpu = { quality: "held", source: "procfs" };
  snapshot.system.quality!.disk = { quality: "held", source: "procfs" };
  snapshot.system.quality!.network = { quality: "held", source: "procfs" };

  for (const mode of ["cpu", "disk", "network"] as const) {
    const brief = buildResourceBrief(
      snapshot,
      mode,
      { memoryPercent: 20, diskRate: 8 * 1024 * 1024, networkRate: 4 * 1024 * 1024 },
      "live",
    );
    assert.equal(brief.valueLabel, "Waiting", `${mode} readout`);
    assert.equal(brief.confidence, "Unavailable", `${mode} confidence`);
    assert.match(brief.headline, /no trusted sample/, `${mode} headline`);
  }

  for (const value of snapshot.system.logical_cpu_percent) {
    assert.deepEqual(nextMetricHistory([12], value, snapshot.system.quality?.logical_cpu, 30), []);
  }
  snapshot.system.quality!.logical_cpu = { quality: "native", source: "procfs" };
  const aggregateHeldCoreQuality = logicalCpuMetricQuality(snapshot.system.quality!);
  assert.equal(aggregateHeldCoreQuality?.quality, "held");
  assert.equal(
    displayMetricValue(0, aggregateHeldCoreQuality, snapshot.sampled_at_ms, formatPercent),
    "Waiting",
  );
  assert.deepEqual(nextMetricHistory([12], 90, aggregateHeldCoreQuality, 30), []);
  snapshot.system.quality!.cpu = { quality: "unavailable", source: "procfs" };
  const aggregateUnavailableCoreQuality = logicalCpuMetricQuality(snapshot.system.quality!);
  assert.equal(
    displayMetricValue(0, aggregateUnavailableCoreQuality, snapshot.sampled_at_ms, formatPercent),
    "Unavailable",
  );
  assert.deepEqual(nextMetricHistory([1], 8, snapshot.system.quality?.disk, 30), []);
  assert.deepEqual(nextMetricHistory([1], 4, snapshot.system.quality?.network, 30), []);
});

test("overview states cannot turn missing or retained samples into a reassuring zero", () => {
  const zero = resourceSnapshot(null);
  zero.system.cpu_percent = 0;
  const current = buildResourceBrief(
    zero,
    "cpu",
    { memoryPercent: 0, diskRate: 0, networkRate: 0 },
    "live",
  );
  assert.equal(current.valueLabel, "0%");
  assert.equal(current.stateLabel, "Current");
  assert.equal(current.headline, "Machine-total CPU is 0%.");
  assert.equal(current.confidence, "High");
  assert.equal(current.contributorStatusLabel, "No processes in this sample");

  const unavailable = resourceSnapshot(null);
  unavailable.system.cpu_percent = 0;
  unavailable.system.quality!.cpu = { quality: "unavailable", source: "runtime" };
  const missing = buildResourceBrief(
    unavailable,
    "cpu",
    { memoryPercent: 0, diskRate: 0, networkRate: 0 },
    "live",
  );
  assert.equal(missing.valueLabel, "Unavailable");
  assert.equal(missing.stateLabel, "Unavailable");
  assert.equal(missing.confidence, "Unavailable");
  assert.equal(missing.headline, "Machine-total CPU has no trusted sample.");

  const partial = resourceSnapshot(null);
  partial.system.quality!.cpu = { quality: "partial", source: "sysinfo" };
  const limited = buildResourceBrief(
    partial,
    "cpu",
    { memoryPercent: 0, diskRate: 0, networkRate: 0 },
    "live",
  );
  assert.equal(limited.valueLabel, "90%");
  assert.equal(limited.stateLabel, "Partial");
  assert.equal(limited.confidence, "Limited");

  const degraded = resourceSnapshot(null);
  degraded.health.degraded = true;
  const warning = buildResourceBrief(
    degraded,
    "cpu",
    { memoryPercent: 0, diskRate: 0, networkRate: 0 },
    "live",
  );
  assert.equal(warning.valueLabel, "90%");
  assert.equal(warning.stateLabel, "Degraded");
  assert.equal(warning.confidence, "Limited");

  const paused = buildResourceBrief(
    resourceSnapshot(null),
    "cpu",
    { memoryPercent: 0, diskRate: 0, networkRate: 0 },
    "paused",
  );
  assert.equal(paused.stateLabel, "Paused");
  assert.equal(paused.headline, "Machine-total CPU was 90% when collection paused.");

  const stale = buildResourceBrief(
    resourceSnapshot(null),
    "cpu",
    { memoryPercent: 0, diskRate: 0, networkRate: 0 },
    "stale",
  );
  assert.equal(stale.stateLabel, "Stale");
  assert.equal(stale.headline, "Machine-total CPU was 90% in the last successful sample.");
});

test("unavailable resource samples clear trusted history and its current window label", () => {
  assert.deepEqual(
    nextMetricHistory([12, 18], 0, { quality: "unavailable", source: "runtime" }, 30),
    [],
  );
  assert.deepEqual(nextMetricHistory([12, 18], 0, { quality: "held" }, 30), []);
  assert.deepEqual(nextMetricHistory([12, 18], 0, { quality: "native" }, 30), [12, 18, 0]);
  assert.deepEqual(nextMetricHistory([12, 18], 9, { quality: "partial" }, 30), [12, 18, 9]);
  assert.equal(
    resourceHistoryWindowLabel(12, 1_000, { quality: "unavailable" }, true),
    "No trusted history",
  );
  assert.equal(
    resourceHistoryWindowLabel(12, 1_000, { quality: "held" }, true),
    "No trusted history",
  );
  assert.equal(resourceHistoryWindowLabel(12, 1_000, { quality: "native" }, true), "Last 11s");
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
  assert.equal(displayMetricValue(0, unavailable, 1, String), "Unavailable");
  assert.equal(displayMetricValue(0, { quality: "held" }, 1, String), "Waiting");
  assert.equal(displayMetricValue(0, { quality: "native" }, 1, String), "0");
  assert.equal(displayMetricValue(0, { quality: "native" }, null, String), "Unavailable");
});

test("Linux first-sample and denied Windows rows never publish placeholder CPU or I/O zeros", () => {
  const linuxFirstSample = process({
    cpu_percent: 0,
    io_read_bps: 0,
    io_write_bps: 0,
    quality: {
      cpu: { quality: "held", source: "procfs" },
      memory: { quality: "native", source: "procfs" },
      io: { quality: "held", source: "procfs" },
      other_io: { quality: "unavailable", source: "procfs" },
      network: { quality: "unavailable", source: "procfs" },
    },
  });
  assert.equal(
    displayProcessMetricValue(
      linuxFirstSample.cpu_percent,
      linuxFirstSample.quality?.cpu,
      formatPercent,
    ),
    "Pending",
  );
  assert.equal(
    displayProcessMetricValue(
      linuxFirstSample.io_read_bps + linuxFirstSample.io_write_bps,
      linuxFirstSample.quality?.io,
      formatRate,
    ),
    "Pending",
  );
  assert.deepEqual(nextProcessMetricHistory([12], 0, linuxFirstSample.quality?.cpu, 30), []);
  assert.deepEqual(nextProcessMetricHistory([4_096], 0, linuxFirstSample.quality?.io, 30), []);
  assert.equal(
    displayProcessMetricValue(0, linuxFirstSample.quality?.network, formatRate),
    "Unavailable",
  );
  assert.equal(
    displayProcessMetricValue(
      linuxFirstSample.other_io_bps,
      linuxFirstSample.quality?.other_io,
      formatOptionalRate,
    ),
    "Unavailable",
  );
  assert.equal(processActivityLabel(linuxFirstSample, 0, 0), "Pending");

  const deniedWindows = process({
    cpu_percent: 0,
    memory_bytes: 2 * 1024 * 1024 * 1024,
    io_read_bps: 0,
    io_write_bps: 0,
    access_state: "denied",
    quality: {
      cpu: { quality: "unavailable", source: "direct_api" },
      memory: { quality: "unavailable", source: "direct_api" },
      io: { quality: "unavailable", source: "direct_api" },
      other_io: { quality: "unavailable", source: "direct_api" },
      network: { quality: "unavailable", source: "direct_api" },
    },
  });
  assert.equal(
    displayProcessMetricValue(deniedWindows.cpu_percent, deniedWindows.quality?.cpu, formatPercent),
    "Unavailable",
  );
  assert.equal(
    displayProcessMetricValue(
      deniedWindows.io_read_bps + deniedWindows.io_write_bps,
      deniedWindows.quality?.io,
      formatRate,
    ),
    "Unavailable",
  );
  assert.deepEqual(nextProcessMetricHistory([12], 0, deniedWindows.quality?.cpu, 30), []);
  assert.deepEqual(nextProcessMetricHistory([4_096], 0, deniedWindows.quality?.io, 30), []);
  assert.deepEqual(nextProcessMetricHistory([50], 0, deniedWindows.quality?.memory, 30), []);
  assert.deepEqual(nextProcessMetricHistory([8], 0, deniedWindows.quality?.network, 30), []);
  assert.equal(processBytesLabel(deniedWindows, deniedWindows.memory_bytes), "Unavailable");
  assert.equal(
    displayProcessMetricValue(0, deniedWindows.quality?.network, formatRate),
    "Unavailable",
  );
  assert.equal(
    displayProcessMetricValue(
      deniedWindows.other_io_bps,
      deniedWindows.quality?.other_io,
      formatOptionalRate,
    ),
    "Unavailable",
  );
  assert.equal(
    processFindingLabel(deniedWindows, 0, 0, "Working set"),
    "Some activity metrics are unavailable for this workload.",
  );
  assert.equal(processActivityLabel(deniedWindows, 0, 0), "Unavailable");
});

test("process history records only explicitly publishable metric samples", () => {
  assert.deepEqual(nextProcessMetricHistory([12], 0, undefined, 30), []);
  assert.deepEqual(nextProcessMetricHistory([12], 0, { quality: "held" }, 30), []);
  assert.deepEqual(nextProcessMetricHistory([12], 0, { quality: "unavailable" }, 30), []);
  assert.deepEqual(nextProcessMetricHistory([12], 0, { quality: "native" }, 30), [12, 0]);
  assert.deepEqual(nextProcessMetricHistory([12], 8, { quality: "estimated" }, 30), [12, 8]);
});

test("missing per-process metric quality never turns a placeholder zero into a measurement", () => {
  const unknown = process({ cpu_percent: 0, io_read_bps: 0, io_write_bps: 0, quality: undefined });

  assert.equal(
    displayProcessMetricValue(unknown.cpu_percent, unknown.quality?.cpu, formatPercent),
    "Quality not reported",
  );
  assert.equal(
    displayProcessMetricValue(0, unknown.quality?.io, formatRate),
    "Quality not reported",
  );
  assert.equal(processBytesLabel(unknown, unknown.memory_bytes), "Quality not reported");
  assert.equal(
    displayProcessMetricValue(0, unknown.quality?.network, formatRate),
    "Quality not reported",
  );
  assert.equal(
    displayProcessMetricValue(undefined, unknown.quality?.other_io, formatOptionalRate),
    "Quality not reported",
  );
  assert.deepEqual(nextProcessMetricHistory([12], 0, unknown.quality?.cpu, 30), []);
  assert.deepEqual(nextProcessMetricHistory([50], 0, unknown.quality?.memory, 30), []);
  assert.deepEqual(nextProcessMetricHistory([8], 0, unknown.quality?.network, 30), []);
  assert.equal(
    processFindingLabel(unknown, 0, 0, "Working set"),
    "Some activity metric quality was not reported for this workload.",
  );
  assert.equal(processActivityLabel(unknown, 0, 0), "Quality not reported");
});

test("publishable process metrics retain real zero values", () => {
  const native = process({
    cpu_percent: 0,
    memory_bytes: 0,
    other_io_bps: 0,
    quality: {
      cpu: { quality: "native" },
      memory: { quality: "native" },
      io: { quality: "native" },
      other_io: { quality: "native" },
      network: { quality: "native" },
    },
  });

  assert.equal(displayProcessMetricValue(0, native.quality?.cpu, formatPercent), "0%");
  assert.equal(processBytesLabel(native, native.memory_bytes), "0 B");
  assert.equal(displayProcessMetricValue(0, native.quality?.network, formatRate), "0 B/s");
  assert.equal(
    displayProcessMetricValue(native.other_io_bps, native.quality?.other_io, formatOptionalRate),
    "0 B/s",
  );
});

test("process trust summary reports the worst activity quality", () => {
  const mixed = process({
    quality: {
      cpu: { quality: "native", source: "direct_api" },
      memory: { quality: "unavailable", source: "direct_api" },
      io: { quality: "estimated", source: "sysinfo" },
      other_io: { quality: "native", source: "direct_api" },
      network: { quality: "partial", source: "interface_aggregate" },
      threads: { quality: "native", source: "direct_api" },
      handles: { quality: "native", source: "direct_api" },
    },
  });
  assert.equal(processTrustLabel(mixed), "Partial coverage");

  delete mixed.quality?.network;
  assert.equal(processTrustLabel(mixed), "Quality not reported");
});

test("unsupported optional process fields produce partial rather than unavailable coverage", () => {
  const mixed = process({
    quality: {
      cpu: { quality: "native" },
      memory: { quality: "native" },
      io: { quality: "native" },
      other_io: { quality: "unavailable" },
      network: { quality: "unavailable" },
      threads: { quality: "native" },
      handles: { quality: "native" },
    },
  });
  assert.equal(processTrustLabel(mixed), "Partial coverage");

  const allUnavailable = process({
    quality: {
      cpu: { quality: "unavailable" },
      memory: { quality: "unavailable" },
      io: { quality: "unavailable" },
      other_io: { quality: "unavailable" },
      network: { quality: "unavailable" },
      threads: { quality: "unavailable" },
      handles: { quality: "unavailable" },
    },
  });
  assert.equal(processTrustLabel(allUnavailable), "Unavailable");

  const held = process({
    quality: {
      cpu: { quality: "native" },
      memory: { quality: "native" },
      io: { quality: "held" },
      other_io: { quality: "native" },
      network: { quality: "native" },
      threads: { quality: "native" },
      handles: { quality: "native" },
    },
  });
  assert.equal(processTrustLabel(held), "Pending coverage");
});

test("macOS physical footprint stays unavailable when rusage is partial or zero", () => {
  const partial = process({
    memory_bytes: 8 * 1024,
    private_bytes: 16 * 1024,
    quality: {
      cpu: { quality: "native" },
      memory: { quality: "partial", source: "sysinfo" },
      io: { quality: "estimated" },
      other_io: { quality: "unavailable" },
      network: { quality: "unavailable" },
      threads: { quality: "native" },
      handles: { quality: "native" },
    },
  });
  assert.equal(processBytesLabel(partial, partial.memory_bytes), "8.0 KB");
  assert.equal(processPrivateMemoryValue(partial, "macos"), "Unavailable");

  const nativeZero = process({
    private_bytes: 0,
    quality: { ...partial.quality, memory: { quality: "native", source: "direct_api" } },
  });
  assert.equal(processPrivateMemoryValue(nativeZero, "macos"), "Unavailable");
});

test("native high network activity is never described as normal", () => {
  const networked = process({
    cpu_percent: 0,
    network_received_bps: 70 * 1024 * 1024,
    network_transmitted_bps: 30 * 1024 * 1024,
    quality: {
      cpu: { quality: "native" },
      memory: { quality: "native" },
      io: { quality: "native" },
      other_io: { quality: "native" },
      network: { quality: "native" },
      threads: { quality: "native" },
      handles: { quality: "native" },
    },
  });
  const networkRate =
    (networked.network_received_bps ?? 0) + (networked.network_transmitted_bps ?? 0);

  assert.equal(processActivityLabel(networked, 0, networkRate), "Network");
  assert.equal(processTrustLabel(networked), "Native");
  assert.equal(
    processFindingLabel(networked, 0, networkRate, "Working set"),
    "High network activity relative to other workloads.",
  );
});

test("group metric values require publishable coverage", () => {
  const native = { quality: "native" as const, source: "process_aggregate" as const };
  const held = { quality: "held" as const, source: "process_aggregate" as const };
  const unavailable = {
    quality: "unavailable" as const,
    source: "process_aggregate" as const,
  };
  const unknown = { quality: "partial" as const, source: "process_aggregate" as const };
  const estimated = { quality: "estimated" as const, source: "process_aggregate" as const };

  assert.equal(displayGroupMetricValue(10, native, { available: 2, total: 2 }, String), "10");
  assert.equal(
    displayGroupMetricValue(10, unknown, { available: 1, total: 2 }, String),
    "10 · 1/2 · limited",
  );
  assert.equal(
    displayGroupMetricValue(10, estimated, { available: 2, total: 2 }, String),
    "10 · estimated",
  );
  assert.equal(displayGroupMetricValue(0, held, { available: 0, total: 2 }, String), "Pending");
  assert.equal(
    displayGroupMetricValue(0, unavailable, { available: 0, total: 2 }, String),
    "Unavailable",
  );
  assert.equal(displayGroupMetricValue(0, unknown, { available: 0, total: 2 }, String), "Limited");
  assert.equal(groupMetricCanDisplay(native, { available: 1, total: 2 }), true);
  assert.equal(groupMetricCanDisplay(unknown, { available: 0, total: 2 }), false);
});

test("group findings include native network activity and explicit network limitations", () => {
  const row = groupRow(process(), 2);
  assert.equal(row.kind, "group");
  if (row.kind !== "group") throw new Error("expected group row");

  const nativeHigh = structuredClone(row.detail);
  nativeHigh.network_bps = 2 * 1024 * 1024;
  assert.equal(groupFindingLabel(nativeHigh), "Aggregate network use is high right now.");

  const held = structuredClone(nativeHigh);
  held.network_bps = 0;
  held.quality.network = { quality: "held", source: "process_aggregate" };
  held.coverage.network = { available: 0, total: 2 };
  assert.equal(groupFindingLabel(held), "Network aggregate activity is pending for this group.");

  const unavailable = structuredClone(held);
  unavailable.quality.network = { quality: "unavailable", source: "process_aggregate" };
  assert.equal(
    groupFindingLabel(unavailable),
    "Network aggregate activity is unavailable for this group.",
  );

  const partial = structuredClone(nativeHigh);
  partial.quality.network = { quality: "partial", source: "process_aggregate" };
  partial.coverage.network = { available: 1, total: 2 };
  assert.equal(
    groupFindingLabel(partial),
    "Aggregate network use is high right now. Coverage is limited to 1 of 2 processes.",
  );

  const partialLow = structuredClone(partial);
  partialLow.network_bps = 0;
  assert.equal(
    groupFindingLabel(partialLow),
    "Network aggregate activity is limited by process telemetry coverage.",
  );

  const estimated = structuredClone(nativeHigh);
  estimated.quality.network = { quality: "estimated", source: "process_aggregate" };
  assert.equal(
    groupFindingLabel(estimated),
    "Aggregate network use is high right now. This aggregate is estimated.",
  );
});

test("group inspection actions expose exact selection state on desktop and mobile", () => {
  const desktop = readFileSync(
    new URL("../src/lib/components/processes/ProcessTable.svelte", import.meta.url),
    "utf8",
  );
  const mobile = readFileSync(
    new URL("../src/lib/components/processes/MobileProcessList.svelte", import.meta.url),
    "utf8",
  );

  assert.match(desktop, /aria-pressed=\{groupActionSelected\}/);
  assert.match(mobile, /aria-pressed=\{actionSelected\}/);
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
  assert.equal(nextSortDirection("desc"), "asc");
  assert.equal(nextSortDirection("asc"), "desc");
  assert.equal(
    sortDirectionButtonLabel("desc"),
    "Sort direction: descending. Change to ascending.",
  );
});

test("compact workload controls expose the active sort direction", () => {
  const commandBar = readFileSync(
    new URL("../src/lib/components/shell/ProcessCommandBar.svelte", import.meta.url),
    "utf8",
  );
  const app = readFileSync(new URL("../src/App.svelte", import.meta.url), "utf8");

  assert.match(commandBar, /export let sortDirection: SortDirection/);
  assert.match(commandBar, /aria-label=\{sortDirectionButtonLabel\(sortDirection\)\}/);
  assert.match(commandBar, /onclick=\{onToggleDirection\}/);
  assert.match(app, /function toggleSortDirection\(\): void/);
  assert.match(app, /onToggleDirection=\{toggleSortDirection\}/);
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
  const standalone = processRow(sample, "Processes");
  const categorized = processRow(sample, "Browsers");
  const child = processRow(sample, "Processes", true);
  const group = groupRow(sample, 4);

  assert.equal(processRowSecondaryLabel(standalone), null);
  assert.equal(processRowSecondaryLabel(categorized), "Browsers");
  assert.equal(processRowSecondaryLabel(child), "PID 99");
  assert.equal(processRowSecondaryLabel(group), "4");
});

function resourceSnapshot(contributor: string | null): RuntimeSnapshot {
  const snapshot = makeEmptySnapshot("");
  snapshot.sampled_at_ms = 1;
  snapshot.health.degraded = false;
  snapshot.system.cpu_percent = 90;
  snapshot.system.memory_total_bytes = 100;
  snapshot.system.memory_used_bytes = 20;
  snapshot.system.quality = {
    cpu: { quality: "native", source: "direct_api" },
    memory: { quality: "native", source: "sysinfo" },
    disk: { quality: "native", source: "pdh" },
    network: { quality: "native", source: "interface_aggregate" },
  };
  snapshot.process_contributors.cpu = contributor;
  snapshot.process_contributors.cpu_quality = contributor
    ? { quality: "native", source: "direct_api" }
    : undefined;
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
    io_read_total_bytes: 0,
    io_write_total_bytes: 0,
    io_read_bps: 0,
    io_write_bps: 0,
    threads: 1,
    handles: 1,
    access_state: "full",
    quality: {
      cpu: { quality: "native", source: "direct_api" },
      memory: { quality: "native", source: "direct_api" },
      io: { quality: "native", source: "direct_api" },
      network: { quality: "native", source: "direct_api" },
    },
    ...overrides,
  };
}

function processRow(
  sample = process(),
  groupCategory = "Processes",
  isGrouped = false,
): import("../src/lib/types.ts").ProcessViewRow {
  return {
    kind: "process" as const,
    detail: {
      kind: "process",
      workload_id: `process:${sample.pid}:${sample.start_time_ms}`,
      process: sample,
      io_bps: 0,
      network_bps: 0,
    },
    group_key: sample.name.toLocaleLowerCase(),
    group_label: sample.name,
    group_category: groupCategory,
    group_count: 1,
    icon_kind: "process",
    is_child: false,
    is_grouped: isGrouped,
    attention_label: "Normal",
  };
}

function groupRow(
  sample: ProcessSample,
  processCount: number,
): import("../src/lib/types.ts").ProcessViewRow {
  const coverage = { available: processCount, total: processCount };
  const quality = { quality: "native" as const, source: "process_aggregate" as const };
  const unavailable = { quality: "unavailable" as const, source: "process_aggregate" as const };
  return {
    kind: "group",
    detail: {
      kind: "group",
      workload_id: `group:${sample.name.toLocaleLowerCase()}`,
      group_key: sample.name.toLocaleLowerCase(),
      label: sample.name,
      category: "Processes",
      process_count: processCount,
      cpu_percent: 0,
      memory_bytes: 0,
      io_bps: 0,
      network_bps: 0,
      threads: processCount,
      quality: {
        cpu: quality,
        memory: quality,
        io: quality,
        other_io: unavailable,
        network: quality,
        threads: quality,
      },
      coverage: {
        cpu: coverage,
        memory: coverage,
        io: coverage,
        other_io: { available: 0, total: processCount },
        network: coverage,
        threads: coverage,
      },
    },
    icon_kind: "process",
    attention_label: "Normal",
  };
}
