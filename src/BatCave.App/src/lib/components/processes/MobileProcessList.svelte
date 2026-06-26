<script lang="ts">
  import { processHint, processIoRate, type ProcessRates } from "../../process";
  import { formatPercent, formatRate, processBytesLabel, processMemoryTitle } from "../../format";
  import type { ProcessSample } from "../../types";

  export let processes: ProcessSample[] = [];
  export let selectedPid = "";
  export let processRates: Record<string, ProcessRates>;
  export let onSelect: (pid: string) => void;
</script>

<div class="mobile-process-list" aria-label="Attention queue cards">
  {#each processes.slice(0, 10) as process}
    <button
      class="mobile-process-card"
      class:selected={process.pid === selectedPid}
      type="button"
      aria-pressed={process.pid === selectedPid}
      onclick={() => onSelect(process.pid)}
    >
      <span class="card-title-row">
        <span>{process.name}</span>
        <small>{processHint(process, processRates)}</small>
      </span>
      <span class="card-metrics">
        <span>
          <em>CPU</em>
          <b>{formatPercent(process.cpu_percent)}</b>
        </span>
        <span>
          <em>Working set</em>
          <b title={processMemoryTitle(process)}>{processBytesLabel(process, process.memory_bytes)}</b>
        </span>
        <span>
          <em>I/O</em>
          <b>{formatRate(processIoRate(process, processRates))}</b>
        </span>
      </span>
      <span class="card-foot">
        <span>PID {process.pid}</span>
        <span>{process.status}</span>
      </span>
    </button>
  {:else}
    <div class="mobile-empty-state">No process matches this view.</div>
  {/each}
</div>
