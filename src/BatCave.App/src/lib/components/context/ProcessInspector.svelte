<script lang="ts">
  import { Copy } from "phosphor-svelte";
  import MiniChart from "../../MiniChart.svelte";
  import {
    accessLabel,
    displayProcessMetricValue,
    formatBytes,
    formatOptionalRate,
    formatPercent,
    formatRate,
    metricQualityLabel,
    processActivityLabel,
    processFindingLabel,
    processMemoryQuality,
    processTrustLabel,
  } from "../../format";
  import {
    platformPresentation,
    privateMemoryValue,
    residentMemoryValue,
    type PlatformPresentation,
  } from "../../platformPresentation";
  import {
    processIdentity,
    processOtherIoRate,
    type ProcessRates,
  } from "../../process";
  import type { ChartPalette } from "../../themes";
  import type { ProcessDetail, ProcessSample } from "../../types";
  import ProcessIcon from "../processes/ProcessIcon.svelte";

  // oxlint-disable-next-line no-unassigned-vars -- Svelte assigns this required component prop.
  export let detail: ProcessDetail;
  export let processHistory: {
    cpu: number[];
    memory: number[];
    readRate: number[];
    writeRate: number[];
    networkRate: number[];
  } = { cpu: [], memory: [], readRate: [], writeRate: [], networkRate: [] };
  export let processRates: Record<string, ProcessRates> = {};
  export let processReadRate = 0;
  export let processWriteRate = 0;
  export let processIcons: Record<string, string> = {};
  export let copyStatus = "";
  export let activeTheme: ChartPalette;
  export let presentation: PlatformPresentation = platformPresentation({ platform: "fixture" });
  export let platform: "windows" | "linux" | "macos" | "fixture" = "fixture";
  export let processNetworkLabel: (process: ProcessSample) => string;
  export let onCopy: () => void;

  $: selectedProcess = detail.process;
  $: copyFailed = copyStatus !== "" && copyStatus !== "Workload summary copied.";
  $: cpuChartMax = Math.max(100, Math.ceil(Math.max(0, ...processHistory.cpu) / 100) * 100);

  function processReadWriteIoRate(): number {
    return processReadRate + processWriteRate;
  }

  function processNetworkRate(process: ProcessSample): number {
    return (process.network_received_bps ?? 0) + (process.network_transmitted_bps ?? 0);
  }

  function processCpuLabel(process: ProcessSample): string {
    return displayProcessMetricValue(process.cpu_percent, process.quality?.cpu, formatPercent);
  }

  function processIoLabel(process: ProcessSample): string {
    return displayProcessMetricValue(processReadWriteIoRate(), process.quality?.io, formatRate);
  }

  function processIoTotalLabel(process: ProcessSample, value: number): string {
    return displayProcessMetricValue(value, process.quality?.io, formatBytes);
  }

  function processOtherIoLabel(process: ProcessSample): string {
    return displayProcessMetricValue(
      processOtherIoRate(process, processRates),
      process.quality?.other_io,
      formatOptionalRate,
    );
  }

  function processOtherIoTotalLabel(process: ProcessSample): string {
    return displayProcessMetricValue(
      process.other_io_total_bytes,
      process.quality?.other_io,
      (value) => (value === undefined ? "Unavailable" : formatBytes(value)),
    );
  }

  function findingCopy(process: ProcessSample): string {
    return processFindingLabel(
      process,
      processReadWriteIoRate(),
      processNetworkRate(process),
      presentation.memoryLabel,
    );
  }

  function accentTone(accent: string): "hot" | "heavy" | "io" | "normal" {
    if (accent === "Hot") return "hot";
    if (accent === "Heavy") return "heavy";
    if (accent === "I/O") return "io";
    return "normal";
  }
</script>

<section class="process-inspector" aria-label="Workload inspector">
  {#if selectedProcess}
    {@const identity = processIdentity(selectedProcess)}
    {@const iconSrc = processIcons[selectedProcess.exe || selectedProcess.name]}
    {@const accent = processActivityLabel(selectedProcess, processReadWriteIoRate(), processNetworkRate(selectedProcess))}
    {@const categoryLabel = identity.group === "Processes" ? null : identity.group}
    <div class="process-identity redesigned-identity">
      <span class="identity-icon"><ProcessIcon kind={identity.icon} child={identity.isChild} src={iconSrc} /></span>
      <span class="identity-copy">
        <span class="identity-title-row">
          <strong title={selectedProcess.name}>{selectedProcess.name}</strong>
          <small class="identity-chip">PID {selectedProcess.pid}</small>
        </span>
        <span class="identity-meta-row">
          {#if categoryLabel}<small class="identity-category">{categoryLabel}</small>{/if}
          <em class={`identity-status tone-${accentTone(accent)}`}>{accent}</em>
        </span>
      </span>
      <span class="identity-actions">
        <button
          class="icon-action inspector-copy"
          type="button"
          aria-label="Copy workload summary"
          title="Copy summary"
          onclick={onCopy}
        >
          <Copy size={18} weight="regular" aria-hidden="true" />
        </button>
      </span>
    </div>

    <div class="finding-card">
      <span>Finding</span>
      <h3>{findingCopy(selectedProcess)}</h3>
      <p>
        {selectedProcess.name} CPU (one core): {processCpuLabel(selectedProcess)}.
        Telemetry coverage: {processTrustLabel(selectedProcess)}; missing fields stay explicitly unavailable.
      </p>
    </div>

    <section class="key-metrics" aria-labelledby="key-metrics-title">
      <h3 id="key-metrics-title">Key metrics</h3>
      <dl>
        <div class="metric-cpu"><dt>CPU <small>One core</small></dt><dd>{processCpuLabel(selectedProcess)}</dd></div>
        <div class="metric-memory"><dt>{presentation.memoryLabel} <small>Bytes</small></dt><dd>{residentMemoryValue(selectedProcess, platform)}</dd></div>
        <div class="metric-disk"><dt>Read/write I/O <small>Bytes/s</small></dt><dd>{processIoLabel(selectedProcess)}</dd></div>
        <div class="metric-network"><dt>Network <small>Bytes/s</small></dt><dd>{processNetworkLabel(selectedProcess)}</dd></div>
      </dl>
    </section>

    <div class="inspector-hero-chart">
      <div><span>One-core-equivalent CPU over time</span><strong>{processCpuLabel(selectedProcess)}</strong></div>
      <MiniChart values={processHistory.cpu} max={cpuChartMax} stroke={activeTheme.cpuStroke} fill={activeTheme.cpuFill} />
    </div>

    <details class="technical-disclosure inspector-technical" open>
      <summary>Technical details</summary>
      <dl class="key-value-grid technical-grid">
        <div><dt>Process ID</dt><dd>{selectedProcess.pid}</dd></div>
        <div><dt>Parent</dt><dd>{selectedProcess.parent_pid ?? "Unavailable"}</dd></div>
        <div><dt>Kernel CPU (one core)</dt><dd>{selectedProcess.kernel_cpu_percent === undefined ? "Unavailable" : displayProcessMetricValue(selectedProcess.kernel_cpu_percent, selectedProcess.quality?.cpu, formatPercent)}</dd></div>
        <div><dt>{presentation.privateMemoryLabel}</dt><dd>{privateMemoryValue(selectedProcess, platform)}</dd></div>
        <div><dt>Read I/O total</dt><dd>{processIoTotalLabel(selectedProcess, selectedProcess.io_read_total_bytes)}</dd></div>
        <div><dt>Write I/O total</dt><dd>{processIoTotalLabel(selectedProcess, selectedProcess.io_write_total_bytes)}</dd></div>
        <div><dt>Other I/O rate</dt><dd>{processOtherIoLabel(selectedProcess)}</dd></div>
        <div><dt>Other I/O total</dt><dd>{processOtherIoTotalLabel(selectedProcess)}</dd></div>
        <div><dt>Threads</dt><dd>{displayProcessMetricValue(selectedProcess.threads, selectedProcess.quality?.threads, String)}</dd></div>
        <div><dt>{presentation.handlesLabel}</dt><dd>{displayProcessMetricValue(selectedProcess.handles, selectedProcess.quality?.handles, String)}</dd></div>
        <div><dt>Access</dt><dd>{accessLabel(selectedProcess.access_state)}</dd></div>
        <div><dt>Memory quality</dt><dd>{metricQualityLabel(processMemoryQuality(selectedProcess), "Quality not reported")}</dd></div>
      </dl>
      <div class="technical-path">
        <span>Executable path</span>
        <code>{selectedProcess.exe || "Path unavailable"}</code>
      </div>
    </details>

    {#if copyStatus}
      <p class="copy-status" role={copyFailed ? "alert" : "status"} aria-live={copyFailed ? "assertive" : "polite"}>
        {copyStatus}
      </p>
    {/if}
  {:else}
    <div class="empty-panel">
      <strong>The selected workload is no longer available</strong>
      <span>Return to the system overview or choose another row from the workload queue.</span>
    </div>
  {/if}
</section>
