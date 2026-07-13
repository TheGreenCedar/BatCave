import type {
  AccessState,
  KernelPoolKind,
  KernelPoolTag,
  MetricCoverage,
  MetricQuality,
  MetricQualityInfo,
  MetricSource,
  ProcessSample,
  RuntimePlatform,
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

export function formatOptionalRate(value: number | undefined): string {
  return value === undefined ? "Unavailable" : formatRate(value);
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

export function displayMetricValue<T>(
  value: T,
  metric: MetricQualityInfo | undefined,
  sampledAtMs: number | null,
  formatter: (value: T) => string,
): string {
  if (sampledAtMs === null || metric?.quality === "unavailable") {
    return "Unavailable";
  }
  if (metric?.quality === "held") {
    return "Waiting";
  }
  return formatter(value);
}

export function logicalCpuMetricQuality(
  quality: SystemMetricQuality,
): MetricQualityInfo | undefined {
  if (quality.cpu?.quality === "held" || quality.cpu?.quality === "unavailable") {
    return quality.cpu;
  }
  return quality.logical_cpu ?? quality.cpu;
}

export function displayProcessMetricValue<T>(
  value: T,
  metric: MetricQualityInfo | undefined,
  formatter: (value: T) => string,
): string {
  if (!metric) return "Quality not reported";
  if (metric?.quality === "unavailable") return "Unavailable";
  if (metric?.quality === "held") return "Pending";
  return formatter(value);
}

export function processMetricIsPublishable(metric: MetricQualityInfo | undefined): boolean {
  return metric !== undefined && metric.quality !== "unavailable" && metric.quality !== "held";
}

export function nextProcessMetricHistory(
  points: number[],
  value: number,
  quality: MetricQualityInfo | undefined,
  maxPoints: number,
): number[] {
  if (!processMetricIsPublishable(quality)) return [];
  const nextValue = Number.isFinite(value) ? value : 0;
  return [...points, nextValue].slice(-Math.max(1, maxPoints));
}

export function processFindingLabel(
  process: ProcessSample,
  readWriteIoRate: number,
  networkRate: number,
  memoryLabel: string,
): string {
  if (processMetricIsPublishable(process.quality?.cpu) && process.cpu_percent >= 30) {
    return "High CPU usage relative to other workloads.";
  }
  if (
    processMetricIsPublishable(process.quality?.memory) &&
    process.memory_bytes >= 900 * 1024 * 1024
  ) {
    return `High ${memoryLabel.toLocaleLowerCase()} relative to other workloads.`;
  }
  if (processMetricIsPublishable(process.quality?.io) && readWriteIoRate >= 500 * 1024) {
    return "High read/write I/O relative to other workloads.";
  }
  if (processMetricIsPublishable(process.quality?.network) && networkRate >= 1024 * 1024) {
    return "High network activity relative to other workloads.";
  }
  const activityQuality = [
    process.quality?.cpu,
    process.quality?.memory,
    process.quality?.io,
    process.quality?.network,
  ];
  if (activityQuality.some((quality) => quality?.quality === "held")) {
    return "Activity metrics are pending for this workload.";
  }
  if (activityQuality.some((quality) => quality?.quality === "unavailable")) {
    return "Some activity metrics are unavailable for this workload.";
  }
  if (activityQuality.some((quality) => quality === undefined)) {
    return "Some activity metric quality was not reported for this workload.";
  }
  return "No unusual activity is visible for this workload right now.";
}

export function processActivityLabel(
  process: ProcessSample,
  readWriteIoRate: number,
  networkRate: number,
): string {
  if (processMetricIsPublishable(process.quality?.cpu) && process.cpu_percent >= 30) return "Hot";
  if (
    processMetricIsPublishable(process.quality?.memory) &&
    process.memory_bytes >= 900 * 1024 * 1024
  ) {
    return "Heavy";
  }
  if (processMetricIsPublishable(process.quality?.io) && readWriteIoRate >= 500 * 1024) {
    return "I/O";
  }
  if (processMetricIsPublishable(process.quality?.network) && networkRate >= 1024 * 1024) {
    return "Network";
  }
  const activityQuality = [
    process.quality?.cpu,
    process.quality?.memory,
    process.quality?.io,
    process.quality?.network,
  ];
  if (activityQuality.some((quality) => quality?.quality === "held")) return "Pending";
  if (activityQuality.some((quality) => quality?.quality === "unavailable")) return "Unavailable";
  if (activityQuality.some((quality) => quality === undefined)) return "Quality not reported";
  return "Normal";
}

export function processTrustLabel(process: ProcessSample): string {
  const qualities = [
    process.quality?.cpu,
    process.quality?.memory,
    process.quality?.io,
    process.quality?.other_io,
    process.quality?.network,
    process.quality?.threads,
    process.quality?.handles,
  ];
  if (qualities.some((quality) => quality === undefined)) return "Quality not reported";
  const reported = qualities as MetricQualityInfo[];
  if (reported.some((quality) => quality.quality === "held")) return "Pending coverage";
  if (reported.every((quality) => quality.quality === "unavailable")) return "Unavailable";
  if (reported.every((quality) => quality.quality === "native")) return "Native";
  return "Partial coverage";
}

export function groupMetricCanDisplay(
  metric: MetricQualityInfo | undefined,
  coverage: MetricCoverage | undefined,
): boolean {
  return (
    !!metric &&
    !!coverage &&
    coverage.available > 0 &&
    metric.quality !== "held" &&
    metric.quality !== "unavailable"
  );
}

export function displayGroupMetricValue<T>(
  value: T,
  metric: MetricQualityInfo | undefined,
  coverage: MetricCoverage | undefined,
  formatter: (value: T) => string,
): string {
  if (!metric || !coverage || (metric.quality === "partial" && coverage.available === 0)) {
    return "Limited";
  }
  if (metric.quality === "held") {
    return "Pending";
  }
  if (metric.quality === "unavailable") {
    return "Unavailable";
  }
  if (coverage.available === 0) {
    return "Limited";
  }
  const formatted = formatter(value);
  if (metric.quality === "partial" || coverage.available < coverage.total) {
    return `${formatted} · ${coverage.available}/${coverage.total} · limited`;
  }
  return metric.quality === "estimated" ? `${formatted} · estimated` : formatted;
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
  return processMetricIsPublishable(processMemoryQuality(process));
}

export function processBytesLabel(process: ProcessSample, value: number): string {
  return displayProcessMetricValue(value, processMemoryQuality(process), formatBytes);
}

export function processPrivateMemoryValue(
  process: ProcessSample,
  platform: RuntimePlatform,
): string {
  if (platform === "macos") {
    const quality = processMemoryQuality(process);
    if (!processMetricIsPublishable(quality)) {
      return displayProcessMetricValue(process.private_bytes, quality, formatBytes);
    }
    return quality?.quality === "native" && process.private_bytes > 0
      ? formatBytes(process.private_bytes)
      : "Unavailable";
  }
  return processBytesLabel(process, process.private_bytes);
}

export function processMemoryTitle(process: ProcessSample): string {
  const quality = processMemoryQuality(process);
  if (quality?.message) {
    return quality.message;
  }

  return metricQualityLabel(quality, "Quality not reported");
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
