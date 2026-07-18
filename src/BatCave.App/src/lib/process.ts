import type {
  GroupDetail,
  MetricCoverage,
  MetricQualityInfo,
  ProcessFocusMode,
  ProcessSample,
  ProcessViewRow,
  WorkloadDetail,
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
  otherRate?: number;
}

export interface WorkloadMetrics {
  cpuPercent: number;
  memoryBytes: number;
  ioBps: number;
  networkBps: number;
  threads: number;
}

export type ProcessIconKind =
  | "batcave"
  | "apple"
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
  | "terminal"
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
  { value: "cpu", label: "CPU (one core)" },
  { value: "memory", label: "Resident memory" },
  { value: "io", label: "I/O" },
  { value: "network", label: "Network" },
  { value: "name", label: "Name" },
];

export const processColumns: ProcessColumn[] = [
  { key: "name", label: "Workload" },
  { key: "attention", label: "Status" },
  { key: "cpu", label: "CPU / core", metric: true },
  { key: "memory", label: "Resident memory", metric: true },
  { key: "io", label: "Read/write I/O", metric: true },
  { key: "network", label: "Network", metric: true },
];

export function processViewRowKey(row: ProcessViewRow): string {
  return row.detail.workload_id;
}

export function processRowSecondaryLabel(row: ProcessViewRow): string | null {
  if (row.kind === "group") {
    return String(row.detail.process_count);
  }

  if (row.is_grouped) {
    return `PID ${row.detail.process.pid}`;
  }

  const category = row.group_category?.trim();
  return category && category.toLocaleLowerCase() !== "processes" ? category : null;
}

export function processViewRowMetrics(row: ProcessViewRow): WorkloadMetrics {
  if (row.kind === "group") {
    return {
      cpuPercent: row.detail.cpu_percent,
      memoryBytes: row.detail.memory_bytes,
      ioBps: row.detail.io_bps,
      networkBps: row.detail.network_bps,
      threads: row.detail.threads,
    };
  }

  return {
    cpuPercent: row.detail.process.cpu_percent,
    memoryBytes: row.detail.process.memory_bytes,
    ioBps: row.detail.io_bps,
    networkBps: row.detail.network_bps,
    threads: row.detail.process.threads,
  };
}

export function selectedWorkloadDetail(
  rows: ProcessViewRow[],
  selection: string,
): WorkloadDetail | null {
  return rows.find((row) => processViewRowKey(row) === selection)?.detail ?? null;
}

export function isProcessViewRow(value: unknown): value is ProcessViewRow {
  if (!isRecord(value) || !isRecord(value.detail) || typeof value.icon_kind !== "string") {
    return false;
  }
  const detail = value.detail;

  if (value.kind === "process") {
    return (
      detail.kind === "process" &&
      typeof detail.workload_id === "string" &&
      detail.workload_id.startsWith("process:") &&
      isRecord(detail.process) &&
      typeof detail.process.pid === "string" &&
      typeof detail.process.start_time_ms === "number" &&
      typeof detail.io_bps === "number" &&
      typeof detail.network_bps === "number" &&
      typeof value.group_key === "string" &&
      typeof value.group_label === "string" &&
      typeof value.group_category === "string" &&
      typeof value.group_count === "number" &&
      typeof value.is_child === "boolean" &&
      typeof value.is_grouped === "boolean"
    );
  }

  if (value.kind !== "group" || detail.kind !== "group") return false;
  if (
    ["pid", "parent_pid", "exe", "access_state", "process", "representative"].some(
      (key) => key in detail,
    )
  ) {
    return false;
  }
  if ("process" in value || "representative" in value) return false;

  return (
    typeof detail.workload_id === "string" &&
    detail.workload_id.startsWith("group:") &&
    typeof detail.group_key === "string" &&
    typeof detail.label === "string" &&
    typeof detail.category === "string" &&
    typeof detail.process_count === "number" &&
    typeof detail.cpu_percent === "number" &&
    typeof detail.memory_bytes === "number" &&
    typeof detail.io_bps === "number" &&
    typeof detail.network_bps === "number" &&
    typeof detail.threads === "number" &&
    isRecord(detail.quality) &&
    isRecord(detail.quality.other_io) &&
    isRecord(detail.coverage) &&
    isRecord(detail.coverage.other_io)
  );
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

export function processSelectionKey(process: Pick<ProcessSample, "pid" | "start_time_ms">): string {
  return `process:${process.pid}:${process.start_time_ms}`;
}

const attentionCpuPercent = 10;
const attentionMemoryBytes = 900 * 1024 * 1024;
const attentionIoBps = 500 * 1024;
const attentionNetworkBps = 1024 * 1024;

export function processNeedsAttention(process: ProcessSample): boolean {
  return (
    process.cpu_percent >= attentionCpuPercent ||
    process.memory_bytes >= attentionMemoryBytes ||
    rawProcessIoRate(process) >= attentionIoBps ||
    rawProcessNetworkRate(process) >= attentionNetworkBps ||
    process.access_state !== "full"
  );
}

function rawProcessIoRate(process: ProcessSample): number {
  return process.io_read_bps + process.io_write_bps;
}

function rawProcessNetworkRate(process: ProcessSample): number {
  return (process.network_received_bps ?? 0) + (process.network_transmitted_bps ?? 0);
}

type ProcessAttentionMetricState =
  | "native"
  | "estimated"
  | "partial"
  | "held"
  | "unavailable"
  | "missing";

interface ProcessAttentionMetric {
  thresholdReached: boolean;
  label: string;
  quality?: MetricQualityInfo;
  hasValue: boolean;
}

function processAttentionMetricState(metric: ProcessAttentionMetric): ProcessAttentionMetricState {
  if (!metric.quality) return "missing";
  if (metric.quality.quality === "held") return "held";
  if (metric.quality.quality === "unavailable" || !metric.hasValue) return "unavailable";
  return metric.quality.quality;
}

function processActivityAttentionLabel(
  metric: ProcessAttentionMetric,
  state: "native" | "estimated" | "partial",
  allMetricsComplete: boolean,
): string {
  const qualifiers: string[] = [];
  if (state === "partial") qualifiers.push("limited");
  if (state === "estimated") qualifiers.push("estimated");
  if (!allMetricsComplete && state !== "partial") qualifiers.push("telemetry limited");
  return qualifiers.length > 0 ? `${metric.label} · ${qualifiers.join(" · ")}` : metric.label;
}

export function processAttentionLabel(process: ProcessSample): string {
  const metrics: ProcessAttentionMetric[] = [
    {
      thresholdReached: process.cpu_percent >= attentionCpuPercent,
      label: "CPU activity",
      quality: process.quality?.cpu,
      hasValue: true,
    },
    {
      thresholdReached: process.memory_bytes >= attentionMemoryBytes,
      label: "memory activity",
      quality: process.quality?.memory,
      hasValue: true,
    },
    {
      thresholdReached: rawProcessIoRate(process) >= attentionIoBps,
      label: "I/O activity",
      quality: process.quality?.io,
      hasValue: true,
    },
    {
      thresholdReached: rawProcessNetworkRate(process) >= attentionNetworkBps,
      label: "network activity",
      quality: process.quality?.network,
      hasValue:
        process.network_received_bps !== undefined || process.network_transmitted_bps !== undefined,
    },
  ];
  const states = metrics.map(processAttentionMetricState);
  const allMetricsComplete = states.every((state) => state === "native" || state === "estimated");

  for (let index = 0; index < metrics.length; index += 1) {
    const metric = metrics[index];
    const state = states[index];
    if (
      metric.thresholdReached &&
      (state === "native" || state === "estimated" || state === "partial")
    ) {
      return processActivityAttentionLabel(metric, state, allMetricsComplete);
    }
  }

  if (states.every((state) => state === "unavailable")) return "Unavailable";
  if (states.includes("held")) return "Pending";
  if (states.includes("missing") || states.includes("partial") || states.includes("unavailable")) {
    return "Limited";
  }
  if (process.access_state !== "full") return "access limited";
  if (states.includes("estimated")) return "steady · estimated";
  return "steady";
}

function groupMetricCanDisplayForAttention(
  quality: MetricQualityInfo,
  coverage: MetricCoverage,
): boolean {
  return coverage.available > 0 && quality.quality !== "held" && quality.quality !== "unavailable";
}

function groupMetricIsCompleteForAttention(
  quality: MetricQualityInfo,
  coverage: MetricCoverage,
): boolean {
  return (
    groupMetricCanDisplayForAttention(quality, coverage) &&
    quality.quality !== "partial" &&
    coverage.available === coverage.total
  );
}

function groupActivityLabel(
  label: string,
  quality: MetricQualityInfo,
  coverage: MetricCoverage,
  allMetricsComplete: boolean,
): string {
  if (quality.quality === "partial" || coverage.available < coverage.total) {
    return `${label} · ${coverage.available}/${coverage.total} · limited`;
  }
  if (!allMetricsComplete) return `${label} · telemetry limited`;
  if (quality.quality === "estimated") return `${label} · estimated`;
  return label;
}

export function groupAttentionLabel(detail: GroupDetail, limitedAccess: boolean): string {
  const metrics = [
    [detail.quality.cpu, detail.coverage.cpu],
    [detail.quality.memory, detail.coverage.memory],
    [detail.quality.io, detail.coverage.io],
    [detail.quality.network, detail.coverage.network],
  ] as const;
  const allMetricsComplete = metrics.every(([quality, coverage]) =>
    groupMetricIsCompleteForAttention(quality, coverage),
  );
  const activities = [
    [detail.cpu_percent >= 10, "CPU activity", detail.quality.cpu, detail.coverage.cpu],
    [
      detail.memory_bytes >= 900 * 1024 * 1024,
      "memory activity",
      detail.quality.memory,
      detail.coverage.memory,
    ],
    [detail.io_bps >= 500 * 1024, "I/O activity", detail.quality.io, detail.coverage.io],
    [
      detail.network_bps >= 1024 * 1024,
      "network activity",
      detail.quality.network,
      detail.coverage.network,
    ],
  ] as const;

  for (const [thresholdReached, label, quality, coverage] of activities) {
    if (thresholdReached && groupMetricCanDisplayForAttention(quality, coverage)) {
      return groupActivityLabel(label, quality, coverage, allMetricsComplete);
    }
  }

  if (allMetricsComplete) return limitedAccess ? "access limited" : "steady";

  const state = metrics.every(([quality]) => quality.quality === "held")
    ? "Pending"
    : metrics.every(([quality]) => quality.quality === "unavailable")
      ? "Unavailable"
      : "Limited";
  const coverage = metrics.find(
    ([quality, metricCoverage]) => !groupMetricIsCompleteForAttention(quality, metricCoverage),
  )?.[1] ?? { available: 0, total: detail.process_count };
  return `${state} · ${coverage.available}/${coverage.total} coverage`;
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

export function prepareProcessViewRows(
  rows: ProcessViewRow[],
  selection: string,
  visibleRowLimit: number,
): { rows: ProcessViewRow[]; selection: string } {
  const windowedRows = windowProcessViewRows(rows, visibleRowLimit);
  return {
    rows: windowedRows,
    selection: reconcileWorkloadSelection(windowedRows, selection),
  };
}

export function workloadSelectionMatchesRow(row: ProcessViewRow, selection: string): boolean {
  return row.detail.workload_id === selection;
}

export function workloadSelectionHighlightsRow(
  rows: ProcessViewRow[],
  row: ProcessViewRow,
  selection: string,
): boolean {
  if (workloadSelectionMatchesRow(row, selection) || row.kind === "process") {
    return workloadSelectionMatchesRow(row, selection);
  }

  return rows.some(
    (candidate) =>
      candidate.kind === "process" &&
      candidate.group_key === row.detail.group_key &&
      workloadSelectionMatchesRow(candidate, selection),
  );
}

export function shouldStabilizeProcessOrder(sortKey: SortKey): boolean {
  return sortKey === "attention";
}

export function shouldHoldProcessOrder(
  sortKey: SortKey,
  interacting: boolean,
  expandedGroupCount: number,
  selectedWorkloadVisible: boolean,
): boolean {
  return (
    interacting ||
    (shouldStabilizeProcessOrder(sortKey) && (expandedGroupCount > 0 || selectedWorkloadVisible))
  );
}

export function reconcileWorkloadSelection(rows: ProcessViewRow[], selection: string): string {
  return rows.some((row) => processViewRowKey(row) === selection) ? selection : "";
}

const sortColumnByKey: Record<SortKey, SortColumn> = {
  attention: "attention",
  name: "name",
  pid: "pid",
  cpu: "cpu_pct",
  memory: "memory_bytes",
  io: "io_bps",
  network: "network_bps",
  read: "io_bps",
  write: "io_bps",
  status: "name",
  threads: "threads",
};

const sortKeyByColumn: Partial<Record<SortColumn, SortKey>> = {
  attention: "attention",
  cpu_pct: "cpu",
  memory_bytes: "memory",
  io_bps: "io",
  network_bps: "network",
  name: "name",
  pid: "pid",
};

export function processIoRate(
  process: ProcessSample,
  processRates: Record<string, ProcessRates>,
): number {
  const rates = processRates[processSelectionKey(process)];
  return (rates?.readRate ?? process.io_read_bps) + (rates?.writeRate ?? process.io_write_bps);
}

export function processOtherIoRate(
  process: ProcessSample,
  processRates: Record<string, ProcessRates>,
): number | undefined {
  const rates = processRates[processSelectionKey(process)];
  return rates?.otherRate ?? process.other_io_bps;
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

export function nextSortDirection(direction: SortDirection): SortDirection {
  return direction === "asc" ? "desc" : "asc";
}

export function sortDirectionButtonLabel(direction: SortDirection): string {
  const current = direction === "asc" ? "ascending" : "descending";
  const next = direction === "asc" ? "descending" : "ascending";
  return `Sort direction: ${current}. Change to ${next}.`;
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

  if (matchesAny(haystack, ["chrome", "msedge", "firefox", "brave", "browser", "safari"])) {
    return { icon: "browser", group: "Browsers", isChild };
  }

  if (matchesAny(haystack, ["code.exe", "visual studio code", "\\code\\", "/code/"])) {
    return { icon: "code", group: "Developer tools", isChild };
  }

  if (matchesAny(haystack, ["node", "npm", "deno", "bun.exe"])) {
    return { icon: "node", group: "Runtimes", isChild };
  }

  if (matchesAny(haystack, ["iterm", "terminal.app", "windowsterminal", "kitty"])) {
    return { icon: "terminal", group: "Terminals", isChild };
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
    matchesAny(haystack, ["finder", "kernel_task", "windowserver", "controlcenter", "mds_stores"])
  ) {
    return { icon: "apple", group: "macOS", isChild };
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
