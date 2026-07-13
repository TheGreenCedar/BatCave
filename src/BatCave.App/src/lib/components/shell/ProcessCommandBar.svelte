<script lang="ts">
  import { ArrowClockwise, ArrowDown, ArrowUp } from "phosphor-svelte";
  import { sortDirectionButtonLabel, type FocusMode, type SortKey } from "../../process";
  import type { SortDirection } from "../../types";
  import SegmentedControl from "../ui/SegmentedControl.svelte";
  import SortSelect from "../ui/SortSelect.svelte";

  export let focusMode: FocusMode;
  export let sortKey: SortKey;
  export let sortDirection: SortDirection;
  export let commandError: string;
  export let rankingUpdateAvailable: boolean;
  export let focusOptions: { value: FocusMode; label: string }[];
  export let sortOptions: { value: SortKey; label: string }[];
  export let onFocus: (mode: FocusMode) => void;
  export let onSort: (key: SortKey) => void;
  export let onToggleDirection: () => void;
  export let onApplyRanking: () => void;
</script>

<section class="process-command-bar" aria-label="Workload controls">
  <SegmentedControl label="Workload view" options={focusOptions} value={focusMode} onChange={onFocus} />

  <div class="sort-control" role="group" aria-label="Workload sort">
    <label>
      <span>Sort by</span>
      <SortSelect options={sortOptions} value={sortKey} onChange={onSort} />
    </label>
    <button
      class="sort-direction-toggle"
      type="button"
      aria-label={sortDirectionButtonLabel(sortDirection)}
      title={sortDirectionButtonLabel(sortDirection)}
      onclick={onToggleDirection}
    >
      {#if sortDirection === "asc"}
        <ArrowUp size={16} weight="bold" aria-hidden="true" />
        <span>Asc</span>
      {:else}
        <ArrowDown size={16} weight="bold" aria-hidden="true" />
        <span>Desc</span>
      {/if}
    </button>
  </div>

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
