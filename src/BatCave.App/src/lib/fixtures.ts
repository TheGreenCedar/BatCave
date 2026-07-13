import type {
  GroupMetricQuality,
  GroupDetail,
  MetricCoverage,
  MetricQualityInfo,
  ProcessSample,
  ProcessViewRow,
  RuntimeInstallKind,
  RuntimePlatform,
  RuntimeQuery,
  RuntimeSnapshot,
} from "./types";
import { compareProcessSamples, processIdentity, processNeedsAttention } from "./process";
import { makeDefaultRuntimeQuery } from "./runtimeSnapshot";
import { summarizeProcessContributors } from "./systemPressure";

const names = [
  "Code.exe",
  "msedge.exe",
  "batcave-monitor.exe",
  "WindowsTerminal.exe",
  "MsMpEng.exe",
  "explorer.exe",
  "SearchHost.exe",
  "dwm.exe",
  "powershell.exe",
  "OneDrive.exe",
  "Rider64.exe",
  "Teams.exe",
];
const macNames = [
  "Visual Studio Code",
  "Safari",
  "Docker Desktop",
  "kernel_task",
  "Finder",
  "postgres",
  "iTerm2",
  "Slack",
  "WindowServer",
  "mds_stores",
  "rapportd",
  "ControlCenter",
];
const linuxNames = [
  "code",
  "firefox",
  "dockerd",
  "gnome-shell",
  "systemd",
  "postgres",
  "kitty",
  "slack",
  "Xorg",
  "NetworkManager",
  "containerd",
  "pipewire",
];
const backgroundNames = [
  "RuntimeBroker.exe",
  "svchost.exe",
  "conhost.exe",
  "ShellExperienceHost.exe",
  "StartMenuExperienceHost.exe",
  "TextInputHost.exe",
  "SearchIndexer.exe",
  "SecurityHealthService.exe",
  "WidgetService.exe",
  "NVIDIAContainer.exe",
  "PhoneExperienceHost.exe",
  "ApplicationFrameHost.exe",
];
const macBackgroundNames = [
  "launchservicesd",
  "runningboardd",
  "coreaudiod",
  "sharingd",
  "locationd",
  "airportd",
  "distnoted",
  "cfprefsd",
  "analyticsd",
  "trustd",
  "logd",
  "symptomsd",
];
const linuxBackgroundNames = [
  "systemd-journald",
  "systemd-resolved",
  "dbus-daemon",
  "pipewire-pulse",
  "xdg-desktop-portal",
  "gvfsd",
  "udisksd",
  "upowerd",
  "polkitd",
  "tracker-miner-fs",
  "cron",
  "sshd",
];

const baseTs = Date.now();

export function makeFixtureSnapshot(
  tick: number,
  query: RuntimeQuery = makeDefaultRuntimeQuery(),
  platform: RuntimePlatform = "fixture",
): RuntimeSnapshot {
  const cpu = platform === "macos" ? wave(tick, 71, 7, 0.18) : wave(tick, 31, 14, 0.34);
  const memoryTotal = 32 * 1024 * 1024 * 1024;
  const memoryRatio = 0.55 + Math.sin(tick / 14) * 0.06;
  const logicalCpu = Array.from({ length: 12 }, (_, index) =>
    clamp(wave(tick + index * 2, 24 + index * 1.5, 20, 0.22 + index * 0.017), 1, 96),
  );
  const processCount = 284 + Math.round(Math.sin(tick / 8) * 8);
  const processes = Array.from({ length: processCount }, (_, index) =>
    makeProcess(fixtureProcessName(index, platform), index, tick, platform),
  ).sort((left, right) => right.cpu_percent - left.cpu_percent);
  const processNetworkReceivedBps = processes.reduce(
    (total, process) => total + (process.network_received_bps ?? 0),
    0,
  );
  const processNetworkTransmittedBps = processes.reduce(
    (total, process) => total + (process.network_transmitted_bps ?? 0),
    0,
  );
  const processWorkingSet = processes.reduce((total, process) => total + process.memory_bytes, 0);
  const processPrivate = processes.reduce((total, process) => total + process.private_bytes, 0);
  const memoryUsed = Math.round(memoryTotal * memoryRatio);
  const kernelPaged = Math.round(1.2 * 1024 * 1024 * 1024 + Math.sin(tick / 9) * 80_000_000);
  const kernelNonpaged = Math.round(760 * 1024 * 1024 + Math.cos(tick / 11) * 50_000_000);
  const diskReadBps = 4_200_000 + Math.round(Math.sin(tick / 2) * 800_000);
  const diskWriteBps = 2_600_000 + Math.round(Math.cos(tick / 4) * 600_000);
  const networkReceivedBps =
    processNetworkReceivedBps + 120_000 + Math.round(Math.sin(tick / 3) * 24_000);
  const networkTransmittedBps =
    processNetworkTransmittedBps + 70_000 + Math.round(Math.cos(tick / 5) * 18_000);

  return {
    event_kind: "runtime_snapshot",
    publication_seq: tick,
    published_at_ms: baseTs + tick * 1000,
    sample_seq: tick,
    sampled_at_ms: baseTs + tick * 1000,
    source: "fixture",
    environment: {
      platform,
      admin_mode_available: false,
      install_kind: fixtureInstallKind(platform),
      data_directory: fixtureDataDirectory(platform),
    },
    admin_mode: {
      state: "unavailable",
      detail: null,
      last_success_at_ms: null,
    },
    settings: {
      query,
      admin_mode_requested: false,
      admin_mode_enabled: false,
      metric_window_seconds: 60,
      sample_interval_ms: 1000,
      paused: false,
    },
    health: {
      tick_count: tick,
      snapshot_latency_ms: 3 + Math.round(Math.abs(Math.sin(tick / 3)) * 9),
      degraded: false,
      collector_warnings: 0,
      runtime_loop_enabled: true,
      runtime_loop_running: true,
      status_summary: "Fixture telemetry is running.",
      updated_at_ms: baseTs + tick * 1000,
      tick_p95_ms: 4,
      sort_p95_ms: 1,
      jitter_p95_ms: 2,
      dropped_ticks: 0,
      app_cpu_percent: 0.4,
      app_rss_bytes: 96 * 1024 * 1024,
      last_warning: null,
    },
    system: {
      cpu_percent: clamp(cpu, 0, 100),
      kernel_cpu_percent: clamp(cpu * 0.18, 0, 100),
      logical_cpu_percent: logicalCpu,
      memory_used_bytes: memoryUsed,
      memory_total_bytes: memoryTotal,
      memory_available_bytes: Math.round(memoryTotal * (1 - memoryRatio)),
      swap_used_bytes:
        platform === "macos"
          ? undefined
          : Math.round(1.8 * 1024 * 1024 * 1024 + Math.sin(tick / 7) * 220_000_000),
      swap_total_bytes: platform === "macos" ? undefined : 8 * 1024 * 1024 * 1024,
      process_count: processes.length,
      disk_read_total_bytes: 62_000_000_000 + tick * diskReadBps,
      disk_write_total_bytes: 39_000_000_000 + tick * diskWriteBps,
      disk_read_bps: diskReadBps,
      disk_write_bps: diskWriteBps,
      network_received_total_bytes: 512_000_000_000 + tick * networkReceivedBps,
      network_transmitted_total_bytes: 188_000_000_000 + tick * networkTransmittedBps,
      network_received_bps: networkReceivedBps,
      network_transmitted_bps: networkTransmittedBps,
      memory_accounting: {
        process_working_set_bytes: processWorkingSet,
        process_private_bytes: processPrivate,
        denied_process_count: 0,
        partial_process_count: 0,
        commit_used_bytes: memoryUsed + 2 * 1024 * 1024 * 1024,
        commit_limit_bytes: memoryTotal + 8 * 1024 * 1024 * 1024,
        system_cache_bytes: Math.round(2.4 * 1024 * 1024 * 1024),
        kernel_total_bytes: kernelPaged + kernelNonpaged,
        kernel_paged_pool_bytes: kernelPaged,
        kernel_nonpaged_pool_bytes: kernelNonpaged,
        kernel_pool_tags: [
          {
            tag: "Leak",
            kind: "nonpaged",
            bytes: Math.round(kernelNonpaged * 0.34),
            allocations: 42_318 + tick,
            frees: 8_912,
            driver_candidates: ["ExampleLeakDriver.sys"],
            driver_candidates_pending: false,
          },
          {
            tag: "Nvld",
            kind: "paged",
            bytes: Math.round(kernelPaged * 0.18),
            allocations: 88_104 + tick * 2,
            frees: 87_011,
            driver_candidates: ["nvlddmkm.sys"],
            driver_candidates_pending: false,
          },
          {
            tag: "WsLt",
            kind: "nonpaged",
            bytes: Math.round(kernelNonpaged * 0.12),
            allocations: 9_824,
            frees: 1_032,
            driver_candidates: [],
            driver_candidates_pending: false,
          },
          {
            tag: "Pend",
            kind: "paged",
            bytes: Math.round(kernelPaged * 0.08),
            allocations: 1_442,
            frees: 112,
            driver_candidates: [],
            driver_candidates_pending: true,
          },
        ],
      },
      quality: {
        cpu: { quality: "estimated", source: "fixture" },
        kernel_cpu: { quality: "estimated", source: "fixture" },
        logical_cpu: { quality: "estimated", source: "fixture" },
        memory: { quality: "estimated", source: "fixture" },
        swap:
          platform === "macos"
            ? {
                quality: "unavailable",
                source: "fixture",
                message: "Swap pressure is not available in this macOS layout fixture.",
              }
            : { quality: "estimated", source: "fixture" },
        disk: { quality: "estimated", source: "fixture" },
        network: { quality: "estimated", source: "fixture" },
      },
    },
    process_contributors: summarizeProcessContributors(processes),
    processes,
    process_view_rows: shapeProcessView(processes, query),
    total_process_count: processes.length,
    warnings: [],
  };
}

interface FixtureProcessGroup {
  key: string;
  label: string;
  category: string;
  iconKind: string;
  presentationProcess: ProcessSample;
  processes: ProcessSample[];
  cpuPercent: number;
  memoryBytes: number;
  ioBps: number;
  networkBps: number;
  threads: number;
}

function shapeProcessView(processes: ProcessSample[], query: RuntimeQuery): ProcessViewRow[] {
  const needle = query.filter_text.trim().toLocaleLowerCase();
  const rows = processes
    .filter(
      (process) =>
        !needle ||
        process.name.toLocaleLowerCase().includes(needle) ||
        process.pid.includes(needle) ||
        process.exe.toLocaleLowerCase().includes(needle),
    )
    .filter((process) => matchesFocusMode(process, query.focus_mode))
    .slice()
    .sort((left, right) => compareProcessSamples(left, right, query))
    .slice(0, Math.max(1, query.limit));

  const groups = new Map<string, FixtureProcessGroup>();
  for (const process of rows) {
    const key = processAppKey(process);
    let group = groups.get(key);
    if (!group) {
      const identity = processIdentity(process);
      group = {
        key,
        label: normalizedProcessName(process.name),
        category: identity.group,
        iconKind: identity.icon,
        presentationProcess: process,
        processes: [],
        cpuPercent: 0,
        memoryBytes: 0,
        ioBps: 0,
        networkBps: 0,
        threads: 0,
      };
      groups.set(key, group);
    }

    group.processes.push(process);
    group.cpuPercent += process.cpu_percent;
    group.memoryBytes += process.memory_bytes;
    group.ioBps += processIoRate(process);
    group.networkBps += processNetworkRate(process);
    group.threads += process.threads;
  }

  return [...groups.values()]
    .sort((left, right) => compareGroups(left, right, query))
    .flatMap((group) => {
      const grouped = group.processes.length > 1;
      const processRows = group.processes.map((process) => {
        const identity = processIdentity(process);
        const ioBps = processIoRate(process);
        return {
          kind: "process" as const,
          detail: {
            kind: "process" as const,
            workload_id: processWorkloadId(process),
            process,
            io_bps: ioBps,
            network_bps: processNetworkRate(process),
          },
          group_key: group.key,
          group_label: group.label,
          group_category: group.category,
          group_count: group.processes.length,
          icon_kind: identity.icon,
          is_child: identity.isChild,
          is_grouped: grouped,
          attention_label: attentionLabel(
            process.cpu_percent,
            process.memory_bytes,
            ioBps,
            processNetworkRate(process),
            process.access_state !== "full",
          ),
        };
      });

      if (!grouped) {
        return processRows;
      }

      return [
        {
          kind: "group" as const,
          detail: groupDetail(group),
          icon_kind: group.iconKind,
          icon_source: group.presentationProcess.exe || undefined,
          example_label:
            group.presentationProcess.name === group.label
              ? undefined
              : group.presentationProcess.name,
          attention_label: attentionLabel(
            group.cpuPercent,
            group.memoryBytes,
            group.ioBps,
            group.networkBps,
            group.processes.some((process) => process.access_state !== "full"),
          ),
        },
        ...processRows,
      ];
    });
}

type GroupMetricKey = keyof GroupMetricQuality;

function groupDetail(group: FixtureProcessGroup): GroupDetail {
  const cpu = groupMetricSummary(group.processes, "cpu");
  const memory = groupMetricSummary(group.processes, "memory");
  const io = groupMetricSummary(group.processes, "io");
  const otherIo = groupMetricSummary(group.processes, "other_io");
  const network = groupMetricSummary(group.processes, "network");
  const threads = groupMetricSummary(group.processes, "threads");
  return {
    kind: "group",
    workload_id: groupWorkloadId(group.key),
    group_key: group.key,
    label: group.label,
    category: group.category,
    process_count: group.processes.length,
    cpu_percent: round1(group.cpuPercent),
    memory_bytes: group.memoryBytes,
    io_bps: group.ioBps,
    network_bps: group.networkBps,
    threads: group.threads,
    quality: {
      cpu: cpu.quality,
      memory: memory.quality,
      io: io.quality,
      other_io: otherIo.quality,
      network: network.quality,
      threads: threads.quality,
    },
    coverage: {
      cpu: cpu.coverage,
      memory: memory.coverage,
      io: io.coverage,
      other_io: otherIo.coverage,
      network: network.coverage,
      threads: threads.coverage,
    },
  };
}

function groupMetricSummary(
  processes: ProcessSample[],
  metric: GroupMetricKey,
): { quality: MetricQualityInfo; coverage: MetricCoverage } {
  const availableProcesses = processes.filter((process) => groupMetricAvailable(process, metric));
  const availableQualities = availableProcesses
    .map((process) => groupMetricQuality(process, metric)?.quality)
    .filter((quality): quality is MetricQualityInfo["quality"] => !!quality);
  const coverage: MetricCoverage = {
    available: availableProcesses.length,
    total: processes.length,
  };
  const quality: MetricQualityInfo["quality"] =
    coverage.available === 0
      ? "unavailable"
      : coverage.available < coverage.total ||
          availableQualities.some((value) => value === "partial" || value === "unavailable") ||
          (availableQualities.some((value) => value === "held") &&
            !availableQualities.every((value) => value === "held"))
        ? "partial"
        : availableQualities.length > 0 && availableQualities.every((value) => value === "held")
          ? "held"
          : availableQualities.some((value) => value === "estimated")
            ? "estimated"
            : "native";

  return {
    quality: {
      quality,
      source: "process_aggregate",
      message:
        coverage.available < coverage.total
          ? `${coverage.available} of ${coverage.total} processes contribute to this aggregate.`
          : undefined,
    },
    coverage,
  };
}

function groupMetricAvailable(process: ProcessSample, metric: GroupMetricKey): boolean {
  if (metric === "other_io") return false;
  const hasValue =
    metric !== "network" ||
    process.network_received_bps !== undefined ||
    process.network_transmitted_bps !== undefined;
  return hasValue && groupMetricQuality(process, metric)?.quality !== "unavailable";
}

function groupMetricQuality(
  process: ProcessSample,
  metric: GroupMetricKey,
): MetricQualityInfo | undefined {
  if (metric === "io") return process.quality?.io;
  if (metric === "other_io") return process.quality?.other_io;
  return process.quality?.[metric];
}

function processWorkloadId(process: ProcessSample): string {
  return `process:${process.pid}:${process.start_time_ms}`;
}

function groupWorkloadId(groupKey: string): string {
  return `group:${groupKey}`;
}

function compareGroups(
  left: FixtureProcessGroup,
  right: FixtureProcessGroup,
  query: RuntimeQuery,
): number {
  const comparison =
    query.sort_column === "name"
      ? left.label.localeCompare(right.label)
      : query.sort_column === "pid"
        ? left.key.localeCompare(right.key)
        : query.sort_column === "cpu_pct"
          ? left.cpuPercent - right.cpuPercent
          : query.sort_column === "memory_bytes"
            ? left.memoryBytes - right.memoryBytes
            : query.sort_column === "io_bps"
              ? left.ioBps - right.ioBps
              : query.sort_column === "network_bps"
                ? left.networkBps - right.networkBps
                : query.sort_column === "threads"
                  ? left.threads - right.threads
                  : query.sort_column === "handles" || query.sort_column === "start_time_ms"
                    ? left.key.localeCompare(right.key)
                    : groupAttentionScore(left) - groupAttentionScore(right);

  return directed(comparison, query.sort_direction) || left.label.localeCompare(right.label);
}

function directed(comparison: number, direction: RuntimeQuery["sort_direction"]): number {
  return direction === "asc" ? comparison : -comparison;
}

function groupAttentionScore(group: FixtureProcessGroup): number {
  return (
    group.cpuPercent * 3 +
    Math.min(group.memoryBytes / (128 * 1024 * 1024), 20) +
    Math.min(group.ioBps / (512 * 1024), 20) +
    Math.min(group.networkBps / (1024 * 1024), 20) +
    (group.processes.some((process) => process.access_state !== "full") ? 12 : 0)
  );
}

function matchesFocusMode(process: ProcessSample, focusMode: RuntimeQuery["focus_mode"]): boolean {
  if (focusMode === "attention") {
    return processNeedsAttention(process);
  }

  if (focusMode === "io") {
    return processIoRate(process) > 0;
  }

  return true;
}

function processAppKey(process: ProcessSample): string {
  const executableName = normalizedProcessName(executableFileName(process.exe)).trim();
  const processName = normalizedProcessName(process.name).trim();
  return (executableName || processName || `pid:${process.pid}`).toLocaleLowerCase();
}

function executableFileName(path: string): string {
  const trimmed = path.trim();
  return trimmed.split(/[\\/]/).pop() || trimmed;
}

function normalizedProcessName(name: string): string {
  return name.replace(/-\d+(?=\.exe$)/i, "");
}

function processIoRate(process: ProcessSample): number {
  return process.io_read_bps + process.io_write_bps;
}

function processNetworkRate(process: ProcessSample): number {
  return (process.network_received_bps ?? 0) + (process.network_transmitted_bps ?? 0);
}

function attentionLabel(
  cpuPercent: number,
  memoryBytes: number,
  ioBps: number,
  networkBps: number,
  limitedAccess: boolean,
): string {
  if (cpuPercent >= 1) return "CPU activity";
  if (memoryBytes >= 900 * 1024 * 1024) return "memory activity";
  if (ioBps >= 500 * 1024) return "I/O activity";
  if (networkBps >= 1024 * 1024) return "network activity";
  if (limitedAccess) return "access limited";
  return "steady";
}

function makeProcess(
  name: string,
  index: number,
  tick: number,
  platform: RuntimePlatform,
): ProcessSample {
  const pid = 2100 + index * 7 + (index % 5) * 17;
  const priorityCpu =
    platform === "macos"
      ? index === 0
        ? 27
        : index === 1
          ? 13
          : index === 2
            ? 8
            : index % 37 === 0
              ? 5
              : 0
      : index === 2
        ? 18
        : index % 37 === 0
          ? 11
          : index % 13 === 0
            ? 6
            : 0;
  const baseCpu =
    platform === "macos" && index >= macNames.length
      ? 0.25 + (index % 6) * 0.16
      : 0.8 + (index % 12) * 0.72 + priorityCpu;
  const cpu = clamp(
    wave(tick + index, baseCpu, 3.8 + (index % 5), 0.22 + (index % 13) * 0.012),
    0.1,
    72,
  );
  const baseMemory = index < names.length ? 180 + index * 74 : 42 + (index % 22) * 28;
  const memory = (baseMemory + Math.sin(tick / 5 + index) * 28) * 1024 * 1024;
  const networkReceived = Math.max(
    0,
    Math.round((index % 4) * 2_800 + wave(tick, 4_800, 1_700, 0.2 + index * 0.01)),
  );
  const networkTransmitted = Math.max(
    0,
    Math.round((index % 3) * 1_600 + wave(tick, 2_400, 1_000, 0.17 + index * 0.01)),
  );
  const diskReadBps = Math.max(
    0,
    Math.round(
      7_000 + (index % 24) * 520 + priorityCpu * 1_100 + wave(tick + index, 1_600, 900, 0.13),
    ),
  );
  const diskWriteBps = Math.max(
    0,
    Math.round(
      4_600 + (index % 18) * 360 + priorityCpu * 720 + wave(tick + index, 1_000, 620, 0.12),
    ),
  );
  const otherIoBps = Math.max(0, Math.round(1_100 + (index % 10) * 150 + priorityCpu * 180));

  const networkUnavailable = platform === "macos";
  return {
    pid: `${pid}`,
    parent_pid: index < 2 ? null : `${1800 + index * 19}`,
    start_time_ms: baseTs - index * 180_000,
    name,
    exe: fixtureExecutable(name, platform),
    status: cpu >= 2 ? "Run" : "Idle",
    cpu_percent: round1(cpu),
    memory_bytes: Math.max(18 * 1024 * 1024, Math.round(memory)),
    private_bytes: Math.max(12 * 1024 * 1024, Math.round(memory * 0.76)),
    virtual_memory_bytes: Math.round(memory * 1.7),
    io_read_total_bytes: 12_000_000_000 + index * 500_000_000 + tick * diskReadBps,
    io_write_total_bytes: 8_000_000_000 + index * 290_000_000 + tick * diskWriteBps,
    other_io_total_bytes: 1_000_000_000 + index * 85_000_000 + tick * otherIoBps,
    io_read_bps: diskReadBps,
    io_write_bps: diskWriteBps,
    other_io_bps: otherIoBps,
    network_received_bps: networkUnavailable ? undefined : networkReceived,
    network_transmitted_bps: networkUnavailable ? undefined : networkTransmitted,
    threads: 4 + index * 2,
    handles: 80 + index * 17,
    access_state: "full",
    quality: {
      cpu: { quality: "estimated", source: "fixture" },
      memory: { quality: "estimated", source: "fixture" },
      io: { quality: "estimated", source: "fixture" },
      other_io: { quality: "estimated", source: "fixture" },
      network: networkUnavailable
        ? {
            quality: "unavailable",
            source: "fixture",
            message: "Per-process network attribution is unavailable on macOS.",
          }
        : { quality: "estimated", source: "fixture" },
      threads: { quality: "estimated", source: "fixture" },
      handles: { quality: "estimated", source: "fixture" },
    },
  };
}

function fixtureProcessName(index: number, platform: RuntimePlatform): string {
  const primaryNames = platform === "macos" ? macNames : platform === "linux" ? linuxNames : names;
  if (index < primaryNames.length) {
    return primaryNames[index];
  }

  const backgroundPool =
    platform === "macos"
      ? macBackgroundNames
      : platform === "linux"
        ? linuxBackgroundNames
        : backgroundNames;
  const family = backgroundPool[index % backgroundPool.length].replace(".exe", "");
  const suffix = String(index - primaryNames.length + 1).padStart(3, "0");
  return platform === "windows" || platform === "fixture"
    ? `${family}-${suffix}.exe`
    : `${family.toLocaleLowerCase()}-${suffix}`;
}

function fixtureInstallKind(platform: RuntimePlatform): RuntimeInstallKind {
  if (platform === "macos") return "dmg";
  if (platform === "linux") return "appimage";
  if (platform === "windows") return "nsis";
  return "portable";
}

function fixtureDataDirectory(platform: RuntimePlatform): string | null {
  if (platform === "macos") return "~/Library/Application Support/BatCaveMonitor";
  if (platform === "linux") return "~/.local/share/BatCaveMonitor";
  if (platform === "windows") return "%LOCALAPPDATA%\\BatCaveMonitor";
  return null;
}

function fixtureExecutable(name: string, platform: RuntimePlatform): string {
  if (platform === "macos") {
    const bundleNames = new Set([
      "Visual Studio Code",
      "Safari",
      "Docker Desktop",
      "Finder",
      "iTerm2",
      "Slack",
    ]);
    return bundleNames.has(name)
      ? `/Applications/${name}.app/Contents/MacOS/${name}`
      : `/usr/${name.startsWith("kernel_") ? "sbin" : "bin"}/${name}`;
  }
  if (platform === "linux") return `/usr/bin/${name}`;
  return `C:\\Program Files\\${name.replace(".exe", "")}\\${name}`;
}

function wave(tick: number, base: number, amplitude: number, speed: number): number {
  return (
    base + Math.sin(tick * speed) * amplitude + Math.cos(tick * speed * 0.43) * amplitude * 0.35
  );
}

function clamp(value: number, min: number, max: number): number {
  return Math.min(max, Math.max(min, value));
}

function round1(value: number): number {
  return Math.round(value * 10) / 10;
}
