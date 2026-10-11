<script lang="ts">
  import MiniChart from "../../MiniChart.svelte";
  import type { DetailMode, ResourceSummaryOption } from "./types";

  export let resources: ResourceSummaryOption[] = [];
  export let activeMode: DetailMode;
  export let detailOpen = false;
  export let onSelect: (mode: DetailMode) => void;
</script>

<aside class="resource-strip" aria-label="System resources">
  <h3>System resources</h3>
  <div class="resource-strip-list">
    {#each resources as resource (resource.mode)}
      <button
        class={`resource-summary resource-strip-card resource-${resource.mode}`}
        class:active={resource.mode === activeMode}
        type="button"
        style={`--resource-accent: ${resource.stroke}; --resource-fill: ${resource.fill}`}
        aria-label={`Inspect ${resource.label}. ${resource.value}. ${resource.statusLabel}`}
        aria-pressed={resource.mode === activeMode}
        aria-controls={detailOpen ? "detail-pane" : undefined}
        aria-haspopup="dialog"
        data-resource-mode={resource.mode}
        onclick={() => onSelect(resource.mode)}
      >
        <span class="resource-heading">
          <span><i aria-hidden="true"></i>{resource.label}</span>
          <small title={resource.statusLabel} aria-hidden="true">{resource.shortStatusLabel}</small>
        </span>
        <strong class="resource-strip-value">{resource.value}</strong>
        <span class="resource-strip-chart" aria-hidden="true">
          <MiniChart values={resource.values} max={resource.max} stroke={resource.stroke} fill={resource.fill} />
        </span>
        <span class="resource-strip-supporting">
          {#each resource.supportingMetrics as metric (metric.label)}
            <span class="resource-strip-reading">
              <span>{metric.label}</span>
              <b>{metric.value}</b>
            </span>
          {/each}
        </span>
      </button>
    {/each}
  </div>
</aside>
