import type {
  GroupDetailV3,
  MetricObservation,
  MetricScope,
  MetricSemantic,
  ProcessDetailV3,
  RuntimeSnapshotPayloadV3,
} from "../generated/runtime-protocol-v3.ts";
import type {
  GroupDetail,
  KernelPoolTag,
  MetricCoverage,
  MetricQualityInfo,
  ProcessContributorSummary,
  ProcessSample,
  ProcessViewRow,
  RuntimeSnapshot,
  SystemMetricsSnapshot,
} from "../types.ts";
import { groupAttentionLabel, processAttentionLabel } from "../process.ts";

interface MeasurementView {
  value: number | null;
  quality: MetricQualityInfo;
  descriptorIndex: number;
  limitationIndex: number | null;
}

export function adaptRuntimePayload(payload: RuntimeSnapshotPayloadV3): RuntimeSnapshot {
  const system = adaptSystem(payload);
  const processRows = new Map<string, ProcessViewRow>();
  const groupRows = new Map<string, ProcessViewRow>();
  const processes: ProcessSample[] = [];

  for (const workload of payload.workloads) {
    if (workload.kind === "process") {
      const row = adaptProcessRow(workload.detail, payload);
      processRows.set(workload.detail.stable_id, row);
      processes.push(row.detail.process);
    } else {
      groupRows.set(workload.detail.stable_id, adaptGroupRow(workload.detail, payload));
    }
  }
  const process_view_rows = payload.workloads.flatMap((workload) => {
    const row =
      workload.kind === "process"
        ? processRows.get(workload.detail.stable_id)
        : groupRows.get(workload.detail.stable_id);
    return row ? [row] : [];
  });

  return {
    event_kind: "runtime_snapshot",
    publication_seq: payload.publication_seq,
    published_at_ms: payload.published_at_ms,
    sample_seq: payload.sample_seq,
    sampled_at_ms: payload.sampled_at_ms,
    source: runtimeSource(payload.source),
    environment: {
      platform: payload.environment.platform,
      architecture: payload.environment.architecture,
      admin_mode_available: payload.privileged_collection.state !== "unavailable",
      process_elevation: payload.environment.process_elevation,
      install_kind: payload.environment.install_kind,
      data_directory: payload.environment.data_directory,
      release_identity: { ...payload.environment.release_identity },
    },
    admin_mode: {
      state: legacyAdminState(payload.privileged_collection.state),
      source: payload.privileged_collection.source,
      detail: payload.privileged_collection.detail,
      last_success_at_ms: payload.privileged_collection.last_success_at_ms,
    },
    settings: {
      query: { ...payload.settings.query },
      admin_mode_requested: payload.privileged_collection.preference === "best_available",
      admin_mode_enabled: payload.privileged_collection.state === "active",
      metric_window_seconds: payload.settings.metric_window_seconds,
      sample_interval_ms: payload.settings.effective_sample_interval_ms,
      paused: payload.settings.collection_paused,
    },
    health: { ...payload.health },
    system,
    process_contributors: adaptContributors(payload),
    processes,
    process_view_rows,
    total_process_count: payload.total_process_count,
    warnings: payload.warnings.map((warning) => ({ ...warning })),
  };
}

function legacyAdminState(
  state: RuntimeSnapshotPayloadV3["privileged_collection"]["state"],
): RuntimeSnapshot["admin_mode"]["state"] {
  switch (state) {
    case "unavailable":
      return "unavailable";
    case "standard_only":
      return "off";
    case "connecting":
      return "requesting";
    case "active":
      return "active";
    case "recovering":
      return "recovering";
    case "failed":
      return "failed";
  }
}

function adaptSystem(payload: RuntimeSnapshotPayloadV3): SystemMetricsSnapshot {
  const metrics = payload.system.metrics;
  const cpu = measurement(metrics, "cpu_usage", "system", payload);
  const kernelCpu = measurement(metrics, "kernel_cpu_usage", "system", payload);
  const memory = measurement(metrics, "memory_used", "system", payload);
  const memoryCapacity = measurement(metrics, "memory_capacity", "system", payload);
  const memoryAvailable = measurement(metrics, "memory_available", "system", payload);
  const swap = measurement(metrics, "swap_used", "system", payload);
  const swapCapacity = measurement(metrics, "swap_capacity", "system", payload);
  const processCount = measurement(metrics, "process_count", "system", payload);
  const diskReadTotal = measurement(metrics, "physical_disk_read_total", "system", payload);
  const diskWriteTotal = measurement(metrics, "physical_disk_write_total", "system", payload);
  const diskReadRate = measurement(metrics, "physical_disk_read_rate", "system", payload);
  const diskWriteRate = measurement(metrics, "physical_disk_write_rate", "system", payload);
  const netReceiveTotal = measurement(metrics, "network_receive_total", "system", payload);
  const netTransmitTotal = measurement(metrics, "network_transmit_total", "system", payload);
  const netReceiveRate = measurement(metrics, "network_receive_rate", "system", payload);
  const netTransmitRate = measurement(metrics, "network_transmit_rate", "system", payload);
  const logicalCpu = payload.system.logical_cpus.map((logical) =>
    requiredNumber(measurement(logical.metrics, "logical_cpu_usage", "system", payload).value),
  );
  const accountingWorking = optionalMeasurement(
    metrics,
    "process_working_set_memory",
    "system",
    payload,
  );
  const accountingPrivate = optionalMeasurement(
    metrics,
    "process_private_memory",
    "system",
    payload,
  );
  const memoryAccounting =
    accountingWorking || accountingPrivate
      ? {
          process_working_set_bytes: requiredNumber(accountingWorking?.value ?? null),
          process_private_bytes: requiredNumber(accountingPrivate?.value ?? null),
          denied_process_count: requiredNumber(
            optionalMeasurement(metrics, "denied_process_count", "system", payload)?.value ?? null,
          ),
          partial_process_count: requiredNumber(
            optionalMeasurement(metrics, "partial_process_count", "system", payload)?.value ?? null,
          ),
          ...optionalNumberField(
            "commit_used_bytes",
            optionalMeasurement(metrics, "commit_used", "system", payload)?.value,
          ),
          ...optionalNumberField(
            "commit_limit_bytes",
            optionalMeasurement(metrics, "commit_limit", "system", payload)?.value,
          ),
          ...optionalNumberField(
            "system_cache_bytes",
            optionalMeasurement(metrics, "system_cache", "system", payload)?.value,
          ),
          ...optionalNumberField(
            "kernel_total_bytes",
            optionalMeasurement(metrics, "kernel_memory", "system", payload)?.value,
          ),
          ...optionalNumberField(
            "kernel_paged_pool_bytes",
            optionalMeasurement(metrics, "kernel_paged_pool", "system", payload)?.value,
          ),
          ...optionalNumberField(
            "kernel_nonpaged_pool_bytes",
            optionalMeasurement(metrics, "kernel_nonpaged_pool", "system", payload)?.value,
          ),
          kernel_pool_tags: payload.system.kernel_pool_tags.map(
            (tag): KernelPoolTag => ({
              tag: tag.tag,
              kind: tag.kind,
              bytes: requiredNumber(
                measurement(tag.metrics, "kernel_pool_bytes", "system", payload).value,
              ),
              allocations: requiredNumber(
                measurement(tag.metrics, "kernel_pool_allocations", "system", payload).value,
              ),
              frees: requiredNumber(
                measurement(tag.metrics, "kernel_pool_frees", "system", payload).value,
              ),
              driver_candidates: [...tag.driver_candidates],
              ...(tag.driver_candidates_pending ? { driver_candidates_pending: true } : {}),
            }),
          ),
        }
      : undefined;

  return {
    cpu_percent: requiredNumber(cpu.value),
    kernel_cpu_percent: requiredNumber(kernelCpu.value),
    logical_cpu_percent: logicalCpu,
    memory_used_bytes: requiredNumber(memory.value),
    memory_total_bytes: requiredNumber(memoryCapacity.value),
    ...optionalNumberField("memory_available_bytes", memoryAvailable.value),
    ...optionalNumberField("swap_used_bytes", swap.value),
    ...optionalNumberField("swap_total_bytes", swapCapacity.value),
    process_count: requiredNumber(processCount.value),
    disk_read_total_bytes: requiredNumber(diskReadTotal.value),
    disk_write_total_bytes: requiredNumber(diskWriteTotal.value),
    disk_read_bps: requiredNumber(diskReadRate.value),
    disk_write_bps: requiredNumber(diskWriteRate.value),
    network_received_total_bytes: requiredNumber(netReceiveTotal.value),
    network_transmitted_total_bytes: requiredNumber(netTransmitTotal.value),
    network_received_bps: requiredNumber(netReceiveRate.value),
    network_transmitted_bps: requiredNumber(netTransmitRate.value),
    ...(memoryAccounting ? { memory_accounting: memoryAccounting } : {}),
    quality: {
      cpu: cpu.quality,
      kernel_cpu: kernelCpu.quality,
      logical_cpu: firstLogicalQuality(payload),
      memory: memory.quality,
      swap: swap.quality,
      disk: worstQuality(diskReadTotal.quality, diskReadRate.quality),
      network: worstQuality(netReceiveTotal.quality, netReceiveRate.quality),
    },
  };
}

function adaptProcessRow(
  detail: ProcessDetailV3,
  payload: RuntimeSnapshotPayloadV3,
): Extract<ProcessViewRow, { kind: "process" }> {
  const cpu = measurement(detail.metrics, "cpu_usage", "process", payload);
  const kernelCpu = measurement(detail.metrics, "kernel_cpu_usage", "process", payload);
  const resident = measurement(detail.metrics, "resident_memory", "process", payload);
  const privateMemory = measurement(detail.metrics, "private_memory", "process", payload);
  const virtualMemory = measurement(detail.metrics, "virtual_memory", "process", payload);
  const readTotal = measurement(detail.metrics, "read_io_total", "process", payload);
  const writeTotal = measurement(detail.metrics, "write_io_total", "process", payload);
  const otherTotal = measurement(detail.metrics, "other_io_total", "process", payload);
  const readRate = measurement(detail.metrics, "read_io_rate", "process", payload);
  const writeRate = measurement(detail.metrics, "write_io_rate", "process", payload);
  const otherRate = measurement(detail.metrics, "other_io_rate", "process", payload);
  const networkReceive = measurement(detail.metrics, "network_receive_rate", "process", payload);
  const networkTransmit = measurement(detail.metrics, "network_transmit_rate", "process", payload);
  const threads = measurement(detail.metrics, "thread_count", "process", payload);
  const handles = measurement(detail.metrics, "handle_count", "process", payload);
  const process: ProcessSample = {
    pid: detail.pid,
    parent_pid: detail.parent_pid,
    start_time_ms: detail.start_time_ms ?? 0,
    name: detail.display_name,
    exe: detail.executable,
    status: detail.status,
    cpu_percent: requiredNumber(cpu.value),
    ...optionalNumberField("kernel_cpu_percent", kernelCpu.value),
    memory_bytes: requiredNumber(resident.value),
    private_bytes: requiredNumber(privateMemory.value),
    ...optionalNumberField("virtual_memory_bytes", virtualMemory.value),
    io_read_total_bytes: requiredNumber(readTotal.value),
    io_write_total_bytes: requiredNumber(writeTotal.value),
    ...optionalNumberField("other_io_total_bytes", otherTotal.value),
    io_read_bps: requiredNumber(readRate.value),
    io_write_bps: requiredNumber(writeRate.value),
    ...optionalNumberField("other_io_bps", otherRate.value),
    ...optionalNumberField("network_received_bps", networkReceive.value),
    ...optionalNumberField("network_transmitted_bps", networkTransmit.value),
    threads: requiredNumber(threads.value),
    handles: requiredNumber(handles.value),
    access_state: detail.access_state,
    quality: {
      cpu: cpu.quality,
      memory: resident.quality,
      io: worstQuality(readRate.quality, writeRate.quality),
      other_io: otherRate.quality,
      network: worstQuality(networkReceive.quality, networkTransmit.quality),
      threads: threads.quality,
      handles: handles.quality,
    },
  };
  return {
    kind: "process",
    detail: {
      kind: "process",
      workload_id: detail.stable_id,
      process,
      io_bps: sumMeasurements(readRate.value, writeRate.value),
      network_bps: sumMeasurements(networkReceive.value, networkTransmit.value),
    },
    group_key: detail.presentation.group_key,
    group_label: detail.presentation.group_label,
    group_category: detail.presentation.group_category,
    group_count: detail.presentation.group_count,
    icon_kind: detail.presentation.icon_kind,
    is_child: detail.presentation.is_child,
    is_grouped: detail.presentation.is_grouped,
    attention_label: processAttentionLabel(process),
  };
}

function adaptGroupRow(
  detail: GroupDetailV3,
  payload: RuntimeSnapshotPayloadV3,
): Extract<ProcessViewRow, { kind: "group" }> {
  const cpu = measurement(detail.metrics, "cpu_usage", "group", payload);
  const memory = measurement(detail.metrics, "resident_memory", "group", payload);
  const io = measurement(detail.metrics, "read_write_io_rate", "group", payload);
  const otherIo = measurement(detail.metrics, "other_io_rate", "group", payload);
  const network = measurement(detail.metrics, "network_rate", "group", payload);
  const threads = measurement(detail.metrics, "thread_count", "group", payload);
  const group: GroupDetail = {
    kind: "group",
    workload_id: detail.stable_id,
    group_key: detail.group_key,
    label: detail.label,
    category: detail.category,
    process_count: detail.member_ids.length,
    cpu_percent: requiredNumber(cpu.value),
    memory_bytes: requiredNumber(memory.value),
    io_bps: requiredNumber(io.value),
    ...optionalNumberField("other_io_bps", otherIo.value),
    network_bps: requiredNumber(network.value),
    threads: requiredNumber(threads.value),
    quality: {
      cpu: cpu.quality,
      memory: memory.quality,
      io: io.quality,
      other_io: otherIo.quality,
      network: network.quality,
      threads: threads.quality,
    },
    coverage: {
      cpu: coverageFor(detail, cpu),
      memory: coverageFor(detail, memory),
      io: coverageFor(detail, io),
      other_io: coverageFor(detail, otherIo),
      network: coverageFor(detail, network),
      threads: coverageFor(detail, threads),
    },
  };
  return {
    kind: "group",
    detail: group,
    icon_kind: detail.icon_kind,
    ...(detail.icon_source ? { icon_source: detail.icon_source } : {}),
    ...(detail.example_label ? { example_label: detail.example_label } : {}),
    attention_label: groupAttentionLabel(
      group,
      detail.member_ids.some((memberId) => {
        const member = payload.workloads.find(
          (workload) => workload.kind === "process" && workload.detail.stable_id === memberId,
        );
        return member?.kind === "process" && member.detail.access_state !== "full";
      }),
    ),
  };
}

function adaptContributors(payload: RuntimeSnapshotPayloadV3): ProcessContributorSummary {
  const result: ProcessContributorSummary = {
    cpu: null,
    cpu_process_id: null,
    cpu_coverage: { available: 0, total: payload.total_process_count },
    cpu_name_ambiguous: false,
    memory: null,
    memory_process_id: null,
    memory_coverage: { available: 0, total: payload.total_process_count },
    memory_name_ambiguous: false,
    io: null,
    io_process_id: null,
    io_coverage: { available: 0, total: payload.total_process_count },
    io_name_ambiguous: false,
    network: null,
    network_process_id: null,
    network_coverage: { available: 0, total: payload.total_process_count },
    network_name_ambiguous: false,
  };
  for (const contributor of payload.contributors) {
    const quality: MetricQualityInfo = {
      quality: payload.quality_codes[contributor.quality_code],
      source: contributor.source,
      ...(contributor.limitation_index !== null
        ? { message: payload.limitations[contributor.limitation_index]?.message }
        : {}),
    };
    const coverage = {
      available: contributor.available_contributors,
      total: contributor.total_contributors,
    };
    switch (contributor.metric) {
      case "cpu":
        result.cpu = contributor.display_name;
        result.cpu_process_id = contributor.process_id;
        result.cpu_coverage = coverage;
        result.cpu_name_ambiguous = contributor.name_ambiguous;
        result.cpu_quality = quality;
        break;
      case "memory":
        result.memory = contributor.display_name;
        result.memory_process_id = contributor.process_id;
        result.memory_coverage = coverage;
        result.memory_name_ambiguous = contributor.name_ambiguous;
        result.memory_quality = quality;
        break;
      case "io":
        result.io = contributor.display_name;
        result.io_process_id = contributor.process_id;
        result.io_coverage = coverage;
        result.io_name_ambiguous = contributor.name_ambiguous;
        result.io_quality = quality;
        break;
      case "network":
        result.network = contributor.display_name;
        result.network_process_id = contributor.process_id;
        result.network_coverage = coverage;
        result.network_name_ambiguous = contributor.name_ambiguous;
        result.network_quality = quality;
        break;
    }
  }
  return result;
}

function measurement(
  metrics: MetricObservation[],
  semantic: MetricSemantic,
  scope: MetricScope,
  payload: RuntimeSnapshotPayloadV3,
): MeasurementView {
  const observation = metrics.find(
    (candidate) => payload.descriptors[candidate[0]]?.semantic === semantic,
  );
  if (!observation) return unavailableMeasurement("Metric was omitted from the protocol payload.");
  const descriptor = payload.descriptors[observation[0]];
  if (!descriptor || descriptor.scope !== scope)
    return unavailableMeasurement("Metric scope does not match its subject.");
  const quality = payload.quality_codes[observation[2]];
  const limitation = observation[4] === null ? undefined : payload.limitations[observation[4]];
  return {
    value: observation[1],
    descriptorIndex: observation[0],
    limitationIndex: observation[4],
    quality: {
      quality,
      source: descriptor.source,
      ...(observation[3] !== null
        ? {
            updated_at_ms: observation[3],
            age_ms: Math.max(0, payload.published_at_ms - observation[3]),
          }
        : {}),
      ...(limitation ? { message: limitation.message } : {}),
    },
  };
}

function optionalMeasurement(
  metrics: MetricObservation[],
  semantic: MetricSemantic,
  scope: MetricScope,
  payload: RuntimeSnapshotPayloadV3,
): MeasurementView | undefined {
  return metrics.some((candidate) => payload.descriptors[candidate[0]]?.semantic === semantic)
    ? measurement(metrics, semantic, scope, payload)
    : undefined;
}

function unavailableMeasurement(message: string): MeasurementView {
  return {
    value: null,
    descriptorIndex: -1,
    limitationIndex: null,
    quality: { quality: "unavailable", source: "runtime", message },
  };
}

function coverageFor(detail: GroupDetailV3, measurement: MeasurementView): MetricCoverage {
  const coverage = detail.coverage.find(
    (entry) => entry.descriptor_index === measurement.descriptorIndex,
  );
  return coverage
    ? { available: coverage.available_contributors, total: coverage.total_contributors }
    : { available: 0, total: detail.member_ids.length };
}

function firstLogicalQuality(payload: RuntimeSnapshotPayloadV3): MetricQualityInfo {
  const first = payload.system.logical_cpus[0];
  return first
    ? measurement(first.metrics, "logical_cpu_usage", "system", payload).quality
    : { quality: "unavailable", source: "runtime" };
}

function worstQuality(left: MetricQualityInfo, right: MetricQualityInfo): MetricQualityInfo {
  const rank = { native: 0, estimated: 1, partial: 2, held: 3, unavailable: 4 } as const;
  return rank[left.quality] >= rank[right.quality] ? left : right;
}

function requiredNumber(value: number | null): number {
  return value ?? Number.NaN;
}

function sumMeasurements(left: number | null, right: number | null): number {
  return left === null || right === null ? Number.NaN : left + right;
}

function optionalNumberField<Key extends string>(
  key: Key,
  value: number | null | undefined,
): Partial<Record<Key, number>> {
  return value === null || value === undefined ? {} : ({ [key]: value } as Record<Key, number>);
}

function runtimeSource(value: string): RuntimeSnapshot["source"] {
  return value === "fixture" || value === "tauri_sysinfo" || value === "batcave_runtime"
    ? value
    : "tauri_runtime";
}
