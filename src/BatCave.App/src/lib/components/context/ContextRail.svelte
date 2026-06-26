<script lang="ts">
  import type { DetailMode } from "../metrics/types";
  import type { ChartPalette } from "../../themes";
  import type { KernelPoolTag, ProcessSample, RuntimeSnapshot, SystemMemoryAccounting, SystemMetricQuality, TrendState } from "../../types";
  import type { ProcessRates } from "../../process";
  import ProcessInspector from "./ProcessInspector.svelte";
  import SystemDetail from "./SystemDetail.svelte";

  export let activeTab: "process" | "system";
  export let onTab: (tab: "process" | "system") => void;
  export let selectedProcess: ProcessSample | null;
  export let processHistory: { cpu: number[]; memory: number[]; readRate: number[]; writeRate: number[] };
  export let processRates: Record<string, ProcessRates>;
  export let processReadRate = 0;
  export let processWriteRate = 0;
  export let copyStatus = "";
  export let activeTheme: ChartPalette;
  export let maxRate: (points: number[], fallback: number) => number;
  export let processNetworkLabel: (process: ProcessSample) => string;
  export let onCopy: () => void;
  export let detailMode: DetailMode;
  export let detailTitle: string;
  export let detailReadout: string;
  export let snapshot: RuntimeSnapshot;
  export let history: TrendState;
  export let systemQuality: SystemMetricQuality;
  export let memoryPercent: number;
  export let swapPercent: number;
  export let memoryAccounting: SystemMemoryAccounting | undefined;
  export let topKernelPoolTags: KernelPoolTag[] = [];
  export let diskReadRate = 0;
  export let diskWriteRate = 0;
  export let networkDownRate = 0;
  export let networkUpRate = 0;
  export let diskScaleMax = 1_000_000;
  export let networkScaleMax = 750_000;
  export let coreLoads: { index: number; load: number; trend: number[] }[] = [];
  export let corePeak = 0;
  export let coreSpread = 0;
  export let hotCoreCount = 0;
  export let busyCoreCount = 0;
  export let coreTone: (load: number) => string;
</script>

<aside id="context-rail" class="panel context-rail" aria-label="Context rail">
  <div class="rail-tabs" role="tablist" aria-label="Context view">
    <button
      class:active={activeTab === "process"}
      type="button"
      role="tab"
      aria-selected={activeTab === "process"}
      onclick={() => onTab("process")}
    >
      Process
    </button>
    <button
      class:active={activeTab === "system"}
      type="button"
      role="tab"
      aria-selected={activeTab === "system"}
      onclick={() => onTab("system")}
    >
      System
    </button>
  </div>
  {#if activeTab === "process"}
    <ProcessInspector
      {selectedProcess}
      {processHistory}
      {processRates}
      {processReadRate}
      {processWriteRate}
      {copyStatus}
      {activeTheme}
      {maxRate}
      {processNetworkLabel}
      {onCopy}
    />
  {:else}
    <SystemDetail
      {detailMode}
      {detailTitle}
      {detailReadout}
      {snapshot}
      {history}
      {activeTheme}
      {systemQuality}
      {memoryPercent}
      {swapPercent}
      {memoryAccounting}
      {topKernelPoolTags}
      {diskReadRate}
      {diskWriteRate}
      {networkDownRate}
      {networkUpRate}
      {diskScaleMax}
      {networkScaleMax}
      {coreLoads}
      {corePeak}
      {coreSpread}
      {hotCoreCount}
      {busyCoreCount}
      {coreTone}
    />
  {/if}
</aside>
