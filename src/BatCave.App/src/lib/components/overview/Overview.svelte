<script lang="ts">
  import Cpu from "phosphor-svelte/lib/Cpu";
  import HardDrive from "phosphor-svelte/lib/HardDrive";
  import Memory from "phosphor-svelte/lib/Memory";
  import WifiHigh from "phosphor-svelte/lib/WifiHigh";
  import MiniChart from "../../MiniChart.svelte";
  import { formatBytes, formatPercent, formatRate } from "../../format";
  import type { OverviewStatus } from "../../overview";
  import {
    resolvedProcessIcon,
    type ResolvedProcessIcon,
    type ResolvedProcessIconCatalog,
  } from "../../processIcons";
  import {
    processRowSecondaryLabel,
    processViewRowKey,
    processViewRowMetrics,
    type ProcessIconKind,
  } from "../../process";
  import type { ProcessViewRow } from "../../types";
  import type { DetailMode, ResourceSummaryOption } from "../metrics/types";
  import ProcessIcon from "../processes/ProcessIcon.svelte";

  export let status: OverviewStatus;
  export let resources: ResourceSummaryOption[] = [];
  export let leadingRows: ProcessViewRow[] = [];
  export let processIcons: ResolvedProcessIconCatalog = {};
  export let primaryCpuValue = 0;
  export let primaryCpuHistory: number[] = [];
  export let primaryCpuStroke = "#2aa88f";
  export let primaryCpuFill = "rgba(42, 168, 143, 0.12)";
  export let leadingCpuName: string | null = null;
  export let leadingCpuValue: string | null = null;
  export let leadingCpuIconKind: ProcessIconKind = "process";
  export let leadingCpuIconSrc: string | undefined = undefined;
  export let leadingCpuIconMatched = false;
  export let leadingCpuSelection: string | null = null;
  export let onSelectResource: (mode: DetailMode) => void;
  export let onSelectWorkload: (selection: string) => void;
  export let onOpenExplore: () => void;

  function resourceIcon(mode: DetailMode) {
    if (mode === "cpu") return Cpu;
    if (mode === "memory") return Memory;
    if (mode === "disk") return HardDrive;
    return WifiHigh;
  }

  function resourceQualityVisible(resource: ResourceSummaryOption): boolean {
    return !["Measured", "Aggregate", "Native", "Current"].includes(resource.shortStatusLabel);
  }

  function rowIcon(row: ProcessViewRow): ResolvedProcessIcon {
    if (row.kind === "group") {
      return resolvedProcessIcon(processIcons, row.icon_source);
    }
    const process = row.detail.process;
    return resolvedProcessIcon(processIcons, process.exe || process.name);
  }

  function iconKind(row: ProcessViewRow): ProcessIconKind {
    return (row.icon_kind as ProcessIconKind) || "process";
  }

  function rowLabel(row: ProcessViewRow): string {
    return row.kind === "group" ? row.detail.label : row.detail.process.name;
  }

  function memoryLabel(row: ProcessViewRow): string {
    return formatBytes(processViewRowMetrics(row).memoryBytes);
  }

  function ioLabel(row: ProcessViewRow): string {
    const ioBps = processViewRowMetrics(row).ioBps;
    return Number.isFinite(ioBps) ? formatRate(ioBps) : "Unavailable";
  }

  function networkLabel(row: ProcessViewRow): string {
    const networkBps = processViewRowMetrics(row).networkBps;
    return Number.isFinite(networkBps) ? formatRate(networkBps) : "Unavailable";
  }

  function secondaryLabel(row: ProcessViewRow): string | null {
    if (row.kind === "group") {
      return `${row.detail.process_count} ${row.detail.process_count === 1 ? "process" : "processes"}`;
    }
    return processRowSecondaryLabel(row);
  }
</script>

<main class="overview-view" aria-labelledby="overview-heading">
  <section class="overview-hero">
    <div class="overview-status-copy">
      <span class={`overview-state tone-${status.tone}`}>System overview</span>
      <h2 id="overview-heading">{status.headline}</h2>
      <p>{status.summary}</p>
    </div>

    <div class="overview-primary-metric" aria-label={`Total CPU ${formatPercent(primaryCpuValue)}`}>
      <div class="overview-primary-heading">
        <span>Total CPU</span>
        <strong>{formatPercent(primaryCpuValue)}</strong>
      </div>
      <MiniChart
        values={primaryCpuHistory}
        max={100}
        stroke={primaryCpuStroke}
        fill={primaryCpuFill}
      />
      <span class={`overview-metric-state tone-${status.tone}`}>
        {status.pressuredResource === "cpu" ? "Needs attention" : status.tone === "healthy" ? "Normal" : "Current"}
      </span>
    </div>

    <div class="overview-contributor">
      <span>Top CPU contributor</span>
      {#if leadingCpuName}
        <button
          type="button"
          onclick={() =>
            leadingCpuSelection ? onSelectWorkload(leadingCpuSelection) : onOpenExplore()}
        >
          <ProcessIcon
            kind={leadingCpuIconKind}
            src={leadingCpuIconSrc}
            matched={leadingCpuIconMatched}
          />
          <span>
            <strong>{leadingCpuName}</strong>
            <small>{leadingCpuValue ?? "Current contribution available in Explore"}</small>
          </span>
        </button>
      {:else}
        <p>No compatible process attribution is available for this sample.</p>
      {/if}
    </div>
  </section>

  <section class="overview-resources" aria-label="System resources">
    {#each resources as resource (resource.mode)}
      {@const Icon = resourceIcon(resource.mode)}
      <button
        class="overview-resource-card"
        class:pressured={status.pressuredResource === resource.mode}
        type="button"
        data-resource-mode={resource.mode}
        aria-label={`${resource.ariaLabel}. ${resource.value}. ${resource.statusLabel}`}
        onclick={() => onSelectResource(resource.mode)}
      >
        <span class={`resource-icon resource-${resource.mode}`}><Icon size={24} weight="regular" aria-hidden="true" /></span>
        <span class="resource-card-copy">
          <span>{resource.label}</span>
          <small>{resource.supportingLabel}</small>
        </span>
        <span class="resource-card-chart"><MiniChart values={resource.values} max={resource.max} stroke={resource.stroke} fill={resource.fill} /></span>
        <span class="resource-card-value">
          <strong>{resource.value}</strong>
          {#if resourceQualityVisible(resource)}<small>{resource.shortStatusLabel}</small>{/if}
        </span>
      </button>
    {/each}
  </section>

  {#if status.attention}
    <section class={`overview-attention tone-${status.attention.tone}`} aria-labelledby="overview-attention-heading">
      <div>
        <span>Attention</span>
        <h3 id="overview-attention-heading">{status.attention.title}</h3>
        <p>{status.attention.detail}</p>
      </div>
      <button type="button" onclick={onOpenExplore}>Review in Explore</button>
    </section>
  {/if}

  <section class="overview-workloads" aria-labelledby="leading-workloads-heading">
    <header>
      <div>
        <h3 id="leading-workloads-heading">Worth a look</h3>
        <p>These workloads are leading the current ranking.</p>
      </div>
      <button type="button" onclick={onOpenExplore}>View all in Explore</button>
    </header>
    <div class="overview-workload-list">
      {#each leadingRows as row (processViewRowKey(row))}
        {@const metrics = processViewRowMetrics(row)}
        {@const resolvedIcon = rowIcon(row)}
        <button
          type="button"
          data-workload-id={processViewRowKey(row)}
          aria-label={`Open ${rowLabel(row)} in Explore`}
          onclick={() => onSelectWorkload(processViewRowKey(row))}
        >
          <ProcessIcon
            kind={iconKind(row)}
            src={resolvedIcon.src}
            matched={resolvedIcon.origin === "name_match"}
          />
          <span class="overview-workload-name">
            <strong>{rowLabel(row)}</strong>
            {#if secondaryLabel(row)}<small>{secondaryLabel(row)}</small>{/if}
          </span>
          <span><small>CPU</small><strong>{formatPercent(metrics.cpuPercent)}</strong></span>
          <span><small>Memory</small><strong>{memoryLabel(row)}</strong></span>
          <span class="overview-workload-io"><small>I/O</small><strong>{ioLabel(row)}</strong></span>
          <span class="overview-workload-network"><small>Network</small><strong>{networkLabel(row)}</strong></span>
          <span class="overview-workload-link">View in Explore</span>
        </button>
      {:else}
        <p class="overview-empty">No workloads are available in this sample.</p>
      {/each}
    </div>
  </section>
</main>
