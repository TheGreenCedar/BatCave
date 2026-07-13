export const accessibilityFixtureStates = [
  "overview",
  "process",
  "group",
  "settings",
  "diagnostics",
  "stale",
  "degraded",
  "compact",
] as const;

export type AccessibilityFixtureState = (typeof accessibilityFixtureStates)[number];

export function resolveAccessibilityFixtureState(
  search: string,
  development: boolean,
  hasTauriRuntime: boolean,
): AccessibilityFixtureState | null {
  if (!development || hasTauriRuntime) return null;
  const requested = new URLSearchParams(search).get("a11y");
  return accessibilityFixtureStates.find((state) => state === requested) ?? null;
}
