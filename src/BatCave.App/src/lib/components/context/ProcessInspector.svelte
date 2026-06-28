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

  export let selectedProcess: ProcessSample | null;
  export let processHistory: { cpu: number[]; memory: number[]; readRate: number[]; writeRate: number[]; networkRate: number[] };
  export let processRates: Record<string, ProcessRates> = {};
  export let processReadRate = 0;
  export let processWriteRate = 0;
  export let processIcons: Record<string, string> = {};
  export let copyStatus = "";
  export let activeTheme: ChartPalette;
  export let maxRate: (points: number[], fallback: number) => number;
  export let processNetworkLabel: (process: ProcessSample) => string;
  export let onCopy: () => void;

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
</script>

<section class="process-inspector" aria-label="Process inspector">
  {#if selectedProcess}
    {@const identity = processIdentity(selectedProcess)}
    {@const iconSrc = processIcons[selectedProcess.exe || selectedProcess.name]}
    <div class="process-identity">
      <ProcessIcon kind={identity.icon} child={identity.isChild} src={iconSrc} />
      <span>
        <strong>{selectedProcess.name}</strong>
        <small>{identity.group} / PID {selectedProcess.pid} / {selectedProcess.exe || "Path unavailable"}</small>
      </span>
      <span class="identity-actions">
        <em>{processAccent(selectedProcess, processRates)}</em>
        <button class="subtle-action" type="button" onclick={onCopy}>Copy</button>
      </span>
    </div>
    <div class="trust-strip" aria-label="Selected process metric source">
      <span>{processTrustLabel(selectedProcess)}</span>
      <small>CPU/core</small>
    </div>
    <h3>Resources</h3>
    <div class="resource-list" aria-label="Selected process resources">
      <div class="resource-row">
        <span>CPU/core</span>
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
    <h3>Details</h3>
    <dl class="key-value-grid">
      <div>
        <dt>PID</dt>
        <dd>{selectedProcess.pid}</dd>
      </div>
      <div>
        <dt>Parent</dt>
        <dd>{selectedProcess.parent_pid ?? "--"}</dd>
      </div>
      <div>
        <dt>CPU/core</dt>
        <dd>{formatPercent(selectedProcess.cpu_percent)}</dd>
      </div>
      <div>
        <dt>Kernel CPU</dt>
        <dd>{selectedProcess.kernel_cpu_percent === undefined ? "--" : formatPercent(selectedProcess.kernel_cpu_percent)}</dd>
      </div>
      <div>
        <dt>Working set</dt>
        <dd title={processMemoryTitle(selectedProcess)}>{processBytesLabel(selectedProcess, selectedProcess.memory_bytes)}</dd>
      </div>
      <div>
        <dt>Private</dt>
        <dd title={processMemoryTitle(selectedProcess)}>{processBytesLabel(selectedProcess, selectedProcess.private_bytes)}</dd>
      </div>
      <div>
        <dt>Write rate</dt>
        <dd>{formatRate(processWriteRate)}</dd>
      </div>
      <div>
        <dt>Other I/O</dt>
        <dd>{formatRate(processRates[selectedProcess.pid]?.otherRate ?? selectedProcess.other_io_bps ?? 0)}</dd>
      </div>
      <div>
        <dt>Read total</dt>
        <dd>{formatBytes(selectedProcess.disk_read_total_bytes)}</dd>
      </div>
      <div>
        <dt>Write total</dt>
        <dd>{formatBytes(selectedProcess.disk_write_total_bytes)}</dd>
      </div>
      <div>
        <dt>Threads</dt>
        <dd>{selectedProcess.threads || "--"}</dd>
      </div>
      <div>
        <dt>Handles</dt>
        <dd>{selectedProcess.handles || "--"}</dd>
      </div>
      <div>
        <dt>Access</dt>
        <dd>{accessLabel(selectedProcess.access_state)}</dd>
      </div>
      <div>
        <dt>Network</dt>
        <dd>{processNetworkLabel(selectedProcess)}</dd>
      </div>
      <div>
        <dt>Memory quality</dt>
        <dd>{metricQualityLabel(processMemoryQuality(selectedProcess), "Measured")}</dd>
      </div>
    </dl>
    {#if copyStatus}
      <p class="copy-status" role="status" aria-live="polite">{copyStatus}</p>
    {/if}
  {:else}
    <div class="empty-panel">
      <strong>No selected process</strong>
      <span>Clear the search or change the focus filter to inspect a process.</span>
    </div>
  {/if}
</section>
