import type {
  AccessState,
  KernelPoolKind,
  KernelPoolTag,
  MetricQuality,
  MetricQualityInfo,
  MetricSource,
  ProcessSample,
  SystemMetricQuality,
} from "./types";

export function formatBytes(value: number): string {
  const units = ["B", "KB", "MB", "GB", "TB"];
  let amount = Math.max(0, value);
  let unit = 0;

  while (amount >= 1024 && unit < units.length - 1) {
    amount /= 1024;
    unit += 1;
  }

  return `${amount >= 10 || unit === 0 ? amount.toFixed(0) : amount.toFixed(1)} ${units[unit]}`;
}

export function formatRate(value: number): string {
  return `${formatBytes(value)}/s`;
}

export function formatPercent(value: number): string {
  return `${Math.round(value)}%`;
}

export function formatInterval(value: number): string {
  return value < 1000 ? `${value} ms` : `${value / 1000}s`;
}

export function metricQualityLabel(
  metric: MetricQualityInfo | undefined,
  fallback: string,
): string {
  if (!metric) {
    return fallback;
  }

  const quality = formatMetricQuality(metric.quality);
  const source = metric.source ? formatMetricSource(metric.source) : "";
  return source ? `${quality} / ${source}` : quality;
}

/**
 * Returns the compact quality vocabulary used in high-density summaries.
 * The detailed, source-aware label remains available through metricQualityLabel.
 */
export function metricQualityShortLabel(
  metric: MetricQualityInfo | undefined,
  fallback: string,
): string {
  if (!metric) {
    return fallback;
  }

  switch (metric.quality) {
    case "native":
      return "Native";
    case "partial":
      return "Partial";
    case "estimated":
      return "Estimated";
    case "held":
      return "Held";
    case "unavailable":
      return "Unavailable";
    default:
      return fallback;
  }
}

export function metricQualityAction(metric: MetricQualityInfo | undefined): string {
  if (!metric) {
    return "";
  }

  if (metric.message) {
    return metric.message;
  }

  if (metric.quality === "held") {
    return "waiting for sample";
  }

  if (metric.quality === "partial") {
    return "fallback/incomplete source";
  }

  if (metric.quality === "unavailable") {
    return "unavailable/permissions";
  }

  return "";
}

export function qualityGuidance(quality: SystemMetricQuality): string[] {
  return [quality.cpu, quality.disk, quality.network]
    .map(metricQualityAction)
    .filter((guidance) => guidance.length > 0);
}

export function formatMetricQuality(value: MetricQuality): string {
  switch (value) {
    case "native":
      return "Native";
    case "estimated":
      return "Estimated";
    case "held":
      return "Held";
    case "partial":
      return "Partial";
    case "unavailable":
      return "Unavailable";
    default:
      return value;
  }
}

export function formatMetricSource(value: MetricSource): string {
  switch (value) {
    case "direct_api":
      return "direct API";
    case "interface_aggregate":
      return "interface aggregate";
    case "process_aggregate":
      return "process aggregate";
    case "ebpf":
      return "eBPF";
    default:
      return value.replaceAll("_", " ");
  }
}

export function accessLabel(access: AccessState): string {
  if (access === "full") {
    return "Full";
  }

  return access === "partial" ? "Partial" : "Denied";
}

export function processMemoryQuality(process: ProcessSample): MetricQualityInfo | undefined {
  return process.quality?.memory;
}

export function processMemoryIsReported(process: ProcessSample): boolean {
  return processMemoryQuality(process)?.quality !== "unavailable";
}

export function processBytesLabel(process: ProcessSample, value: number): string {
  return processMemoryIsReported(process) ? formatBytes(value) : "Blocked";
}

export function processMemoryTitle(process: ProcessSample): string {
  const quality = processMemoryQuality(process);
  if (quality?.message) {
    return quality.message;
  }

  return processMemoryIsReported(process)
    ? metricQualityLabel(quality, "Measured")
    : "Process memory was not reported by this collector.";
}

export function optionalBytes(value: number | undefined): string {
  return value === undefined ? "--" : formatBytes(value);
}

export function poolKindLabel(kind: KernelPoolKind): string {
  return kind === "nonpaged" ? "Nonpaged" : "Paged";
}

export function driverCandidateLabel(tag: KernelPoolTag): string {
  if (tag.driver_candidates_pending) {
    return "Checking local drivers";
  }

  return tag.driver_candidates.length > 0 ? tag.driver_candidates.join(", ") : "No local candidate";
}

export function poolTagKey(tag: KernelPoolTag): string {
  return `${tag.tag}:${tag.kind}`;
}
