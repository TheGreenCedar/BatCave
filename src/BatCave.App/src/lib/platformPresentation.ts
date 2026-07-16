import type {
  MetricQualityInfo,
  ProcessSample,
  RuntimeEnvironment,
  RuntimePlatform,
} from "./types";
import { processBytesLabel, processMetricIsPublishable, processPrivateMemoryValue } from "./format";

export interface PlatformPresentation {
  platformName: string;
  memoryLabel: string;
  privateMemoryLabel: string;
  handlesLabel: string;
  privilegedAccessLabel: string;
  privilegedAccessDescription: string;
}

const presentations: Record<RuntimePlatform, PlatformPresentation> = {
  windows: {
    platformName: "Windows",
    memoryLabel: "Resident memory",
    privateMemoryLabel: "Private memory",
    handlesLabel: "Handles",
    privilegedAccessLabel: "Administrator access",
    privilegedAccessDescription:
      "Protected fields can come from the installed collector service while the app keeps its current token.",
  },
  linux: {
    platformName: "Linux",
    memoryLabel: "Resident memory",
    privateMemoryLabel: "Private memory",
    handlesLabel: "File descriptors",
    privilegedAccessLabel: "Privileged access",
    privilegedAccessDescription:
      "Elevated collection is unavailable; BatCave keeps standard-access metrics current.",
  },
  macos: {
    platformName: "macOS",
    memoryLabel: "Resident memory",
    privateMemoryLabel: "Physical footprint",
    handlesLabel: "File descriptors",
    privilegedAccessLabel: "Privileged access",
    privilegedAccessDescription:
      "BatCave monitors macOS with standard local access and marks protected fields unavailable.",
  },
  fixture: {
    platformName: "Fixture",
    memoryLabel: "Resident memory",
    privateMemoryLabel: "Private memory",
    handlesLabel: "Handles",
    privilegedAccessLabel: "Privileged access",
    privilegedAccessDescription: "Fixture telemetry does not request elevated access.",
  },
};

export function platformPresentation(
  environment: Pick<RuntimeEnvironment, "platform">,
): PlatformPresentation {
  return presentations[environment.platform] ?? presentations.fixture;
}

export function metricIsUnavailable(quality: MetricQualityInfo | undefined): boolean {
  return quality?.quality === "unavailable";
}

export function processMetricAvailable(
  process: ProcessSample,
  metric: keyof NonNullable<ProcessSample["quality"]>,
): boolean {
  return processMetricIsPublishable(process.quality?.[metric]);
}

export function qualityAwareZero(
  value: number,
  quality: MetricQualityInfo | undefined,
): number | null {
  return processMetricIsPublishable(quality) ? value : null;
}

export function residentMemoryValue(process: ProcessSample, platform: RuntimePlatform): string {
  void platform;
  return processBytesLabel(process, process.memory_bytes);
}

export function privateMemoryValue(process: ProcessSample, platform: RuntimePlatform): string {
  return processPrivateMemoryValue(process, platform);
}
