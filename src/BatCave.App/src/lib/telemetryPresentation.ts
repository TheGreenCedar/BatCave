import type { MetricQualityInfo, RuntimeSnapshot } from "./types.ts";

export type CollectionState = RuntimeSnapshot["health"]["freshness"];
export type TelemetryTone = "healthy" | "warning" | "danger";

export interface TelemetryPresentation {
  state: CollectionState;
  label: string;
  detail: string;
  tone: TelemetryTone;
}

/** Runtime freshness owns sample age; transport failure can only make it less current. */
export function buildTelemetryPresentation(
  snapshot: RuntimeSnapshot,
  transportState: CollectionState,
): TelemetryPresentation {
  const health = snapshot.health;
  const reasons = new Set(health.reason_codes);
  if (health.engine_state === "fatal" || health.fatal_error || reasons.has("engine_fatal")) {
    return {
      state: "stale",
      label: "Monitoring stopped",
      detail:
        health.fatal_error?.message ??
        "The monitoring engine stopped. Open diagnostics for details.",
      tone: "danger",
    };
  }
  const state = transportState === "stale" ? "stale" : health.freshness;
  if (state === "paused" || (transportState === "paused" && state !== "stale")) {
    return {
      state: "paused",
      label: "Monitoring paused",
      detail:
        snapshot.sampled_at_ms === null
          ? "Monitoring paused before the first sample. Resume to collect measurements."
          : "Values remain at the last sample until monitoring resumes.",
      tone: "warning",
    };
  }
  if (snapshot.sampled_at_ms === null || transportState === "starting") {
    return {
      state: "starting",
      label: "Starting monitoring",
      detail: "Waiting for the first local system sample.",
      tone: "warning",
    };
  }
  if (state === "stale" || health.collector_state === "unavailable") {
    return {
      state: "stale",
      label: "Last sample",
      detail: "Measurements are stale. Values may have changed since collection succeeded.",
      tone: "danger",
    };
  }
  if (state === "starting") {
    return {
      state,
      label: "Starting monitoring",
      detail: "Waiting for current measurements.",
      tone: "warning",
    };
  }
  if (reasons.has("runtime_cpu_budget") || reasons.has("runtime_memory_budget")) {
    return {
      state,
      label: "Monitor resource warning",
      detail: reasons.has("runtime_cpu_budget")
        ? "BatCave's own CPU use is above its collection budget."
        : "BatCave's own memory use is above its collection budget.",
      tone: "warning",
    };
  }
  if (
    health.collector_state === "limited" ||
    reasons.has("collector_limited") ||
    reasons.has("collector_warning")
  ) {
    return {
      state,
      label: "Collection limited",
      detail: "Some measurements have limited coverage. Each value carries its own quality.",
      tone: "warning",
    };
  }
  if (snapshot.persistence?.state === "unavailable" || reasons.has("persistence_unavailable")) {
    return {
      state,
      label: "Local storage unavailable",
      detail: "Monitoring is current, but settings or cached telemetry may not be saved.",
      tone: "warning",
    };
  }
  if (snapshot.persistence?.state === "degraded" || reasons.has("persistence_degraded")) {
    return {
      state,
      label: "Local storage limited",
      detail: "Monitoring is current, but a local save operation failed.",
      tone: "warning",
    };
  }
  if (reasons.has("cadence_missed")) {
    return {
      state,
      label: "Sampling delayed",
      detail:
        "The monitor missed a collection deadline. Available measurements remain labeled by freshness.",
      tone: "warning",
    };
  }
  if (health.degraded) {
    return {
      state,
      label: "Monitoring limited",
      detail: "The runtime reported a monitoring issue. Open diagnostics for details.",
      tone: "warning",
    };
  }
  return { state, label: "Monitoring", detail: "Local measurements are current.", tone: "healthy" };
}

export interface MetricPresentation {
  canDisplay: boolean;
  label: string;
  emptyLabel: string;
}

export function metricPresentation(
  quality: MetricQualityInfo | undefined,
  state: CollectionState,
  hasSample: boolean,
): MetricPresentation {
  if (!hasSample || state === "starting")
    return { canDisplay: false, label: "No sample", emptyLabel: "No sample" };
  if (!quality)
    return { canDisplay: false, label: "Quality not reported", emptyLabel: "Quality not reported" };
  if (quality.quality === "unavailable")
    return { canDisplay: false, label: "Unavailable", emptyLabel: "Unavailable" };
  if (quality.quality === "held")
    return { canDisplay: false, label: "Pending", emptyLabel: "Pending" };
  const qualityLabel =
    quality.quality === "partial"
      ? "Limited"
      : quality.quality === "estimated"
        ? "Estimated"
        : "Current";
  const freshness = state === "stale" ? "Stale" : state === "paused" ? "Paused" : null;
  return {
    canDisplay: true,
    label: freshness
      ? qualityLabel === "Current"
        ? freshness
        : `${freshness} · ${qualityLabel.toLocaleLowerCase()}`
      : qualityLabel,
    emptyLabel: "Unavailable",
  };
}
