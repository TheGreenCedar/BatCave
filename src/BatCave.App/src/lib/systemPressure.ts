import type { ProcessContributorSummary, ProcessSample } from "./types";

export function systemPressureHeadline(
  cpuPercent: number,
  memoryPercent: number,
  diskRate: number,
  networkRate: number,
  contributors: ProcessContributorSummary,
): string {
  const pressure = [
    {
      label: "CPU",
      score: cpuPercent / 100,
      contributor: contributors.cpu,
    },
    {
      label: "memory",
      score: memoryPercent / 100,
      contributor: contributors.memory,
    },
    {
      label: "disk",
      score: diskRate / (50 * 1024 * 1024),
      contributor: contributors.disk,
    },
    {
      label: "network",
      score: networkRate / (25 * 1024 * 1024),
      contributor: contributors.network,
    },
  ].sort((left, right) => right.score - left.score)[0];

  if (pressure.score < 0.65) {
    return "System is steady.";
  }

  const prefix =
    pressure.score >= 0.85
      ? `High ${pressure.label} pressure`
      : `${pressure.label[0].toLocaleUpperCase()}${pressure.label.slice(1)} is elevated`;

  return pressure.contributor
    ? `${prefix} - ${pressure.contributor} is the top activity.`
    : `${prefix}.`;
}

export function summarizeProcessContributors(
  processes: ProcessSample[],
): ProcessContributorSummary {
  return {
    cpu: topProcessName(processes, (process) => process.cpu_percent),
    memory: topProcessName(processes, (process) => process.memory_bytes),
    disk: topProcessName(
      processes,
      (process) => process.disk_read_bps + process.disk_write_bps + (process.other_io_bps ?? 0),
    ),
    network: topProcessName(
      processes,
      (process) => (process.network_received_bps ?? 0) + (process.network_transmitted_bps ?? 0),
    ),
  };
}

function topProcessName(
  processes: ProcessSample[],
  metric: (process: ProcessSample) => number,
): string | null {
  const process = processes.reduce<ProcessSample | null>(
    (best, candidate) => (!best || metric(candidate) > metric(best) ? candidate : best),
    null,
  );
  return process && metric(process) > 0 ? process.name : null;
}
