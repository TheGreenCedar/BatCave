<script lang="ts">
  import type { DetailMode } from "../metrics/types";
  import type { ProcessRates } from "../../process";
  import type { ChartPalette } from "../../themes";
  import type {
    KernelPoolTag,
    ProcessSample,
    RuntimeSnapshot,
    SystemMemoryAccounting,
    SystemMetricQuality,
    TrendState,
  } from "../../types";
  import ProcessInspector from "./ProcessInspector.svelte";
  import SystemDetail from "./SystemDetail.svelte";

  export let subject: "process" | "system";
  export let compact = false;
  export let onClose: () => void = () => {};
  export let onShowSystem: () => void;
  export let selectedProcess: ProcessSample | null;
  export let processHistory: {
    cpu: number[];
    memory: number[];
    readRate: number[];
    writeRate: number[];
    networkRate: number[];
  };
  export let processRates: Record<string, ProcessRates>;
  export let processReadRate = 0;
  export let processWriteRate = 0;
  export let processIcons: Record<string, string> = {};
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

  let pane: HTMLElement | null = null;
  let opener: HTMLElement | null = null;

  $: if (compact && pane instanceof HTMLDialogElement && !pane.open) {
    opener = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    pane.showModal();
  }

  $: if (!compact && opener) restoreOpener();

  function requestClose(): void {
    if (pane instanceof HTMLDialogElement) {
      pane.close();
    }
    restoreOpener();
    onClose();
  }

  function restoreOpener(): void {
    opener?.focus();
    opener = null;
  }

  function handleBackdropClick(event: MouseEvent): void {
    if (event.target === event.currentTarget && event.currentTarget instanceof HTMLDialogElement) {
      requestClose();
    }
  }

  function handleCancel(event: Event): void {
    event.preventDefault();
    requestClose();
  }
</script>

<svelte:element
  this={compact ? "dialog" : "aside"}
  bind:this={pane}
  id="detail-pane"
  class:detail-pane={true}
  class:is-drawer={compact}
  role={compact ? undefined : "complementary"}
  aria-label="Resource detail"
  oncancel={handleCancel}
  onclose={restoreOpener}
  onclick={handleBackdropClick}
>
  <header class="detail-pane-heading">
    <div>
      <span>{subject === "process" ? "Selected workload" : "System resource"}</span>
      <h2>{subject === "process" ? selectedProcess?.name ?? "Process unavailable" : detailTitle}</h2>
    </div>
    <div class="detail-pane-actions">
      {#if subject === "process"}
        <button type="button" onclick={onShowSystem}>System overview</button>
      {:else}
        <strong>{detailReadout}</strong>
      {/if}
      {#if compact}
        <button
          class="detail-pane-close"
          type="button"
          aria-label="Close resource detail"
          onclick={requestClose}
        >
          <svg aria-hidden="true" viewBox="0 0 24 24">
            <path d="M6 6l12 12M18 6 6 18"></path>
          </svg>
        </button>
      {/if}
    </div>
  </header>

  <div class="detail-pane-scroll">
    {#if subject === "process"}
      <ProcessInspector
        {selectedProcess}
        {processHistory}
        {processRates}
        {processReadRate}
        {processWriteRate}
        {processIcons}
        {copyStatus}
        {activeTheme}
        {maxRate}
        {processNetworkLabel}
        {onCopy}
      />
    {:else}
      <SystemDetail
        {detailMode}
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
  </div>
</svelte:element>
