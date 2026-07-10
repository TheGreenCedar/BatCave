import type {
  ProcessFocusMode,
  ProcessSample,
  ProcessViewRow,
  RuntimeQuery,
  SortColumn,
  SortDirection,
} from "./types";

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
  { value: "all", label: "All apps" },
  { value: "attention", label: "Attention" },
  { value: "io", label: "I/O active" },
];

export const sortOptions: { value: SortKey; label: string }[] = [
  { value: "attention", label: "Attention" },
  { value: "cpu", label: "CPU" },
  { value: "memory", label: "Working set" },
  { value: "io", label: "I/O" },
  { value: "network", label: "Network" },
  { value: "name", label: "Name" },
];

export const processColumns: ProcessColumn[] = [
  { key: "name", label: "App or process" },
  { key: "attention", label: "Impact" },
  { key: "cpu", label: "CPU", metric: true },
  { key: "memory", label: "Memory", metric: true },
  { key: "io", label: "I/O", metric: true },
  { key: "network", label: "Network", metric: true },
];

export function processViewRowKey(row: ProcessViewRow): string {
  if (row.kind === "group") {
    return `group:${row.group_key ?? row.group_label ?? "unknown"}`;
  }

  return row.process ? processSelectionKey(row.process) : "process:unknown";
}

export function processSelectionKey(process: Pick<ProcessSample, "pid" | "start_time_ms">): string {
  return `process:${process.pid}:${process.start_time_ms}`;
}

export function processNeedsAttention(process: ProcessSample): boolean {
  return (
    process.cpu_percent >= 1 ||
    process.memory_bytes >= 900 * 1024 * 1024 ||
    rawProcessIoRate(process) >= 500 * 1024 ||
    rawProcessNetworkRate(process) >= 1024 * 1024 ||
    process.access_state !== "full"
  );
}

export function compareProcessSamples(
  left: ProcessSample,
  right: ProcessSample,
  query: RuntimeQuery,
): number {
  const comparison =
    query.sort_column === "name"
      ? left.name.localeCompare(right.name)
      : query.sort_column === "pid"
        ? left.pid.localeCompare(right.pid)
        : query.sort_column === "cpu_pct"
          ? left.cpu_percent - right.cpu_percent
          : query.sort_column === "memory_bytes"
            ? left.memory_bytes - right.memory_bytes
            : query.sort_column === "disk_bps"
              ? rawProcessIoRate(left) - rawProcessIoRate(right)
              : query.sort_column === "network_bps"
                ? rawProcessNetworkRate(left) - rawProcessNetworkRate(right)
                : query.sort_column === "threads"
                  ? left.threads - right.threads
                  : query.sort_column === "handles"
                    ? left.handles - right.handles
                    : query.sort_column === "start_time_ms"
                      ? left.start_time_ms - right.start_time_ms
                      : processAttentionScore(left) - processAttentionScore(right);

  const directed = query.sort_direction === "asc" ? comparison : -comparison;
  return directed || left.name.localeCompare(right.name);
}

function rawProcessIoRate(process: ProcessSample): number {
  return process.disk_read_bps + process.disk_write_bps + (process.other_io_bps ?? 0);
}

function rawProcessNetworkRate(process: ProcessSample): number {
  return (process.network_received_bps ?? 0) + (process.network_transmitted_bps ?? 0);
}

function processAttentionScore(process: ProcessSample): number {
  return (
    process.cpu_percent * 3 +
    Math.min(process.memory_bytes / (128 * 1024 * 1024), 20) +
    Math.min(rawProcessIoRate(process) / (512 * 1024), 20) +
    Math.min(rawProcessNetworkRate(process) / (1024 * 1024), 20) +
    (process.access_state === "full" ? 0 : 12)
  );
}

export function hasSameProcessOrder(
  current: ProcessViewRow[],
  incoming: ProcessViewRow[],
): boolean {
  if (current.length !== incoming.length) {
    return false;
  }

  return current.every(
    (row, index) => processViewRowKey(row) === processViewRowKey(incoming[index]),
  );
}

export function stabilizeProcessRows(
  current: ProcessViewRow[],
  incoming: ProcessViewRow[],
): ProcessViewRow[] {
  if (current.length === 0) {
    return incoming;
  }

  const incomingByKey = new Map(incoming.map((row) => [processViewRowKey(row), row]));
  const stable = current.flatMap((row) => {
    const next = incomingByKey.get(processViewRowKey(row));
    return next ? [next] : [];
  });
  const stableKeys = new Set(stable.map(processViewRowKey));

  return [...stable, ...incoming.filter((row) => !stableKeys.has(processViewRowKey(row)))];
}

export function windowProcessViewRows(
  rows: ProcessViewRow[],
  visibleRowLimit: number,
): ProcessViewRow[] {
  const windowed: ProcessViewRow[] = [];
  let visibleRowCount = 0;

  for (const row of rows) {
    const isVisibleByDefault = row.kind === "group" || !row.is_grouped;
    if (isVisibleByDefault) {
      if (visibleRowCount >= Math.max(1, visibleRowLimit)) {
        break;
      }
      visibleRowCount += 1;
    }

    windowed.push(row);
  }

  return windowed;
}

export function shouldStabilizeProcessOrder(sortKey: SortKey): boolean {
  return sortKey === "attention";
}

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
  const rates = processRates[processSelectionKey(process)];
  return (
    (rates?.readRate ?? process.disk_read_bps) +
    (rates?.writeRate ?? process.disk_write_bps) +
    (rates?.otherRate ?? process.other_io_bps ?? 0)
  );
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
