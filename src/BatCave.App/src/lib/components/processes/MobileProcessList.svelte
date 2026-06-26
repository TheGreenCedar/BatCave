<script lang="ts">
  import { processHint, processIdentity, processIoRate, type ProcessRates } from "../../process";
  import { formatPercent, formatRate, processBytesLabel, processMemoryTitle } from "../../format";
  import type { ProcessSample } from "../../types";
  import ProcessIcon from "./ProcessIcon.svelte";

  export let processes: ProcessSample[] = [];
  export let selectedPid = "";
  export let processRates: Record<string, ProcessRates>;
  export let onSelect: (pid: string) => void;
</script>

<div class="mobile-process-list" aria-label="Attention queue cards">
  {#each processes.slice(0, 10) as process}
    {@const identity = processIdentity(process)}
    <button
      class="mobile-process-card"
      class:selected={process.pid === selectedPid}
      type="button"
      aria-pressed={process.pid === selectedPid}
      onclick={() => onSelect(process.pid)}
    >
      <span class="card-title-row">
        <span class="mobile-process-title">
          <ProcessIcon kind={identity.icon} child={identity.isChild} />
          <span>
            <strong>{process.name}</strong>
            <small>{identity.group}</small>
          </span>
        </span>
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
