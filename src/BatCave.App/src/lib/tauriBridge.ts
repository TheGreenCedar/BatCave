import type { RuntimeQuery, RuntimeSnapshot } from "./types";

export type RuntimeInvoke = <T>(command: string, args?: Record<string, unknown>) => Promise<T>;

export interface NativeSnapshotRead {
  snapshot: RuntimeSnapshot;
  error: string;
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
      snapshot: await invoke<RuntimeSnapshot>("get_snapshot"),
      error: "",
      ok: true,
    };
  } catch (error) {
    const message = commandErrorMessage(error, "Native telemetry is unavailable.");

    return {
      snapshot: fallback.hasNativeSnapshot
        ? fallback.currentSnapshot
        : fallback.emptySnapshot(message),
      error: message,
      ok: false,
    };
  }
}

export function setRuntimePaused(invoke: RuntimeInvoke, paused: boolean): Promise<RuntimeSnapshot> {
  return invoke<RuntimeSnapshot>(paused ? "pause_runtime" : "resume_runtime");
}

export function refreshRuntime(invoke: RuntimeInvoke): Promise<RuntimeSnapshot> {
  return invoke<RuntimeSnapshot>("refresh_now");
}

export function setRuntimeAdminMode(
  invoke: RuntimeInvoke,
  enabled: boolean,
): Promise<RuntimeSnapshot> {
  return invoke<RuntimeSnapshot>("set_admin_mode", { enabled });
}

export function setRuntimeProcessQuery(
  invoke: RuntimeInvoke,
  query: RuntimeQuery,
): Promise<RuntimeSnapshot> {
  return invoke<RuntimeSnapshot>("set_process_query", { query });
}

export async function getRuntimeProcessIcon(
  invoke: RuntimeInvoke,
  exe: string,
): Promise<string | null> {
  try {
    return await invoke<string | null>("get_process_icon", { exe });
  } catch {
    return null;
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
