import type { MetricQualityInfo, RuntimeSnapshot } from "./types.ts";

export type CollectionState = RuntimeSnapshot["health"]["freshness"];
export type TelemetryTone = "healthy" | "warning" | "danger";

export interface TelemetryPresentation {
  state: CollectionState;
  label: string;
  detail: string;
  tone: TelemetryTone;
}

export const COLLECTION_LIMITED_LABEL = "Collection limited";
const HEALTHY_PRESENTATION = {
  label: "Monitoring",
  detail: "Local measurements are current.",
} as const;

export function isCollectionLimited(presentation: TelemetryPresentation): boolean {
  return presentation.label === COLLECTION_LIMITED_LABEL;
}

/**
 * Latched hysteresis for the noisy "Collection limited" condition: it engages
 * after 3 consecutive limited samples and clears after 5 consecutive samples
 * that are not limited. Callers feed it once per new sample.
 */
export function createCollectionHysteresis(): (limited: boolean) => boolean {
  let limitedStreak = 0;
  let clearStreak = 0;
  let latched = false;
  return (limited: boolean) => {
    if (limited) {
      clearStreak = 0;
      limitedStreak += 1;
      if (limitedStreak >= 3) latched = true;
    } else {
      limitedStreak = 0;
      clearStreak += 1;
      if (clearStreak >= 5) latched = false;
    }
    return latched;
  };
}

/**
 * Applies the latched hysteresis to a raw presentation. Danger, paused, and
 * starting states pass through untouched; only the healthy/limited boundary is
 * smoothed.
 */
export function applyCollectionHysteresis(
  presentation: TelemetryPresentation,
  limitedLatched: boolean,
): TelemetryPresentation {
  if (isCollectionLimited(presentation) && !limitedLatched) {
    return {
      state: presentation.state,
      label: HEALTHY_PRESENTATION.label,
      detail: HEALTHY_PRESENTATION.detail,
      tone: "healthy",
    };
  }
  if (!isCollectionLimited(presentation) && limitedLatched && presentation.tone === "healthy") {
    return {
      state: presentation.state,
      label: COLLECTION_LIMITED_LABEL,
      detail: "Some measurements have limited coverage. Each value carries its own quality.",
      tone: "warning",
    };
  }
  return presentation;
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
    if (transportState === "stale") {
      return {
        state: "stale",
        label: "Monitoring unavailable",
        detail: "The first sample could not be read. Open diagnostics for the failure detail.",
        tone: "danger",
      };
    }
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
      label: COLLECTION_LIMITED_LABEL,
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
    const device = snapshot.environment.platform === "macos" ? "your Mac" : "your computer";
    return {
      state,
      label: "Sampling delayed",
      detail: `Some samples arrived late, usually because ${device} is busy. Values stay labeled by freshness.`,
      tone: "warning",
    };
  }
  if (health.degraded) {
    return {
      state,
      label: "Monitoring limited",
      detail:
        "A monitoring source reported a problem. Open diagnostics to see which measurements are affected.",
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
  const qualityLabel = quality.quality === "partial" ? "Limited" : "Current";
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
