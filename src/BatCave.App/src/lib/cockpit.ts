import type { DetailMode } from "./components/metrics/types";
import type { MetricQualityInfo, ProcessSample, RuntimeSnapshot } from "./types";

export type CollectionState = "live" | "paused" | "stale";
export type ResourceConfidence = "High" | "Limited" | "Unavailable";

export interface ResourceBrief {
  mode: DetailMode;
  label: string;
  semanticLabel: string;
  value: number;
  valueLabel: string;
  headline: string;
  stateLabel: string;
  confidence: ResourceConfidence;
  leadingWorkload: string | null;
  leadingValueLabel: string;
  attributionLabel: string;
}

export interface ResourceValues {
  memoryPercent: number;
  diskRate: number;
  networkRate: number;
}

export function buildResourceBrief(
  snapshot: RuntimeSnapshot,
  mode: DetailMode,
  values: ResourceValues,
  collectionState: CollectionState,
): ResourceBrief {
  const definition = resourceDefinition(snapshot, mode, values);
  const quality = resourceQuality(snapshot, mode);
  const hasSample = snapshot.sampled_at_ms !== null;
  const canShowValue =
    hasSample && quality?.quality !== "unavailable" && quality?.quality !== "held";
  const stateLabel = resourceStateLabel(snapshot, quality, collectionState, hasSample);
  const valueLabel = canShowValue ? definition.valueLabel : unavailableValueLabel(quality);
  const headline = resourceHeadline(
    definition.semanticLabel,
    valueLabel,
    canShowValue,
    collectionState,
  );
  const contributor = canShowValue ? definition.contributor : null;
  const leadingProcess = contributor
    ? snapshot.processes.find((process) => process.name === contributor)
    : undefined;

  return {
    mode,
    label: definition.label,
    semanticLabel: definition.semanticLabel,
    value: definition.value,
    valueLabel,
    headline,
    stateLabel,
    confidence: resourceConfidence(snapshot, quality, collectionState, canShowValue),
    leadingWorkload: contributor ? displayProcessName(contributor) : null,
    leadingValueLabel: contributorValueLabel(mode, leadingProcess),
    attributionLabel: definition.attributionLabel,
  };
}

function resourceDefinition(
  snapshot: RuntimeSnapshot,
  mode: DetailMode,
  values: ResourceValues,
): {
  label: string;
  semanticLabel: string;
  value: number;
  valueLabel: string;
  contributor: string | null;
  attributionLabel: string;
} {
  switch (mode) {
    case "cpu":
      return {
        label: "Machine CPU",
        semanticLabel: "Machine-total CPU",
        value: snapshot.system.cpu_percent,
        valueLabel: formatPercent(snapshot.system.cpu_percent),
        contributor: snapshot.process_contributors.cpu,
        attributionLabel:
          "Process CPU is one-core-equivalent; machine CPU is total capacity across logical cores.",
      };
    case "memory":
      return {
        label: "Memory",
        semanticLabel: "Physical memory use",
        value: values.memoryPercent,
        valueLabel: formatPercent(values.memoryPercent),
        contributor: snapshot.process_contributors.memory,
        attributionLabel:
          "Process resident-memory values are ranked independently; they are not added up to reconcile physical memory.",
      };
    case "disk":
      return {
        label: "Disk",
        semanticLabel: "Physical disk throughput",
        value: values.diskRate,
        valueLabel: formatRate(values.diskRate),
        contributor: null,
        attributionLabel: "Process read/write I/O is not used as physical-disk attribution.",
      };
    case "network":
      return {
        label: "Network",
        semanticLabel: "Interface network throughput",
        value: values.networkRate,
        valueLabel: formatRate(values.networkRate),
        contributor: snapshot.process_contributors.network,
        attributionLabel:
          "Process network traffic is attributed independently from interface totals.",
      };
  }
}

function resourceQuality(
  snapshot: RuntimeSnapshot,
  mode: DetailMode,
): MetricQualityInfo | undefined {
  return snapshot.system.quality?.[mode];
}

function resourceStateLabel(
  snapshot: RuntimeSnapshot,
  quality: MetricQualityInfo | undefined,
  collectionState: CollectionState,
  hasSample: boolean,
): string {
  if (!hasSample) return "No sample";
  if (collectionState === "stale") return "Stale";
  if (collectionState === "paused") return "Paused";
  if (quality?.quality === "unavailable") return "Unavailable";
  if (quality?.quality === "held") return "Waiting";
  if (quality?.quality === "partial") return "Partial";
  if (snapshot.health.degraded || snapshot.warnings.length > 0) return "Degraded";
  if (quality?.quality === "estimated") return "Estimated";
  return "Current";
}

function resourceConfidence(
  snapshot: RuntimeSnapshot,
  quality: MetricQualityInfo | undefined,
  collectionState: CollectionState,
  canShowValue: boolean,
): ResourceConfidence {
  if (!canShowValue) return "Unavailable";
  if (
    collectionState !== "live" ||
    quality?.quality !== "native" ||
    snapshot.health.degraded ||
    snapshot.warnings.length > 0
  ) {
    return "Limited";
  }
  return "High";
}

function unavailableValueLabel(quality: MetricQualityInfo | undefined): string {
  return quality?.quality === "held" ? "Waiting" : "Unavailable";
}

function resourceHeadline(
  semanticLabel: string,
  valueLabel: string,
  canShowValue: boolean,
  collectionState: CollectionState,
): string {
  if (!canShowValue) return `${semanticLabel} has no trusted sample.`;
  if (collectionState === "paused")
    return `${semanticLabel} was ${valueLabel} when collection paused.`;
  if (collectionState === "stale") {
    return `${semanticLabel} was ${valueLabel} in the last successful sample.`;
  }
  return `${semanticLabel} is ${valueLabel}.`;
}

function contributorValueLabel(mode: DetailMode, process: ProcessSample | undefined): string {
  if (!process) return "Contributor value is outside the current workload view";
  switch (mode) {
    case "cpu":
      return `${formatPercent(process.cpu_percent)} of one core`;
    case "memory":
      return `${formatBytes(process.memory_bytes)} resident memory`;
    case "network":
      return `${formatRate(
        (process.network_received_bps ?? 0) + (process.network_transmitted_bps ?? 0),
      )} process traffic`;
    case "disk":
      return "No compatible process attribution";
  }
}

export function displayProcessName(name: string): string {
  const normalized = name.replace(/\.exe$/i, "");
  if (normalized.toLocaleLowerCase() === "code") return "Visual Studio Code";
  if (normalized.toLocaleLowerCase() === "msedge") return "Microsoft Edge";
  return normalized;
}

function formatPercent(value: number): string {
  return `${Math.round(value)}%`;
}

function formatRate(value: number): string {
  return `${formatBytes(value)}/s`;
}

function formatBytes(value: number): string {
  const units = ["B", "KB", "MB", "GB", "TB"];
  let amount = Math.max(0, value);
  let unit = 0;
  while (amount >= 1024 && unit < units.length - 1) {
    amount /= 1024;
    unit += 1;
  }
  return `${amount >= 10 || unit === 0 ? amount.toFixed(0) : amount.toFixed(1)} ${units[unit]}`;
}
