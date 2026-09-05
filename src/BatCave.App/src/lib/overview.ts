import {
  formatBytes,
  formatPercent,
  formatRate,
  displayGroupMetricValue,
  displayProcessMetricValue,
} from "./format.ts";
import {
  advanceProcessRanking,
  ProcessInteraction,
  processViewRowKey,
  processViewRowMetrics,
} from "./process.ts";
import {
  buildTelemetryPresentation,
  metricPresentation,
  type CollectionState,
} from "./telemetryPresentation.ts";
import type { DetailMode } from "./components/metrics/types.ts";
import type { MetricQualityInfo, ProcessViewRow, RuntimeSnapshot } from "./types.ts";

export type OverviewTone = "neutral" | "healthy" | "warning" | "danger";
export type OverviewCollectionState = CollectionState;

export interface OverviewStatus {
  headline: string;
  summary: string;
  tone: OverviewTone;
  attention: { title: string; detail: string; tone: "warning" | "danger" } | null;
  primaryResource: DetailMode;
}

const resourceLabels: Record<DetailMode, string> = {
  cpu: "Machine CPU",
  memory: "Physical memory",
  disk: "Physical disk throughput",
  network: "Interface network throughput",
};

export function buildOverviewStatus(
  snapshot: RuntimeSnapshot,
  collectionState: CollectionState,
  limitationCount: number,
  primaryResource: DetailMode = "cpu",
): OverviewStatus {
  const telemetry = buildTelemetryPresentation(snapshot, collectionState);
  const quality = snapshot.system.quality?.[primaryResource];
  const metric = metricPresentation(quality, telemetry.state, snapshot.sampled_at_ms !== null);
  const value =
    primaryResource === "cpu"
      ? formatPercent(snapshot.system.cpu_percent)
      : primaryResource === "memory"
        ? formatPercent(
            snapshot.system.memory_total_bytes > 0
              ? (snapshot.system.memory_used_bytes / snapshot.system.memory_total_bytes) * 100
              : 0,
          )
        : primaryResource === "disk"
          ? formatRate(snapshot.system.disk_read_bps + snapshot.system.disk_write_bps)
          : formatRate(
              snapshot.system.network_received_bps + snapshot.system.network_transmitted_bps,
            );
  const label = resourceLabels[primaryResource];
  const headline =
    telemetry.state === "starting"
      ? "Waiting for measurements."
      : !metric.canDisplay
        ? `${label} is ${metric.emptyLabel.toLocaleLowerCase()}.`
        : `${label}${telemetry.state === "live" ? " is" : " was"} ${value}.`;
  const summary =
    telemetry.state !== "live"
      ? telemetry.detail
      : !metric.canDisplay
        ? (quality?.message ??
          "This measurement has no current value. Choose another resource to inspect available activity.")
        : primaryResource === "cpu"
          ? "Machine CPU measures total capacity. Process CPU below uses one logical core as 100%."
          : primaryResource === "memory"
            ? "Physical memory occupancy. Process resident memory is ranked separately and does not sum to this total."
            : primaryResource === "disk"
              ? "Device throughput. Process read/write I/O is ranked separately and does not identify physical disk activity."
              : "Network interface throughput. Process traffic is attributed separately.";
  const warning = telemetry.tone !== "healthy" && telemetry.state === "live";
  return {
    headline,
    summary,
    tone: telemetry.state === "starting" ? "neutral" : telemetry.tone,
    attention: warning
      ? {
          title: telemetry.label,
          detail: telemetry.detail,
          tone: telemetry.tone === "danger" ? "danger" : "warning",
        }
      : limitationCount > 0 && telemetry.state === "live"
        ? {
            title: `${limitationCount} data limitation${limitationCount === 1 ? "" : "s"}`,
            detail: "Affected measurements carry their quality beside the value.",
            tone: "warning",
          }
        : null,
    primaryResource,
  };
}

/** Owns only Overview ordering; Explore controls and selection never enter this state. */
export class OverviewRanking {
  private resource: DetailMode | null = null;
  private rows: ProcessViewRow[] = [];
  private incoming: ProcessViewRow[] = [];
  private interacting = false;
  private readonly interaction = new ProcessInteraction();

  update(resource: DetailMode, incoming: ProcessViewRow[]): ProcessViewRow[] {
    const held = this.resource === resource && this.interacting;
    this.resource = resource;
    this.incoming = incoming;
    this.rows = advanceProcessRanking(this.rows, incoming, held).rows;
    return this.rows;
  }

  setInteraction(source: "pointer" | "focus", active: boolean): ProcessViewRow[] {
    this.interacting = this.interaction.set(source, active);
    if (!this.interacting) this.rows = this.incoming;
    return this.rows;
  }
}

export function leadingOverviewRows(
  rows: ProcessViewRow[],
  resource: DetailMode = "cpu",
  limit = 5,
): ProcessViewRow[] {
  if (limit <= 0) return [];
  const seen = new Set<string>();
  return rows
    .filter((row) => {
      if (row.kind === "process" && row.is_grouped) return false;
      if (overviewRankValue(row, resource) < 0) return false;
      const key = processViewRowKey(row);
      if (seen.has(key)) return false;
      seen.add(key);
      return true;
    })
    .sort((left, right) => {
      const leftValue = overviewRankValue(left, resource);
      const rightValue = overviewRankValue(right, resource);
      return (
        rightValue - leftValue || processViewRowKey(left).localeCompare(processViewRowKey(right))
      );
    })
    .slice(0, limit);
}

function overviewRankValue(row: ProcessViewRow, resource: DetailMode): number {
  const metric = resource === "disk" ? "io" : resource;
  const quality =
    row.kind === "group" ? row.detail.quality[metric] : row.detail.process.quality?.[metric];
  if (!metricPresentation(quality, "live", true).canDisplay) return -1;
  if (row.kind === "group" && row.detail.coverage[metric].available === 0) return -1;
  const values = processViewRowMetrics(row);
  return resource === "cpu"
    ? values.cpuPercent
    : resource === "memory"
      ? values.memoryBytes
      : resource === "disk"
        ? values.ioBps
        : values.networkBps;
}

export function overviewMetricValue(row: ProcessViewRow, resource: DetailMode): string {
  const metric = resource === "disk" ? "io" : resource;
  const values = processViewRowMetrics(row);
  const value =
    resource === "cpu"
      ? values.cpuPercent
      : resource === "memory"
        ? values.memoryBytes
        : resource === "disk"
          ? values.ioBps
          : values.networkBps;
  const formatter =
    resource === "cpu" ? formatPercent : resource === "memory" ? formatBytes : formatRate;
  if (row.kind === "group")
    return displayGroupMetricValue(
      value,
      row.detail.quality[metric],
      row.detail.coverage[metric],
      formatter,
    );
  const quality = row.detail.process.quality?.[metric];
  const label = displayProcessMetricValue(value, quality, formatter);
  return quality?.quality === "estimated"
    ? `${label} · estimated`
    : quality?.quality === "partial"
      ? `${label} · limited`
      : label;
}

export function overviewQualityLabel(
  quality: MetricQualityInfo | undefined,
  collectionState: CollectionState,
  hasSample: boolean,
): string | null {
  const metric = metricPresentation(quality, collectionState, hasSample);
  return metric.label === "Current" ? null : metric.label;
}
