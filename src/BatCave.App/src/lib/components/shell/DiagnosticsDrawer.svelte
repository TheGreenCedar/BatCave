<script lang="ts">
  import { tick } from "svelte";
  import { formatBytes, metricQualityAction, metricQualityLabel } from "../../format";
  import type { RuntimeSnapshot, RuntimeWarning, SystemMetricQuality } from "../../types";

  export let open = false;
  export let snapshot = {} as RuntimeSnapshot;
  export let sourceLabel: string;
  export let systemQuality: SystemMetricQuality;
  export let pollState: "starting" | "native" | "fixture" | "error";
  export let lastError = "";
  export let adminStatus = "";
  export let onClose: () => void;

  interface DiagnosticIssue {
    title: string;
    impact: string;
    action: string;
    raw: string;
  }

  $: issues = snapshot.warnings.slice().reverse().map(toDiagnosticIssue);

  let closeButton: HTMLButtonElement | null = null;
  let previouslyOpen = false;

  $: if (open && !previouslyOpen) {
    previouslyOpen = true;
    void tick().then(() => closeButton?.focus());
  }
  $: if (!open) previouslyOpen = false;

  function toDiagnosticIssue(warning: RuntimeWarning): DiagnosticIssue {
    const raw = warning.message;
    const value = `${warning.category} ${warning.message}`.toLocaleLowerCase();

    if (value.includes("admin_mode") || value.includes("elevat")) {
      return {
        title: "Privileged access is not active",
        impact: "Restricted process fields may remain unavailable while standard monitoring continues.",
        action: "Finish or cancel the Windows elevation prompt, then refresh BatCave.",
        raw,
      };
    }

    if (value.includes("network_attribution") || value.includes("etw") || value.includes("ebpf")) {
      return {
        title: "Per-process network attribution is limited",
        impact: "System network totals remain available, but app-level network values may be missing.",
        action: "Review privileged access if you need app-level network attribution.",
        raw,
      };
    }

    if (value.includes("permission") || value.includes("access") || value.includes("denied")) {
      return {
        title: "Some process details are blocked",
        impact: "BatCave cannot read every field for protected processes.",
        action: "Use privileged access only when those missing fields matter to the diagnosis.",
        raw,
      };
    }

    return {
      title: titleCase(warning.category || "Collector limitation"),
      impact: "A collector reported a limitation. Available telemetry continues to update.",
      action: "Open technical details below when the missing data affects your diagnosis.",
      raw,
    };
  }

  function titleCase(value: string): string {
    return value
      .replaceAll("_", " ")
      .replace(/\b\w/g, (character) => character.toLocaleUpperCase());
  }
</script>

<svelte:window
  onkeydown={(event) => {
    if (open && event.key === "Escape") onClose();
  }}
/>

{#if open}
  <div class="drawer-layer diagnostics-layer">
    <button class="drawer-backdrop" type="button" aria-label="Close diagnostics" onclick={onClose}></button>
    <div class="diagnostics-drawer" role="dialog" aria-modal="true" aria-labelledby="diagnostics-title">
      <header class="drawer-header">
        <div>
          <span>Local telemetry</span>
          <h2 id="diagnostics-title">Diagnostics</h2>
        </div>
        <button bind:this={closeButton} class="icon-action" type="button" aria-label="Close diagnostics" onclick={onClose}>
          <svg class="control-icon" viewBox="0 0 24 24" aria-hidden="true"><path d="m6 6 12 12M18 6 6 18" /></svg>
        </button>
      </header>

      <div class="drawer-scroll">
        <section class="diagnostic-overview" class:healthy={!snapshot.health.degraded && pollState !== "error"}>
          <span>{pollState === "error" ? "Stale" : snapshot.health.degraded ? "Limited" : "Healthy"}</span>
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
                    <div><dt>What to do</dt><dd>{issue.action}</dd></div>
                  </dl>
                  <details>
                    <summary>Technical detail</summary>
                    <code>{issue.raw}</code>
                  </details>
                </article>
              {/each}
            </div>
          </section>
        {/if}

        <section class="diagnostic-section">
          <div class="drawer-section-title"><h3>Collector state</h3></div>
          <dl class="diagnostic-grid">
            <div><dt>Source</dt><dd>{sourceLabel}</dd></div>
            <div><dt>CPU quality</dt><dd>{metricQualityLabel(systemQuality.cpu, "Legacy")}</dd></div>
            <div><dt>Disk quality</dt><dd>{metricQualityLabel(systemQuality.disk, "Legacy")}</dd></div>
            <div><dt>Network quality</dt><dd>{metricQualityLabel(systemQuality.network, "Aggregate")}</dd></div>
            <div><dt>Privileged access</dt><dd>{adminStatus}</dd></div>
            <div><dt>App CPU</dt><dd>{snapshot.health.app_cpu_percent.toFixed(1)}%</dd></div>
            <div><dt>App memory</dt><dd>{formatBytes(snapshot.health.app_rss_bytes)}</dd></div>
            <div><dt>Collector p95</dt><dd>{snapshot.health.tick_p95_ms.toFixed(1)} ms</dd></div>
          </dl>
          <details class="technical-disclosure">
            <summary>Quality guidance</summary>
            <p>{metricQualityAction(systemQuality.cpu)}</p>
            <p>{metricQualityAction(systemQuality.disk)}</p>
            <p>{metricQualityAction(systemQuality.network)}</p>
          </details>
        </section>
      </div>
    </div>
  </div>
{/if}
