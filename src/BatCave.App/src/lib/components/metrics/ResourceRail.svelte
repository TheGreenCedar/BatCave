<script lang="ts">
  import { Pulse, CaretUp } from "phosphor-svelte";
  import MiniChart from "../../MiniChart.svelte";
  import type { DetailMode, ResourceSummaryOption } from "./types";

  export let resources: ResourceSummaryOption[] = [];
  export let activeMode: DetailMode;
  export let environmentLabel = "Local environment";
  export let sourceLabel = "Local collectors";
  export let diagnosticsLabel = "All green";
  export let onSelect: (mode: DetailMode) => void;
  export let onOpenDiagnostics: () => void;
</script>

<aside class="resource-rail" aria-label="System resources">
  <header class="resource-rail-heading">
    <span>System resources</span>
    <CaretUp size={15} weight="bold" aria-hidden="true" />
  </header>
  <div class="resource-rail-list">
    {#each resources as resource (resource.mode)}
      <button
        class={`resource-summary resource-${resource.mode}`}
        class:active={resource.mode === activeMode}
        type="button"
        style={`--resource-accent: ${resource.stroke}; --resource-fill: ${resource.fill}`}
        aria-label={`${resource.ariaLabel}. ${resource.statusLabel}`}
        aria-pressed={resource.mode === activeMode}
        aria-controls="detail-pane"
        data-resource-mode={resource.mode}
        onclick={() => onSelect(resource.mode)}
      >
        <span class="resource-heading">
          <span><i aria-hidden="true"></i>{resource.label}</span>
          <small title={resource.statusLabel} aria-hidden="true">{resource.shortStatusLabel}</small>
        </span>
        <span class="resource-rail-readout">
          <strong>{resource.value}</strong>
          <span class="resource-spark" aria-hidden="true">
            <MiniChart values={resource.values} max={resource.max} stroke={resource.stroke} fill={resource.fill} />
          </span>
        </span>
        <span class="resource-supporting">
          <span>{resource.supportingLabel}</span>
          <b>{resource.supportingValue}</b>
        </span>
      </button>
    {/each}
  </div>
  <div class="resource-environment">
    <span>Environment</span>
    <strong>{environmentLabel}</strong>
    <span>Collectors</span>
    <strong>{sourceLabel}</strong>
  </div>
  <button class="rail-diagnostics" type="button" onclick={onOpenDiagnostics}>
    <Pulse size={20} weight="regular" aria-hidden="true" />
    <span>Diagnostics</span>
    <small>{diagnosticsLabel}</small>
  </button>
</aside>
