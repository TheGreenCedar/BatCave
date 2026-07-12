<script lang="ts">
  import { currentDiagnosticIssues } from "../../diagnostics";
  import { formatBytes, metricQualityLabel, qualityGuidance } from "../../format";
  import type { RuntimeSnapshot, SystemMetricQuality } from "../../types";

  export let open = false;
  export let snapshot = {} as RuntimeSnapshot;
  export let sourceLabel: string;
  export let systemQuality: SystemMetricQuality = {};
  export let pollState: "starting" | "native" | "fixture" | "error" = "starting";
  export let lastError = "";
  export let adminStatus = "";
  export let onClose: () => void = () => {};

  $: issues = currentDiagnosticIssues(
    snapshot.warnings,
    snapshot.admin_mode,
    snapshot.environment.admin_mode_available,
  );
  $: guidance = qualityGuidance(systemQuality);
  $: overviewLabel =
    pollState === "error"
      ? "Stale"
      : snapshot.admin_mode.state === "requesting"
        ? "Waiting"
        : snapshot.admin_mode.state === "recovering"
          ? "Recovering"
          : snapshot.health.degraded
            ? "Limited"
            : "Healthy";

  let dialog: HTMLDialogElement | null = null;
  let opener: HTMLElement | null = null;
  let copyStatus = "";

  $: if (dialog) syncDialog(dialog, open);

  function syncDialog(element: HTMLDialogElement, shouldOpen: boolean): void {
    if (shouldOpen && !element.open) {
      opener = document.activeElement instanceof HTMLElement ? document.activeElement : null;
      element.showModal();
    } else if (!shouldOpen && element.open) {
      element.close();
      restoreOpener();
    }
  }

  function requestClose(): void {
    dialog?.close();
    restoreOpener();
    onClose();
  }

  function restoreOpener(): void {
    opener?.focus();
    opener = null;
  }

  function handleBackdropClick(event: MouseEvent): void {
    if (event.target === event.currentTarget) requestClose();
  }

  async function copyLocalData(): Promise<void> {
    const path = snapshot.environment.data_directory;
    if (!path) return;

    try {
      await navigator.clipboard.writeText(path);
      copyStatus = "Copied";
    } catch {
      copyStatus = "Copy failed";
    }
  }
</script>

<dialog
  bind:this={dialog}
  class="drawer-layer diagnostics-layer"
  aria-labelledby="diagnostics-title"
  oncancel={(event) => {
    event.preventDefault();
    requestClose();
  }}
  onclose={restoreOpener}
  onclick={handleBackdropClick}
>
    <div class="diagnostics-drawer">
      <header class="drawer-header">
        <div>
          <span>Local telemetry</span>
          <h2 id="diagnostics-title">Diagnostics</h2>
        </div>
        <button class="icon-action" type="button" aria-label="Close diagnostics" onclick={requestClose}>
          <svg class="control-icon" viewBox="0 0 24 24" aria-hidden="true"><path d="m6 6 12 12M18 6 6 18" /></svg>
        </button>
      </header>

      <div class="drawer-scroll">
        <section class="diagnostic-overview" class:healthy={overviewLabel === "Healthy"}>
          <span>{overviewLabel}</span>
          <h3>{pollState === "error" ? lastError : snapshot.health.status_summary}</h3>
          <p>
            {pollState === "fixture"
              ? "Fixture data is useful for layout work, not native collector proof."
              : snapshot.health.degraded
                ? "BatCave keeps the trustworthy parts running and marks the gaps instead of inventing data."
                : "Collectors are current and no limitations are active."}
          </p>
        </section>

        {#if issues.length > 0}
          <section class="diagnostic-section" aria-labelledby="limitations-title">
            <div class="drawer-section-title">
              <h3 id="limitations-title">Active limitations</h3>
              <span>{issues.length}</span>
            </div>
            <div class="diagnostic-list">
              {#each issues as issue}
                <article class="diagnostic-issue">
                  <h4>{issue.title}</h4>
                  <dl>
                    <div><dt>Impact</dt><dd>{issue.impact}</dd></div>
                  </dl>
                  <details>
                    <summary>Error details</summary>
                    <small>{issue.key} · {new Date(issue.occurredAtMs).toLocaleString()}</small>
                    <code>{issue.raw}</code>
                  </details>
                </article>
              {/each}
            </div>
          </section>
        {/if}

        <section class="diagnostic-section">
          <details class="technical-disclosure collector-details">
            <summary>Technical details</summary>
            <dl class="diagnostic-grid">
              <div><dt>Source</dt><dd>{sourceLabel}</dd></div>
              <div><dt>Platform</dt><dd>{snapshot.environment.platform}</dd></div>
              <div><dt>CPU quality</dt><dd>{metricQualityLabel(systemQuality.cpu, "Legacy")}</dd></div>
              <div><dt>Disk quality</dt><dd>{metricQualityLabel(systemQuality.disk, "Legacy")}</dd></div>
              <div><dt>Network quality</dt><dd>{metricQualityLabel(systemQuality.network, "Aggregate")}</dd></div>
              <div><dt>Privileged access</dt><dd>{adminStatus}</dd></div>
              <div><dt>Last elevated sample</dt><dd>{snapshot.admin_mode.last_success_at_ms ? new Date(snapshot.admin_mode.last_success_at_ms).toLocaleString() : "None this session"}</dd></div>
              <div><dt>App CPU</dt><dd>{snapshot.health.app_cpu_percent.toFixed(1)}%</dd></div>
              <div><dt>App memory</dt><dd>{formatBytes(snapshot.health.app_rss_bytes)}</dd></div>
              <div><dt>Collector p95</dt><dd>{snapshot.health.tick_p95_ms.toFixed(1)} ms</dd></div>
            </dl>
            <div class="local-data-detail">
              <span><strong>Local data</strong>{snapshot.environment.data_directory ?? "No native runtime directory"}</span>
              {#if snapshot.environment.data_directory}
                <button type="button" onclick={copyLocalData}>Copy path</button>
              {/if}
              <small aria-live="polite">{copyStatus}</small>
            </div>
            {#if guidance.length > 0}
              <div class="quality-guidance">
                <h4>Quality guidance</h4>
                {#each guidance as item}<p>{item}</p>{/each}
              </div>
            {/if}
          </details>
        </section>
      </div>
    </div>
</dialog>
