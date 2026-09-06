<script lang="ts">
  import InspectionChart from "../../InspectionChart.svelte";
  import type { WorkloadInspection } from "../../workloadInspection";
  import X from "phosphor-svelte/lib/X";
  import { focusDialogStart, trapDialogFocus } from "../../dialogFocus";
  import type { TelemetryPresentation } from "../../telemetryPresentation";
  import type { DetailMode } from "../metrics/types";
  import type { ProcessIconKind } from "../../process";
  import type { PlatformPresentation } from "../../platformPresentation";
  import type { ResolvedProcessIconCatalog } from "../../processIcons";
  import type { ChartPalette } from "../../themes";
  import type {
    KernelPoolTag,
    ProcessSample,
    RuntimeSnapshot,
    SystemMemoryAccounting,
    SystemMetricQuality,
    TrendState,
    WorkloadDetail,
  } from "../../types";
  import GroupInspector from "./GroupInspector.svelte";
  import ProcessInspector from "./ProcessInspector.svelte";
  import SystemDetail from "./SystemDetail.svelte";

  export let subject: "process" | "system";
  export let telemetryStatus: TelemetryPresentation;
  export let compact = false;
  export let onClose: () => void = () => {};
  export let onShowSystem: () => void;
  export let selectedWorkload: WorkloadDetail | null;
  export let inspection: WorkloadInspection | null = null;
  export let inspectionLoading = false;
  export let inspectionError = "";
  export let inspectionCurrent = false;
  export let selectedWorkloadIconKind: ProcessIconKind = "process";
  export let selectedWorkloadIconSrc: string | undefined = undefined;
  export let selectedWorkloadIconMatched = false;
  export let processReadRate = 0;
  export let processWriteRate = 0;
  export let processIcons: ResolvedProcessIconCatalog = {};
  export let copyStatus = "";
  export let activeTheme: ChartPalette;
  export let presentation: PlatformPresentation;
  export let processNetworkLabel: (process: ProcessSample) => string;
  export let insightNarrative: string | null = null;
  export let insightNarrativeGenerated = false;
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
    focusDialogStart(pane);
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

  function handleKeydown(event: KeyboardEvent): void {
    if (compact && event.key === "Escape") {
      event.preventDefault();
      requestClose();
      return;
    }
    if (compact && pane instanceof HTMLDialogElement) {
      trapDialogFocus(event, pane);
    }
  }
</script>

<svelte:element
  this={compact ? "dialog" : "aside"}
  bind:this={pane}
  id="detail-pane"
  class:detail-pane={true}
  class:is-drawer={compact}
  class:process-detail={subject === "process"}
  role={compact ? undefined : "complementary"}
  tabindex={compact ? -1 : undefined}
  aria-label="Resource detail"
  oncancel={handleCancel}
  onclose={restoreOpener}
  onkeydown={handleKeydown}
  onclick={handleBackdropClick}
>
  <header class="detail-pane-heading">
    <div>
      <h2>{subject === "process" ? "Workload details" : detailTitle}</h2>
    </div>
    <div class="detail-pane-actions">
      {#if subject === "process"}
        <button class="system-overview-action" type="button" onclick={onShowSystem}>System overview</button>
      {/if}
      {#if compact}
        <button
          class="detail-pane-close"
          type="button"
          aria-label="Close resource detail"
          data-dialog-initial-focus
          onclick={requestClose}
        >
          <X size={19} weight="bold" aria-hidden="true" />
        </button>
      {/if}
    </div>
  </header>

  <div class="detail-pane-scroll">
    {#if telemetryStatus.state !== "live"}
      <p class="detail-freshness" role="status">{telemetryStatus.label}. {telemetryStatus.detail}</p>
    {/if}
    {#if subject === "process"}
      {#if inspectionError}<p class="detail-freshness" role="alert">{inspectionError}</p>
      {:else if inspectionLoading}<p class="detail-freshness" role="status">Loading the selected workload…</p>
      {:else if inspection?.status === "exited"}<p class="detail-freshness" role="status">This identity is no longer in the latest sample. Showing its last recorded activity.</p>
      {:else if inspection && !inspectionCurrent && inspection.status === "current"}<p class="detail-freshness" role="status">Showing the last recorded sample.</p>{/if}
      {#if selectedWorkload?.kind === "process"}
        <ProcessInspector
          detail={selectedWorkload}
          {processReadRate}
          {processWriteRate}
          {processIcons}
          {copyStatus}
          current={inspectionCurrent}
          {presentation}
          platform={snapshot.environment.platform}
          {processNetworkLabel}
          {insightNarrative}
          {insightNarrativeGenerated}
          {onCopy}
        />
      {:else if selectedWorkload?.kind === "group"}
        <GroupInspector
          detail={selectedWorkload}
          {copyStatus}
          current={inspectionCurrent}
          iconKind={selectedWorkloadIconKind}
          iconSrc={selectedWorkloadIconSrc}
          iconMatched={selectedWorkloadIconMatched}
          {onCopy}
        />
      {:else}
        <div class="empty-panel">
          <strong>{inspectionLoading ? "Loading workload" : inspection?.status === "evicted" ? "History was evicted" : "Identity not recorded"}</strong>
          <span>{inspection?.status === "evicted" ? "The bounded history store released this identity to make room for newer samples." : "No retained detail is available for this exact identity."}</span>
        </div>
      {/if}
      {#if selectedWorkload && inspection}<InspectionChart points={inspection.history} retainedPoints={inspection.retained_points} historyTruncated={inspection.history_truncated} {activeTheme} />{/if}
    {:else}
      <SystemDetail
        {detailMode}
        {detailReadout}
        {snapshot}
        {history}
        {activeTheme}
        {presentation}
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
