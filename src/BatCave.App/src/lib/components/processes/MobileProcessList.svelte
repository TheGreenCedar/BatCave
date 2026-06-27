<script lang="ts">
  import { groupProcessesByApp, processHint, processIdentity, type ProcessRates } from "../../process";
  import { formatPercent, formatRate, processBytesLabel, processMemoryTitle } from "../../format";
  import type { ProcessSample } from "../../types";
  import ProcessIcon from "./ProcessIcon.svelte";

  export let processes: ProcessSample[] = [];
  export let selectedPid = "";
  export let processRates: Record<string, ProcessRates> = {};
  export let processIcons: Record<string, string> = {};
  export let onSelect: (pid: string) => void;

  $: processGroups = groupProcessesByApp(processes, processRates);

  function processCountLabel(count: number): string {
    return `${count} ${count === 1 ? "process" : "processes"}`;
  }
</script>

<div class="mobile-process-list" aria-label="Attention queue cards">
  {#each processGroups.slice(0, 10) as group}
    {@const identity = processIdentity(group.representative)}
    {@const iconSrc = processIcons[group.representative.exe || group.representative.name]}
    {@const selectedProcess = group.processes.find((process) => process.pid === selectedPid)}
    <button
      class="mobile-process-card"
      class:selected={!!selectedProcess}
      type="button"
      aria-pressed={!!selectedProcess}
      onclick={() => onSelect(selectedProcess?.pid ?? group.representative.pid)}
    >
      <span class="card-title-row">
        <span class="mobile-process-title">
          <ProcessIcon kind={identity.icon} child={identity.isChild} src={iconSrc} />
          <span>
            <strong>{group.label}</strong>
            <small>{processCountLabel(group.processes.length)} / {group.category}</small>
          </span>
        </span>
        <small>{processHint(group.representative, processRates)}</small>
      </span>
      <span class="card-metrics">
        <span>
          <em>CPU</em>
          <b>{formatPercent(group.cpuPercent)}</b>
        </span>
        <span>
          <em>Working set</em>
          <b title={processMemoryTitle(group.representative)}>{processBytesLabel(group.representative, group.memoryBytes)}</b>
        </span>
        <span>
          <em>I/O</em>
          <b>{formatRate(group.ioRate)}</b>
        </span>
      </span>
      <span class="card-foot">
        <span>{group.processes.length === 1 ? `PID ${group.representative.pid}` : `${group.processes.length} rows`}</span>
        <span>{group.processes.length === 1 ? group.representative.status : "grouped"}</span>
      </span>
    </button>
  {:else}
    <div class="mobile-empty-state">No process matches this view.</div>
  {/each}
</div>
