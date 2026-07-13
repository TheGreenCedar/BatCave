import { readFileSync, writeFileSync } from "node:fs";
import { resolve } from "node:path";
import { performance } from "node:perf_hooks";
import { fileURLToPath } from "node:url";

export const rowCounts = [1_000, 5_000] as const;
export const strategies = [
  "current_v2",
  "per_value_metadata_v3",
  "per_family_metadata_v3",
  "shared_descriptor_catalog_v3",
] as const;

export type Strategy = (typeof strategies)[number];

type MetricSpec<T> = {
  semantic: string;
  family: string;
  unit: string;
  source: string;
  value: (subject: T) => number | null;
};

type ProcessShape = ReturnType<typeof currentProcess>;
type GroupShape = ReturnType<typeof currentGroup>;

const canonicalV2 = JSON.parse(
  readFileSync(new URL("./fixtures/runtime-snapshot.v2.json", import.meta.url), "utf8"),
);

const processMetricSpecs: Array<MetricSpec<ProcessShape>> = [
  metric("cpu_usage", "cpu", "percent_one_core", "direct_api", (row) => row.cpu_percent),
  metric(
    "kernel_cpu_usage",
    "cpu",
    "percent_one_core",
    "direct_api",
    (row) => row.kernel_cpu_percent,
  ),
  metric("resident_memory", "memory", "bytes", "direct_api", (row) => row.memory_bytes),
  metric("private_memory", "memory", "bytes", "direct_api", (row) => row.private_bytes),
  metric("virtual_memory", "memory", "bytes", "direct_api", (row) => row.virtual_memory_bytes),
  metric("read_io_total", "io", "bytes", "direct_api", (row) => row.disk_read_total_bytes),
  metric("write_io_total", "io", "bytes", "direct_api", (row) => row.disk_write_total_bytes),
  metric("other_io_total", "io", "bytes", "direct_api", (row) => row.other_io_total_bytes),
  metric("read_io_rate", "io", "bytes_per_second", "direct_api", (row) => row.disk_read_bps),
  metric("write_io_rate", "io", "bytes_per_second", "direct_api", (row) => row.disk_write_bps),
  metric("other_io_rate", "io", "bytes_per_second", "direct_api", (row) => row.other_io_bps),
  metric(
    "network_receive_rate",
    "network",
    "bytes_per_second",
    "etw",
    (row) => row.network_received_bps,
  ),
  metric(
    "network_transmit_rate",
    "network",
    "bytes_per_second",
    "etw",
    (row) => row.network_transmitted_bps,
  ),
  metric("thread_count", "process", "count", "direct_api", (row) => row.threads),
  metric("handle_count", "process", "count", "direct_api", (row) => row.handles),
];

const groupMetricSpecs: Array<MetricSpec<GroupShape>> = [
  metric("cpu_usage", "cpu", "percent_one_core", "runtime", (group) => group.cpu_percent),
  metric("resident_memory", "memory", "bytes", "runtime", (group) => group.memory_bytes),
  metric("io_rate", "io", "bytes_per_second", "runtime", (group) => group.io_bps),
  metric("network_rate", "network", "bytes_per_second", "runtime", (group) => group.network_bps),
  metric("thread_count", "process", "count", "runtime", (group) => group.threads),
];

const systemMetricSpecs: Array<MetricSpec<Record<string, unknown>>> = [
  metric("cpu_usage", "cpu", "percent_system", "direct_api", numberField("cpu_percent")),
  metric(
    "kernel_cpu_usage",
    "cpu",
    "percent_system",
    "direct_api",
    numberField("kernel_cpu_percent"),
  ),
  metric("memory_used", "memory", "bytes", "direct_api", numberField("memory_used_bytes")),
  metric("memory_capacity", "memory", "bytes", "direct_api", numberField("memory_total_bytes")),
  metric(
    "memory_available",
    "memory",
    "bytes",
    "direct_api",
    numberField("memory_available_bytes"),
  ),
  metric(
    "process_working_set_memory",
    "memory_accounting",
    "bytes",
    "runtime",
    nestedNumberField("memory_accounting", "process_working_set_bytes"),
  ),
  metric(
    "process_private_memory",
    "memory_accounting",
    "bytes",
    "runtime",
    nestedNumberField("memory_accounting", "process_private_bytes"),
  ),
  metric(
    "denied_process_count",
    "memory_accounting",
    "count",
    "runtime",
    nestedNumberField("memory_accounting", "denied_process_count"),
  ),
  metric(
    "partial_process_count",
    "memory_accounting",
    "count",
    "runtime",
    nestedNumberField("memory_accounting", "partial_process_count"),
  ),
  metric(
    "unattributed_memory",
    "memory_accounting",
    "bytes",
    "runtime",
    nestedNumberField("memory_accounting", "unattributed_bytes"),
  ),
  metric(
    "commit_used",
    "memory_accounting",
    "bytes",
    "direct_api",
    nestedNumberField("memory_accounting", "commit_used_bytes"),
  ),
  metric(
    "commit_limit",
    "memory_accounting",
    "bytes",
    "direct_api",
    nestedNumberField("memory_accounting", "commit_limit_bytes"),
  ),
  metric(
    "system_cache",
    "memory_accounting",
    "bytes",
    "direct_api",
    nestedNumberField("memory_accounting", "system_cache_bytes"),
  ),
  metric(
    "kernel_memory",
    "memory_accounting",
    "bytes",
    "direct_api",
    nestedNumberField("memory_accounting", "kernel_total_bytes"),
  ),
  metric(
    "kernel_paged_pool",
    "memory_accounting",
    "bytes",
    "direct_api",
    nestedNumberField("memory_accounting", "kernel_paged_pool_bytes"),
  ),
  metric(
    "kernel_nonpaged_pool",
    "memory_accounting",
    "bytes",
    "direct_api",
    nestedNumberField("memory_accounting", "kernel_nonpaged_pool_bytes"),
  ),
  metric("process_count", "process", "count", "runtime", numberField("process_count")),
  metric("read_io_total", "io", "bytes", "pdh", numberField("disk_read_total_bytes")),
  metric("write_io_total", "io", "bytes", "pdh", numberField("disk_write_total_bytes")),
  metric("read_io_rate", "io", "bytes_per_second", "pdh", numberField("disk_read_bps")),
  metric("write_io_rate", "io", "bytes_per_second", "pdh", numberField("disk_write_bps")),
  metric(
    "network_receive_total",
    "network",
    "bytes",
    "direct_api",
    numberField("network_received_total_bytes"),
  ),
  metric(
    "network_transmit_total",
    "network",
    "bytes",
    "direct_api",
    numberField("network_transmitted_total_bytes"),
  ),
  metric(
    "network_receive_rate",
    "network",
    "bytes_per_second",
    "direct_api",
    numberField("network_received_bps"),
  ),
  metric(
    "network_transmit_rate",
    "network",
    "bytes_per_second",
    "direct_api",
    numberField("network_transmitted_bps"),
  ),
];

function metric<T>(
  semantic: string,
  family: string,
  unit: string,
  source: string,
  value: (subject: T) => number | null,
): MetricSpec<T> {
  return { semantic, family, unit, source, value };
}

function numberField(field: string): (subject: Record<string, unknown>) => number | null {
  return (subject) => (typeof subject[field] === "number" ? subject[field] : null);
}

function nestedNumberField(
  parent: string,
  field: string,
): (subject: Record<string, unknown>) => number | null {
  return (subject) => {
    const nested = subject[parent];
    if (!nested || typeof nested !== "object") return null;
    const value = (nested as Record<string, unknown>)[field];
    return typeof value === "number" ? value : null;
  };
}

function quality(source: string, index: number, limited: boolean) {
  return {
    quality: limited ? "partial" : "native",
    source,
    updated_at_ms: 1_720_000_000_000 + index * 1_000,
    age_ms: 0,
    ...(limited ? { message: "Some protected fields could not be read." } : {}),
  };
}

function currentProcess(index: number) {
  const limited = index % 19 === 0;
  const observedAt = 1_720_000_000_000 + index * 1_000;
  return {
    pid: String(10_000 + index),
    parent_pid: index % 7 === 0 ? null : String(9_999 + index),
    start_time_ms: 1_700_000_000_000 + index * 1_000,
    name: `workload-${index % 211}.exe`,
    exe: `C:\\Program Files\\BatCave Fixtures\\workload-${index % 211}.exe`,
    status: limited ? "limited" : "running",
    cpu_percent: (index * 17) % 101,
    kernel_cpu_percent: (index * 7) % 61,
    memory_bytes: 80_000_000 + index * 4_096,
    private_bytes: 64_000_000 + index * 2_048,
    virtual_memory_bytes: 160_000_000 + index * 8_192,
    disk_read_total_bytes: index * 4_000_003,
    disk_write_total_bytes: index * 2_000_003,
    other_io_total_bytes: index * 1_000_003,
    disk_read_bps: (index * 65_537) % 8_000_000,
    disk_write_bps: (index * 32_771) % 4_000_000,
    other_io_bps: (index * 24_593) % 3_000_000,
    network_received_bps: (index * 16_381) % 2_000_000,
    network_transmitted_bps: (index * 8_191) % 1_000_000,
    threads: 2 + (index % 31),
    handles: 20 + (index % 401),
    access_state: limited ? "partial" : "full",
    quality: {
      cpu: quality("direct_api", index, false),
      memory: quality("direct_api", index, limited),
      disk: quality("direct_api", index, limited),
      other_io: quality("direct_api", index, limited),
      network: quality("etw", index, limited),
      threads: quality("direct_api", index, false),
      handles: quality("direct_api", index, limited),
    },
    observed_at_ms: observedAt,
  };
}

function currentGroup(members: ProcessShape[], groupIndex: number) {
  return {
    stable_id: `group:fixture-${groupIndex}`,
    display_name: `Fixture group ${groupIndex}`,
    member_ids: members.map(stableProcessId),
    included_processes: members.filter((member) => member.access_state === "full").length,
    total_processes: members.length,
    cpu_percent: sum(members, (member) => member.cpu_percent),
    memory_bytes: sum(members, (member) => member.memory_bytes),
    io_bps: sum(members, (member) => member.disk_read_bps + member.disk_write_bps),
    network_bps: sum(
      members,
      (member) => member.network_received_bps + member.network_transmitted_bps,
    ),
    threads: sum(members, (member) => member.threads),
    observed_at_ms: Math.max(...members.map((member) => member.observed_at_ms)),
  };
}

function stableProcessId(process: ProcessShape): string {
  return `process:${process.pid}:${process.start_time_ms}`;
}

function sum<T>(values: T[], select: (value: T) => number): number {
  return values.reduce((total, value) => total + select(value), 0);
}

function workloadLayout(processes: ProcessShape[]) {
  const groupedProcessCount = Math.floor((processes.length * 0.2) / 10) * 10;
  const groups = Array.from({ length: groupedProcessCount / 10 }, (_, groupIndex) =>
    currentGroup(processes.slice(groupIndex * 10, groupIndex * 10 + 10), groupIndex),
  );
  const singletons = processes.slice(groupedProcessCount);
  return { groups, singletons };
}

function currentViewRows(processes: ProcessShape[]) {
  const { groups, singletons } = workloadLayout(processes);
  const rows = groups.map((group, groupIndex) => {
    const representative = processes[groupIndex * 10];
    return {
      kind: "group",
      representative,
      group_key: group.stable_id,
      group_label: group.display_name,
      group_category: "fixture",
      group_count: group.total_processes,
      icon_kind: "group",
      is_child: false,
      is_grouped: true,
      attention_label: "Grouped workload",
      cpu_percent: group.cpu_percent,
      memory_bytes: group.memory_bytes,
      io_bps: group.io_bps,
      network_bps: group.network_bps,
      threads: group.threads,
    };
  });

  return rows.concat(
    singletons.map((process) => ({
      kind: "process",
      process,
      group_count: 1,
      icon_kind: "process",
      is_child: false,
      is_grouped: false,
      attention_label: process.access_state === "full" ? "" : "Limited access",
      cpu_percent: process.cpu_percent,
      memory_bytes: process.memory_bytes,
      io_bps: process.disk_read_bps + process.disk_write_bps,
      network_bps: process.network_received_bps + process.network_transmitted_bps,
      threads: process.threads,
    })),
  );
}

function currentPayload(rowCount: number) {
  const processes = Array.from({ length: rowCount }, (_, index) => currentProcess(index));
  const payload = structuredClone(canonicalV2);
  payload.processes = processes;
  payload.process_view_rows = currentViewRows(processes);
  payload.total_process_count = rowCount;
  payload.system.process_count = rowCount;
  payload.system.memory_accounting.process_working_set_bytes = sum(
    processes,
    (process) => process.memory_bytes,
  );
  payload.system.memory_accounting.process_private_bytes = sum(
    processes,
    (process) => process.private_bytes,
  );
  payload.system.memory_accounting.partial_process_count = processes.filter(
    (process) => process.access_state === "partial",
  ).length;
  payload.system.quality = {
    cpu: quality("direct_api", 0, false),
    memory: quality("direct_api", 0, false),
    disk: quality("pdh", 0, false),
    network: quality("direct_api", 0, false),
    swap: {
      quality: "unavailable",
      source: "direct_api",
      updated_at_ms: 1_720_000_000_000,
      age_ms: 0,
      message: "Windows reports commit instead of swap.",
    },
  };
  return payload;
}

type CandidateStrategy = Exclude<Strategy, "current_v2">;

function descriptorCatalog() {
  let id = 0;
  const entries: Array<Record<string, unknown>> = [];
  for (const [scope, specs] of [
    ["system", systemMetricSpecs],
    ["process", processMetricSpecs],
    ["group", groupMetricSpecs],
  ] as const) {
    for (const spec of specs) {
      entries.push({
        id,
        semantic: spec.semantic,
        scope,
        unit: spec.unit,
        interval_ms: 1_000,
        source: spec.source,
      });
      id += 1;
    }
  }
  return entries;
}

function descriptorOffset(scope: "system" | "process" | "group"): number {
  if (scope === "system") return 0;
  if (scope === "process") return systemMetricSpecs.length;
  return systemMetricSpecs.length + processMetricSpecs.length;
}

function metricShape<T>(
  strategy: CandidateStrategy,
  scope: "system" | "process" | "group",
  specs: Array<MetricSpec<T>>,
  subject: T,
  observedAt: number,
  limitationIndex: number | null,
) {
  if (strategy === "shared_descriptor_catalog_v3") {
    const qualityCode = limitationIndex === null ? 0 : 3;
    return {
      metrics: specs.map((spec, index) => [
        descriptorOffset(scope) + index,
        spec.value(subject),
        qualityCode,
        observedAt,
        limitationIndex,
      ]),
    };
  }

  if (strategy === "per_value_metadata_v3") {
    return {
      metrics: specs.map((spec) => ({
        semantic: spec.semantic,
        family: spec.family,
        scope,
        unit: spec.unit,
        interval_ms: 1_000,
        source: spec.source,
        quality: limitationIndex === null ? "native" : "partial",
        observed_at_ms: observedAt,
        ...(limitationIndex === null
          ? {}
          : { limitation: "Some protected fields could not be read." }),
        value: spec.value(subject),
      })),
    };
  }

  const families = new Map<string, Array<MetricSpec<T>>>();
  for (const spec of specs) {
    const key = `${spec.family}:${spec.unit}:${spec.source}`;
    families.set(key, [...(families.get(key) ?? []), spec]);
  }
  return {
    metric_families: [...families.values()].map((familySpecs) => ({
      family: familySpecs[0].family,
      scope,
      unit: familySpecs[0].unit,
      interval_ms: 1_000,
      source: familySpecs[0].source,
      quality: limitationIndex === null ? "native" : "partial",
      observed_at_ms: observedAt,
      ...(limitationIndex === null
        ? {}
        : { limitation: "Some protected fields could not be read." }),
      values: familySpecs.map((spec) => ({
        semantic: spec.semantic,
        value: spec.value(subject),
      })),
    })),
  };
}

function processDetail(strategy: CandidateStrategy, process: ProcessShape) {
  const limitationIndex = process.access_state === "full" ? null : 0;
  return {
    kind: "process",
    detail: {
      stable_id: stableProcessId(process),
      pid: process.pid,
      parent_id: process.parent_pid,
      start_time_ms: process.start_time_ms,
      display_name: process.name,
      executable: process.exe,
      status: process.status,
      access_state: process.access_state,
      ...metricShape(
        strategy,
        "process",
        processMetricSpecs,
        process,
        process.observed_at_ms,
        limitationIndex,
      ),
    },
  };
}

function groupDetail(strategy: CandidateStrategy, group: GroupShape) {
  const limited = group.included_processes < group.total_processes;
  return {
    kind: "group",
    detail: {
      stable_id: group.stable_id,
      display_name: group.display_name,
      member_ids: group.member_ids,
      coverage: {
        included_processes: group.included_processes,
        total_processes: group.total_processes,
        ...(limited
          ? strategy === "shared_descriptor_catalog_v3"
            ? { limitation_indexes: [0] }
            : { limitations: ["Some protected fields could not be read."] }
          : {}),
      },
      ...metricShape(
        strategy,
        "group",
        groupMetricSpecs,
        group,
        group.observed_at_ms,
        limited ? 0 : null,
      ),
    },
  };
}

function candidatePayload(strategy: CandidateStrategy, rowCount: number) {
  const current = currentPayload(rowCount);
  const processes = current.processes as ProcessShape[];
  const { groups } = workloadLayout(processes);
  const {
    processes: _processes,
    process_view_rows: _processViewRows,
    total_process_count: _totalProcessCount,
    system,
    ...runtimeContext
  } = current;
  const observedAt = current.sampled_at_ms as number;
  const shared = strategy === "shared_descriptor_catalog_v3";

  return {
    ...runtimeContext,
    protocol_version: 3,
    compatibility: { minimum_reader_version: 3, breaking: true },
    process_row_count: rowCount,
    group_row_count: groups.length,
    ...(shared
      ? {
          descriptors: descriptorCatalog(),
          quality_codes: ["native", "estimated", "held", "partial", "unavailable"],
          limitations: [
            "Some protected fields could not be read.",
            "Windows reports commit instead of swap.",
          ],
        }
      : {}),
    system: {
      stable_id: "system:local",
      ...(shared
        ? { limitation_indexes: [1] }
        : { limitations: ["Windows reports commit instead of swap."] }),
      ...metricShape(
        strategy,
        "system",
        systemMetricSpecs,
        system as Record<string, unknown>,
        observedAt,
        null,
      ),
    },
    workloads: [
      ...processes.map((process) => processDetail(strategy, process)),
      ...groups.map((group) => groupDetail(strategy, group)),
    ],
  };
}

export function buildPayload(strategy: Strategy, rowCount: number) {
  return strategy === "current_v2"
    ? currentPayload(rowCount)
    : candidatePayload(strategy, rowCount);
}

function percentile(samples: number[], fraction: number): number {
  const ordered = [...samples].sort((left, right) => left - right);
  return ordered[Math.ceil(ordered.length * fraction) - 1];
}

function timedPayload(strategy: Strategy, rowCount: number) {
  const payload = buildPayload(strategy, rowCount);
  const encoded = JSON.stringify(payload);
  const encodeSamples: number[] = [];
  const parseSamples: number[] = [];

  for (let iteration = 0; iteration < 7; iteration += 1) {
    const encodeStart = performance.now();
    JSON.stringify(payload);
    encodeSamples.push(performance.now() - encodeStart);

    const parseStart = performance.now();
    JSON.parse(encoded);
    parseSamples.push(performance.now() - parseStart);
  }

  return {
    bytes: Buffer.byteLength(encoded),
    encode_ms: {
      median: Number(percentile(encodeSamples, 0.5).toFixed(3)),
      p95: Number(percentile(encodeSamples, 0.95).toFixed(3)),
    },
    parse_ms: {
      median: Number(percentile(parseSamples, 0.5).toFixed(3)),
      p95: Number(percentile(parseSamples, 0.95).toFixed(3)),
    },
  };
}

export function measurePayloads() {
  return rowCounts.map((rowCount) => {
    const measured = Object.fromEntries(
      strategies.map((strategy) => [strategy, timedPayload(strategy, rowCount)]),
    );
    const currentBytes = measured.current_v2.bytes;

    return {
      process_row_count: rowCount,
      grouped_process_count: Math.floor((rowCount * 0.2) / 10) * 10,
      group_row_count: Math.floor((rowCount * 0.2) / 10),
      strategies: Object.fromEntries(
        strategies.map((strategy) => [
          strategy,
          {
            ...measured[strategy],
            relative_to_current_percent: Number(
              (((measured[strategy].bytes - currentBytes) / currentBytes) * 100).toFixed(1),
            ),
          },
        ]),
      ),
    };
  });
}

export function buildEvidence() {
  return {
    spike: "BatCave issue #66",
    contract_versions: { baseline: 2, candidate: 3 },
    baseline_fixture: "scripts/fixtures/runtime-snapshot.v2.json",
    reproduction:
      "npm run benchmark:dto-spike -- --write ../../docs/evidence/dto-payload-spike-20260713.json",
    environment: { node: process.version, platform: process.platform, arch: process.arch },
    workload_note:
      "The v2 baseline preserves the checked RuntimeSnapshot envelope, full ProcessSample fields and quality, and duplicated ProcessViewRow payloads. Twenty percent of processes form ten-member groups; the remainder are singleton view rows. V3 carries each process once plus explicit group and system detail.",
    timing_note:
      "Seven in-process JSON.stringify and JSON.parse samples; use sizes for architecture decisions and timings only as directional local evidence.",
    results: measurePayloads(),
  };
}

if (process.argv[1] && fileURLToPath(import.meta.url) === resolve(process.argv[1])) {
  const evidence = `${JSON.stringify(buildEvidence(), null, 2)}\n`;
  const writeIndex = process.argv.indexOf("--write");

  if (writeIndex >= 0) {
    const outputPath = process.argv[writeIndex + 1];
    if (!outputPath) throw new Error("--write requires an output path");
    writeFileSync(resolve(outputPath), evidence);
  } else {
    process.stdout.write(evidence);
  }
}
