import { writeFileSync } from "node:fs";
import { performance } from "node:perf_hooks";
import { fileURLToPath } from "node:url";
import { resolve } from "node:path";

export const rowCounts = [1_000, 5_000] as const;
export const strategies = [
  "current_v2",
  "per_value_metadata_v3",
  "per_family_metadata_v3",
  "shared_descriptor_catalog_v3",
] as const;

export type Strategy = (typeof strategies)[number];

const descriptors = [
  ["cpu_usage", "cpu", "percent_one_core", "direct_api"],
  ["kernel_cpu_usage", "cpu", "percent_one_core", "direct_api"],
  ["resident_memory", "memory", "bytes", "direct_api"],
  ["private_memory", "memory", "bytes", "direct_api"],
  ["read_io_rate", "io", "bytes_per_second", "pdh"],
  ["write_io_rate", "io", "bytes_per_second", "pdh"],
  ["network_receive_rate", "network", "bytes_per_second", "etw"],
  ["network_transmit_rate", "network", "bytes_per_second", "etw"],
] as const;

const familyDescriptors = [
  ["cpu", "percent_one_core", "direct_api", ["usage", "kernel_usage"]],
  ["memory", "bytes", "direct_api", ["resident", "private"]],
  ["io", "bytes_per_second", "pdh", ["read_rate", "write_rate"]],
  ["network", "bytes_per_second", "etw", ["receive_rate", "transmit_rate"]],
] as const;

function values(index: number): number[] {
  return [
    (index * 17) % 101,
    (index * 7) % 61,
    80_000_000 + index * 4_096,
    64_000_000 + index * 2_048,
    (index * 65_537) % 8_000_000,
    (index * 32_771) % 4_000_000,
    (index * 16_381) % 2_000_000,
    (index * 8_191) % 1_000_000,
  ];
}

function identity(index: number) {
  return {
    pid: String(10_000 + index),
    parent_pid: index % 7 === 0 ? null : String(9_999 + index),
    start_time_ms: 1_720_000_000_000 + index * 1_000,
    name: `workload-${index % 211}.exe`,
    exe: `C:\\Program Files\\BatCave Fixtures\\workload-${index % 211}.exe`,
    status: index % 19 === 0 ? "limited" : "running",
    threads: 2 + (index % 31),
    handles: 20 + (index % 401),
    access_state: index % 19 === 0 ? "partial" : "full",
  };
}

function currentRow(index: number) {
  const metricValues = values(index);
  return {
    ...identity(index),
    cpu_percent: metricValues[0],
    kernel_cpu_percent: metricValues[1],
    memory_bytes: metricValues[2],
    private_bytes: metricValues[3],
    disk_read_bps: metricValues[4],
    disk_write_bps: metricValues[5],
    network_received_bps: metricValues[6],
    network_transmitted_bps: metricValues[7],
  };
}

function perValueRow(index: number) {
  const metricValues = values(index);
  return {
    ...identity(index),
    metrics: descriptors.map(([semantic, family, unit, source], descriptorIndex) => ({
      semantic,
      family,
      scope: "process",
      unit,
      interval_ms: 1_000,
      source,
      quality: index % 19 === 0 ? "partial" : "native",
      observed_at_ms: 1_720_000_000_000 + index * 1_000,
      value: metricValues[descriptorIndex],
    })),
  };
}

function perFamilyRow(index: number) {
  const metricValues = values(index);
  return {
    ...identity(index),
    metric_families: familyDescriptors.map(([family, unit, source, semantics], familyIndex) => ({
      family,
      scope: "process",
      unit,
      interval_ms: 1_000,
      source,
      quality: index % 19 === 0 ? "partial" : "native",
      observed_at_ms: 1_720_000_000_000 + index * 1_000,
      values: semantics.map((semantic, valueIndex) => ({
        semantic,
        value: metricValues[familyIndex * 2 + valueIndex],
      })),
    })),
  };
}

function sharedDescriptorRow(index: number) {
  const quality = index % 19 === 0 ? 1 : 0;
  const observedAt = 1_720_000_000_000 + index * 1_000;
  return {
    ...identity(index),
    // [descriptor index, value, quality code, observed_at_ms]
    metric_values: values(index).map((value, descriptorIndex) => [
      descriptorIndex,
      value,
      quality,
      observedAt,
    ]),
  };
}

export function buildPayload(strategy: Strategy, rowCount: number) {
  const rows = Array.from({ length: rowCount }, (_, index) => {
    switch (strategy) {
      case "current_v2":
        return currentRow(index);
      case "per_value_metadata_v3":
        return perValueRow(index);
      case "per_family_metadata_v3":
        return perFamilyRow(index);
      case "shared_descriptor_catalog_v3":
        return sharedDescriptorRow(index);
    }
  });

  if (strategy === "current_v2") {
    return { contract_version: 2, event_kind: "runtime_snapshot", processes: rows };
  }

  const envelope = {
    contract_version: 3,
    event_kind: "runtime_snapshot",
    compatibility: { minimum_reader_version: 3, breaking: true },
    process_count: rowCount,
    processes: rows,
  };

  if (strategy !== "shared_descriptor_catalog_v3") return envelope;

  return {
    ...envelope,
    quality_codes: ["native", "partial", "estimated", "held", "unavailable"],
    descriptors: descriptors.map(([semantic, family, unit, source], id) => ({
      id,
      semantic,
      family,
      scope: "process",
      unit,
      interval_ms: 1_000,
      source,
    })),
  };
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

  for (let iteration = 0; iteration < 9; iteration += 1) {
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
      row_count: rowCount,
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
    reproduction:
      "npm run benchmark:dto-spike -- --write ../../docs/evidence/dto-payload-spike-20260713.json",
    environment: { node: process.version, platform: process.platform, arch: process.arch },
    timing_note:
      "Nine in-process JSON.stringify and JSON.parse samples; use sizes for architecture decisions and timings only as directional local evidence.",
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
