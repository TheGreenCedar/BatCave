<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { onMount } from "svelte";
  import MiniChart from "./lib/MiniChart.svelte";
  import { makeFixtureSnapshot } from "./lib/fixtures";
  import type {
    MetricQualityInfo,
    ProcessSample,
    RuntimeQuery,
    RuntimeSnapshot,
    SortColumn,
    TrendState,
  } from "./lib/types";

  type FocusMode = "all" | "active" | "io";
  type SortKey = "cpu" | "memory" | "io" | "name";
  type DetailMode = "cpu" | "memory" | "disk" | "network";
  type ThemeName = "cave" | "aurora" | "ember" | "daylight";

  interface ThemeOption {
    name: ThemeName;
    label: string;
  }

  interface ChartPalette {
    cpuStroke: string;
    cpuFill: string;
    memoryStroke: string;
    memoryFill: string;
    diskReadStroke: string;
    diskReadFill: string;
    diskWriteStroke: string;
    diskWriteFill: string;
    networkDownStroke: string;
    networkDownFill: string;
    networkUpStroke: string;
    networkUpFill: string;
    swapStroke: string;
    swapFill: string;
  }

  interface ProcessTrendState {
    cpu: number[];
    memory: number[];
    readRate: number[];
    writeRate: number[];
  }

  interface ProcessRates {
    readRate: number;
    writeRate: number;
    otherRate: number;
  }

  interface CoreLoad {
    index: number;
    load: number;
    trend: number[];
  }

  const historyLimit = 72;
  const pollIntervals = [500, 1000, 2000] as const;
  const themeStorageKey = "batcave.monitor.theme";
  const themeOptions: ThemeOption[] = [
    { name: "cave", label: "Cave" },
    { name: "aurora", label: "Aurora" },
    { name: "ember", label: "Ember" },
    { name: "daylight", label: "Daylight" },
  ];
  const chartPalettes: Record<ThemeName, ChartPalette> = {
    cave: {
      cpuStroke: "#72f1b8",
      cpuFill: "rgba(114, 241, 184, 0.22)",
      memoryStroke: "#8bd5ff",
      memoryFill: "rgba(139, 213, 255, 0.22)",
      diskReadStroke: "#ffd166",
      diskReadFill: "rgba(255, 209, 102, 0.2)",
      diskWriteStroke: "#fca5a5",
      diskWriteFill: "rgba(252, 165, 165, 0.2)",
      networkDownStroke: "#a78bfa",
      networkDownFill: "rgba(167, 139, 250, 0.2)",
      networkUpStroke: "#fb7185",
      networkUpFill: "rgba(251, 113, 133, 0.2)",
      swapStroke: "#a78bfa",
      swapFill: "rgba(167, 139, 250, 0.16)",
    },
    aurora: {
      cpuStroke: "#5eead4",
      cpuFill: "rgba(94, 234, 212, 0.22)",
      memoryStroke: "#93c5fd",
      memoryFill: "rgba(147, 197, 253, 0.24)",
      diskReadStroke: "#c4b5fd",
      diskReadFill: "rgba(196, 181, 253, 0.22)",
      diskWriteStroke: "#f0abfc",
      diskWriteFill: "rgba(240, 171, 252, 0.18)",
      networkDownStroke: "#67e8f9",
      networkDownFill: "rgba(103, 232, 249, 0.18)",
      networkUpStroke: "#bef264",
      networkUpFill: "rgba(190, 242, 100, 0.16)",
      swapStroke: "#c4b5fd",
      swapFill: "rgba(196, 181, 253, 0.16)",
    },
    ember: {
      cpuStroke: "#fbbf24",
      cpuFill: "rgba(251, 191, 36, 0.22)",
      memoryStroke: "#fb7185",
      memoryFill: "rgba(251, 113, 133, 0.2)",
      diskReadStroke: "#fdba74",
      diskReadFill: "rgba(253, 186, 116, 0.22)",
      diskWriteStroke: "#f97316",
      diskWriteFill: "rgba(249, 115, 22, 0.18)",
      networkDownStroke: "#fca5a5",
      networkDownFill: "rgba(252, 165, 165, 0.18)",
      networkUpStroke: "#fde68a",
      networkUpFill: "rgba(253, 230, 138, 0.16)",
      swapStroke: "#fb7185",
      swapFill: "rgba(251, 113, 133, 0.16)",
    },
    daylight: {
      cpuStroke: "#047857",
      cpuFill: "rgba(4, 120, 87, 0.18)",
      memoryStroke: "#0369a1",
      memoryFill: "rgba(3, 105, 161, 0.16)",
      diskReadStroke: "#b45309",
      diskReadFill: "rgba(180, 83, 9, 0.15)",
      diskWriteStroke: "#be123c",
      diskWriteFill: "rgba(190, 18, 60, 0.14)",
      networkDownStroke: "#6d28d9",
      networkDownFill: "rgba(109, 40, 217, 0.14)",
      networkUpStroke: "#0f766e",
      networkUpFill: "rgba(15, 118, 110, 0.14)",
      swapStroke: "#7c3aed",
      swapFill: "rgba(124, 58, 237, 0.12)",
    },
  };

  let fixtureTick = 0;
  let snapshot: RuntimeSnapshot = makeFixtureSnapshot(fixtureTick);
  let selectedPid = snapshot.processes[0]?.pid ?? "";
  let pollState: "starting" | "native" | "fixture" | "error" = "starting";
  let lastError = "";
  let commandError = "";
  let isPaused = false;
  let hasHydratedRuntimeSettings = false;
  let pollIntervalMs: (typeof pollIntervals)[number] = 1000;
  let searchText = "";
  let focusMode: FocusMode = "all";
  let sortKey: SortKey = "cpu";
  let detailMode: DetailMode = "cpu";
  let themeName: ThemeName = "cave";
  let history: TrendState = {
    cpu: [],
    memory: [],
    swap: [],
    diskRead: [],
    diskWrite: [],
    netRx: [],
    netTx: [],
    cores: [],
  };
  let processHistory: ProcessTrendState = {
    cpu: [],
    memory: [],
    readRate: [],
    writeRate: [],
  };
  let processRates: Record<string, ProcessRates> = {};

  $: activeTheme = chartPalettes[themeName];
  $: memoryPercent = percentage(snapshot.system.memory_used_bytes, snapshot.system.memory_total_bytes);
  $: swapPercent = percentage(snapshot.system.swap_used_bytes, snapshot.system.swap_total_bytes);
  $: filteredProcesses = snapshot.processes
    .filter((process) => matchesSearch(process, searchText))
    .filter((process) => matchesFocusMode(process, focusMode))
    .slice()
    .sort((left, right) => compareProcesses(left, right, sortKey));
  $: selectedProcess = filteredProcesses.find((process) => process.pid === selectedPid) ?? null;
  $: topProcess = filteredProcesses[0] ?? null;
  $: warnings =
    snapshot.warnings.length > 0 ? snapshot.warnings.map((warning) => warning.message) : lastError ? [lastError] : [];
  $: sourceLabel =
    snapshot.source === "batcave_runtime" ||
    snapshot.source === "tauri_runtime" ||
    snapshot.source === "tauri_sysinfo"
      ? "native telemetry"
      : "fixture demo";
  $: systemQuality = snapshot.system.quality ?? {};
  $: diskReadRate = history.diskRead.at(-1) ?? 0;
  $: diskWriteRate = history.diskWrite.at(-1) ?? 0;
  $: networkDownRate = history.netRx.at(-1) ?? 0;
  $: networkUpRate = history.netTx.at(-1) ?? 0;
  $: diskScaleMax = maxRate([...history.diskRead, ...history.diskWrite], 1_000_000);
  $: networkScaleMax = maxRate([...history.netRx, ...history.netTx], 750_000);
  $: selectedRates = selectedProcess ? processRates[selectedProcess.pid] : undefined;
  $: processReadRate = selectedRates?.readRate ?? processHistory.readRate.at(-1) ?? 0;
  $: processWriteRate = selectedRates?.writeRate ?? processHistory.writeRate.at(-1) ?? 0;
  $: coreLoads = history.cores.map((core, index) => ({ index, load: currentCoreLoad(core), trend: core }));
  $: coreAverage = average(coreLoads.map((core) => core.load), snapshot.system.cpu_percent);
  $: corePeak = Math.max(...coreLoads.map((core) => core.load), 0);
  $: coreMinimum = coreLoads.length > 0 ? Math.min(...coreLoads.map((core) => core.load)) : 0;
  $: coreSpread = Math.max(0, corePeak - coreMinimum);
  $: hotCoreCount = coreLoads.filter((core) => core.load >= 75).length;
  $: busyCoreCount = coreLoads.filter((core) => core.load >= 45).length;
  $: if (filteredProcesses.length > 0 && !filteredProcesses.some((process) => process.pid === selectedPid)) {
    selectProcess(filteredProcesses[0].pid);
  }

  onMount(() => {
    let timeoutId: number | undefined;
    let disposed = false;
    const savedTheme = window.localStorage.getItem(themeStorageKey);

    if (isThemeName(savedTheme)) {
      themeName = savedTheme;
    }

    ingest(snapshot);

    const loop = async () => {
      if (!isPaused) {
        const next = await readSnapshot();
        ingest(next);
      }

      if (!disposed) {
        timeoutId = window.setTimeout(loop, pollIntervalMs);
      }
    };

    timeoutId = window.setTimeout(loop, 120);

    return () => {
      disposed = true;
      if (timeoutId !== undefined) {
        window.clearTimeout(timeoutId);
      }
    };
  });

  async function readSnapshot(): Promise<RuntimeSnapshot> {
    if (!hasTauriRuntime()) {
      fixtureTick += 1;
      pollState = "fixture";
      lastError = "";
      return makeFixtureSnapshot(fixtureTick);
    }

    try {
      const nativeSnapshot = await invoke<RuntimeSnapshot>("get_snapshot");
      pollState = "native";
      lastError = "";
      return nativeSnapshot;
    } catch (error) {
      fixtureTick += 1;
      pollState = "fixture";
      lastError = error instanceof Error ? error.message : "Native telemetry unavailable in browser dev mode.";
      return makeFixtureSnapshot(fixtureTick);
    }
  }

  function hasTauriRuntime(): boolean {
    return "__TAURI_INTERNALS__" in window;
  }

  function isThemeName(value: string | null): value is ThemeName {
    return themeOptions.some((theme) => theme.name === value);
  }

  function setTheme(name: ThemeName): void {
    themeName = name;
    window.localStorage.setItem(themeStorageKey, name);
  }

  async function setPaused(nextPaused: boolean): Promise<void> {
    const previousPaused = isPaused;
    isPaused = nextPaused;
    if (!hasTauriRuntime()) {
      return;
    }

    try {
      const next = await invoke<RuntimeSnapshot>(nextPaused ? "pause_runtime" : "resume_runtime");
      pollState = "native";
      lastError = "";
      commandError = "";
      ingest(next);
    } catch (error) {
      isPaused = previousPaused;
      commandError = error instanceof Error ? error.message : "Unable to change runtime pause state.";
    }
  }

  async function refreshNow(): Promise<void> {
    if (!hasTauriRuntime()) {
      fixtureTick += 1;
      ingest(makeFixtureSnapshot(fixtureTick));
      return;
    }

    try {
      const next = await invoke<RuntimeSnapshot>("refresh_now");
      pollState = "native";
      lastError = "";
      commandError = "";
      ingest(next);
    } catch (error) {
      commandError = error instanceof Error ? error.message : "Unable to refresh runtime.";
    }
  }

  async function setAdminMode(enabled: boolean): Promise<void> {
    if (!hasTauriRuntime()) {
      return;
    }

    try {
      const next = await invoke<RuntimeSnapshot>("set_admin_mode", { enabled });
      pollState = "native";
      lastError = "";
      commandError = "";
      ingest(next);
    } catch (error) {
      commandError = error instanceof Error ? error.message : "Unable to change admin mode.";
    }
  }

  function setSortKey(key: SortKey): void {
    sortKey = key;
    void syncRuntimeQuery();
  }

  function setSearchText(value: string): void {
    searchText = value;
    void syncRuntimeQuery();
  }

  async function syncRuntimeQuery(): Promise<void> {
    if (!hasTauriRuntime()) {
      return;
    }

    const query: RuntimeQuery = {
      filter_text: searchText,
      sort_column: sortColumnForKey(sortKey),
      sort_direction: sortKey === "name" ? "asc" : "desc",
      limit: 5000,
    };

    try {
      const next = await invoke<RuntimeSnapshot>("set_process_query", { query });
      pollState = "native";
      lastError = "";
      commandError = "";
      ingest(next);
    } catch (error) {
      commandError = error instanceof Error ? error.message : "Unable to update runtime query.";
    }
  }

  function ingest(next: RuntimeSnapshot): void {
    const previous = snapshot;
    hydrateRuntimeControls(next);
    const previousProcess = previous.processes.find((process) => process.pid === selectedPid);
    const nextProcess = selectedPid ? next.processes.find((process) => process.pid === selectedPid) : next.processes[0];
    const elapsedSeconds = Math.max(0.5, (next.ts_ms - previous.ts_ms) / 1000);
    const diskRead =
      next.system.disk_read_bps ||
      byteRate(next.system.disk_read_total_bytes, previous.system.disk_read_total_bytes, elapsedSeconds);
    const diskWrite =
      next.system.disk_write_bps ||
      byteRate(next.system.disk_write_total_bytes, previous.system.disk_write_total_bytes, elapsedSeconds);
    const netRx =
      next.system.network_received_bps ||
      byteRate(next.system.network_received_total_bytes, previous.system.network_received_total_bytes, elapsedSeconds);
    const netTx =
      next.system.network_transmitted_bps ||
      byteRate(
        next.system.network_transmitted_total_bytes,
        previous.system.network_transmitted_total_bytes,
        elapsedSeconds,
      );
    const logicalCpu = next.system.logical_cpu_percent.length
      ? next.system.logical_cpu_percent
      : [next.system.cpu_percent];
    const nextMemoryPercent = percentage(next.system.memory_used_bytes, next.system.memory_total_bytes);
    const nextSwapPercent = percentage(next.system.swap_used_bytes, next.system.swap_total_bytes);
    processRates = buildProcessRates(previous.processes, next.processes, elapsedSeconds);
    isPaused = next.settings.paused;

    if (!selectedPid && nextProcess) {
      selectedPid = nextProcess.pid;
      resetProcessHistory(nextProcess);
    } else if (nextProcess) {
      const nextRates = processRates[nextProcess.pid] ?? { readRate: 0, writeRate: 0, otherRate: 0 };
      processHistory = {
        cpu: pushPoint(processHistory.cpu, nextProcess.cpu_percent),
        memory: pushPoint(
          processHistory.memory,
          percentage(nextProcess.memory_bytes, Math.max(next.system.memory_total_bytes, 1)),
        ),
        readRate: pushPoint(processHistory.readRate, nextRates.readRate),
        writeRate: pushPoint(processHistory.writeRate, nextRates.writeRate),
      };
    } else if (previousProcess) {
      processHistory = {
        cpu: pushPoint(processHistory.cpu, 0),
        memory: pushPoint(processHistory.memory, 0),
        readRate: pushPoint(processHistory.readRate, 0),
        writeRate: pushPoint(processHistory.writeRate, 0),
      };
    }

    snapshot = next;
    history = {
      cpu: pushPoint(history.cpu, next.system.cpu_percent),
      memory: pushPoint(history.memory, nextMemoryPercent),
      swap: pushPoint(history.swap, nextSwapPercent),
      diskRead: pushPoint(history.diskRead, diskRead),
      diskWrite: pushPoint(history.diskWrite, diskWrite),
      netRx: pushPoint(history.netRx, netRx),
      netTx: pushPoint(history.netTx, netTx),
      cores: logicalCpu.map((value, index) => pushPoint(history.cores[index] ?? [], value)),
    };
  }

  function hydrateRuntimeControls(next: RuntimeSnapshot): void {
    if (next.source === "fixture" || hasHydratedRuntimeSettings) {
      return;
    }

    searchText = next.settings.query.filter_text;
    sortKey = sortKeyForColumn(next.settings.query.sort_column);
    isPaused = next.settings.paused;
    hasHydratedRuntimeSettings = true;
  }

  function selectProcess(pid: string): void {
    selectedPid = pid;
    const process = snapshot.processes.find((candidate) => candidate.pid === pid);
    if (process) {
      resetProcessHistory(process);
    }
  }

  function setDetailMode(mode: DetailMode): void {
    detailMode = mode;

    window.requestAnimationFrame(() => {
      const detailPanelElement = document.getElementById("resource-detail-panel");

      if (!detailPanelElement) {
        return;
      }

      const bounds = detailPanelElement.getBoundingClientRect();
      const viewportHeight = window.innerHeight || document.documentElement.clientHeight;

      if (bounds.top > viewportHeight * 0.62) {
        window.scrollBy({ top: bounds.top - 72, behavior: "auto" });
      }
    });
  }

  function resetProcessHistory(process: ProcessSample): void {
    processHistory = {
      cpu: [process.cpu_percent],
      memory: [percentage(process.memory_bytes, Math.max(snapshot.system.memory_total_bytes, 1))],
      readRate: [0],
      writeRate: [0],
    };
  }

  function resetHistory(): void {
    history = {
      cpu: [],
      memory: [],
      swap: [],
      diskRead: [],
      diskWrite: [],
      netRx: [],
      netTx: [],
      cores: [],
    };
    if (selectedProcess) {
      resetProcessHistory(selectedProcess);
    }
  }

  function pushPoint(points: number[], value: number): number[] {
    return [...points, Number.isFinite(value) ? value : 0].slice(-historyLimit);
  }

  function byteRate(current: number, previous: number, elapsedSeconds: number): number {
    return Math.max(0, (current - previous) / elapsedSeconds);
  }

  function buildProcessRates(
    previousProcesses: ProcessSample[],
    nextProcesses: ProcessSample[],
    elapsedSeconds: number,
  ): Record<string, ProcessRates> {
    const previousByPid = new Map(previousProcesses.map((process) => [process.pid, process]));
    const rates: Record<string, ProcessRates> = {};

    for (const process of nextProcesses) {
      const previousProcess = previousByPid.get(process.pid);
      rates[process.pid] = {
        readRate:
          process.disk_read_bps ||
          byteRate(
            process.disk_read_total_bytes,
            previousProcess?.disk_read_total_bytes ?? process.disk_read_total_bytes,
            elapsedSeconds,
          ),
        otherRate:
          process.other_io_bps ??
          byteRate(process.other_io_total_bytes ?? 0, previousProcess?.other_io_total_bytes ?? 0, elapsedSeconds),
        writeRate:
          process.disk_write_bps ||
          byteRate(
            process.disk_write_total_bytes,
            previousProcess?.disk_write_total_bytes ?? process.disk_write_total_bytes,
            elapsedSeconds,
          ),
      };
    }

    return rates;
  }

  function percentage(value: number, total: number): number {
    if (total <= 0) {
      return 0;
    }

    return Math.min(100, Math.max(0, (value / total) * 100));
  }

  function currentCoreLoad(points: number[]): number {
    return boundedPercent(points.at(-1) ?? 0);
  }

  function boundedPercent(value: number): number {
    return Math.min(100, Math.max(0, Number.isFinite(value) ? value : 0));
  }

  function average(values: number[], fallback: number): number {
    if (values.length === 0) {
      return fallback;
    }

    return values.reduce((total, value) => total + value, 0) / values.length;
  }

  function coreTone(load: number): string {
    if (load >= 75) {
      return "hot";
    }

    if (load >= 45) {
      return "busy";
    }

    return "cool";
  }

  function processIoRate(process: ProcessSample): number {
    const rates = processRates[process.pid];
    return (rates?.readRate ?? 0) + (rates?.writeRate ?? 0) + (rates?.otherRate ?? 0);
  }

  function compareProcesses(left: ProcessSample, right: ProcessSample, key: SortKey): number {
    switch (key) {
      case "memory":
        return right.memory_bytes - left.memory_bytes || right.cpu_percent - left.cpu_percent;
      case "io":
        return processIoRate(right) - processIoRate(left) || right.cpu_percent - left.cpu_percent;
      case "name":
        return left.name.localeCompare(right.name, undefined, { sensitivity: "base" });
      case "cpu":
      default:
        return right.cpu_percent - left.cpu_percent || right.memory_bytes - left.memory_bytes;
    }
  }

  function sortColumnForKey(key: SortKey): SortColumn {
    switch (key) {
      case "memory":
        return "memory_bytes";
      case "io":
        return "disk_bps";
      case "name":
        return "name";
      case "cpu":
      default:
        return "cpu_pct";
    }
  }

  function sortKeyForColumn(column: SortColumn): SortKey {
    switch (column) {
      case "memory_bytes":
        return "memory";
      case "disk_bps":
        return "io";
      case "name":
        return "name";
      case "attention":
      case "pid":
      case "cpu_pct":
      case "threads":
      case "handles":
      case "start_time_ms":
      default:
        return "cpu";
    }
  }

  function matchesSearch(process: ProcessSample, query: string): boolean {
    const normalized = query.trim().toLocaleLowerCase();
    if (!normalized) {
      return true;
    }

    return (
      process.name.toLocaleLowerCase().includes(normalized) ||
      process.pid.includes(normalized) ||
      process.exe.toLocaleLowerCase().includes(normalized)
    );
  }

  function matchesFocusMode(process: ProcessSample, mode: FocusMode): boolean {
    if (mode === "active") {
      return process.cpu_percent >= 1;
    }

    if (mode === "io") {
      return processIoRate(process) > 0;
    }

    return true;
  }

  function formatBytes(value: number): string {
    const units = ["B", "KB", "MB", "GB", "TB"];
    let amount = Math.max(0, value);
    let unit = 0;
    while (amount >= 1024 && unit < units.length - 1) {
      amount /= 1024;
      unit += 1;
    }
    return `${amount >= 10 || unit === 0 ? amount.toFixed(0) : amount.toFixed(1)} ${units[unit]}`;
  }

  function formatRate(value: number): string {
    return `${formatBytes(value)}/s`;
  }

  function formatPercent(value: number): string {
    return `${Math.round(value)}%`;
  }

  function formatInterval(value: number): string {
    return value < 1000 ? `${value} ms` : `${value / 1000}s`;
  }

  function metricQualityLabel(metric: MetricQualityInfo | undefined, fallback: string): string {
    if (!metric) {
      return fallback;
    }

    const quality = formatMetricQuality(metric.quality);
    const source = metric.source ? formatMetricSource(metric.source) : "";
    return source ? `${quality} / ${source}` : quality;
  }

  function formatMetricQuality(value: string): string {
    switch (value) {
      case "native":
        return "Native";
      case "estimated":
        return "Estimated";
      case "held":
        return "Held";
      case "partial":
        return "Partial";
      case "unavailable":
        return "Unavailable";
      default:
        return value;
    }
  }

  function formatMetricSource(value: string): string {
    switch (value) {
      case "direct_api":
        return "direct API";
      case "interface_aggregate":
        return "interface aggregate";
      case "process_aggregate":
        return "process aggregate";
      default:
        return value.replaceAll("_", " ");
    }
  }

  function accessLabel(process: ProcessSample): string {
    if (process.access_state === "full") {
      return "Full";
    }

    return process.access_state === "partial" ? "Partial" : "Denied";
  }

  function processNetworkLabel(process: ProcessSample): string {
    const quality = process.quality?.network;
    const rate = (process.network_received_bps ?? 0) + (process.network_transmitted_bps ?? 0);

    if (rate > 0 && quality?.quality !== "unavailable") {
      const suffix = quality ? ` ${metricQualityLabel(quality, "")}` : "";
      return `${formatRate(rate)}${suffix}`;
    }

    return metricQualityLabel(quality, "Unavailable");
  }

  function processAccent(process: ProcessSample | undefined): string {
    if (!process) {
      return "Idle";
    }

    if (process.cpu_percent >= 30) {
      return "Hot";
    }

    if (process.memory_bytes >= 900 * 1024 * 1024) {
      return "Heavy";
    }

    if (processIoRate(process) >= 500 * 1024) {
      return "I/O";
    }

    return "Stable";
  }

  function processHint(process: ProcessSample): string {
    if (process.cpu_percent >= 20) {
      return "CPU lead";
    }

    if (process.memory_bytes >= 900 * 1024 * 1024) {
      return "memory lead";
    }

    if (processIoRate(process) >= 500 * 1024) {
      return "I/O lead";
    }

    return "watch";
  }

  function maxRate(points: number[], fallback: number): number {
    return Math.max(fallback, Math.max(...points, 0) * 1.2);
  }

  function needsReadoutContrast(value: number, max: number): boolean {
    return max > 0 && value / max >= 0.82;
  }
</script>

<svelte:head>
  <title>BatCave Monitor</title>
</svelte:head>

<main class="app-shell" data-theme={themeName}>
  <header class="topbar">
    <div>
      <p class="eyebrow">BatCave monitor</p>
      <h1>Resource cockpit</h1>
    </div>
    <div class="topbar-tools">
      <div class="theme-picker" role="group" aria-label="Theme">
        {#each themeOptions as theme}
          <button
            class:active={themeName === theme.name}
            type="button"
            aria-pressed={themeName === theme.name}
            onclick={() => setTheme(theme.name)}
          >
            {theme.label}
          </button>
        {/each}
      </div>
      <div class="status-stack" aria-label="Runtime status">
        <span class:live={pollState === "native"} class:paused={isPaused}>{isPaused ? "paused" : pollState}</span>
        <span>{snapshot.system.process_count} processes</span>
        <span>{snapshot.health.snapshot_latency_ms} ms</span>
      </div>
    </div>
  </header>

  <section class="metric-band" aria-label="System metrics">
    <button
      class="metric-card"
      class:active={detailMode === "cpu"}
      type="button"
      aria-controls="resource-detail-panel"
      aria-label="Open CPU logical core detail"
      onclick={() => setDetailMode("cpu")}
    >
      <MiniChart values={history.cpu} max={100} stroke={activeTheme.cpuStroke} fill={activeTheme.cpuFill} />
      <span class="metric-copy" class:on-fill={needsReadoutContrast(snapshot.system.cpu_percent, 100)}>
        <span>CPU</span>
        <strong>{formatPercent(snapshot.system.cpu_percent)}</strong>
        <small>Logical cores</small>
      </span>
    </button>
    <button
      class="metric-card"
      class:active={detailMode === "memory"}
      type="button"
      aria-controls="resource-detail-panel"
      aria-label="Open memory detail"
      onclick={() => setDetailMode("memory")}
    >
      <MiniChart values={history.memory} max={100} stroke={activeTheme.memoryStroke} fill={activeTheme.memoryFill} />
      <span class="metric-copy" class:on-fill={needsReadoutContrast(memoryPercent, 100)}>
        <span>Memory</span>
        <strong>{formatPercent(memoryPercent)}</strong>
        <small>{formatBytes(snapshot.system.memory_used_bytes)} used</small>
      </span>
    </button>
    <button
      class="metric-card"
      class:active={detailMode === "disk"}
      type="button"
      aria-controls="resource-detail-panel"
      aria-label="Open disk detail from read throughput"
      onclick={() => setDetailMode("disk")}
    >
      <MiniChart
        values={history.diskRead}
        max={diskScaleMax}
        stroke={activeTheme.diskReadStroke}
        fill={activeTheme.diskReadFill}
      />
      <span class="metric-copy" class:on-fill={needsReadoutContrast(diskReadRate, diskScaleMax)}>
        <span>Disk read</span>
        <strong>{formatRate(diskReadRate)}</strong>
        <small>{metricQualityLabel(systemQuality.disk, "Aggregate")}</small>
      </span>
    </button>
    <button
      class="metric-card"
      class:active={detailMode === "disk"}
      type="button"
      aria-controls="resource-detail-panel"
      aria-label="Open disk detail from write throughput"
      onclick={() => setDetailMode("disk")}
    >
      <MiniChart
        values={history.diskWrite}
        max={diskScaleMax}
        stroke={activeTheme.diskWriteStroke}
        fill={activeTheme.diskWriteFill}
      />
      <span class="metric-copy" class:on-fill={needsReadoutContrast(diskWriteRate, diskScaleMax)}>
        <span>Disk write</span>
        <strong>{formatRate(diskWriteRate)}</strong>
        <small>{metricQualityLabel(systemQuality.disk, "Aggregate")}</small>
      </span>
    </button>
    <button
      class="metric-card"
      class:active={detailMode === "network"}
      type="button"
      aria-controls="resource-detail-panel"
      aria-label="Open network detail from download throughput"
      onclick={() => setDetailMode("network")}
    >
      <MiniChart
        values={history.netRx}
        max={networkScaleMax}
        stroke={activeTheme.networkDownStroke}
        fill={activeTheme.networkDownFill}
      />
      <span class="metric-copy" class:on-fill={needsReadoutContrast(networkDownRate, networkScaleMax)}>
        <span>Network down</span>
        <strong>{formatRate(networkDownRate)}</strong>
        <small>{metricQualityLabel(systemQuality.network, "Aggregate")}</small>
      </span>
    </button>
    <button
      class="metric-card"
      class:active={detailMode === "network"}
      type="button"
      aria-controls="resource-detail-panel"
      aria-label="Open network detail from upload throughput"
      onclick={() => setDetailMode("network")}
    >
      <MiniChart
        values={history.netTx}
        max={networkScaleMax}
        stroke={activeTheme.networkUpStroke}
        fill={activeTheme.networkUpFill}
      />
      <span class="metric-copy" class:on-fill={needsReadoutContrast(networkUpRate, networkScaleMax)}>
        <span>Network up</span>
        <strong>{formatRate(networkUpRate)}</strong>
        <small>{metricQualityLabel(systemQuality.network, "Aggregate")}</small>
      </span>
    </button>
  </section>

  <section class="triage-grid">
    <div class="panel process-panel">
      <div class="panel-heading">
        <div>
          <p class="eyebrow">Attention queue</p>
          <h2>{topProcess?.name ?? "No matching processes"}</h2>
        </div>
        <strong>{topProcess ? formatPercent(topProcess.cpu_percent) : "--"}</strong>
      </div>
      {#if selectedProcess}
        <section class="mobile-selected-summary" aria-label="Selected process summary">
          <span class="summary-title">
            <span>
              <small>Selected</small>
              <strong>{selectedProcess.name}</strong>
            </span>
            <span>{selectedProcess.pid}</span>
          </span>
          <span class="summary-metrics" aria-label="Selected process metrics">
            <span>
              <em>CPU</em>
              <b>{formatPercent(selectedProcess.cpu_percent)}</b>
            </span>
            <span>
              <em>Memory</em>
              <b>{formatBytes(selectedProcess.memory_bytes)}</b>
            </span>
            <span>
              <em>I/O</em>
              <b>{formatRate(processIoRate(selectedProcess))}</b>
            </span>
          </span>
        </section>
      {:else}
        <section class="mobile-selected-summary muted" aria-label="Selected process summary">
          <span class="summary-title">
            <span>
              <small>Selected</small>
              <strong>No process</strong>
            </span>
            <span>--</span>
          </span>
        </section>
      {/if}
      <div class="table-wrap">
        <table>
          <thead>
            <tr>
              <th>Process</th>
              <th>PID</th>
              <th>CPU</th>
              <th>Memory</th>
              <th>I/O rate</th>
              <th>Read total</th>
              <th>Write total</th>
              <th>Status</th>
            </tr>
          </thead>
          <tbody>
            {#each filteredProcesses as process}
              <tr class:selected={process.pid === selectedPid}>
                <td>
                  <button
                    class="process-button"
                    class:selected={process.pid === selectedPid}
                    type="button"
                    aria-pressed={process.pid === selectedPid}
                    onclick={() => selectProcess(process.pid)}
                  >
                    <span>{process.name}</span>
                    <small>{processHint(process)}</small>
                  </button>
                </td>
                <td>{process.pid}</td>
                <td>{formatPercent(process.cpu_percent)}</td>
                <td>{formatBytes(process.memory_bytes)}</td>
                <td>{formatRate(processIoRate(process))}</td>
                <td>{formatBytes(process.disk_read_total_bytes)}</td>
                <td>{formatBytes(process.disk_write_total_bytes)}</td>
                <td>{process.status}</td>
              </tr>
            {:else}
              <tr>
                <td class="empty-state" colspan="8">No process matches this view.</td>
              </tr>
            {/each}
          </tbody>
        </table>
      </div>
      <div class="mobile-process-list" aria-label="Attention queue cards">
        {#each filteredProcesses.slice(0, 10) as process}
          <button
            class="mobile-process-card"
            class:selected={process.pid === selectedPid}
            type="button"
            aria-pressed={process.pid === selectedPid}
            onclick={() => selectProcess(process.pid)}
          >
            <span class="card-title-row">
              <span>{process.name}</span>
              <small>{processHint(process)}</small>
            </span>
            <span class="card-metrics">
              <span>
                <em>CPU</em>
                <b>{formatPercent(process.cpu_percent)}</b>
              </span>
              <span>
                <em>Memory</em>
                <b>{formatBytes(process.memory_bytes)}</b>
              </span>
              <span>
                <em>I/O</em>
                <b>{formatRate(processIoRate(process))}</b>
              </span>
            </span>
            <span class="card-foot">
              <span>PID {process.pid}</span>
              <span>{process.status}</span>
            </span>
          </button>
        {:else}
          <div class="mobile-empty-state">No process matches this view.</div>
        {/each}
      </div>
    </div>

    <aside class="panel inspector" aria-label="Process inspector">
      <div class="panel-heading">
        <div>
          <p class="eyebrow">Selected process</p>
          <h2>{selectedProcess?.name ?? "No process"}</h2>
        </div>
        <strong>{processAccent(selectedProcess ?? undefined)}</strong>
      </div>
      {#if selectedProcess}
        <div class="inspector-charts" aria-label="Selected process trends">
          <div class="inspector-chart">
            <span>CPU</span>
            <strong>{formatPercent(selectedProcess.cpu_percent)}</strong>
            <MiniChart values={processHistory.cpu} max={100} stroke={activeTheme.cpuStroke} fill={activeTheme.cpuFill} />
          </div>
          <div class="inspector-chart">
            <span>Read</span>
            <strong>{formatRate(processReadRate)}</strong>
            <MiniChart
              values={processHistory.readRate}
              max={maxRate(processHistory.readRate, 250_000)}
              stroke={activeTheme.diskReadStroke}
              fill={activeTheme.diskReadFill}
            />
          </div>
        </div>
        <dl class="inspector-grid">
          <div>
            <dt>PID</dt>
            <dd>{selectedProcess.pid}</dd>
          </div>
          <div>
            <dt>Parent</dt>
            <dd>{selectedProcess.parent_pid ?? "--"}</dd>
          </div>
          <div>
            <dt>CPU</dt>
            <dd>{formatPercent(selectedProcess.cpu_percent)}</dd>
          </div>
          <div>
            <dt>Kernel CPU</dt>
            <dd>
              {selectedProcess.kernel_cpu_percent === undefined
                ? "--"
                : formatPercent(selectedProcess.kernel_cpu_percent)}
            </dd>
          </div>
          <div>
            <dt>Memory</dt>
            <dd>{formatBytes(selectedProcess.memory_bytes)}</dd>
          </div>
          <div>
            <dt>Private</dt>
            <dd>{formatBytes(selectedProcess.private_bytes)}</dd>
          </div>
          <div>
            <dt>Write rate</dt>
            <dd>{formatRate(processWriteRate)}</dd>
          </div>
          <div>
            <dt>Other I/O</dt>
            <dd>{formatRate(processRates[selectedProcess.pid]?.otherRate ?? selectedProcess.other_io_bps ?? 0)}</dd>
          </div>
          <div>
            <dt>Read total</dt>
            <dd>{formatBytes(selectedProcess.disk_read_total_bytes)}</dd>
          </div>
          <div>
            <dt>Write total</dt>
            <dd>{formatBytes(selectedProcess.disk_write_total_bytes)}</dd>
          </div>
          <div>
            <dt>Threads</dt>
            <dd>{selectedProcess.threads || "--"}</dd>
          </div>
          <div>
            <dt>Handles</dt>
            <dd>{selectedProcess.handles || "--"}</dd>
          </div>
          <div>
            <dt>Access</dt>
            <dd>{accessLabel(selectedProcess)}</dd>
          </div>
          <div>
            <dt>Network</dt>
            <dd>{processNetworkLabel(selectedProcess)}</dd>
          </div>
        </dl>
        <p class="path">{selectedProcess.exe || "Path unavailable"}</p>
      {:else}
        <div class="empty-panel">
          <strong>No selected process</strong>
          <span>Clear the search or change the focus filter to inspect a process.</span>
        </div>
      {/if}
    </aside>
  </section>

  <section class="command-bar" aria-label="Monitor controls">
    <div class="control-group grow">
      <label for="process-search">Find</label>
      <input
        id="process-search"
        class="search-input"
        bind:value={searchText}
        oninput={(event) => setSearchText(event.currentTarget.value)}
        placeholder="Process, PID, or path"
        autocomplete="off"
      />
    </div>
    <div class="control-group focus-group">
      <span>Focus</span>
      <div class="segmented" role="group" aria-label="Process focus">
        <button
          class:active={focusMode === "all"}
          type="button"
          aria-pressed={focusMode === "all"}
          onclick={() => (focusMode = "all")}
        >
          All
        </button>
        <button
          class:active={focusMode === "active"}
          type="button"
          aria-pressed={focusMode === "active"}
          onclick={() => (focusMode = "active")}
        >
          Active
        </button>
        <button
          class:active={focusMode === "io"}
          type="button"
          aria-pressed={focusMode === "io"}
          onclick={() => (focusMode = "io")}
        >
          I/O
        </button>
      </div>
    </div>
    <div class="control-group sort-group">
      <span>Sort</span>
      <div class="segmented" role="group" aria-label="Process sort">
        <button
          class:active={sortKey === "cpu"}
          type="button"
          aria-pressed={sortKey === "cpu"}
          onclick={() => setSortKey("cpu")}
        >
          CPU
        </button>
        <button
          class:active={sortKey === "memory"}
          type="button"
          aria-pressed={sortKey === "memory"}
          onclick={() => setSortKey("memory")}
        >
          Memory
        </button>
        <button
          class:active={sortKey === "io"}
          type="button"
          aria-pressed={sortKey === "io"}
          onclick={() => setSortKey("io")}
        >
          I/O
        </button>
        <button
          class:active={sortKey === "name"}
          type="button"
          aria-pressed={sortKey === "name"}
          onclick={() => setSortKey("name")}
        >
          Name
        </button>
      </div>
    </div>
    <div class="control-group refresh-group">
      <span>Refresh</span>
      <div class="segmented" role="group" aria-label="Refresh rate">
        {#each pollIntervals as interval}
          <button
            class:active={pollIntervalMs === interval}
            type="button"
            aria-pressed={pollIntervalMs === interval}
            onclick={() => (pollIntervalMs = interval)}
          >
            {formatInterval(interval)}
          </button>
        {/each}
      </div>
    </div>
    <div class="control-actions">
      <button class="primary-action" type="button" onclick={() => setPaused(!isPaused)}>
        {isPaused ? "Resume" : "Pause"}
      </button>
      <button type="button" onclick={refreshNow}>Refresh</button>
      <button type="button" onclick={() => setAdminMode(!snapshot.settings.admin_mode_requested)}>
        {snapshot.settings.admin_mode_requested ? "Use standard" : "Request admin"}
      </button>
      <button type="button" onclick={resetHistory}>Reset</button>
    </div>
  </section>

  <section class="details-grid">
    <section
      id="resource-detail-panel"
      class="panel detail-panel"
      aria-label="Resource detail view"
    >
      <div class="panel-heading">
        <div>
          <p class="eyebrow">Detail view</p>
          <h2>
            {#if detailMode === "cpu"}
              Logical cores
            {:else if detailMode === "memory"}
              Memory detail
            {:else if detailMode === "disk"}
              Disk throughput
            {:else}
              Network throughput
            {/if}
          </h2>
        </div>
        <strong>
          {#if detailMode === "cpu"}
            {formatPercent(coreAverage)} avg
          {:else if detailMode === "memory"}
            {formatPercent(memoryPercent)} used
          {:else if detailMode === "disk"}
            {formatRate(diskReadRate + diskWriteRate)}
          {:else}
            {formatRate(networkDownRate + networkUpRate)}
          {/if}
        </strong>
      </div>
      {#if detailMode === "cpu"}
        <div class="detail-summary" aria-label="CPU distribution summary">
          <div>
            <span>Peak</span>
            <strong>{formatPercent(corePeak)}</strong>
          </div>
          <div>
            <span>Hot cores</span>
            <strong>{hotCoreCount}</strong>
          </div>
          <div>
            <span>Busy</span>
            <strong>{busyCoreCount}</strong>
          </div>
          <div>
            <span>Spread</span>
            <strong>{formatPercent(coreSpread)}</strong>
          </div>
        </div>
        <div class="core-timeseries" aria-label="Logical core time series">
          {#each coreLoads as core}
            <div class={`core-trend-card ${coreTone(core.load)}`}>
              <div>
                <span>Core {core.index + 1}</span>
                <strong>{formatPercent(core.load)}</strong>
              </div>
              <MiniChart values={core.trend} max={100} stroke={activeTheme.cpuStroke} fill={activeTheme.cpuFill} />
            </div>
          {/each}
        </div>
      {:else if detailMode === "memory"}
        <div class="detail-summary" aria-label="Memory summary">
          <div>
            <span>Used</span>
            <strong>{formatBytes(snapshot.system.memory_used_bytes)}</strong>
          </div>
          <div>
            <span>Total</span>
            <strong>{formatBytes(snapshot.system.memory_total_bytes)}</strong>
          </div>
          <div>
            <span>Swap</span>
            <strong>{formatPercent(swapPercent)}</strong>
          </div>
          <div>
            {#if snapshot.system.memory_available_bytes !== undefined}
              <span>Available</span>
              <strong>{formatBytes(snapshot.system.memory_available_bytes)}</strong>
            {:else}
              <span>Processes</span>
              <strong>{snapshot.system.process_count}</strong>
            {/if}
          </div>
        </div>
        <div class="detail-chart-grid two-up">
          <div class="detail-chart-card large">
            <div>
              <span>Memory load</span>
              <strong>{formatPercent(memoryPercent)}</strong>
            </div>
            <MiniChart values={history.memory} max={100} stroke={activeTheme.memoryStroke} fill={activeTheme.memoryFill} />
          </div>
          <div class="detail-chart-card large">
            <div>
              <span>Swap load</span>
              <strong>{formatPercent(swapPercent)}</strong>
            </div>
            <MiniChart values={history.swap} max={100} stroke={activeTheme.swapStroke} fill={activeTheme.swapFill} />
          </div>
        </div>
      {:else if detailMode === "disk"}
        <div class="detail-summary" aria-label="Disk summary">
          <div>
            <span>Read rate</span>
            <strong>{formatRate(diskReadRate)}</strong>
          </div>
          <div>
            <span>Write rate</span>
            <strong>{formatRate(diskWriteRate)}</strong>
          </div>
          <div>
            <span>Read total</span>
            <strong>{formatBytes(snapshot.system.disk_read_total_bytes)}</strong>
          </div>
          <div>
            <span>Write total</span>
            <strong>{formatBytes(snapshot.system.disk_write_total_bytes)}</strong>
          </div>
        </div>
        <div class="detail-chart-grid two-up">
          <div class="detail-chart-card large">
            <div>
              <span>Read throughput</span>
              <strong>{formatRate(diskReadRate)}</strong>
            </div>
            <MiniChart
              values={history.diskRead}
              max={diskScaleMax}
              stroke={activeTheme.diskReadStroke}
              fill={activeTheme.diskReadFill}
            />
          </div>
          <div class="detail-chart-card large">
            <div>
              <span>Write throughput</span>
              <strong>{formatRate(diskWriteRate)}</strong>
            </div>
            <MiniChart
              values={history.diskWrite}
              max={diskScaleMax}
              stroke={activeTheme.diskWriteStroke}
              fill={activeTheme.diskWriteFill}
            />
          </div>
        </div>
      {:else}
        <div class="detail-summary" aria-label="Network summary">
          <div>
            <span>Down</span>
            <strong>{formatRate(networkDownRate)}</strong>
          </div>
          <div>
            <span>Up</span>
            <strong>{formatRate(networkUpRate)}</strong>
          </div>
          <div>
            <span>Received</span>
            <strong>{formatBytes(snapshot.system.network_received_total_bytes)}</strong>
          </div>
          <div>
            <span>Sent</span>
            <strong>{formatBytes(snapshot.system.network_transmitted_total_bytes)}</strong>
          </div>
          <div>
            <span>Source</span>
            <strong>{metricQualityLabel(systemQuality.network, "Aggregate")}</strong>
          </div>
        </div>
        <div class="detail-chart-grid two-up">
          <div class="detail-chart-card large">
            <div>
              <span>Download rate</span>
              <strong>{formatRate(networkDownRate)}</strong>
            </div>
            <MiniChart
              values={history.netRx}
              max={networkScaleMax}
              stroke={activeTheme.networkDownStroke}
              fill={activeTheme.networkDownFill}
            />
          </div>
          <div class="detail-chart-card large">
            <div>
              <span>Upload rate</span>
              <strong>{formatRate(networkUpRate)}</strong>
            </div>
            <MiniChart
              values={history.netTx}
              max={networkScaleMax}
              stroke={activeTheme.networkUpStroke}
              fill={activeTheme.networkUpFill}
            />
          </div>
        </div>
      {/if}
    </section>

    <aside class="panel health-panel">
      <div class="panel-heading">
        <div>
          <p class="eyebrow">Runtime</p>
          <h2>Health</h2>
        </div>
        <strong>{snapshot.health.degraded ? "Degraded" : "Clean"}</strong>
      </div>
      <dl class="health-list">
        <div>
          <dt>Status</dt>
          <dd>{snapshot.health.status_summary}</dd>
        </div>
        <div>
          <dt>Source</dt>
          <dd>{sourceLabel}</dd>
        </div>
        <div>
          <dt>CPU quality</dt>
          <dd>{metricQualityLabel(systemQuality.cpu, "Legacy")}</dd>
        </div>
        <div>
          <dt>Disk quality</dt>
          <dd>{metricQualityLabel(systemQuality.disk, "Legacy")}</dd>
        </div>
        <div>
          <dt>Network quality</dt>
          <dd>{metricQualityLabel(systemQuality.network, "Aggregate")}</dd>
        </div>
        <div>
          <dt>App CPU</dt>
          <dd>{formatPercent(snapshot.health.app_cpu_percent)}</dd>
        </div>
        <div>
          <dt>App RSS</dt>
          <dd>{formatBytes(snapshot.health.app_rss_bytes)}</dd>
        </div>
        <div>
          <dt>Tick p95</dt>
          <dd>{snapshot.health.tick_p95_ms.toFixed(1)} ms</dd>
        </div>
        <div>
          <dt>Jitter p95</dt>
          <dd>{snapshot.health.jitter_p95_ms.toFixed(1)} ms</dd>
        </div>
        <div>
          <dt>Admin</dt>
          <dd>
            {snapshot.settings.admin_mode_enabled
              ? "Active"
              : snapshot.settings.admin_mode_requested
                ? "Requested (not active)"
                : "Off"}
          </dd>
        </div>
        <div>
          <dt>Memory load</dt>
          <dd>{formatPercent(memoryPercent)}</dd>
        </div>
        <div>
          <dt>Swap load</dt>
          <dd>{formatPercent(swapPercent)}</dd>
        </div>
        <div>
          <dt>Visible rows</dt>
          <dd>{filteredProcesses.length}</dd>
        </div>
      </dl>
      {#if commandError}
        <p class="command-error" role="status" aria-live="polite">{commandError}</p>
      {/if}
      {#if warnings.length}
        <ul class="warnings" aria-label="Collector warnings" aria-live="polite">
          {#each warnings.slice(0, 3) as warning}
            <li>{warning}</li>
          {/each}
        </ul>
      {:else}
        <p class="quiet-note">Collector is steady. Native telemetry is local-only and running without warnings.</p>
      {/if}
    </aside>
  </section>
</main>
