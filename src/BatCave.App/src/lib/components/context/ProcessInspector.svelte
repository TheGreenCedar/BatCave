<script lang="ts">
  import { Copy } from "phosphor-svelte";
  import MiniChart from "../../MiniChart.svelte";
  import {
    accessLabel,
    formatBytes,
    formatPercent,
    formatRate,
    metricQualityLabel,
    processMemoryQuality,
  } from "../../format";
  import {
    platformPresentation,
    privateMemoryValue,
    residentMemoryValue,
    type PlatformPresentation,
  } from "../../platformPresentation";
  import { processAccent, processIdentity, processSelectionKey, type ProcessRates } from "../../process";
  import type { ChartPalette } from "../../themes";
  import type { ProcessSample } from "../../types";
  import ProcessIcon from "../processes/ProcessIcon.svelte";

  export let selectedProcess: ProcessSample | null = null;
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

  $: selectedIsGroup = selectedProcess?.pid.startsWith("group:") ?? false;
  $: copyFailed = copyStatus !== "" && copyStatus !== "Process summary copied.";
  $: cpuChartMax = Math.max(100, Math.ceil(Math.max(0, ...processHistory.cpu) / 100) * 100);

  function processTotalIoRate(process: ProcessSample): number {
    const rates = processRates[processSelectionKey(process)];
    return processReadRate + processWriteRate + (rates?.otherRate ?? process.other_io_bps ?? 0);
  }

  function processTrustLabel(process: ProcessSample): string {
    return metricQualityLabel(
      process.quality?.cpu ?? process.quality?.memory ?? process.quality?.io ?? process.quality?.network,
      "Measured",
    );
  }

  function findingCopy(process: ProcessSample): string {
    if (process.cpu_percent >= 30) return "High CPU usage relative to other workloads.";
    if (process.memory_bytes >= 900 * 1024 * 1024) return `High ${presentation.memoryLabel.toLocaleLowerCase()} relative to other workloads.`;
    if (processTotalIoRate(process) >= 500 * 1024) return "High read/write I/O relative to other workloads.";
    return "No unusual activity is visible for this workload right now.";
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
    {@const accent = processAccent(selectedProcess, processRates)}
    {@const categoryLabel = identity.group === "Processes" ? null : identity.group}
    <div class="process-identity redesigned-identity">
      <span class="identity-icon"><ProcessIcon kind={identity.icon} child={identity.isChild} src={iconSrc} /></span>
      <span class="identity-copy">
        <span class="identity-title-row">
          <strong title={selectedProcess.name}>{selectedProcess.name}</strong>
          <small class="identity-chip">{selectedIsGroup ? "Grouped" : `PID ${selectedProcess.pid}`}</small>
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
        {selectedProcess.name} is using {formatPercent(selectedProcess.cpu_percent)} of one CPU core.
        {processTrustLabel(selectedProcess)} data; missing fields stay explicitly unavailable.
      </p>
    </div>

    <section class="key-metrics" aria-labelledby="key-metrics-title">
      <h3 id="key-metrics-title">Key metrics</h3>
      <dl>
        <div class="metric-cpu"><dt>CPU <small>One core</small></dt><dd>{formatPercent(selectedProcess.cpu_percent)}</dd></div>
        <div class="metric-memory"><dt>{presentation.memoryLabel} <small>Bytes</small></dt><dd>{residentMemoryValue(selectedProcess, platform)}</dd></div>
        <div class="metric-disk"><dt>Read/write I/O <small>Bytes/s</small></dt><dd>{formatRate(processTotalIoRate(selectedProcess))}</dd></div>
        <div class="metric-network"><dt>Network <small>Bytes/s</small></dt><dd>{processNetworkLabel(selectedProcess)}</dd></div>
      </dl>
    </section>

    <div class="inspector-hero-chart">
      <div><span>One-core-equivalent CPU over time</span><strong>{formatPercent(selectedProcess.cpu_percent)}</strong></div>
      <MiniChart values={processHistory.cpu} max={cpuChartMax} stroke={activeTheme.cpuStroke} fill={activeTheme.cpuFill} />
    </div>

    <details class="technical-disclosure inspector-technical" open>
      <summary>Technical details</summary>
      <dl class="key-value-grid technical-grid">
        <div><dt>Process ID</dt><dd>{selectedProcess.pid}</dd></div>
        <div><dt>Parent</dt><dd>{selectedProcess.parent_pid ?? "Unavailable"}</dd></div>
        <div><dt>Kernel CPU (one core)</dt><dd>{selectedProcess.kernel_cpu_percent === undefined ? "Unavailable" : formatPercent(selectedProcess.kernel_cpu_percent)}</dd></div>
        <div><dt>{presentation.privateMemoryLabel}</dt><dd>{privateMemoryValue(selectedProcess, platform)}</dd></div>
        <div><dt>Read I/O total</dt><dd>{formatBytes(selectedProcess.io_read_total_bytes)}</dd></div>
        <div><dt>Write I/O total</dt><dd>{formatBytes(selectedProcess.io_write_total_bytes)}</dd></div>
        <div><dt>Threads</dt><dd>{selectedProcess.threads || "Unavailable"}</dd></div>
        <div><dt>{presentation.handlesLabel}</dt><dd>{selectedProcess.handles || "Unavailable"}</dd></div>
        <div><dt>Access</dt><dd>{accessLabel(selectedProcess.access_state)}</dd></div>
        <div><dt>Memory quality</dt><dd>{metricQualityLabel(processMemoryQuality(selectedProcess), "Measured")}</dd></div>
      </dl>
      <div class="technical-path">
        <span>Executable path</span>
        <code>{selectedIsGroup ? "Expand this group to inspect individual executable paths." : selectedProcess.exe || "Path unavailable"}</code>
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
