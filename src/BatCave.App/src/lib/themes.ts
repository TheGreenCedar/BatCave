export type ThemeFamily = "cave" | "aurora" | "ember" | "canopy";
export type ThemeModePreference = "system" | "light" | "dark";
export type ResolvedThemeMode = Exclude<ThemeModePreference, "system">;

export interface ThemePreference {
  family: ThemeFamily;
  mode: ThemeModePreference;
}

export interface ResolvedTheme {
  family: ThemeFamily;
  mode: ResolvedThemeMode;
}

export type PersistedThemePreference = `${ThemeFamily}:${ThemeModePreference}`;
export type ResolvedThemeName = `${ThemeFamily}:${ResolvedThemeMode}`;

export interface ThemeFamilyOption {
  family: ThemeFamily;
  label: string;
  ariaLabel: string;
}

export interface ThemeModeOption {
  mode: ThemeModePreference;
  label: string;
  ariaLabel: string;
}

export interface ChartPalette {
  cpuStroke: string;
  cpuFill: string;
  memoryStroke: string;
  memoryFill: string;
  diskReadStroke: string;
  diskReadFill: string;
  diskWriteStroke: string;
  diskWriteFill: string;
  networkDownStroke: string;
  networkDownFill: string;
  networkUpStroke: string;
  networkUpFill: string;
  swapStroke: string;
  swapFill: string;
}

export const themeStorageKey = "batcave.monitor.theme";

export const defaultThemePreference: ThemePreference = {
  family: "cave",
  mode: "system",
};

export const themeFamilyOptions: ThemeFamilyOption[] = [
  { family: "cave", label: "Cave", ariaLabel: "Use the Cave theme family" },
  { family: "aurora", label: "Aurora", ariaLabel: "Use the Aurora theme family" },
  { family: "ember", label: "Ember", ariaLabel: "Use the Ember theme family" },
  { family: "canopy", label: "Canopy", ariaLabel: "Use the Canopy theme family" },
];

export const themeModeOptions: ThemeModeOption[] = [
  { mode: "system", label: "System", ariaLabel: "Follow the system appearance" },
  { mode: "light", label: "Light", ariaLabel: "Use the light appearance" },
  { mode: "dark", label: "Dark", ariaLabel: "Use the dark appearance" },
];

export const chartPalettes: Record<ResolvedThemeName, ChartPalette> = {
  "cave:dark": {
    cpuStroke: "#4a9cff",
    cpuFill: "rgba(74, 156, 255, 0.2)",
    memoryStroke: "#b26cff",
    memoryFill: "rgba(178, 108, 255, 0.2)",
    diskReadStroke: "#ffd166",
    diskReadFill: "rgba(255, 209, 102, 0.2)",
    diskWriteStroke: "#fca5a5",
    diskWriteFill: "rgba(252, 165, 165, 0.2)",
    networkDownStroke: "#a78bfa",
    networkDownFill: "rgba(167, 139, 250, 0.2)",
    networkUpStroke: "#fb7185",
    networkUpFill: "rgba(251, 113, 133, 0.2)",
    swapStroke: "#a78bfa",
    swapFill: "rgba(167, 139, 250, 0.16)",
  },
  "cave:light": {
    cpuStroke: "#047857",
    cpuFill: "rgba(4, 120, 87, 0.18)",
    memoryStroke: "#0369a1",
    memoryFill: "rgba(3, 105, 161, 0.16)",
    diskReadStroke: "#b45309",
    diskReadFill: "rgba(180, 83, 9, 0.15)",
    diskWriteStroke: "#be123c",
    diskWriteFill: "rgba(190, 18, 60, 0.14)",
    networkDownStroke: "#6d28d9",
    networkDownFill: "rgba(109, 40, 217, 0.14)",
    networkUpStroke: "#0f766e",
    networkUpFill: "rgba(15, 118, 110, 0.14)",
    swapStroke: "#7c3aed",
    swapFill: "rgba(124, 58, 237, 0.12)",
  },
  "aurora:dark": {
    cpuStroke: "#5eead4",
    cpuFill: "rgba(94, 234, 212, 0.22)",
    memoryStroke: "#93c5fd",
    memoryFill: "rgba(147, 197, 253, 0.24)",
    diskReadStroke: "#c4b5fd",
    diskReadFill: "rgba(196, 181, 253, 0.22)",
    diskWriteStroke: "#f0abfc",
    diskWriteFill: "rgba(240, 171, 252, 0.18)",
    networkDownStroke: "#67e8f9",
    networkDownFill: "rgba(103, 232, 249, 0.18)",
    networkUpStroke: "#bef264",
    networkUpFill: "rgba(190, 242, 100, 0.16)",
    swapStroke: "#c4b5fd",
    swapFill: "rgba(196, 181, 253, 0.16)",
  },
  "aurora:light": {
    cpuStroke: "#0f766e",
    cpuFill: "rgba(15, 118, 110, 0.16)",
    memoryStroke: "#2563eb",
    memoryFill: "rgba(37, 99, 235, 0.14)",
    diskReadStroke: "#7c3aed",
    diskReadFill: "rgba(124, 58, 237, 0.14)",
    diskWriteStroke: "#a21caf",
    diskWriteFill: "rgba(162, 28, 175, 0.12)",
    networkDownStroke: "#0891b2",
    networkDownFill: "rgba(8, 145, 178, 0.14)",
    networkUpStroke: "#4d7c0f",
    networkUpFill: "rgba(77, 124, 15, 0.12)",
    swapStroke: "#6d28d9",
    swapFill: "rgba(109, 40, 217, 0.12)",
  },
  "ember:dark": {
    cpuStroke: "#fbbf24",
    cpuFill: "rgba(251, 191, 36, 0.22)",
    memoryStroke: "#fb7185",
    memoryFill: "rgba(251, 113, 133, 0.2)",
    diskReadStroke: "#fdba74",
    diskReadFill: "rgba(253, 186, 116, 0.22)",
    diskWriteStroke: "#f97316",
    diskWriteFill: "rgba(249, 115, 22, 0.18)",
    networkDownStroke: "#fca5a5",
    networkDownFill: "rgba(252, 165, 165, 0.18)",
    networkUpStroke: "#fde68a",
    networkUpFill: "rgba(253, 230, 138, 0.16)",
    swapStroke: "#fb7185",
    swapFill: "rgba(251, 113, 133, 0.16)",
  },
  "ember:light": {
    cpuStroke: "#c2410c",
    cpuFill: "rgba(194, 65, 12, 0.16)",
    memoryStroke: "#be123c",
    memoryFill: "rgba(190, 18, 60, 0.14)",
    diskReadStroke: "#b45309",
    diskReadFill: "rgba(180, 83, 9, 0.14)",
    diskWriteStroke: "#ea580c",
    diskWriteFill: "rgba(234, 88, 12, 0.12)",
    networkDownStroke: "#b91c1c",
    networkDownFill: "rgba(185, 28, 28, 0.12)",
    networkUpStroke: "#a16207",
    networkUpFill: "rgba(161, 98, 7, 0.12)",
    swapStroke: "#9f1239",
    swapFill: "rgba(159, 18, 57, 0.12)",
  },
  "canopy:dark": {
    cpuStroke: "#86efac",
    cpuFill: "rgba(134, 239, 172, 0.2)",
    memoryStroke: "#67e8f9",
    memoryFill: "rgba(103, 232, 249, 0.18)",
    diskReadStroke: "#facc15",
    diskReadFill: "rgba(250, 204, 21, 0.18)",
    diskWriteStroke: "#fb923c",
    diskWriteFill: "rgba(251, 146, 60, 0.16)",
    networkDownStroke: "#5eead4",
    networkDownFill: "rgba(94, 234, 212, 0.16)",
    networkUpStroke: "#bef264",
    networkUpFill: "rgba(190, 242, 100, 0.16)",
    swapStroke: "#a7f3d0",
    swapFill: "rgba(167, 243, 208, 0.14)",
  },
  "canopy:light": {
    cpuStroke: "#15803d",
    cpuFill: "rgba(21, 128, 61, 0.16)",
    memoryStroke: "#0e7490",
    memoryFill: "rgba(14, 116, 144, 0.14)",
    diskReadStroke: "#a16207",
    diskReadFill: "rgba(161, 98, 7, 0.14)",
    diskWriteStroke: "#c2410c",
    diskWriteFill: "rgba(194, 65, 12, 0.12)",
    networkDownStroke: "#0f766e",
    networkDownFill: "rgba(15, 118, 110, 0.14)",
    networkUpStroke: "#4d7c0f",
    networkUpFill: "rgba(77, 124, 15, 0.12)",
    swapStroke: "#047857",
    swapFill: "rgba(4, 120, 87, 0.12)",
  },
};

export function isThemeFamily(value: string): value is ThemeFamily {
  return value === "cave" || value === "aurora" || value === "ember" || value === "canopy";
}

export function isThemeModePreference(value: string): value is ThemeModePreference {
  return value === "system" || value === "light" || value === "dark";
}

export function parseThemePreference(value: string | null): ThemePreference | null {
  const legacy = parseLegacyThemePreference(value);
  if (legacy) return legacy;
  if (value === null) return null;

  const parts = value.split(":");
  if (parts.length !== 2) return null;
  const [family, mode] = parts;
  return isThemeFamily(family) && isThemeModePreference(mode) ? { family, mode } : null;
}

export function serializeThemePreference(preference: ThemePreference): PersistedThemePreference {
  return `${preference.family}:${preference.mode}`;
}

export function resolveThemePreference(
  preference: ThemePreference,
  systemMode: ResolvedThemeMode,
): ResolvedTheme {
  return {
    family: preference.family,
    mode: preference.mode === "system" ? systemMode : preference.mode,
  };
}

export function serializeResolvedTheme(theme: ResolvedTheme): ResolvedThemeName {
  return `${theme.family}:${theme.mode}`;
}

function parseLegacyThemePreference(value: string | null): ThemePreference | null {
  switch (value) {
    case "system":
    case "auto":
      return { family: "cave", mode: "system" };
    case "cave":
    case "aurora":
    case "ember":
      return { family: value, mode: "dark" };
    case "daylight":
      return { family: "cave", mode: "light" };
    default:
      return null;
  }
}
