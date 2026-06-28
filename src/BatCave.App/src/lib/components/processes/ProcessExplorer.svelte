<script lang="ts">
  import type { ProcessFocusMode, ProcessSample, ProcessViewRow, SortDirection } from "../../types";
  import type { ProcessColumn, SortKey } from "../../process";
  import ProcessTable from "./ProcessTable.svelte";
  import MobileProcessList from "./MobileProcessList.svelte";

  export let processes: ProcessSample[] = [];
  export let processRows: ProcessViewRow[] = [];
  export let totalProcessCount = 0;
  export let focusMode: ProcessFocusMode = "all";
  export let searchText = "";
  export let columns: ProcessColumn[] = [];
  export let selectedPid = "";
  export let sortKey: SortKey;
  export let sortDirection: SortDirection;
  export let processIcons: Record<string, string> = {};
  export let onSelect: (pid: string) => void;
  export let onToggleSort: (key: SortKey) => void;

  $: countLabel = processCountLabel(processes.length, totalProcessCount, focusMode, searchText);

  function processCountLabel(
    visibleCount: number,
    totalCount: number,
    mode: ProcessFocusMode,
    filterText: string,
  ): string {
    const scope = filterText.trim() ? "matching" : mode === "active" ? "busy" : mode === "io" ? "I/O" : "shown";
    return totalCount > 0 && visibleCount !== totalCount
      ? `${visibleCount} ${scope} of ${totalCount}`
      : `${visibleCount} processes`;
  }
</script>

<section class="panel process-panel" aria-label="Process explorer">
  <div class="panel-heading">
    <div>
      <h2>Process explorer <small>{countLabel}</small></h2>
    </div>
  </div>
  <ProcessTable
    {processRows}
    {columns}
    {selectedPid}
    {sortKey}
    {sortDirection}
    {processIcons}
    {onSelect}
    {onToggleSort}
  />
  <MobileProcessList {processRows} {selectedPid} {processIcons} {onSelect} />
</section>
