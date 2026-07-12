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
  export let onClose: () => void;

</script>

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
