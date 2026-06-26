export type ThemeName = "cave" | "aurora" | "ember" | "daylight";
export type ThemePreference = "system" | ThemeName;

export interface ThemeOption {
  name: ThemePreference;
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

export const themeOptions: ThemeOption[] = [
  { name: "system", label: "System", ariaLabel: "Use system theme" },
  { name: "cave", label: "Cave", ariaLabel: "Use Cave low-light monitoring theme" },
  { name: "aurora", label: "Aurora", ariaLabel: "Use Aurora cool monitoring theme" },
  { name: "ember", label: "Ember", ariaLabel: "Use Ember warm monitoring theme" },
  { name: "daylight", label: "Daylight", ariaLabel: "Use Daylight high-visibility theme" },
];

export const chartPalettes: Record<ThemeName, ChartPalette> = {
  cave: {
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
  aurora: {
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
  ember: {
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
  daylight: {
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
};

export function isThemeName(value: string | null): value is ThemeName {
  return value === "cave" || value === "aurora" || value === "ember" || value === "daylight";
}

export function parseThemePreference(value: string | null): ThemePreference | null {
  if (value === "system" || value === "auto") {
    return "system";
  }

  return isThemeName(value) ? value : null;
}

export function resolveThemeName(preference: ThemePreference, systemTheme: ThemeName): ThemeName {
  return preference === "system" ? systemTheme : preference;
}
