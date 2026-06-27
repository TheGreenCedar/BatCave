<script lang="ts">
  import type { ProcessSample, SortDirection } from "../../types";
  import type { ProcessColumn, ProcessRates, SortKey } from "../../process";
  import ProcessTable from "./ProcessTable.svelte";
  import MobileProcessList from "./MobileProcessList.svelte";

  export let processes: ProcessSample[] = [];
  export let columns: ProcessColumn[] = [];
  export let selectedPid = "";
  export let sortKey: SortKey;
  export let sortDirection: SortDirection;
  export let processRates: Record<string, ProcessRates>;
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
    {processes}
    {columns}
    {selectedPid}
    {sortKey}
    {sortDirection}
    {processRates}
    {processIcons}
    {onSelect}
    {onToggleSort}
  />
  <MobileProcessList {processes} {selectedPid} {processRates} {processIcons} {onSelect} />
</section>
