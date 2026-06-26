<script lang="ts">
  import { onMount } from "svelte";
  import { formatBytes, formatPercent, formatRate, metricQualityAction, metricQualityLabel } from "../../format";
  import type { RuntimeSnapshot, SystemMetricQuality } from "../../types";

  export let snapshot: RuntimeSnapshot;
  export let sourceLabel: string;
  export let systemQuality: SystemMetricQuality;
  export let memoryPercent = 0;
  export let swapPercent = 0;
  export let visibleRows = 0;
  export let warnings: string[] = [];
  export let pollState: "starting" | "native" | "fixture" | "error";
  export let lastError = "";
  export let adminStatus = "";

  const healthStorageKey = "batcave.monitor.health-expanded";
  let expanded = false;

  onMount(() => {
    expanded = window.localStorage.getItem(healthStorageKey) === "true";
  });

  function setExpanded(next: boolean): void {
    expanded = next;
    window.localStorage.setItem(healthStorageKey, String(next));
  }
</script>

<footer class="health-footer" class:expanded aria-label="Runtime health">
  <button class="health-summary" type="button" aria-expanded={expanded} onclick={() => setExpanded(!expanded)}>
    <strong>
      <svg class="health-chevron" viewBox="0 0 24 24" aria-hidden="true">
        <path d="m6 9 6 6 6-6" />
      </svg>
      System health
    </strong>
    <span><i></i> CPU {formatPercent(snapshot.system.cpu_percent)}</span>
    <span><i></i> Memory {formatPercent(memoryPercent)}</span>
    <span><i></i> Disk {formatRate(snapshot.system.disk_read_bps + snapshot.system.disk_write_bps)}</span>
    <span><i></i> Network {formatRate(snapshot.system.network_received_bps + snapshot.system.network_transmitted_bps)}</span>
    <span><i></i> {snapshot.health.degraded ? "Degraded" : "Clean"}</span>
  </button>
  {#if expanded}
    <div class="health-detail">
      <dl class="key-value-grid">
        <div><dt>Status</dt><dd>{snapshot.health.status_summary}</dd></div>
        <div><dt>Source</dt><dd>{sourceLabel}</dd></div>
        <div><dt>CPU quality</dt><dd>{metricQualityLabel(systemQuality.cpu, "Legacy")}<small>{metricQualityAction(systemQuality.cpu)}</small></dd></div>
        <div><dt>Disk quality</dt><dd>{metricQualityLabel(systemQuality.disk, "Legacy")}<small>{metricQualityAction(systemQuality.disk)}</small></dd></div>
        <div><dt>Network quality</dt><dd>{metricQualityLabel(systemQuality.network, "Aggregate")}<small>{metricQualityAction(systemQuality.network)}</small></dd></div>
        <div><dt>App CPU</dt><dd>{formatPercent(snapshot.health.app_cpu_percent)}</dd></div>
        <div><dt>App RSS</dt><dd>{formatBytes(snapshot.health.app_rss_bytes)}</dd></div>
        <div><dt>Tick p95</dt><dd>{snapshot.health.tick_p95_ms.toFixed(1)} ms</dd></div>
        <div><dt>Jitter p95</dt><dd>{snapshot.health.jitter_p95_ms.toFixed(1)} ms</dd></div>
        <div><dt>Admin</dt><dd>{adminStatus}</dd></div>
        <div><dt>Memory load</dt><dd>{formatPercent(memoryPercent)}</dd></div>
        <div><dt>Swap load</dt><dd>{formatPercent(swapPercent)}</dd></div>
        <div><dt>Visible rows</dt><dd>{visibleRows}</dd></div>
        <div><dt>Warnings</dt><dd>{snapshot.warnings.length}</dd></div>
      </dl>
      {#if pollState === "error"}
        <p class="command-error" role="status" aria-live="polite">{lastError}</p>
      {:else if warnings.length}
        <ul class="warnings" aria-label="Collector warnings" aria-live="polite">
          {#each warnings.slice(0, 3) as warning}
            <li>{warning}</li>
          {/each}
        </ul>
      {:else if pollState === "fixture"}
        <p class="quiet-note">Browser fixture mode is running deterministic demo telemetry. Native collector proof requires the desktop app.</p>
      {:else}
        <p class="quiet-note">Local collectors are steady. Native telemetry is local-only and running without warnings.</p>
      {/if}
    </div>
  {/if}
</footer>
