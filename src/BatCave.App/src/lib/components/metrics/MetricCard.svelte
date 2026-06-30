<script lang="ts">
  import MiniChart from "../../MiniChart.svelte";
  import type { DetailMode, MetricCardOption } from "./types";

  export let card = {} as MetricCardOption;
  export let activeMode: DetailMode;
  export let needsReadoutContrast: (value: number, max: number) => boolean;
  export let onSelect: (mode: DetailMode) => void;

  $: gaugePercent = Math.max(0, Math.min(100, card.max > 0 ? (card.contrastValue / card.max) * 100 : 0));
</script>

<button
  class={`metric-card metric-card-${card.mode}`}
  class:active={activeMode === card.mode}
  type="button"
  style={`--metric-accent: ${card.stroke}; --metric-fill: ${card.fill}`}
  aria-controls="context-rail"
  aria-label={card.ariaLabel}
  aria-pressed={activeMode === card.mode}
  onclick={() => onSelect(card.mode)}
>
  <span class="metric-gauge" aria-hidden="true">
    <svg viewBox="0 0 200 120" role="img">
      <path class="gauge-track" pathLength="100" d="M24 100 A76 76 0 0 1 176 100" />
      <path
        class="gauge-value"
        pathLength="100"
        d="M24 100 A76 76 0 0 1 176 100"
        style={`stroke-dasharray: ${gaugePercent} 100`}
      />
    </svg>
  </span>
  <span
    class="metric-readout"
    class:compact={card.value.length > 5}
    class:on-fill={needsReadoutContrast(card.contrastValue, card.max)}
  >
    <span>{card.label}</span>
    <strong>{card.value}</strong>
    <small>{card.sublabel}</small>
  </span>
  <span class="metric-spark">
    <MiniChart values={card.values} max={card.max} stroke={card.stroke} fill={card.fill} />
  </span>
</button>
