<script lang="ts">
  import MiniChart from "../../MiniChart.svelte";
  import {
    accessLabel,
    formatBytes,
    formatPercent,
    formatRate,
    metricQualityLabel,
    processBytesLabel,
    processMemoryQuality,
    processMemoryTitle,
  } from "../../format";
  import { processAccent, processIdentity, type ProcessRates } from "../../process";
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
  export let maxRate: (points: number[], fallback: number) => number;
  export let processNetworkLabel: (process: ProcessSample) => string;
  export let onCopy: () => void;

  let activeTab: "overview" | "resources" | "technical" = "overview";
  let previousPid = "";

  $: selectedIsGroup = selectedProcess?.pid.startsWith("group:") ?? false;
  $: if ((selectedProcess?.pid ?? "") !== previousPid) {
    previousPid = selectedProcess?.pid ?? "";
    activeTab = "overview";
  }

  function processTotalIoRate(process: ProcessSample): number {
    const rates = processRates[process.pid];
    return processReadRate + processWriteRate + (rates?.otherRate ?? process.other_io_bps ?? 0);
  }

  function processTrustLabel(process: ProcessSample): string {
    return metricQualityLabel(
      process.quality?.cpu ?? process.quality?.memory ?? process.quality?.disk ?? process.quality?.network,
      "Measured",
    );
  }

  function findingCopy(process: ProcessSample): string {
    if (process.cpu_percent >= 30) return "CPU is the dominant signal for this workload right now.";
    if (process.memory_bytes >= 900 * 1024 * 1024) return "Memory is the dominant signal for this workload right now.";
    if (processTotalIoRate(process) >= 500 * 1024) return "Disk activity is the dominant signal for this workload right now.";
    return "This workload is visible but is not creating unusual pressure right now.";
  }
</script>

<section class="process-inspector" aria-label="Process inspector">
  {#if selectedProcess}
    {@const identity = processIdentity(selectedProcess)}
    {@const iconSrc = processIcons[selectedProcess.exe || selectedProcess.name]}
    <div class="process-identity redesigned-identity">
      <ProcessIcon kind={identity.icon} child={identity.isChild} src={iconSrc} />
      <span>
        <strong>{selectedProcess.name}</strong>
        <small>{selectedIsGroup ? `${identity.group} / grouped workload` : `${identity.group} / PID ${selectedProcess.pid}`}</small>
      </span>
      <span class="identity-actions">
        <em>{processAccent(selectedProcess, processRates)}</em>
        <button class="subtle-action" type="button" onclick={onCopy}>Copy summary</button>
      </span>
    </div>

    <div class="inspector-tabs" role="tablist" aria-label="Selected workload detail">
      <button
        class:active={activeTab === "overview"}
        type="button"
        role="tab"
        aria-selected={activeTab === "overview"}
        onclick={() => (activeTab = "overview")}
      >Overview</button>
      <button
        class:active={activeTab === "resources"}
        type="button"
        role="tab"
        aria-selected={activeTab === "resources"}
        onclick={() => (activeTab = "resources")}
      >Resources</button>
      <button
        class:active={activeTab === "technical"}
        type="button"
        role="tab"
        aria-selected={activeTab === "technical"}
        onclick={() => (activeTab = "technical")}
      >Technical</button>
    </div>

    {#if activeTab === "overview"}
      <div class="finding-card">
        <span>Current finding</span>
        <h3>{findingCopy(selectedProcess)}</h3>
        <p>{processTrustLabel(selectedProcess)} CPU data. Missing fields stay marked instead of being estimated silently.</p>
      </div>
      <div class="overview-metrics">
        <div><span>CPU</span><strong>{formatPercent(selectedProcess.cpu_percent)}</strong></div>
        <div><span>Memory</span><strong>{processBytesLabel(selectedProcess, selectedProcess.memory_bytes)}</strong></div>
        <div><span>I/O</span><strong>{formatRate(processTotalIoRate(selectedProcess))}</strong></div>
        <div><span>Network</span><strong>{processNetworkLabel(selectedProcess)}</strong></div>
      </div>
      <div class="inspector-hero-chart">
        <div><span>CPU trend</span><strong>{formatPercent(selectedProcess.cpu_percent)}</strong></div>
        <MiniChart values={processHistory.cpu} max={100} stroke={activeTheme.cpuStroke} fill={activeTheme.cpuFill} />
      </div>
    {:else if activeTab === "resources"}
      <div class="resource-list" aria-label="Selected process resources">
        <div class="resource-row">
          <span>CPU</span>
          <strong>{formatPercent(selectedProcess.cpu_percent)}</strong>
          <MiniChart values={processHistory.cpu} max={100} stroke={activeTheme.cpuStroke} fill={activeTheme.cpuFill} />
        </div>
        <div class="resource-row">
          <span>Memory</span>
          <strong title={processMemoryTitle(selectedProcess)}>{processBytesLabel(selectedProcess, selectedProcess.memory_bytes)}</strong>
          <MiniChart values={processHistory.memory} max={100} stroke={activeTheme.memoryStroke} fill={activeTheme.memoryFill} />
        </div>
        <div class="resource-row">
          <span>Disk I/O</span>
          <strong>{formatRate(processTotalIoRate(selectedProcess))}</strong>
          <MiniChart
            values={processHistory.readRate}
            max={maxRate([...processHistory.readRate, ...processHistory.writeRate], 250_000)}
            stroke={activeTheme.diskReadStroke}
            fill={activeTheme.diskReadFill}
          />
        </div>
        <div class="resource-row">
          <span>Network</span>
          <strong class="stacked-value">
            <span>{processNetworkLabel(selectedProcess)}</span>
            <small>{metricQualityLabel(selectedProcess.quality?.network, "Measured")}</small>
          </strong>
          <MiniChart
            values={processHistory.networkRate}
            max={maxRate(processHistory.networkRate, 250_000)}
            stroke={activeTheme.networkDownStroke}
            fill={activeTheme.networkDownFill}
          />
        </div>
      </div>
    {:else}
      <dl class="key-value-grid technical-grid">
        <div><dt>PID</dt><dd>{selectedProcess.pid}</dd></div>
        <div><dt>Parent</dt><dd>{selectedProcess.parent_pid ?? "--"}</dd></div>
        <div><dt>Kernel CPU</dt><dd>{selectedProcess.kernel_cpu_percent === undefined ? "--" : formatPercent(selectedProcess.kernel_cpu_percent)}</dd></div>
        <div><dt>Private memory</dt><dd>{processBytesLabel(selectedProcess, selectedProcess.private_bytes)}</dd></div>
        <div><dt>Read total</dt><dd>{formatBytes(selectedProcess.disk_read_total_bytes)}</dd></div>
        <div><dt>Write total</dt><dd>{formatBytes(selectedProcess.disk_write_total_bytes)}</dd></div>
        <div><dt>Threads</dt><dd>{selectedProcess.threads || "--"}</dd></div>
        <div><dt>Handles</dt><dd>{selectedProcess.handles || "--"}</dd></div>
        <div><dt>Access</dt><dd>{accessLabel(selectedProcess.access_state)}</dd></div>
        <div><dt>Memory quality</dt><dd>{metricQualityLabel(processMemoryQuality(selectedProcess), "Measured")}</dd></div>
      </dl>
      <div class="technical-path">
        <span>Executable path</span>
        <code>{selectedIsGroup ? "Expand this group to inspect individual executable paths." : selectedProcess.exe || "Path unavailable"}</code>
      </div>
    {/if}

    {#if copyStatus}<p class="copy-status" role="status" aria-live="polite">{copyStatus}</p>{/if}
  {:else}
    <div class="empty-panel">
      <strong>The selected workload is no longer available</strong>
      <span>Return to the system overview or choose another row from the attention queue.</span>
    </div>
  {/if}
</section>
