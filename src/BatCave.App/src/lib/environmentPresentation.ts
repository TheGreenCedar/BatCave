import type {
  RuntimeAdminModeStatus,
  RuntimeEnvironment,
  RuntimeInstallKind,
  RuntimePrivilegedSource,
} from "./types";

const installKindLabels: Record<RuntimeInstallKind, string> = {
  unknown: "Package state unavailable",
  nsis: "NSIS install",
  appimage: "AppImage",
  deb: "Debian package",
  dmg: "Mounted DMG",
  app_bundle: "App bundle",
  portable: "Portable",
  development: "Development",
};

const platformNames: Record<RuntimeEnvironment["platform"], string> = {
  windows: "Windows",
  linux: "Linux",
  macos: "macOS",
  fixture: "Fixture",
};

export function installKindLabel(installKind: RuntimeInstallKind): string {
  return installKindLabels[installKind] ?? "Unknown package";
}

export function processElevationLabel(environment: RuntimeEnvironment): string {
  switch (environment.process_elevation) {
    case "elevated":
      return "Administrator token";
    case "standard":
      return "Standard token";
    case "unknown":
      return "Windows token state unavailable";
    default:
      return `Not applicable on ${platformNames[environment.platform] ?? "this platform"}`;
  }
}

export function privilegedCollectionLabel(
  adminMode: RuntimeAdminModeStatus,
  blockedProcessCount = 0,
): string {
  switch (adminMode.state) {
    case "requesting":
      return "Waiting for Windows";
    case "active":
      if (adminMode.source === "current_process") {
        return blockedProcessCount > 0
          ? `Current process, ${blockedProcessCount} blocked`
          : "Current process";
      }
      return blockedProcessCount > 0
        ? `Elevated helper active, ${blockedProcessCount} blocked`
        : "Elevated helper active";
    case "recovering":
      return "Helper recovering; standard monitoring current";
    case "failed":
      return adminMode.source === "elevated_helper"
        ? "Helper unavailable; retry available"
        : "Inactive";
    case "unavailable":
      return "Not available";
    default:
      return "Off";
  }
}

export function privilegedCollectionNote(adminMode: RuntimeAdminModeStatus): string {
  switch (adminMode.state) {
    case "active":
      return adminMode.source === "current_process"
        ? "Protected fields come from the manually elevated current process."
        : "Protected fields come from the local elevated helper; the parent app keeps its original token.";
    case "recovering":
      return "The helper is recovering; current values use standard monitoring.";
    case "failed":
      return adminMode.source === "elevated_helper"
        ? "The elevation request did not complete. Standard monitoring remains available."
        : "Privileged collection is inactive because the current process token could not be read.";
    case "requesting":
      return "Standard monitoring remains available while Windows handles the elevation request.";
    case "unavailable":
      return "Privileged collection is unavailable on this platform.";
    default:
      return "Protected fields remain unavailable until the local helper is enabled.";
  }
}

export function privilegedSourceLabel(source: RuntimePrivilegedSource): string {
  switch (source) {
    case "current_process":
      return "Current process";
    case "elevated_helper":
      return "Local elevated helper";
    default:
      return "None";
  }
}

export interface PrivilegedCollectionAction {
  label: string;
  enabled: boolean;
}

export function privilegedCollectionAction(
  available: boolean,
  adminMode: RuntimeAdminModeStatus,
): PrivilegedCollectionAction | null {
  if (
    !available ||
    adminMode.source === "current_process" ||
    adminMode.state === "requesting" ||
    adminMode.state === "unavailable"
  ) {
    return null;
  }
  if (adminMode.state === "active" || adminMode.state === "recovering") {
    return { label: "Disable helper", enabled: false };
  }
  if (adminMode.state === "failed" && adminMode.source === "elevated_helper") {
    return { label: "Retry helper", enabled: true };
  }
  return { label: "Enable helper", enabled: true };
}
