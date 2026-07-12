<script lang="ts">
  import MiniChart from "../../MiniChart.svelte";
  import type { PressureBrief } from "../../cockpit";
  import type { ProcessIconKind } from "../../process";
  import { formatPercent, formatRate } from "../../format";
  import ProcessIcon from "../processes/ProcessIcon.svelte";
  import type { DetailMode, ResourceSummaryOption } from "./types";

  export let brief: PressureBrief;
  export let resources: ResourceSummaryOption[] = [];
  export let activeMode: DetailMode;
  export let supportingText: string;
  export let sampledAtLabel: string;
  export let activeValues: number[] = [];
  export let activeMax = 100;
  export let activeStroke = "#4a9cff";
  export let activeFill = "rgba(74, 156, 255, 0.14)";
  export let leadingIconKind: ProcessIconKind = "process";
  export let leadingIconSrc: string | undefined = undefined;
  export let onSelect: (mode: DetailMode) => void;

  function resourceValue(resource: ResourceSummaryOption): string {
    return resource.value;
  }
</script>

<section class="pressure-brief" aria-labelledby="pressure-brief-heading">
  <header class="pressure-brief-heading">
    <span>Top pressure</span>
    <p><i aria-hidden="true"></i>Data fresh <b>·</b> {sampledAtLabel} <b>·</b> Confidence: {brief.confidence}</p>
  </header>
  <h2 id="pressure-brief-heading" aria-label={brief.headline}>
    {brief.headlinePrefix}{#if brief.leadingWorkload}{" — "}<span class="headline-workload">{brief.leadingWorkload}</span> is the leading workload.{:else}.{/if}
  </h2>
  <div class="pressure-brief-body">
    <div class="pressure-readout">
      <strong>{brief.mode === "cpu" || brief.mode === "memory" ? formatPercent(brief.value) : formatRate(brief.value)}</strong>
      <span>{brief.label} pressure <b class={`tone-${brief.tone}`}>{brief.tone}</b></span>
    </div>
    <div class="pressure-chart" aria-label={`${brief.label} trend`}>
      <MiniChart values={activeValues} max={activeMax} stroke={activeStroke} fill={activeFill} />
      <small>60s</small>
    </div>
    <div class="leading-workload">
      <ProcessIcon kind={leadingIconKind} src={leadingIconSrc} />
      <span>
        <small>Leading workload</small>
        <strong>{brief.leadingWorkload ?? "No single workload"}</strong>
        <b>{brief.leadingValue === null ? "Current activity is distributed" : `${formatPercent(brief.leadingValue)} of ${brief.label}`}</b>
      </span>
    </div>
    <div class="pressure-resource-strip" aria-label="System pressure summary">
      {#each resources as resource (resource.mode)}
        <button
          class:active={resource.mode === activeMode}
          type="button"
          aria-label={`${resource.ariaLabel}. ${resource.statusLabel}`}
          aria-pressed={resource.mode === activeMode}
          title={resource.statusLabel}
          onclick={() => onSelect(resource.mode)}
        >
          <span>{resource.label}</span>
          <strong>{resourceValue(resource)}</strong>
          <small aria-hidden="true">{resource.shortStatusLabel}</small>
        </button>
      {/each}
    </div>
  </div>
  <p class="pressure-supporting">{supportingText}</p>
</section>
