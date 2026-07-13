<script lang="ts">
  import { Copy } from "phosphor-svelte";
  import MiniChart from "../../MiniChart.svelte";
  import {
    displayGroupMetricValue,
    formatBytes,
    formatPercent,
    formatRate,
    groupMetricCanDisplay,
    metricQualityLabel,
  } from "../../format";
  import type { ProcessIconKind } from "../../process";
  import type { ChartPalette } from "../../themes";
  import type { GroupDetail, MetricCoverage } from "../../types";
  import ProcessIcon from "../processes/ProcessIcon.svelte";

  // oxlint-disable-next-line no-unassigned-vars -- Svelte assigns this required component prop.
  export let detail: GroupDetail;
  // oxlint-disable-next-line no-unassigned-vars -- Svelte assigns this required component prop.
  export let processHistory: { cpu: number[] };
  export let copyStatus = "";
  export let activeTheme: ChartPalette;
  export let iconKind: ProcessIconKind = "process";
  export let iconSrc: string | undefined = undefined;
  export let onCopy: () => void;

  $: copyFailed = copyStatus !== "" && copyStatus !== "Workload summary copied.";
  $: cpuCanDisplay = groupMetricCanDisplay(detail.quality.cpu, detail.coverage.cpu);
  $: cpuHistory = cpuCanDisplay ? processHistory.cpu : [];
  $: cpuChartMax = Math.max(100, Math.ceil(Math.max(0, ...cpuHistory) / 100) * 100);

  function processCountLabel(count: number): string {
    return `${count} ${count === 1 ? "process" : "processes"}`;
  }

  function coverageLabel(coverage: MetricCoverage): string {
    return `${coverage.available} of ${coverage.total}`;
  }

  function networkLabel(): string {
    return displayGroupMetricValue(
      detail.network_bps,
      detail.quality.network,
      detail.coverage.network,
      formatRate,
    );
  }

  function findingCopy(): string {
    if (cpuCanDisplay && detail.cpu_percent >= 30) return "Aggregate CPU use is high right now.";
    if (
      groupMetricCanDisplay(detail.quality.memory, detail.coverage.memory) &&
      detail.memory_bytes >= 900 * 1024 * 1024
    )
      return "Aggregate memory use is high right now.";
    if (
      groupMetricCanDisplay(detail.quality.io, detail.coverage.io) &&
      detail.io_bps >= 500 * 1024
    )
      return "Aggregate read/write I/O is high right now.";
    if (
      !cpuCanDisplay ||
      !groupMetricCanDisplay(detail.quality.memory, detail.coverage.memory) ||
      !groupMetricCanDisplay(detail.quality.io, detail.coverage.io) ||
      !groupMetricCanDisplay(detail.quality.threads, detail.coverage.threads)
    ) {
      return "Some aggregate activity is limited by process telemetry coverage.";
    }
    if (
      detail.coverage.cpu.available < detail.coverage.cpu.total ||
      detail.coverage.memory.available < detail.coverage.memory.total ||
      detail.coverage.io.available < detail.coverage.io.total ||
      detail.coverage.threads.available < detail.coverage.threads.total
    ) {
      return "Coverage is partial; no unusual activity is visible in the reported aggregates.";
    }
    return "No unusual aggregate activity is visible for this group right now.";
  }
</script>

<section class="process-inspector" aria-label="Workload group inspector">
  <div class="process-identity redesigned-identity">
    <span class="identity-icon"><ProcessIcon kind={iconKind} src={iconSrc} /></span>
    <span class="identity-copy">
      <span class="identity-title-row">
        <strong title={detail.label}>{detail.label}</strong>
        <small class="identity-chip">{processCountLabel(detail.process_count)}</small>
      </span>
      <span class="identity-meta-row">
        <small class="identity-category">{detail.category}</small>
        <em class="identity-status tone-normal">Aggregate</em>
      </span>
    </span>
    <span class="identity-actions">
      <button
        class="icon-action inspector-copy"
        type="button"
        aria-label="Copy workload group summary"
        title="Copy summary"
        onclick={onCopy}
      >
        <Copy size={18} weight="regular" aria-hidden="true" />
      </button>
    </span>
  </div>

  <div class="finding-card">
    <span>Group finding</span>
    <h3>{findingCopy()}</h3>
    <p>
      Values below aggregate {processCountLabel(detail.process_count)}. Coverage stays explicit when
      one or more processes cannot contribute a metric.
    </p>
  </div>

  <section class="key-metrics" aria-labelledby="group-key-metrics-title">
    <h3 id="group-key-metrics-title">Aggregate metrics</h3>
    <dl>
      <div class="metric-cpu"><dt>CPU <small>One-core equivalent</small></dt><dd>{displayGroupMetricValue(detail.cpu_percent, detail.quality.cpu, detail.coverage.cpu, formatPercent)}</dd></div>
      <div class="metric-memory"><dt>Memory <small>Bytes</small></dt><dd>{displayGroupMetricValue(detail.memory_bytes, detail.quality.memory, detail.coverage.memory, formatBytes)}</dd></div>
      <div class="metric-disk"><dt>Read/write I/O <small>Bytes/s</small></dt><dd>{displayGroupMetricValue(detail.io_bps, detail.quality.io, detail.coverage.io, formatRate)}</dd></div>
      <div class="metric-network"><dt>Network <small>Bytes/s</small></dt><dd>{networkLabel()}</dd></div>
    </dl>
  </section>

  <div class="inspector-hero-chart">
    <div><span>Aggregate CPU over time</span><strong>{displayGroupMetricValue(detail.cpu_percent, detail.quality.cpu, detail.coverage.cpu, formatPercent)}</strong></div>
    <MiniChart
      values={cpuHistory}
      max={cpuChartMax}
      stroke={activeTheme.cpuStroke}
      fill={activeTheme.cpuFill}
    />
  </div>

  <details class="technical-disclosure inspector-technical" open>
    <summary>Coverage and quality</summary>
    <dl class="key-value-grid technical-grid">
      <div><dt>Processes</dt><dd>{detail.process_count}</dd></div>
      <div><dt>Total threads</dt><dd>{displayGroupMetricValue(detail.threads, detail.quality.threads, detail.coverage.threads, String)}</dd></div>
      <div><dt>CPU</dt><dd>{metricQualityLabel(detail.quality.cpu, "Aggregate")} · {coverageLabel(detail.coverage.cpu)}</dd></div>
      <div><dt>Memory</dt><dd>{metricQualityLabel(detail.quality.memory, "Aggregate")} · {coverageLabel(detail.coverage.memory)}</dd></div>
      <div><dt>Read/write I/O</dt><dd>{metricQualityLabel(detail.quality.io, "Aggregate")} · {coverageLabel(detail.coverage.io)}</dd></div>
      <div><dt>Other I/O</dt><dd>{metricQualityLabel(detail.quality.other_io, "Unavailable")} · {coverageLabel(detail.coverage.other_io)}</dd></div>
      <div><dt>Network</dt><dd>{metricQualityLabel(detail.quality.network, "Aggregate")} · {coverageLabel(detail.coverage.network)}</dd></div>
      <div><dt>Thread coverage</dt><dd>{metricQualityLabel(detail.quality.threads, "Aggregate")} · {coverageLabel(detail.coverage.threads)}</dd></div>
    </dl>
  </details>

  {#if copyStatus}
    <p
      class="copy-status"
      role={copyFailed ? "alert" : "status"}
      aria-live={copyFailed ? "assertive" : "polite"}
    >
      {copyStatus}
    </p>
  {/if}
</section>
