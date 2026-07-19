import type { RuntimeQueryInputV3, RuntimeUiPreferencesV3 } from "./generated/runtime-protocol-v3";
import type { RuntimeSnapshot } from "./types";
import type { ResolvedThemeName } from "./themes";
import {
  defaultNarrativeCapability,
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
    return {
      snapshot: decodeRuntimeSnapshot(await invoke<unknown>("get_snapshot")),
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
  query: RuntimeQueryInputV3,
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
  preferences: RuntimeUiPreferencesV3,
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

export function decodeRuntimeSnapshot(value: unknown): RuntimeSnapshot {
  const decoded = decodeProtocolEnvelope(value);
  if (decoded.kind === "protocol_mismatch") throw new ProtocolMismatchError(decoded.mismatch);
  return adaptRuntimePayload(decoded.payload);
}

async function invokeRuntimeSnapshot(
  invoke: RuntimeInvoke,
  command: string,
  args?: Record<string, unknown>,
): Promise<RuntimeSnapshot> {
  return decodeRuntimeSnapshot(await invoke<unknown>(command, args));
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
    typeof value.text !== "string"
  ) {
    return null;
  }
  return {
    provider: value.provider,
    publication_seq: value.publication_seq,
    fact_digest: value.fact_digest,
    text: value.text,
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
