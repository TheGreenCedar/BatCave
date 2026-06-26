import type { ProcessSample, SortColumn, SortDirection } from "./types";

export type FocusMode = "all" | "active" | "io";
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
  { value: "active", label: "Active" },
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
  { key: "cpu", label: "CPU %", metric: true },
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
  network: "disk_bps",
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

export function compareProcesses(
  left: ProcessSample,
  right: ProcessSample,
  key: SortKey,
  direction: SortDirection,
  processRates: Record<string, ProcessRates>,
  totalMemoryBytes: number,
): number {
  const factor = direction === "asc" ? 1 : -1;

  switch (key) {
    case "attention":
      return (
        compareNumber(
          attentionScore(left, processRates, totalMemoryBytes),
          attentionScore(right, processRates, totalMemoryBytes),
          direction,
        ) || compareText(left.name, right.name)
      );
    case "name":
      return (
        compareText(left.name, right.name) * factor || compareText(left.pid, right.pid) * factor
      );
    case "pid":
      return (
        comparePid(left.pid, right.pid) * factor || compareText(left.name, right.name) * factor
      );
    case "memory":
      return (
        compareNumber(left.memory_bytes, right.memory_bytes, direction) ||
        compareNumber(left.cpu_percent, right.cpu_percent, "desc")
      );
    case "io":
      return (
        compareNumber(
          processIoRate(left, processRates),
          processIoRate(right, processRates),
          direction,
        ) || compareNumber(left.cpu_percent, right.cpu_percent, "desc")
      );
    case "network":
      return (
        compareNumber(processNetworkRate(left), processNetworkRate(right), direction) ||
        compareNumber(left.cpu_percent, right.cpu_percent, "desc")
      );
    case "read":
      return (
        compareNumber(left.disk_read_total_bytes, right.disk_read_total_bytes, direction) ||
        compareText(left.name, right.name)
      );
    case "write":
      return (
        compareNumber(left.disk_write_total_bytes, right.disk_write_total_bytes, direction) ||
        compareText(left.name, right.name)
      );
    case "status":
      return compareText(left.status, right.status) * factor || compareText(left.name, right.name);
    case "threads":
      return (
        compareNumber(left.threads, right.threads, direction) || compareText(left.name, right.name)
      );
    case "cpu":
    default:
      return (
        compareNumber(left.cpu_percent, right.cpu_percent, direction) ||
        compareNumber(left.memory_bytes, right.memory_bytes, "desc")
      );
  }
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

export function matchesSearch(process: ProcessSample, query: string): boolean {
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

export function matchesFocusMode(
  process: ProcessSample,
  mode: FocusMode,
  processRates: Record<string, ProcessRates>,
): boolean {
  if (mode === "active") {
    return process.cpu_percent >= 1;
  }

  if (mode === "io") {
    return processIoRate(process, processRates) > 0;
  }

  return true;
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

  return "Stable";
}

export function processHint(
  process: ProcessSample,
  processRates: Record<string, ProcessRates>,
): string {
  if (process.cpu_percent >= 20) {
    return "CPU lead";
  }

  if (process.memory_bytes >= 900 * 1024 * 1024) {
    return "memory lead";
  }

  if (processIoRate(process, processRates) >= 500 * 1024) {
    return "I/O lead";
  }

  return "steady";
}

export function processNetworkRate(process: ProcessSample): number {
  return (process.network_received_bps ?? 0) + (process.network_transmitted_bps ?? 0);
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

function attentionScore(
  process: ProcessSample,
  processRates: Record<string, ProcessRates>,
  totalMemoryBytes: number,
): number {
  const cpuWeight = process.cpu_percent * 6;
  const memoryWeight = percentage(process.memory_bytes, Math.max(totalMemoryBytes, 1)) * 2;
  const ioWeight = Math.min(processIoRate(process, processRates) / 1024 / 1024, 100) * 3;
  const accessWeight =
    process.access_state === "denied" ? 18 : process.access_state === "partial" ? 8 : 0;
  return cpuWeight + memoryWeight + ioWeight + accessWeight;
}

function percentage(value: number, total: number): number {
  if (total <= 0) {
    return 0;
  }

  return Math.min(100, Math.max(0, (value / total) * 100));
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

function matchesAny(value: string, needles: string[]): boolean {
  return needles.some((needle) => value.includes(needle));
}
