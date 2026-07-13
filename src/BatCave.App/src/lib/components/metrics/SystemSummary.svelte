<script lang="ts">
  import MiniChart from "../../MiniChart.svelte";
  import type { ResourceBrief } from "../../cockpit";
  import type { ProcessIconKind } from "../../process";
  import ProcessIcon from "../processes/ProcessIcon.svelte";
  import type { DetailMode, ResourceSummaryOption } from "./types";

  export let brief: ResourceBrief;
  export let resources: ResourceSummaryOption[] = [];
  export let activeMode: DetailMode;
  export let supportingText: string;
  export let sampledAtLabel: string;
  export let windowLabel = "No history";
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
    <span>Selected resource</span>
    <p><i class:inactive={brief.stateLabel !== "Current"} aria-hidden="true"></i>{brief.stateLabel} <b>·</b> {sampledAtLabel} <b>·</b> Confidence: {brief.confidence}</p>
  </header>
  <h2 id="pressure-brief-heading">{brief.headline}</h2>
  <div class="pressure-brief-body">
    <div class="pressure-readout">
      <strong>{brief.valueLabel}</strong>
      <span>{brief.semanticLabel}</span>
    </div>
    <div class="pressure-chart" aria-label={`${brief.semanticLabel} trend, ${windowLabel}`}>
      <MiniChart values={activeValues} max={activeMax} stroke={activeStroke} fill={activeFill} />
      <small>{windowLabel}</small>
    </div>
    <div class="leading-workload">
      <ProcessIcon kind={leadingIconKind} src={leadingIconSrc} />
      <span>
        <small>Compatible process attribution</small>
        <strong>{brief.leadingWorkload ?? "Not available"}</strong>
        <b>{brief.leadingWorkload ? brief.leadingValueLabel : brief.attributionLabel}</b>
      </span>
    </div>
    <div class="pressure-resource-strip" aria-label="System resource summary">
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
  <p class="pressure-supporting">{supportingText} {brief.attributionLabel}</p>
</section>
