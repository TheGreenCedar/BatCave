<script lang="ts">
  import { windowProcessViewRows, type ProcessColumn, type SortKey } from "../../process";
  import type { ProcessFocusMode, ProcessViewRow, RuntimePlatform, SortDirection } from "../../types";
  import MobileProcessList from "./MobileProcessList.svelte";
  import ProcessTable from "./ProcessTable.svelte";

  export let processRows: ProcessViewRow[] = [];
  export let totalProcessCount = 0;
  export let focusMode: ProcessFocusMode = "all";
  export let searchText = "";
  export let columns: ProcessColumn[] = [];
  export let selectedWorkloadId = "";
  export let sortKey: SortKey = "attention";
  export let sortDirection: SortDirection;
  export let processIcons: Record<string, string> = {};
  export let rankingUpdateAvailable = false;
  export let platform: RuntimePlatform = "fixture";
  export let onSelect: (pid: string) => void;
  export let onToggleSort: (key: SortKey) => void;
  export let onInteractionChange: (active: boolean) => void;
  export let onExpandedChange: (count: number) => void = () => {};

  const resultWindow = 180;
  let expandedGroups: Record<string, boolean> = {};

  $: visibleRows = windowProcessViewRows(processRows, resultWindow);
  $: visibleGroupKeys = new Set(
    visibleRows.flatMap((row) => (row.kind === "group" ? [row.detail.group_key] : [])),
  );
  $: pruneExpandedGroups(visibleGroupKeys);
  $: rankedCount = processRows.filter((row) => row.kind === "group" || !row.is_grouped).length;
  $: visibleRankedCount = visibleRows.filter((row) => row.kind === "group" || !row.is_grouped).length;
  $: countLabel = processCountLabel(rankedCount, totalProcessCount, focusMode, searchText);
  $: queueTitle = focusMode === "attention" ? "Attention queue" : focusMode === "io" ? "I/O active" : "All apps";
  $: queueEyebrow = sortKey === "attention" ? "Live values, stable order while you inspect" : "Live values, sorted as samples update";

  function processCountLabel(
    visibleCount: number,
    totalCount: number,
    mode: ProcessFocusMode,
    filterText: string,
  ): string {
    const scope = filterText.trim() ? "matching" : mode === "attention" ? "needing attention" : mode === "io" ? "I/O active" : "ranked";
    return totalCount > 0 && visibleCount !== totalCount
      ? `${visibleCount} ${scope} of ${totalCount}`
      : `${visibleCount} ${scope}`;
  }

  function toggleGroup(key: string): void {
    const next = { ...expandedGroups };
    if (next[key]) delete next[key];
    else next[key] = true;
    expandedGroups = next;
    onExpandedChange(Object.keys(next).length);
  }

  function pruneExpandedGroups(visibleKeys: Set<string>): void {
    const currentKeys = Object.keys(expandedGroups);
    if (!currentKeys.some((key) => !visibleKeys.has(key))) return;

    expandedGroups = Object.fromEntries(currentKeys.filter((key) => visibleKeys.has(key)).map((key) => [key, true]));
    onExpandedChange(Object.keys(expandedGroups).length);
  }
</script>

<section
  class="attention-queue"
  aria-labelledby="attention-queue-title"
  data-order-held={rankingUpdateAvailable || undefined}
>
  <header class="queue-heading">
    <div>
      <span>{queueEyebrow}</span>
      <h2 id="attention-queue-title">{queueTitle} <small>{countLabel}</small></h2>
    </div>
  </header>

  <ProcessTable
    processRows={visibleRows}
    {columns}
    {selectedWorkloadId}
    {sortKey}
    {sortDirection}
    {processIcons}
    {expandedGroups}
    {onSelect}
    {onToggleSort}
    onToggleGroup={toggleGroup}
    {onInteractionChange}
    {platform}
  />
  <MobileProcessList
    processRows={visibleRows}
    {selectedWorkloadId}
    {processIcons}
    {expandedGroups}
    {onSelect}
    onToggleGroup={toggleGroup}
    {onInteractionChange}
    {platform}
  />

  {#if rankedCount > visibleRankedCount}
    <p class="result-window-note">Showing the first {visibleRankedCount} of {rankedCount} apps and processes. Search to narrow the list.</p>
  {/if}
</section>
