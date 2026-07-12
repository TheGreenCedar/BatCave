<script lang="ts">
  import { ArrowDown, ArrowUp, CaretRight } from "phosphor-svelte";
  import {
    sortAriaValue,
    sortButtonLabel,
    processSelectionKey,
    type ProcessColumn,
    type ProcessIconKind,
    type SortKey,
  } from "../../process";
  import { formatPercent, formatRate, processMemoryTitle } from "../../format";
  import { residentMemoryValue } from "../../platformPresentation";
  import type { ProcessSample, ProcessViewRow, RuntimePlatform, SortDirection } from "../../types";
  import ProcessIcon from "./ProcessIcon.svelte";

  export let processRows: ProcessViewRow[] = [];
  export let columns: ProcessColumn[] = [];
  export let selectedPid = "";
  export let sortKey: SortKey;
  export let sortDirection: SortDirection;
  export let processIcons: Record<string, string> = {};
  export let expandedGroups: Record<string, boolean> = {};
  export let onSelect: (pid: string) => void;
  export let onToggleSort: (key: SortKey) => void;
  export let onToggleGroup: (key: string) => void = () => {};
  export let onInteractionChange: (active: boolean) => void = () => {};
  export let platform: RuntimePlatform = "fixture";

  function processCountLabel(count: number): string {
    return `${count} ${count === 1 ? "process" : "processes"}`;
  }

  function groupSelectionKey(key: string): string {
    return `group:${key}`;
  }

  function iconSrc(process: ProcessSample | undefined): string | undefined {
    return process ? processIcons[process.exe || process.name] : undefined;
  }

  function iconKind(row: ProcessViewRow): ProcessIconKind {
    return (row.icon_kind as ProcessIconKind) || "process";
  }

  function isGroupSelected(key: string | undefined): boolean {
    return (
      !!key &&
      (selectedPid === groupSelectionKey(key) ||
        processRows.some(
          (row) => row.group_key === key && !!row.process && processSelectionKey(row.process) === selectedPid,
        ))
    );
  }

  function networkCellLabel(row: ProcessViewRow): string {
    const quality = (row.process ?? row.representative)?.quality?.network;
    if (quality?.quality === "unavailable") return "Not available";
    if (quality?.quality === "held") return "Waiting";
    return formatRate(row.network_bps);
  }

  function networkCellTitle(row: ProcessViewRow): string {
    return (row.process ?? row.representative)?.quality?.network?.message ?? "";
  }

  function handleFocusOut(event: FocusEvent & { currentTarget: HTMLDivElement }): void {
    const next = event.relatedTarget;
    if (!(next instanceof Node) || !event.currentTarget.contains(next)) {
      onInteractionChange(false);
    }
  }
</script>

<div
  class="table-wrap attention-table-wrap"
  role="region"
  aria-label="Ranked apps and processes"
  onpointerenter={() => onInteractionChange(true)}
  onpointerleave={() => onInteractionChange(false)}
  onfocusin={() => onInteractionChange(true)}
  onfocusout={handleFocusOut}
>
  <table class="attention-table" class:without-network={!columns.some((column) => column.key === "network")}>
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
              {#if sortKey === column.key}
                <small class="sort-direction-icon" aria-hidden="true">
                  {#if sortDirection === "asc"}
                    <ArrowUp size={13} weight="bold" />
                  {:else}
                    <ArrowDown size={13} weight="bold" />
                  {/if}
                </small>
              {/if}
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
          {@const expanded = row.group_key ? !!expandedGroups[row.group_key] : false}
          <tr class:group-selected={groupSelected} class="app-group-row">
            {#each columns as column}
              {#if column.key === "name"}
                <td>
                  <div class="group-name-cell">
                    <button
                      class="group-expand"
                      class:expanded
                      type="button"
                      aria-expanded={expanded}
                      aria-label={`${expanded ? "Collapse" : "Expand"} ${row.group_label ?? "process"} group, ${processCountLabel(row.group_count)}`}
                      onclick={() => row.group_key && onToggleGroup(row.group_key)}
                    >
                      <CaretRight size={15} weight="bold" aria-hidden="true" />
                    </button>
                    <button
                      class="process-button app-group-button"
                      class:selected={groupSelected}
                      type="button"
                      aria-pressed={groupSelected}
                      aria-label={`Inspect ${row.group_label ?? "process"} group`}
                      onclick={() => row.group_key && onSelect(groupSelectionKey(row.group_key))}
                    >
                      <ProcessIcon kind={iconKind(row)} src={iconSrc(representative)} />
                      <span class="process-name-stack">
                        <span>{row.group_label}</span>
                        <small>{processCountLabel(row.group_count)} / {row.group_category}</small>
                      </span>
                    </button>
                  </div>
                </td>
              {:else if column.key === "attention"}
                <td><span class="impact-label">{row.attention_label || "Normal"}</span></td>
              {:else if column.key === "cpu"}
                <td>{formatPercent(row.cpu_percent)}</td>
              {:else if column.key === "memory"}
                <td>{representative ? residentMemoryValue({ ...representative, memory_bytes: row.memory_bytes }, platform) : "--"}</td>
              {:else if column.key === "io"}
                <td>{formatRate(row.io_bps)}</td>
              {:else if column.key === "network"}
                <td title={networkCellTitle(row)}>{networkCellLabel(row)}</td>
              {:else}
                <td></td>
              {/if}
            {/each}
          </tr>
        {:else if row.process && (!row.is_grouped || !row.group_key || expandedGroups[row.group_key])}
          {@const process = row.process}
          {@const selectionKey = processSelectionKey(process)}
          <tr class:selected={selectionKey === selectedPid} class:child-row={row.is_grouped || row.is_child}>
            {#each columns as column}
              {#if column.key === "name"}
                <td>
                  <button
                    class="process-button"
                    class:selected={selectionKey === selectedPid}
                    class:child={row.is_grouped || row.is_child}
                    type="button"
                    aria-pressed={selectionKey === selectedPid}
                    aria-label={`Inspect ${process.name}, PID ${process.pid}`}
                    onclick={() => onSelect(selectionKey)}
                  >
                    {#if row.is_grouped || row.is_child}<span class="process-tree-branch" aria-hidden="true"></span>{/if}
                    <ProcessIcon kind={iconKind(row)} child={row.is_grouped || row.is_child} src={iconSrc(process)} />
                    <span class="process-name-stack">
                      <span>{process.name}</span>
                      <small>{row.is_grouped ? `PID ${process.pid}` : row.group_category}</small>
                    </span>
                  </button>
                </td>
              {:else if column.key === "attention"}
                <td><span class="impact-label">{row.attention_label || "Normal"}</span></td>
              {:else if column.key === "cpu"}
                <td>{formatPercent(process.cpu_percent)}</td>
              {:else if column.key === "memory"}
                <td title={processMemoryTitle(process)}>{residentMemoryValue(process, platform)}</td>
              {:else if column.key === "io"}
                <td>{formatRate(row.io_bps)}</td>
              {:else if column.key === "network"}
                <td title={networkCellTitle(row)}>{networkCellLabel(row)}</td>
              {:else}
                <td></td>
              {/if}
            {/each}
          </tr>
        {/if}
      {:else}
        <tr><td class="empty-state" colspan={columns.length}>No app or process matches this view.</td></tr>
      {/each}
    </tbody>
  </table>
</div>
