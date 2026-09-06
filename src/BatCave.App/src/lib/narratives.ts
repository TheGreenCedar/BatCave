export type NarrativeProvider = "apple_foundation" | "foundry_local";
export type NarrativeSurface = "overview_contributor" | "workload_insight";
export type NarrativeAvailability =
  | "available"
  | "unsupported"
  | "model_not_ready"
  | "runtime_missing"
  | "busy";

export interface NarrativeRequest {
  surface: NarrativeSurface;
  publication_seq: number;
  subject_stable_id?: string;
  fact_digest: string;
}

export type NarrativeExplanationId =
  | "cpu_usage"
  | "memory_usage"
  | "disk_activity"
  | "network_activity";

export interface NarrativeResult {
  provider: NarrativeProvider;
  publication_seq: number;
  fact_digest: string;
  surface: NarrativeSurface;
  subject_stable_id?: string;
  explanation_id: NarrativeExplanationId;
}

export interface NarrativeMetricFact {
  kind: "cpu" | "memory" | "io" | "network";
  rounded_value: number;
  unit: "percent" | "megabytes" | "kilobytes_per_second";
}

/**
 * This is the complete privacy boundary for local narrative generation. Keep it
 * deliberately boring: no executable path, process ID, collector detail, raw
 * diagnostic, or information about any other process belongs here.
 */
export interface NarrativeFactPacket {
  display_name: string;
  category: string;
  metrics: NarrativeMetricFact[];
  leading_resource?: NarrativeMetricFact["kind"];
  ranking_state: "top_contributor" | "leading" | "notable" | "normal";
  measurement_limitations: NarrativeMeasurementLimitation[];
}

export interface NarrativeMeasurementLimitation {
  kind: NarrativeMetricFact["kind"];
  quality: "estimated" | "limited" | "stale" | "unavailable";
}

export interface NarrativeInvocation {
  request: NarrativeRequest;
  facts: NarrativeFactPacket;
}

export interface NarrativeCapability {
  provider: NarrativeProvider;
  availability: NarrativeAvailability;
  model_id?: string;
  model_name?: string;
  download_state: "not_required" | "not_downloaded" | "downloading" | "ready" | "failed";
  download_size_bytes?: number;
  downloaded_bytes?: number;
  license_name?: string;
  license_url?: string;
  can_download: boolean;
  can_cancel_download: boolean;
  detail_code?: string;
}

export interface NarrativePreferences {
  enhanced_narratives: boolean;
}

export const defaultNarrativeCapability: NarrativeCapability = {
  provider: "foundry_local",
  availability: "unsupported",
  download_state: "not_downloaded",
  can_download: false,
  can_cancel_download: false,
};

export interface BuildNarrativeFactPacketInput {
  displayName: string;
  category: string;
  cpuPercent: number;
  memoryBytes: number;
  ioBytesPerSecond: number;
  networkBytesPerSecond: number;
  leadingResource?: NarrativeMetricFact["kind"];
  rankingState: NarrativeFactPacket["ranking_state"];
  measurementLimitations?: Iterable<NarrativeMeasurementLimitation>;
}

export function buildNarrativeFactPacket(
  input: BuildNarrativeFactPacketInput,
): NarrativeFactPacket {
  return {
    display_name: cleanFactText(input.displayName, "Unknown workload", 120),
    category: cleanFactText(input.category, "Process", 80),
    metrics: [
      {
        kind: "cpu",
        rounded_value: roundNumber(input.cpuPercent, 1),
        unit: "percent",
      },
      {
        kind: "memory",
        rounded_value: roundNumber(input.memoryBytes / 1024 ** 2, 0),
        unit: "megabytes",
      },
      {
        kind: "io",
        rounded_value: roundNumber(input.ioBytesPerSecond / 1024, 0),
        unit: "kilobytes_per_second",
      },
      {
        kind: "network",
        rounded_value: roundNumber(input.networkBytesPerSecond / 1024, 0),
        unit: "kilobytes_per_second",
      },
    ],
    ...(input.leadingResource ? { leading_resource: input.leadingResource } : {}),
    ranking_state: input.rankingState,
    measurement_limitations: deduplicateLimitations(input.measurementLimitations ?? []),
  };
}

export function narrativeFactDigest(facts: NarrativeFactPacket): string {
  return hashNarrativeValue(facts);
}

/**
 * Only the resource selection is cached. Wording, numbers, and qualifications
 * are rendered from current facts; eligibility and quality changes invalidate it.
 */
export function narrativeRelevanceKey(facts: NarrativeFactPacket): string {
  return hashNarrativeValue({
    display_name: facts.display_name,
    category: facts.category,
    leading_resource: facts.leading_resource ?? null,
    ranking_state: facts.ranking_state,
    measurement_limitations: facts.measurement_limitations,
    candidate_ids: admittedNarrativeCandidates(facts),
  });
}

function hashNarrativeValue(value: unknown): string {
  const serialized = JSON.stringify(value);
  let hash = 0xcbf29ce484222325n;
  for (let index = 0; index < serialized.length; index += 1) {
    hash ^= BigInt(serialized.charCodeAt(index));
    hash = BigInt.asUintN(64, hash * 0x100000001b3n);
  }
  return `fnv1a64:${hash.toString(16).padStart(16, "0")}`;
}

export function makeNarrativeInvocation(
  surface: NarrativeSurface,
  publicationSeq: number,
  facts: NarrativeFactPacket,
  subjectStableId?: string,
  factDigest = narrativeFactDigest(facts),
): NarrativeInvocation {
  return {
    request: {
      surface,
      publication_seq: publicationSeq,
      ...(subjectStableId ? { subject_stable_id: subjectStableId } : {}),
      fact_digest: factDigest,
    },
    facts,
  };
}

export function narrativeCapabilityExplanation(capability: NarrativeCapability): string {
  if (capability.availability === "available") {
    return "Ready to choose among explanations supported by local measurements.";
  }
  if (capability.availability === "model_not_ready") {
    return capability.can_download
      ? "The optional local model has not been downloaded."
      : "The local model is not ready.";
  }
  if (capability.availability === "runtime_missing") {
    return "The local model runtime is not installed.";
  }
  if (capability.availability === "busy") {
    return "The local model is busy. Deterministic explanations remain active.";
  }
  return "Enhanced explanations are not supported on this system.";
}

export interface AcceptedNarrative extends NarrativeResult {
  relevance_key: string;
}

export function validateNarrativeResult(
  invocation: NarrativeInvocation,
  value: NarrativeResult,
): AcceptedNarrative | null {
  if (
    value.publication_seq !== invocation.request.publication_seq ||
    value.fact_digest !== invocation.request.fact_digest ||
    value.surface !== invocation.request.surface ||
    value.subject_stable_id !== invocation.request.subject_stable_id ||
    !["apple_foundation", "foundry_local"].includes(value.provider) ||
    !admittedNarrativeCandidates(invocation.facts, invocation.request.surface).includes(
      value.explanation_id,
    )
  ) {
    return null;
  }
  return { ...value, relevance_key: narrativeRelevanceKey(invocation.facts) };
}

export function isNarrativeExplanationId(value: unknown): value is NarrativeExplanationId {
  return (
    value === "cpu_usage" ||
    value === "memory_usage" ||
    value === "disk_activity" ||
    value === "network_activity"
  );
}

const explanationResource: Record<NarrativeExplanationId, NarrativeMetricFact["kind"]> = {
  cpu_usage: "cpu",
  memory_usage: "memory",
  disk_activity: "io",
  network_activity: "network",
};

/** Mirrors native admission for display invalidation; Rust remains the generation authority. */
export function admittedNarrativeCandidates(
  facts: NarrativeFactPacket,
  surface: NarrativeSurface = "workload_insight",
): NarrativeExplanationId[] {
  const metricKinds = new Set<NarrativeMetricFact["kind"]>();
  const qualityKinds = new Set<NarrativeMetricFact["kind"]>();
  for (const metric of facts.metrics) {
    const scale = metric.kind === "cpu" ? 10 : 1;
    const expectedUnit =
      metric.kind === "cpu"
        ? "percent"
        : metric.kind === "memory"
          ? "megabytes"
          : "kilobytes_per_second";
    if (
      !Number.isFinite(metric.rounded_value) ||
      metric.rounded_value < 0 ||
      metric.rounded_value > 1_000_000_000 ||
      metric.unit !== expectedUnit ||
      metric.rounded_value !== Math.round(metric.rounded_value * scale) / scale ||
      metricKinds.has(metric.kind)
    )
      return [];
    metricKinds.add(metric.kind);
  }
  if (facts.leading_resource && !metricKinds.has(facts.leading_resource)) return [];
  for (const limitation of facts.measurement_limitations) {
    if (!metricKinds.has(limitation.kind) || qualityKinds.has(limitation.kind)) return [];
    qualityKinds.add(limitation.kind);
  }
  const candidates: NarrativeExplanationId[] = [
    "cpu_usage",
    "memory_usage",
    "disk_activity",
    "network_activity",
  ];
  return candidates.filter((id) => {
    if (surface === "overview_contributor" && explanationResource[id] !== facts.leading_resource)
      return false;
    const resource = explanationResource[id];
    return (
      facts.metrics.some((metric) => metric.kind === resource && metric.rounded_value > 0) &&
      !facts.measurement_limitations.some(
        (limitation) =>
          limitation.kind === resource &&
          (limitation.quality === "stale" || limitation.quality === "unavailable"),
      )
    );
  });
}

/** No provider-authored text reaches this renderer. Values always come from the current sample. */
export function renderNarrative(
  narrative: AcceptedNarrative | null,
  facts: NarrativeFactPacket,
  surface: NarrativeSurface,
  subjectStableId?: string,
): string | null {
  if (!narrative || !isNarrativeRelevant(narrative, facts, surface, subjectStableId)) return null;
  const resource = explanationResource[narrative.explanation_id];
  const metric = facts.metrics.find((value) => value.kind === resource);
  if (!metric) return null;
  const quality = facts.measurement_limitations.find((value) => value.kind === resource)?.quality;
  const qualification =
    quality === "estimated" ? " (estimated)" : quality === "limited" ? " (limited coverage)" : "";
  const measured = formatMeasuredResource(resource, metric.rounded_value);
  return `${facts.display_name}: ${measured} in this sample${qualification}.`;
}

function formatMeasuredResource(resource: NarrativeMetricFact["kind"], value: number): string {
  switch (resource) {
    case "cpu":
      return `${value}% CPU relative to one logical core`;
    case "memory":
      return `${value} MiB of memory`;
    case "io":
      return `${value} KiB/s of recorded read/write I/O`;
    case "network":
      return `${value} KiB/s of recorded network activity`;
  }
}

export function isNarrativeRelevant(
  narrative: AcceptedNarrative,
  facts: NarrativeFactPacket,
  surface: NarrativeSurface,
  subjectStableId?: string,
): boolean {
  return (
    narrative.surface === surface &&
    narrative.subject_stable_id === subjectStableId &&
    narrative.relevance_key === narrativeRelevanceKey(facts) &&
    admittedNarrativeCandidates(facts, surface).includes(narrative.explanation_id)
  );
}

export type NarrativeGenerator = (
  invocation: NarrativeInvocation,
  signal: AbortSignal,
) => Promise<NarrativeResult>;

export class NarrativeController {
  private readonly cache = new Map<string, AcceptedNarrative>();
  private readonly lastStartedAt = new Map<string, number>();
  private readonly generate: NarrativeGenerator;
  private readonly now: () => number;
  private readonly minimumIntervalMs: number;
  private inFlight: { generation: number; abort: AbortController } | null = null;
  private generation = 0;
  private disposed = false;

  constructor(
    generate: NarrativeGenerator,
    options: { now?: () => number; minimumIntervalMs?: number } = {},
  ) {
    this.generate = generate;
    this.now = options.now ?? Date.now;
    this.minimumIntervalMs = options.minimumIntervalMs ?? 30_000;
  }

  async request(invocation: NarrativeInvocation): Promise<AcceptedNarrative | null> {
    if (
      this.disposed ||
      admittedNarrativeCandidates(invocation.facts, invocation.request.surface).length < 2
    )
      return null;
    const cacheKey = invocationCacheKey(invocation);
    const cached = this.cache.get(cacheKey);
    if (cached) return cached;
    if (this.inFlight) return null;

    const subjectKey = invocationSubjectKey(invocation);
    const now = this.now();
    const previousStart = this.lastStartedAt.get(subjectKey);
    if (previousStart !== undefined && now - previousStart < this.minimumIntervalMs) {
      return null;
    }

    this.lastStartedAt.set(subjectKey, now);
    const generation = ++this.generation;
    const abort = new AbortController();
    this.inFlight = { generation, abort };
    try {
      const result = await this.generate(invocation, abort.signal);
      if (abort.signal.aborted || this.disposed || this.inFlight?.generation !== generation) {
        return null;
      }
      const accepted = validateNarrativeResult(invocation, result);
      if (accepted) this.cache.set(cacheKey, accepted);
      return accepted;
    } catch {
      return null;
    } finally {
      if (this.inFlight?.generation === generation) this.inFlight = null;
    }
  }

  cancel(): void {
    this.generation += 1;
    this.inFlight?.abort.abort();
    this.inFlight = null;
  }

  clear(): void {
    this.cancel();
    this.cache.clear();
    this.lastStartedAt.clear();
  }

  dispose(): void {
    this.disposed = true;
    this.clear();
  }
}

function invocationCacheKey(invocation: NarrativeInvocation): string {
  return `${invocationSubjectKey(invocation)}:${invocation.request.publication_seq}:${invocation.request.fact_digest}`;
}

function invocationSubjectKey(invocation: NarrativeInvocation): string {
  return `${invocation.request.surface}:${invocation.request.subject_stable_id ?? "system"}`;
}

function cleanFactText(value: string, fallback: string, maxLength: number): string {
  const cleaned = value
    .replace(/[\r\n\t]+/gu, " ")
    .replace(/\s+/gu, " ")
    .trim()
    .slice(0, maxLength);
  return cleaned || fallback;
}

function deduplicateLimitations(
  values: Iterable<NarrativeMeasurementLimitation>,
): NarrativeMeasurementLimitation[] {
  const seen = new Set<string>();
  const result: NarrativeMeasurementLimitation[] = [];
  for (const value of values) {
    const key = `${value.kind}:${value.quality}`;
    if (seen.has(key)) continue;
    seen.add(key);
    result.push(value);
  }
  return result.sort((left, right) =>
    `${left.kind}:${left.quality}`.localeCompare(`${right.kind}:${right.quality}`),
  );
}

function roundNumber(value: number, decimalPlaces: number): number {
  const safe = Number.isFinite(value) ? Math.max(0, value) : 0;
  return Number(safe.toFixed(decimalPlaces));
}
