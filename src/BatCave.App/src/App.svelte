<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { onMount } from "svelte";
  import ContextRail from "./lib/components/context/ContextRail.svelte";
  import type { DetailMode, MetricCardOption } from "./lib/components/metrics/types";
  import MetricStrip from "./lib/components/metrics/MetricStrip.svelte";
  import ProcessExplorer from "./lib/components/processes/ProcessExplorer.svelte";
  import AppShell from "./lib/components/shell/AppShell.svelte";
  import HealthFooter from "./lib/components/shell/HealthFooter.svelte";
  import Toolbar from "./lib/components/shell/Toolbar.svelte";
  import TopBar from "./lib/components/shell/TopBar.svelte";
  import {
    accessLabel,
    formatBytes,
    formatPercent,
    formatRate,
    metricQualityLabel,
    processBytesLabel,
    processMemoryQuality,
  } from "./lib/format";
  import { makeFixtureSnapshot } from "./lib/fixtures";
  import {
    compareProcesses,
    defaultSortDirection,
    focusOptions,
    matchesFocusMode,
    matchesSearch,
    processColumns,
    processIoRate,
    sortColumnForKey,
    sortKeyForColumn,
    sortOptions,
    type FocusMode,
    type ProcessRates,
    type SortKey,
  } from "./lib/process";
  import {
    chartPalettes,
    parseThemePreference,
    resolveThemeName,
    themeOptions,
    themeStorageKey,
    type ThemeName,
    type ThemePreference,
  } from "./lib/themes";
  import type {
    KernelPoolTag,
    MetricQualityInfo,
    ProcessSample,
    RuntimeQuery,
    RuntimeSnapshot,
    SortDirection,
    TrendState,
  } from "./lib/types";

  interface ProcessTrendState {
    cpu: number[];
    memory: number[];
    readRate: number[];
    writeRate: number[];
  }

  const historyPointOptions = [30, 72, 180, 360] as const;
  type HistoryPointLimit = (typeof historyPointOptions)[number];

  const pollIntervals = [500, 1000, 2000] as const;
  const historyStorageKey = "batcave.monitor.history-points";

  let fixtureTick = 0;
  let snapshot: RuntimeSnapshot = makeEmptySnapshot();
  let selectedPid = "";
  let contextTab: "process" | "system" = "process";
  let pollState: "starting" | "native" | "fixture" | "error" = "starting";
  let lastError = "";
  let commandError = "";
  let copyStatus = "";
  let isPaused = false;
  let hasNativeSnapshot = false;
  let hasHydratedRuntimeSettings = false;
  let pollIntervalMs: (typeof pollIntervals)[number] = 1000;
  let searchText = "";
  let focusMode: FocusMode = "all";
  let sortKey: SortKey = "attention";
  let sortDirection: SortDirection = "desc";
  let detailMode: DetailMode = "cpu";
  let themePreference: ThemePreference = "system";
  let systemThemeName: ThemeName = "cave";
  let themeName: ThemeName = "cave";
  let historyPointLimit: HistoryPointLimit = 72;
  let history: TrendState = emptyTrendState();
  let processHistory: ProcessTrendState = emptyProcessTrendState();
  let processRates: Record<string, ProcessRates> = {};
  let processIcons: Record<string, string> = {};
  let requestedProcessIcons = new Set<string>();
  let metricCards: MetricCardOption[] = [];

  $: themeName = resolveThemeName(themePreference, systemThemeName);
  $: activeTheme = chartPalettes[themeName];
  $: memoryPercent = percentage(snapshot.system.memory_used_bytes, snapshot.system.memory_total_bytes);
  $: swapPercent = percentage(snapshot.system.swap_used_bytes, snapshot.system.swap_total_bytes);
  $: filteredProcesses = snapshot.processes
    .filter((process) => matchesSearch(process, searchText))
    .filter((process) => matchesFocusMode(process, focusMode, processRates))
    .slice()
    .sort((left, right) =>
      compareProcesses(left, right, sortKey, sortDirection, processRates, snapshot.system.memory_total_bytes),
    );
  $: selectedProcess = filteredProcesses.find((process) => process.pid === selectedPid) ?? null;
  $: warnings =
    snapshot.warnings.length > 0
      ? snapshot.warnings
          .slice()
          .reverse()
          .map((warning) => warning.message)
      : lastError
        ? [lastError]
        : [];
  $: sourceLabel =
    snapshot.source === "batcave_runtime" ||
    snapshot.source === "tauri_runtime" ||
    snapshot.source === "tauri_sysinfo"
      ? "native telemetry"
      : "fixture demo";
  $: systemQuality = snapshot.system.quality ?? {};
  $: memoryAccounting = snapshot.system.memory_accounting;
  $: topKernelPoolTags = topPoolTags(memoryAccounting?.kernel_pool_tags);
  $: blockedProcessCount =
    memoryAccounting?.denied_process_count ?? snapshot.processes.filter((process) => process.access_state === "denied").length;
  $: diskReadRate = history.diskRead.at(-1) ?? 0;
  $: diskWriteRate = history.diskWrite.at(-1) ?? 0;
  $: networkDownRate = history.netRx.at(-1) ?? 0;
  $: networkUpRate = history.netTx.at(-1) ?? 0;
  $: diskScaleMax = maxRate([...history.diskRead, ...history.diskWrite], 1_000_000);
  $: networkScaleMax = maxRate([...history.netRx, ...history.netTx], 750_000);
  $: selectedRates = selectedProcess ? processRates[selectedProcess.pid] : undefined;
  $: processReadRate = selectedRates?.readRate ?? processHistory.readRate.at(-1) ?? 0;
  $: processWriteRate = selectedRates?.writeRate ?? processHistory.writeRate.at(-1) ?? 0;
  $: void hydrateProcessIcons(filteredProcesses, selectedProcess);
  $: coreLoads = history.cores.map((core, index) => ({ index, load: currentCoreLoad(core), trend: core }));
  $: coreAverage = average(coreLoads.map((core) => core.load), snapshot.system.cpu_percent);
  $: corePeak = Math.max(...coreLoads.map((core) => core.load), 0);
  $: coreMinimum = coreLoads.length > 0 ? Math.min(...coreLoads.map((core) => core.load)) : 0;
  $: coreSpread = Math.max(0, corePeak - coreMinimum);
  $: hotCoreCount = coreLoads.filter((core) => core.load >= 75).length;
  $: busyCoreCount = coreLoads.filter((core) => core.load >= 45).length;
  $: detailTitle =
    detailMode === "cpu"
      ? "Logical cores"
      : detailMode === "memory"
        ? "Memory detail"
        : detailMode === "disk"
          ? "Disk throughput"
          : "Network throughput";
  $: detailReadout =
    detailMode === "cpu"
      ? `${formatPercent(coreAverage)} avg`
      : detailMode === "memory"
        ? `${formatPercent(memoryPercent)} used`
        : detailMode === "disk"
          ? formatRate(diskReadRate + diskWriteRate)
          : formatRate(networkDownRate + networkUpRate);
  $: metricCards = [
    {
      mode: "cpu",
      ariaLabel: "Open CPU logical core detail",
      label: "CPU",
      value: formatPercent(snapshot.system.cpu_percent),
      sublabel: "Logical cores",
      values: history.cpu,
      max: 100,
      stroke: activeTheme.cpuStroke,
      fill: activeTheme.cpuFill,
      contrastValue: snapshot.system.cpu_percent,
    },
    {
      mode: "memory",
      ariaLabel: "Open memory detail",
      label: "Memory",
      value: formatPercent(memoryPercent),
      sublabel: `${formatBytes(snapshot.system.memory_used_bytes)} used`,
      values: history.memory,
      max: 100,
      stroke: activeTheme.memoryStroke,
      fill: activeTheme.memoryFill,
      contrastValue: memoryPercent,
    },
    {
      mode: "disk",
      ariaLabel: "Open disk detail from read throughput",
      label: "Disk read",
      value: formatRate(diskReadRate),
      sublabel: metricQualityLabel(systemQuality.disk, "Aggregate"),
      values: history.diskRead,
      max: diskScaleMax,
      stroke: activeTheme.diskReadStroke,
      fill: activeTheme.diskReadFill,
      contrastValue: diskReadRate,
    },
    {
      mode: "disk",
      ariaLabel: "Open disk detail from write throughput",
      label: "Disk write",
      value: formatRate(diskWriteRate),
      sublabel: metricQualityLabel(systemQuality.disk, "Aggregate"),
      values: history.diskWrite,
      max: diskScaleMax,
      stroke: activeTheme.diskWriteStroke,
      fill: activeTheme.diskWriteFill,
      contrastValue: diskWriteRate,
    },
    {
      mode: "network",
      ariaLabel: "Open network detail from download throughput",
      label: "Network down",
      value: formatRate(networkDownRate),
      sublabel: metricQualityLabel(systemQuality.network, "Aggregate"),
      values: history.netRx,
      max: networkScaleMax,
      stroke: activeTheme.networkDownStroke,
      fill: activeTheme.networkDownFill,
      contrastValue: networkDownRate,
    },
    {
      mode: "network",
      ariaLabel: "Open network detail from upload throughput",
      label: "Network up",
      value: formatRate(networkUpRate),
      sublabel: metricQualityLabel(systemQuality.network, "Aggregate"),
      values: history.netTx,
      max: networkScaleMax,
      stroke: activeTheme.networkUpStroke,
      fill: activeTheme.networkUpFill,
      contrastValue: networkUpRate,
    },
  ];
  $: if (filteredProcesses.length > 0 && !filteredProcesses.some((process) => process.pid === selectedPid)) {
    selectProcess(filteredProcesses[0].pid);
  }

  onMount(() => {
    let timeoutId: number | undefined;
    let disposed = false;
    const systemThemeQuery = window.matchMedia("(prefers-color-scheme: light)");
    const savedTheme = window.localStorage.getItem(themeStorageKey);
    const savedHistoryPointLimit = Number(window.localStorage.getItem(historyStorageKey));

    systemThemeName = systemThemeQuery.matches ? "daylight" : "cave";

    const savedThemePreference = parseThemePreference(savedTheme);
    if (savedThemePreference) {
      themePreference = savedThemePreference;
    } else if (savedTheme !== null) {
      window.localStorage.removeItem(themeStorageKey);
    }

    if (isHistoryPointLimit(savedHistoryPointLimit)) {
      historyPointLimit = savedHistoryPointLimit;
    }

    if (!hasTauriRuntime()) {
      snapshot = makeFixtureSnapshot(fixtureTick);
      selectedPid = snapshot.processes[0]?.pid ?? "";
      ingest(snapshot);
    }

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

    const handleSystemThemeChange = (event: MediaQueryListEvent) => {
      systemThemeName = event.matches ? "daylight" : "cave";
    };

    systemThemeQuery.addEventListener("change", handleSystemThemeChange);

    return () => {
      disposed = true;
      systemThemeQuery.removeEventListener("change", handleSystemThemeChange);
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
      hasNativeSnapshot = true;
      return nativeSnapshot;
    } catch (error) {
      pollState = "error";
      lastError = commandErrorMessage(error, "Native telemetry is unavailable.");
      return hasNativeSnapshot ? snapshot : makeEmptySnapshot(lastError);
    }
  }

  function hasTauriRuntime(): boolean {
    return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
  }

  function makeEmptySnapshot(statusSummary = "Waiting for native telemetry."): RuntimeSnapshot {
    const now = Date.now();

    return {
      event_kind: "runtime_snapshot",
      seq: 0,
      ts_ms: now,
      source: "tauri_runtime",
      settings: {
        query: {
          filter_text: "",
          sort_column: "attention",
          sort_direction: "desc",
          limit: 5000,
        },
        admin_mode_requested: false,
        admin_mode_enabled: false,
        metric_window_seconds: 60,
        paused: false,
      },
      health: {
        tick_count: 0,
        snapshot_latency_ms: 0,
        degraded: true,
        collector_warnings: statusSummary ? 1 : 0,
        runtime_loop_enabled: true,
        runtime_loop_running: false,
        status_summary: statusSummary,
        updated_at_ms: now,
        tick_p95_ms: 0,
        sort_p95_ms: 0,
        jitter_p95_ms: 0,
        dropped_ticks: 0,
        app_cpu_percent: 0,
        app_rss_bytes: 0,
        last_warning: statusSummary,
      },
      system: {
        cpu_percent: 0,
        kernel_cpu_percent: 0,
        logical_cpu_percent: [],
        memory_used_bytes: 0,
        memory_total_bytes: 0,
        memory_available_bytes: 0,
        swap_used_bytes: 0,
        swap_total_bytes: 0,
        process_count: 0,
        disk_read_total_bytes: 0,
        disk_write_total_bytes: 0,
        disk_read_bps: 0,
        disk_write_bps: 0,
        network_received_total_bytes: 0,
        network_transmitted_total_bytes: 0,
        network_received_bps: 0,
        network_transmitted_bps: 0,
        quality: {
          cpu: { quality: "unavailable", source: "runtime", message: statusSummary },
          kernel_cpu: { quality: "unavailable", source: "runtime", message: statusSummary },
          logical_cpu: { quality: "unavailable", source: "runtime", message: statusSummary },
          memory: { quality: "unavailable", source: "runtime", message: statusSummary },
          swap: { quality: "unavailable", source: "runtime", message: statusSummary },
          disk: { quality: "unavailable", source: "runtime", message: statusSummary },
          network: { quality: "unavailable", source: "runtime", message: statusSummary },
        },
      },
      processes: [],
      total_process_count: 0,
      warnings: [],
    };
  }

  function isHistoryPointLimit(value: number): value is HistoryPointLimit {
    return historyPointOptions.some((option) => option === value);
  }

  function setTheme(preference: ThemePreference): void {
    themePreference = preference;
    window.localStorage.setItem(themeStorageKey, preference);
  }

  function setHistoryPointLimit(limit: number): void {
    if (!isHistoryPointLimit(limit)) {
      return;
    }

    historyPointLimit = limit;
    window.localStorage.setItem(historyStorageKey, String(limit));
    trimHistory();
  }

  async function setPaused(nextPaused: boolean): Promise<void> {
    const previousPaused = isPaused;
    isPaused = nextPaused;
    if (!hasTauriRuntime()) {
      return;
    }

    try {
      const next = await invoke<RuntimeSnapshot>(nextPaused ? "pause_runtime" : "resume_runtime");
      applyNativeSnapshot(next);
    } catch (error) {
      isPaused = previousPaused;
      commandError = commandErrorMessage(error, "Unable to change runtime pause state.");
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
      applyNativeSnapshot(next);
    } catch (error) {
      commandError = commandErrorMessage(error, "Unable to refresh runtime.");
    }
  }

  async function setAdminMode(enabled: boolean): Promise<void> {
    if (!hasTauriRuntime()) {
      return;
    }

    try {
      const next = await invoke<RuntimeSnapshot>("set_admin_mode", { enabled });
      applyNativeSnapshot(next);
    } catch (error) {
      commandError = commandErrorMessage(error, "Unable to change admin mode.");
    }
  }

  function setSortKey(key: SortKey): void {
    sortKey = key;
    sortDirection = defaultSortDirection(key);
    void syncRuntimeQuery();
  }

  function toggleSortKey(key: SortKey): void {
    if (sortKey === key) {
      sortDirection = sortDirection === "asc" ? "desc" : "asc";
    } else {
      sortKey = key;
      sortDirection = defaultSortDirection(key);
    }

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
      sort_direction: sortDirection,
      limit: 5000,
    };

    try {
      const next = await invoke<RuntimeSnapshot>("set_process_query", { query });
      applyNativeSnapshot(next);
    } catch (error) {
      commandError = commandErrorMessage(error, "Unable to update runtime query.");
    }
  }

  function applyNativeSnapshot(next: RuntimeSnapshot): void {
    pollState = "native";
    lastError = "";
    commandError = "";
    copyStatus = "";
    hasNativeSnapshot = true;
    ingest(next);
  }

  function commandErrorMessage(error: unknown, fallback: string): string {
    if (error instanceof Error && error.message.trim()) {
      return error.message;
    }

    if (typeof error === "string" && error.trim()) {
      return error;
    }

    if (error && typeof error === "object") {
      try {
        const serialized = JSON.stringify(error);
        if (serialized) {
          return serialized;
        }
      } catch {
        return fallback;
      }
    }

    return fallback;
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
    if (next.source === "fixture" || hasHydratedRuntimeSettings || (!hasNativeSnapshot && pollState === "error")) {
      return;
    }

    searchText = next.settings.query.filter_text;
    sortKey = sortKeyForColumn(next.settings.query.sort_column);
    sortDirection = next.settings.query.sort_direction;
    isPaused = next.settings.paused;
    hasHydratedRuntimeSettings = true;
  }

  function selectProcess(pid: string): void {
    selectedPid = pid;
    copyStatus = "";
    const process = snapshot.processes.find((candidate) => candidate.pid === pid);
    if (process) {
      resetProcessHistory(process);
    }
  }

  async function hydrateProcessIcons(processes: ProcessSample[], selected: ProcessSample | null): Promise<void> {
    if (!hasNativeSnapshot) {
      return;
    }

    const iconCandidates = selected ? [selected, ...processes.slice(0, 80)] : processes.slice(0, 80);
    for (const process of iconCandidates) {
      const key = processIconKey(process);
      if (!process.exe || processIcons[key] || requestedProcessIcons.has(key)) {
        continue;
      }

      requestedProcessIcons.add(key);
      try {
        const icon = await invoke<string | null>("get_process_icon", { exe: process.exe });
        if (icon) {
          processIcons = { ...processIcons, [key]: icon };
        }
      } catch {
        // Native icon lookup is cosmetic; keep the category fallback when Windows denies it.
      }
    }
  }

  function processIconKey(process: ProcessSample): string {
    return process.exe || process.name;
  }

  function selectDetailMode(mode: DetailMode): void {
    detailMode = mode;
    contextTab = "system";
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
    history = emptyTrendState();
    if (selectedProcess) {
      resetProcessHistory(selectedProcess);
    }
  }

  function emptyTrendState(): TrendState {
    return {
      cpu: [],
      memory: [],
      swap: [],
      diskRead: [],
      diskWrite: [],
      netRx: [],
      netTx: [],
      cores: [],
    };
  }

  function emptyProcessTrendState(): ProcessTrendState {
    return {
      cpu: [],
      memory: [],
      readRate: [],
      writeRate: [],
    };
  }

  function trimHistory(): void {
    history = {
      cpu: trimPoints(history.cpu),
      memory: trimPoints(history.memory),
      swap: trimPoints(history.swap),
      diskRead: trimPoints(history.diskRead),
      diskWrite: trimPoints(history.diskWrite),
      netRx: trimPoints(history.netRx),
      netTx: trimPoints(history.netTx),
      cores: history.cores.map(trimPoints),
    };
    processHistory = {
      cpu: trimPoints(processHistory.cpu),
      memory: trimPoints(processHistory.memory),
      readRate: trimPoints(processHistory.readRate),
      writeRate: trimPoints(processHistory.writeRate),
    };
  }

  function trimPoints(points: number[]): number[] {
    return points.slice(-historyPointLimit);
  }

  function pushPoint(points: number[], value: number): number[] {
    return trimPoints([...points, Number.isFinite(value) ? value : 0]);
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

  function topPoolTags(tags: KernelPoolTag[] | undefined): KernelPoolTag[] {
    return [...(tags ?? [])].sort((left, right) => right.bytes - left.bytes).slice(0, 8);
  }

  function adminStatusLabel(): string {
    if (snapshot.settings.admin_mode_enabled) {
      return blockedProcessCount > 0 ? `Active, ${blockedProcessCount} blocked` : "Active";
    }

    return snapshot.settings.admin_mode_requested ? "Requested (not active)" : "Off";
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

  function processSummary(process: ProcessSample): string {
    return [
      "BatCave process snapshot",
      `Name: ${process.name}`,
      `PID: ${process.pid}`,
      `Parent PID: ${process.parent_pid ?? "--"}`,
      `Status: ${process.status}`,
      `CPU: ${formatPercent(process.cpu_percent)}`,
      `Working set: ${processBytesLabel(process, process.memory_bytes)}`,
      `Private: ${processBytesLabel(process, process.private_bytes)}`,
      `I/O rate: ${formatRate(processIoRate(process, processRates))}`,
      `Network: ${processNetworkLabel(process)}`,
      `Access: ${accessLabel(process.access_state)}`,
      `Memory quality: ${metricQualityLabel(processMemoryQuality(process) as MetricQualityInfo | undefined, "Measured")}`,
      `Path: ${process.exe || "Path unavailable"}`,
      `Snapshot seq: ${snapshot.seq}`,
      `Snapshot source: ${snapshot.source}`,
    ].join("\n");
  }

  async function copySelectedProcessSummary(): Promise<void> {
    if (!selectedProcess) {
      copyStatus = "No selected process to copy.";
      return;
    }

    try {
      await navigator.clipboard.writeText(processSummary(selectedProcess));
      copyStatus = "Process summary copied.";
      commandError = "";
    } catch (error) {
      copyStatus = "";
      commandError = commandErrorMessage(error, "Unable to copy process summary.");
    }
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

<AppShell {themeName}>
  <TopBar
    {isPaused}
    {pollState}
    processCount={snapshot.system.process_count}
    latencyMs={snapshot.health.snapshot_latency_ms}
  />
  <MetricStrip
    cards={metricCards}
    activeMode={detailMode}
    {needsReadoutContrast}
    onSelect={selectDetailMode}
  />
  <Toolbar
    {searchText}
    {focusMode}
    {sortKey}
    {isPaused}
    {commandError}
    {focusOptions}
    {sortOptions}
    {themeOptions}
    {themePreference}
    {pollIntervals}
    {pollIntervalMs}
    {historyPointOptions}
    {historyPointLimit}
    adminRequested={snapshot.settings.admin_mode_requested}
    adminEnabled={snapshot.settings.admin_mode_enabled}
    onSearch={setSearchText}
    onFocus={(mode) => (focusMode = mode)}
    onSort={setSortKey}
    onPaused={() => void setPaused(!isPaused)}
    onRefresh={() => void refreshNow()}
    onTheme={setTheme}
    onPollInterval={(interval) => (pollIntervalMs = interval as (typeof pollIntervals)[number])}
    onHistoryLimit={setHistoryPointLimit}
    onAdminMode={(enabled) => void setAdminMode(enabled)}
    onResetHistory={resetHistory}
  />
  <section class="workspace-grid">
    <ProcessExplorer
      processes={filteredProcesses}
      columns={processColumns}
      {selectedPid}
      {sortKey}
      {sortDirection}
      {processRates}
      {processIcons}
      onSelect={selectProcess}
      onToggleSort={toggleSortKey}
    />
    <ContextRail
      activeTab={contextTab}
      onTab={(tab) => (contextTab = tab)}
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
      onCopy={() => void copySelectedProcessSummary()}
      {detailMode}
      {detailTitle}
      {detailReadout}
      {snapshot}
      {history}
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
  </section>
  <HealthFooter
    {snapshot}
    {sourceLabel}
    {systemQuality}
    {memoryPercent}
    {swapPercent}
    visibleRows={filteredProcesses.length}
    {warnings}
    {pollState}
    {lastError}
    adminStatus={adminStatusLabel()}
  />
</AppShell>
