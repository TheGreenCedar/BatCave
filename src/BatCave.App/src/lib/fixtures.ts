import type { ProcessSample, ProcessViewRow, RuntimeQuery, RuntimeSnapshot } from "./types";

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

const baseTs = Date.now();

export function makeFixtureSnapshot(
  tick: number,
  query: RuntimeQuery = defaultRuntimeQuery(),
): RuntimeSnapshot {
  const cpu = wave(tick, 31, 14, 0.34);
  const memoryTotal = 32 * 1024 * 1024 * 1024;
  const memoryRatio = 0.55 + Math.sin(tick / 14) * 0.06;
  const logicalCpu = Array.from({ length: 12 }, (_, index) =>
    clamp(wave(tick + index * 2, 24 + index * 1.5, 20, 0.22 + index * 0.017), 1, 96),
  );
  const processCount = 284 + Math.round(Math.sin(tick / 8) * 8);
  const processes = Array.from({ length: processCount }, (_, index) =>
    makeProcess(fixtureProcessName(index), index, tick),
  ).sort((left, right) => right.cpu_percent - left.cpu_percent);
  const processDiskReadBps = processes.reduce((total, process) => total + process.disk_read_bps, 0);
  const processDiskWriteBps = processes.reduce(
    (total, process) => total + process.disk_write_bps,
    0,
  );
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
  const diskReadBps = processDiskReadBps + 420_000 + Math.round(Math.sin(tick / 2) * 80_000);
  const diskWriteBps = processDiskWriteBps + 260_000 + Math.round(Math.cos(tick / 4) * 60_000);
  const networkReceivedBps =
    processNetworkReceivedBps + 120_000 + Math.round(Math.sin(tick / 3) * 24_000);
  const networkTransmittedBps =
    processNetworkTransmittedBps + 70_000 + Math.round(Math.cos(tick / 5) * 18_000);

  return {
    event_kind: "runtime_snapshot",
    seq: tick,
    ts_ms: baseTs + tick * 1000,
    source: "fixture",
    settings: {
      query,
      admin_mode_requested: false,
      admin_mode_enabled: false,
      metric_window_seconds: 60,
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
      swap_used_bytes: Math.round(1.8 * 1024 * 1024 * 1024 + Math.sin(tick / 7) * 220_000_000),
      swap_total_bytes: 8 * 1024 * 1024 * 1024,
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
        unattributed_bytes: Math.max(0, memoryUsed - processWorkingSet),
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
        swap: { quality: "estimated", source: "fixture" },
        disk: { quality: "estimated", source: "fixture" },
        network: { quality: "estimated", source: "fixture" },
      },
    },
    processes,
    process_view_rows: shapeProcessView(processes, query),
    total_process_count: processes.length,
    warnings: [],
  };
}

function defaultRuntimeQuery(): RuntimeQuery {
  return {
    filter_text: "",
    focus_mode: "all",
    sort_column: "attention",
    sort_direction: "desc",
    limit: 5000,
  };
}

interface FixtureIdentity {
  iconKind: string;
  category: string;
  isChild: boolean;
}

interface FixtureProcessGroup {
  key: string;
  label: string;
  category: string;
  iconKind: string;
  representative: ProcessSample;
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
    .sort((left, right) => compareProcess(left, right, query))
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
        category: identity.category,
        iconKind: identity.iconKind,
        representative: process,
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
    .sort((left, right) => compareProcessGroup(left, right, query))
    .flatMap((group) => {
      const grouped = group.processes.length > 1;
      const processRows = group.processes.map((process) => {
        const identity = processIdentity(process);
        const ioBps = processIoRate(process);
        return {
          kind: "process" as const,
          process,
          group_key: group.key,
          group_label: group.label,
          group_category: group.category,
          group_count: group.processes.length,
          icon_kind: identity.iconKind,
          is_child: identity.isChild,
          is_grouped: grouped,
          attention_label: attentionLabel(process.cpu_percent, process.memory_bytes, ioBps),
          cpu_percent: process.cpu_percent,
          memory_bytes: process.memory_bytes,
          io_bps: ioBps,
          network_bps: processNetworkRate(process),
          threads: process.threads,
        };
      });

      if (!grouped) {
        return processRows;
      }

      return [
        {
          kind: "group" as const,
          representative: group.representative,
          group_key: group.key,
          group_label: group.label,
          group_category: group.category,
          group_count: group.processes.length,
          icon_kind: group.iconKind,
          is_child: false,
          is_grouped: true,
          attention_label: attentionLabel(group.cpuPercent, group.memoryBytes, group.ioBps),
          cpu_percent: round1(group.cpuPercent),
          memory_bytes: group.memoryBytes,
          io_bps: group.ioBps,
          network_bps: group.networkBps,
          threads: group.threads,
        },
        ...processRows,
      ];
    });
}

function matchesFocusMode(process: ProcessSample, focusMode: RuntimeQuery["focus_mode"]): boolean {
  if (focusMode === "active") {
    return process.cpu_percent >= 1;
  }

  if (focusMode === "io") {
    return processIoRate(process) > 0;
  }

  return true;
}

function compareProcess(left: ProcessSample, right: ProcessSample, query: RuntimeQuery): number {
  const direction = query.sort_direction === "asc" ? 1 : -1;
  let comparison = 0;

  if (query.sort_column === "name") {
    comparison = left.name.localeCompare(right.name, undefined, {
      sensitivity: "base",
      numeric: true,
    });
  } else if (query.sort_column === "pid") {
    comparison = Number(left.pid) - Number(right.pid);
  } else if (query.sort_column === "memory_bytes") {
    comparison = left.memory_bytes - right.memory_bytes;
  } else if (query.sort_column === "disk_bps") {
    comparison = processIoRate(left) - processIoRate(right);
  } else if (query.sort_column === "network_bps") {
    comparison = processNetworkRate(left) - processNetworkRate(right);
  } else if (query.sort_column === "threads") {
    comparison = left.threads - right.threads;
  } else if (query.sort_column === "handles") {
    comparison = left.handles - right.handles;
  } else if (query.sort_column === "start_time_ms") {
    comparison = left.start_time_ms - right.start_time_ms;
  } else if (query.sort_column === "cpu_pct") {
    comparison = left.cpu_percent - right.cpu_percent;
  } else {
    comparison = attentionScore(left) - attentionScore(right);
  }

  return (
    comparison * direction ||
    left.name.localeCompare(right.name, undefined, { sensitivity: "base", numeric: true })
  );
}

function compareProcessGroup(
  left: FixtureProcessGroup,
  right: FixtureProcessGroup,
  query: RuntimeQuery,
): number {
  const direction = query.sort_direction === "asc" ? 1 : -1;
  let comparison = 0;

  if (query.sort_column === "name") {
    comparison = left.label.localeCompare(right.label, undefined, {
      sensitivity: "base",
      numeric: true,
    });
  } else if (query.sort_column === "pid") {
    comparison = Number(left.representative.pid) - Number(right.representative.pid);
  } else if (query.sort_column === "memory_bytes") {
    comparison = left.memoryBytes - right.memoryBytes;
  } else if (query.sort_column === "disk_bps") {
    comparison = left.ioBps - right.ioBps;
  } else if (query.sort_column === "network_bps") {
    comparison = left.networkBps - right.networkBps;
  } else if (query.sort_column === "threads") {
    comparison = left.threads - right.threads;
  } else if (query.sort_column === "handles") {
    comparison = left.representative.handles - right.representative.handles;
  } else if (query.sort_column === "start_time_ms") {
    comparison = left.representative.start_time_ms - right.representative.start_time_ms;
  } else if (query.sort_column === "cpu_pct") {
    comparison = left.cpuPercent - right.cpuPercent;
  } else {
    comparison = groupAttentionScore(left) - groupAttentionScore(right);
  }

  return (
    comparison * direction ||
    left.label.localeCompare(right.label, undefined, { sensitivity: "base", numeric: true })
  );
}

function processIdentity(process: ProcessSample): FixtureIdentity {
  const haystack = `${process.name} ${process.exe}`.toLocaleLowerCase();
  const name = process.name.toLocaleLowerCase();
  const isChild =
    name.startsWith("--") ||
    haystack.includes("--type=") ||
    haystack.includes("renderer") ||
    haystack.includes("gpu-process") ||
    haystack.includes("utility");

  if (haystack.includes("batcave")) return { iconKind: "batcave", category: "BatCave", isChild };
  if (matchesAny(haystack, ["chrome", "msedge", "firefox", "brave", "browser"])) {
    return { iconKind: "browser", category: "Browsers", isChild };
  }
  if (matchesAny(haystack, ["code.exe", "visual studio code", "\\code\\", "/code/"])) {
    return { iconKind: "code", category: "Developer tools", isChild };
  }
  if (matchesAny(haystack, ["node", "npm", "deno", "bun.exe"]))
    return { iconKind: "node", category: "Runtimes", isChild };
  if (haystack.includes("docker"))
    return { iconKind: "container", category: "Containers", isChild };
  if (matchesAny(haystack, ["postgres", "mysql", "redis", "sqlserver", "mariadb"])) {
    return { iconKind: "database", category: "Databases", isChild };
  }
  if (matchesAny(haystack, ["slack", "teams", "discord", "zoom"])) {
    return { iconKind: "chat", category: "Communication", isChild };
  }
  if (matchesAny(haystack, ["spotify", "vlc", "media player"]))
    return { iconKind: "media", category: "Media", isChild };
  if (matchesAny(haystack, ["dropbox", "onedrive", "googledrive"]))
    return { iconKind: "sync", category: "Sync", isChild };
  if (matchesAny(haystack, ["nvidia", "amd", "radeon", "intel graphics"]))
    return { iconKind: "gpu", category: "GPU", isChild };
  if (
    matchesAny(haystack, [
      "applicationframehost",
      "conhost",
      "ctfmon",
      "dwm",
      "explorer.exe",
      "phoneexperiencehost",
      "searchindexer",
      "securityhealthservice",
      "shellexperiencehost",
      "sihost",
      "startmenuexperiencehost",
      "svchost",
      "textinputhost",
      "widgetservice",
      "windows",
    ])
  ) {
    return { iconKind: "windows", category: "Windows", isChild };
  }

  return { iconKind: "process", category: "Processes", isChild };
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

function matchesAny(haystack: string, needles: string[]): boolean {
  return needles.some((needle) => haystack.includes(needle));
}

function processIoRate(process: ProcessSample): number {
  return process.disk_read_bps + process.disk_write_bps + (process.other_io_bps ?? 0);
}

function processNetworkRate(process: ProcessSample): number {
  return (process.network_received_bps ?? 0) + (process.network_transmitted_bps ?? 0);
}

function attentionScore(process: ProcessSample): number {
  return (
    process.cpu_percent * 3 +
    Math.min(process.memory_bytes / (128 * 1024 * 1024), 20) +
    Math.min(processIoRate(process) / (512 * 1024), 20) +
    Math.min(processNetworkRate(process) / (1024 * 1024), 20) +
    (process.access_state === "full" ? 0 : 12)
  );
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

function attentionLabel(cpuPercent: number, memoryBytes: number, ioBps: number): string {
  if (cpuPercent >= 20) return "CPU lead";
  if (memoryBytes >= 900 * 1024 * 1024) return "memory lead";
  if (ioBps >= 500 * 1024) return "I/O lead";
  return "steady";
}

function makeProcess(name: string, index: number, tick: number): ProcessSample {
  const pid = 2100 + index * 7 + (index % 5) * 17;
  const priorityCpu = index === 2 ? 18 : index % 37 === 0 ? 11 : index % 13 === 0 ? 6 : 0;
  const baseCpu = 0.8 + (index % 12) * 0.72 + priorityCpu;
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

  return {
    pid: `${pid}`,
    parent_pid: index < 2 ? null : `${1800 + index * 19}`,
    start_time_ms: baseTs - index * 180_000,
    name,
    exe: `C:\\Program Files\\${name.replace(".exe", "")}\\${name}`,
    status: cpu >= 2 ? "Run" : "Idle",
    cpu_percent: round1(cpu),
    memory_bytes: Math.max(18 * 1024 * 1024, Math.round(memory)),
    private_bytes: Math.max(12 * 1024 * 1024, Math.round(memory * 0.76)),
    virtual_memory_bytes: Math.round(memory * 1.7),
    disk_read_total_bytes: 12_000_000_000 + index * 500_000_000 + tick * diskReadBps,
    disk_write_total_bytes: 8_000_000_000 + index * 290_000_000 + tick * diskWriteBps,
    other_io_total_bytes: 1_000_000_000 + index * 85_000_000 + tick * otherIoBps,
    disk_read_bps: diskReadBps,
    disk_write_bps: diskWriteBps,
    other_io_bps: otherIoBps,
    network_received_bps: networkReceived,
    network_transmitted_bps: networkTransmitted,
    threads: 4 + index * 2,
    handles: 80 + index * 17,
    access_state: "full",
    quality: {
      cpu: { quality: "estimated", source: "fixture" },
      memory: { quality: "estimated", source: "fixture" },
      disk: { quality: "estimated", source: "fixture" },
      other_io: { quality: "estimated", source: "fixture" },
      network: { quality: "estimated", source: "fixture" },
      threads: { quality: "estimated", source: "fixture" },
      handles: { quality: "estimated", source: "fixture" },
    },
  };
}

function fixtureProcessName(index: number): string {
  if (index < names.length) {
    return names[index];
  }

  const family = backgroundNames[index % backgroundNames.length].replace(".exe", "");
  const suffix = String(index - names.length + 1).padStart(3, "0");
  return `${family}-${suffix}.exe`;
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
