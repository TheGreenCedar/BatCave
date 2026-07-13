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
  adminRequestLabel: string;
}

const presentations: Record<RuntimePlatform, PlatformPresentation> = {
  windows: {
    platformName: "Windows",
    memoryLabel: "Resident memory",
    privateMemoryLabel: "Private memory",
    handlesLabel: "Handles",
    privilegedAccessLabel: "Administrator access",
    privilegedAccessDescription:
      "Installed Windows releases use administrator access to read protected process fields.",
    adminRequestLabel: "Waiting for Windows",
  },
  linux: {
    platformName: "Linux",
    memoryLabel: "Resident memory",
    privateMemoryLabel: "Private memory",
    handlesLabel: "File descriptors",
    privilegedAccessLabel: "Privileged access",
    privilegedAccessDescription:
      "Elevated collection is unavailable; BatCave keeps standard-access metrics current.",
    adminRequestLabel: "Waiting for approval",
  },
  macos: {
    platformName: "macOS",
    memoryLabel: "Resident memory",
    privateMemoryLabel: "Physical footprint",
    handlesLabel: "File descriptors",
    privilegedAccessLabel: "Privileged access",
    privilegedAccessDescription:
      "BatCave monitors macOS with standard local access and marks protected fields unavailable.",
    adminRequestLabel: "Waiting for approval",
  },
  fixture: {
    platformName: "Fixture",
    memoryLabel: "Resident memory",
    privateMemoryLabel: "Private memory",
    handlesLabel: "Handles",
    privilegedAccessLabel: "Privileged access",
    privilegedAccessDescription: "Fixture telemetry does not request elevated access.",
    adminRequestLabel: "Waiting for approval",
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
