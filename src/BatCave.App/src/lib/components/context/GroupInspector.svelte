<script lang="ts">
  import Copy from "phosphor-svelte/lib/Copy";
  import InspectionChart from "../../InspectionChart.svelte";
  import { groupActivitySummary, groupFindingLabel } from "../../format";
  import type { ProcessIconKind } from "../../process";
  import type { ChartPalette } from "../../themes";
  import type { GroupDetail } from "../../types";
  import type { WorkloadInspection } from "../../workloadInspection";
  import ProcessIcon from "../processes/ProcessIcon.svelte";

  // oxlint-disable-next-line no-unassigned-vars -- Svelte assigns this required component prop.
  export let detail: GroupDetail;
  // oxlint-disable-next-line no-unassigned-vars -- Svelte assigns this required component prop.
  export let copyStatus = "";
  export let current = false;
  export let iconKind: ProcessIconKind = "process";
  export let iconSrc: string | undefined = undefined;
  export let iconMatched = false;
  export let onCopy: () => void;
  export let inspection: WorkloadInspection | null = null;
  // oxlint-disable-next-line no-unassigned-vars -- Svelte assigns this required component prop.
  export let activeTheme: ChartPalette;

  $: copyFailed = copyStatus !== "" && copyStatus !== "Workload summary copied.";

  function processCountLabel(count: number): string {
    return `${count} ${count === 1 ? "process" : "processes"}`;
  }

  function hasNotableActivity(): boolean {
    return groupFindingLabel(detail) !== "Aggregate measurements are available for this sample.";
  }
</script>

<section class="process-inspector" aria-label="Workload group inspector">
  <div class="process-identity redesigned-identity">
    <span class="identity-icon">
      <ProcessIcon kind={iconKind} src={iconSrc} matched={iconMatched} />
    </span>
    <span class="identity-copy">
      <span class="identity-title-row">
        <strong title={detail.label}>{detail.label}</strong>
        <small class="identity-chip">{processCountLabel(detail.process_count)}</small>
      </span>
      <span class="identity-meta-row">
        <small class="identity-category">{detail.category}</small>
        <em class="identity-status tone-normal">Aggregate</em>
      </span>
    </span>
    <span class="identity-actions">
      <button
        class="icon-action inspector-copy"
        type="button"
        aria-label="Copy workload group summary"
        title="Copy summary"
        onclick={onCopy}
      >
        <Copy size={18} weight="regular" aria-hidden="true" />
      </button>
    </span>
  </div>

  <section class="current-activity" aria-labelledby="group-current-activity-title">
    <div>
      <span>{current ? "Current activity" : "Last recorded activity"}</span>
      <h3 id="group-current-activity-title">{groupActivitySummary(detail)}</h3>
    </div>
  </section>

  {#if hasNotableActivity()}
    <p class="insight-copy"><strong>Worth noting:</strong> {groupFindingLabel(detail)}</p>
  {/if}

  {#if inspection}<InspectionChart points={inspection.history} retainedPoints={inspection.retained_points} historyTruncated={inspection.history_truncated} {activeTheme} />{/if}

  {#if copyStatus}
    <p
      class="copy-status"
      role={copyFailed ? "alert" : "status"}
      aria-live={copyFailed ? "assertive" : "polite"}
    >
      {copyStatus}
    </p>
  {/if}
</section>
