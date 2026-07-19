<script lang="ts">
  import X from "phosphor-svelte/lib/X";
  import { focusDialogStart, trapDialogFocus } from "../../dialogFocus";
  import {
    currentDiagnosticIssues,
    diagnosticOverviewLabel,
    nativeLifecycleDiagnosticLabels,
    suppressedDiagnosticsLabel,
    windowsLifecycleDiagnosticsVisible,
  } from "../../diagnostics";
  import {
    collectorServiceStateLabel,
    installKindLabel,
    privilegedSourceLabel,
    processElevationLabel,
  } from "../../environmentPresentation";
  import { formatBytes, metricQualityLabel, qualityGuidance } from "../../format";
  import { platformPresentation } from "../../platformPresentation";
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
  $: hasQualityLimitations = guidance.length > 0;
  $: hasActiveLimitations = issues.length > 0 || hasQualityLimitations;
  $: presentation = platformPresentation(snapshot.environment);
  $: overviewLabel = diagnosticOverviewLabel(
    pollState,
    snapshot.admin_mode.state,
    snapshot.health.degraded,
    issues.length,
    guidance.length,
  );

  let dialog: HTMLDialogElement | null = null;
  let opener: HTMLElement | null = null;
  let copyStatus = "";

  $: collectorService = snapshot.admin_mode.collector_service ?? null;
  $: lifecycleDiagnostics = nativeLifecycleDiagnosticLabels(snapshot, pollState);
  $: showWindowsLifecycleDiagnostics = windowsLifecycleDiagnosticsVisible(snapshot);

  $: if (dialog) syncDialog(dialog, open);

  function syncDialog(element: HTMLDialogElement, shouldOpen: boolean): void {
    if (shouldOpen && !element.open) {
      opener = document.activeElement instanceof HTMLElement ? document.activeElement : null;
      element.showModal();
      focusDialogStart(element);
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
  tabindex="-1"
  oncancel={(event) => {
    event.preventDefault();
    requestClose();
  }}
  onclose={restoreOpener}
  onkeydown={(event) => trapDialogFocus(event, dialog)}
  onclick={handleBackdropClick}
>
    <div class="diagnostics-drawer">
      <header class="drawer-header">
        <div>
          <span>Local telemetry</span>
          <h2 id="diagnostics-title">Diagnostics</h2>
        </div>
        <button
          class="icon-action"
          type="button"
          aria-label="Close diagnostics"
          data-dialog-initial-focus
          onclick={requestClose}
        >
          <X size={20} weight="bold" aria-hidden="true" />
        </button>
      </header>

      <div class="drawer-scroll">
        <section class="diagnostic-overview" class:healthy={overviewLabel === "Healthy"}>
          <span>{overviewLabel}</span>
          <h3>
            {pollState === "error"
              ? lastError
              : hasActiveLimitations && !snapshot.health.degraded
                ? "Core telemetry is current with known limitations."
                : snapshot.health.status_summary}
          </h3>
          <p>
            {pollState === "fixture"
              ? "Fixture data is useful for layout work, not native collector proof."
              : snapshot.health.degraded || hasActiveLimitations
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

        {#if guidance.length > 0}
          <section class="diagnostic-section" aria-labelledby="quality-limitations-title">
            <div class="drawer-section-title">
              <h3 id="quality-limitations-title">Data limitations</h3>
              <span>{guidance.length}</span>
            </div>
            <div class="diagnostic-list">
              {#each guidance as item}
                <article class="diagnostic-issue">
                  <h4>Metric coverage</h4>
                  <p>{item}</p>
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
              <div><dt>Platform</dt><dd>{presentation.platformName}</dd></div>
              <div><dt>Package</dt><dd>{installKindLabel(snapshot.environment.install_kind)}</dd></div>
              <div><dt>CPU quality</dt><dd>{metricQualityLabel(systemQuality.cpu, "Legacy")}</dd></div>
              <div><dt>Kernel CPU quality</dt><dd>{metricQualityLabel(systemQuality.kernel_cpu, "Not reported")}</dd></div>
              <div><dt>Logical CPU quality</dt><dd>{metricQualityLabel(systemQuality.logical_cpu, "Not reported")}</dd></div>
              <div><dt>Memory quality</dt><dd>{metricQualityLabel(systemQuality.memory, "Legacy")}</dd></div>
              <div><dt>Swap quality</dt><dd>{metricQualityLabel(systemQuality.swap, "Not reported")}</dd></div>
              <div><dt>Disk quality</dt><dd>{metricQualityLabel(systemQuality.disk, "Legacy")}</dd></div>
              <div><dt>Network quality</dt><dd>{metricQualityLabel(systemQuality.network, "Aggregate")}</dd></div>
              <div
                role="group"
                aria-label={`Current process: ${processElevationLabel(snapshot.environment)}`}
              ><dt>Current process</dt><dd>{processElevationLabel(snapshot.environment)}</dd></div>
              <div><dt>Privileged collection</dt><dd>{adminStatus}</dd></div>
              <div
                role="group"
                aria-label={`Privileged source: ${privilegedSourceLabel(snapshot.admin_mode.source)}`}
              ><dt>Privileged source</dt><dd>{privilegedSourceLabel(snapshot.admin_mode.source)}</dd></div>
              <div><dt>Last privileged sample</dt><dd>{snapshot.admin_mode.last_success_at_ms ? new Date(snapshot.admin_mode.last_success_at_ms).toLocaleString() : "None this session"}</dd></div>
              {#if showWindowsLifecycleDiagnostics}
                <div
                  role="group"
                  aria-label={`Standard fallback: ${lifecycleDiagnostics.standardFallback}`}
                ><dt>Standard fallback</dt><dd>{lifecycleDiagnostics.standardFallback}</dd></div>
                <div
                  role="group"
                  aria-label={`Protected sample: ${lifecycleDiagnostics.protectedSample}`}
                ><dt>Protected sample</dt><dd>{lifecycleDiagnostics.protectedSample}</dd></div>
                <div
                  role="group"
                  aria-label={`Fallback process ETW: ${lifecycleDiagnostics.fallbackProcessEtw}`}
                ><dt>Fallback process ETW</dt><dd>{lifecycleDiagnostics.fallbackProcessEtw}</dd></div>
              {/if}
              {#if collectorService}
                <div
                  role="group"
                  aria-label={`Collector service: ${collectorServiceStateLabel(collectorService)}`}
                ><dt>Collector service</dt><dd>{collectorServiceStateLabel(collectorService)}</dd></div>
                <div
                  role="group"
                  aria-label={`Service version: ${collectorService.service_version ?? "Not reported"}`}
                ><dt>Service version</dt><dd>{collectorService.service_version ?? "Not reported"}</dd></div>
                <div
                  role="group"
                  aria-label={`Service protocol: ${collectorService.negotiated_protocol_version ?? "Not negotiated"}`}
                ><dt>Service protocol</dt><dd>{collectorService.negotiated_protocol_version ?? "Not negotiated"}</dd></div>
                <div
                  role="group"
                  aria-label={`Minimum desktop: ${collectorService.minimum_desktop_version ?? "Not reported"}`}
                ><dt>Minimum desktop</dt><dd>{collectorService.minimum_desktop_version ?? "Not reported"}</dd></div>
                <div
                  role="group"
                  aria-label={`Service release: ${collectorService.release_identity?.app_version ?? "Not reported"}`}
                ><dt>Service release</dt><dd>{collectorService.release_identity?.app_version ?? "Not reported"}</dd></div>
                <div
                  role="group"
                  aria-label={`Service instance: ${collectorService.instance_id ?? "Not connected"}`}
                ><dt>Service instance</dt><dd>{collectorService.instance_id ?? "Not connected"}</dd></div>
                <div><dt>Last service connection</dt><dd>{collectorService.last_connected_at_ms ? new Date(collectorService.last_connected_at_ms).toLocaleString() : "None this session"}</dd></div>
                <div
                  role="group"
                  aria-label={`Service detail: ${collectorService.detail ?? "None"}`}
                ><dt>Service detail</dt><dd>{collectorService.detail ?? "None"}</dd></div>
              {/if}
              <div><dt>App CPU</dt><dd>{snapshot.health.app_cpu_percent.toFixed(1)}%</dd></div>
              <div><dt>App memory</dt><dd>{formatBytes(snapshot.health.app_rss_bytes)}</dd></div>
              <div><dt>Collector p95</dt><dd>{snapshot.health.collection_p95_ms === null ? "Unavailable" : `${snapshot.health.collection_p95_ms.toFixed(1)} ms`}</dd></div>
              <div><dt>Local persistence</dt><dd>{snapshot.persistence?.state ?? "Not reported"}</dd></div>
              <div><dt>Current-user permissions</dt><dd>{snapshot.persistence?.roots.find((root) => root.owner === "current_user")?.permission_state ?? "Not reported"}</dd></div>
              <div><dt>Suppressed diagnostics</dt><dd>{suppressedDiagnosticsLabel(snapshot.persistence)}</dd></div>
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
