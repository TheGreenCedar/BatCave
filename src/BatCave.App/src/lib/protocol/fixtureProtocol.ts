import {
  RUNTIME_PROTOCOL_VERSION,
  type GroupMetricCoverageV3,
  type LimitationEntry,
  type MeasurementDescriptor,
  type MetricObservation,
  type MetricQualityV3,
  type MetricScope,
  type MetricSemantic,
  type MetricSourceV3,
  type MetricUnit,
  type ProtocolEnvelope,
  type WorkloadDetailV3,
} from "../generated/runtime-protocol-v3.ts";
import type { MetricQualityInfo, ProcessSample, RuntimeSnapshot } from "../types.ts";
import { adaptRuntimePayload } from "./runtimeAdapter.ts";
import { decodeProtocolEnvelope } from "./runtimeProtocol.ts";

const qualityCodes: MetricQualityV3[] = ["native", "estimated", "held", "partial", "unavailable"];

export function canonicalKernelPoolStableId(tag: string, kind: "paged" | "nonpaged"): string {
  return `system:local:pool:${tag}:${kind}`.toLocaleLowerCase();
}

export function roundTripFixtureSnapshot(snapshot: RuntimeSnapshot): RuntimeSnapshot {
  const decoded = decodeProtocolEnvelope(encodeFixtureSnapshot(snapshot));
  if (decoded.kind !== "snapshot") throw new Error(decoded.mismatch.message);
  return adaptRuntimePayload(decoded.payload);
}

export function encodeFixtureSnapshot(snapshot: RuntimeSnapshot): ProtocolEnvelope {
  const catalog = new FixtureCatalog(snapshot.settings.sample_interval_ms);
  const sampled = snapshot.sampled_at_ms;
  const systemQuality = snapshot.system.quality;
  const systemMetrics: MetricObservation[] = [
    catalog.metric(
      "cpu_usage",
      "system",
      "percent_system",
      snapshot.system.cpu_percent,
      systemQuality?.cpu,
      sampled,
    ),
    catalog.metric(
      "kernel_cpu_usage",
      "system",
      "percent_system",
      snapshot.system.kernel_cpu_percent,
      systemQuality?.kernel_cpu,
      sampled,
    ),
    catalog.metric(
      "memory_used",
      "system",
      "bytes",
      snapshot.system.memory_used_bytes,
      systemQuality?.memory,
      sampled,
    ),
    catalog.metric(
      "memory_capacity",
      "system",
      "bytes",
      snapshot.system.memory_total_bytes,
      systemQuality?.memory,
      sampled,
    ),
    catalog.metric(
      "memory_available",
      "system",
      "bytes",
      snapshot.system.memory_available_bytes,
      systemQuality?.memory,
      sampled,
    ),
    catalog.metric(
      "swap_used",
      "system",
      "bytes",
      snapshot.system.swap_used_bytes,
      systemQuality?.swap,
      sampled,
    ),
    catalog.metric(
      "swap_capacity",
      "system",
      "bytes",
      snapshot.system.swap_total_bytes,
      systemQuality?.swap,
      sampled,
    ),
    catalog.metric(
      "process_count",
      "system",
      "count",
      snapshot.system.process_count,
      nativeRuntime(),
      sampled,
    ),
    catalog.metric(
      "physical_disk_read_total",
      "system",
      "bytes",
      snapshot.system.disk_read_total_bytes,
      systemQuality?.disk,
      sampled,
    ),
    catalog.metric(
      "physical_disk_write_total",
      "system",
      "bytes",
      snapshot.system.disk_write_total_bytes,
      systemQuality?.disk,
      sampled,
    ),
    catalog.metric(
      "physical_disk_read_rate",
      "system",
      "bytes_per_second",
      snapshot.system.disk_read_bps,
      systemQuality?.disk,
      sampled,
    ),
    catalog.metric(
      "physical_disk_write_rate",
      "system",
      "bytes_per_second",
      snapshot.system.disk_write_bps,
      systemQuality?.disk,
      sampled,
    ),
    catalog.metric(
      "network_receive_total",
      "system",
      "bytes",
      snapshot.system.network_received_total_bytes,
      systemQuality?.network,
      sampled,
    ),
    catalog.metric(
      "network_transmit_total",
      "system",
      "bytes",
      snapshot.system.network_transmitted_total_bytes,
      systemQuality?.network,
      sampled,
    ),
    catalog.metric(
      "network_receive_rate",
      "system",
      "bytes_per_second",
      snapshot.system.network_received_bps,
      systemQuality?.network,
      sampled,
    ),
    catalog.metric(
      "network_transmit_rate",
      "system",
      "bytes_per_second",
      snapshot.system.network_transmitted_bps,
      systemQuality?.network,
      sampled,
    ),
  ];
  const accounting = snapshot.system.memory_accounting;
  if (accounting) {
    for (const [semantic, value, unit] of [
      ["process_working_set_memory", accounting.process_working_set_bytes, "bytes"],
      ["process_private_memory", accounting.process_private_bytes, "bytes"],
      ["denied_process_count", accounting.denied_process_count, "count"],
      ["partial_process_count", accounting.partial_process_count, "count"],
      ["commit_used", accounting.commit_used_bytes, "bytes"],
      ["commit_limit", accounting.commit_limit_bytes, "bytes"],
      ["system_cache", accounting.system_cache_bytes, "bytes"],
      ["kernel_memory", accounting.kernel_total_bytes, "bytes"],
      ["kernel_paged_pool", accounting.kernel_paged_pool_bytes, "bytes"],
      ["kernel_nonpaged_pool", accounting.kernel_nonpaged_pool_bytes, "bytes"],
    ] as const) {
      systemMetrics.push(
        catalog.metric(semantic, "system", unit, value, systemQuality?.memory, sampled),
      );
    }
  }
  const processId = (process: ProcessSample) =>
    process.start_time_ms
      ? `process:${process.pid}:${process.start_time_ms}`
      : `process:${process.pid}:publication:${snapshot.sample_seq}`;
  const memberIds = new Map<string, string[]>();
  for (const row of snapshot.process_view_rows) {
    if (row.kind === "process" && row.is_grouped) {
      memberIds.set(row.group_key, [
        ...(memberIds.get(row.group_key) ?? []),
        processId(row.detail.process),
      ]);
    }
  }
  const parentIds = new Map(snapshot.processes.map((process) => [process.pid, processId(process)]));
  const workloads: WorkloadDetailV3[] = snapshot.process_view_rows.map((row) => {
    if (row.kind === "process") {
      const process = row.detail.process;
      return {
        kind: "process",
        detail: {
          stable_id: processId(process),
          identity_stability: process.start_time_ms ? "stable" : "publication",
          pid: process.pid,
          parent_pid: process.parent_pid,
          parent_process_id: process.parent_pid
            ? (parentIds.get(process.parent_pid) ?? null)
            : null,
          start_time_ms: process.start_time_ms || null,
          display_name: process.name,
          executable: process.exe,
          status: process.status,
          access_state: process.access_state,
          presentation: {
            group_id: row.is_grouped ? `group:${row.group_key}` : null,
            group_key: row.group_key,
            group_label: row.group_label,
            group_category: row.group_category,
            group_count: row.group_count,
            icon_kind: row.icon_kind,
            is_child: row.is_child,
            is_grouped: row.is_grouped,
          },
          metrics: processMetrics(catalog, process, sampled),
        },
      };
    }
    const detail = row.detail;
    const specs = [
      [
        "cpu_usage",
        "percent_one_core",
        detail.cpu_percent,
        detail.quality.cpu,
        detail.coverage.cpu,
      ],
      [
        "resident_memory",
        "bytes",
        detail.memory_bytes,
        detail.quality.memory,
        detail.coverage.memory,
      ],
      [
        "read_write_io_rate",
        "bytes_per_second",
        detail.io_bps,
        detail.quality.io,
        detail.coverage.io,
      ],
      [
        "other_io_rate",
        "bytes_per_second",
        detail.other_io_bps,
        detail.quality.other_io,
        detail.coverage.other_io,
      ],
      [
        "network_rate",
        "bytes_per_second",
        detail.network_bps,
        detail.quality.network,
        detail.coverage.network,
      ],
      ["thread_count", "count", detail.threads, detail.quality.threads, detail.coverage.threads],
    ] as const;
    const metrics: MetricObservation[] = [];
    const coverage: GroupMetricCoverageV3[] = [];
    for (const [semantic, unit, value, quality, currentCoverage] of specs) {
      const metric = catalog.metric(semantic, "group", unit, value, quality, sampled);
      metrics.push(metric);
      coverage.push({
        descriptor_index: metric[0],
        available_contributors: currentCoverage.available,
        total_contributors: currentCoverage.total,
        limitation_index: metric[4],
      });
    }
    return {
      kind: "group",
      detail: {
        stable_id: detail.workload_id,
        group_key: detail.group_key,
        label: detail.label,
        category: detail.category,
        member_ids: memberIds.get(detail.group_key) ?? [],
        icon_kind: row.icon_kind,
        icon_source: row.icon_source ?? null,
        example_label: row.example_label ?? null,
        metrics,
        coverage,
      },
    };
  });
  const contributor = (metric: "cpu" | "memory" | "io" | "network") => {
    const name = snapshot.process_contributors[metric];
    const quality = snapshot.process_contributors[`${metric}_quality`];
    const hasSource = quality?.source != null;
    const coverage = snapshot.process_contributors[`${metric}_coverage`];
    let qualityCode = quality?.source ? qualityCodes.indexOf(quality.quality) : 4;
    let limitationIndex = quality?.message
      ? catalog.limit(
          quality.limitation_code ??
            (quality.quality === "held"
              ? "held_value"
              : quality.quality === "partial"
                ? "partial_coverage"
                : "unsupported_metric"),
          quality.message,
        )
      : !hasSource
        ? catalog.limit(
            "missing_metadata",
            "Contributor source provenance was not reported by fixture telemetry.",
          )
        : coverage.available < coverage.total
          ? catalog.limit(
              "partial_coverage",
              `${coverage.available} of ${coverage.total} processes provide this contributor metric.`,
            )
          : null;
    if (coverage.available < coverage.total && [0, 1].includes(qualityCode)) qualityCode = 3;
    if ([2, 3, 4].includes(qualityCode) && limitationIndex === null) {
      qualityCode = 4;
      limitationIndex = catalog.limit(
        "missing_metadata",
        "Contributor quality is missing a typed explanation.",
      );
    }
    const processId = hasSource ? snapshot.process_contributors[`${metric}_process_id`] : null;
    return {
      metric,
      process_id: processId,
      display_name: processId ? name : null,
      name_ambiguous: snapshot.process_contributors[`${metric}_name_ambiguous`],
      available_contributors: coverage.available,
      total_contributors: coverage.total,
      quality_code: qualityCode,
      source: quality?.source ?? "unknown",
      limitation_index: limitationIndex,
    };
  };
  const payload = {
    publication_seq: snapshot.publication_seq,
    published_at_ms: snapshot.published_at_ms,
    sample_seq: snapshot.sample_seq,
    sampled_at_ms: snapshot.sampled_at_ms,
    source: "fixture",
    environment: {
      platform: snapshot.environment.platform,
      architecture: snapshot.environment.architecture,
      process_elevation: snapshot.environment.process_elevation,
      install_kind: snapshot.environment.install_kind,
      data_directory: snapshot.environment.data_directory,
      release_identity: { ...snapshot.environment.release_identity },
    },
    privileged_collection: {
      state: fixturePrivilegedState(snapshot.admin_mode.state),
      source: fixturePrivilegedSource(snapshot),
      preference: snapshot.settings.admin_mode_requested
        ? ("best_available" as const)
        : ("standard_only" as const),
      detail: snapshot.admin_mode.detail,
      last_success_at_ms: snapshot.admin_mode.last_success_at_ms,
      collector_service: null,
    },
    settings: {
      query: { ...snapshot.settings.query },
      metric_window_seconds: snapshot.settings.metric_window_seconds,
      effective_sample_interval_ms: snapshot.settings.sample_interval_ms,
      collection_paused: snapshot.settings.paused,
      ui_preferences: null,
    },
    health: fixtureHealth(snapshot),
    persistence: null,
    descriptors: catalog.descriptors,
    quality_codes: qualityCodes,
    limitations: catalog.limitations,
    system: {
      stable_id: "system:local",
      metrics: systemMetrics,
      logical_cpus: snapshot.system.logical_cpu_percent.map((value, index) => ({
        stable_id: `system:local:cpu:${index}`,
        index,
        metrics: [
          catalog.metric(
            "logical_cpu_usage",
            "system",
            "percent_system",
            value,
            systemQuality?.logical_cpu,
            sampled,
          ),
        ],
      })),
      kernel_pool_tags: (accounting?.kernel_pool_tags ?? []).map((tag) => ({
        stable_id: canonicalKernelPoolStableId(tag.tag, tag.kind),
        tag: tag.tag,
        kind: tag.kind,
        driver_candidates: tag.driver_candidates,
        driver_candidates_pending: tag.driver_candidates_pending ?? false,
        metrics: [
          catalog.metric(
            "kernel_pool_bytes",
            "system",
            "bytes",
            tag.bytes,
            systemQuality?.memory,
            sampled,
          ),
          catalog.metric(
            "kernel_pool_allocations",
            "system",
            "count",
            tag.allocations,
            systemQuality?.memory,
            sampled,
          ),
          catalog.metric(
            "kernel_pool_frees",
            "system",
            "count",
            tag.frees,
            systemQuality?.memory,
            sampled,
          ),
        ],
      })),
    },
    workloads,
    contributors: [
      contributor("cpu"),
      contributor("memory"),
      contributor("io"),
      contributor("network"),
    ],
    total_process_count: snapshot.total_process_count,
    visible_process_count: workloads.filter((workload) => workload.kind === "process").length,
    warnings: snapshot.warnings.map((warning) => ({ ...warning })),
  };
  return {
    protocol_version: RUNTIME_PROTOCOL_VERSION,
    compatibility: { minimum_reader_version: RUNTIME_PROTOCOL_VERSION, breaking: true },
    event: { kind: "runtime_snapshot", payload },
  };
}

function fixturePrivilegedState(
  state: RuntimeSnapshot["admin_mode"]["state"],
): "unavailable" | "standard_only" | "connecting" | "active" | "recovering" | "failed" {
  if (state === "off") return "standard_only";
  if (state === "requesting") return "connecting";
  return state;
}

function fixturePrivilegedSource(
  snapshot: RuntimeSnapshot,
): "none" | "local_process" | "collector_service" {
  if (fixturePrivilegedState(snapshot.admin_mode.state) !== "active") return "none";
  return snapshot.admin_mode.source === "collector_service" ? "collector_service" : "local_process";
}

function fixtureHealth(snapshot: RuntimeSnapshot): RuntimeSnapshot["health"] {
  const evaluatedAt = Math.max(
    snapshot.health.evaluated_at_ms,
    snapshot.published_at_ms,
    snapshot.sampled_at_ms ?? 0,
  );
  return {
    ...snapshot.health,
    evaluated_at_ms: evaluatedAt,
    publication_age_ms: evaluatedAt - snapshot.published_at_ms,
    sample_age_ms: snapshot.sampled_at_ms === null ? null : evaluatedAt - snapshot.sampled_at_ms,
  };
}

function processMetrics(
  catalog: FixtureCatalog,
  process: ProcessSample,
  sampled: number | null,
): MetricObservation[] {
  const quality = process.quality;
  return [
    catalog.metric(
      "cpu_usage",
      "process",
      "percent_one_core",
      process.cpu_percent,
      quality?.cpu,
      sampled,
    ),
    catalog.metric(
      "kernel_cpu_usage",
      "process",
      "percent_one_core",
      process.kernel_cpu_percent,
      quality?.cpu,
      sampled,
    ),
    catalog.metric(
      "resident_memory",
      "process",
      "bytes",
      process.memory_bytes,
      quality?.memory,
      sampled,
    ),
    catalog.metric(
      "private_memory",
      "process",
      "bytes",
      process.private_bytes,
      quality?.memory,
      sampled,
    ),
    catalog.metric(
      "virtual_memory",
      "process",
      "bytes",
      process.virtual_memory_bytes,
      quality?.memory,
      sampled,
    ),
    catalog.metric(
      "read_io_total",
      "process",
      "bytes",
      process.io_read_total_bytes,
      totalsQuality(quality?.io),
      sampled,
    ),
    catalog.metric(
      "write_io_total",
      "process",
      "bytes",
      process.io_write_total_bytes,
      totalsQuality(quality?.io),
      sampled,
    ),
    catalog.metric(
      "other_io_total",
      "process",
      "bytes",
      process.other_io_total_bytes,
      totalsQuality(quality?.other_io),
      sampled,
    ),
    catalog.metric(
      "read_io_rate",
      "process",
      "bytes_per_second",
      process.io_read_bps,
      quality?.io,
      sampled,
    ),
    catalog.metric(
      "write_io_rate",
      "process",
      "bytes_per_second",
      process.io_write_bps,
      quality?.io,
      sampled,
    ),
    catalog.metric(
      "other_io_rate",
      "process",
      "bytes_per_second",
      process.other_io_bps,
      quality?.other_io,
      sampled,
    ),
    catalog.metric(
      "network_receive_rate",
      "process",
      "bytes_per_second",
      process.network_received_bps,
      quality?.network,
      sampled,
    ),
    catalog.metric(
      "network_transmit_rate",
      "process",
      "bytes_per_second",
      process.network_transmitted_bps,
      quality?.network,
      sampled,
    ),
    catalog.metric("thread_count", "process", "count", process.threads, quality?.threads, sampled),
    catalog.metric("handle_count", "process", "count", process.handles, quality?.handles, sampled),
  ];
}

class FixtureCatalog {
  descriptors: MeasurementDescriptor[] = [];
  limitations: LimitationEntry[] = [];
  private readonly sampleIntervalMs: number;

  constructor(sampleIntervalMs: number) {
    if (!Number.isSafeInteger(sampleIntervalMs) || sampleIntervalMs <= 0) {
      throw new Error("Fixture sample interval must be a positive safe integer.");
    }
    this.sampleIntervalMs = sampleIntervalMs;
  }

  metric(
    semantic: MetricSemantic,
    scope: MetricScope,
    unit: MetricUnit,
    input: number | undefined,
    quality: MetricQualityInfo | undefined,
    sampled: number | null,
  ): MetricObservation {
    const missingSource = quality?.source == null;
    const source = quality?.source ?? "unknown";
    const descriptor =
      this.descriptors.find(
        (entry) =>
          entry.semantic === semantic &&
          entry.scope === scope &&
          entry.unit === unit &&
          entry.source === source,
      ) ?? this.addDescriptor(semantic, scope, unit, source);
    let value = input ?? null;
    let qualityCode = quality && !missingSource ? qualityCodes.indexOf(quality.quality) : 4;
    const pending = quality?.quality === "held" && quality.limitation_code === "pending_baseline";
    if (qualityCode === 4 || pending || !Number.isFinite(value)) value = null;
    if (value === null && [0, 1, 3].includes(qualityCode)) qualityCode = 4;
    if (qualityCode < 0) qualityCode = 4;
    const heldMissingTime =
      quality?.quality === "held" && !pending && (value === null || quality.updated_at_ms == null);
    if (heldMissingTime) {
      value = null;
      qualityCode = 4;
    }
    let limitation = missingSource
      ? this.limit(
          "missing_metadata",
          "Metric source provenance was not reported by fixture telemetry.",
        )
      : heldMissingTime
        ? this.limit("missing_metadata", "Held metric is missing its original observation time.")
        : quality?.message
          ? this.limit(
              quality.limitation_code ??
                (pending
                  ? "pending_baseline"
                  : quality.quality === "held"
                    ? "held_value"
                    : quality.quality === "partial"
                      ? "partial_coverage"
                      : "unsupported_metric"),
              quality.message,
            )
          : value === null
            ? this.limit("unsupported_metric", "Metric is unavailable in fixture telemetry.")
            : null;
    if (limitation === null && [2, 3, 4].includes(qualityCode)) {
      limitation = this.limit(
        qualityCode === 2
          ? "held_value"
          : qualityCode === 3
            ? "partial_coverage"
            : "unsupported_metric",
        "Fixture metric quality requires an explicit explanation.",
      );
    }
    return [
      descriptor.id,
      value,
      qualityCode,
      value === null
        ? null
        : quality?.quality === "held"
          ? (quality.updated_at_ms ?? null)
          : (quality?.updated_at_ms ?? sampled),
      limitation,
    ];
  }

  limit(code: LimitationEntry["code"], message: string): number {
    const index = this.limitations.findIndex(
      (entry) => entry.code === code && entry.message === message,
    );
    if (index >= 0) return index;
    this.limitations.push({ code, message });
    return this.limitations.length - 1;
  }

  private addDescriptor(
    semantic: MetricSemantic,
    scope: MetricScope,
    unit: MetricUnit,
    source: MetricSourceV3,
  ): MeasurementDescriptor {
    const descriptor = {
      id: this.descriptors.length,
      semantic,
      scope,
      unit,
      interval_ms: ["percent_one_core", "percent_system", "bytes_per_second"].includes(unit)
        ? this.sampleIntervalMs
        : null,
      network_scope: networkScope(semantic, scope, source),
      source,
    };
    this.descriptors.push(descriptor);
    return descriptor;
  }
}

function networkScope(
  semantic: MetricSemantic,
  scope: MetricScope,
  source: MetricSourceV3,
): "non_loopback_interface_aggregate" | "all_interface_aggregate" | "ip_socket_payload" | null {
  if (source === "unknown") return null;
  if (
    scope === "system" &&
    [
      "network_receive_total",
      "network_transmit_total",
      "network_receive_rate",
      "network_transmit_rate",
    ].includes(semantic)
  )
    return source === "sysinfo" ? "all_interface_aggregate" : "non_loopback_interface_aggregate";
  if (
    (scope === "process" && ["network_receive_rate", "network_transmit_rate"].includes(semantic)) ||
    (scope === "group" && semantic === "network_rate")
  )
    return "ip_socket_payload";
  return null;
}

function totalsQuality(quality: MetricQualityInfo | undefined): MetricQualityInfo | undefined {
  if (quality?.quality !== "held" || quality.limitation_code !== "pending_baseline") return quality;
  return { quality: quality.source === "sysinfo" ? "estimated" : "native", source: quality.source };
}

function nativeRuntime(): MetricQualityInfo {
  return { quality: "native", source: "runtime" };
}
