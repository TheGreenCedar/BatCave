<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { onMount } from "svelte";
  import MiniChart from "./lib/MiniChart.svelte";
  import { makeFixtureSnapshot } from "./lib/fixtures";
  import type {
    MetricQuality,
    MetricQualityInfo,
    MetricSource,
    ProcessSample,
    RuntimeQuery,
    RuntimeSnapshot,
    SortColumn,
    SortDirection,
    TrendState,
  } from "./lib/types";

  type FocusMode = "all" | "active" | "io";
  type SortKey = "name" | "pid" | "cpu" | "memory" | "io" | "read" | "write" | "status";
  type DetailMode = "cpu" | "memory" | "disk" | "network";
  type ThemeName = "cave" | "aurora" | "ember" | "daylight";
  type ThemePreference = "system" | ThemeName;

  interface ThemeOption {
    name: ThemePreference;
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

  interface MetricCardOption {
    mode: DetailMode;
    ariaLabel: string;
    label: string;
    value: string;
    sublabel: string;
    values: number[];
    max: number;
    stroke: string;
    fill: string;
    contrastValue: number;
  }

  interface ProcessColumn {
    key: SortKey;
    label: string;
    metric?: boolean;
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

  const historyPointOptions = [30, 72, 180, 360] as const;
  type HistoryPointLimit = (typeof historyPointOptions)[number];

  const pollIntervals = [500, 1000, 2000] as const;
  const themeStorageKey = "batcave.monitor.theme";
  const historyStorageKey = "batcave.monitor.history-points";
  const themeOptions: ThemeOption[] = [
    { name: "system", label: "System" },
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
  const focusOptions: { value: FocusMode; label: string }[] = [
    { value: "all", label: "All" },
    { value: "active", label: "Active" },
    { value: "io", label: "I/O" },
  ];
  const sortOptions: { value: SortKey; label: string }[] = [
    { value: "cpu", label: "CPU" },
    { value: "memory", label: "Memory" },
    { value: "io", label: "I/O" },
    { value: "name", label: "Name" },
  ];
  const sortColumnByKey: Record<SortKey, SortColumn> = {
    name: "name",
    pid: "pid",
    cpu: "cpu_pct",
    memory: "memory_bytes",
    io: "disk_bps",
    read: "disk_bps",
    write: "disk_bps",
    status: "name",
  };
  const sortKeyByColumn: Partial<Record<SortColumn, SortKey>> = {
    cpu_pct: "cpu",
    memory_bytes: "memory",
    disk_bps: "io",
    name: "name",
    pid: "pid",
  };
  const processColumns: ProcessColumn[] = [
    { key: "name", label: "Process" },
    { key: "pid", label: "PID" },
    { key: "cpu", label: "CPU", metric: true },
    { key: "memory", label: "Memory", metric: true },
    { key: "io", label: "I/O rate", metric: true },
    { key: "read", label: "Read total", metric: true },
    { key: "write", label: "Write total", metric: true },
    { key: "status", label: "Status" },
  ];

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
  let sortDirection: SortDirection = "desc";
  let detailMode: DetailMode = "cpu";
  let themePreference: ThemePreference = "system";
  let systemThemeName: ThemeName = "cave";
  let themeName: ThemeName = "cave";
  let historyPointLimit: HistoryPointLimit = 72;
  let history: TrendState = emptyTrendState();
  let processHistory: ProcessTrendState = emptyProcessTrendState();
  let processRates: Record<string, ProcessRates> = {};
  let metricCards: MetricCardOption[] = [];

  $: themeName = resolveThemeName(themePreference, systemThemeName);
  $: activeTheme = chartPalettes[themeName];
  $: memoryPercent = percentage(snapshot.system.memory_used_bytes, snapshot.system.memory_total_bytes);
  $: swapPercent = percentage(snapshot.system.swap_used_bytes, snapshot.system.swap_total_bytes);
  $: filteredProcesses = snapshot.processes
    .filter((process) => matchesSearch(process, searchText))
    .filter((process) => matchesFocusMode(process, focusMode))
    .slice()
    .sort((left, right) => compareProcesses(left, right, sortKey, sortDirection));
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
    return value === "cave" || value === "aurora" || value === "ember" || value === "daylight";
  }

  function parseThemePreference(value: string | null): ThemePreference | null {
    if (value === "system" || value === "auto") {
      return "system";
    }

    return isThemeName(value) ? value : null;
  }

  function resolveThemeName(preference: ThemePreference, systemTheme: ThemeName): ThemeName {
    return preference === "system" ? systemTheme : preference;
  }

  function isHistoryPointLimit(value: number): value is HistoryPointLimit {
    return historyPointOptions.some((option) => option === value);
  }

  function setTheme(preference: ThemePreference): void {
    themePreference = preference;
    window.localStorage.setItem(themeStorageKey, preference);
  }

  function setHistoryPointLimit(limit: HistoryPointLimit): void {
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
    ingest(next);
  }

  function commandErrorMessage(error: unknown, fallback: string): string {
    return error instanceof Error ? error.message : fallback;
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
    sortDirection = next.settings.query.sort_direction;
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

  function selectProcessFromKeyboard(event: KeyboardEvent, pid: string): void {
    if (event.key !== "Enter" && event.key !== " ") {
      return;
    }

    event.preventDefault();
    selectProcess(pid);
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

  function processIoRate(process: ProcessSample): number {
    const rates = processRates[process.pid];
    return (rates?.readRate ?? 0) + (rates?.writeRate ?? 0) + (rates?.otherRate ?? 0);
  }

  function compareProcesses(
    left: ProcessSample,
    right: ProcessSample,
    key: SortKey,
    direction: SortDirection,
  ): number {
    const factor = direction === "asc" ? 1 : -1;

    switch (key) {
      case "name":
        return compareText(left.name, right.name) * factor || compareText(left.pid, right.pid) * factor;
      case "pid":
        return comparePid(left.pid, right.pid) * factor || compareText(left.name, right.name) * factor;
      case "memory":
        return compareNumber(left.memory_bytes, right.memory_bytes, direction) || compareNumber(left.cpu_percent, right.cpu_percent, "desc");
      case "io":
        return compareNumber(processIoRate(left), processIoRate(right), direction) || compareNumber(left.cpu_percent, right.cpu_percent, "desc");
      case "read":
        return compareNumber(left.disk_read_total_bytes, right.disk_read_total_bytes, direction) || compareText(left.name, right.name);
      case "write":
        return compareNumber(left.disk_write_total_bytes, right.disk_write_total_bytes, direction) || compareText(left.name, right.name);
      case "status":
        return compareText(left.status, right.status) * factor || compareText(left.name, right.name);
      case "cpu":
      default:
        return compareNumber(left.cpu_percent, right.cpu_percent, direction) || compareNumber(left.memory_bytes, right.memory_bytes, "desc");
    }
  }

  function compareNumber(left: number, right: number, direction: SortDirection): number {
    return direction === "asc" ? left - right : right - left;
  }

  function compareText(left: string, right: string): number {
    return left.localeCompare(right, undefined, { sensitivity: "base", numeric: true });
  }

  function comparePid(left: string, right: string): number {
    const leftNumber = Number(left);
    const rightNumber = Number(right);

    if (Number.isFinite(leftNumber) && Number.isFinite(rightNumber)) {
      return leftNumber - rightNumber;
    }

    return compareText(left, right);
  }

  function defaultSortDirection(key: SortKey): SortDirection {
    return key === "name" || key === "pid" || key === "status" ? "asc" : "desc";
  }

  function sortColumnForKey(key: SortKey): SortColumn {
    return sortColumnByKey[key];
  }

  function sortKeyForColumn(column: SortColumn): SortKey {
    return sortKeyByColumn[column] ?? "cpu";
  }

  function sortAriaValue(
    key: SortKey,
    activeKey: SortKey,
    direction: SortDirection,
  ): "ascending" | "descending" | "none" {
    if (activeKey !== key) {
      return "none";
    }

    return direction === "asc" ? "ascending" : "descending";
  }

  function sortButtonLabel(
    column: ProcessColumn,
    activeKey: SortKey,
    direction: SortDirection,
  ): string {
    if (activeKey === column.key) {
      const nextDirection = direction === "asc" ? "descending" : "ascending";
      return `${column.label}, sorted ${direction === "asc" ? "ascending" : "descending"}. Sort ${nextDirection}.`;
    }

    const defaultDirection = defaultSortDirection(column.key) === "asc" ? "ascending" : "descending";
    return `Sort by ${column.label} ${defaultDirection}.`;
  }

  function sortIndicator(key: SortKey, activeKey: SortKey, direction: SortDirection): string {
    if (activeKey !== key) {
      return "";
    }

    return direction === "asc" ? "Asc" : "Desc";
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

  function formatMetricQuality(value: MetricQuality): string {
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

  function formatMetricSource(value: MetricSource): string {
    switch (value) {
      case "direct_api":
        return "direct API";
      case "interface_aggregate":
        return "interface aggregate";
      case "process_aggregate":
        return "process aggregate";
      case "ebpf":
        return "eBPF";
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
            class:active={themePreference === theme.name}
            type="button"
            aria-pressed={themePreference === theme.name}
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
    {#each metricCards as card (card.ariaLabel)}
      <button
        class="metric-card"
        class:active={detailMode === card.mode}
        type="button"
        aria-controls="resource-detail-panel"
        aria-label={card.ariaLabel}
        onclick={() => setDetailMode(card.mode)}
      >
        <MiniChart values={card.values} max={card.max} stroke={card.stroke} fill={card.fill} />
        <span class="metric-copy" class:on-fill={needsReadoutContrast(card.contrastValue, card.max)}>
          <span>{card.label}</span>
          <strong>{card.value}</strong>
          <small>{card.sublabel}</small>
        </span>
      </button>
    {/each}
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
              {#each processColumns as column}
                <th aria-sort={sortAriaValue(column.key, sortKey, sortDirection)} class:metric={column.metric}>
                  <button
                    class="sort-header"
                    class:active={sortKey === column.key}
                    type="button"
                    aria-label={sortButtonLabel(column, sortKey, sortDirection)}
                    aria-pressed={sortKey === column.key}
                    onclick={() => toggleSortKey(column.key)}
                  >
                    <span>{column.label}</span>
                    <small aria-hidden="true">{sortIndicator(column.key, sortKey, sortDirection)}</small>
                  </button>
                </th>
              {/each}
            </tr>
          </thead>
          <tbody>
            {#each filteredProcesses as process}
              <tr
                class:selected={process.pid === selectedPid}
                tabindex="0"
                aria-selected={process.pid === selectedPid}
                aria-label={`Select ${process.name}, PID ${process.pid}`}
                onclick={() => selectProcess(process.pid)}
                onkeydown={(event) => selectProcessFromKeyboard(event, process.pid)}
              >
                <td>
                  <span class="process-button" class:selected={process.pid === selectedPid}>
                    <span>{process.name}</span>
                    <small>{processHint(process)}</small>
                  </span>
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
        {#each focusOptions as option}
          <button
            class:active={focusMode === option.value}
            type="button"
            aria-pressed={focusMode === option.value}
            onclick={() => (focusMode = option.value)}
          >
            {option.label}
          </button>
        {/each}
      </div>
    </div>
    <div class="control-group sort-group">
      <span>Sort</span>
      <div class="segmented" role="group" aria-label="Process sort">
        {#each sortOptions as option}
          <button
            class:active={sortKey === option.value}
            type="button"
            aria-pressed={sortKey === option.value}
            onclick={() => setSortKey(option.value)}
          >
            {option.label}
          </button>
        {/each}
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
    <div class="control-group history-group">
      <span>History</span>
      <div class="segmented" role="group" aria-label="Chart history length">
        {#each historyPointOptions as option}
          <button
            class:active={historyPointLimit === option}
            type="button"
            aria-pressed={historyPointLimit === option}
            aria-label={`Show last ${option} chart samples`}
            onclick={() => setHistoryPointLimit(option)}
          >
            {option}
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
          <h2>{detailTitle}</h2>
        </div>
        <strong>{detailReadout}</strong>
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
