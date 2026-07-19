<script lang="ts">
  import CaretRight from "phosphor-svelte/lib/CaretRight";
  import {
    processRowSecondaryLabel,
    processViewRowKey,
    processViewRowMetrics,
    workloadSelectionHighlightsRow,
    workloadSelectionMatchesRow,
    type ProcessIconKind,
  } from "../../process";
  import {
    displayGroupMetricValue,
    displayProcessMetricValue,
    formatBytes,
    formatPercent,
    formatRate,
    processMemoryTitle,
  } from "../../format";
  import { platformPresentation, residentMemoryValue } from "../../platformPresentation";
  import type { ProcessSample, ProcessViewRow, RuntimePlatform } from "../../types";
  import ProcessIcon from "./ProcessIcon.svelte";

  export let processRows: ProcessViewRow[] = [];
  export let selectedWorkloadId = "";
  export let processIcons: Record<string, string> = {};
  export let expandedGroups: Record<string, boolean> = {};
  export let onSelect: (pid: string) => void = () => {};
  export let onToggleGroup: (key: string) => void = () => {};
  export let onInteractionChange: (active: boolean) => void = () => {};
  export let platform: RuntimePlatform = "fixture";

  $: cardRows = processRows.filter(
    (row) =>
      row.kind === "group" || !row.is_grouped || !!expandedGroups[row.group_key],
  );
  $: presentation = platformPresentation({ platform });

  function processCountLabel(count: number): string {
    return `${count} ${count === 1 ? "process" : "processes"}`;
  }

  function processForRow(row: ProcessViewRow): ProcessSample | undefined {
    return row.kind === "process" ? row.detail.process : undefined;
  }

  function iconSrc(process: ProcessSample | undefined): string | undefined {
    return process ? processIcons[process.exe || process.name] : undefined;
  }

  function iconKind(row: ProcessViewRow): ProcessIconKind {
    return (row.icon_kind as ProcessIconKind) || "process";
  }

  function selectRow(row: ProcessViewRow): void {
    onSelect(row.detail.workload_id);
  }

  function networkLabel(row: ProcessViewRow): string {
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

  function handleFocusOut(event: FocusEvent & { currentTarget: HTMLDivElement }): void {
    const next = event.relatedTarget;
    if (!(next instanceof Node) || !event.currentTarget.contains(next)) {
      onInteractionChange(false);
    }
  }

  function cpuLabel(row: ProcessViewRow): string {
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

  function ioLabel(row: ProcessViewRow): string {
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
</script>

<div
  class="mobile-process-list"
  role="region"
  aria-label="Attention queue cards"
  onpointerenter={() => onInteractionChange(true)}
  onpointerleave={() => onInteractionChange(false)}
  onfocusin={() => onInteractionChange(true)}
  onfocusout={handleFocusOut}
>
  {#each cardRows as row (processViewRowKey(row))}
    {@const process = processForRow(row)}
    {@const metrics = processViewRowMetrics(row)}
    {@const highlighted = workloadSelectionHighlightsRow(processRows, row, selectedWorkloadId)}
    {@const actionSelected = workloadSelectionMatchesRow(row, selectedWorkloadId)}
    {@const expanded = row.kind === "group" ? !!expandedGroups[row.detail.group_key] : false}
    {@const secondaryLabel = processRowSecondaryLabel(row)}
    <article
      class="mobile-process-card"
      class:selected={highlighted}
      class:child-card={row.kind === "process" && row.is_grouped}
    >
      <button
        class="mobile-card-select"
        type="button"
        aria-pressed={actionSelected}
        aria-label={row.kind === "group" ? `Inspect ${row.detail.label} group` : `Inspect ${process?.name}, PID ${process?.pid}`}
        data-workload-id={row.detail.workload_id}
        onclick={() => selectRow(row)}
      >
        <span class="card-title-row">
          <span class="mobile-process-title">
            <ProcessIcon
              kind={iconKind(row)}
              child={row.kind === "process" && row.is_grouped}
              src={row.kind === "group" && row.icon_source ? processIcons[row.icon_source] : iconSrc(process)}
            />
            <span>
              <strong>{row.kind === "group" ? row.detail.label : process?.name}</strong>
              {#if secondaryLabel}<small>{secondaryLabel}</small>{/if}
            </span>
          </span>
          <small>{row.attention_label}</small>
        </span>
        <span class="card-metrics">
          <span>
            <em>CPU / core</em>
            <b title={process?.quality?.cpu?.message ?? ""}>{cpuLabel(row)}</b>
          </span>
          <span>
            <em>{presentation.memoryLabel}</em>
            <b title={process ? processMemoryTitle(process) : ""}>{row.kind === "process" ? residentMemoryValue(row.detail.process, platform) : displayGroupMetricValue(metrics.memoryBytes, row.detail.quality.memory, row.detail.coverage.memory, formatBytes)}</b>
          </span>
          <span>
            <em>I/O</em>
            <b title={process?.quality?.io?.message ?? ""}>{ioLabel(row)}</b>
          </span>
          <span>
            <em>Network</em>
            <b>{networkLabel(row)}</b>
          </span>
        </span>
        <span class="card-foot">
          <span>{row.kind === "group" ? processCountLabel(row.detail.process_count) : `PID ${process?.pid}`}</span>
          <span>{row.kind === "group" ? "Aggregate" : process?.status}</span>
        </span>
      </button>
      {#if row.kind === "group"}
        <button
          class="mobile-group-expand"
          type="button"
          aria-expanded={expanded}
          onclick={() => onToggleGroup(row.detail.group_key)}
        >
          <CaretRight class={expanded ? "expanded" : ""} size={15} weight="bold" aria-hidden="true" />
          {expanded ? "Collapse" : "Expand"} {processCountLabel(row.detail.process_count)}
        </button>
      {/if}
    </article>
  {:else}
    <div class="mobile-empty-state">No process matches this view.</div>
  {/each}
</div>
