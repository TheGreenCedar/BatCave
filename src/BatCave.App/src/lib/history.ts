import type { MetricQualityInfo } from "./types";

export function nextMetricHistory(
  points: number[],
  value: number,
  quality: MetricQualityInfo | undefined,
  maxPoints: number,
): number[] {
  if (quality?.quality === "unavailable" || quality?.quality === "held") {
    return [];
  }

  const nextValue = Number.isFinite(value) ? value : 0;
  return [...points, nextValue].slice(-Math.max(1, maxPoints));
}

export function resourceHistoryWindowLabel(
  pointCount: number,
  intervalMs: number,
  quality: MetricQualityInfo | undefined,
  hasSample: boolean,
): string {
  if (!hasSample || quality?.quality === "unavailable" || quality?.quality === "held") {
    return "No trusted history";
  }
  if (pointCount === 0) return "No history";
  if (pointCount === 1) return "Current sample";
  const seconds = Math.max(1, Math.round(((pointCount - 1) * intervalMs) / 1000));
  if (seconds < 60) return `Last ${seconds}s`;
  const minutes = seconds / 60;
  return `Last ${Number.isInteger(minutes) ? minutes : minutes.toFixed(1)}m`;
}
