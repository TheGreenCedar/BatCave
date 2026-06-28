import type { ProcessFocusMode, ProcessSample, SortColumn, SortDirection } from "./types";

export type FocusMode = ProcessFocusMode;
export type SortKey =
  | "attention"
  | "name"
  | "pid"
  | "cpu"
  | "memory"
  | "io"
  | "network"
  | "read"
  | "write"
  | "status"
  | "threads";

export interface ProcessColumn {
  key: SortKey;
  label: string;
  metric?: boolean;
}

export interface ProcessRates {
  readRate: number;
  writeRate: number;
  otherRate: number;
}

export type ProcessIconKind =
  | "batcave"
  | "browser"
  | "code"
  | "chat"
  | "container"
  | "database"
  | "gpu"
  | "media"
  | "node"
  | "process"
  | "sync"
  | "windows";

export interface ProcessIdentity {
  icon: ProcessIconKind;
  group: string;
  isChild: boolean;
}

export const focusOptions: { value: FocusMode; label: string }[] = [
  { value: "all", label: "All" },
  { value: "active", label: "Busy" },
  { value: "io", label: "I/O" },
];

export const sortOptions: { value: SortKey; label: string }[] = [
  { value: "attention", label: "Attention" },
  { value: "cpu", label: "CPU" },
  { value: "memory", label: "Working set" },
  { value: "io", label: "I/O" },
  { value: "name", label: "Name" },
];

export const processColumns: ProcessColumn[] = [
  { key: "pid", label: "PID" },
  { key: "name", label: "Process" },
  { key: "status", label: "Status" },
  { key: "cpu", label: "CPU/core", metric: true },
  { key: "memory", label: "Memory", metric: true },
  { key: "io", label: "Disk I/O", metric: true },
  { key: "network", label: "Network", metric: true },
  { key: "threads", label: "Threads", metric: true },
];

const sortColumnByKey: Record<SortKey, SortColumn> = {
  attention: "attention",
  name: "name",
  pid: "pid",
  cpu: "cpu_pct",
  memory: "memory_bytes",
  io: "disk_bps",
  network: "network_bps",
  read: "disk_bps",
  write: "disk_bps",
  status: "name",
  threads: "threads",
};

const sortKeyByColumn: Partial<Record<SortColumn, SortKey>> = {
  attention: "attention",
  cpu_pct: "cpu",
  memory_bytes: "memory",
  disk_bps: "io",
  network_bps: "network",
  name: "name",
  pid: "pid",
};

export function processIoRate(
  process: ProcessSample,
  processRates: Record<string, ProcessRates>,
): number {
  const rates = processRates[process.pid];
  return (rates?.readRate ?? 0) + (rates?.writeRate ?? 0) + (rates?.otherRate ?? 0);
}

export function defaultSortDirection(key: SortKey): SortDirection {
  return key === "attention" ||
    key === "cpu" ||
    key === "memory" ||
    key === "io" ||
    key === "network" ||
    key === "threads"
    ? "desc"
    : "asc";
}

export function sortColumnForKey(key: SortKey): SortColumn {
  return sortColumnByKey[key];
}

export function sortKeyForColumn(column: SortColumn): SortKey {
  return sortKeyByColumn[column] ?? "attention";
}

export function sortAriaValue(
  key: SortKey,
  activeKey: SortKey,
  direction: SortDirection,
): "ascending" | "descending" | "none" {
  if (activeKey !== key) {
    return "none";
  }

  return direction === "asc" ? "ascending" : "descending";
}

export function sortButtonLabel(
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

export function sortIndicator(key: SortKey, activeKey: SortKey, direction: SortDirection): string {
  if (activeKey !== key) {
    return "";
  }

  return direction === "asc" ? "Asc" : "Desc";
}

export function processAccent(
  process: ProcessSample | undefined,
  processRates: Record<string, ProcessRates>,
): string {
  if (!process) {
    return "Idle";
  }

  if (process.cpu_percent >= 30) {
    return "Hot";
  }

  if (process.memory_bytes >= 900 * 1024 * 1024) {
    return "Heavy";
  }

  if (processIoRate(process, processRates) >= 500 * 1024) {
    return "I/O";
  }

  return "Normal";
}

export function processIdentity(process: ProcessSample): ProcessIdentity {
  const haystack = `${process.name} ${process.exe}`.toLocaleLowerCase();
  const name = process.name.toLocaleLowerCase();

  const isChild =
    name.startsWith("--") ||
    haystack.includes("--type=") ||
    haystack.includes("renderer") ||
    haystack.includes("gpu-process") ||
    haystack.includes("utility");

  if (haystack.includes("batcave")) {
    return { icon: "batcave", group: "BatCave", isChild };
  }

  if (matchesAny(haystack, ["chrome", "msedge", "firefox", "brave", "browser"])) {
    return { icon: "browser", group: "Browsers", isChild };
  }

  if (matchesAny(haystack, ["code.exe", "visual studio code", "\\code\\", "/code/"])) {
    return { icon: "code", group: "Developer tools", isChild };
  }

  if (matchesAny(haystack, ["node", "npm", "deno", "bun.exe"])) {
    return { icon: "node", group: "Runtimes", isChild };
  }

  if (haystack.includes("docker")) {
    return { icon: "container", group: "Containers", isChild };
  }

  if (matchesAny(haystack, ["postgres", "mysql", "redis", "sqlserver", "mariadb"])) {
    return { icon: "database", group: "Databases", isChild };
  }

  if (matchesAny(haystack, ["slack", "teams", "discord", "zoom"])) {
    return { icon: "chat", group: "Communication", isChild };
  }

  if (matchesAny(haystack, ["spotify", "vlc", "media player"])) {
    return { icon: "media", group: "Media", isChild };
  }

  if (matchesAny(haystack, ["dropbox", "onedrive", "googledrive"])) {
    return { icon: "sync", group: "Sync", isChild };
  }

  if (matchesAny(haystack, ["nvidia", "amd", "radeon", "intel graphics"])) {
    return { icon: "gpu", group: "GPU", isChild };
  }

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
    return { icon: "windows", group: "Windows", isChild };
  }

  return { icon: "process", group: "Processes", isChild };
}

function matchesAny(value: string, needles: string[]): boolean {
  return needles.some((needle) => value.includes(needle));
}
