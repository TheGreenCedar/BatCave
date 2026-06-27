<script lang="ts">
  import {
    sortAriaValue,
    sortButtonLabel,
    sortIndicator,
    type ProcessColumn,
    type ProcessIconKind,
    type SortKey,
  } from "../../process";
  import { formatPercent, formatRate, processBytesLabel, processMemoryTitle } from "../../format";
  import type { ProcessSample, ProcessViewRow, SortDirection } from "../../types";
  import ProcessIcon from "./ProcessIcon.svelte";

  export let processRows: ProcessViewRow[] = [];
  export let columns: ProcessColumn[] = [];
  export let selectedPid = "";
  export let sortKey: SortKey;
  export let sortDirection: SortDirection;
  export let processIcons: Record<string, string> = {};
  export let onSelect: (pid: string) => void;
  export let onToggleSort: (key: SortKey) => void;

  let collapsedGroups: Record<string, boolean> = {};

  function processCountLabel(count: number): string {
    return `${count} ${count === 1 ? "process" : "processes"}`;
  }

  function isGroupCollapsed(key: string): boolean {
    return collapsedGroups[key] ?? true;
  }

  function toggleGroup(key: string): void {
    collapsedGroups = { ...collapsedGroups, [key]: !isGroupCollapsed(key) };
  }

  function processForRow(row: ProcessViewRow): ProcessSample | undefined {
    return row.process ?? row.representative;
  }

  function iconSrc(process: ProcessSample | undefined): string | undefined {
    return process ? processIcons[process.exe || process.name] : undefined;
  }

  function iconKind(row: ProcessViewRow): ProcessIconKind {
    return (row.icon_kind as ProcessIconKind) || "process";
  }

  function isGroupSelected(key: string | undefined): boolean {
    return !!key && processRows.some((row) => row.group_key === key && row.process?.pid === selectedPid);
  }

  function isVisibleProcessRow(row: ProcessViewRow): boolean {
    return !row.is_grouped || !row.group_key || !isGroupCollapsed(row.group_key);
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
      {#each processRows as row}
        {#if row.kind === "group"}
          {@const representative = row.representative}
          {@const groupSelected = isGroupSelected(row.group_key)}
          {@const collapsed = row.group_key ? isGroupCollapsed(row.group_key) : false}
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
                    aria-label={`${collapsed ? "Expand" : "Collapse"} ${row.group_label ?? "process"} group, ${processCountLabel(row.group_count)}`}
                    onclick={() => row.group_key && toggleGroup(row.group_key)}
                  >
                    <span class="group-toggle-indicator" class:collapsed aria-hidden="true">
                      <svg viewBox="0 0 16 16">
                        <path d="M5.5 3.5 10 8l-4.5 4.5" />
                      </svg>
                    </span>
                    <ProcessIcon kind={iconKind(row)} src={iconSrc(representative)} />
                    <span class="process-name-stack">
                      <span>{row.group_label}</span>
                      <small>{processCountLabel(row.group_count)} / {row.group_category}</small>
                      <span class="process-group-stats" aria-hidden="true">
                        <span>CPU {formatPercent(row.cpu_percent)}</span>
                        <span>{representative ? processBytesLabel(representative, row.memory_bytes) : ""}</span>
                        <span>I/O {formatRate(row.io_bps)}</span>
                        <span>Net {formatRate(row.network_bps)}</span>
                      </span>
                    </span>
                  </button>
                </td>
              {:else if column.key === "status"}
                <td></td>
              {:else if column.key === "cpu"}
                <td>{formatPercent(row.cpu_percent)}</td>
              {:else if column.key === "memory"}
                <td>{representative ? processBytesLabel(representative, row.memory_bytes) : ""}</td>
              {:else if column.key === "io"}
                <td>{formatRate(row.io_bps)}</td>
              {:else if column.key === "network"}
                <td>{formatRate(row.network_bps)}</td>
              {:else if column.key === "threads"}
                <td>{row.threads}</td>
              {:else}
                <td></td>
              {/if}
            {/each}
          </tr>
        {:else if row.process && isVisibleProcessRow(row)}
          {@const process = row.process}
            <tr
              class:selected={process.pid === selectedPid}
              class:child-row={row.is_grouped || row.is_child}
              class:app-process-row={row.is_grouped}
            >
              {#each columns as column}
                {#if column.key === "pid"}
                  <td>{process.pid}</td>
                {:else if column.key === "name"}
                  <td>
                    <button
                      class="process-button"
                      class:selected={process.pid === selectedPid}
                      class:child={row.is_grouped || row.is_child}
                      type="button"
                      aria-pressed={process.pid === selectedPid}
                      aria-label={`Inspect ${process.name}, PID ${process.pid}`}
                      onclick={() => onSelect(process.pid)}
                    >
                      {#if row.is_grouped || row.is_child}
                        <span class="process-tree-branch" aria-hidden="true"></span>
                      {/if}
                      <ProcessIcon kind={iconKind(row)} child={row.is_grouped || row.is_child} src={iconSrc(process)} />
                      <span class="process-name-stack">
                        <span>{process.name}</span>
                        <small>{row.is_grouped ? `PID ${process.pid}` : row.group_category}</small>
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
                  <td>{formatRate(row.io_bps)}</td>
                {:else if column.key === "network"}
                  <td>{formatRate(row.network_bps)}</td>
                {:else if column.key === "threads"}
                  <td>{process.threads}</td>
                {:else}
                  <td></td>
                {/if}
              {/each}
            </tr>
        {/if}
      {:else}
        <tr>
          <td class="empty-state" colspan={columns.length}>No process matches this view.</td>
        </tr>
      {/each}
    </tbody>
  </table>
</div>
