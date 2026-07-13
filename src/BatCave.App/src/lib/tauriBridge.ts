import type { RuntimeQueryInputV3 } from "./generated/runtime-protocol-v3";
import type { RuntimeSnapshot } from "./types";
import { adaptRuntimePayload } from "./protocol/runtimeAdapter.ts";
import { decodeProtocolEnvelope, type ProtocolMismatchView } from "./protocol/runtimeProtocol.ts";

export type RuntimeInvoke = <T>(command: string, args?: Record<string, unknown>) => Promise<T>;

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
  return invokeRuntimeSnapshot(invoke, paused ? "pause_runtime" : "resume_runtime");
}

export function refreshRuntime(invoke: RuntimeInvoke): Promise<RuntimeSnapshot> {
  return invokeRuntimeSnapshot(invoke, "refresh_now");
}

export function setRuntimeProcessQuery(
  invoke: RuntimeInvoke,
  query: RuntimeQueryInputV3,
): Promise<RuntimeSnapshot> {
  return invokeRuntimeSnapshot(invoke, "set_process_query", { query });
}

export function setRuntimeSampleInterval(
  invoke: RuntimeInvoke,
  sampleIntervalMs: number,
): Promise<RuntimeSnapshot> {
  return invokeRuntimeSnapshot(invoke, "set_sample_interval", { sampleIntervalMs });
}

export function setRuntimeAdminMode(
  invoke: RuntimeInvoke,
  enabled: boolean,
): Promise<RuntimeSnapshot> {
  return invokeRuntimeSnapshot(invoke, "set_admin_mode", { enabled });
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
