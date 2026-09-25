<script lang="ts">
  import { displayProcessName } from "../../cockpit";
  import { ProcessInteraction } from "../../process";
  import ArrowDown from "phosphor-svelte/lib/ArrowDown";
  import ArrowsDownUp from "phosphor-svelte/lib/ArrowsDownUp";
  import ArrowElbowDownRight from "phosphor-svelte/lib/ArrowElbowDownRight";
  import ArrowUp from "phosphor-svelte/lib/ArrowUp";
  import CaretRight from "phosphor-svelte/lib/CaretRight";
  import {
    sortAriaValue,
    sortButtonLabel,
    processRowSecondaryLabel,
    processViewRowKey,
    processViewRowMetrics,
    workloadSelectionHighlightsRow,
    workloadSelectionMatchesRow,
    type ProcessColumn,
    type ProcessIconKind,
    type SortKey,
  } from "../../process";
  import {
    displayGroupMetricValue,
    displayProcessMetricValue,
    formatBytes,
    formatPercent,
    formatRate,
    processMemoryTitle,
  } from "../../format";
  import { residentMemoryValue } from "../../platformPresentation";
  import {
    resolvedProcessIcon,
    type ResolvedProcessIcon,
    type ResolvedProcessIconCatalog,
  } from "../../processIcons";
  import type { ProcessSample, ProcessViewRow, RuntimePlatform, SortDirection } from "../../types";
  import ProcessIcon from "./ProcessIcon.svelte";

  export let processRows: ProcessViewRow[] = [];
  export let columns: ProcessColumn[] = [];
  export let selectedWorkloadId = "";
  export let sortKey: SortKey;
  export let sortDirection: SortDirection;
  export let processIcons: ResolvedProcessIconCatalog = {};
  export let expandedGroups: Record<string, boolean> = {};
  export let onSelect: (pid: string) => void;
  export let onToggleSort: (key: SortKey) => void;
  export let onToggleGroup: (key: string) => void = () => {};
  export let onInteractionChange: (active: boolean) => void = () => {};
  export let platform: RuntimePlatform = "fixture";
  export let exitedRowKeys: Set<string> = new Set();

  function metricCellTitleFor(row: ProcessViewRow, metric: "cpu" | "io" | "network"): string {
    return row.kind === "group"
      ? (row.detail.quality[metric].message ?? "")
      : (row.detail.process.quality?.[metric]?.message ?? "");
  }

  function processCountLabel(count: number): string {
    return `${count} ${count === 1 ? "process" : "processes"}`;
  }

  function processIcon(process: ProcessSample | undefined): ResolvedProcessIcon {
    return resolvedProcessIcon(processIcons, process ? process.exe || process.name : undefined);
  }

  function iconKind(row: ProcessViewRow): ProcessIconKind {
    return (row.icon_kind as ProcessIconKind) || "process";
  }

  function networkCellLabel(row: ProcessViewRow): string {
    if (row.kind === "group") {
      return displayGroupMetricValue(
        row.detail.network_bps,
        row.detail.quality.network,
        row.detail.coverage.network,
        formatRate,
      );
    }
    const quality = row.detail.process.quality?.network;
    return displayProcessMetricValue(row.detail.network_bps, quality, formatRate);
  }

  function networkCellTitle(row: ProcessViewRow): string {
    return (
      (row.kind === "group" ? row.detail.quality.network : row.detail.process.quality?.network)
        ?.message ?? ""
    );
  }

  function cpuCellLabel(row: ProcessViewRow): string {
    const metrics = processViewRowMetrics(row);
    if (row.kind === "group") {
      return displayGroupMetricValue(
        metrics.cpuPercent,
        row.detail.quality.cpu,
        row.detail.coverage.cpu,
        formatPercent,
      );
    }
    return displayProcessMetricValue(
      metrics.cpuPercent,
      row.detail.process.quality?.cpu,
      formatPercent,
    );
  }

  function ioCellLabel(row: ProcessViewRow): string {
    const metrics = processViewRowMetrics(row);
    if (row.kind === "group") {
      return displayGroupMetricValue(
        metrics.ioBps,
        row.detail.quality.io,
        row.detail.coverage.io,
        formatRate,
      );
    }
    return displayProcessMetricValue(metrics.ioBps, row.detail.process.quality?.io, formatRate);
  }

  function metricCellTitle(row: ProcessViewRow, metric: "cpu" | "io"): string {
    return row.kind === "group"
      ? (row.detail.quality[metric].message ?? "")
      : (row.detail.process.quality?.[metric]?.message ?? "");
  }

  const interaction = new ProcessInteraction();

  function setInteraction(source: "pointer" | "focus", active: boolean): void {
    onInteractionChange(interaction.set(source, active));
  }

  function handleFocusOut(event: FocusEvent & { currentTarget: HTMLDivElement }): void {
    const next = event.relatedTarget;
    if (!(next instanceof Node) || !event.currentTarget.contains(next)) {
      setInteraction("focus", false);
    }
  }
</script>

{#snippet metricCell(ghost: boolean, ghostLabel: string, label: string, title: string)}
  {#if ghost}
    <td class="ghost-cell">{ghostLabel}</td>
  {:else if label === "Unavailable"}
    <td class="metric-unavailable" title={title || "Unavailable"} aria-label="Unavailable">—</td>
  {:else}
    <td title={title}>{label}</td>
  {/if}
{/snippet}

<div
  class="table-wrap attention-table-wrap"
  role="region"
  aria-label="Ranked apps and processes"
  onpointerenter={() => setInteraction("pointer", true)}
  onpointerleave={() => setInteraction("pointer", false)}
  onfocusin={(event) => {
    // Only keyboard focus holds the order; a clicked row keeps focus and would
    // otherwise freeze the ranking until focus moves elsewhere.
    if (event.target instanceof Element && event.target.matches(":focus-visible")) {
      setInteraction("focus", true);
    }
  }}
  onfocusout={handleFocusOut}
>
  <table class="attention-table" class:without-network={!columns.some((column) => column.key === "network")}>
    <thead>
      <tr>
        {#each columns as column}
          <th
            aria-sort={sortAriaValue(column.key, sortKey, sortDirection)}
            class:metric={column.metric}
            title={column.description}
          >
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
              {:else}
                <small class="sort-direction-icon sort-idle" aria-hidden="true">
                  <ArrowsDownUp size={13} weight="regular" />
                </small>
              {/if}
            </button>
          </th>
        {/each}
      </tr>
    </thead>
    <tbody>
      {#each processRows as row (processViewRowKey(row))}
        {@const metrics = processViewRowMetrics(row)}
        {#if row.kind === "group"}
          {@const resolvedIcon = resolvedProcessIcon(processIcons, row.icon_source)}
          {@const groupHighlighted = workloadSelectionHighlightsRow(processRows, row, selectedWorkloadId)}
          {@const groupActionSelected = workloadSelectionMatchesRow(row, selectedWorkloadId)}
          {@const expanded = !!expandedGroups[row.detail.group_key]}
          {@const secondaryLabel = processRowSecondaryLabel(row)}
          {@const ghost = exitedRowKeys.has(processViewRowKey(row))}
          <tr
            class:group-selected={groupHighlighted}
            class="app-group-row"
            class:expanded
            class:exited-row={ghost}
          >
            {#each columns as column}
              {#if column.key === "name"}
                <td>
                  <div class="process-row-cell">
                    <button
                      class="group-expand"
                      class:expanded
                      type="button"
                      aria-expanded={expanded}
                      data-workload-group-key={row.detail.group_key}
                      title={expanded ? "Collapse group" : "Expand group"}
                      aria-label={`${expanded ? "Collapse" : "Expand"} ${row.detail.label} group, ${processCountLabel(row.detail.process_count)}`}
                      onclick={() => onToggleGroup(row.detail.group_key)}
                    >
                      <CaretRight size={16} weight="bold" aria-hidden="true" />
                    </button>
                    <button
                      class="process-button app-group-button"
                      class:selected={groupActionSelected}
                      type="button"
                      aria-pressed={groupActionSelected}
                      aria-label={`Inspect ${row.detail.label} group`}
                      aria-disabled={ghost || undefined}
                      data-workload-id={row.detail.workload_id}
                      onclick={() => {
                        if (!ghost) onSelect(row.detail.workload_id);
                      }}
                    >
                      <ProcessIcon
                        kind={iconKind(row)}
                        src={resolvedIcon.src}
                        matched={resolvedIcon.origin === "name_match"}
                        systemTool={resolvedIcon.systemTool ?? false}
                      />
                      <span class="process-name-stack">
                        <span title={row.detail.label}>{displayProcessName(row.detail.label)}</span>
                        {#if secondaryLabel}<small>{secondaryLabel}</small>{/if}
                      </span>
                    </button>
                  </div>
                </td>
              {:else if column.key === "attention"}
                <td><span class="impact-label">{row.attention_label || "Sampled"}</span></td>
              {:else if column.key === "cpu"}
                {@render metricCell(ghost, "Exited", cpuCellLabel(row), metricCellTitleFor(row, "cpu"))}
              {:else if column.key === "memory"}
                {@render metricCell(
                  ghost,
                  "—",
                  displayGroupMetricValue(metrics.memoryBytes, row.detail.quality.memory, row.detail.coverage.memory, formatBytes),
                  row.detail.quality.memory?.message ?? "",
                )}
              {:else if column.key === "io"}
                {@render metricCell(ghost, "—", ioCellLabel(row), metricCellTitleFor(row, "io"))}
              {:else if column.key === "network"}
                {@render metricCell(ghost, "—", networkCellLabel(row), metricCellTitleFor(row, "network"))}
              {:else}
                <td></td>
              {/if}
            {/each}
          </tr>
        {:else if !row.is_grouped || expandedGroups[row.group_key]}
          {@const process = row.detail.process}
          {@const resolvedIcon = processIcon(process)}
          {@const selectionKey = row.detail.workload_id}
          {@const secondaryLabel = processRowSecondaryLabel(row)}
          {@const ghost = exitedRowKeys.has(processViewRowKey(row))}
          <tr
            class:selected={selectionKey === selectedWorkloadId}
            class:child-row={row.is_grouped}
            class:exited-row={ghost}
          >
            {#each columns as column}
              {#if column.key === "name"}
                <td>
                  <div class="process-row-cell">
                    <span class:child={row.is_grouped} class="hierarchy-gutter" aria-hidden="true">
                      {#if row.is_grouped}<ArrowElbowDownRight size={14} weight="bold" />{/if}
                    </span>
                    <button
                      class="process-button"
                      class:selected={selectionKey === selectedWorkloadId}
                      type="button"
                      aria-pressed={selectionKey === selectedWorkloadId}
                      aria-label={`Inspect ${process.name}, PID ${process.pid}`}
                      aria-disabled={ghost || undefined}
                      data-workload-id={selectionKey}
                      onclick={() => {
                        if (!ghost) onSelect(selectionKey);
                      }}
                    >
                      <ProcessIcon
                        kind={iconKind(row)}
                        src={resolvedIcon.src}
                        matched={resolvedIcon.origin === "name_match"}
                        systemTool={resolvedIcon.systemTool ?? false}
                      />
                      <span class="process-name-stack">
                        <span title={process.exe || process.name}>{displayProcessName(process.name)}</span>
                        {#if secondaryLabel}<small>{secondaryLabel}</small>{/if}
                      </span>
                    </button>
                  </div>
                </td>
              {:else if column.key === "attention"}
                <td><span class="impact-label">{row.attention_label || "Sampled"}</span></td>
              {:else if column.key === "cpu"}
                {@render metricCell(ghost, "Exited", cpuCellLabel(row), metricCellTitleFor(row, "cpu"))}
              {:else if column.key === "memory"}
                {@render metricCell(ghost, "—", residentMemoryValue(process, platform), processMemoryTitle(process))}
              {:else if column.key === "io"}
                {@render metricCell(ghost, "—", ioCellLabel(row), metricCellTitleFor(row, "io"))}
              {:else if column.key === "network"}
                {@render metricCell(ghost, "—", networkCellLabel(row), metricCellTitleFor(row, "network"))}
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
