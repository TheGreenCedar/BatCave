<script lang="ts">
  import Cpu from "phosphor-svelte/lib/Cpu";
  import HardDrive from "phosphor-svelte/lib/HardDrive";
  import Memory from "phosphor-svelte/lib/Memory";
  import WifiHigh from "phosphor-svelte/lib/WifiHigh";
  import MiniChart from "../../MiniChart.svelte";
  import { displayProcessName } from "../../cockpit";
  import { OverviewRanking, overviewMetricValue, type OverviewStatus } from "../../overview";
  import {
    resolvedProcessIcon,
    type ResolvedProcessIcon,
    type ResolvedProcessIconCatalog,
  } from "../../processIcons";
  import {
    processRowSecondaryLabel,
    processViewRowKey,
    type ProcessIconKind,
  } from "../../process";
  import type { ProcessViewRow } from "../../types";
  import type { DetailMode, ResourceSummaryOption } from "../metrics/types";
  import ProcessIcon from "../processes/ProcessIcon.svelte";

  export let status: OverviewStatus = {
    headline: "Waiting for measurements.",
    summary: "Waiting for the first local system sample.",
    tone: "neutral",
    attention: {
      title: "Starting up",
      detail: "Waiting for the first local system sample.",
      tone: "healthy",
    },
    primaryResource: "cpu",
  };
  export let resources: ResourceSummaryOption[] = [];
  export let leadingRows: ProcessViewRow[] = [];
  export let processIcons: ResolvedProcessIconCatalog = {};
  export let leadingName: string | null = null;
  export let leadingValue: string | null = null;
  export let leadingNarrativeGenerated = false;
  export let leadingIconKind: ProcessIconKind = "process";
  export let leadingIconSrc: string | undefined = undefined;
  export let leadingIconMatched = false;
  export let leadingIconSystemTool = false;
  export let leadingSelection: string | null = null;
  export let onInspectResource: () => void;
  export let onOpenDiagnostics: () => void;
  export let onSelectResource: (mode: DetailMode) => void;
  export let onSelectWorkload: (selection: string) => void;
  export let onOpenExplore: () => void;

  const ranking = new OverviewRanking();
  let displayRows: ProcessViewRow[] = [];
  let orderUpdateAvailable = false;
  $: {
    displayRows = ranking.update(status.primaryResource, leadingRows);
    orderUpdateAvailable = ranking.updateAvailable;
  }

  function setInteraction(source: "pointer" | "focus", active: boolean): void {
    displayRows = ranking.setInteraction(source, active);
    orderUpdateAvailable = ranking.updateAvailable;
  }

  function applyOrderUpdate(): void {
    displayRows = ranking.applyUpdate();
    orderUpdateAvailable = ranking.updateAvailable;
  }

  function handleFocusOut(event: FocusEvent & { currentTarget: HTMLDivElement }): void {
    if (!(event.relatedTarget instanceof Node) || !event.currentTarget.contains(event.relatedTarget)) {
      setInteraction("focus", false);
    }
  }

  function leadingProcessLabel(mode: DetailMode): string {
    if (mode === "disk") return "Process attribution";
    const names: Record<DetailMode, string> = {
      cpu: "CPU",
      memory: "memory",
      disk: "disk",
      network: "network",
    };
    return `Top ${names[mode]} process`;
  }

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
    return displayProcessName(row.kind === "group" ? row.detail.label : row.detail.process.name);
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
      <h2 id="overview-heading">{status.headline}</h2>
      <p>{status.summary}</p>
      <button class="overview-inspect-resource" type="button" onclick={onInspectResource}>Inspect resource</button>
    </div>

    <div class="overview-contributor">
      <span class="overview-contributor-label">{leadingProcessLabel(status.primaryResource)}</span>
      {#if leadingName}
        <button
          type="button"
          onclick={() =>
            leadingSelection ? onSelectWorkload(leadingSelection) : onOpenExplore()}
        >
          <ProcessIcon
            kind={leadingIconKind}
            src={leadingIconSrc}
            matched={leadingIconMatched}
            systemTool={leadingIconSystemTool}
          />
          <span>
            <strong title={leadingName}>{leadingName}</strong>
            <small>{leadingValue ?? "Current contribution available in Explore"}</small>
            {#if leadingNarrativeGenerated}
              <small class="narrative-origin">Locally generated explanation</small>
            {/if}
          </span>
        </button>
      {:else}
        <p>{leadingValue ?? "No compatible process attribution is available for this sample."}</p>
      {/if}
    </div>
  </section>

  <section class="overview-resources" aria-label="System resources">
    {#each resources as resource (resource.mode)}
      {@const Icon = resourceIcon(resource.mode)}
      <button
        class="overview-resource-card"
        class:active={status.primaryResource === resource.mode}
        type="button"
        data-resource-mode={resource.mode}
        aria-pressed={status.primaryResource === resource.mode}
        aria-label={`Select ${resource.label}. ${resource.value}. ${resource.statusLabel}`}
        onclick={() => onSelectResource(resource.mode)}
      >
        <span class={`resource-icon resource-${resource.mode}`}><Icon size={24} weight="regular" aria-hidden="true" /></span>
        <span class="resource-card-copy">
          <span>{resource.label}</span>
          <small class="resource-card-supporting">
            {#each resource.supportingMetrics as metric (metric.label)}
              <span class="resource-card-reading">
                <span>{metric.label}</span>
                <span class="resource-card-reading-value">{metric.value}</span>
              </span>
            {/each}
          </small>
        </span>
        <span class="resource-card-chart"><MiniChart values={resource.values} max={resource.max} stroke={resource.stroke} fill={resource.fill} /></span>
        <span class="resource-card-value">
          <strong>{resource.value}</strong>
          {#if resourceQualityVisible(resource)}<small>{resource.shortStatusLabel}</small>{/if}
        </span>
      </button>
    {/each}
  </section>

  {#if status.attention.tone !== "healthy"}
    <section class={`overview-attention tone-${status.attention.tone}`} aria-labelledby="overview-attention-heading">
      <div>
        <h3 id="overview-attention-heading">{status.attention.title}</h3>
        <p>{status.attention.detail}</p>
      </div>
      <button type="button" onclick={onOpenDiagnostics}>View diagnostics</button>
    </section>
  {/if}

  <section class="overview-workloads" aria-labelledby="leading-workloads-heading">
    <header>
      <div>
        <h3 id="leading-workloads-heading">Leading workloads</h3>
        <p>Sorted by {status.primaryResource === "disk" ? "process read/write I/O" : status.primaryResource === "cpu" ? "CPU use per core" : status.primaryResource === "memory" ? "resident memory" : "process network traffic"} across the sample.</p>
      </div>
      {#if orderUpdateAvailable}
        <span class="overview-order-held">
          Order paused
          <button type="button" onclick={applyOrderUpdate}>Update</button>
        </span>
      {/if}
      <button type="button" onclick={onOpenExplore}>View all in Explore</button>
    </header>
    <div class="overview-workload-list" role="group" aria-label="Leading workloads"
      onpointerenter={() => setInteraction("pointer", true)}
      onpointerleave={() => setInteraction("pointer", false)}
      onfocusin={() => setInteraction("focus", true)}
      onfocusout={handleFocusOut}>
      {#each displayRows as row (processViewRowKey(row))}
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
            systemTool={resolvedIcon.systemTool ?? false}
          />
          <span class="overview-workload-name">
            <strong title={row.kind === "process" ? row.detail.process.exe || row.detail.process.name : row.detail.label}>{rowLabel(row)}</strong>
            {#if secondaryLabel(row)}<small>{secondaryLabel(row)}</small>{/if}
          </span>
          <span><small title="Percent of one CPU core — 100% is one fully busy core">CPU / core</small><strong>{overviewMetricValue(row, "cpu")}</strong></span>
          <span><small>Memory</small><strong>{overviewMetricValue(row, "memory")}</strong></span>
          <span class="overview-workload-io"><small>I/O</small><strong>{overviewMetricValue(row, "disk")}</strong></span>
          <span class="overview-workload-network"><small>Network</small><strong>{overviewMetricValue(row, "network")}</strong></span>
          <span class="overview-workload-link" aria-hidden="true">Inspect</span>
        </button>
      {:else}
        <p class="overview-empty">No workloads have an available {status.primaryResource === "disk" ? "read/write I/O" : status.primaryResource} measurement in this sample.</p>
      {/each}
    </div>
  </section>
</main>
