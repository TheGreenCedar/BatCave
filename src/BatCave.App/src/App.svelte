<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { onMount } from "svelte";
  import DetailPane from "./lib/components/context/DetailPane.svelte";
  import SystemSummary from "./lib/components/metrics/SystemSummary.svelte";
  import type { DetailMode, ResourceSummaryOption } from "./lib/components/metrics/types";
  import AttentionQueue from "./lib/components/processes/AttentionQueue.svelte";
  import AppHeader from "./lib/components/shell/AppHeader.svelte";
  import AppShell from "./lib/components/shell/AppShell.svelte";
  import HealthStatus from "./lib/components/shell/HealthStatus.svelte";
  import ProcessCommandBar from "./lib/components/shell/ProcessCommandBar.svelte";
  import SettingsDrawer from "./lib/components/shell/SettingsDrawer.svelte";
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
    defaultSortDirection,
    focusOptions,
    hasSameProcessOrder,
    processColumns,
    processIoRate,
    sortColumnForKey,
    sortKeyForColumn,
    sortOptions,
    stabilizeProcessRows,
    type FocusMode,
    type ProcessRates,
    type SortKey,
  } from "./lib/process";
  import { makeDefaultRuntimeQuery, makeEmptySnapshot } from "./lib/runtimeSnapshot";
  import {
    chartPalettes,
    parseThemePreference,
    resolveThemeName,
    themeOptions,
    themeStorageKey,
    type ThemeName,
    type ThemePreference,
  } from "./lib/themes";
  import {
    commandErrorMessage,
    getRuntimeProcessIcon,
    readNativeSnapshot,
    refreshRuntime,
    setRuntimeAdminMode,
    setRuntimePaused,
    setRuntimeProcessQuery,
  } from "./lib/tauriBridge";
  import type {
    KernelPoolTag,
    MetricQualityInfo,
    ProcessSample,
    ProcessViewRow,
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
    networkRate: number[];
  }

  const historyPointOptions = [30, 72, 180, 360] as const;
  type HistoryPointLimit = (typeof historyPointOptions)[number];

  const pollIntervals = [500, 1000, 2000] as const;
  const historyStorageKey = "batcave.monitor.history-points";

  let fixtureTick = 0;
  let snapshot: RuntimeSnapshot = makeEmptySnapshot();
  let selectedPid = "";
  let detailSubject: "process" | "system" = "system";
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
  let resourceSummaries: ResourceSummaryOption[] = [];
  let displayProcessRows: ProcessViewRow[] = [];
  let pendingProcessRows: ProcessViewRow[] = [];
  let queueInteracting = false;
  let expandedGroupCount = 0;
  let rankingUpdateAvailable = false;
  let settingsOpen = false;
  let diagnosticsOpen = false;
  let isCompactDetail = false;
  let compactDetailOpen = false;
  let detailReturnFocus: HTMLElement | null = null;
  let healthTone: "healthy" | "warning" | "danger" = "healthy";
  let forceRankingRefresh = false;
  let runtimeQueryRequestSeq = 0;

  $: themeName = resolveThemeName(themePreference, systemThemeName);
  $: activeTheme = chartPalettes[themeName];
  $: memoryPercent = percentage(snapshot.system.memory_used_bytes, snapshot.system.memory_total_bytes);
  $: swapPercent = percentage(snapshot.system.swap_used_bytes, snapshot.system.swap_total_bytes);
  $: processViewRows = displayProcessRows;
  $: filteredProcesses = processViewRows.flatMap((row) => (row.process ? [row.process] : []));
  $: selectedGroupRow = selectedGroupKey(selectedPid)
    ? (processViewRows.find((row) => row.kind === "group" && row.group_key === selectedGroupKey(selectedPid)) ?? null)
    : null;
  $: selectedProcess = selectedGroupRow ? groupProcessFromRow(selectedGroupRow) : (filteredProcesses.find((process) => process.pid === selectedPid) ?? null);
  $: sourceLabel =
    snapshot.source === "batcave_runtime" ||
    snapshot.source === "tauri_runtime" ||
    snapshot.source === "tauri_sysinfo"
      ? "native telemetry"
      : "fixture demo";
  $: systemQuality = snapshot.system.quality ?? {};
  $: processNetworkAvailable = processViewRows.some((row) => {
    const quality = (row.process ?? row.representative)?.quality?.network?.quality;
    return quality !== undefined && quality !== "unavailable" && quality !== "held";
  });
  $: visibleProcessColumns =
    processNetworkAvailable ? processColumns : processColumns.filter((column) => column.key !== "network");
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
  $: void hydrateProcessIcons(processViewRows, filteredProcesses, selectedProcess);
  $: coreLoads = history.cores.map((core, index) => ({ index, load: currentCoreLoad(core), trend: core }));
  $: coreAverage = average(coreLoads.map((core) => core.load), snapshot.system.cpu_percent);
  $: corePeak = Math.max(...coreLoads.map((core) => core.load), 0);
  $: coreMinimum = coreLoads.length > 0 ? Math.min(...coreLoads.map((core) => core.load)) : 0;
  $: coreSpread = Math.max(0, corePeak - coreMinimum);
  $: hotCoreCount = coreLoads.filter((core) => core.load >= 75).length;
  $: busyCoreCount = coreLoads.filter((core) => core.load >= 45).length;
  $: topAttentionRow = processViewRows.find((row) => row.kind === "group" || !row.is_grouped);
  $: topContributorLabel = topAttentionRow?.group_label ?? topAttentionRow?.process?.name ?? "background activity";
  $: systemHeadline = systemPressureHeadline(
    snapshot.system.cpu_percent,
    memoryPercent,
    diskReadRate + diskWriteRate,
    networkDownRate + networkUpRate,
    topContributorLabel,
  );
  $: systemSupportingText = snapshot.health.degraded
    ? `${snapshot.warnings.length || snapshot.health.collector_warnings} telemetry limitation${(snapshot.warnings.length || snapshot.health.collector_warnings) === 1 ? "" : "s"}; available values remain live.`
    : "Local telemetry is current. Select a resource or workload to inspect it.";
  $: healthTone = pollState === "error" ? "danger" : snapshot.health.degraded ? "warning" : "healthy";
  $: healthLabel = pollState === "error"
    ? "Telemetry stale"
    : snapshot.health.degraded
      ? `${snapshot.warnings.length || snapshot.health.collector_warnings} limitations`
      : "Telemetry healthy";
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
  $: resourceSummaries = [
    {
      mode: "cpu",
      ariaLabel: "Open CPU logical core detail",
      label: "CPU",
      value: formatPercent(snapshot.system.cpu_percent),
      supportingLabel: "Peak core",
      supportingValue: formatPercent(corePeak),
      statusLabel: resourceStatusLabel(snapshot.system.cpu_percent),
      values: history.cpu,
      max: 100,
      stroke: activeTheme.cpuStroke,
      fill: activeTheme.cpuFill,
    },
    {
      mode: "memory",
      ariaLabel: "Open memory detail",
      label: "Memory",
      value: formatPercent(memoryPercent),
      supportingLabel: "Used",
      supportingValue: formatBytes(snapshot.system.memory_used_bytes),
      statusLabel: resourceStatusLabel(memoryPercent),
      values: history.memory,
      max: 100,
      stroke: activeTheme.memoryStroke,
      fill: activeTheme.memoryFill,
    },
    {
      mode: "disk",
      ariaLabel: "Open disk throughput detail",
      label: "Disk",
      value: formatRate(diskReadRate + diskWriteRate),
      supportingLabel: "Read / write",
      supportingValue: `${formatRate(diskReadRate)} / ${formatRate(diskWriteRate)}`,
      statusLabel: metricQualityLabel(systemQuality.disk, "Aggregate"),
      values: history.diskWrite,
      max: diskScaleMax,
      stroke: activeTheme.diskWriteStroke,
      fill: activeTheme.diskWriteFill,
    },
    {
      mode: "network",
      ariaLabel: "Open network throughput detail",
      label: "Network",
      value: formatRate(networkDownRate + networkUpRate),
      supportingLabel: "Down / up",
      supportingValue: `${formatRate(networkDownRate)} / ${formatRate(networkUpRate)}`,
      statusLabel: metricQualityLabel(systemQuality.network, "Aggregate"),
      values: history.netRx,
      max: networkScaleMax,
      stroke: activeTheme.networkDownStroke,
      fill: activeTheme.networkDownFill,
    },
  ];

  onMount(() => {
    let timeoutId: number | undefined;
    let disposed = false;
    const systemThemeQuery = window.matchMedia("(prefers-color-scheme: light)");
    const compactDetailQuery = window.matchMedia("(max-width: 1120px)");
    const savedTheme = window.localStorage.getItem(themeStorageKey);
    const savedHistoryPointLimit = Number(window.localStorage.getItem(historyStorageKey));

    systemThemeName = systemThemeQuery.matches ? "daylight" : "cave";
    isCompactDetail = compactDetailQuery.matches;

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
      ingest(makeFixtureSnapshot(fixtureTick, currentRuntimeQuery()));
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

    const handleCompactDetailChange = (event: MediaQueryListEvent) => {
      isCompactDetail = event.matches;
      compactDetailOpen = false;
    };

    systemThemeQuery.addEventListener("change", handleSystemThemeChange);
    compactDetailQuery.addEventListener("change", handleCompactDetailChange);

    return () => {
      disposed = true;
      systemThemeQuery.removeEventListener("change", handleSystemThemeChange);
      compactDetailQuery.removeEventListener("change", handleCompactDetailChange);
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
      return makeFixtureSnapshot(fixtureTick, currentRuntimeQuery());
    }

    const next = await readNativeSnapshot(invoke, {
      currentSnapshot: snapshot,
      emptySnapshot: makeEmptySnapshot,
      hasNativeSnapshot,
    });
    pollState = next.ok ? "native" : "error";
    lastError = next.error;
    hasNativeSnapshot = next.ok || hasNativeSnapshot;
    return next.snapshot;
  }

  function hasTauriRuntime(): boolean {
    return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
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
      const next = await setRuntimePaused(invoke, nextPaused);
      applyNativeSnapshot(next);
    } catch (error) {
      isPaused = previousPaused;
      commandError = commandErrorMessage(error, "Unable to change runtime pause state.");
    }
  }

  async function refreshNow(): Promise<void> {
    if (!hasTauriRuntime()) {
      fixtureTick += 1;
      ingest(makeFixtureSnapshot(fixtureTick, currentRuntimeQuery()));
      return;
    }

    try {
      const next = await refreshRuntime(invoke);
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
      const next = await setRuntimeAdminMode(invoke, enabled);
      applyNativeSnapshot(next);
    } catch (error) {
      commandError = commandErrorMessage(error, "Unable to change admin mode.");
    }
  }

  function setSortKey(key: SortKey): void {
    sortKey = key;
    sortDirection = defaultSortDirection(key);
    forceRankingRefresh = true;
    void syncRuntimeQuery();
  }

  function toggleSortKey(key: SortKey): void {
    if (sortKey === key) {
      sortDirection = sortDirection === "asc" ? "desc" : "asc";
    } else {
      sortKey = key;
      sortDirection = defaultSortDirection(key);
    }

    forceRankingRefresh = true;
    void syncRuntimeQuery();
  }

  function setSearchText(value: string): void {
    searchText = value;
    forceRankingRefresh = true;
    void syncRuntimeQuery();
  }

  function setFocusMode(mode: FocusMode): void {
    focusMode = mode;
    forceRankingRefresh = true;
    void syncRuntimeQuery();
  }

  function currentRuntimeQuery(): RuntimeQuery {
    return {
      ...makeDefaultRuntimeQuery(),
      filter_text: searchText,
      focus_mode: focusMode,
      sort_column: sortColumnForKey(sortKey),
      sort_direction: sortDirection,
    };
  }

  async function syncRuntimeQuery(): Promise<void> {
    const query = currentRuntimeQuery();

    if (!hasTauriRuntime()) {
      runtimeQueryRequestSeq += 1;
      ingest(makeFixtureSnapshot(fixtureTick, query));
      return;
    }

    const requestSeq = (runtimeQueryRequestSeq += 1);
    try {
      const next = await setRuntimeProcessQuery(invoke, query);
      if (requestSeq !== runtimeQueryRequestSeq) {
        return;
      }
      applyNativeSnapshot(next);
    } catch (error) {
      if (requestSeq !== runtimeQueryRequestSeq) {
        return;
      }
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

  function ingest(next: RuntimeSnapshot): void {
    const previous = snapshot;
    hydrateRuntimeControls(next);
    const previousProcess = selectedPid ? selectedProcessFromSnapshot(previous, selectedPid) : null;
    const nextProcess = selectedPid ? selectedProcessFromSnapshot(next, selectedPid) : null;
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

    if (nextProcess) {
      const nextRates = processTrendRates(nextProcess);
      processHistory = {
        cpu: pushPoint(processHistory.cpu, nextProcess.cpu_percent),
        memory: pushPoint(
          processHistory.memory,
          percentage(nextProcess.memory_bytes, Math.max(next.system.memory_total_bytes, 1)),
        ),
        readRate: pushPoint(processHistory.readRate, nextRates.readRate),
        writeRate: pushPoint(processHistory.writeRate, nextRates.writeRate),
        networkRate: pushPoint(processHistory.networkRate, nextRates.networkRate),
      };
    } else if (previousProcess) {
      processHistory = {
        cpu: pushPoint(processHistory.cpu, 0),
        memory: pushPoint(processHistory.memory, 0),
        readRate: pushPoint(processHistory.readRate, 0),
        writeRate: pushPoint(processHistory.writeRate, 0),
        networkRate: pushPoint(processHistory.networkRate, 0),
      };
    }

    snapshot = next;
    updateProcessRows(next.process_view_rows);
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

    const useAttentionByDefault =
      next.settings.query.focus_mode === "all" &&
      !next.settings.query.filter_text.trim() &&
      isSystemPressured(next);
    searchText = next.settings.query.filter_text;
    sortKey = sortKeyForColumn(next.settings.query.sort_column);
    sortDirection = next.settings.query.sort_direction;
    focusMode = useAttentionByDefault ? "active" : next.settings.query.focus_mode;
    isPaused = next.settings.paused;
    hasHydratedRuntimeSettings = true;

    if (useAttentionByDefault) {
      window.setTimeout(() => void syncRuntimeQuery(), 0);
    }
  }

  function isSystemPressured(next: RuntimeSnapshot): boolean {
    return (
      next.system.cpu_percent >= 75 ||
      percentage(next.system.memory_used_bytes, next.system.memory_total_bytes) >= 85
    );
  }

  function selectProcess(pid: string): void {
    selectedPid = pid;
    detailSubject = "process";
    copyStatus = "";
    openCompactDetail();
    const process = selectedProcessFromSnapshot(snapshot, pid);
    if (process) {
      resetProcessHistory(process);
    }
  }

  async function hydrateProcessIcons(
    rows: RuntimeSnapshot["process_view_rows"],
    processes: ProcessSample[],
    selected: ProcessSample | null,
  ): Promise<void> {
    if (!hasNativeSnapshot) {
      return;
    }

    const visibleRowProcesses = rows.flatMap((row) => {
      if (row.kind === "group") {
        return row.representative ? [row.representative] : [];
      }
      return !row.is_grouped && row.process ? [row.process] : [];
    });
    const iconCandidates = uniqueIconCandidates(
      selected ? [selected, ...visibleRowProcesses, ...processes.slice(0, 80)] : [...visibleRowProcesses, ...processes.slice(0, 80)],
    ).slice(0, 120);
    for (const process of iconCandidates) {
      const key = processIconKey(process);
      if (!process.exe || processIcons[key] || requestedProcessIcons.has(key)) {
        continue;
      }

      requestedProcessIcons.add(key);
      let iconError = "";
      const icon = await getRuntimeProcessIcon(invoke, process.exe, (message) => {
        iconError = message;
      });
      if (icon) {
        processIcons = { ...processIcons, [key]: icon };
      } else if (iconError === "process_icon_untrusted_exe") {
        requestedProcessIcons.delete(key);
      }
    }
  }

  function processIconKey(process: ProcessSample): string {
    return process.exe || process.name;
  }

  function uniqueIconCandidates(processes: ProcessSample[]): ProcessSample[] {
    const seen = new Set<string>();
    return processes.filter((process) => {
      const key = processIconKey(process);
      if (seen.has(key)) {
        return false;
      }

      seen.add(key);
      return true;
    });
  }

  function selectDetailMode(mode: DetailMode): void {
    detailMode = mode;
    detailSubject = "system";
    selectedPid = "";
    openCompactDetail();
    applyPendingRankingIfReleased();
  }

  function openCompactDetail(): void {
    if (!isCompactDetail) {
      return;
    }

    if (!compactDetailOpen) {
      detailReturnFocus = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    }
    compactDetailOpen = true;
  }

  function closeCompactDetail(): void {
    compactDetailOpen = false;
    window.requestAnimationFrame(() => detailReturnFocus?.focus());
  }

  function handleAppKeydown(event: KeyboardEvent): void {
    if (event.key === "Escape" && compactDetailOpen && !settingsOpen && !diagnosticsOpen) {
      closeCompactDetail();
    }
  }

  function updateProcessRows(incoming: ProcessViewRow[]): void {
    if (forceRankingRefresh || displayProcessRows.length === 0 || !shouldHoldRanking()) {
      displayProcessRows = incoming;
      pendingProcessRows = [];
      rankingUpdateAvailable = false;
      forceRankingRefresh = false;
      return;
    }

    rankingUpdateAvailable = !hasSameProcessOrder(displayProcessRows, incoming);
    pendingProcessRows = incoming;
    displayProcessRows = stabilizeProcessRows(displayProcessRows, incoming);
  }

  function shouldHoldRanking(): boolean {
    return queueInteracting || expandedGroupCount > 0 || (detailSubject === "process" && !!selectedPid);
  }

  function applyPendingRanking(): void {
    if (pendingProcessRows.length > 0) {
      displayProcessRows = pendingProcessRows;
    }
    pendingProcessRows = [];
    rankingUpdateAvailable = false;
  }

  function applyPendingRankingIfReleased(): void {
    if (!shouldHoldRanking()) {
      applyPendingRanking();
    }
  }

  function setQueueInteraction(active: boolean): void {
    queueInteracting = active;
    if (!active) {
      applyPendingRankingIfReleased();
    }
  }

  function setExpandedGroupCount(count: number): void {
    expandedGroupCount = count;
    if (count === 0) {
      applyPendingRankingIfReleased();
    }
  }

  function resetProcessHistory(process: ProcessSample): void {
    const rates = processTrendRates(process);
    processHistory = {
      cpu: [process.cpu_percent],
      memory: [percentage(process.memory_bytes, Math.max(snapshot.system.memory_total_bytes, 1))],
      readRate: [rates.readRate],
      writeRate: [rates.writeRate],
      networkRate: [rates.networkRate],
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
      networkRate: [],
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
      networkRate: trimPoints(processHistory.networkRate),
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
          byteRate(
            process.other_io_total_bytes ?? 0,
            previousProcess?.other_io_total_bytes ?? process.other_io_total_bytes ?? 0,
            elapsedSeconds,
          ),
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

  function groupSelectionKey(key: string): string {
    return `group:${key}`;
  }

  function selectedGroupKey(selection: string): string | null {
    return selection.startsWith("group:") ? selection.slice("group:".length) : null;
  }

  function selectedProcessFromSnapshot(source: RuntimeSnapshot, selection: string): ProcessSample | null {
    const key = selectedGroupKey(selection);
    if (key) {
      const row = source.process_view_rows.find((candidate) => candidate.kind === "group" && candidate.group_key === key);
      return row ? groupProcessFromRow(row) : null;
    }

    return source.processes.find((process) => process.pid === selection) ?? null;
  }

  function groupProcessFromRow(row: RuntimeSnapshot["process_view_rows"][number]): ProcessSample {
    const representative = row.representative;
    return {
      pid: row.group_key ? groupSelectionKey(row.group_key) : "group",
      parent_pid: null,
      start_time_ms: representative?.start_time_ms ?? 0,
      name: row.group_label ?? representative?.name ?? "Process group",
      exe: "",
      status: "Group",
      cpu_percent: row.cpu_percent,
      kernel_cpu_percent: representative?.kernel_cpu_percent,
      memory_bytes: row.memory_bytes,
      private_bytes: row.memory_bytes,
      virtual_memory_bytes: row.memory_bytes,
      disk_read_total_bytes: representative?.disk_read_total_bytes ?? 0,
      disk_write_total_bytes: representative?.disk_write_total_bytes ?? 0,
      other_io_total_bytes: representative?.other_io_total_bytes,
      disk_read_bps: row.io_bps,
      disk_write_bps: 0,
      other_io_bps: 0,
      network_received_bps: row.network_bps,
      network_transmitted_bps: 0,
      threads: row.threads,
      handles: representative?.handles ?? 0,
      access_state: representative?.access_state ?? "full",
      quality: representative?.quality,
    };
  }

  function processTrendRates(process: ProcessSample): ProcessRates & { networkRate: number } {
    const rates = processRates[process.pid];
    return {
      readRate: rates?.readRate ?? process.disk_read_bps,
      writeRate: rates?.writeRate ?? process.disk_write_bps,
      otherRate: rates?.otherRate ?? process.other_io_bps ?? 0,
      networkRate: (process.network_received_bps ?? 0) + (process.network_transmitted_bps ?? 0),
    };
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

    if (rate > 0) {
      return formatRate(rate);
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

  function resourceStatusLabel(percent: number): string {
    if (percent >= 85) return "High";
    if (percent >= 65) return "Elevated";
    return "Steady";
  }

  function systemPressureHeadline(
    cpuPercent: number,
    currentMemoryPercent: number,
    diskRate: number,
    networkRate: number,
    contributor: string,
  ): string {
    const pressure = [
      { label: "CPU", score: cpuPercent / 100 },
      { label: "memory", score: currentMemoryPercent / 100 },
      { label: "disk", score: diskRate / (50 * 1024 * 1024) },
      { label: "network", score: networkRate / (25 * 1024 * 1024) },
    ].sort((left, right) => right.score - left.score)[0];

    if (pressure.score >= 0.85) {
      return `High ${pressure.label} pressure - ${contributor} is the top activity.`;
    }

    if (pressure.score >= 0.65) {
      return `${pressure.label[0].toLocaleUpperCase()}${pressure.label.slice(1)} is elevated - ${contributor} is the top activity.`;
    }

    return `System is steady - ${contributor} is the top activity.`;
  }

  function timeLabel(timestampMs: number): string {
    if (timestampMs <= 0) {
      return "--";
    }

    return new Intl.DateTimeFormat(undefined, {
      hour: "numeric",
      minute: "2-digit",
      second: "2-digit",
    }).format(new Date(timestampMs));
  }
</script>

<svelte:head>
  <title>BatCave Monitor</title>
</svelte:head>

<svelte:window onkeydown={handleAppKeydown} />

<AppShell {themeName}>
  <AppHeader
    {isPaused}
    {pollState}
    updatedAtLabel={timeLabel(snapshot.ts_ms)}
    {healthLabel}
    {healthTone}
    onOpenDiagnostics={() => (diagnosticsOpen = true)}
  />
  <SystemSummary
    resources={resourceSummaries}
    activeMode={detailMode}
    headline={systemHeadline}
    supportingText={systemSupportingText}
    onSelect={selectDetailMode}
  />
  <ProcessCommandBar
    {searchText}
    {focusMode}
    {sortKey}
    {isPaused}
    {commandError}
    {rankingUpdateAvailable}
    {focusOptions}
    {sortOptions}
    onSearch={setSearchText}
    onFocus={setFocusMode}
    onSort={setSortKey}
    onPaused={() => void setPaused(!isPaused)}
    onRefresh={() => void refreshNow()}
    onOpenSettings={() => (settingsOpen = true)}
    onApplyRanking={applyPendingRanking}
  />
  <section class="triage-workspace">
    <AttentionQueue
      processRows={processViewRows}
      totalProcessCount={snapshot.total_process_count || snapshot.system.process_count}
      {focusMode}
      {searchText}
      columns={visibleProcessColumns}
      {selectedPid}
      {sortKey}
      {sortDirection}
      {processIcons}
      {rankingUpdateAvailable}
      onSelect={selectProcess}
      onToggleSort={toggleSortKey}
      onInteractionChange={setQueueInteraction}
      onExpandedChange={setExpandedGroupCount}
    />
    {#if !isCompactDetail || compactDetailOpen}
      {#if isCompactDetail}
        <button
          class="detail-drawer-backdrop"
          type="button"
          aria-label="Close resource detail"
          onclick={closeCompactDetail}
        ></button>
      {/if}
      <DetailPane
        subject={detailSubject}
        compact={isCompactDetail}
        onClose={closeCompactDetail}
        onShowSystem={() => selectDetailMode(detailMode)}
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
    {/if}
  </section>

  <HealthStatus
    {snapshot}
    {sourceLabel}
    {systemQuality}
    {pollState}
    {lastError}
    adminStatus={adminStatusLabel()}
    open={diagnosticsOpen}
    onOpen={() => (diagnosticsOpen = true)}
    onClose={() => (diagnosticsOpen = false)}
  />

  <SettingsDrawer
    open={settingsOpen}
    {themeOptions}
    {themePreference}
    {pollIntervals}
    {pollIntervalMs}
    {historyPointOptions}
    {historyPointLimit}
    adminRequested={snapshot.settings.admin_mode_requested}
    adminEnabled={snapshot.settings.admin_mode_enabled}
    onClose={() => (settingsOpen = false)}
    onTheme={setTheme}
    onPollInterval={(interval) => (pollIntervalMs = interval as (typeof pollIntervals)[number])}
    onHistoryLimit={setHistoryPointLimit}
    onAdminMode={(enabled) => void setAdminMode(enabled)}
    onResetHistory={resetHistory}
  />
</AppShell>
