<script lang="ts">
  import { CaretRight } from "phosphor-svelte";
  import {
    processRowSecondaryLabel,
    processViewRowKey,
    processViewRowMetrics,
    type ProcessIconKind,
  } from "../../process";
  import {
    displayProcessMetricValue,
    formatBytes,
    formatPercent,
    formatRate,
    processMemoryTitle,
  } from "../../format";
  import { residentMemoryValue } from "../../platformPresentation";
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

  function selectedInRow(row: ProcessViewRow): boolean {
    if (row.detail.workload_id === selectedWorkloadId) return true;
    if (row.kind === "process") return false;
    return processRows.some(
      (candidate) =>
        candidate.kind === "process" &&
        candidate.group_key === row.detail.group_key &&
        candidate.detail.workload_id === selectedWorkloadId,
    );
  }

  function selectRow(row: ProcessViewRow): void {
    onSelect(row.detail.workload_id);
  }

  function networkLabel(row: ProcessViewRow): string {
    const quality =
      row.kind === "group" ? row.detail.quality.network : row.detail.process.quality?.network;
    if (row.kind === "process") {
      return displayProcessMetricValue(row.detail.network_bps, quality, formatRate);
    }
    if (quality?.quality === "unavailable") return "Unavailable";
    if (quality?.quality === "held") return "Waiting";
    return formatRate(processViewRowMetrics(row).networkBps);
  }

  function handleFocusOut(event: FocusEvent & { currentTarget: HTMLDivElement }): void {
    const next = event.relatedTarget;
    if (!(next instanceof Node) || !event.currentTarget.contains(next)) {
      onInteractionChange(false);
    }
  }

  function cpuLabel(row: ProcessViewRow): string {
    const metrics = processViewRowMetrics(row);
    if (row.kind === "group") return formatPercent(metrics.cpuPercent);
    return displayProcessMetricValue(
      metrics.cpuPercent,
      row.detail.process.quality?.cpu,
      formatPercent,
    );
  }

  function ioLabel(row: ProcessViewRow): string {
    const metrics = processViewRowMetrics(row);
    if (row.kind === "group") return formatRate(metrics.ioBps);
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
    {@const selected = selectedInRow(row)}
    {@const expanded = row.kind === "group" ? !!expandedGroups[row.detail.group_key] : false}
    {@const secondaryLabel = processRowSecondaryLabel(row)}
    <article
      class="mobile-process-card"
      class:selected={selected}
      class:child-card={row.kind === "process" && row.is_grouped}
    >
      <button
        class="mobile-card-select"
        type="button"
        aria-pressed={selected}
        aria-label={row.kind === "group" ? `Inspect ${row.detail.label} group` : `Inspect ${process?.name}, PID ${process?.pid}`}
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
            <em>Working set</em>
            <b title={process ? processMemoryTitle(process) : ""}>{process ? residentMemoryValue(process, platform) : formatBytes(metrics.memoryBytes)}</b>
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
