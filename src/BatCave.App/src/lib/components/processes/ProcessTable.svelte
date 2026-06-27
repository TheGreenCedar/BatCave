<script lang="ts">
  import {
    processIdentity,
    processIoRate,
    processNetworkRate,
    sortAriaValue,
    sortButtonLabel,
    sortIndicator,
    type ProcessColumn,
    type ProcessRates,
    type SortKey,
  } from "../../process";
  import { formatBytes, formatPercent, formatRate, processBytesLabel, processMemoryTitle } from "../../format";
  import type { ProcessSample, SortDirection } from "../../types";
  import ProcessIcon from "./ProcessIcon.svelte";

  export let processes: ProcessSample[] = [];
  export let columns: ProcessColumn[] = [];
  export let selectedPid = "";
  export let sortKey: SortKey;
  export let sortDirection: SortDirection;
  export let processRates: Record<string, ProcessRates>;
  export let processIcons: Record<string, string> = {};
  export let onSelect: (pid: string) => void;
  export let onToggleSort: (key: SortKey) => void;
</script>

<div class="table-wrap">
  <table>
    <thead>
      <tr>
        {#each columns as column}
          <th aria-sort={sortAriaValue(column.key, sortKey, sortDirection)} class:metric={column.metric}>
            <button
              class="sort-header"
              class:active={sortKey === column.key}
              type="button"
              aria-label={sortButtonLabel(column, sortKey, sortDirection)}
              aria-pressed={sortKey === column.key}
              onclick={() => onToggleSort(column.key)}
            >
              <span>{column.label}</span>
              <small aria-hidden="true">{sortIndicator(column.key, sortKey, sortDirection)}</small>
            </button>
          </th>
        {/each}
      </tr>
    </thead>
    <tbody>
      {#each processes as process}
        {@const identity = processIdentity(process)}
        {@const iconSrc = processIcons[process.exe || process.name]}
        <tr class:selected={process.pid === selectedPid} class:child-row={identity.isChild}>
          {#each columns as column}
            {#if column.key === "pid"}
              <td>{process.pid}</td>
            {:else if column.key === "name"}
              <td>
                <button
                  class="process-button"
                  class:selected={process.pid === selectedPid}
                  class:child={identity.isChild}
                  type="button"
                  aria-pressed={process.pid === selectedPid}
                  aria-label={`Inspect ${process.name}, PID ${process.pid}`}
                  onclick={() => onSelect(process.pid)}
                >
                  {#if identity.isChild}
                    <span class="process-tree-branch" aria-hidden="true"></span>
                  {/if}
                  <ProcessIcon kind={identity.icon} child={identity.isChild} src={iconSrc} />
                  <span class="process-name-stack">
                    <span>{process.name}</span>
                    <small>{identity.group}</small>
                  </span>
                </button>
              </td>
            {:else if column.key === "status"}
              <td><span class="status-cell">{process.status}</span></td>
            {:else if column.key === "cpu"}
              <td>{formatPercent(process.cpu_percent)}</td>
            {:else if column.key === "memory"}
              <td title={processMemoryTitle(process)}>{processBytesLabel(process, process.memory_bytes)}</td>
            {:else if column.key === "io"}
              <td>{formatRate(processIoRate(process, processRates))}</td>
            {:else if column.key === "network"}
              <td>{formatRate(processNetworkRate(process))}</td>
            {:else if column.key === "threads"}
              <td>{process.threads}</td>
            {:else}
              <td></td>
            {/if}
          {/each}
        </tr>
      {:else}
        <tr>
          <td class="empty-state" colspan={columns.length}>No process matches this view.</td>
        </tr>
      {/each}
    </tbody>
  </table>
</div>
