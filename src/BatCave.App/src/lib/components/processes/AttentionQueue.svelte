<script lang="ts">
  import { onMount, tick } from "svelte";
  import { windowProcessViewRows, type ProcessColumn, type SortKey } from "../../process";
  import type { ResolvedProcessIconCatalog } from "../../processIcons";
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
  export let processIcons: ResolvedProcessIconCatalog = {};
  export let rankingUpdateAvailable = false;
  export let platform: RuntimePlatform = "fixture";
  export let onSelect: (pid: string) => void;
  export let onToggleSort: (key: SortKey) => void;
  // oxlint-disable-next-line no-unassigned-vars -- Svelte assigns this required component prop.
  export let onInteractionChange: (active: boolean) => void;
  export let onExpandedChange: (count: number) => void = () => {};

  const resultWindow = 180;
  let expandedGroups: Record<string, boolean> = {};
  let mobileLayout = false;
  // oxlint-disable-next-line no-unassigned-vars -- Svelte assigns this element binding before onMount.
  let queue: HTMLElement;

  onMount(() => {
    // Keep this boundary aligned with the cards layout in redesign.css.
    const media = window.matchMedia("(max-width: 899px)");
    let revision = 0;
    mobileLayout = media.matches;

    const changeLayout = async (event: MediaQueryListEvent) => {
      if (mobileLayout === event.matches) return;
      const active = document.activeElement;
      const restoreFocus = active instanceof HTMLElement && queue.contains(active);
      const attribute = active?.hasAttribute("data-workload-group-key")
        ? "data-workload-group-key"
        : "data-workload-id";
      const identity = active?.getAttribute(attribute);
      const current = ++revision;
      // Replacing a focused or hovered list must release its ranking hold.
      onInteractionChange(false);
      mobileLayout = event.matches;
      await tick();
      if (current !== revision || !restoreFocus) return;
      const replacement = identity
        ? queue.querySelector<HTMLElement>(`[${attribute}="${CSS.escape(identity)}"]`)
        : null;
      (replacement ?? queue).focus();
    };

    media.addEventListener("change", changeLayout);
    return () => {
      revision += 1;
      media.removeEventListener("change", changeLayout);
      onInteractionChange(false);
    };
  });

  $: visibleRows = windowProcessViewRows(processRows, resultWindow);
  $: visibleGroupKeys = new Set(
    visibleRows.flatMap((row) => (row.kind === "group" ? [row.detail.group_key] : [])),
  );
  $: pruneExpandedGroups(visibleGroupKeys);
  $: rankedCount = processRows.filter((row) => row.kind === "group" || !row.is_grouped).length;
  $: visibleRankedCount = visibleRows.filter((row) => row.kind === "group" || !row.is_grouped).length;
  $: countLabel = processCountLabel(rankedCount, totalProcessCount, focusMode, searchText);
  $: queueTitle = focusMode === "attention" ? "Attention queue" : focusMode === "io" ? "I/O active" : "All apps";

  function processCountLabel(
    visibleCount: number,
    totalCount: number,
    mode: ProcessFocusMode,
    filterText: string,
  ): string {
    const scope = filterText.trim() ? "matching workloads" : mode === "attention" ? "active workloads" : mode === "io" ? "I/O workloads" : "workloads";
    return `${visibleCount} ${scope}${totalCount > 0 ? ` · ${totalCount} processes sampled` : ""}`;
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
  bind:this={queue}
  class="attention-queue"
  aria-labelledby="attention-queue-title"
  tabindex="-1"
  data-order-held={rankingUpdateAvailable || undefined}
>
  <header class="queue-heading">
    <div>
      <h2 id="attention-queue-title">{queueTitle} <small>{countLabel}</small></h2>
    </div>
  </header>

  {#if mobileLayout}
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
  {:else}
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
  {/if}

  {#if rankedCount > visibleRankedCount}
    <p class="result-window-note">Showing the first {visibleRankedCount} of {rankedCount} apps and processes. Search to narrow the list.</p>
  {/if}
</section>
