import type { DetailMode } from "./components/metrics/types";
import type { RuntimeSnapshot } from "./types";

export type PressureTone = "high" | "moderate" | "low";

export interface PressureBrief {
  mode: DetailMode;
  label: string;
  value: number;
  tone: PressureTone;
  headlinePrefix: string;
  headline: string;
  leadingWorkload: string | null;
  leadingValue: number | null;
  confidence: "High" | "Limited";
}

interface PressureCandidate {
  mode: DetailMode;
  label: string;
  score: number;
  value: number;
  contributor: string | null;
}

export function buildPressureBrief(
  snapshot: RuntimeSnapshot,
  memoryPercent: number,
  diskRate: number,
  networkRate: number,
): PressureBrief {
  const candidates: PressureCandidate[] = [
    {
      mode: "cpu",
      label: "CPU",
      score:
        snapshot.system.quality?.cpu?.quality === "unavailable"
          ? -1
          : snapshot.system.cpu_percent / 100,
      value: snapshot.system.cpu_percent,
      contributor:
        snapshot.system.quality?.cpu?.quality === "unavailable"
          ? null
          : snapshot.process_contributors.cpu,
    },
    {
      mode: "memory",
      label: "memory",
      score: snapshot.system.quality?.memory?.quality === "unavailable" ? -1 : memoryPercent / 100,
      value: memoryPercent,
      contributor:
        snapshot.system.quality?.memory?.quality === "unavailable"
          ? null
          : snapshot.process_contributors.memory,
    },
    {
      mode: "disk",
      label: "disk",
      score:
        snapshot.system.quality?.disk?.quality === "unavailable"
          ? -1
          : diskRate / (50 * 1024 * 1024),
      value: diskRate,
      contributor:
        snapshot.system.quality?.disk?.quality === "unavailable"
          ? null
          : snapshot.process_contributors.disk,
    },
    {
      mode: "network",
      label: "network",
      score:
        snapshot.system.quality?.network?.quality === "unavailable"
          ? -1
          : networkRate / (25 * 1024 * 1024),
      value: networkRate,
      contributor:
        snapshot.system.quality?.network?.quality === "unavailable"
          ? null
          : snapshot.process_contributors.network,
    },
  ];
  const dominant = candidates.sort((left, right) => right.score - left.score)[0];
  const tone: PressureTone =
    dominant.score >= 0.85 ? "high" : dominant.score >= 0.65 ? "moderate" : "low";
  const leadingWorkload = dominant.contributor ? displayProcessName(dominant.contributor) : null;
  const prefix =
    tone === "high"
      ? `High ${dominant.label} pressure`
      : tone === "moderate"
        ? `${titleCase(dominant.label)} is elevated`
        : "System pressure is low";
  const headline = leadingWorkload
    ? `${prefix} — ${leadingWorkload} is the leading workload.`
    : `${prefix}.`;
  const leadingProcess = dominant.contributor
    ? snapshot.processes.find((process) => process.name === dominant.contributor)
    : undefined;

  return {
    mode: dominant.mode,
    label: dominant.label,
    value: dominant.value,
    tone,
    headlinePrefix: prefix,
    headline,
    leadingWorkload,
    leadingValue: leadingProcess
      ? dominant.mode === "cpu"
        ? leadingProcess.cpu_percent
        : dominant.mode === "memory"
          ? (leadingProcess.memory_bytes / Math.max(snapshot.system.memory_total_bytes, 1)) * 100
          : null
      : null,
    confidence: snapshot.health.degraded || snapshot.warnings.length > 0 ? "Limited" : "High",
  };
}

export function displayProcessName(name: string): string {
  const normalized = name.replace(/\.exe$/i, "");
  if (normalized.toLocaleLowerCase() === "code") return "Visual Studio Code";
  if (normalized.toLocaleLowerCase() === "msedge") return "Microsoft Edge";
  return normalized;
}

function titleCase(value: string): string {
  return `${value.charAt(0).toLocaleUpperCase()}${value.slice(1)}`;
}
