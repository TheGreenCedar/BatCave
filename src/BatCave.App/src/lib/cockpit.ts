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
  contributorStatusLabel: string;
  contributorNameAmbiguous: boolean;
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
  const contributor =
    canShowValue && contributorQualityIsPublishable(definition.contributorQuality)
      ? definition.contributor
      : null;
  const matchingProcesses = contributor
    ? snapshot.processes.filter((process) => process.name === contributor)
    : [];
  const contributorNameAmbiguous =
    definition.contributorNameAmbiguous || matchingProcesses.length > 1;
  const leadingProcess =
    !contributorNameAmbiguous && matchingProcesses.length === 1 ? matchingProcesses[0] : undefined;

  return {
    mode,
    label: definition.label,
    semanticLabel: definition.semanticLabel,
    value: definition.value,
    valueLabel,
    headline,
    stateLabel,
    confidence: resourceConfidence(
      snapshot,
      mode,
      quality,
      definition.contributor,
      definition.contributorQuality,
      contributorNameAmbiguous,
      collectionState,
      canShowValue,
    ),
    leadingWorkload: contributor ? displayProcessName(contributor) : null,
    contributorStatusLabel: contributorStatusLabel(
      mode,
      contributor,
      leadingProcess,
      definition.contributorQuality,
      contributorNameAmbiguous,
      snapshot.total_process_count,
      quality,
      hasSample,
    ),
    contributorNameAmbiguous,
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
  contributorQuality?: MetricQualityInfo;
  contributorNameAmbiguous: boolean;
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
        contributorQuality: snapshot.process_contributors.cpu_quality,
        contributorNameAmbiguous: snapshot.process_contributors.cpu_name_ambiguous !== false,
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
        contributorQuality: snapshot.process_contributors.memory_quality,
        contributorNameAmbiguous: snapshot.process_contributors.memory_name_ambiguous !== false,
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
        contributorQuality: undefined,
        contributorNameAmbiguous: false,
        attributionLabel: "Process read/write I/O is not used as physical-disk attribution.",
      };
    case "network":
      return {
        label: "Network",
        semanticLabel: "Interface network throughput",
        value: values.networkRate,
        valueLabel: formatRate(values.networkRate),
        contributor: snapshot.process_contributors.network,
        contributorQuality: snapshot.process_contributors.network_quality,
        contributorNameAmbiguous: snapshot.process_contributors.network_name_ambiguous !== false,
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
  mode: DetailMode,
  quality: MetricQualityInfo | undefined,
  contributor: string | null,
  contributorQuality: MetricQualityInfo | undefined,
  contributorNameAmbiguous: boolean,
  collectionState: CollectionState,
  canShowValue: boolean,
): ResourceConfidence {
  if (!canShowValue) return "Unavailable";
  if (
    collectionState !== "live" ||
    quality?.quality !== "native" ||
    (mode !== "disk" && snapshot.total_process_count > 0 && contributorQuality === undefined) ||
    (contributor !== null && contributorQuality?.quality !== "native") ||
    contributorNameAmbiguous ||
    contributorQuality?.quality === "estimated" ||
    contributorQuality?.quality === "partial" ||
    contributorQuality?.quality === "held" ||
    contributorQuality?.quality === "unavailable" ||
    snapshot.health.degraded ||
    snapshot.warnings.length > 0
  ) {
    return "Limited";
  }
  return "High";
}

function contributorStatusLabel(
  mode: DetailMode,
  contributor: string | null,
  process: ProcessSample | undefined,
  quality: MetricQualityInfo | undefined,
  contributorNameAmbiguous: boolean,
  totalProcessCount: number,
  systemQuality: MetricQualityInfo | undefined,
  hasSystemSample: boolean,
): string {
  if (mode === "disk") return "No compatible process attribution";
  if (!hasSystemSample) return "Process attribution unavailable without a system sample";
  if (systemQuality?.quality === "unavailable") return "Process attribution unavailable";
  if (systemQuality?.quality === "held") return "Process attribution pending";
  if (contributor) {
    return contributorValueLabel(mode, process, quality, contributorNameAmbiguous);
  }
  if (totalProcessCount === 0) return "No processes in this sample";
  if (!quality) return "Attribution quality not reported";
  if (quality.quality === "unavailable") return "Process attribution unavailable";
  if (quality.quality === "held") return "Process attribution pending";
  return `No process activity attributed${contributorQualitySuffix(quality)}`;
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

function contributorValueLabel(
  mode: DetailMode,
  process: ProcessSample | undefined,
  quality: MetricQualityInfo | undefined,
  contributorNameAmbiguous: boolean,
): string {
  const qualitySuffix = contributorQualitySuffix(quality);
  if (contributorNameAmbiguous) {
    return `Contributor name is ambiguous across the full process sample${qualitySuffix}`;
  }
  if (!process) return `Contributor value is outside the current workload view${qualitySuffix}`;
  let valueLabel: string;
  switch (mode) {
    case "cpu":
      valueLabel = `${formatPercent(process.cpu_percent)} of one core`;
      break;
    case "memory":
      valueLabel = `${formatBytes(process.memory_bytes)} resident memory`;
      break;
    case "network":
      valueLabel = `${formatRate(
        (process.network_received_bps ?? 0) + (process.network_transmitted_bps ?? 0),
      )} process traffic`;
      break;
    case "disk":
      valueLabel = "No compatible process attribution";
      break;
  }
  return `${valueLabel}${qualitySuffix}`;
}

function contributorQualityIsPublishable(quality: MetricQualityInfo | undefined): boolean {
  return quality?.quality !== "unavailable" && quality?.quality !== "held";
}

function contributorQualitySuffix(quality: MetricQualityInfo | undefined): string {
  if (!quality) return " · Attribution quality not reported";
  if (quality.quality === "estimated") return " · Estimated attribution";
  if (quality.quality === "partial") return " · Partial attribution";
  return "";
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
