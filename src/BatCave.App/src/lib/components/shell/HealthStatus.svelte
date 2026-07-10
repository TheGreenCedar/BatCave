<script lang="ts">
  import type { RuntimeSnapshot, SystemMetricQuality } from "../../types";
  import DiagnosticsDrawer from "./DiagnosticsDrawer.svelte";

  export let snapshot = {} as RuntimeSnapshot;
  export let sourceLabel: string;
  export let systemQuality: SystemMetricQuality;
  export let pollState: "starting" | "native" | "fixture" | "error" = "starting";
  export let lastError = "";
  export let adminStatus = "";
  export let open = false;
  export let onOpen: () => void;
  export let onClose: () => void;

  $: limitationCount = snapshot.warnings.length;
  $: stateLabel =
    pollState === "error"
      ? "Telemetry is stale"
      : snapshot.settings.paused
        ? "Telemetry paused"
        : pollState === "fixture"
          ? "Fixture telemetry"
          : snapshot.health.degraded
            ? `${limitationCount || snapshot.health.collector_warnings} telemetry limitation${(limitationCount || snapshot.health.collector_warnings) === 1 ? "" : "s"}`
            : "Telemetry healthy";
</script>

<footer
  class="health-status"
  class:warning={snapshot.settings.paused || snapshot.health.degraded}
  class:danger={pollState === "error"}
>
  <div>
    <i aria-hidden="true"></i>
    <strong>{stateLabel}</strong>
    <span>
      {pollState === "error"
        ? lastError
        : snapshot.settings.paused
          ? "Values and charts remain at the last successful sample."
          : snapshot.health.degraded
            ? "Unaffected data remains current; open diagnostics for impact and next steps."
            : "Local collectors are current."}
    </span>
  </div>
  <button type="button" onclick={onOpen}>View diagnostics</button>
</footer>

<DiagnosticsDrawer
  {open}
  {snapshot}
  {sourceLabel}
  {systemQuality}
  {pollState}
  {lastError}
  {adminStatus}
  {onClose}
/>
