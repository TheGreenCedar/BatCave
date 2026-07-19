import { processViewRowKey } from "./process.ts";
import type {
  MetricQualityInfo,
  ProcessViewRow,
  RuntimePlatform,
  RuntimeSnapshot,
} from "./types.ts";

export type OverviewTone = "neutral" | "healthy" | "warning" | "danger";
export type OverviewCollectionState = "starting" | "live" | "paused" | "stale";

export interface OverviewAttention {
  title: string;
  detail: string;
  tone: "warning" | "danger";
}

export interface OverviewStatus {
  headline: string;
  summary: string;
  tone: OverviewTone;
  attention: OverviewAttention | null;
  pressuredResource: "cpu" | "memory" | null;
}

export function buildOverviewStatus(
  snapshot: RuntimeSnapshot,
  collectionState: OverviewCollectionState,
  limitationCount: number,
): OverviewStatus {
  const machine = machineLabel(snapshot.environment.platform);
  const hasSample = snapshot.sampled_at_ms !== null;

  if (collectionState === "starting" || !hasSample) {
    return {
      headline: "BatCave is getting ready.",
      summary: "Waiting for the first local system sample.",
      tone: "neutral",
      attention: null,
      pressuredResource: null,
    };
  }

  if (collectionState === "stale") {
    return {
      headline: "BatCave is showing the last sample.",
      summary: `Monitoring for this ${machine} is temporarily unavailable.`,
      tone: "danger",
      attention: {
        title: "Data is stale",
        detail: "Current values may have changed since the last successful sample.",
        tone: "danger",
      },
      pressuredResource: null,
    };
  }

  if (collectionState === "paused") {
    return {
      headline: "Monitoring is paused.",
      summary: "Values and charts remain at the last local sample until monitoring resumes.",
      tone: "warning",
      attention: {
        title: "Live updates are paused",
        detail: "Resume monitoring when you want BatCave to collect new samples.",
        tone: "warning",
      },
      pressuredResource: null,
    };
  }

  const memoryPercent = percentage(
    snapshot.system.memory_used_bytes,
    snapshot.system.memory_total_bytes,
  );
  const cpuPressured =
    qualityCanSupportPressure(snapshot.system.quality?.cpu) && snapshot.system.cpu_percent >= 85;
  const memoryPressured =
    qualityCanSupportPressure(snapshot.system.quality?.memory) && memoryPercent >= 85;

  if (cpuPressured || memoryPressured) {
    const pressuredResource = cpuPressured ? "cpu" : "memory";
    const value = cpuPressured ? snapshot.system.cpu_percent : memoryPercent;
    const label = cpuPressured ? "CPU" : "Memory";
    return {
      headline: `Your ${machine} is under pressure.`,
      summary: `${label} is at ${Math.round(value)}%. Open Explore to inspect the leading workloads.`,
      tone: "warning",
      attention: {
        title: `${label} needs attention`,
        detail: `${label} crossed BatCave's current ${label.toLocaleLowerCase()} pressure threshold.`,
        tone: "warning",
      },
      pressuredResource,
    };
  }

  if (snapshot.health.degraded) {
    return {
      headline: "BatCave is using more resources than expected.",
      summary: `System measurements for this ${machine} remain available.`,
      tone: "warning",
      attention: {
        title: "Monitor overhead is elevated",
        detail: "BatCave's own CPU or memory use is above its internal budget.",
        tone: "warning",
      },
      pressuredResource: null,
    };
  }

  if (limitationCount > 0) {
    return {
      headline: `Your ${machine}'s main signals look normal.`,
      summary: "Some measurements are limited and are marked where they appear.",
      tone: "warning",
      attention: {
        title: `${limitationCount} data limitation${limitationCount === 1 ? "" : "s"}`,
        detail: "Available measurements remain current; unavailable fields are not estimated.",
        tone: "warning",
      },
      pressuredResource: null,
    };
  }

  return {
    headline: `Your ${machine} is running normally.`,
    summary: "CPU and memory are within BatCave's current pressure thresholds.",
    tone: "healthy",
    attention: null,
    pressuredResource: null,
  };
}

export function leadingOverviewRows(rows: ProcessViewRow[], limit = 5): ProcessViewRow[] {
  const seen = new Set<string>();
  const leading: ProcessViewRow[] = [];

  for (const row of rows) {
    if (row.kind === "process" && row.is_grouped) continue;
    const key = processViewRowKey(row);
    if (seen.has(key)) continue;
    seen.add(key);
    leading.push(row);
    if (leading.length >= Math.max(0, limit)) break;
  }

  return leading;
}

export function overviewQualityLabel(
  quality: MetricQualityInfo | undefined,
  collectionState: OverviewCollectionState,
  hasSample: boolean,
): string | null {
  if (!hasSample) return "No sample";
  if (collectionState === "stale") return "Stale";
  if (collectionState === "paused") return "Paused";
  if (quality?.quality === "estimated") return "Estimated";
  if (quality?.quality === "partial") return "Limited";
  if (quality?.quality === "held") return "Waiting";
  if (quality?.quality === "unavailable") return "Unavailable";
  return null;
}

function qualityCanSupportPressure(quality: MetricQualityInfo | undefined): boolean {
  return quality?.quality !== "unavailable" && quality?.quality !== "held";
}

function machineLabel(platform: RuntimePlatform): string {
  if (platform === "macos") return "Mac";
  if (platform === "windows") return "PC";
  return "machine";
}

function percentage(used: number, total: number): number {
  if (total <= 0) return 0;
  return Math.max(0, Math.min(100, (used / total) * 100));
}
