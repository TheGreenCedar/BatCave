<script lang="ts">
  import { ArrowClockwise } from "phosphor-svelte";
  import type { FocusMode, SortKey } from "../../process";
  import SegmentedControl from "../ui/SegmentedControl.svelte";
  import SortSelect from "../ui/SortSelect.svelte";

  export let focusMode: FocusMode;
  export let sortKey: SortKey;
  export let commandError: string;
  export let rankingUpdateAvailable: boolean;
  export let focusOptions: { value: FocusMode; label: string }[];
  export let sortOptions: { value: SortKey; label: string }[];
  export let onFocus: (mode: FocusMode) => void;
  export let onSort: (key: SortKey) => void;
  export let onApplyRanking: () => void;
</script>

<section class="process-command-bar" aria-label="Workload controls">
  <SegmentedControl label="Workload view" options={focusOptions} value={focusMode} onChange={onFocus} />

  <label class="sort-control">
    <span>Sort by</span>
    <SortSelect options={sortOptions} value={sortKey} onChange={onSort} />
  </label>

  {#if rankingUpdateAvailable}
    <button
      class="ranking-update"
      type="button"
      aria-label="Update workload order"
      title="Update workload order"
      onclick={onApplyRanking}
    >
      <ArrowClockwise size={16} weight="bold" aria-hidden="true" />
      <span>Update order</span>
    </button>
  {/if}

  {#if commandError}
    <p class="command-error command-bar-error" role="alert">{commandError}</p>
  {/if}
</section>
