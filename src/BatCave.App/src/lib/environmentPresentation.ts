import type { RuntimeAdminModeStatus, RuntimeEnvironment, RuntimeInstallKind } from "./types";

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

export function adminAccessLabel(
  environment: RuntimeEnvironment,
  adminMode: RuntimeAdminModeStatus,
  blockedProcessCount = 0,
): string {
  if (!environment.admin_mode_available) {
    return `Not available on ${platformNames[environment.platform] ?? "this platform"}`;
  }

  switch (adminMode.state) {
    case "requesting":
      return environment.platform === "windows" ? "Waiting for Windows" : "Waiting for approval";
    case "active":
      return blockedProcessCount > 0
        ? `Administrator token, ${blockedProcessCount} blocked`
        : "Administrator token";
    case "recovering":
      return "Recovering with standard access";
    case "failed":
      if (adminMode.detail?.startsWith("process_token_")) {
        return "Windows token state unavailable";
      }
      return "Standard access; privileged access unavailable";
    default:
      return "Standard access";
  }
}

export function adminAccessNote(
  environment: RuntimeEnvironment,
  adminMode: RuntimeAdminModeStatus,
): string {
  if (!environment.admin_mode_available) {
    return "Privileged collection is unavailable on this platform.";
  }

  switch (adminMode.state) {
    case "active":
      return "This process is running with an administrator token.";
    case "recovering":
      return "Protected collection is recovering; current values use standard access.";
    case "failed":
      if (adminMode.detail?.startsWith("process_token_")) {
        return "BatCave could not read the Windows process token. Privileged collectors remain inactive; the token state is unknown.";
      }
      return "The elevation request did not complete. Standard monitoring remains available.";
    case "requesting":
      return "Standard monitoring remains available while Windows handles the elevation request.";
    default:
      return "Protected fields remain unavailable while this process has standard access.";
  }
}
