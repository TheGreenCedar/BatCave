import type { RuntimeQueryInputV4, RuntimeUiPreferencesV4 } from "./generated/runtime-protocol-v4";
import type {
  MetricQuality,
  RuntimeSnapshot,
  SystemHistoryPoint,
  SystemMetricsSnapshot,
} from "./types";
import type { ResolvedThemeName } from "./themes";
import {
  defaultNarrativeCapability,
  isNarrativeExplanationId,
  type NarrativeAvailability,
  type NarrativeCapability,
  type NarrativeFactPacket,
  type NarrativePreferences,
  type NarrativeRequest,
  type NarrativeResult,
} from "./narratives.ts";
import { adaptRuntimePayload } from "./protocol/runtimeAdapter.ts";
import { decodeProtocolEnvelope, type ProtocolMismatchView } from "./protocol/runtimeProtocol.ts";

export type RuntimeInvoke = <T>(command: string, args?: Record<string, unknown>) => Promise<T>;
export type RuntimeQueryWriteIntent = "runtime_only" | "user_mutation";
export const RUNTIME_MUTATION_QUEUE_CAPACITY = 32;

export class RuntimeMutationQueue {
  private readonly capacity: number;
  private pending = 0;
  private tail: Promise<void> = Promise.resolve();

  constructor(capacity = RUNTIME_MUTATION_QUEUE_CAPACITY) {
    this.capacity = capacity;
  }

  run<T>(mutation: () => Promise<T>): Promise<T> {
    if (this.pending >= this.capacity) {
      return Promise.reject("runtime_control_busy");
    }
    this.pending += 1;
    const result = this.tail.then(mutation);
    this.tail = result.then(
      () => {
        this.pending -= 1;
      },
      () => {
        this.pending -= 1;
      },
    );
    return result;
  }
}

const runtimeMutationQueues = new WeakMap<RuntimeInvoke, RuntimeMutationQueue>();
const runtimePublicationTimings = new WeakMap<
  RuntimeSnapshot,
  { startedAtMs: number; transportElapsedMs: number }
>();

export interface NativeSnapshotRead {
  snapshot: RuntimeSnapshot;
  error: string;
  mismatch: ProtocolMismatchView | null;
  ok: boolean;
}

export interface NativeSnapshotFallback {
  currentSnapshot: RuntimeSnapshot;
  emptySnapshot: (statusSummary: string) => RuntimeSnapshot;
  hasNativeSnapshot: boolean;
}

export async function readNativeSnapshot(
  invoke: RuntimeInvoke,
  fallback: NativeSnapshotFallback,
): Promise<NativeSnapshotRead> {
  try {
    const started = performance.now();
    const value = await invoke<unknown>("get_snapshot");
    return {
      snapshot: decodeRuntimeSnapshot(value, performance.now() - started),
      error: "",
      mismatch: null,
      ok: true,
    };
  } catch (error) {
    if (error instanceof ProtocolMismatchError) {
      return {
        snapshot: fallback.emptySnapshot(error.mismatch.message),
        error: error.mismatch.message,
        mismatch: error.mismatch,
        ok: false,
      };
    }
    const message = commandErrorMessage(error, "Native telemetry is unavailable.");

    return {
      snapshot: fallback.hasNativeSnapshot
        ? fallback.currentSnapshot
        : fallback.emptySnapshot(message),
      error: message,
      mismatch: null,
      ok: false,
    };
  }
}

export function setRuntimePaused(invoke: RuntimeInvoke, paused: boolean): Promise<RuntimeSnapshot> {
  return invokeRuntimeMutationSnapshot(invoke, paused ? "pause_runtime" : "resume_runtime");
}

export function refreshRuntime(invoke: RuntimeInvoke): Promise<RuntimeSnapshot> {
  return invokeRuntimeSnapshot(invoke, "refresh_now");
}

export function setRuntimeProcessQuery(
  invoke: RuntimeInvoke,
  query: RuntimeQueryInputV4,
  intent: RuntimeQueryWriteIntent = "user_mutation",
): Promise<RuntimeSnapshot> {
  return invokeRuntimeMutationSnapshot(invoke, "set_process_query", {
    query,
    persist: intent === "user_mutation",
  });
}

export function setRuntimeSampleInterval(
  invoke: RuntimeInvoke,
  sampleIntervalMs: number,
): Promise<RuntimeSnapshot> {
  return invokeRuntimeMutationSnapshot(invoke, "set_sample_interval", { sampleIntervalMs });
}

export function setRuntimeUiPreferences(
  invoke: RuntimeInvoke,
  preferences: RuntimeUiPreferencesV4,
): Promise<RuntimeSnapshot> {
  return invokeRuntimeMutationSnapshot(invoke, "set_ui_preferences", { preferences });
}

export class ProtocolMismatchError extends Error {
  readonly mismatch: ProtocolMismatchView;

  constructor(mismatch: ProtocolMismatchView) {
    super(mismatch.message);
    this.name = "ProtocolMismatchError";
    this.mismatch = mismatch;
  }
}

export function runtimeMutationAllowed(mismatch: ProtocolMismatchView | null): mismatch is null {
  return mismatch === null;
}

export function decodeRuntimeSnapshot(value: unknown, transportElapsedMs = 0): RuntimeSnapshot {
  const started = performance.now();
  const decoded = decodeProtocolEnvelope(value);
  if (decoded.kind === "protocol_mismatch") throw new ProtocolMismatchError(decoded.mismatch);
  const snapshot = adaptRuntimePayload(decoded.payload);
  runtimePublicationTimings.set(snapshot, { startedAtMs: started, transportElapsedMs });
  return snapshot;
}

export function observeAcceptedRuntimePublication(
  previous: Pick<RuntimeSnapshot, "publication_seq">,
  snapshot: RuntimeSnapshot,
  observe: (snapshot: RuntimeSnapshot, transportElapsedMs: number) => void,
): void {
  const timing = runtimePublicationTimings.get(snapshot);
  runtimePublicationTimings.delete(snapshot);
  // Equal publications may refresh displayed ages, but are not a new paint observation.
  if (!timing || snapshot.publication_seq <= previous.publication_seq) return;
  observe(snapshot, timing.transportElapsedMs + performance.now() - timing.startedAtMs);
}

async function invokeRuntimeSnapshot(
  invoke: RuntimeInvoke,
  command: string,
  args?: Record<string, unknown>,
): Promise<RuntimeSnapshot> {
  const started = performance.now();
  const value = await invoke<unknown>(command, args);
  return decodeRuntimeSnapshot(value, performance.now() - started);
}

function invokeRuntimeMutationSnapshot(
  invoke: RuntimeInvoke,
  command: string,
  args?: Record<string, unknown>,
): Promise<RuntimeSnapshot> {
  let queue = runtimeMutationQueues.get(invoke);
  if (!queue) {
    queue = new RuntimeMutationQueue();
    runtimeMutationQueues.set(invoke, queue);
  }
  return queue.run(() => invokeRuntimeSnapshot(invoke, command, args));
}

export async function getRuntimeProcessIcons(
  invoke: RuntimeInvoke,
  exes: string[],
  onError?: (message: string) => void,
): Promise<Record<string, string | null>> {
  try {
    return await invoke<Record<string, string | null>>("get_process_icons", { exes });
  } catch (error) {
    onError?.(commandErrorMessage(error, ""));
    return {};
  }
}

export async function syncRuntimeAppearance(
  invoke: RuntimeInvoke,
  theme: ResolvedThemeName,
  onError?: (message: string) => void,
): Promise<void> {
  try {
    await invoke("sync_app_appearance", { theme });
  } catch (error) {
    onError?.(commandErrorMessage(error, "Unable to synchronize the application icon."));
  }
}

export async function getNarrativePreferences(
  invoke: RuntimeInvoke,
): Promise<NarrativePreferences> {
  return decodeNarrativePreferences(await invoke<unknown>("get_narrative_preferences"));
}

export async function setEnhancedNarratives(
  invoke: RuntimeInvoke,
  enabled: boolean,
): Promise<NarrativePreferences> {
  return decodeNarrativePreferences(await invoke<unknown>("set_enhanced_narratives", { enabled }));
}

export async function getNarrativeCapability(invoke: RuntimeInvoke): Promise<NarrativeCapability> {
  return decodeNarrativeCapability(await invoke<unknown>("get_narrative_capability"));
}

export async function getNarrativeFactDigest(
  invoke: RuntimeInvoke,
  facts: NarrativeFactPacket,
): Promise<string> {
  const value = await invoke<unknown>("get_narrative_fact_digest", { facts });
  if (typeof value !== "string" || !/^[a-f0-9]{64}$/u.test(value)) {
    throw new Error("Narrative fact digest was not recognized.");
  }
  return value;
}

export async function generateLocalNarrative(
  invoke: RuntimeInvoke,
  request: NarrativeRequest,
  facts: NarrativeFactPacket,
): Promise<NarrativeResult | null> {
  const value = await invoke<unknown>("generate_narrative", { request, facts });
  if (!isRecord(value) || !isNarrativeAvailability(value.availability)) return null;
  return decodeNarrativeResult(value.result);
}

export async function cancelLocalNarrativeGeneration(invoke: RuntimeInvoke): Promise<void> {
  await invoke("cancel_narrative_generation");
}

export async function downloadNarrativeModel(invoke: RuntimeInvoke): Promise<NarrativeCapability> {
  return decodeNarrativeCapability(await invoke<unknown>("download_narrative_model"));
}

export async function cancelNarrativeModelDownload(
  invoke: RuntimeInvoke,
): Promise<NarrativeCapability> {
  return decodeNarrativeCapability(await invoke<unknown>("cancel_narrative_model_download"));
}

export async function readSystemHistory(
  invoke: RuntimeInvoke,
  afterSampleSeq: number,
): Promise<SystemHistoryPoint[]> {
  const value = await invoke<unknown>("get_system_history", { afterSampleSeq });
  return decodeSystemHistoryPoints(value);
}

export function decodeSystemHistoryPoints(value: unknown): SystemHistoryPoint[] {
  if (!Array.isArray(value)) {
    throw new Error("System history response was not recognized.");
  }
  return value.map(decodeSystemHistoryPoint);
}

const METRIC_QUALITIES: readonly MetricQuality[] = [
  "native",
  "estimated",
  "held",
  "partial",
  "unavailable",
];

const SYSTEM_NUMBER_FIELDS = [
  "cpu_percent",
  "kernel_cpu_percent",
  "memory_used_bytes",
  "memory_total_bytes",
  "process_count",
  "disk_read_total_bytes",
  "disk_write_total_bytes",
  "disk_read_bps",
  "disk_write_bps",
  "network_received_total_bytes",
  "network_transmitted_total_bytes",
  "network_received_bps",
  "network_transmitted_bps",
] as const;

const SYSTEM_OPTIONAL_NUMBER_FIELDS = [
  "memory_available_bytes",
  "swap_used_bytes",
  "swap_total_bytes",
] as const;

function decodeSystemHistoryPoint(value: unknown): SystemHistoryPoint {
  if (
    !isRecord(value) ||
    !isFiniteNumber(value.sample_seq) ||
    !isFiniteNumber(value.sampled_at_ms)
  ) {
    throw new Error("System history point was not recognized.");
  }
  return {
    sample_seq: value.sample_seq,
    sampled_at_ms: value.sampled_at_ms,
    system: decodeSystemMetrics(value.system),
  };
}

function decodeSystemMetrics(value: unknown): SystemMetricsSnapshot {
  if (!isRecord(value)) {
    throw new Error("System history metrics were not recognized.");
  }
  const system: Record<string, unknown> = {};
  for (const field of SYSTEM_NUMBER_FIELDS) {
    if (!isFiniteNumber(value[field])) {
      throw new Error(`System history metric ${field} was not recognized.`);
    }
    system[field] = value[field];
  }
  for (const field of SYSTEM_OPTIONAL_NUMBER_FIELDS) {
    const fieldValue = value[field];
    if (fieldValue !== undefined) {
      if (!isFiniteNumber(fieldValue)) {
        throw new Error(`System history metric ${field} was not recognized.`);
      }
      system[field] = fieldValue;
    }
  }
  const logicalCpu = value.logical_cpu_percent;
  if (!Array.isArray(logicalCpu) || !logicalCpu.every(isFiniteNumber)) {
    throw new Error("System history metric logical_cpu_percent was not recognized.");
  }
  system.logical_cpu_percent = logicalCpu;
  if (value.quality !== undefined) {
    system.quality = decodeMetricQualityMap(value.quality, "quality");
  }
  return system as unknown as SystemMetricsSnapshot;
}

function decodeMetricQualityMap(
  value: unknown,
  label: string,
): NonNullable<SystemMetricsSnapshot["quality"]> {
  if (!isRecord(value)) {
    throw new Error(`System history ${label} was not recognized.`);
  }
  const decoded: Record<string, unknown> = {};
  for (const [key, entry] of Object.entries(value)) {
    if (entry === undefined) continue;
    if (!isRecord(entry) || !METRIC_QUALITIES.includes(entry.quality as MetricQuality)) {
      throw new Error(`System history ${label}.${key} was not recognized.`);
    }
    decoded[key] = entry;
  }
  return decoded as NonNullable<SystemMetricsSnapshot["quality"]>;
}

function isFiniteNumber(value: unknown): value is number {
  return typeof value === "number" && Number.isFinite(value);
}

export function commandErrorMessage(error: unknown, fallback: string): string {
  if (error instanceof Error && error.message.trim()) {
    return error.message;
  }

  if (typeof error === "string" && error.trim()) {
    return error;
  }

  if (error && typeof error === "object") {
    try {
      const serialized = JSON.stringify(error);
      if (serialized) {
        return serialized;
      }
    } catch {
      return fallback;
    }
  }

  return fallback;
}

function decodeNarrativePreferences(value: unknown): NarrativePreferences {
  if (!isRecord(value) || typeof value.enhanced_narratives !== "boolean") {
    throw new Error("Narrative preferences were not recognized.");
  }
  return { enhanced_narratives: value.enhanced_narratives };
}

function decodeNarrativeCapability(value: unknown): NarrativeCapability {
  if (
    !isRecord(value) ||
    !isNarrativeAvailability(value.availability) ||
    (value.provider !== "apple_foundation" && value.provider !== "foundry_local") ||
    !isNarrativeDownloadState(value.download_state) ||
    typeof value.can_download !== "boolean" ||
    typeof value.can_cancel_download !== "boolean"
  ) {
    return defaultNarrativeCapability;
  }
  return {
    provider: value.provider,
    availability: value.availability,
    download_state: value.download_state,
    can_download: value.can_download,
    can_cancel_download: value.can_cancel_download,
    ...(typeof value.model_id === "string" ? { model_id: value.model_id } : {}),
    ...(typeof value.model_name === "string" ? { model_name: value.model_name } : {}),
    ...(typeof value.download_size_bytes === "number"
      ? { download_size_bytes: Math.max(0, value.download_size_bytes) }
      : {}),
    ...(typeof value.downloaded_bytes === "number"
      ? { downloaded_bytes: Math.max(0, value.downloaded_bytes) }
      : {}),
    ...(typeof value.license_name === "string" ? { license_name: value.license_name } : {}),
    ...(typeof value.license_url === "string" ? { license_url: value.license_url } : {}),
    ...(typeof value.detail_code === "string" ? { detail_code: value.detail_code } : {}),
  };
}

function decodeNarrativeResult(value: unknown): NarrativeResult | null {
  if (
    !isRecord(value) ||
    (value.provider !== "apple_foundation" && value.provider !== "foundry_local") ||
    typeof value.publication_seq !== "number" ||
    typeof value.fact_digest !== "string" ||
    (value.surface !== "overview_contributor" && value.surface !== "workload_insight") ||
    (value.subject_stable_id != null && typeof value.subject_stable_id !== "string") ||
    !isNarrativeExplanationId(value.explanation_id) ||
    "text" in value
  ) {
    return null;
  }
  return {
    provider: value.provider,
    publication_seq: value.publication_seq,
    fact_digest: value.fact_digest,
    surface: value.surface,
    ...(typeof value.subject_stable_id === "string"
      ? { subject_stable_id: value.subject_stable_id }
      : {}),
    explanation_id: value.explanation_id,
  };
}

function isNarrativeAvailability(value: unknown): value is NarrativeAvailability {
  return ["available", "unsupported", "model_not_ready", "runtime_missing", "busy"].includes(
    String(value),
  );
}

function isNarrativeDownloadState(value: unknown): value is NarrativeCapability["download_state"] {
  return ["not_required", "not_downloaded", "downloading", "ready", "failed"].includes(
    String(value),
  );
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null;
}
