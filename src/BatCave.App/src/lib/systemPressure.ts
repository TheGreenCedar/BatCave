import type { ProcessSample } from "./types";

export function systemPressureHeadline(
  cpuPercent: number,
  memoryPercent: number,
  diskRate: number,
  networkRate: number,
  processes: ProcessSample[],
): string {
  const pressure = [
    {
      label: "CPU",
      score: cpuPercent / 100,
      value: (process: ProcessSample) => process.cpu_percent,
    },
    {
      label: "memory",
      score: memoryPercent / 100,
      value: (process: ProcessSample) => process.memory_bytes,
    },
    {
      label: "disk",
      score: diskRate / (50 * 1024 * 1024),
      value: (process: ProcessSample) =>
        process.disk_read_bps + process.disk_write_bps + (process.other_io_bps ?? 0),
    },
    {
      label: "network",
      score: networkRate / (25 * 1024 * 1024),
      value: (process: ProcessSample) =>
        (process.network_received_bps ?? 0) + (process.network_transmitted_bps ?? 0),
    },
  ].sort((left, right) => right.score - left.score)[0];

  if (pressure.score < 0.65) {
    return "System is steady.";
  }

  const contributor = processes.reduce<ProcessSample | null>(
    (best, process) => (!best || pressure.value(process) > pressure.value(best) ? process : best),
    null,
  );
  const prefix =
    pressure.score >= 0.85
      ? `High ${pressure.label} pressure`
      : `${pressure.label[0].toLocaleUpperCase()}${pressure.label.slice(1)} is elevated`;

  return contributor && pressure.value(contributor) > 0
    ? `${prefix} - ${contributor.name} is the top activity.`
    : `${prefix}.`;
}
