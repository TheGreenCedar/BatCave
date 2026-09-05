<script lang="ts">
  import { FixtureInspectionArchive, InspectionRequestGate, getWorkloadInspection, type WorkloadInspection } from "./lib/workloadInspection";
  import { invoke } from "@tauri-apps/api/core";
  import { check, type Update } from "@tauri-apps/plugin-updater";
  import MagnifyingGlass from "phosphor-svelte/lib/MagnifyingGlass";
  import { onMount } from "svelte";
  import fixtureProcessIcon from "../src-tauri/icons/64x64.png";
  import DetailPane from "./lib/components/context/DetailPane.svelte";
  import type { DetailMode, ResourceSummaryOption } from "./lib/components/metrics/types";
  import Overview from "./lib/components/overview/Overview.svelte";
  import AttentionQueue from "./lib/components/processes/AttentionQueue.svelte";
  import AppHeader from "./lib/components/shell/AppHeader.svelte";
  import AppShell from "./lib/components/shell/AppShell.svelte";
  import DiagnosticsDrawer from "./lib/components/shell/DiagnosticsDrawer.svelte";
  import ProcessCommandBar from "./lib/components/shell/ProcessCommandBar.svelte";
  import SettingsDrawer from "./lib/components/shell/SettingsDrawer.svelte";
  import {
    resolveAccessibilityFixtureState,
    type AccessibilityFixtureState,
  } from "./lib/accessibilityFixtures";
  import {
    buildResourceBrief,
    resolveContributorProcess,
    type CollectionState,
  } from "./lib/cockpit";
  import { uniqueWarningCount } from "./lib/diagnostics";
  import {
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
    logicalCpuMetricQuality,
    metricQualityLabel,
    processMemoryQuality,
    processFindingLabel,
  } from "./lib/format";
  import { makeFixtureSnapshot } from "./lib/fixtures";
  import {
    NarrativeController,
    buildNarrativeFactPacket,
    admittedNarrativeCandidates,
    renderNarrative,
    defaultNarrativeCapability,
    isNarrativeRelevant,
    makeNarrativeInvocation,
    narrativeCapabilityExplanation,
    narrativeRelevanceKey,
    type AcceptedNarrative,
    type NarrativeFactPacket,
  } from "./lib/narratives";
  import { buildOverviewStatus, leadingOverviewRows } from "./lib/overview";
  import { buildTelemetryPresentation, metricPresentation } from "./lib/telemetryPresentation";
  import {
    platformPresentation,
    privateMemoryValue,
    residentMemoryValue,
  } from "./lib/platformPresentation";
  import {
    defaultSortDirection,
    focusOptions,
    advanceProcessRanking,
    nextSortDirection,
    processColumns,
    processIdentity,
    processNeedsAttention,
    prepareProcessViewRows,
    processSelectionKey,
    processViewRowKey,
    sortColumnForKey,
    sortKeyForColumn,
    sortOptions,
    shouldHoldProcessOrder,
    type FocusMode,
    type SortKey,
  } from "./lib/process";
  import {
    buildResolvedProcessIconCatalog,
    processIconFamily,
    processIconKey,
    resolvedProcessIcon,
    type ResolvedProcessIcon,
    type ResolvedProcessIconCatalog,
  } from "./lib/processIcons";
  import {
    hasNewRuntimeSample,
    makeDefaultRuntimeQuery,
    makeEmptySnapshot,
    shouldApplyRuntimePublication,
    shouldPollRuntime,
  } from "./lib/runtimeSnapshot";
  import { startRuntimePolling } from "./lib/runtimePolling";
  import { observeDesktopPublication } from "./lib/desktopProbe";
  import {
    boundedPercent,
    combineSeries,
    emptyTrendState,
    maxRate,
    nextSystemHistory,
    percentage,
    trimSystemHistory,
  } from "./lib/telemetryHistory";
  import { AcceptedRuntimeControls } from "./lib/runtimeControls";
  import {
    dispatchAutomaticRuntimeHydration,
    planAutomaticRuntimeFocusHydration,
  } from "./lib/runtimeHydration";
  import { runtimeSurfaceMode } from "./lib/runtimeMode";
  import {
    chartPalettes,
    defaultThemePreference,
    parseThemePreference,
    resolveThemePreference,
    serializeResolvedTheme,
    serializeThemePreference,
    themeFamilyOptions,
    themeModeOptions,
    themeStorageKey,
    type ResolvedThemeMode,
    type ResolvedThemeName,
    type ThemeFamily,
    type ThemeModePreference,
    type ThemePreference,
  } from "./lib/themes";
  import {
    commandErrorMessage,
    cancelLocalNarrativeGeneration,
    cancelNarrativeModelDownload,
    downloadNarrativeModel,
    generateLocalNarrative,
    getNarrativeCapability,
    getNarrativeFactDigest,
    getNarrativePreferences,
    getRuntimeProcessIcons,
    observeAcceptedRuntimePublication,
    ProtocolMismatchError,
    readNativeSnapshot,
    refreshRuntime,
    runtimeMutationAllowed,
    setRuntimePaused,
    setRuntimeProcessQuery,
    setRuntimeSampleInterval,
    setRuntimeUiPreferences,
    setEnhancedNarratives,
    syncRuntimeAppearance,
    type RuntimeQueryWriteIntent,
  } from "./lib/tauriBridge";
  import type {
    KernelPoolTag,
    MetricQualityInfo,
    ProcessSample,
    ProcessViewRow,
    RuntimeQuery,
    RuntimeSnapshot,
    SortDirection,
    WorkloadDetail,
  } from "./lib/types";
  import { UiPreferencePersistenceSequence } from "./lib/uiPreferencePersistence";
  import {
    StableUpdateController,
    type StableUpdateState,
  } from "./lib/stableUpdate";
  import { RUNTIME_PROTOCOL_VERSION } from "./lib/generated/runtime-protocol-v4";
  import type { ProtocolMismatchView } from "./lib/protocol/runtimeProtocol";

  const historyPointOptions = [30, 72, 180, 360] as const;
  type HistoryPointLimit = (typeof historyPointOptions)[number];
  type CommandErrorSurface = "global" | "settings" | "workload";
  type AppView = "overview" | "explore";

  const pollIntervals = [500, 1000, 2000] as const;
  const historyStorageKey = "batcave.monitor.history-points";
  const uiPreferencePersistence = new UiPreferencePersistenceSequence();
  const acceptedRuntimeControls = new AcceptedRuntimeControls(makeDefaultRuntimeQuery(), 1000);
  const stableUpdateController = new StableUpdateController<Update>(() =>
    check({ timeout: 15_000 }),
  );
  const narrativeController = new NarrativeController(async (invocation, signal) => {
    if (signal.aborted) throw new Error("narrative_cancelled");
    const result = await generateLocalNarrative(invoke, invocation.request, invocation.facts);
    if (signal.aborted || !result) throw new Error("narrative_unavailable");
    return result;
  });
  const browserFixturePlatform = "macos" as const;
  const accessibilityFixtureState = resolveAccessibilityFixtureState(
    typeof window === "undefined" ? "" : window.location.search,
    import.meta.env.DEV,
    hasTauriRuntime(),
  );

  let fixtureTick = 0;
  let snapshot: RuntimeSnapshot = makeEmptySnapshot();
  let selectedWorkloadId = "";
  let hasAutoSelectedWorkload = false;
  let activeView: AppView = "overview";
  let detailSubject: "process" | "system" = "system";
  let pollState: "starting" | "native" | "fixture" | "error" = "starting";
  let lastError = "";
  let commandError = "";
  let commandErrorSurface: CommandErrorSurface = "global";
  let protocolMismatch: ProtocolMismatchView | null = null;
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
  let overviewResource: DetailMode = "cpu";
  let themePreference: ThemePreference = defaultThemePreference;
  let systemThemeMode: ResolvedThemeMode = "dark";
  let synchronizedThemeName: ResolvedThemeName | null = null;
  let historyPointLimit: HistoryPointLimit = 72;
  let history = emptyTrendState();
  let inspection: WorkloadInspection | null = null;
  let inspectionLoading = false;
  let inspectionError = "";
  const inspectionGate = new InspectionRequestGate();
  const fixtureInspection = new FixtureInspectionArchive();
  let nativeProcessIcons: Record<string, string> = {};
  let processIcons: ResolvedProcessIconCatalog = {};
  let requestedProcessIcons = new Set<string>();
  let resourceSummaries: ResourceSummaryOption[] = [];
  let displayProcessRows: ProcessViewRow[] = [];
  let pendingProcessRows: ProcessViewRow[] | null = null;
  let queueInteracting = false;
  let rankingUpdateAvailable = false;
  let settingsOpen = false;
  let diagnosticsOpen = false;
  let isCompactDetail = false;
  let compactDetailOpen = false;
  let healthTone: "healthy" | "warning" | "danger" = "healthy";
  let collectionState: CollectionState = "live";
  let forceRankingRefresh = false;
  let runtimeQueryRequestSeq = 0;
  let runtimeCadenceRequestSeq = 0;
  let pendingCadenceRequestSeq = 0;
  let searchDebounceId: number | undefined;
  let updateStatus: "idle" | "checking" | "available" | "current" | "installing" | "error" = "idle";
  let updateMessage = "Checks only when you ask.";
  let enhancedNarratives = false;
  let narrativeCapability = defaultNarrativeCapability;
  let narrativeSettingsStatus = "";
  let narrativeModelAction: "idle" | "downloading" | "cancelling" = "idle";
  let overviewNarrative: AcceptedNarrative | null = null;
  let workloadNarrative: AcceptedNarrative | null = null;
  let narrativeInitialRequestDone = false;
  let narrativeCapabilityPollId: number | undefined;
  let narrativeEpoch = 0;
  let narrativeCancellation: Promise<void> = Promise.resolve();

  $: resolvedTheme = resolveThemePreference(themePreference, systemThemeMode);
  $: resolvedThemeName = serializeResolvedTheme(resolvedTheme);
  $: if (resolvedThemeName !== synchronizedThemeName) {
    synchronizedThemeName = resolvedThemeName;
    if (hasTauriRuntime()) {
      void syncRuntimeAppearance(invoke, resolvedThemeName, (message) => console.warn(message));
    }
  }
  $: activeTheme = chartPalettes[resolvedThemeName];
  $: presentation = platformPresentation(snapshot.environment);
  $: memoryPercent = percentage(snapshot.system.memory_used_bytes, snapshot.system.memory_total_bytes);
  $: swapPercent = percentage(
    snapshot.system.swap_used_bytes ?? 0,
    snapshot.system.swap_total_bytes ?? 0,
  );
  $: processViewRows = displayProcessRows;
  $: iconProcesses = snapshot.overview_rows.flatMap((row) => row.kind === "process" ? [row.detail.process] : []);
  $: processIcons = buildResolvedProcessIconCatalog([...iconProcesses, ...snapshot.processes], nativeProcessIcons);
  $: filteredProcesses = processViewRows.flatMap((row) =>
    row.kind === "process" ? [row.detail.process] : [],
  );
  $: selectedRow = inspection?.stable_id === selectedWorkloadId ? inspection.row : null;
  $: inspectionCurrent = inspection?.stable_id === selectedWorkloadId && inspection.status === "current" && inspection.sample_seq === snapshot.sample_seq && collectionState === "live" && !inspectionLoading && !inspectionError;
  $: void refreshInspection(selectedWorkloadId, historyPointLimit, snapshot.publication_seq);
  $: selectedWorkload = selectedRow?.detail ?? null;
  $: selectedProcess =
    selectedWorkload?.kind === "process" ? selectedWorkload.process : null;
  $: selectedWorkloadIconKind =
    ((selectedRow?.icon_kind ?? "process") as import("./lib/process").ProcessIconKind);
  $: selectedWorkloadIcon =
    selectedRow?.kind === "group" && selectedRow.icon_source
      ? resolvedProcessIcon(processIcons, selectedRow.icon_source)
      : selectedProcess
        ? resolvedProcessIcon(processIcons, selectedProcess.exe || selectedProcess.name)
        : ({ origin: "fallback" } satisfies ResolvedProcessIcon);
  $: selectedWorkloadIconSrc = selectedWorkloadIcon.src;
  $: selectedWorkloadIconMatched = selectedWorkloadIcon.origin === "name_match";
  $: sourceLabel =
    snapshot.source === "batcave_runtime" ||
    snapshot.source === "tauri_runtime" ||
    snapshot.source === "tauri_sysinfo"
      ? "native telemetry"
      : "fixture demo";
  $: systemQuality = snapshot.system.quality ?? {};
  $: visibleProcessColumns = processColumns
    .filter((column) => column.key !== "attention")
    .map((column) =>
      column.key === "memory" ? { ...column, label: presentation.memoryLabel } : column,
    );
  $: memoryAccounting = snapshot.system.memory_accounting;
  $: topKernelPoolTags = topPoolTags(memoryAccounting?.kernel_pool_tags);
  $: blockedProcessCount =
    memoryAccounting?.denied_process_count ?? snapshot.processes.filter((process) => process.access_state === "denied").length;
  $: adminStatus = privilegedCollectionLabel(snapshot.admin_mode, blockedProcessCount);
  $: diskReadRate = snapshot.system.disk_read_bps;
  $: diskWriteRate = snapshot.system.disk_write_bps;
  $: networkDownRate = snapshot.system.network_received_bps;
  $: networkUpRate = snapshot.system.network_transmitted_bps;
  $: diskTotalHistory = combineSeries(history.diskRead, history.diskWrite);
  $: networkTotalHistory = combineSeries(history.netRx, history.netTx);
  $: diskScaleMax = maxRate(diskTotalHistory, 1_000_000);
  $: networkScaleMax = maxRate(networkTotalHistory, 750_000);
  $: processReadRate = selectedProcess?.io_read_bps ?? 0;
  $: processWriteRate = selectedProcess?.io_write_bps ?? 0;
  $: void hydrateProcessIcons([...overviewRows, ...processViewRows], [...iconProcesses, ...filteredProcesses], activeView === "overview" ? overviewPrimaryProcess : selectedProcess);
  $: coreLoads = history.cores.flatMap((core, index) =>
    core.length > 0 ? [{ index, load: currentCoreLoad(core), trend: core }] : [],
  );
  $: corePeak = Math.max(...coreLoads.map((core) => core.load), 0);
  $: coreMinimum = coreLoads.length > 0 ? Math.min(...coreLoads.map((core) => core.load)) : 0;
  $: coreSpread = Math.max(0, corePeak - coreMinimum);
  $: hotCoreCount = coreLoads.filter((core) => core.load >= 75).length;
  $: busyCoreCount = coreLoads.filter((core) => core.load >= 45).length;
  $: telemetry = buildTelemetryPresentation(snapshot, pollState === "starting" ? "starting" : pollState === "error" ? "stale" : isPaused ? "paused" : "live");
  $: collectionState = telemetry.state;
  $: overviewNarrativeCopy = collectionState === "live" && enhancedNarratives && overviewPrimaryProcess
    ? renderNarrative(overviewNarrative, processNarrativeFacts(overviewPrimaryProcess, overviewResource === "disk" ? "io" : overviewResource, "top_contributor"),
      "overview_contributor", overviewPrimaryBrief.leadingProcessId ?? undefined)
    : null;
  $: overviewContributorCopy = overviewNarrativeCopy ?? overviewPrimaryBrief.contributorStatusLabel;
  $: workloadNarrativeCopy = inspectionCurrent && enhancedNarratives && selectedProcess && selectedWorkload?.kind === "process"
    ? renderNarrative(workloadNarrative, processNarrativeFacts(selectedProcess, leadingNarrativeResource(selectedProcess), "notable"),
      "workload_insight", selectedWorkload.workload_id)
    : null;
  $: selectedWorkloadInsight =
    collectionState === "live" && selectedProcess && processHasNotableFinding(selectedProcess)
      ? workloadNarrativeCopy ?? deterministicProcessFinding(selectedProcess)
      : null;
  $: limitationCount =
    uniqueWarningCount(snapshot.warnings) || snapshot.health.collector_warning_count;
  $: overviewStatus = buildOverviewStatus(
    snapshot,
    pollState === "starting" ? "starting" : collectionState,
    limitationCount,
    overviewResource,
  );
  $: overviewRows = leadingOverviewRows(snapshot.overview_rows, overviewResource, 5);
  $: overviewPrimaryBrief = buildResourceBrief(snapshot, overviewResource, { memoryPercent, diskRate: diskReadRate + diskWriteRate, networkRate: networkDownRate + networkUpRate }, collectionState);
  $: overviewPrimaryProcess = resolveContributorProcess(snapshot, overviewPrimaryBrief.leadingProcessId);
  $: overviewPrimaryIdentity = overviewPrimaryProcess ? processIdentity(overviewPrimaryProcess) : null;
  $: overviewPrimaryIcon = resolvedProcessIcon(processIcons, overviewPrimaryProcess ? processIconKey(overviewPrimaryProcess) : undefined);
  $: overviewPrimaryMetric = resourceSummaries.find((resource) => resource.mode === overviewResource);
  $: healthTone = telemetry.tone;
  $: healthLabel = telemetry.label;
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
      ? !metricCanDisplay(systemQuality.cpu)
        ? cpuDetailValue
        : `${cpuDetailValue} machine total`
      : detailMode === "memory"
        ? !metricCanDisplay(systemQuality.memory)
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
      statusLabel: resourceQualityStatus(systemQuality.cpu, collectionState),
      shortStatusLabel: resourceQualityStatus(systemQuality.cpu, collectionState),
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
      statusLabel: resourceQualityStatus(systemQuality.memory, collectionState),
      shortStatusLabel: resourceQualityStatus(systemQuality.memory, collectionState),
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
      statusLabel: resourceQualityStatus(systemQuality.disk, collectionState),
      shortStatusLabel: resourceQualityStatus(systemQuality.disk, collectionState),
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
      statusLabel: resourceQualityStatus(systemQuality.network, collectionState),
      shortStatusLabel: resourceQualityStatus(systemQuality.network, collectionState),
      values: networkTotalHistory,
      max: networkScaleMax,
      stroke: activeTheme.networkDownStroke,
      fill: activeTheme.networkDownFill,
    },
  ];

  onMount(() => {
    let stopPolling: (() => void) | undefined;
    let detailFocusFrame: number | undefined;
    const systemThemeQuery = window.matchMedia("(prefers-color-scheme: light)");
    const compactDetailQuery = window.matchMedia("(max-width: 1279px)");
    const savedTheme = window.localStorage.getItem(themeStorageKey);
    const savedHistoryPointLimit = Number(window.localStorage.getItem(historyStorageKey));

    systemThemeMode = systemThemeQuery.matches ? "light" : "dark";
    isCompactDetail = compactDetailQuery.matches;

    const savedThemePreference = parseThemePreference(savedTheme);
    if (savedThemePreference) {
      themePreference = savedThemePreference;
    } else if (savedTheme !== null) {
      window.localStorage.removeItem(themeStorageKey);
    }

    if (isHistoryPointLimit(savedHistoryPointLimit)) {
      historyPointLimit = savedHistoryPointLimit;
    } else if (window.localStorage.getItem(historyStorageKey) !== null) {
      window.localStorage.removeItem(historyStorageKey);
    }

    if (runtimeMode() === "fixture") {
      if (accessibilityFixtureState) {
        fixtureTick = 8;
        const next = makeFixtureSnapshot(
          fixtureTick,
          currentRuntimeQuery(),
          accessibilityFixtureState === "group" || accessibilityFixtureState === "diagnostics"
            ? "windows"
            : browserFixturePlatform,
          "compact",
        );
        prepareAccessibilityFixture(next, accessibilityFixtureState);
        ingest(next);
        applyAccessibilityFixtureSelection(next, accessibilityFixtureState);
        if (accessibilityFixtureState === "exited") {
          const exited = structuredClone(next);
          exited.publication_seq += 1;
          exited.sample_seq += 1;
          exited.sampled_at_ms = (exited.sampled_at_ms ?? exited.published_at_ms) + 1000;
          exited.published_at_ms = exited.sampled_at_ms;
          exited.processes = [];
          exited.process_view_rows = [];
          exited.overview_rows = [];
          ingest(exited);
        }
        activeView = ["process", "group", "compact", "exited"].includes(accessibilityFixtureState)
          ? "explore"
          : "overview";
      } else {
        ingest(makeFixtureSnapshot(fixtureTick, currentRuntimeQuery(), browserFixturePlatform, accessibilityFixtureState ? "compact" : "dense"));
      }
    } else if (runtimeMode() === "unavailable") {
      lastError = "BatCave telemetry requires the native desktop runtime.";
      pollState = "error";
      ingest(makeEmptySnapshot(lastError));
    }

    void hydrateNarrativeState();

    if (!accessibilityFixtureState) {
      stopPolling = startRuntimePolling({
        initialDelayMs: 120,
        intervalMs: () => pollIntervalMs,
        poll: async () => {
          if (shouldPollRuntime(isPaused, hasTauriRuntime())) {
            const next = await readSnapshot();
            ingest(next);
          }
        },
        scheduler: {
          setTimeout: (callback, delayMs) => window.setTimeout(callback, delayMs),
          clearTimeout: (timeoutId) => window.clearTimeout(timeoutId),
        },
      });
    }

    const handleSystemThemeChange = (event: MediaQueryListEvent) => {
      systemThemeMode = event.matches ? "light" : "dark";
    };

    const handleCompactDetailChange = (event: MediaQueryListEvent) => {
      if (isCompactDetail === event.matches) return;
      if (detailFocusFrame !== undefined) {
        window.cancelAnimationFrame(detailFocusFrame);
        detailFocusFrame = undefined;
      }
      const active = document.activeElement;
      const focusWasInDetail =
        active instanceof HTMLElement && active.closest("#detail-pane") !== null;
      const shouldRestoreLogicalFocus = isCompactDetail
        ? compactDetailOpen
        : focusWasInDetail;
      isCompactDetail = event.matches;
      compactDetailOpen = false;
      if (shouldRestoreLogicalFocus) {
        detailFocusFrame = window.requestAnimationFrame(() => {
          detailFocusFrame = window.requestAnimationFrame(() => {
            detailFocusFrame = undefined;
            focusCurrentDetailControl();
          });
        });
      }
    };

    systemThemeQuery.addEventListener("change", handleSystemThemeChange);
    compactDetailQuery.addEventListener("change", handleCompactDetailChange);

    return () => {
      systemThemeQuery.removeEventListener("change", handleSystemThemeChange);
      compactDetailQuery.removeEventListener("change", handleCompactDetailChange);
      stopPolling?.();
      inspectionGate.clear();
      if (searchDebounceId !== undefined) {
        window.clearTimeout(searchDebounceId);
      }
      if (detailFocusFrame !== undefined) {
        window.cancelAnimationFrame(detailFocusFrame);
      }
      if (narrativeCapabilityPollId !== undefined) {
        window.clearTimeout(narrativeCapabilityPollId);
      }
      narrativeController.dispose();
      if (hasTauriRuntime()) void cancelLocalNarrativeGeneration(invoke).catch(() => {});
    };
  });

  async function readSnapshot(): Promise<RuntimeSnapshot> {
    const mode = runtimeMode();
    if (mode === "fixture") {
      fixtureTick += 1;
      pollState = "fixture";
      lastError = "";
      protocolMismatch = null;
      return makeFixtureSnapshot(fixtureTick, currentRuntimeQuery(), browserFixturePlatform, accessibilityFixtureState ? "compact" : "dense");
    }
    if (mode === "unavailable") {
      const message = "BatCave telemetry requires the native desktop runtime.";
      pollState = "error";
      lastError = message;
      return makeEmptySnapshot(message);
    }

    const next = await readNativeSnapshot(invoke, {
      currentSnapshot: snapshot,
      emptySnapshot: makeEmptySnapshot,
      hasNativeSnapshot,
    });
    pollState = next.ok ? "native" : "error";
    lastError = next.error;
    if (next.mismatch) {
      enterProtocolMismatch(next.mismatch, next.snapshot);
    } else if (next.ok) {
      protocolMismatch = null;
    }
    hasNativeSnapshot = next.ok || hasNativeSnapshot;
    return next.snapshot;
  }

  function hasTauriRuntime(): boolean {
    return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
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

    if (state === "diagnostics") {
      next.environment.process_elevation = "standard";
      next.admin_mode = {
        state: "active",
        source: "collector_service",
        detail: null,
        last_success_at_ms: next.sampled_at_ms,
        collector_service: {
          state: "active",
          release_identity: { ...next.environment.release_identity },
          service_version: next.environment.release_identity.app_version,
          negotiated_protocol_version: 3,
          minimum_desktop_version: null,
          instance_id: "accessibility-fixture-service",
          last_connected_at_ms: next.published_at_ms,
          detail: null,
        },
      };
    }

    seedAccessibilityProcessIcon(next);
  }

  function seedAccessibilityProcessIcon(next: RuntimeSnapshot): void {
    const donor = next.overview_rows.flatMap((row) =>
      row.kind === "process" ? [row.detail.process] : [],
    ).find(
      (process) => processIconFamily(process.name) === "fixtureworker",
    );
    if (donor) nativeProcessIcons = { [processIconKey(donor)]: fixtureProcessIcon };
  }

  function applyAccessibilityFixtureSelection(
    next: RuntimeSnapshot,
    state: AccessibilityFixtureState,
  ): void {
    settingsOpen = state === "settings";
    diagnosticsOpen = state === "diagnostics";
    compactDetailOpen = false;

    if (state === "process" || state === "compact" || state === "exited") {
      const processRow = next.process_view_rows.find(
        (row) =>
          row.kind === "process" &&
          !row.is_grouped &&
          processIconFamily(row.detail.process.name) === "fixtureworker" &&
          !nativeProcessIcons[processIconKey(row.detail.process)],
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

  function runtimeMode() {
    return runtimeSurfaceMode(hasTauriRuntime(), import.meta.env.DEV);
  }

  async function hydrateNarrativeState(): Promise<void> {
    if (runtimeMode() !== "native") return;
    try {
      const [preferences, capability] = await Promise.all([
        getNarrativePreferences(invoke),
        getNarrativeCapability(invoke),
      ]);
      enhancedNarratives = preferences.enhanced_narratives;
      narrativeCapability = capability;
      narrativeSettingsStatus = "";
      window.queueMicrotask(maybeRequestInitialNarrative);
    } catch (error) {
      enhancedNarratives = false;
      narrativeSettingsStatus = commandErrorMessage(
        error,
        "Enhanced explanations are unavailable. Deterministic explanations remain active.",
      );
    }
  }

  async function refreshNarrativeCapability(): Promise<void> {
    if (runtimeMode() !== "native") return;
    try {
      narrativeCapability = await getNarrativeCapability(invoke);
    } catch {
      // Capability refresh is optional; the last known state and deterministic copy remain valid.
    }
  }

  async function setEnhancedNarrativePreference(enabled: boolean): Promise<void> {
    cancelNarrativeWork();
    enhancedNarratives = enabled;
    narrativeSettingsStatus = "";
    if (runtimeMode() !== "native") {
      if (enabled) {
        narrativeSettingsStatus =
          "Enhanced explanations require the native app. Deterministic explanations remain active.";
      }
      return;
    }
    try {
      const persisted = await setEnhancedNarratives(invoke, enabled);
      enhancedNarratives = persisted.enhanced_narratives;
      if (enhancedNarratives) void requestCurrentSurfaceNarrative();
    } catch (error) {
      enhancedNarratives = !enabled;
      narrativeSettingsStatus = commandErrorMessage(
        error,
        "Unable to save enhanced explanation preferences.",
      );
    }
  }

  async function startNarrativeModelDownload(): Promise<void> {
    if (runtimeMode() !== "native" || narrativeModelAction !== "idle") return;
    narrativeModelAction = "downloading";
    narrativeSettingsStatus = "Downloading the local model…";
    scheduleNarrativeCapabilityPoll();
    try {
      narrativeCapability = await downloadNarrativeModel(invoke);
      if (narrativeCapability.download_state === "downloading") {
        narrativeSettingsStatus = "Downloading the local model…";
        scheduleNarrativeCapabilityPoll();
      } else {
        narrativeModelAction = "idle";
        stopNarrativeCapabilityPoll();
        narrativeSettingsStatus = narrativeCapability.availability === "available"
          ? "The local model is ready."
          : narrativeCapabilityExplanation(narrativeCapability);
      }
    } catch (error) {
      narrativeModelAction = "idle";
      stopNarrativeCapabilityPoll();
      narrativeSettingsStatus = commandErrorMessage(error, "The local model download failed.");
    }
  }

  async function stopNarrativeModelDownload(): Promise<void> {
    if (runtimeMode() !== "native" || narrativeModelAction !== "downloading") return;
    narrativeModelAction = "cancelling";
    narrativeSettingsStatus = "Cancelling the download…";
    try {
      narrativeCapability = await cancelNarrativeModelDownload(invoke);
      if (narrativeCapability.download_state === "downloading") {
        narrativeSettingsStatus = "Cancelling the download…";
        scheduleNarrativeCapabilityPoll();
      } else {
        narrativeModelAction = "idle";
        stopNarrativeCapabilityPoll();
        narrativeSettingsStatus = narrativeCapability.availability === "available"
          ? "The local model is ready."
          : "Local model download cancelled.";
      }
    } catch (error) {
      narrativeSettingsStatus = commandErrorMessage(error, "Unable to cancel the download.");
      if (narrativeCapability.download_state === "downloading") {
        narrativeModelAction = "downloading";
        scheduleNarrativeCapabilityPoll();
      } else {
        narrativeModelAction = "idle";
        stopNarrativeCapabilityPoll();
      }
    }
  }

  function scheduleNarrativeCapabilityPoll(): void {
    stopNarrativeCapabilityPoll();
    narrativeCapabilityPollId = window.setTimeout(async () => {
      narrativeCapabilityPollId = undefined;
      if (narrativeModelAction === "idle") return;
      const wasCancelling = narrativeModelAction === "cancelling";
      await refreshNarrativeCapability();
      if (narrativeCapability.download_state === "downloading") {
        narrativeSettingsStatus = wasCancelling
          ? "Cancelling the download…"
          : "Downloading the local model…";
        scheduleNarrativeCapabilityPoll();
      } else {
        narrativeModelAction = "idle";
        narrativeSettingsStatus = narrativeCapability.availability === "available"
          ? "The local model is ready."
          : wasCancelling
            ? "Local model download cancelled."
            : narrativeCapabilityExplanation(narrativeCapability);
      }
    }, 750);
  }

  function stopNarrativeCapabilityPoll(): void {
    if (narrativeCapabilityPollId !== undefined) {
      window.clearTimeout(narrativeCapabilityPollId);
      narrativeCapabilityPollId = undefined;
    }
  }

  function cancelNarrativeWork(): void {
    narrativeEpoch += 1;
    narrativeController.cancel();
    overviewNarrative = null;
    workloadNarrative = null;
    if (runtimeMode() === "native") {
      narrativeCancellation = narrativeCancellation
        .then(() => cancelLocalNarrativeGeneration(invoke))
        .catch(() => {});
    }
  }

  async function requestCurrentSurfaceNarrative(): Promise<void> {
    if (!enhancedNarratives || collectionState !== "live" || narrativeCapability.availability !== "available") return;
    if (activeView === "overview") {
      await requestOverviewNarrative();
    } else if (detailSubject === "process") {
      await requestWorkloadNarrative();
    }
  }

  function maybeRequestInitialNarrative(): void {
    if (
      narrativeInitialRequestDone ||
      !enhancedNarratives ||
      narrativeCapability.availability !== "available"
    ) {
      return;
    }
    const hasNarrativeSubject = activeView === "overview"
      ? currentOverviewNarrativeContext() !== null
      : detailSubject === "process" && currentWorkloadNarrativeContext() !== null;
    if (!hasNarrativeSubject) return;
    narrativeInitialRequestDone = true;
    void requestCurrentSurfaceNarrative();
  }

  async function requestOverviewNarrative(): Promise<void> {
    await narrativeCancellation;
    const epoch = narrativeEpoch;
    const context = currentOverviewNarrativeContext();
    if (!context || runtimeMode() !== "native" || !enhancedNarratives || admittedNarrativeCandidates(context.facts, "overview_contributor").length < 2) return;
    const relevanceKey = narrativeRelevanceKey(context.facts);
    const publicationSeq = snapshot.publication_seq;
    let factDigest = "";
    try {
      factDigest = await getNarrativeFactDigest(invoke, context.facts);
    } catch {
      return;
    }
    const currentBeforeGeneration = currentOverviewNarrativeContext();
    if (
      !currentBeforeGeneration ||
      epoch !== narrativeEpoch ||
      narrativeRelevanceKey(currentBeforeGeneration.facts) !== relevanceKey ||
      currentBeforeGeneration.subjectStableId !== context.subjectStableId
    ) {
      return;
    }
    const invocation = makeNarrativeInvocation(
      "overview_contributor",
      publicationSeq,
      context.facts,
      context.subjectStableId,
      factDigest,
    );
    const result = await narrativeController.request(invocation);
    const current = currentOverviewNarrativeContext();
    if (
      result &&
      epoch === narrativeEpoch &&
      activeView === "overview" &&
      current &&
      isNarrativeRelevant(
        result,
        current.facts,
        "overview_contributor",
        current.subjectStableId,
      )
    ) {
      overviewNarrative = result;
    }
  }

  async function requestWorkloadNarrative(): Promise<void> {
    await narrativeCancellation;
    const epoch = narrativeEpoch;
    const context = currentWorkloadNarrativeContext();
    if (!context || runtimeMode() !== "native" || !enhancedNarratives || admittedNarrativeCandidates(context.facts).length < 2) return;
    const relevanceKey = narrativeRelevanceKey(context.facts);
    const publicationSeq = snapshot.publication_seq;
    let factDigest = "";
    try {
      factDigest = await getNarrativeFactDigest(invoke, context.facts);
    } catch {
      return;
    }
    const currentBeforeGeneration = currentWorkloadNarrativeContext();
    if (
      !currentBeforeGeneration ||
      epoch !== narrativeEpoch ||
      narrativeRelevanceKey(currentBeforeGeneration.facts) !== relevanceKey ||
      currentBeforeGeneration.subjectStableId !== context.subjectStableId
    ) {
      return;
    }
    const invocation = makeNarrativeInvocation(
      "workload_insight",
      publicationSeq,
      context.facts,
      context.subjectStableId,
      factDigest,
    );
    const result = await narrativeController.request(invocation);
    const current = currentWorkloadNarrativeContext();
    if (
      result &&
      epoch === narrativeEpoch &&
      activeView === "explore" &&
      current &&
      isNarrativeRelevant(result, current.facts, "workload_insight", current.subjectStableId)
    ) {
      workloadNarrative = result;
    }
  }

  function currentOverviewNarrativeContext(): {
    facts: NarrativeFactPacket;
    subjectStableId: string;
  } | null {
    if (collectionState !== "live" || !overviewPrimaryProcess || !overviewPrimaryBrief.leadingProcessId) return null;
    return {
      facts: processNarrativeFacts(overviewPrimaryProcess, overviewResource === "disk" ? "io" : overviewResource, "top_contributor"),
      subjectStableId: overviewPrimaryBrief.leadingProcessId,
    };
  }

  function currentWorkloadNarrativeContext(): {
    facts: NarrativeFactPacket;
    subjectStableId: string;
  } | null {
    if (
      collectionState !== "live" ||
      !inspectionCurrent ||
      selectedWorkload?.kind !== "process" ||
      !processHasNotableFinding(selectedWorkload.process)
    ) {
      return null;
    }
    return {
      facts: processNarrativeFacts(
        selectedWorkload.process,
        leadingNarrativeResource(selectedWorkload.process),
        "notable",
      ),
      subjectStableId: selectedWorkload.workload_id,
    };
  }

  function processNarrativeFacts(
    process: ProcessSample,
    leadingResource: NarrativeFactPacket["leading_resource"],
    rankingState: NarrativeFactPacket["ranking_state"],
  ): NarrativeFactPacket {
    return buildNarrativeFactPacket({
      displayName: process.name,
      category: processIdentity(process).group,
      cpuPercent: process.cpu_percent,
      memoryBytes: process.memory_bytes,
      ioBytesPerSecond: (process.io_read_bps + process.io_write_bps),
      networkBytesPerSecond:
        (process.network_received_bps ?? 0) + (process.network_transmitted_bps ?? 0),
      leadingResource,
      rankingState,
      measurementLimitations: processMeasurementLimitations(process),
    });
  }

  function processMeasurementLimitations(
    process: ProcessSample,
  ): NarrativeFactPacket["measurement_limitations"] {
    const limitations: NarrativeFactPacket["measurement_limitations"] = [];
    for (const kind of ["cpu", "memory", "io", "network"] as const) {
      const quality = process.quality?.[kind]?.quality;
      if (quality === "native") continue;
      if (quality === "estimated") limitations.push({ kind, quality: "estimated" });
      else if (quality === "partial") limitations.push({ kind, quality: "limited" });
      else if (quality === "held") limitations.push({ kind, quality: "stale" });
      else limitations.push({ kind, quality: "unavailable" });
    }
    return limitations;
  }

  function leadingNarrativeResource(
    process: ProcessSample,
  ): NarrativeFactPacket["leading_resource"] {
    const values = [
      ["cpu", process.cpu_percent / 30],
      ["memory", process.memory_bytes / (900 * 1024 ** 2)],
      ["io", (process.io_read_bps + process.io_write_bps) / (500 * 1024)],
      [
        "network",
        ((process.network_received_bps ?? 0) + (process.network_transmitted_bps ?? 0)) /
          (1024 ** 2),
      ],
    ] as const;
    const leading = values.reduce((best, candidate) =>
      candidate[1] > best[1] ? candidate : best,
    );
    return leading[1] > 0 ? leading[0] : undefined;
  }

  function deterministicProcessFinding(process: ProcessSample): string {
    return processFindingLabel(
      process,
      (process.io_read_bps + process.io_write_bps),
      (process.network_received_bps ?? 0) + (process.network_transmitted_bps ?? 0),
      presentation.memoryLabel,
    );
  }

  function processHasNotableFinding(process: ProcessSample): boolean {
    return deterministicProcessFinding(process) !==
      "Activity measurements are available for this sample.";
  }

  function dropStaleNarratives(): void {
    const overviewCurrent = currentOverviewNarrativeContext();
    if (
      overviewNarrative &&
      (!overviewCurrent ||
        !isNarrativeRelevant(
          overviewNarrative,
          overviewCurrent.facts,
          "overview_contributor",
          overviewCurrent.subjectStableId,
        ))
    ) {
      overviewNarrative = null;
    }
    const workloadCurrent = currentWorkloadNarrativeContext();
    if (
      workloadNarrative &&
      (!workloadCurrent ||
        !isNarrativeRelevant(
          workloadNarrative,
          workloadCurrent.facts,
          "workload_insight",
          workloadCurrent.subjectStableId,
        ))
    ) {
      workloadNarrative = null;
    }
  }

  function isHistoryPointLimit(value: number): value is HistoryPointLimit {
    return historyPointOptions.some((option) => option === value);
  }

  function setTheme(preference: ThemePreference): void {
    themePreference = preference;
    const serialized = serializeThemePreference(preference);
    if (runtimeMode() === "native" && runtimeMutationAllowed(protocolMismatch)) {
      persistUiPreferences(preference, historyPointLimit);
    } else {
      window.localStorage.setItem(themeStorageKey, serialized);
    }
  }

  function setThemeFamily(family: ThemeFamily): void {
    setTheme({ ...themePreference, family });
  }

  function setThemeMode(mode: ThemeModePreference): void {
    setTheme({ ...themePreference, mode });
  }

  function setHistoryPointLimit(limit: number): void {
    if (!isHistoryPointLimit(limit)) {
      return;
    }

    historyPointLimit = limit;
    if (runtimeMode() === "native" && runtimeMutationAllowed(protocolMismatch)) {
      persistUiPreferences(themePreference, limit);
    } else {
      window.localStorage.setItem(historyStorageKey, String(limit));
    }
    trimHistory();
  }

  function persistUiPreferences(
    preference: ThemePreference,
    limit: HistoryPointLimit,
  ): void {
    const serializedTheme = serializeThemePreference(preference);
    window.localStorage.setItem(themeStorageKey, serializedTheme);
    window.localStorage.setItem(historyStorageKey, String(limit));
    const save = uiPreferencePersistence.begin({
      theme: serializedTheme,
      history_point_limit: limit,
    });
    void (async () => {
      try {
        const next = await setRuntimeUiPreferences(invoke, {
          theme: serializedTheme,
          history_point_limit: limit,
        });
        if (!uiPreferencePersistence.isLatest(save)) return;
        applyNativeSnapshot(next);
        if (uiPreferencePersistence.isLatestDurable(save, next)) {
          clearMigratedUiPreferences(next, save.preferences);
        }
      } catch (error) {
        if (uiPreferencePersistence.isLatest(save)) {
          commandErrorSurface = "settings";
          commandError = runtimeCommandError(error, "Unable to save interface preferences.");
        }
      }
    })();
  }

  function clearMigratedUiPreferences(
    next: RuntimeSnapshot,
    expected = next.settings.ui_preferences,
  ): void {
    if (
      !expected ||
      window.localStorage.getItem(themeStorageKey) !== expected.theme ||
      Number(window.localStorage.getItem(historyStorageKey)) !== expected.history_point_limit
    ) {
      return;
    }
    const settingsPersistence = next.persistence?.components.find(
      (component) => component.owner === "current_user" && component.kind === "settings",
    );
    if (
      settingsPersistence?.state === "healthy" &&
      settingsPersistence.durability === "durable" &&
      settingsPersistence.active_failure === null
    ) {
      window.localStorage.removeItem(themeStorageKey);
      window.localStorage.removeItem(historyStorageKey);
    }
  }

  async function setPaused(nextPaused: boolean): Promise<void> {
    if (!runtimeMutationAllowed(protocolMismatch)) return;
    if (runtimeMode() === "unavailable") {
      commandError = "BatCave telemetry requires the native desktop runtime.";
      return;
    }
    const previousPaused = isPaused;
    isPaused = nextPaused;
    if (runtimeMode() === "fixture") {
      return;
    }

    try {
      const next = await setRuntimePaused(invoke, nextPaused);
      applyNativeSnapshot(next);
    } catch (error) {
      isPaused = previousPaused;
      commandErrorSurface = "global";
      commandError = runtimeCommandError(error, "Unable to change runtime pause state.");
    }
  }

  async function refreshNow(): Promise<void> {
    const mode = runtimeMode();
    if (mode === "fixture") {
      fixtureTick += 1;
      ingest(makeFixtureSnapshot(fixtureTick, currentRuntimeQuery(), browserFixturePlatform, accessibilityFixtureState ? "compact" : "dense"));
      return;
    }
    if (mode === "unavailable") {
      commandError = "BatCave telemetry requires the native desktop runtime.";
      return;
    }

    try {
      const next = await refreshRuntime(invoke);
      applyNativeSnapshot(next);
    } catch (error) {
      commandErrorSurface = "global";
      commandError = runtimeCommandError(error, "Unable to refresh runtime.");
    }
  }

  async function setPollInterval(interval: number): Promise<void> {
    if (!runtimeMutationAllowed(protocolMismatch)) return;
    pollIntervalMs = interval as (typeof pollIntervals)[number];
    if (runtimeMode() !== "native") return;

    const requestSeq = (runtimeCadenceRequestSeq += 1);
    pendingCadenceRequestSeq = requestSeq;
    try {
      const next = await setRuntimeSampleInterval(invoke, interval);
      acceptedRuntimeControls.observe(next);
      if (requestSeq !== runtimeCadenceRequestSeq) return;
      pendingCadenceRequestSeq = 0;
      applyNativeSnapshot(next);
    } catch (error) {
      if (requestSeq !== runtimeCadenceRequestSeq) return;
      pendingCadenceRequestSeq = 0;
      const acceptedInterval = acceptedRuntimeControls.acceptedSampleIntervalMs();
      if (pollIntervals.includes(acceptedInterval as (typeof pollIntervals)[number])) {
        pollIntervalMs = acceptedInterval as (typeof pollIntervals)[number];
      }
      commandErrorSurface = "settings";
      commandError = runtimeCommandError(error, "Unable to change sampling cadence.");
    }
  }

  async function checkForStableUpdate(): Promise<void> {
    await stableUpdateController.check(snapshot.environment.install_kind, applyStableUpdateState);
  }

  async function installStableUpdate(): Promise<void> {
    await stableUpdateController.install(applyStableUpdateState);
  }

  function applyStableUpdateState(state: StableUpdateState): void {
    updateStatus = state.status;
    updateMessage = state.message;
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

  async function syncRuntimeQuery(
    query: RuntimeQuery = currentRuntimeQuery(),
    intent: RuntimeQueryWriteIntent = "user_mutation",
    onApplied?: () => void,
  ): Promise<void> {
    if (!runtimeMutationAllowed(protocolMismatch)) return;

    const mode = runtimeMode();
    if (mode === "fixture") {
      runtimeQueryRequestSeq += 1;
      ingest(makeFixtureSnapshot(fixtureTick, query, browserFixturePlatform, accessibilityFixtureState ? "compact" : "dense"));
      onApplied?.();
      return;
    }
    if (mode === "unavailable") return;

    const requestSeq = (runtimeQueryRequestSeq += 1);
    try {
      const next = await setRuntimeProcessQuery(invoke, query, intent);
      acceptedRuntimeControls.observe(next);
      if (requestSeq !== runtimeQueryRequestSeq) {
        return;
      }
      onApplied?.();
      applyNativeSnapshot(next);
    } catch (error) {
      if (requestSeq !== runtimeQueryRequestSeq) {
        return;
      }
      restoreRuntimeQueryControls(acceptedRuntimeControls.acceptedQuery());
      commandErrorSurface = "workload";
      commandError = runtimeCommandError(error, "Unable to update runtime query.");
    }
  }

  function restoreRuntimeQueryControls(query: RuntimeQuery): void {
    searchText = query.filter_text;
    focusMode = query.focus_mode;
    sortKey = sortKeyForColumn(query.sort_column);
    sortDirection = query.sort_direction;
    forceRankingRefresh = true;
  }

  function runtimeCommandError(error: unknown, fallback: string): string {
    if (error instanceof ProtocolMismatchError) {
      enterProtocolMismatch(error.mismatch, makeEmptySnapshot(error.mismatch.message));
    }
    return commandErrorMessage(error, fallback);
  }

  function enterProtocolMismatch(
    mismatch: ProtocolMismatchView,
    emptySnapshot: RuntimeSnapshot,
  ): void {
    protocolMismatch = mismatch;
    snapshot = emptySnapshot;
    selectedWorkloadId = "";
    hasAutoSelectedWorkload = false;
    history = emptyTrendState();
    inspection = null;
    inspectionGate.clear();
    resourceSummaries = [];
    displayProcessRows = [];
    pendingProcessRows = null;
    runtimeQueryRequestSeq += 1;
    if (searchDebounceId !== undefined) {
      window.clearTimeout(searchDebounceId);
      searchDebounceId = undefined;
    }
  }

  function applyNativeSnapshot(next: RuntimeSnapshot): void {
    pollState = "native";
    lastError = "";
    commandError = "";
    commandErrorSurface = "global";
    protocolMismatch = null;
    copyStatus = "";
    hasNativeSnapshot = true;
    ingest(next);
  }

  function ingest(next: RuntimeSnapshot): void {
    if (!shouldApplyRuntimePublication(snapshot, next)) {
      return;
    }

    acceptedRuntimeControls.observe(next);
    const previous = snapshot;
    hydrateRuntimeControls(next);
    const hasNewSample = hasNewRuntimeSample(previous, next);
    isPaused = next.settings.paused;
    if (
      pendingCadenceRequestSeq === 0 &&
      pollIntervals.includes(next.settings.sample_interval_ms as (typeof pollIntervals)[number])
    ) {
      pollIntervalMs = next.settings.sample_interval_ms as (typeof pollIntervals)[number];
    }
    if (runtimeMode() === "fixture") fixtureInspection.observe(next);
    snapshot = next;
    updateProcessRows(next.process_view_rows);
    observeAcceptedRuntimePublication(previous, next, observeDesktopPublication);
    window.queueMicrotask(maybeRequestInitialNarrative);
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


    history = nextSystemHistory(history, next, historyPointLimit);
    dropStaleNarratives();
  }

  function hydrateRuntimeControls(next: RuntimeSnapshot): void {
    if (next.source === "fixture" || hasHydratedRuntimeSettings || (!hasNativeSnapshot && pollState === "error")) {
      return;
    }

    const useAttentionByDefault =
      next.settings.query.focus_mode === "all" &&
      !next.settings.query.filter_text.trim() &&
      hasHighResourceUtilization(next) &&
      next.processes.some(processNeedsAttention);
    const focusHydration = planAutomaticRuntimeFocusHydration(next, useAttentionByDefault);
    searchText = next.settings.query.filter_text;
    sortKey = sortKeyForColumn(next.settings.query.sort_column);
    sortDirection = next.settings.query.sort_direction;
    focusMode = focusHydration.visible;
    isPaused = next.settings.paused;
    const pendingTheme = parseThemePreference(window.localStorage.getItem(themeStorageKey));
    const pendingHistory = Number(window.localStorage.getItem(historyStorageKey));
    const hasPendingUiMigration =
      pendingTheme !== null || isHistoryPointLimit(pendingHistory);
    let shouldPersistUiPreferences = false;
    if (hasPendingUiMigration) {
      if (pendingTheme !== null) themePreference = pendingTheme;
      if (isHistoryPointLimit(pendingHistory)) historyPointLimit = pendingHistory;
      shouldPersistUiPreferences = true;
    } else if (
      next.settings.ui_preferences &&
      parseThemePreference(next.settings.ui_preferences.theme) &&
      isHistoryPointLimit(next.settings.ui_preferences.history_point_limit)
    ) {
      themePreference = parseThemePreference(next.settings.ui_preferences.theme) ?? defaultThemePreference;
      historyPointLimit = next.settings.ui_preferences.history_point_limit;
      clearMigratedUiPreferences(next);
    } else {
      shouldPersistUiPreferences = true;
    }
    hasHydratedRuntimeSettings = true;
    dispatchAutomaticRuntimeHydration(
      next,
      {
        persistUiPreferences: shouldPersistUiPreferences,
        syncRuntimeQuery: focusHydration.requiresSync,
      },
      {
        persistUiPreferences: () =>
          window.setTimeout(() => persistUiPreferences(themePreference, historyPointLimit), 0),
        syncRuntimeQuery: () =>
          void syncRuntimeQuery(
            { ...currentRuntimeQuery(), focus_mode: focusHydration.desired },
            "runtime_only",
            () => {
              focusMode = focusHydration.desired;
              forceRankingRefresh = true;
            },
          ),
      },
    );
  }

  function hasHighResourceUtilization(next: RuntimeSnapshot): boolean {
    return (
      next.system.cpu_percent >= 75 ||
      percentage(next.system.memory_used_bytes, next.system.memory_total_bytes) >= 85
    );
  }

  function selectProcess(selection: string): void {
    cancelNarrativeWork();
    activeView = "explore";
    const changedSelection = selectedWorkloadId !== selection;
    selectedWorkloadId = selection;
    detailSubject = "process";
    copyStatus = "";
    openCompactDetail();
    if (changedSelection) {
      inspection = null;
      inspectionError = "";
      inspectionGate.clear();
    }
    window.queueMicrotask(() => void requestWorkloadNarrative());
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
      if (!process.exe || nativeProcessIcons[key] || requestedProcessIcons.has(key)) {
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
        nativeProcessIcons = { ...nativeProcessIcons, [key]: icon };
      } else if (iconError === "process_icon_untrusted_exe") {
        requestedProcessIcons.delete(key);
      }
    }
    const cached = Object.entries(nativeProcessIcons);
    if (cached.length > 256) {
      nativeProcessIcons = Object.fromEntries(cached.slice(-256));
    }
    requestedProcessIcons = new Set(iconCandidates.map(processIconKey));
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
    cancelNarrativeWork();
    activeView = "explore";
    detailMode = mode;
    detailSubject = "system";
    selectedWorkloadId = "";
    openCompactDetail();
    applyPendingRankingIfReleased();
  }

  function focusCurrentDetailControl(): void {
    const candidates =
      detailSubject === "process" && selectedWorkloadId
        ? document.querySelectorAll<HTMLElement>("[data-workload-id]")
        : document.querySelectorAll<HTMLElement>("[data-resource-mode]");
    const target = [...candidates].find((candidate) => {
      const matchesIdentity =
        detailSubject === "process" && selectedWorkloadId
          ? candidate.dataset.workloadId === selectedWorkloadId
          : candidate.dataset.resourceMode === detailMode;
      return matchesIdentity && candidate.getClientRects().length > 0;
    });
    target?.focus({ preventScroll: true });
    if (!target) {
      document
        .querySelector<HTMLElement>(`[data-view="${activeView}"]`)
        ?.focus({ preventScroll: true });
    }
  }

  function selectOverviewResource(mode: DetailMode): void {
    cancelNarrativeWork();
    overviewResource = mode;
    void requestCurrentSurfaceNarrative();
  }

  function navigateTo(view: AppView): void {
    cancelNarrativeWork();
    activeView = view;
    if (view === "overview") {
      queueInteracting = false;
      compactDetailOpen = false;
      applyPendingRankingIfReleased();
    }
    void requestCurrentSurfaceNarrative();
  }

  function openExplore(): void {
    cancelNarrativeWork();
    activeView = "explore";
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
    window.requestAnimationFrame(() => focusCurrentDetailControl());
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
      event.preventDefault();
      activeView = "explore";
      window.requestAnimationFrame(() => {
        const search = document.querySelector<HTMLInputElement>("#process-search");
        search?.focus();
        search?.select();
      });
    }
  }

  function updateProcessRows(incoming: ProcessViewRow[]): void {
    const prepared = prepareProcessViewRows(incoming, selectedWorkloadId, 180);
    incoming = prepared.rows;

    const hold = !forceRankingRefresh && shouldHoldRanking();
    const ranking = advanceProcessRanking(displayProcessRows, incoming, hold);
    displayProcessRows = ranking.rows;
    rankingUpdateAvailable = ranking.updateAvailable;
    pendingProcessRows = hold ? incoming : null;
    forceRankingRefresh = false;
  }

  function shouldHoldRanking(): boolean {
    return shouldHoldProcessOrder({ view: activeView, interacting: queueInteracting });
  }

  function applyPendingRanking(): void {
    if (pendingProcessRows !== null) {
      displayProcessRows = pendingProcessRows;
    }
    pendingProcessRows = null;
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

  function resetHistory(): void { history = emptyTrendState(); }
  function trimHistory(): void { history = trimSystemHistory(history, historyPointLimit); }

  async function refreshInspection(stableId: string, pointLimit: HistoryPointLimit, publication: number): Promise<void> {
    const ticket = inspectionGate.begin(stableId, pointLimit, publication);
    if (!stableId || protocolMismatch) { inspection = null; inspectionLoading = false; return; }
    inspectionLoading = true;
    inspectionError = "";
    try {
      const result = runtimeMode() === "native"
        ? await getWorkloadInspection(invoke, stableId, pointLimit)
        : await Promise.resolve(fixtureInspection.read(stableId, pointLimit));
      if (!inspectionGate.accept(ticket, result)) return;
      inspection = result;
      inspectionLoading = false;
      window.queueMicrotask(() => void requestWorkloadNarrative());
    } catch (error) {
      if (!inspectionGate.accept(ticket, { stable_id: stableId, publication_seq: publication })) return;
      inspectionLoading = false;
      inspectionError = commandErrorMessage(error, "Unable to load this workload.");
    }
  }

  function metricValueLabel(
    value: number,
    quality: MetricQualityInfo | undefined,
    formatter: (value: number) => string,
  ): string {
    const metric = metricPresentation(quality, collectionState, snapshot.sampled_at_ms !== null);
    return metric.canDisplay ? formatter(value) : metric.emptyLabel;
  }

  function metricCanDisplay(quality: MetricQualityInfo | undefined): boolean {
    return metricPresentation(quality, collectionState, snapshot.sampled_at_ms !== null).canDisplay;
  }

  function resourceQualityStatus(quality: MetricQualityInfo | undefined, state: CollectionState): string {
    return metricPresentation(quality, state, snapshot.sampled_at_ms !== null).label;
  }

  function currentCoreLoad(points: number[]): number {
    return boundedPercent(points.at(-1) ?? 0);
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
    return [...(tags ?? [])]
      .sort((left, right) => (right.bytes ?? -1) - (left.bytes ?? -1))
      .slice(0, 8);
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
      `Read/write I/O rate: ${displayProcessMetricValue((process.io_read_bps + process.io_write_bps), process.quality?.io, formatRate)}`,
      `Other I/O rate: ${displayProcessMetricValue(process.other_io_bps, process.quality?.other_io, formatOptionalRate)}`,
      `Network: ${processNetworkLabel(process)}`,
      `Access: ${accessLabel(process.access_state)}`,
      `Memory quality: ${metricQualityLabel(processMemoryQuality(process) as MetricQualityInfo | undefined, "Measured")}`,
      `Path: ${process.exe || "Path unavailable"}`,
      `Inspection publication seq: ${inspection?.publication_seq ?? "Unavailable"}`,
      `Sample seq: ${inspection?.catalog?.sample_seq ?? "Unavailable"}`,
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
      `Inspection publication seq: ${inspection?.publication_seq ?? "Unavailable"}`,
      `Sample seq: ${inspection?.catalog?.sample_seq ?? "Unavailable"}`,
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
      await navigator.clipboard.writeText((inspectionCurrent ? workloadSummary(selectedWorkload) : `Last recorded sample (${inspection?.catalog?.sampled_at_ms ? new Date(inspection.catalog.sampled_at_ms).toISOString() : "time unavailable"})\n${workloadSummary(selectedWorkload)}`));
      copyStatus = "Workload summary copied.";
      commandError = "";
    } catch (error) {
      const active = document.activeElement instanceof HTMLElement ? document.activeElement : null;
      const textarea = document.createElement("textarea");
      textarea.value = (inspectionCurrent ? workloadSummary(selectedWorkload) : `Last recorded sample (${inspection?.catalog?.sampled_at_ms ? new Date(inspection.catalog.sampled_at_ms).toISOString() : "time unavailable"})\n${workloadSummary(selectedWorkload)}`);
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

</script>

<svelte:head>
  <title>BatCave Monitor</title>
</svelte:head>

<svelte:window onkeydown={handleAppKeydown} />

<AppShell theme={resolvedTheme} {accessibilityFixtureState}>
  <p class="visually-hidden" role="status" aria-live="polite" aria-atomic="true">{liveStatus}</p>
  {#if protocolMismatch}
    <section class="protocol-mismatch" role="alert" aria-live="assertive">
      <strong>Telemetry protocol mismatch</strong>
      <span>{protocolMismatch.message}</span>
      <small>
        Reader v{RUNTIME_PROTOCOL_VERSION} · writer {protocolMismatch.writerVersion ?? "unknown"} · minimum reader
        {protocolMismatch.minimumReaderVersion ?? "unknown"}
      </small>
    </section>
  {/if}
  <AppHeader
    {activeView}
    {pollState}
    {healthLabel}
    {healthTone}
    onNavigate={navigateTo}
    onOpenSettings={() => {
      settingsOpen = true;
      void refreshNarrativeCapability();
    }}
    onOpenDiagnostics={() => (diagnosticsOpen = true)}
  />
  {#if commandError && commandErrorSurface === "global"}
    <p class="command-error global-command-error" role="alert">{commandError}</p>
  {/if}
  {#if activeView === "overview"}
    <Overview
      status={overviewStatus}
      resources={resourceSummaries}
      leadingRows={overviewRows}
      {processIcons}
      primaryMetric={overviewPrimaryMetric}
      leadingName={overviewPrimaryBrief.leadingWorkload}
      leadingValue={overviewContributorCopy}
      leadingNarrativeGenerated={overviewNarrativeCopy !== null}
      leadingSelection={overviewPrimaryBrief.leadingProcessId}
      leadingIconKind={overviewPrimaryIdentity?.icon ?? "process"}
      leadingIconSrc={overviewPrimaryIcon.src}
      leadingIconMatched={overviewPrimaryIcon.origin === "name_match"}
      onSelectResource={selectOverviewResource}
      onInspectResource={() => selectDetailMode(overviewResource)}
      onOpenDiagnostics={() => (diagnosticsOpen = true)}
      onSelectWorkload={selectProcess}
      onOpenExplore={openExplore}
    />
  {:else}
    <main class="explore-view" aria-labelledby="explore-heading">
      <header class="explore-heading">
        <h2 id="explore-heading">Workloads</h2>
      </header>
      <div class="explore-toolbar">
        <label class="explore-search" for="process-search">
          <MagnifyingGlass size={19} weight="regular" aria-hidden="true" />
          <input
            id="process-search"
            value={searchText}
            oninput={(event) => setSearchText(event.currentTarget.value)}
            aria-label="Search apps and processes"
            placeholder="Search by name or process"
            autocomplete="off"
            disabled={protocolMismatch !== null}
          />
          <kbd>/</kbd>
        </label>
        <ProcessCommandBar
          {focusMode}
          {sortKey}
          {sortDirection}
          commandError={commandErrorSurface === "workload" ? commandError : ""}
          {rankingUpdateAvailable}
          {focusOptions}
          {sortOptions}
          mutationsDisabled={protocolMismatch !== null}
          onFocus={setFocusMode}
          onSort={setSortKey}
          onToggleDirection={toggleSortDirection}
          onApplyRanking={applyPendingRanking}
        />
      </div>
      <section class="explore-workspace">
        <div class="explore-queue">
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
          />
        </div>
        {#if !isCompactDetail || compactDetailOpen}
          <DetailPane
            subject={detailSubject}
            telemetryStatus={telemetry}
            compact={isCompactDetail}
            onClose={closeCompactDetail}
            onShowSystem={() => selectDetailMode(detailMode)}
            {selectedWorkload}
            {selectedWorkloadIconKind}
            {selectedWorkloadIconSrc}
            {selectedWorkloadIconMatched}
            {inspection}
            {inspectionLoading}
            {inspectionError}
            {inspectionCurrent}
            {processReadRate}
            {processWriteRate}
            {processIcons}
            {copyStatus}
            {activeTheme}
            {presentation}
            {processNetworkLabel}
            insightNarrative={selectedWorkloadInsight}
            insightNarrativeGenerated={workloadNarrativeCopy !== null}
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
    </main>
  {/if}

  <DiagnosticsDrawer
    {snapshot}
    {sourceLabel}
    {systemQuality}
    {pollState}
    {lastError}
    {adminStatus}
    open={diagnosticsOpen}
    onClose={() => (diagnosticsOpen = false)}
  />

  <SettingsDrawer
    open={settingsOpen}
    {themeFamilyOptions}
    {themeModeOptions}
    {themePreference}
    {pollIntervals}
    {pollIntervalMs}
    {historyPointOptions}
    {historyPointLimit}
    {isPaused}
    commandError={commandErrorSurface === "settings" ? commandError : ""}
    adminAvailable={snapshot.environment.admin_mode_available}
    runtimeMutationsDisabled={protocolMismatch !== null}
    processStatus={processElevationLabel(snapshot.environment)}
    {adminStatus}
    adminNote={privilegedCollectionNote(snapshot.admin_mode)}
    dataDirectory={snapshot.environment.data_directory}
    {presentation}
    {enhancedNarratives}
    {narrativeCapability}
    {narrativeSettingsStatus}
    {narrativeModelAction}
    onClose={() => (settingsOpen = false)}
    onThemeFamily={setThemeFamily}
    onThemeMode={setThemeMode}
    onEnhancedNarratives={(enabled) => void setEnhancedNarrativePreference(enabled)}
    onDownloadNarrativeModel={() => void startNarrativeModelDownload()}
    onCancelNarrativeModelDownload={() => void stopNarrativeModelDownload()}
    onPollInterval={(interval) => void setPollInterval(interval)}
    onHistoryLimit={setHistoryPointLimit}
    onPaused={() => void setPaused(!isPaused)}
    onRefresh={() => void refreshNow()}
    onOpenDiagnostics={() => {
      settingsOpen = false;
      diagnosticsOpen = true;
    }}
    {updateStatus}
    {updateMessage}
    onCheckForUpdates={() => void checkForStableUpdate()}
    onInstallUpdate={() => void installStableUpdate()}
    onResetHistory={resetHistory}
  />
</AppShell>
