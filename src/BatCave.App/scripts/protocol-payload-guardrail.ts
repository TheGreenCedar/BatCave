import { readFileSync, writeFileSync } from "node:fs";
import { performance } from "node:perf_hooks";
import { resolve } from "node:path";
import { fileURLToPath } from "node:url";

export const processRowCount = 5_000;
export const groupedProcessCount = 1_000;
export const groupSize = 10;
export const groupRowCount = groupedProcessCount / groupSize;
export const minimumSizeReduction = 0.5;
export const maximumTimingRatio = 3;

const productionFixtureUrl = new URL("./fixtures/runtime-snapshot.v3.json", import.meta.url);
const canonicalV2 = JSON.parse(
  readFileSync(new URL("./fixtures/runtime-snapshot.v2.json", import.meta.url), "utf8"),
) as JsonObject;

type JsonObject = Record<string, any>;
type TimingSamples = { stringify: number[]; parse: number[] };
type PayloadMeasurement = {
  bytes: number;
  stringify_ms: { median: number; p95: number };
  parse_ms: { median: number; p95: number };
};

export type ProductionFixture = {
  fixturePath: string;
  envelope: JsonObject;
};

export function loadProductionFixture(fixturePath?: string): ProductionFixture {
  const resolvedPath = fixturePath ? resolve(fixturePath) : fileURLToPath(productionFixtureUrl);
  let envelope: unknown;

  try {
    envelope = JSON.parse(readFileSync(resolvedPath, "utf8"));
  } catch (error) {
    const reason = error instanceof Error ? error.message : String(error);
    throw new Error(
      `Unable to read the production v3 protocol fixture at ${resolvedPath}: ${reason}. ` +
        "Pass --fixture <path> to a checked ProtocolEnvelope fixture if it lives elsewhere.",
    );
  }

  assertProductionFixture(envelope, resolvedPath);
  return { fixturePath: resolvedPath, envelope };
}

export function assertProductionFixture(
  value: unknown,
  source = "v3 fixture",
): asserts value is JsonObject {
  const envelope = object(value, source);
  assert(envelope.protocol_version === 3, `${source}: protocol_version must be 3`);
  const compatibility = object(envelope.compatibility, `${source}.compatibility`);
  assert(compatibility.minimum_reader_version === 3, `${source}: minimum_reader_version must be 3`);

  const event = object(envelope.event, `${source}.event`);
  assert(event.kind === "runtime_snapshot", `${source}: event.kind must be runtime_snapshot`);
  const payload = object(event.payload, `${source}.event.payload`);
  const descriptors = array(payload.descriptors, `${source}: descriptors`);
  const qualityCodes = array(payload.quality_codes, `${source}: quality_codes`);
  const limitations = array(payload.limitations, `${source}: limitations`);
  const workloads = array(payload.workloads, `${source}: workloads`);
  const system = object(payload.system, `${source}: system`);

  assert(descriptors.length > 0, `${source}: descriptors must not be empty`);
  assert(qualityCodes.length > 0, `${source}: quality_codes must not be empty`);
  assert(
    workloads.some((row) => object(row, `${source}: workload`).kind === "process"),
    `${source}: workloads must contain a representative process row`,
  );
  assert(
    workloads.some((row) => object(row, `${source}: workload`).kind === "group"),
    `${source}: workloads must contain a representative group row`,
  );

  const descriptorIds = new Set<number>();
  const descriptorMeanings = new Set<string>();
  for (const descriptorValue of descriptors) {
    const descriptor = object(descriptorValue, `${source}: descriptor`);
    assert(Number.isInteger(descriptor.id), `${source}: descriptor.id must be an integer`);
    assert(typeof descriptor.semantic === "string", `${source}: descriptor.semantic is required`);
    assert(typeof descriptor.scope === "string", `${source}: descriptor.scope is required`);
    assert(typeof descriptor.unit === "string", `${source}: descriptor.unit is required`);
    assert("interval_ms" in descriptor, `${source}: descriptor.interval_ms is required`);
    assert(typeof descriptor.source === "string", `${source}: descriptor.source is required`);
    assert(!descriptorIds.has(descriptor.id), `${source}: descriptor ids must be unique`);
    descriptorIds.add(descriptor.id);

    const meaning = [
      descriptor.semantic,
      descriptor.scope,
      descriptor.unit,
      descriptor.interval_ms,
      descriptor.source,
    ].join(":");
    assert(!descriptorMeanings.has(meaning), `${source}: descriptor meaning is duplicated`);
    descriptorMeanings.add(meaning);
  }

  const validateMetrics = (owner: JsonObject, label: string) => {
    for (const observationValue of array(owner.metrics, `${source}: ${label}.metrics`)) {
      const observation = array(observationValue, `${source}: ${label} observation`);
      assert(observation.length === 5, `${source}: observations must use the five-value tuple`);
      assert(
        Number.isInteger(observation[0]) && descriptorIds.has(observation[0]),
        `${source}: observation references an unknown descriptor`,
      );
      assert(
        Number.isInteger(observation[2]) &&
          observation[2] >= 0 &&
          observation[2] < qualityCodes.length,
        `${source}: observation references an unknown quality code`,
      );
      assert(
        observation[4] === null ||
          (Number.isInteger(observation[4]) &&
            observation[4] >= 0 &&
            observation[4] < limitations.length),
        `${source}: observation references an unknown limitation`,
      );
    }
  };

  validateMetrics(system, "system");
  for (const workloadValue of workloads) {
    const workload = object(workloadValue, `${source}: workload`);
    validateMetrics(object(workload.detail, `${source}: workload.detail`), String(workload.kind));
  }
}

export function buildProductionV3(fixture: JsonObject, rowCount = processRowCount): JsonObject {
  assert(
    rowCount >= groupSize * 5,
    "rowCount must be large enough to include representative groups",
  );
  assertProductionFixture(fixture);
  const envelope = structuredClone(fixture);
  const payload = envelope.event.payload as JsonObject;
  const workloads = payload.workloads as JsonObject[];
  const processTemplates = workloads.filter((row) => row.kind === "process");
  const groupTemplates = workloads.filter((row) => row.kind === "group");
  const targetGroupedProcesses = Math.floor((rowCount * 0.2) / groupSize) * groupSize;
  const targetGroupCount = targetGroupedProcesses / groupSize;

  const processes = Array.from({ length: rowCount }, (_, index) => {
    const template = structuredClone(processTemplates[index % processTemplates.length]);
    const detail = template.detail as JsonObject;
    const pid = String(10_000 + index);
    const startTime = 1_700_000_000_000 + index * 1_000;
    const stableId = `process:${pid}:${startTime}`;
    const grouped = index < targetGroupedProcesses;
    const groupIndex = Math.floor(index / groupSize);

    detail.stable_id = stableId;
    detail.identity_stability = "stable";
    detail.pid = pid;
    detail.parent_pid = null;
    detail.parent_process_id = null;
    detail.start_time_ms = startTime;
    detail.display_name = `workload-${index % 211}.exe`;
    detail.executable = `C:\\Program Files\\BatCave Fixtures\\workload-${index % 211}.exe`;

    if (detail.presentation && typeof detail.presentation === "object") {
      const presentation = detail.presentation as JsonObject;
      presentation.group_id = grouped ? `group:fixture-${groupIndex}` : null;
      presentation.group_key = grouped ? `fixture-${groupIndex}` : stableId;
      presentation.group_label = grouped ? `Fixture group ${groupIndex}` : detail.display_name;
      presentation.group_category = grouped ? "fixture" : "process";
      presentation.group_count = grouped ? groupSize : 1;
      presentation.is_child = grouped;
      presentation.is_grouped = grouped;
    }

    return template;
  });

  const processIds = processes.map((row) => row.detail.stable_id as string);
  const groups = Array.from({ length: targetGroupCount }, (_, index) => {
    const template = structuredClone(groupTemplates[index % groupTemplates.length]);
    const detail = template.detail as JsonObject;
    detail.stable_id = `group:fixture-${index}`;
    detail.group_key = `fixture-${index}`;
    detail.label = `Fixture group ${index}`;
    detail.member_ids = processIds.slice(index * groupSize, (index + 1) * groupSize);
    if (Array.isArray(detail.coverage)) {
      for (const coverageValue of detail.coverage) {
        const coverage = object(coverageValue, "group coverage");
        coverage.available_contributors = groupSize;
        coverage.total_contributors = groupSize;
      }
    }
    return template;
  });

  payload.workloads = [...processes, ...groups];
  payload.total_process_count = rowCount;
  payload.visible_process_count = rowCount;
  if (payload.settings?.query) payload.settings.query.limit = rowCount;
  updateSystemProcessCount(payload, rowCount);
  updateContributorCoverage(payload, processIds, rowCount);
  return envelope;
}

function updateSystemProcessCount(payload: JsonObject, rowCount: number) {
  const descriptor = (payload.descriptors as JsonObject[]).find(
    (candidate) => candidate.scope === "system" && candidate.semantic === "process_count",
  );
  if (!descriptor) return;

  const observation = (payload.system.metrics as any[][]).find(
    (candidate) => candidate[0] === descriptor.id,
  );
  if (observation) observation[1] = rowCount;
}

function updateContributorCoverage(payload: JsonObject, processIds: string[], rowCount: number) {
  if (!Array.isArray(payload.contributors)) return;
  for (const contributorValue of payload.contributors) {
    const contributor = object(contributorValue, "contributor");
    contributor.total_contributors = rowCount;
    contributor.available_contributors = rowCount;
    if (contributor.process_id !== null) contributor.process_id = processIds[0];
  }
}

export function buildEquivalentV2(rowCount = processRowCount): JsonObject {
  const processes = Array.from({ length: rowCount }, (_, index) => currentProcess(index));
  const payload = structuredClone(canonicalV2);
  payload.processes = processes;
  payload.process_view_rows = currentViewRows(processes);
  payload.total_process_count = rowCount;
  payload.settings.query.limit = rowCount;
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

type CurrentProcess = ReturnType<typeof currentProcess>;

function currentProcess(index: number) {
  const limited = index % 19 === 0;
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

function currentViewRows(processes: CurrentProcess[]) {
  const groups = Array.from(
    { length: Math.floor((processes.length * 0.2) / groupSize) },
    (_, index) => {
      const members = processes.slice(index * groupSize, (index + 1) * groupSize);
      const representative = members[0];
      return {
        kind: "group",
        representative,
        group_key: `group:fixture-${index}`,
        group_label: `Fixture group ${index}`,
        group_category: "fixture",
        group_count: members.length,
        icon_kind: "group",
        is_child: false,
        is_grouped: true,
        attention_label: "Grouped workload",
        cpu_percent: sum(members, (member) => member.cpu_percent),
        memory_bytes: sum(members, (member) => member.memory_bytes),
        io_bps: sum(members, (member) => member.disk_read_bps + member.disk_write_bps),
        network_bps: sum(
          members,
          (member) => member.network_received_bps + member.network_transmitted_bps,
        ),
        threads: sum(members, (member) => member.threads),
      };
    },
  );
  const singletons = processes.slice(groups.length * groupSize).map((process) => ({
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
  }));
  return [...groups, ...singletons];
}

function sum<T>(values: T[], select: (value: T) => number): number {
  return values.reduce((total, value) => total + select(value), 0);
}

export function measurePayloadPair(baseline: JsonObject, candidate: JsonObject, sampleCount = 21) {
  assert(sampleCount >= 3, "sampleCount must be at least 3");
  const payloads = { baseline_v2: baseline, production_v3: candidate };
  const encoded = {
    baseline_v2: JSON.stringify(baseline),
    production_v3: JSON.stringify(candidate),
  };
  const samples: Record<keyof typeof payloads, TimingSamples> = {
    baseline_v2: { stringify: [], parse: [] },
    production_v3: { stringify: [], parse: [] },
  };

  for (let iteration = 0; iteration < 3; iteration += 1) {
    JSON.stringify(baseline);
    JSON.stringify(candidate);
    JSON.parse(encoded.baseline_v2);
    JSON.parse(encoded.production_v3);
  }

  for (let iteration = 0; iteration < sampleCount; iteration += 1) {
    const order =
      iteration % 2 === 0
        ? (["baseline_v2", "production_v3"] as const)
        : (["production_v3", "baseline_v2"] as const);
    for (const name of order) {
      const stringifyStart = performance.now();
      JSON.stringify(payloads[name]);
      samples[name].stringify.push(performance.now() - stringifyStart);

      const parseStart = performance.now();
      JSON.parse(encoded[name]);
      samples[name].parse.push(performance.now() - parseStart);
    }
  }

  return {
    baseline_v2: measurement(encoded.baseline_v2, samples.baseline_v2),
    production_v3: measurement(encoded.production_v3, samples.production_v3),
  };
}

function measurement(encoded: string, samples: TimingSamples): PayloadMeasurement {
  return {
    bytes: Buffer.byteLength(encoded),
    stringify_ms: summarize(samples.stringify),
    parse_ms: summarize(samples.parse),
  };
}

function summarize(samples: number[]) {
  return {
    median: roundedPercentile(samples, 0.5),
    p95: roundedPercentile(samples, 0.95),
  };
}

function roundedPercentile(samples: number[], fraction: number): number {
  const ordered = [...samples].sort((left, right) => left - right);
  const value = ordered[Math.ceil(ordered.length * fraction) - 1];
  return Number(value.toFixed(3));
}

export function buildGuardrailEvidence(fixturePath?: string, sampleCount = 21) {
  const fixture = loadProductionFixture(fixturePath);
  const baseline = buildEquivalentV2();
  const candidate = buildProductionV3(fixture.envelope);
  const measured = measurePayloadPair(baseline, candidate, sampleCount);
  const sizeReduction = 1 - measured.production_v3.bytes / measured.baseline_v2.bytes;
  const stringifyRatio =
    measured.production_v3.stringify_ms.p95 / measured.baseline_v2.stringify_ms.p95;
  const parseRatio = measured.production_v3.parse_ms.p95 / measured.baseline_v2.parse_ms.p95;

  return {
    guardrail: "BatCave issue #67 production protocol payload",
    contract_versions: { baseline: 2, production: 3 },
    fixtures: {
      baseline_v2: "scripts/fixtures/runtime-snapshot.v2.json",
      production_v3: fixture.fixturePath,
    },
    workload: {
      process_rows: processRowCount,
      grouped_processes: groupedProcessCount,
      group_rows: groupRowCount,
      group_size: groupSize,
    },
    budgets: {
      minimum_size_reduction_percent: minimumSizeReduction * 100,
      maximum_stringify_p95_ratio: maximumTimingRatio,
      maximum_parse_p95_ratio: maximumTimingRatio,
    },
    measurement: {
      sample_count: sampleCount,
      note: "Same-process, interleaved JSON.stringify and JSON.parse samples after three warmups.",
      ...measured,
    },
    comparison: {
      size_reduction_percent: Number((sizeReduction * 100).toFixed(1)),
      stringify_p95_ratio: Number(stringifyRatio.toFixed(2)),
      parse_p95_ratio: Number(parseRatio.toFixed(2)),
    },
  };
}

export function assertGuardrail(evidence: ReturnType<typeof buildGuardrailEvidence>) {
  const { comparison, budgets } = evidence;
  assert(
    comparison.size_reduction_percent >= budgets.minimum_size_reduction_percent,
    `production v3 is only ${comparison.size_reduction_percent}% smaller than v2; budget is ` +
      `${budgets.minimum_size_reduction_percent}%`,
  );
  assert(
    comparison.stringify_p95_ratio <= budgets.maximum_stringify_p95_ratio,
    `production v3 stringify p95 is ${comparison.stringify_p95_ratio}x v2; budget is ` +
      `${budgets.maximum_stringify_p95_ratio}x`,
  );
  assert(
    comparison.parse_p95_ratio <= budgets.maximum_parse_p95_ratio,
    `production v3 parse p95 is ${comparison.parse_p95_ratio}x v2; budget is ` +
      `${budgets.maximum_parse_p95_ratio}x`,
  );
}

function object(value: unknown, label: string): JsonObject {
  assert(
    value !== null && typeof value === "object" && !Array.isArray(value),
    `${label} must be an object`,
  );
  return value as JsonObject;
}

function array(value: unknown, label: string): any[] {
  assert(Array.isArray(value), `${label} must be an array`);
  return value;
}

function assert(condition: unknown, message: string): asserts condition {
  if (!condition) throw new Error(message);
}

function argument(name: string): string | undefined {
  const index = process.argv.indexOf(name);
  if (index < 0) return undefined;
  const value = process.argv[index + 1];
  if (!value) throw new Error(`${name} requires a value`);
  return value;
}

if (process.argv[1] && fileURLToPath(import.meta.url) === resolve(process.argv[1])) {
  const fixturePath = argument("--fixture");
  const outputPath = argument("--write");
  const sampleCountArgument = argument("--samples");
  const sampleCount = sampleCountArgument ? Number.parseInt(sampleCountArgument, 10) : 21;
  assert(Number.isInteger(sampleCount), "--samples must be an integer");
  const evidence = buildGuardrailEvidence(fixturePath, sampleCount);
  assertGuardrail(evidence);
  const output = `${JSON.stringify(evidence, null, 2)}\n`;
  if (outputPath) writeFileSync(resolve(outputPath), output);
  else process.stdout.write(output);
}
