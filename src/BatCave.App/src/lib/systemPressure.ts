import type { ProcessContributorSummary, ProcessSample } from "./types";

export function summarizeProcessContributors(
  processes: ProcessSample[],
): ProcessContributorSummary {
  return {
    cpu: topProcessName(processes, (process) => process.cpu_percent),
    memory: topProcessName(processes, (process) => process.memory_bytes),
    io: topProcessName(
      processes,
      (process) => process.io_read_bps + process.io_write_bps + (process.other_io_bps ?? 0),
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
