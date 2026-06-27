<script lang="ts">
  import type { ProcessSample, ProcessViewRow, SortDirection } from "../../types";
  import type { ProcessColumn, SortKey } from "../../process";
  import ProcessTable from "./ProcessTable.svelte";
  import MobileProcessList from "./MobileProcessList.svelte";

  export let processes: ProcessSample[] = [];
  export let processRows: ProcessViewRow[] = [];
  export let columns: ProcessColumn[] = [];
  export let selectedPid = "";
  export let sortKey: SortKey;
  export let sortDirection: SortDirection;
  export let processIcons: Record<string, string> = {};
  export let onSelect: (pid: string) => void;
  export let onToggleSort: (key: SortKey) => void;
</script>

<section class="panel process-panel" aria-label="Process explorer">
  <div class="panel-heading">
    <div>
      <h2>Process explorer <small>{processes.length} processes</small></h2>
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
