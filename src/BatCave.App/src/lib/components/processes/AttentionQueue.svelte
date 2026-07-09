<script lang="ts">
  import type { ProcessColumn, SortKey } from "../../process";
  import type { ProcessFocusMode, ProcessViewRow, SortDirection } from "../../types";
  import MobileProcessList from "./MobileProcessList.svelte";
  import ProcessTable from "./ProcessTable.svelte";

  export let processRows: ProcessViewRow[] = [];
  export let totalProcessCount = 0;
  export let focusMode: ProcessFocusMode = "all";
  export let searchText = "";
  export let columns: ProcessColumn[] = [];
  export let selectedPid = "";
  export let sortKey: SortKey;
  export let sortDirection: SortDirection;
  export let processIcons: Record<string, string> = {};
  export let rankingUpdateAvailable = false;
  export let onSelect: (pid: string) => void;
  export let onToggleSort: (key: SortKey) => void;
  export let onInteractionChange: (active: boolean) => void;
  export let onExpandedChange: (count: number) => void;

  const resultWindow = 180;

  $: visibleRows = processRows.slice(0, resultWindow);
  $: rankedCount = processRows.filter((row) => row.kind === "group" || !row.is_grouped).length;
  $: countLabel = processCountLabel(rankedCount, totalProcessCount, focusMode, searchText);

  function processCountLabel(
    visibleCount: number,
    totalCount: number,
    mode: ProcessFocusMode,
    filterText: string,
  ): string {
    const scope = filterText.trim() ? "matching" : mode === "active" ? "needing attention" : mode === "io" ? "I/O active" : "ranked";
    return totalCount > 0 && visibleCount !== totalCount
      ? `${visibleCount} ${scope} of ${totalCount}`
      : `${visibleCount} ${scope}`;
  }
</script>

<section class="attention-queue" aria-labelledby="attention-queue-title">
  <header class="queue-heading">
    <div>
      <span>Live values, stable order while you inspect</span>
      <h2 id="attention-queue-title">Attention queue <small>{countLabel}</small></h2>
    </div>
    {#if rankingUpdateAvailable}<span class="queue-hold-label">Order held while inspecting</span>{/if}
  </header>

  <ProcessTable
    processRows={visibleRows}
    totalRowCount={processRows.length}
    {columns}
    {selectedPid}
    {sortKey}
    {sortDirection}
    {processIcons}
    {onSelect}
    {onToggleSort}
    {onInteractionChange}
    {onExpandedChange}
  />
  <MobileProcessList processRows={visibleRows} {selectedPid} {processIcons} {onSelect} />

  {#if processRows.length > visibleRows.length}
    <p class="result-window-note">Showing the first {visibleRows.length} of {processRows.length} ranked rows. Search to narrow the list.</p>
  {/if}
</section>
