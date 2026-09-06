import { invoke, isTauri } from "@tauri-apps/api/core";
import type { RuntimeSnapshot } from "./types";

let enabled = false;
let pendingPublication = false;

function afterPaint(callback: () => void): void {
  requestAnimationFrame(() => requestAnimationFrame(callback));
}

export async function installDesktopProbe(): Promise<void> {
  if (!isTauri()) return;
  enabled = await invoke<boolean>("desktop_probe_enabled").catch(() => false);
  if (!enabled) return;
  const listener = (event: Event) => {
    if (!event.isTrusted || document.visibilityState !== "visible") return;
    const started = performance.now();
    afterPaint(() => {
      void invoke("record_desktop_probe", {
        observation: { kind: "interaction", duration_ms: performance.now() - started },
      }).catch(() => undefined);
    });
  };
  document.addEventListener("click", listener, { capture: true });
  document.addEventListener("keydown", listener, { capture: true });
  setTimeout(() => {
    enabled = false;
    document.removeEventListener("click", listener, { capture: true });
    document.removeEventListener("keydown", listener, { capture: true });
  }, 150_000);
}

// Called after ingestion accepts and assigns the snapshot. Two frames include
// the pending DOM update and a paint opportunity.
export function observeDesktopPublication(snapshot: RuntimeSnapshot, transportElapsedMs = 0): void {
  if (!enabled || pendingPublication || document.visibilityState !== "visible") return;
  const received = performance.now();
  pendingPublication = true;
  afterPaint(() => {
    pendingPublication = false;
    void invoke("record_desktop_probe", {
      observation: {
        kind: "publication",
        publication_seq: snapshot.publication_seq,
        age_ms:
          snapshot.health.publication_age_ms + transportElapsedMs + performance.now() - received,
        process_count: snapshot.total_process_count,
        interval_ms: snapshot.settings.sample_interval_ms,
      },
    }).catch(() => undefined);
  });
}
