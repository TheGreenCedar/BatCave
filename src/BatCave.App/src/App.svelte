<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { check, type Update } from "@tauri-apps/plugin-updater";
  import { onMount } from "svelte";
  import DetailPane from "./lib/components/context/DetailPane.svelte";
  import ResourceRail from "./lib/components/metrics/ResourceRail.svelte";
  import SystemSummary from "./lib/components/metrics/SystemSummary.svelte";
  import type { DetailMode, ResourceSummaryOption } from "./lib/components/metrics/types";
  import AttentionQueue from "./lib/components/processes/AttentionQueue.svelte";
  import AppHeader from "./lib/components/shell/AppHeader.svelte";
  import AppShell from "./lib/components/shell/AppShell.svelte";
  import HealthStatus from "./lib/components/shell/HealthStatus.svelte";
  import ProcessCommandBar from "./lib/components/shell/ProcessCommandBar.svelte";
  import SettingsDrawer from "./lib/components/shell/SettingsDrawer.svelte";
  import { buildResourceBrief, type CollectionState } from "./lib/cockpit";
  import { uniqueWarningCount } from "./lib/diagnostics";
  import {
    installKindLabel,
    privilegedCollectionAction,
    privilegedCollectionLabel,
    privilegedCollectionNote,
    processElevationLabel,
  } from "./lib/environmentPresentation";
  import {
    accessLabel,
    displayGroupMetricValue,
    displayProcessMetricValue,
    formatBytes,
    formatOptionalRate,
    formatPercent,
    formatRate,
    groupMetricCanDisplay,
    logicalCpuMetricQuality,
    metricQualityLabel,
    metricQualityShortLabel,
    nextProcessMetricHistory,
    processMemoryQuality,
  } from "./lib/format";
  import { makeFixtureSnapshot } from "./lib/fixtures";
  import { nextMetricHistory, resourceHistoryWindowLabel } from "./lib/history";
  import {
    platformPresentation,
    privateMemoryValue,
    residentMemoryValue,
  } from "./lib/platformPresentation";
  import {
    defaultSortDirection,
    focusOptions,
    hasSameProcessOrder,
    nextSortDirection,
    processColumns,
    processIoRate,
    processIdentity,
    processNeedsAttention,
    processOtherIoRate,
    prepareProcessViewRows,
    processSelectionKey,
    processViewRowKey,
    selectedWorkloadDetail,
    sortColumnForKey,
    sortKeyForColumn,
    sortOptions,
    shouldHoldProcessOrder,
    stabilizeProcessRows,
    type FocusMode,
    type ProcessRates,
    type SortKey,
  } from "./lib/process";
  import {
    hasNewRuntimeSample,
    makeDefaultRuntimeQuery,
    makeEmptySnapshot,
    shouldApplyRuntimePublication,
    shouldPollRuntime,
  } from "./lib/runtimeSnapshot";
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
    getRuntimeProcessIcons,
    readNativeSnapshot,
    refreshRuntime,
    setRuntimePaused,
    setRuntimeAdminMode,
    setRuntimeProcessQuery,
    setRuntimeSampleInterval,
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
    WorkloadDetail,
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

  const accessibilityFixtureStates = [
    "overview",
    "process",
    "group",
    "settings",
    "diagnostics",
    "stale",
    "degraded",
    "compact",
  ] as const;
  type AccessibilityFixtureState = (typeof accessibilityFixtureStates)[number];

  const pollIntervals = [500, 1000, 2000] as const;
  const historyStorageKey = "batcave.monitor.history-points";
  const browserFixturePlatform = "macos" as const;
  const accessibilityFixtureState = readAccessibilityFixtureState();

  let fixtureTick = 0;
  let snapshot: RuntimeSnapshot = makeEmptySnapshot();
  let selectedWorkloadId = "";
  let hasAutoSelectedWorkload = false;
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
  let healthTone: "healthy" | "warning" | "danger" = "healthy";
  let collectionState: CollectionState = "live";
  let forceRankingRefresh = false;
  let runtimeQueryRequestSeq = 0;
  let searchDebounceId: number | undefined;
  let updateStatus: "idle" | "checking" | "available" | "current" | "installing" | "error" = "idle";
  let updateMessage = "Checks only when you ask.";
  let pendingUpdate: Update | null = null;

  $: themeName = resolveThemeName(themePreference, systemThemeName);
  $: activeTheme = chartPalettes[themeName];
  $: presentation = platformPresentation(snapshot.environment);
  $: memoryPercent = percentage(snapshot.system.memory_used_bytes, snapshot.system.memory_total_bytes);
  $: swapPercent = percentage(
    snapshot.system.swap_used_bytes ?? 0,
    snapshot.system.swap_total_bytes ?? 0,
  );
  $: processViewRows = displayProcessRows;
  $: filteredProcesses = processViewRows.flatMap((row) =>
    row.kind === "process" ? [row.detail.process] : [],
  );
  $: selectedRow =
    processViewRows.find((row) => processViewRowKey(row) === selectedWorkloadId) ??
    snapshot.process_view_rows.find((row) => processViewRowKey(row) === selectedWorkloadId) ??
    null;
  $: selectedWorkload = selectedRow?.detail ?? null;
  $: selectedProcess =
    selectedWorkload?.kind === "process" ? selectedWorkload.process : null;
  $: selectedWorkloadIconKind =
    ((selectedRow?.icon_kind ?? "process") as import("./lib/process").ProcessIconKind);
  $: selectedWorkloadIconSrc =
    selectedRow?.kind === "group" && selectedRow.icon_source
      ? processIcons[selectedRow.icon_source]
      : selectedProcess
        ? processIcons[selectedProcess.exe || selectedProcess.name]
        : undefined;
  $: processNetworkAvailable = processViewRows.some((row) => {
    const quality =
      row.kind === "group" ? row.detail.quality.network : row.detail.process.quality?.network;
    return quality?.quality !== "unavailable";
  });
  $: sourceLabel =
    snapshot.source === "batcave_runtime" ||
    snapshot.source === "tauri_runtime" ||
    snapshot.source === "tauri_sysinfo"
      ? "native telemetry"
      : "fixture demo";
  $: systemQuality = snapshot.system.quality ?? {};
  $: visibleProcessColumns = processColumns
    .filter((column) => column.key !== "network" || processNetworkAvailable)
    .map((column) =>
      column.key === "memory" ? { ...column, label: presentation.memoryLabel } : column,
    );
  $: memoryAccounting = snapshot.system.memory_accounting;
  $: topKernelPoolTags = topPoolTags(memoryAccounting?.kernel_pool_tags);
  $: blockedProcessCount =
    memoryAccounting?.denied_process_count ?? snapshot.processes.filter((process) => process.access_state === "denied").length;
  $: diskReadRate = snapshot.system.disk_read_bps;
  $: diskWriteRate = snapshot.system.disk_write_bps;
  $: networkDownRate = snapshot.system.network_received_bps;
  $: networkUpRate = snapshot.system.network_transmitted_bps;
  $: diskTotalHistory = combineSeries(history.diskRead, history.diskWrite);
  $: networkTotalHistory = combineSeries(history.netRx, history.netTx);
  $: diskScaleMax = maxRate(diskTotalHistory, 1_000_000);
  $: networkScaleMax = maxRate(networkTotalHistory, 750_000);
  $: selectedRates = selectedProcess ? processRates[processSelectionKey(selectedProcess)] : undefined;
  $: processReadRate = selectedRates?.readRate ?? processHistory.readRate.at(-1) ?? 0;
  $: processWriteRate = selectedRates?.writeRate ?? processHistory.writeRate.at(-1) ?? 0;
  $: void hydrateProcessIcons(processViewRows, filteredProcesses, selectedProcess);
  $: coreLoads = history.cores.flatMap((core, index) =>
    core.length > 0 ? [{ index, load: currentCoreLoad(core), trend: core }] : [],
  );
  $: corePeak = Math.max(...coreLoads.map((core) => core.load), 0);
  $: coreMinimum = coreLoads.length > 0 ? Math.min(...coreLoads.map((core) => core.load)) : 0;
  $: coreSpread = Math.max(0, corePeak - coreMinimum);
  $: hotCoreCount = coreLoads.filter((core) => core.load >= 75).length;
  $: busyCoreCount = coreLoads.filter((core) => core.load >= 45).length;
  $: collectionState = pollState === "error" ? "stale" : isPaused ? "paused" : "live";
  $: resourceBrief = buildResourceBrief(
    snapshot,
    detailMode,
    {
      memoryPercent,
      diskRate: diskReadRate + diskWriteRate,
      networkRate: networkDownRate + networkUpRate,
    },
    collectionState,
  );
  $: leadingContributor = resourceBrief.leadingWorkload
    ? resourceBrief.mode === "cpu"
      ? snapshot.process_contributors.cpu
      : resourceBrief.mode === "memory"
        ? snapshot.process_contributors.memory
        : resourceBrief.mode === "network"
          ? snapshot.process_contributors.network
          : null
    : null;
  $: leadingProcess = leadingContributor
    ? resourceBrief.contributorNameAmbiguous
      ? null
      : (() => {
        const matches = snapshot.processes.filter(
          (process) => process.name === leadingContributor,
        );
        return matches.length === 1 ? matches[0] : null;
      })()
    : null;
  $: leadingIdentity = leadingProcess ? processIdentity(leadingProcess) : null;
  $: limitationCount = uniqueWarningCount(snapshot.warnings) || snapshot.health.collector_warnings;
  $: sampledAtLabel = snapshot.sampled_at_ms ? ageLabel(snapshot.sampled_at_ms) : "no sample yet";
  $: systemSupportingText = pollState === "error"
    ? `Telemetry is unavailable; the last successful sample from ${sampledAtLabel} is retained.`
    : isPaused
      ? `Collection is paused; values and charts show the last sample from ${sampledAtLabel}.`
      : limitationCount > 0
        ? `${limitationCount} telemetry limitation${limitationCount === 1 ? "" : "s"}; unaffected values remain current.`
        : snapshot.health.degraded
          ? "BatCave resource use is above its budget; telemetry remains current."
        : "Local telemetry is current. Select a resource or workload to inspect it.";
  $: healthTone = pollState === "error" ? "danger" : isPaused || snapshot.health.degraded ? "warning" : "healthy";
  $: healthLabel = pollState === "error"
    ? "Telemetry stale"
    : isPaused
      ? "Telemetry paused"
      : limitationCount > 0
        ? `${limitationCount} limitation${limitationCount === 1 ? "" : "s"}`
        : snapshot.health.degraded
          ? "App resource warning"
        : "Telemetry healthy";
  $: railDiagnosticsLabel = pollState === "error"
    ? "Stale"
    : isPaused
      ? "Paused"
      : limitationCount > 0
        ? `${limitationCount} limit${limitationCount === 1 ? "" : "s"}`
        : snapshot.health.degraded
          ? "Warning"
          : "Healthy";
  $: liveStatus = rankingUpdateAvailable ? `${healthLabel}. A new workload ranking is available.` : healthLabel;
  $: detailTitle =
    detailMode === "cpu"
      ? "Logical cores"
      : detailMode === "memory"
        ? "Memory detail"
        : detailMode === "disk"
          ? "Disk throughput"
          : "Network throughput";
  $: cpuDetailValue = metricValueLabel(
    snapshot.system.cpu_percent,
    systemQuality.cpu,
    formatPercent,
  );
  $: memoryDetailValue = metricValueLabel(
    memoryPercent,
    systemQuality.memory,
    formatPercent,
  );
  $: diskDetailValue = metricValueLabel(
    diskReadRate + diskWriteRate,
    systemQuality.disk,
    formatRate,
  );
  $: networkDetailValue = metricValueLabel(
    networkDownRate + networkUpRate,
    systemQuality.network,
    formatRate,
  );
  $: detailReadout =
    detailMode === "cpu"
      ? cpuDetailValue === "Unavailable" || cpuDetailValue === "Waiting"
        ? cpuDetailValue
        : `${cpuDetailValue} machine total`
      : detailMode === "memory"
        ? memoryDetailValue === "Unavailable" || memoryDetailValue === "Waiting"
          ? memoryDetailValue
          : `${memoryDetailValue} used`
        : detailMode === "disk"
          ? diskDetailValue
          : networkDetailValue;
  $: resourceSummaries = [
    {
      mode: "cpu",
      ariaLabel: "Open CPU logical core detail",
      label: "Machine CPU",
      value: metricValueLabel(snapshot.system.cpu_percent, systemQuality.cpu, formatPercent),
      supportingLabel: "Peak logical core",
      supportingValue: metricValueLabel(
        corePeak,
        logicalCpuMetricQuality(systemQuality),
        formatPercent,
      ),
      statusLabel: resourceQualityStatus(systemQuality.cpu, "Measured"),
      shortStatusLabel: resourceQualityShortStatus(systemQuality.cpu, "Measured"),
      values: history.cpu,
      max: 100,
      stroke: activeTheme.cpuStroke,
      fill: activeTheme.cpuFill,
    },
    {
      mode: "memory",
      ariaLabel: "Open memory detail",
      label: "Memory",
      value: metricValueLabel(memoryPercent, systemQuality.memory, formatPercent),
      supportingLabel: "Used",
      supportingValue: metricValueLabel(
        snapshot.system.memory_used_bytes,
        systemQuality.memory,
        formatBytes,
      ),
      statusLabel: resourceQualityStatus(systemQuality.memory, "Measured"),
      shortStatusLabel: resourceQualityShortStatus(systemQuality.memory, "Measured"),
      values: history.memory,
      max: 100,
      stroke: activeTheme.memoryStroke,
      fill: activeTheme.memoryFill,
    },
    {
      mode: "disk",
      ariaLabel: "Open disk throughput detail",
      label: "Disk",
      value: metricValueLabel(diskReadRate + diskWriteRate, systemQuality.disk, formatRate),
      supportingLabel: "Read / write",
      supportingValue: !metricCanDisplay(systemQuality.disk)
        ? "No trusted sample"
        : `${formatRate(diskReadRate)} / ${formatRate(diskWriteRate)}`,
      statusLabel: resourceQualityStatus(systemQuality.disk, "Aggregate"),
      shortStatusLabel: resourceQualityShortStatus(systemQuality.disk, "Aggregate"),
      values: diskTotalHistory,
      max: diskScaleMax,
      stroke: activeTheme.diskWriteStroke,
      fill: activeTheme.diskWriteFill,
    },
    {
      mode: "network",
      ariaLabel: "Open network throughput detail",
      label: "Network",
      value: metricValueLabel(networkDownRate + networkUpRate, systemQuality.network, formatRate),
      supportingLabel: "Down / up",
      supportingValue: !metricCanDisplay(systemQuality.network)
        ? "No trusted sample"
        : `${formatRate(networkDownRate)} / ${formatRate(networkUpRate)}`,
      statusLabel: resourceQualityStatus(systemQuality.network, "Aggregate"),
      shortStatusLabel: resourceQualityShortStatus(systemQuality.network, "Aggregate"),
      values: networkTotalHistory,
      max: networkScaleMax,
      stroke: activeTheme.networkDownStroke,
      fill: activeTheme.networkDownFill,
    },
  ];
  $: activeResource =
    resourceSummaries.find((resource) => resource.mode === detailMode) ?? resourceSummaries[0];
  $: resourceWindowLabel = resourceHistoryWindowLabel(
    activeResource?.values.length ?? 0,
    snapshot.settings.sample_interval_ms,
    systemQuality[detailMode],
    snapshot.sampled_at_ms !== null,
  );

  onMount(() => {
    let timeoutId: number | undefined;
    let disposed = false;
    const systemThemeQuery = window.matchMedia("(prefers-color-scheme: light)");
    const compactDetailQuery = window.matchMedia("(max-width: 1279px)");
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
      if (accessibilityFixtureState) {
        fixtureTick = 8;
        const next = makeFixtureSnapshot(
          fixtureTick,
          { ...currentRuntimeQuery(), limit: accessibilityFixtureState === "group" ? 48 : 24 },
          accessibilityFixtureState === "group" ? "windows" : browserFixturePlatform,
        );
        prepareAccessibilityFixture(next, accessibilityFixtureState);
        ingest(next);
        applyAccessibilityFixtureSelection(next, accessibilityFixtureState);
      } else {
        ingest(makeFixtureSnapshot(fixtureTick, currentRuntimeQuery(), browserFixturePlatform));
      }
    }

    const loop = async () => {
      if (shouldPollRuntime(isPaused, hasTauriRuntime())) {
        const next = await readSnapshot();
        ingest(next);
      }

      if (!disposed) {
        timeoutId = window.setTimeout(loop, pollIntervalMs);
      }
    };

    if (!accessibilityFixtureState) {
      timeoutId = window.setTimeout(loop, 120);
    }

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
      if (searchDebounceId !== undefined) {
        window.clearTimeout(searchDebounceId);
      }
    };
  });

  async function readSnapshot(): Promise<RuntimeSnapshot> {
    if (!hasTauriRuntime()) {
      fixtureTick += 1;
      pollState = "fixture";
      lastError = "";
      return makeFixtureSnapshot(fixtureTick, currentRuntimeQuery(), browserFixturePlatform);
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

  function readAccessibilityFixtureState(): AccessibilityFixtureState | null {
    if (!import.meta.env.DEV || typeof window === "undefined") return null;
    const requested = new URLSearchParams(window.location.search).get("a11y");
    return accessibilityFixtureStates.find((state) => state === requested) ?? null;
  }

  function prepareAccessibilityFixture(
    next: RuntimeSnapshot,
    state: AccessibilityFixtureState,
  ): void {
    pollState = state === "stale" ? "error" : "fixture";
    lastError = state === "stale" ? "Fixture collector stopped after the last trusted sample." : "";

    if (state === "degraded") {
      next.health.degraded = true;
      next.health.app_cpu_percent = 3.2;
      next.health.status_summary = "Fixture app resource budget is exceeded.";
    }
  }

  function applyAccessibilityFixtureSelection(
    next: RuntimeSnapshot,
    state: AccessibilityFixtureState,
  ): void {
    settingsOpen = state === "settings";
    diagnosticsOpen = state === "diagnostics";
    compactDetailOpen = false;

    if (state === "process" || state === "compact") {
      const processRow = next.process_view_rows.find(
        (row) => row.kind === "process" && !row.is_grouped,
      );
      if (processRow) {
        selectedWorkloadId = processViewRowKey(processRow);
        detailSubject = "process";
        compactDetailOpen = state === "compact";
      }
      return;
    }

    if (state === "group") {
      const groupRow = next.process_view_rows.find((row) => row.kind === "group");
      if (groupRow) {
        selectedWorkloadId = processViewRowKey(groupRow);
        detailSubject = "process";
      }
      return;
    }

    selectedWorkloadId = "";
    detailSubject = "system";
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
      ingest(makeFixtureSnapshot(fixtureTick, currentRuntimeQuery(), browserFixturePlatform));
      return;
    }

    try {
      const next = await refreshRuntime(invoke);
      applyNativeSnapshot(next);
    } catch (error) {
      commandError = commandErrorMessage(error, "Unable to refresh runtime.");
    }
  }

  async function setPollInterval(interval: number): Promise<void> {
    pollIntervalMs = interval as (typeof pollIntervals)[number];
    if (!hasTauriRuntime()) return;

    try {
      applyNativeSnapshot(await setRuntimeSampleInterval(invoke, interval));
    } catch (error) {
      commandError = commandErrorMessage(error, "Unable to change sampling cadence.");
    }
  }

  async function setAdminMode(enabled: boolean): Promise<void> {
    if (!hasTauriRuntime()) return;

    try {
      applyNativeSnapshot(await setRuntimeAdminMode(invoke, enabled));
    } catch (error) {
      commandError = commandErrorMessage(error, "Unable to change privileged collection.");
    }
  }

  async function checkForStableUpdate(): Promise<void> {
    if (snapshot.environment.install_kind === "deb") {
      updateStatus = "current";
      updateMessage = "Debian packages update through your package manager or a downloaded .deb release.";
      return;
    }
    updateStatus = "checking";
    updateMessage = "Checking the stable release channel…";
    pendingUpdate = null;
    try {
      pendingUpdate = await check({ timeout: 15_000 });
      if (pendingUpdate) {
        updateStatus = "available";
        updateMessage = `Version ${pendingUpdate.version} is available.`;
      } else {
        updateStatus = "current";
        updateMessage = "BatCave is up to date.";
      }
    } catch (error) {
      updateStatus = "error";
      updateMessage = String(error).includes("404")
        ? "No stable release is published yet."
        : "Unable to reach the update service. Monitoring remains available offline.";
    }
  }

  async function installStableUpdate(): Promise<void> {
    if (!pendingUpdate) return;
    updateStatus = "installing";
    updateMessage = "Downloading and verifying the signed update…";
    try {
      updateMessage = "Installing the verified update. BatCave will close when installation begins.";
      await pendingUpdate.downloadAndInstall();
    } catch {
      updateStatus = "error";
      updateMessage = "Update verification or installation failed. BatCave was not changed.";
    }
  }

  function setSortKey(key: SortKey): void {
    sortKey = key;
    sortDirection = defaultSortDirection(key);
    forceRankingRefresh = true;
    flushRuntimeQuery();
  }

  function toggleSortKey(key: SortKey): void {
    if (sortKey === key) {
      sortDirection = nextSortDirection(sortDirection);
    } else {
      sortKey = key;
      sortDirection = defaultSortDirection(key);
    }

    forceRankingRefresh = true;
    flushRuntimeQuery();
  }

  function toggleSortDirection(): void {
    sortDirection = nextSortDirection(sortDirection);
    forceRankingRefresh = true;
    flushRuntimeQuery();
  }

  function setSearchText(value: string): void {
    searchText = value;
    forceRankingRefresh = true;
    if (searchDebounceId !== undefined) {
      window.clearTimeout(searchDebounceId);
    }
    searchDebounceId = window.setTimeout(() => {
      searchDebounceId = undefined;
      void syncRuntimeQuery();
    }, 200);
  }

  function setFocusMode(mode: FocusMode): void {
    focusMode = mode;
    forceRankingRefresh = true;
    flushRuntimeQuery();
  }

  function flushRuntimeQuery(): void {
    if (searchDebounceId !== undefined) {
      window.clearTimeout(searchDebounceId);
      searchDebounceId = undefined;
    }
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
      ingest(makeFixtureSnapshot(fixtureTick, query, browserFixturePlatform));
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
    if (!shouldApplyRuntimePublication(snapshot, next)) {
      return;
    }

    const previous = snapshot;
    hydrateRuntimeControls(next);
    const previousWorkload = selectedWorkloadId
      ? selectedWorkloadFromSnapshot(previous, selectedWorkloadId)
      : null;
    const nextWorkload = selectedWorkloadId
      ? selectedWorkloadFromSnapshot(next, selectedWorkloadId)
      : null;
    const hasNewSample = hasNewRuntimeSample(previous, next);
    const logicalCpu = next.system.logical_cpu_percent.length
      ? next.system.logical_cpu_percent
      : [next.system.cpu_percent];
    const nextMemoryPercent = percentage(next.system.memory_used_bytes, next.system.memory_total_bytes);
    isPaused = next.settings.paused;
    if (pollIntervals.includes(next.settings.sample_interval_ms as (typeof pollIntervals)[number])) {
      pollIntervalMs = next.settings.sample_interval_ms as (typeof pollIntervals)[number];
    }
    snapshot = next;
    updateProcessRows(next.process_view_rows);
    if (!selectedWorkloadId && !hasAutoSelectedWorkload) {
      const firstWorkload = next.process_view_rows.find(
        (row) => row.kind === "group" || !row.is_grouped,
      );
      if (firstWorkload) {
        selectedWorkloadId = processViewRowKey(firstWorkload);
        hasAutoSelectedWorkload = true;
        detailSubject = "process";
      }
    }

    if (!hasNewSample) {
      return;
    }

    processRates = buildProcessRates(next.processes);

    if (nextWorkload) {
      const nextRates = workloadTrendRates(nextWorkload);
      const nextMetrics = workloadMetrics(nextWorkload);
      processHistory = {
        cpu: nextWorkloadHistory(
          processHistory.cpu,
          nextMetrics.cpuPercent,
          nextWorkload,
          "cpu",
        ),
        memory: nextWorkloadHistory(
          processHistory.memory,
          percentage(nextMetrics.memoryBytes, Math.max(next.system.memory_total_bytes, 1)),
          nextWorkload,
          "memory",
        ),
        readRate: nextWorkloadHistory(
          processHistory.readRate,
          nextRates.readRate,
          nextWorkload,
          "io",
        ),
        writeRate: nextWorkloadHistory(
          processHistory.writeRate,
          nextRates.writeRate,
          nextWorkload,
          "io",
        ),
        networkRate: nextWorkloadHistory(
          processHistory.networkRate,
          nextRates.networkRate,
          nextWorkload,
          "network",
        ),
      };
    } else if (previousWorkload) {
      processHistory = emptyProcessTrendState();
    }

    history = {
      cpu: nextMetricHistory(
        history.cpu,
        next.system.cpu_percent,
        next.system.quality?.cpu,
        historyPointLimit,
      ),
      memory: nextMetricHistory(
        history.memory,
        nextMemoryPercent,
        next.system.quality?.memory,
        historyPointLimit,
      ),
      swap:
        next.system.swap_total_bytes && next.system.swap_used_bytes !== undefined
          ? pushPoint(history.swap, percentage(next.system.swap_used_bytes, next.system.swap_total_bytes))
          : history.swap,
      diskRead: nextMetricHistory(
        history.diskRead,
        next.system.disk_read_bps,
        next.system.quality?.disk,
        historyPointLimit,
      ),
      diskWrite: nextMetricHistory(
        history.diskWrite,
        next.system.disk_write_bps,
        next.system.quality?.disk,
        historyPointLimit,
      ),
      netRx: nextMetricHistory(
        history.netRx,
        next.system.network_received_bps,
        next.system.quality?.network,
        historyPointLimit,
      ),
      netTx: nextMetricHistory(
        history.netTx,
        next.system.network_transmitted_bps,
        next.system.quality?.network,
        historyPointLimit,
      ),
      cores: logicalCpu.map((value, index) =>
        nextMetricHistory(
          history.cores[index] ?? [],
          value,
          logicalCpuMetricQuality(next.system.quality ?? {}),
          historyPointLimit,
        ),
      ),
    };
  }

  function hydrateRuntimeControls(next: RuntimeSnapshot): void {
    if (next.source === "fixture" || hasHydratedRuntimeSettings || (!hasNativeSnapshot && pollState === "error")) {
      return;
    }

    const useAttentionByDefault =
      next.settings.query.focus_mode === "all" &&
      !next.settings.query.filter_text.trim() &&
      isSystemPressured(next) &&
      next.processes.some(processNeedsAttention);
    searchText = next.settings.query.filter_text;
    sortKey = sortKeyForColumn(next.settings.query.sort_column);
    sortDirection = next.settings.query.sort_direction;
    focusMode = useAttentionByDefault ? "attention" : next.settings.query.focus_mode;
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

  function selectProcess(selection: string): void {
    selectedWorkloadId = selection;
    detailSubject = "process";
    copyStatus = "";
    openCompactDetail();
    const workload = selectedWorkloadFromSnapshot(snapshot, selection);
    if (workload) {
      resetWorkloadHistory(workload);
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
        const iconProcess = row.icon_source
          ? processes.find((process) => process.exe === row.icon_source)
          : undefined;
        return iconProcess ? [iconProcess] : [];
      }
      return !row.is_grouped ? [row.detail.process] : [];
    });
    const iconCandidates = uniqueIconCandidates(
      selected ? [selected, ...visibleRowProcesses, ...processes.slice(0, 80)] : [...visibleRowProcesses, ...processes.slice(0, 80)],
    ).slice(0, 120);
    const pending = iconCandidates.filter((process) => {
      const key = processIconKey(process);
      if (!process.exe || processIcons[key] || requestedProcessIcons.has(key)) {
        return false;
      }
      requestedProcessIcons.add(key);
      return true;
    });
    if (!pending.length) return;

    let iconError = "";
    const icons = await getRuntimeProcessIcons(
      invoke,
      pending.map((process) => process.exe),
      (message) => (iconError = message),
    );
    for (const process of pending) {
      const key = processIconKey(process);
      const icon = icons[process.exe];
      if (icon) {
        processIcons = { ...processIcons, [key]: icon };
      } else if (iconError === "process_icon_untrusted_exe") {
        requestedProcessIcons.delete(key);
      }
    }
    const cached = Object.entries(processIcons);
    if (cached.length > 256) processIcons = Object.fromEntries(cached.slice(-256));
    requestedProcessIcons = new Set(iconCandidates.map(processIconKey));
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
    selectedWorkloadId = "";
    openCompactDetail();
    applyPendingRankingIfReleased();
  }

  function openCompactDetail(): void {
    if (!isCompactDetail) {
      return;
    }
    compactDetailOpen = true;
  }

  function closeCompactDetail(): void {
    compactDetailOpen = false;
    applyPendingRankingIfReleased();
  }

  function handleAppKeydown(event: KeyboardEvent): void {
    const target = event.target;
    if (event.key === "Enter" && target instanceof HTMLInputElement && target.id === "process-search") {
      flushRuntimeQuery();
      return;
    }

    if (
      event.key === "/" &&
      !event.altKey &&
      !event.ctrlKey &&
      !event.metaKey &&
      !event.altKey &&
      !event.shiftKey &&
      !settingsOpen &&
      !diagnosticsOpen &&
      !compactDetailOpen &&
      !(target instanceof HTMLInputElement) &&
      !(target instanceof HTMLTextAreaElement) &&
      !(target instanceof HTMLSelectElement) &&
      !(target instanceof HTMLButtonElement) &&
      !(target instanceof HTMLElement && target.isContentEditable)
    ) {
      const search = document.querySelector<HTMLInputElement>("#process-search");
      if (search) {
        event.preventDefault();
        search.focus();
        search.select();
      }
    }
  }

  function updateProcessRows(incoming: ProcessViewRow[]): void {
    const prepared = prepareProcessViewRows(incoming, selectedWorkloadId, 180);
    if (selectedWorkloadId && !prepared.selection) {
      selectedWorkloadId = "";
      detailSubject = "system";
      compactDetailOpen = false;
    }
    incoming = prepared.rows;

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
    const detailOpen = !isCompactDetail || compactDetailOpen;
    const selectedWorkloadVisible =
      !!selectedWorkloadId && selectionIsVisible(processViewRows, selectedWorkloadId);
    return shouldHoldProcessOrder(
      sortKey,
      queueInteracting,
      expandedGroupCount,
      detailSubject === "process" && detailOpen && selectedWorkloadVisible,
    );
  }

  function selectionIsVisible(rows: ProcessViewRow[], selection: string): boolean {
    const row = rows.find((candidate) => processViewRowKey(candidate) === selection);
    return !!row && (row.kind === "group" || !row.is_grouped || expandedGroupCount > 0);
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

  function resetWorkloadHistory(workload: WorkloadDetail): void {
    const rates = workloadTrendRates(workload);
    const metrics = workloadMetrics(workload);
    processHistory = {
      cpu: initialWorkloadHistory(metrics.cpuPercent, workload, "cpu"),
      memory: initialWorkloadHistory(
        percentage(metrics.memoryBytes, Math.max(snapshot.system.memory_total_bytes, 1)),
        workload,
        "memory",
      ),
      readRate: initialWorkloadHistory(rates.readRate, workload, "io"),
      writeRate: initialWorkloadHistory(rates.writeRate, workload, "io"),
      networkRate: initialWorkloadHistory(rates.networkRate, workload, "network"),
    };
  }

  function resetHistory(): void {
    history = emptyTrendState();
    if (selectedWorkload) {
      resetWorkloadHistory(selectedWorkload);
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

  function combineSeries(left: number[], right: number[]): number[] {
    const length = Math.max(left.length, right.length);
    const leftOffset = length - left.length;
    const rightOffset = length - right.length;
    return Array.from({ length }, (_, index) =>
      (left[index - leftOffset] ?? 0) + (right[index - rightOffset] ?? 0),
    );
  }

  function metricValueLabel(
    value: number,
    quality: MetricQualityInfo | undefined,
    formatter: (value: number) => string,
  ): string {
    if (snapshot.sampled_at_ms === null || quality?.quality === "unavailable") {
      return "Unavailable";
    }
    if (quality?.quality === "held") return "Waiting";
    return formatter(value);
  }

  function metricCanDisplay(quality: MetricQualityInfo | undefined): boolean {
    return (
      snapshot.sampled_at_ms !== null &&
      quality?.quality !== "unavailable" &&
      quality?.quality !== "held"
    );
  }

  function resourceQualityStatus(
    quality: MetricQualityInfo | undefined,
    fallback: string,
  ): string {
    if (snapshot.sampled_at_ms === null) return "No sample";
    if (pollState === "error") return "Stale last sample";
    if (isPaused) return "Paused at last sample";
    if (quality?.quality === "unavailable" || quality?.quality === "held" || quality?.quality === "partial") {
      return metricQualityLabel(quality, fallback);
    }
    if (snapshot.health.degraded || snapshot.warnings.length > 0) {
      return `Degraded / ${metricQualityLabel(quality, fallback)}`;
    }
    return metricQualityLabel(quality, fallback);
  }

  function resourceQualityShortStatus(
    quality: MetricQualityInfo | undefined,
    fallback: string,
  ): string {
    if (snapshot.sampled_at_ms === null) return "No sample";
    if (pollState === "error") return "Stale";
    if (isPaused) return "Paused";
    if (quality?.quality === "unavailable" || quality?.quality === "held" || quality?.quality === "partial") {
      return metricQualityShortLabel(quality, fallback);
    }
    if (snapshot.health.degraded || snapshot.warnings.length > 0) return "Degraded";
    return metricQualityShortLabel(quality, fallback);
  }

  function buildProcessRates(nextProcesses: ProcessSample[]): Record<string, ProcessRates> {
    const rates: Record<string, ProcessRates> = {};

    for (const process of nextProcesses) {
      rates[processSelectionKey(process)] = {
        readRate: process.io_read_bps,
        otherRate: process.other_io_bps,
        writeRate: process.io_write_bps,
      };
    }

    return rates;
  }

  function selectedWorkloadFromSnapshot(
    source: RuntimeSnapshot,
    selection: string,
  ): WorkloadDetail | null {
    return selectedWorkloadDetail(source.process_view_rows, selection);
  }

  function workloadMetrics(workload: WorkloadDetail): import("./lib/process").WorkloadMetrics {
    if (workload.kind === "group") {
      return {
        cpuPercent: workload.cpu_percent,
        memoryBytes: workload.memory_bytes,
        ioBps: workload.io_bps,
        networkBps: workload.network_bps,
        threads: workload.threads,
      };
    }

    return {
      cpuPercent: workload.process.cpu_percent,
      memoryBytes: workload.process.memory_bytes,
      ioBps: workload.io_bps,
      networkBps: workload.network_bps,
      threads: workload.process.threads,
    };
  }

  function workloadTrendRates(workload: WorkloadDetail): ProcessRates & { networkRate: number } {
    if (workload.kind === "group") {
      return {
        readRate: workload.io_bps,
        writeRate: 0,
        otherRate: undefined,
        networkRate: workload.network_bps,
      };
    }

    const process = workload.process;
    const rates = processRates[processSelectionKey(process)];
    return {
      readRate: rates?.readRate ?? process.io_read_bps,
      writeRate: rates?.writeRate ?? process.io_write_bps,
      otherRate: rates?.otherRate ?? process.other_io_bps,
      networkRate: workload.network_bps,
    };
  }

  type HistoricalGroupMetric = "cpu" | "memory" | "io" | "network";

  function initialWorkloadHistory(
    value: number,
    workload: WorkloadDetail,
    metric: HistoricalGroupMetric,
  ): number[] {
    if (workload.kind === "group") {
      return groupMetricCanDisplay(workload.quality[metric], workload.coverage[metric])
        ? [value]
        : [];
    }
    const quality =
      metric === "memory" ? processMemoryQuality(workload.process) : workload.process.quality?.[metric];
    return nextProcessMetricHistory([], value, quality, historyPointLimit);
  }

  function nextWorkloadHistory(
    values: number[],
    value: number,
    workload: WorkloadDetail,
    metric: HistoricalGroupMetric,
  ): number[] {
    if (workload.kind === "group") {
      return groupMetricCanDisplay(workload.quality[metric], workload.coverage[metric])
        ? pushPoint(values, value)
        : [];
    }
    const quality =
      metric === "memory" ? processMemoryQuality(workload.process) : workload.process.quality?.[metric];
    return nextProcessMetricHistory(values, value, quality, historyPointLimit);
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
    return privilegedCollectionLabel(snapshot.admin_mode, blockedProcessCount);
  }

  function processNetworkLabel(process: ProcessSample): string {
    const quality = process.quality?.network;
    const rate = (process.network_received_bps ?? 0) + (process.network_transmitted_bps ?? 0);
    return displayProcessMetricValue(rate, quality, formatRate);
  }

  function processSummary(process: ProcessSample): string {
    return [
      "BatCave process snapshot",
      `Name: ${process.name}`,
      `PID: ${process.pid}`,
      `Parent PID: ${process.parent_pid ?? "--"}`,
      `Status: ${process.status}`,
      `CPU (one-core-equivalent): ${displayProcessMetricValue(process.cpu_percent, process.quality?.cpu, formatPercent)}`,
      `${presentation.memoryLabel}: ${residentMemoryValue(process, snapshot.environment.platform)}`,
      `${presentation.privateMemoryLabel}: ${privateMemoryValue(process, snapshot.environment.platform)}`,
      `Read/write I/O rate: ${displayProcessMetricValue(processIoRate(process, processRates), process.quality?.io, formatRate)}`,
      `Other I/O rate: ${displayProcessMetricValue(processOtherIoRate(process, processRates), process.quality?.other_io, formatOptionalRate)}`,
      `Network: ${processNetworkLabel(process)}`,
      `Access: ${accessLabel(process.access_state)}`,
      `Memory quality: ${metricQualityLabel(processMemoryQuality(process) as MetricQualityInfo | undefined, "Measured")}`,
      `Path: ${process.exe || "Path unavailable"}`,
      `Publication seq: ${snapshot.publication_seq}`,
      `Sample seq: ${snapshot.sample_seq}`,
      `Snapshot source: ${snapshot.source}`,
    ].join("\n");
  }

  function groupSummary(group: Extract<WorkloadDetail, { kind: "group" }>): string {
    return [
      "BatCave workload group snapshot",
      `Name: ${group.label}`,
      `Category: ${group.category}`,
      `Processes: ${group.process_count}`,
      `CPU (one-core-equivalent aggregate): ${displayGroupMetricValue(group.cpu_percent, group.quality.cpu, group.coverage.cpu, formatPercent)}`,
      `Memory: ${displayGroupMetricValue(group.memory_bytes, group.quality.memory, group.coverage.memory, formatBytes)}`,
      `Read/write I/O rate: ${displayGroupMetricValue(group.io_bps, group.quality.io, group.coverage.io, formatRate)}`,
      `Other I/O rate: ${displayGroupMetricValue(group.other_io_bps ?? 0, group.quality.other_io, group.coverage.other_io, formatRate)}`,
      `Network: ${displayGroupMetricValue(group.network_bps, group.quality.network, group.coverage.network, formatRate)}`,
      `Threads: ${displayGroupMetricValue(group.threads, group.quality.threads, group.coverage.threads, String)}`,
      `CPU coverage: ${group.coverage.cpu.available}/${group.coverage.cpu.total}`,
      `Memory coverage: ${group.coverage.memory.available}/${group.coverage.memory.total}`,
      `Read/write I/O coverage: ${group.coverage.io.available}/${group.coverage.io.total}`,
      `Other I/O coverage: ${group.coverage.other_io.available}/${group.coverage.other_io.total}`,
      `Network coverage: ${group.coverage.network.available}/${group.coverage.network.total}`,
      `Thread coverage: ${group.coverage.threads.available}/${group.coverage.threads.total}`,
      `Publication seq: ${snapshot.publication_seq}`,
      `Sample seq: ${snapshot.sample_seq}`,
      `Snapshot source: ${snapshot.source}`,
    ].join("\n");
  }

  function workloadSummary(workload: WorkloadDetail): string {
    return workload.kind === "group" ? groupSummary(workload) : processSummary(workload.process);
  }

  async function copySelectedWorkloadSummary(): Promise<void> {
    if (!selectedWorkload) {
      copyStatus = "No selected workload to copy.";
      return;
    }

    try {
      await navigator.clipboard.writeText(workloadSummary(selectedWorkload));
      copyStatus = "Workload summary copied.";
      commandError = "";
    } catch (error) {
      const active = document.activeElement instanceof HTMLElement ? document.activeElement : null;
      const textarea = document.createElement("textarea");
      textarea.value = workloadSummary(selectedWorkload);
      textarea.setAttribute("readonly", "");
      textarea.style.position = "fixed";
      textarea.style.opacity = "0";
      let copied = false;
      try {
        document.body.append(textarea);
        textarea.select();
        copied = document.execCommand("copy");
      } catch {
        copied = false;
      } finally {
        textarea.remove();
        active?.focus();
      }
      copyStatus = copied
        ? "Workload summary copied."
        : commandErrorMessage(error, "Unable to copy workload summary.");
      commandError = "";
    }
  }

  function maxRate(points: number[], fallback: number): number {
    return Math.max(fallback, Math.max(...points, 0) * 1.2);
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

  function ageLabel(timestampMs: number): string {
    const ageSeconds = Math.max(0, Math.round((Date.now() - timestampMs) / 1000));
    if (ageSeconds < 2) return "just now";
    if (ageSeconds < 60) return `${ageSeconds}s ago`;
    return timeLabel(timestampMs);
  }
</script>

<svelte:head>
  <title>BatCave Monitor</title>
</svelte:head>

<svelte:window onkeydown={handleAppKeydown} />

<AppShell {themeName} {accessibilityFixtureState}>
  <p class="visually-hidden" role="status" aria-live="polite" aria-atomic="true">{liveStatus}</p>
  <AppHeader
    {searchText}
    {isPaused}
    {pollState}
    {healthLabel}
    {healthTone}
    onSearch={setSearchText}
    onPaused={() => void setPaused(!isPaused)}
    onRefresh={() => void refreshNow()}
    onOpenSettings={() => (settingsOpen = true)}
    onOpenDiagnostics={() => (diagnosticsOpen = true)}
  />
  <SystemSummary
    brief={resourceBrief}
    resources={resourceSummaries}
    activeMode={detailMode}
    supportingText={systemSupportingText}
    {sampledAtLabel}
    windowLabel={resourceWindowLabel}
    activeValues={activeResource?.values ?? []}
    activeMax={activeResource?.max ?? 100}
    activeStroke={activeResource?.stroke ?? activeTheme.cpuStroke}
    activeFill={activeResource?.fill ?? activeTheme.cpuFill}
    leadingIconKind={leadingIdentity?.icon ?? "process"}
    leadingIconSrc={leadingProcess ? processIcons[leadingProcess.exe || leadingProcess.name] : undefined}
    onSelect={selectDetailMode}
  />
  <section class="triage-workspace">
    <ResourceRail
      resources={resourceSummaries}
      activeMode={detailMode}
      environmentLabel={`${presentation.platformName} · ${installKindLabel(snapshot.environment.install_kind)}`}
      sourceLabel={pollState === "fixture" ? "Layout fixture" : sourceLabel}
      diagnosticsLabel={railDiagnosticsLabel}
      onSelect={selectDetailMode}
      onOpenDiagnostics={() => (diagnosticsOpen = true)}
    />
    <main class="queue-workspace">
      <ProcessCommandBar
        {focusMode}
        {sortKey}
        {sortDirection}
        {commandError}
        {rankingUpdateAvailable}
        {focusOptions}
        {sortOptions}
        onFocus={setFocusMode}
        onSort={setSortKey}
        onToggleDirection={toggleSortDirection}
        onApplyRanking={applyPendingRanking}
      />
      <AttentionQueue
        processRows={processViewRows}
        totalProcessCount={snapshot.total_process_count || snapshot.system.process_count}
        {focusMode}
        {searchText}
        columns={visibleProcessColumns}
        {selectedWorkloadId}
        {sortKey}
        {sortDirection}
        {processIcons}
        {rankingUpdateAvailable}
        platform={snapshot.environment.platform}
        onSelect={selectProcess}
        onToggleSort={toggleSortKey}
        onInteractionChange={setQueueInteraction}
        onExpandedChange={setExpandedGroupCount}
      />
    </main>
    {#if !isCompactDetail || compactDetailOpen}
      <DetailPane
        subject={detailSubject}
        compact={isCompactDetail}
        onClose={closeCompactDetail}
        onShowSystem={() => selectDetailMode(detailMode)}
        {selectedWorkload}
        {selectedWorkloadIconKind}
        {selectedWorkloadIconSrc}
        {processHistory}
        {processRates}
        {processReadRate}
        {processWriteRate}
        {processIcons}
        {copyStatus}
        {activeTheme}
        {presentation}
        {processNetworkLabel}
        onCopy={() => void copySelectedWorkloadSummary()}
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
    adminAvailable={snapshot.environment.admin_mode_available}
    processStatus={processElevationLabel(snapshot.environment)}
    adminStatus={adminStatusLabel()}
    adminNote={privilegedCollectionNote(snapshot.admin_mode)}
    adminAction={privilegedCollectionAction(
      snapshot.environment.admin_mode_available,
      snapshot.admin_mode,
    )}
    dataDirectory={snapshot.environment.data_directory}
    {presentation}
    onClose={() => (settingsOpen = false)}
    onTheme={setTheme}
    onPollInterval={(interval) => void setPollInterval(interval)}
    onHistoryLimit={setHistoryPointLimit}
    onAdminMode={(enabled) => void setAdminMode(enabled)}
    {updateStatus}
    {updateMessage}
    onCheckForUpdates={() => void checkForStableUpdate()}
    onInstallUpdate={() => void installStableUpdate()}
    onResetHistory={resetHistory}
  />
</AppShell>
