export type RuntimeSurfaceMode = "native" | "fixture" | "unavailable";

export function runtimeSurfaceMode(
  hasTauriRuntime: boolean,
  development: boolean,
): RuntimeSurfaceMode {
  if (hasTauriRuntime) return "native";
  return development ? "fixture" : "unavailable";
}
