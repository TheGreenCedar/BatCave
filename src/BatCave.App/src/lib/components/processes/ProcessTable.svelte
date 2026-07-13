<script lang="ts">
  import { ArrowDown, ArrowElbowDownRight, ArrowUp, CaretRight } from "phosphor-svelte";
  import {
    sortAriaValue,
    sortButtonLabel,
    processSelectionKey,
    processRowSecondaryLabel,
    type ProcessColumn,
    type ProcessIconKind,
    type SortKey,
  } from "../../process";
  import {
    displayProcessMetricValue,
    formatPercent,
    formatRate,
    processMemoryTitle,
  } from "../../format";
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
    return displayProcessMetricValue(row.network_bps, quality, formatRate);
  }

  function networkCellTitle(row: ProcessViewRow): string {
    return (row.process ?? row.representative)?.quality?.network?.message ?? "";
  }

  function cpuCellLabel(row: ProcessViewRow): string {
    const quality = (row.process ?? row.representative)?.quality?.cpu;
    return displayProcessMetricValue(row.cpu_percent, quality, formatPercent);
  }

  function ioCellLabel(row: ProcessViewRow): string {
    const quality = (row.process ?? row.representative)?.quality?.io;
    return displayProcessMetricValue(row.io_bps, quality, formatRate);
  }

  function metricCellTitle(row: ProcessViewRow, metric: "cpu" | "io"): string {
    return (row.process ?? row.representative)?.quality?.[metric]?.message ?? "";
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
          {@const secondaryLabel = processRowSecondaryLabel(row)}
          <tr class:group-selected={groupSelected} class="app-group-row">
            {#each columns as column}
              {#if column.key === "name"}
                <td>
                  <div class="process-row-cell">
                    <button
                      class="group-expand"
                      class:expanded
                      type="button"
                      aria-expanded={expanded}
                      aria-label={`${expanded ? "Collapse" : "Expand"} ${row.group_label ?? "process"} group, ${processCountLabel(row.group_count)}`}
                      onclick={() => row.group_key && onToggleGroup(row.group_key)}
                    >
                      <CaretRight size={16} weight="bold" aria-hidden="true" />
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
                        {#if secondaryLabel}<small>{secondaryLabel}</small>{/if}
                      </span>
                    </button>
                  </div>
                </td>
              {:else if column.key === "attention"}
                <td><span class="impact-label">{row.attention_label || "Normal"}</span></td>
              {:else if column.key === "cpu"}
                <td title={metricCellTitle(row, "cpu")}>{cpuCellLabel(row)}</td>
              {:else if column.key === "memory"}
                <td>{representative ? residentMemoryValue({ ...representative, memory_bytes: row.memory_bytes }, platform) : "--"}</td>
              {:else if column.key === "io"}
                <td title={metricCellTitle(row, "io")}>{ioCellLabel(row)}</td>
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
          {@const secondaryLabel = processRowSecondaryLabel(row)}
          <tr class:selected={selectionKey === selectedPid} class:child-row={row.is_grouped}>
            {#each columns as column}
              {#if column.key === "name"}
                <td>
                  <div class="process-row-cell">
                    <span class:child={row.is_grouped} class="hierarchy-gutter" aria-hidden="true">
                      {#if row.is_grouped}<ArrowElbowDownRight size={14} weight="bold" />{/if}
                    </span>
                    <button
                      class="process-button"
                      class:selected={selectionKey === selectedPid}
                      type="button"
                      aria-pressed={selectionKey === selectedPid}
                      aria-label={`Inspect ${process.name}, PID ${process.pid}`}
                      onclick={() => onSelect(selectionKey)}
                    >
                      <ProcessIcon kind={iconKind(row)} src={iconSrc(process)} />
                      <span class="process-name-stack">
                        <span>{process.name}</span>
                        {#if secondaryLabel}<small>{secondaryLabel}</small>{/if}
                      </span>
                    </button>
                  </div>
                </td>
              {:else if column.key === "attention"}
                <td><span class="impact-label">{row.attention_label || "Normal"}</span></td>
              {:else if column.key === "cpu"}
                <td title={metricCellTitle(row, "cpu")}>{cpuCellLabel(row)}</td>
              {:else if column.key === "memory"}
                <td title={processMemoryTitle(process)}>{residentMemoryValue(process, platform)}</td>
              {:else if column.key === "io"}
                <td title={metricCellTitle(row, "io")}>{ioCellLabel(row)}</td>
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
