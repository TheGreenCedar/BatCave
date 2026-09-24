<script lang="ts">
  import { displayProcessName } from "../../cockpit";
  import Copy from "phosphor-svelte/lib/Copy";
  import InspectionChart from "../../InspectionChart.svelte";
  import {
    displayProcessMetricValue,
    processActivityLabel,
    processFindingLabel,
  } from "../../format";
  import {
    platformPresentation,
    type PlatformPresentation,
  } from "../../platformPresentation";
  import type { ChartPalette } from "../../themes";
  import type { WorkloadInspection } from "../../workloadInspection";
  import {
    processIdentity,
    processStatusLabel,
  } from "../../process";
  import {
    resolvedProcessIcon,
    type ResolvedProcessIconCatalog,
  } from "../../processIcons";
  import type { ProcessDetail, ProcessSample } from "../../types";
  import ProcessIcon from "../processes/ProcessIcon.svelte";

  // oxlint-disable-next-line no-unassigned-vars -- Svelte assigns this required component prop.
  export let detail: ProcessDetail;
  export let processReadRate = 0;
  export let processWriteRate = 0;
  export let processIcons: ResolvedProcessIconCatalog = {};
  export let copyStatus = "";
  export let current = false;
  export let presentation: PlatformPresentation = platformPresentation({ platform: "fixture" });
  export let insightNarrative: string | null = null;
  export let insightNarrativeGenerated = false;
  export let onCopy: () => void;
  export let inspection: WorkloadInspection | null = null;
  // oxlint-disable-next-line no-unassigned-vars -- Svelte assigns this required component prop.
  export let activeTheme: ChartPalette;

  $: selectedProcess = detail.process;
  $: copyFailed = copyStatus !== "" && copyStatus !== "Workload summary copied.";

  function processReadWriteIoRate(): number {
    return processReadRate + processWriteRate;
  }

  function processNetworkRate(process: ProcessSample): number {
    return (process.network_received_bps ?? 0) + (process.network_transmitted_bps ?? 0);
  }

  function findingCopy(process: ProcessSample): string {
    return processFindingLabel(
      process,
      processReadWriteIoRate(),
      processNetworkRate(process),
      presentation.memoryLabel,
    );
  }

  function hasNotableFinding(process: ProcessSample): boolean {
    return findingCopy(process) !== "Activity measurements are available for this sample.";
  }

  function accentTone(accent: string): "hot" | "heavy" | "io" | "normal" {
    if (accent === "CPU") return "hot";
    if (accent === "Memory") return "heavy";
    if (accent === "I/O") return "io";
    return "normal";
  }
</script>

<section class="process-inspector" aria-label="Workload inspector">
  {#if selectedProcess}
    {@const identity = processIdentity(selectedProcess)}
    {@const resolvedIcon = resolvedProcessIcon(processIcons, selectedProcess.exe || selectedProcess.name)}
    {@const accent = processActivityLabel(selectedProcess, processReadWriteIoRate(), processNetworkRate(selectedProcess))}
    {@const categoryLabel = identity.group === "Processes" ? null : identity.group}
    <div class="process-identity redesigned-identity">
      <span class="identity-icon">
        <ProcessIcon
          kind={identity.icon}
          child={identity.isChild}
          src={resolvedIcon.src}
          matched={resolvedIcon.origin === "name_match"}
          systemTool={resolvedIcon.systemTool ?? false}
        />
      </span>
      <span class="identity-copy">
        <span class="identity-title-row">
          <strong title={selectedProcess.exe || selectedProcess.name}>{displayProcessName(selectedProcess.name)}</strong>
        </span>
        <span class="identity-meta-row">
          {#if categoryLabel}<small class="identity-category">{categoryLabel}</small>{/if}
          <em class={`identity-status tone-${accentTone(accent)}`}>{accent}</em>
        </span>
      </span>
      <span class="identity-actions">
        <button
          class="icon-action inspector-copy"
          type="button"
          aria-label="Copy workload summary"
          title="Copy summary"
          onclick={onCopy}
        >
          <Copy size={18} weight="regular" aria-hidden="true" />
        </button>
      </span>
    </div>

    <section class="current-activity" aria-labelledby="current-activity-title">
      <div>
        <span>{current ? "Current activity" : "Last recorded activity"}</span>
        <h3 id="current-activity-title">{accent}</h3>
      </div>
      <div>
        <span class="status-caption">Status</span>
        <small>{processStatusLabel(selectedProcess.status)}</small>
      </div>
    </section>

    {#if hasNotableFinding(selectedProcess)}
      <div class="insight-block">
        <p class="insight-copy"><strong>Worth noting:</strong> {insightNarrative ?? findingCopy(selectedProcess)}</p>
        {#if insightNarrativeGenerated}
          <small class="narrative-origin">Locally generated explanation</small>
        {/if}
      </div>
    {/if}

    {#if inspection}<InspectionChart points={inspection.history} retainedPoints={inspection.retained_points} historyTruncated={inspection.history_truncated} {activeTheme} />{/if}

<details class="technical-disclosure inspector-technical">
      <summary>Technical details</summary>
      <dl class="key-value-grid technical-grid">
        <div><dt>Process ID</dt><dd>{selectedProcess.pid}</dd></div>
        <div><dt>Parent</dt><dd>{selectedProcess.parent_pid ?? "Unavailable"}</dd></div>
        <div><dt>Threads</dt><dd>{displayProcessMetricValue(selectedProcess.threads, selectedProcess.quality?.threads, String)}</dd></div>
      </dl>
      <div class="technical-path">
        <span>Executable path</span>
        <code>{selectedProcess.exe || "Path unavailable"}</code>
      </div>
    </details>

    {#if copyStatus}
      <p class="copy-status" role={copyFailed ? "alert" : "status"} aria-live={copyFailed ? "assertive" : "polite"}>
        {copyStatus}
      </p>
    {/if}
  {:else}
    <div class="empty-panel">
      <strong>The selected workload is no longer available</strong>
      <span>Return to the system overview or choose another row from the workload queue.</span>
    </div>
  {/if}
</section>
