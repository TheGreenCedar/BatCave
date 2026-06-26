<script lang="ts">
  import MetricCard from "./MetricCard.svelte";
  import type { DetailMode, MetricCardOption } from "./types";

  export let cards: MetricCardOption[] = [];
  export let activeMode: DetailMode;
  export let needsReadoutContrast: (value: number, max: number) => boolean;
  export let onSelect: (mode: DetailMode) => void;
</script>

<section class="metric-strip" aria-label="System metrics">
  {#each cards as card (card.ariaLabel)}
    <MetricCard {card} {activeMode} {needsReadoutContrast} {onSelect} />
  {/each}
</section>
