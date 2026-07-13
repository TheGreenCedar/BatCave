import type { MetricQualityInfo, ProcessContributorSummary, ProcessSample } from "./types";

export function summarizeProcessContributors(
  processes: ProcessSample[],
): ProcessContributorSummary {
  const cpu = topProcessContributor(
    processes,
    (process) => process.cpu_percent,
    (process) => process.quality?.cpu,
  );
  const memory = topProcessContributor(
    processes,
    (process) => process.memory_bytes,
    (process) => process.quality?.memory,
  );
  const io = topProcessContributor(
    processes,
    (process) => process.io_read_bps + process.io_write_bps,
    (process) => process.quality?.io,
  );
  const network = topProcessContributor(
    processes,
    (process) => (process.network_received_bps ?? 0) + (process.network_transmitted_bps ?? 0),
    (process) => process.quality?.network,
  );

  return {
    cpu: cpu.name,
    cpu_process_id: cpu.processId,
    cpu_coverage: cpu.coverage,
    cpu_quality: cpu.quality,
    cpu_name_ambiguous: cpu.ambiguous,
    memory: memory.name,
    memory_process_id: memory.processId,
    memory_coverage: memory.coverage,
    memory_quality: memory.quality,
    memory_name_ambiguous: memory.ambiguous,
    io: io.name,
    io_process_id: io.processId,
    io_coverage: io.coverage,
    io_quality: io.quality,
    io_name_ambiguous: io.ambiguous,
    network: network.name,
    network_process_id: network.processId,
    network_coverage: network.coverage,
    network_quality: network.quality,
    network_name_ambiguous: network.ambiguous,
  };
}

function topProcessContributor(
  processes: ProcessSample[],
  metric: (process: ProcessSample) => number,
  quality: (process: ProcessSample) => MetricQualityInfo | undefined,
): {
  name: string | null;
  processId: string | null;
  coverage: { available: number; total: number };
  quality?: MetricQualityInfo;
  ambiguous: boolean;
} {
  const hasUnknownQuality = processes.some((candidate) => quality(candidate) === undefined);
  const qualityCandidates = processes
    .map(quality)
    .filter((candidate): candidate is MetricQualityInfo => candidate !== undefined);
  const coverageQuality = hasUnknownQuality
    ? undefined
    : qualityCandidates.reduce<MetricQualityInfo | undefined>((selected, candidate) => {
        if (!selected) return candidate;
        const candidateRank = contributorQualityRank(candidate);
        const selectedRank = contributorQualityRank(selected);
        return candidateRank > selectedRank ? candidate : selected;
      }, undefined);
  const coverageIsPublishable = processes.every((candidate) => {
    const candidateQuality = quality(candidate);
    return candidateQuality !== undefined && contributorQualityIsPublishable(candidateQuality);
  });
  const coverage = {
    available: processes.filter((candidate) => contributorQualityIsPublishable(quality(candidate)))
      .length,
    total: processes.length,
  };
  const process = processes
    .filter((candidate) => contributorQualityIsPublishable(quality(candidate)))
    .reduce<ProcessSample | null>(
      (best, candidate) => (!best || metric(candidate) > metric(best) ? candidate : best),
      null,
    );
  if (process && metric(process) > 0) {
    if (!coverageIsPublishable) {
      return { name: null, processId: null, coverage, quality: coverageQuality, ambiguous: false };
    }
    return {
      name: process.name,
      processId: process.start_time_ms ? `process:${process.pid}:${process.start_time_ms}` : null,
      coverage,
      quality: coverageQuality,
      ambiguous: processes.filter((candidate) => candidate.name === process.name).length > 1,
    };
  }

  return { name: null, processId: null, coverage, quality: coverageQuality, ambiguous: false };
}

function contributorQualityIsPublishable(quality: MetricQualityInfo | undefined): boolean {
  return quality?.quality !== "unavailable" && quality?.quality !== "held";
}

function contributorQualityRank(quality: MetricQualityInfo): number {
  return {
    native: 1,
    estimated: 2,
    partial: 3,
    held: 4,
    unavailable: 5,
  }[quality.quality];
}
