<script lang="ts">
  import MiniChart from "../../MiniChart.svelte";
  import type { DetailMode, ResourceSummaryOption } from "./types";

  export let resources: ResourceSummaryOption[] = [];
  export let activeMode: DetailMode;
  export let headline: string;
  export let supportingText: string;
  export let onSelect: (mode: DetailMode) => void;
</script>

<section class="system-summary" aria-labelledby="system-summary-heading">
  <div class="system-summary-intro">
    <span>System pressure</span>
    <h2 id="system-summary-heading">{headline}</h2>
    <p>{supportingText}</p>
  </div>

  <div class="resource-summary-grid">
    {#each resources as resource (resource.mode)}
      <button
        class={`resource-summary resource-${resource.mode}`}
        class:active={resource.mode === activeMode}
        type="button"
        style={`--resource-accent: ${resource.stroke}; --resource-fill: ${resource.fill}`}
        aria-label={resource.ariaLabel}
        aria-pressed={resource.mode === activeMode}
        aria-controls="detail-pane"
        onclick={() => onSelect(resource.mode)}
      >
        <span class="resource-heading">
          <span>{resource.label}</span>
          <small>{resource.statusLabel}</small>
        </span>
        <strong>{resource.value}</strong>
        <span class="resource-supporting">
          <span>{resource.supportingLabel}</span>
          <b>{resource.supportingValue}</b>
        </span>
        <span class="resource-spark" aria-hidden="true">
          <MiniChart
            values={resource.values}
            max={resource.max}
            stroke={resource.stroke}
            fill={resource.fill}
          />
        </span>
      </button>
    {/each}
  </div>
</section>
