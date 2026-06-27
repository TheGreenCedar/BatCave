<script lang="ts">
  import {
    groupProcessesByApp,
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
  import { formatPercent, formatRate, processBytesLabel, processMemoryTitle } from "../../format";
  import type { ProcessSample, SortDirection } from "../../types";
  import ProcessIcon from "./ProcessIcon.svelte";

  export let processes: ProcessSample[] = [];
  export let columns: ProcessColumn[] = [];
  export let selectedPid = "";
  export let sortKey: SortKey;
  export let sortDirection: SortDirection;
  export let processRates: Record<string, ProcessRates> = {};
  export let processIcons: Record<string, string> = {};
  export let onSelect: (pid: string) => void;
  export let onToggleSort: (key: SortKey) => void;

  $: processGroups = groupProcessesByApp(processes, processRates);
  let collapsedGroups: Record<string, boolean> = {};

  function processCountLabel(count: number): string {
    return `${count} ${count === 1 ? "process" : "processes"}`;
  }

  function isGroupCollapsed(key: string): boolean {
    return collapsedGroups[key] ?? false;
  }

  function toggleGroup(key: string): void {
    collapsedGroups = { ...collapsedGroups, [key]: !isGroupCollapsed(key) };
  }
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
      {#each processGroups as group}
        {@const groupIdentity = processIdentity(group.representative)}
        {@const groupIconSrc = processIcons[group.representative.exe || group.representative.name]}
        {@const groupSelected = group.processes.some((process) => process.pid === selectedPid)}
        {@const collapsed = isGroupCollapsed(group.key)}
        {#if group.processes.length > 1}
          <tr class:group-selected={groupSelected} class="app-group-row">
            {#each columns as column}
              {#if column.key === "pid"}
                <td></td>
              {:else if column.key === "name"}
                <td>
                  <button
                    class="process-button app-group-button"
                    class:selected={groupSelected}
                    type="button"
                    aria-expanded={!collapsed}
                    aria-label={`${collapsed ? "Expand" : "Collapse"} ${group.label} group, ${processCountLabel(group.processes.length)}`}
                    onclick={() => toggleGroup(group.key)}
                  >
                    <span class="group-toggle-indicator" class:collapsed aria-hidden="true">
                      <svg viewBox="0 0 16 16">
                        <path d="M5.5 3.5 10 8l-4.5 4.5" />
                      </svg>
                    </span>
                    <ProcessIcon kind={groupIdentity.icon} src={groupIconSrc} />
                    <span class="process-name-stack">
                      <span>{group.label}</span>
                      <small>{processCountLabel(group.processes.length)} / {group.category}</small>
                    </span>
                  </button>
                </td>
              {:else if column.key === "status"}
                <td></td>
              {:else if column.key === "cpu"}
                <td>{formatPercent(group.cpuPercent)}</td>
              {:else if column.key === "memory"}
                <td>{processBytesLabel(group.representative, group.memoryBytes)}</td>
              {:else if column.key === "io"}
                <td>{formatRate(group.ioRate)}</td>
              {:else if column.key === "network"}
                <td>{formatRate(group.networkRate)}</td>
              {:else if column.key === "threads"}
                <td>{group.threads}</td>
              {:else}
                <td></td>
              {/if}
            {/each}
          </tr>
        {/if}
        {#if group.processes.length === 1 || !collapsed}
          {#each group.processes as process}
            {@const identity = processIdentity(process)}
            {@const iconSrc = processIcons[process.exe || process.name]}
            <tr
              class:selected={process.pid === selectedPid}
              class:child-row={group.processes.length > 1 || identity.isChild}
              class:app-process-row={group.processes.length > 1}
            >
              {#each columns as column}
                {#if column.key === "pid"}
                  <td>{process.pid}</td>
                {:else if column.key === "name"}
                  <td>
                    <button
                      class="process-button"
                      class:selected={process.pid === selectedPid}
                      class:child={group.processes.length > 1 || identity.isChild}
                      type="button"
                      aria-pressed={process.pid === selectedPid}
                      aria-label={`Inspect ${process.name}, PID ${process.pid}`}
                      onclick={() => onSelect(process.pid)}
                    >
                      {#if group.processes.length > 1 || identity.isChild}
                        <span class="process-tree-branch" aria-hidden="true"></span>
                      {/if}
                      <ProcessIcon kind={identity.icon} child={group.processes.length > 1 || identity.isChild} src={iconSrc} />
                      <span class="process-name-stack">
                        <span>{process.name}</span>
                        <small>{group.processes.length > 1 ? `PID ${process.pid}` : group.category}</small>
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
          {/each}
        {/if}
      {:else}
        <tr>
          <td class="empty-state" colspan={columns.length}>No process matches this view.</td>
        </tr>
      {/each}
    </tbody>
  </table>
</div>
