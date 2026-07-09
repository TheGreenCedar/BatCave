<script lang="ts">
  import type { FocusMode, SortKey } from "../../process";
  import SegmentedControl from "../ui/SegmentedControl.svelte";
  import SortSelect from "../ui/SortSelect.svelte";

  export let searchText: string;
  export let focusMode: FocusMode;
  export let sortKey: SortKey;
  export let isPaused: boolean;
  export let commandError: string;
  export let rankingUpdateAvailable: boolean;
  export let focusOptions: { value: FocusMode; label: string }[];
  export let sortOptions: { value: SortKey; label: string }[];
  export let onSearch: (value: string) => void;
  export let onFocus: (mode: FocusMode) => void;
  export let onSort: (key: SortKey) => void;
  export let onPaused: () => void;
  export let onRefresh: () => void;
  export let onOpenSettings: () => void;
  export let onApplyRanking: () => void;
</script>

<section class="process-command-bar" aria-label="Process controls">
  <label class="command-search" for="process-search">
    <svg class="control-icon" viewBox="0 0 24 24" aria-hidden="true">
      <circle cx="11" cy="11" r="6" />
      <path d="m16 16 4 4" />
    </svg>
    <input
      id="process-search"
      value={searchText}
      oninput={(event) => onSearch(event.currentTarget.value)}
      aria-label="Search apps and processes"
      placeholder="Search apps and processes"
      autocomplete="off"
    />
    <kbd>/</kbd>
  </label>

  <SegmentedControl label="Process view" options={focusOptions} value={focusMode} onChange={onFocus} />

  <label class="sort-control">
    <span>Sort</span>
    <SortSelect options={sortOptions} value={sortKey} onChange={onSort} />
  </label>

  {#if rankingUpdateAvailable}
    <button class="ranking-update" type="button" onclick={onApplyRanking}>
      Ranking updated
    </button>
  {/if}

  <div class="command-actions">
    <button class="pause-action" class:resume={isPaused} type="button" onclick={onPaused}>
      {isPaused ? "Resume" : "Pause"}
    </button>
    <button class="icon-action" type="button" aria-label="Refresh now" title="Refresh now" onclick={onRefresh}>
      <svg class="control-icon" viewBox="0 0 24 24" aria-hidden="true">
        <path d="M20 12a8 8 0 0 1-14.2 5" />
        <path d="M4 12A8 8 0 0 1 18.2 7" />
        <path d="M18 3v4h-4" />
        <path d="M6 21v-4h4" />
      </svg>
    </button>
    <button class="icon-action" type="button" aria-label="Open settings" title="Settings" onclick={onOpenSettings}>
      <svg class="control-icon filled" viewBox="0 0 24 24" aria-hidden="true">
        <path d="M10.8 3h2.4l.5 2.3c.6.2 1.1.4 1.6.7l2-1.2 1.7 1.7-1.2 2c.3.5.5 1 .7 1.6l2.3.5v2.4l-2.3.5c-.2.6-.4 1.1-.7 1.6l1.2 2-1.7 1.7-2-1.2c-.5.3-1 .5-1.6.7l-.5 2.3h-2.4l-.5-2.3c-.6-.2-1.1-.4-1.6-.7l-2 1.2-1.7-1.7 1.2-2c-.3-.5-.5-1-.7-1.6L3.2 13v-2.4l2.3-.5c.2-.6.4-1.1.7-1.6L5 6.5l1.7-1.7 2 1.2c.5-.3 1-.5 1.6-.7L10.8 3Zm1.2 6a3 3 0 1 0 0 6 3 3 0 0 0 0-6Z" />
      </svg>
    </button>
  </div>

  {#if commandError}
    <p class="command-error command-bar-error" role="alert">{commandError}</p>
  {/if}
</section>
