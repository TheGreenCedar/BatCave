import type { RuntimeAdminModeStatus, RuntimeEnvironment, RuntimeInstallKind } from "./types";

const installKindLabels: Record<RuntimeInstallKind, string> = {
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
      return "Standard access; privileged access unavailable";
    default:
      return "Standard access";
  }
}
