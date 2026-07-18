import {
  groupMetricCanDisplay,
  logicalCpuMetricQuality,
  nextProcessMetricHistory,
  processMemoryQuality,
} from "./format.ts";
import { nextMetricHistory } from "./history.ts";
import { processSelectionKey, type ProcessRates, type WorkloadMetrics } from "./process.ts";
import type { ProcessSample, RuntimeSnapshot, TrendState, WorkloadDetail } from "./types.ts";

export interface ProcessTrendState {
  cpu: number[];
  memory: number[];
  readRate: number[];
  writeRate: number[];
  networkRate: number[];
}

type HistoricalGroupMetric = "cpu" | "memory" | "io" | "network";

export function emptyTrendState(): TrendState {
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

export function emptyProcessTrendState(): ProcessTrendState {
  return {
    cpu: [],
    memory: [],
    readRate: [],
    writeRate: [],
    networkRate: [],
  };
}

export function nextSystemHistory(
  previous: TrendState,
  snapshot: RuntimeSnapshot,
  pointLimit: number,
): TrendState {
  const logicalCpu = snapshot.system.logical_cpu_percent.length
    ? snapshot.system.logical_cpu_percent
    : [snapshot.system.cpu_percent];

  return {
    cpu: nextMetricHistory(
      previous.cpu,
      snapshot.system.cpu_percent,
      snapshot.system.quality?.cpu,
      pointLimit,
    ),
    memory: nextMetricHistory(
      previous.memory,
      percentage(snapshot.system.memory_used_bytes, snapshot.system.memory_total_bytes),
      snapshot.system.quality?.memory,
      pointLimit,
    ),
    swap:
      snapshot.system.swap_total_bytes && snapshot.system.swap_used_bytes !== undefined
        ? appendPoint(
            previous.swap,
            percentage(snapshot.system.swap_used_bytes, snapshot.system.swap_total_bytes),
            pointLimit,
          )
        : previous.swap,
    diskRead: nextMetricHistory(
      previous.diskRead,
      snapshot.system.disk_read_bps,
      snapshot.system.quality?.disk,
      pointLimit,
    ),
    diskWrite: nextMetricHistory(
      previous.diskWrite,
      snapshot.system.disk_write_bps,
      snapshot.system.quality?.disk,
      pointLimit,
    ),
    netRx: nextMetricHistory(
      previous.netRx,
      snapshot.system.network_received_bps,
      snapshot.system.quality?.network,
      pointLimit,
    ),
    netTx: nextMetricHistory(
      previous.netTx,
      snapshot.system.network_transmitted_bps,
      snapshot.system.quality?.network,
      pointLimit,
    ),
    cores: logicalCpu.map((value, index) =>
      nextMetricHistory(
        previous.cores[index] ?? [],
        value,
        logicalCpuMetricQuality(snapshot.system.quality ?? {}),
        pointLimit,
      ),
    ),
  };
}

export function processRatesFromSamples(processes: ProcessSample[]): Record<string, ProcessRates> {
  const rates: Record<string, ProcessRates> = {};
  for (const process of processes) {
    rates[processSelectionKey(process)] = {
      readRate: process.io_read_bps,
      otherRate: process.other_io_bps,
      writeRate: process.io_write_bps,
    };
  }
  return rates;
}

export function initialWorkloadTrend(
  workload: WorkloadDetail,
  memoryTotalBytes: number,
  processRates: Record<string, ProcessRates>,
  pointLimit: number,
): ProcessTrendState {
  const rates = workloadTrendRates(workload, processRates);
  const metrics = workloadMetrics(workload);
  return {
    cpu: initialWorkloadMetric(metrics.cpuPercent, workload, "cpu", pointLimit),
    memory: initialWorkloadMetric(
      percentage(metrics.memoryBytes, Math.max(memoryTotalBytes, 1)),
      workload,
      "memory",
      pointLimit,
    ),
    readRate: initialWorkloadMetric(rates.readRate, workload, "io", pointLimit),
    writeRate: initialWorkloadMetric(rates.writeRate, workload, "io", pointLimit),
    networkRate: initialWorkloadMetric(rates.networkRate, workload, "network", pointLimit),
  };
}

export function nextWorkloadTrend(
  previous: ProcessTrendState,
  workload: WorkloadDetail,
  memoryTotalBytes: number,
  processRates: Record<string, ProcessRates>,
  pointLimit: number,
): ProcessTrendState {
  const rates = workloadTrendRates(workload, processRates);
  const metrics = workloadMetrics(workload);
  return {
    cpu: nextWorkloadMetric(previous.cpu, metrics.cpuPercent, workload, "cpu", pointLimit),
    memory: nextWorkloadMetric(
      previous.memory,
      percentage(metrics.memoryBytes, Math.max(memoryTotalBytes, 1)),
      workload,
      "memory",
      pointLimit,
    ),
    readRate: nextWorkloadMetric(previous.readRate, rates.readRate, workload, "io", pointLimit),
    writeRate: nextWorkloadMetric(previous.writeRate, rates.writeRate, workload, "io", pointLimit),
    networkRate: nextWorkloadMetric(
      previous.networkRate,
      rates.networkRate,
      workload,
      "network",
      pointLimit,
    ),
  };
}

export function workloadMetrics(workload: WorkloadDetail): WorkloadMetrics {
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

export function workloadTrendRates(
  workload: WorkloadDetail,
  processRates: Record<string, ProcessRates>,
): ProcessRates & { networkRate: number } {
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

export function trimSystemHistory(history: TrendState, pointLimit: number): TrendState {
  return {
    cpu: trimPoints(history.cpu, pointLimit),
    memory: trimPoints(history.memory, pointLimit),
    swap: trimPoints(history.swap, pointLimit),
    diskRead: trimPoints(history.diskRead, pointLimit),
    diskWrite: trimPoints(history.diskWrite, pointLimit),
    netRx: trimPoints(history.netRx, pointLimit),
    netTx: trimPoints(history.netTx, pointLimit),
    cores: history.cores.map((points) => trimPoints(points, pointLimit)),
  };
}

export function trimProcessHistory(
  history: ProcessTrendState,
  pointLimit: number,
): ProcessTrendState {
  return {
    cpu: trimPoints(history.cpu, pointLimit),
    memory: trimPoints(history.memory, pointLimit),
    readRate: trimPoints(history.readRate, pointLimit),
    writeRate: trimPoints(history.writeRate, pointLimit),
    networkRate: trimPoints(history.networkRate, pointLimit),
  };
}

export function combineSeries(left: number[], right: number[]): number[] {
  const length = Math.max(left.length, right.length);
  const leftOffset = length - left.length;
  const rightOffset = length - right.length;
  return Array.from(
    { length },
    (_, index) => (left[index - leftOffset] ?? 0) + (right[index - rightOffset] ?? 0),
  );
}

export function percentage(value: number, total: number): number {
  if (total <= 0) return 0;
  return Math.min(100, Math.max(0, (value / total) * 100));
}

export function boundedPercent(value: number): number {
  return Math.min(100, Math.max(0, Number.isFinite(value) ? value : 0));
}

export function maxRate(points: number[], fallback: number): number {
  return Math.max(fallback, Math.max(...points, 0) * 1.2);
}

function initialWorkloadMetric(
  value: number,
  workload: WorkloadDetail,
  metric: HistoricalGroupMetric,
  pointLimit: number,
): number[] {
  if (workload.kind === "group") {
    return groupMetricCanDisplay(workload.quality[metric], workload.coverage[metric])
      ? [value]
      : [];
  }
  const quality =
    metric === "memory"
      ? processMemoryQuality(workload.process)
      : workload.process.quality?.[metric];
  return nextProcessMetricHistory([], value, quality, pointLimit);
}

function nextWorkloadMetric(
  values: number[],
  value: number,
  workload: WorkloadDetail,
  metric: HistoricalGroupMetric,
  pointLimit: number,
): number[] {
  if (workload.kind === "group") {
    return groupMetricCanDisplay(workload.quality[metric], workload.coverage[metric])
      ? appendPoint(values, value, pointLimit)
      : [];
  }
  const quality =
    metric === "memory"
      ? processMemoryQuality(workload.process)
      : workload.process.quality?.[metric];
  return nextProcessMetricHistory(values, value, quality, pointLimit);
}

function appendPoint(points: number[], value: number, pointLimit: number): number[] {
  return trimPoints([...points, Number.isFinite(value) ? value : 0], pointLimit);
}

function trimPoints(points: number[], pointLimit: number): number[] {
  return points.slice(-pointLimit);
}
