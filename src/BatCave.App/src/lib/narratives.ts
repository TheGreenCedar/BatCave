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

export interface NarrativeResult {
  provider: NarrativeProvider;
  publication_seq: number;
  fact_digest: string;
  text: string;
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
      { kind: "cpu", rounded_value: roundNumber(input.cpuPercent, 1), unit: "percent" },
      {
        kind: "memory",
        rounded_value: Math.round(Math.max(0, input.memoryBytes) / 1024 ** 2),
        unit: "megabytes",
      },
      {
        kind: "io",
        rounded_value: Math.round(Math.max(0, input.ioBytesPerSecond) / 1024),
        unit: "kilobytes_per_second",
      },
      {
        kind: "network",
        rounded_value: Math.round(Math.max(0, input.networkBytesPerSecond) / 1024),
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
 * Generated copy is deliberately qualitative, so it remains relevant while live
 * metric values move. A change to identity, interpretation, or measurement
 * quality invalidates it; a routine sample refresh does not.
 */
export function narrativeRelevanceKey(facts: NarrativeFactPacket): string {
  return hashNarrativeValue({
    display_name: facts.display_name,
    category: facts.category,
    leading_resource: facts.leading_resource ?? null,
    ranking_state: facts.ranking_state,
    measurement_limitations: facts.measurement_limitations,
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
    return "Ready to generate short explanations locally.";
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
  surface: NarrativeSurface;
  subject_stable_id?: string;
  relevance_key: string;
}

export function validateNarrativeResult(
  invocation: NarrativeInvocation,
  value: NarrativeResult,
): AcceptedNarrative | null {
  if (
    value.publication_seq !== invocation.request.publication_seq ||
    value.fact_digest !== invocation.request.fact_digest ||
    !["apple_foundation", "foundry_local"].includes(value.provider)
  ) {
    return null;
  }

  const text = value.text.trim();
  if (
    text.length === 0 ||
    text.length > 180 ||
    /[\r\n\t<>]/u.test(text) ||
    /^(?:[-*#]|\d+[.)])\s/u.test(text) ||
    sentenceBoundaryCount(text) > 1 ||
    hasUnsupportedNumericClaim(text, invocation.facts) ||
    !hasRequiredGrounding(text, invocation.facts)
  ) {
    return null;
  }

  return {
    ...value,
    text,
    surface: invocation.request.surface,
    relevance_key: narrativeRelevanceKey(invocation.facts),
    ...(invocation.request.subject_stable_id
      ? { subject_stable_id: invocation.request.subject_stable_id }
      : {}),
  };
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
    narrative.relevance_key === narrativeRelevanceKey(facts)
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
    if (this.disposed) return null;
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

function sentenceBoundaryCount(text: string): number {
  return text.match(/[.!?](?=\s|$)/gu)?.length ?? 0;
}

function hasUnsupportedNumericClaim(text: string, facts: NarrativeFactPacket): boolean {
  const allowed = new Set(
    `${facts.display_name}\n${facts.category}`
      .match(/\d+(?:[.,]\d+)?/gu)
      ?.map(normalizeNumericToken) ?? [],
  );
  const claims = text.match(/\d+(?:[.,]\d+)?/gu) ?? [];
  return claims.some((claim) => !allowed.has(normalizeNumericToken(claim)));
}

const genericNameTokens = new Set([
  "app",
  "application",
  "gpu",
  "helper",
  "process",
  "renderer",
  "service",
  "utility",
  "worker",
]);

const allowedNarrativeWords = new Set([
  "a",
  "active",
  "activity",
  "an",
  "and",
  "appears",
  "as",
  "at",
  "attention",
  "category",
  "contributor",
  "contributes",
  "contributing",
  "current",
  "currently",
  "dominant",
  "driver",
  "driving",
  "elevated",
  "for",
  "from",
  "has",
  "heavy",
  "highest",
  "in",
  "is",
  "its",
  "largest",
  "leader",
  "leading",
  "load",
  "main",
  "monitoring",
  "more",
  "most",
  "normal",
  "notable",
  "now",
  "of",
  "on",
  "other",
  "pressure",
  "primary",
  "remains",
  "resource",
  "resources",
  "right",
  "showing",
  "shows",
  "source",
  "steady",
  "surface",
  "than",
  "the",
  "this",
  "to",
  "top",
  "usage",
  "use",
  "uses",
  "using",
  "with",
  "workload",
]);

function hasRequiredGrounding(text: string, facts: NarrativeFactPacket): boolean {
  if (!text.includes(facts.display_name)) return false;
  const textTokens = normalizedWords(text);
  const nameTokens = normalizedWords(facts.display_name).filter(
    (token) => token.length >= 3 && !genericNameTokens.has(token) && !/^\d+$/u.test(token),
  );
  if (nameTokens.length === 0 || !nameTokens.some((token) => textTokens.includes(token))) {
    return false;
  }

  const resourceAliases: Record<NarrativeMetricFact["kind"], readonly string[]> = {
    cpu: ["cpu", "processor"],
    memory: ["memory", "ram"],
    io: ["disk", "storage", "io"],
    network: ["network"],
  };
  if (
    facts.leading_resource !== undefined &&
    !resourceAliases[facts.leading_resource].some((alias) => textTokens.includes(alias))
  ) {
    return false;
  }

  const allowed = new Set(allowedNarrativeWords);
  for (const token of normalizedWords(`${facts.display_name} ${facts.category}`)) {
    allowed.add(token);
  }
  for (const aliases of Object.values(resourceAliases)) {
    for (const alias of aliases) allowed.add(alias);
  }
  for (const limitation of facts.measurement_limitations) {
    allowed.add(limitation.kind);
    allowed.add(limitation.quality);
  }
  return textTokens.every((token) => allowed.has(token));
}

function normalizedWords(value: string): string[] {
  return (
    value
      .normalize("NFKC")
      .toLocaleLowerCase("en-US")
      .match(/[\p{L}\p{N}]+/gu) ?? []
  );
}

function normalizeNumericToken(value: string): string {
  return value.replace(/,/gu, "").replace(/^0+(?=\d)/u, "");
}
