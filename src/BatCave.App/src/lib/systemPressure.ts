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
    cpu_quality: cpu.quality,
    cpu_name_ambiguous: cpu.ambiguous,
    memory: memory.name,
    memory_quality: memory.quality,
    memory_name_ambiguous: memory.ambiguous,
    io: io.name,
    io_quality: io.quality,
    io_name_ambiguous: io.ambiguous,
    network: network.name,
    network_quality: network.quality,
    network_name_ambiguous: network.ambiguous,
  };
}

function topProcessContributor(
  processes: ProcessSample[],
  metric: (process: ProcessSample) => number,
  quality: (process: ProcessSample) => MetricQualityInfo | undefined,
): { name: string | null; quality?: MetricQualityInfo; ambiguous: boolean } {
  const process = processes
    .filter((candidate) => contributorQualityIsPublishable(quality(candidate)))
    .reduce<ProcessSample | null>(
      (best, candidate) => (!best || metric(candidate) > metric(best) ? candidate : best),
      null,
    );
  if (process && metric(process) > 0) {
    return {
      name: process.name,
      quality: quality(process),
      ambiguous: processes.filter((candidate) => candidate.name === process.name).length > 1,
    };
  }

  const publishable = processes.filter((candidate) =>
    contributorQualityIsPublishable(quality(candidate)),
  );
  const summaryCandidates = publishable.length > 0 ? publishable : processes;
  const hasUnknownPublishableQuality = publishable.some(
    (candidate) => quality(candidate) === undefined,
  );
  const qualityCandidates = summaryCandidates
    .map(quality)
    .filter((candidate): candidate is MetricQualityInfo => candidate !== undefined);
  const summaryQuality = hasUnknownPublishableQuality
    ? undefined
    : qualityCandidates.reduce<MetricQualityInfo | undefined>((selected, candidate) => {
        if (!selected) return candidate;
        const candidateRank = contributorQualityRank(candidate);
        const selectedRank = contributorQualityRank(selected);
        return candidateRank > selectedRank ? candidate : selected;
      }, undefined);
  return { name: null, quality: summaryQuality ?? undefined, ambiguous: false };
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
