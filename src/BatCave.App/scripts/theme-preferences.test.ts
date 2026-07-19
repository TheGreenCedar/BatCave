import assert from "node:assert/strict";
import test from "node:test";
import {
  chartPalettes,
  parseThemePreference,
  resolveThemePreference,
  serializeResolvedTheme,
  serializeThemePreference,
  type ResolvedThemeMode,
  type ThemeFamily,
  type ThemeModePreference,
} from "../src/lib/themes.ts";
import { UiPreferencePersistenceSequence } from "../src/lib/uiPreferencePersistence.ts";
import type { RuntimeSnapshot } from "../src/lib/types.ts";

const families: ThemeFamily[] = ["cave", "aurora", "ember", "canopy"];
const preferenceModes: ThemeModePreference[] = ["system", "light", "dark"];
const resolvedModes: ResolvedThemeMode[] = ["light", "dark"];

test("theme preferences round-trip every valid family and mode pair", () => {
  for (const family of families) {
    for (const mode of preferenceModes) {
      const preference = { family, mode };
      const encoded = serializeThemePreference(preference);
      assert.equal(encoded, `${family}:${mode}`);
      assert.deepEqual(parseThemePreference(encoded), preference);
    }
  }
});

test("legacy theme values hydrate to their normalized paired preferences", () => {
  assert.deepEqual(parseThemePreference("system"), { family: "cave", mode: "system" });
  assert.deepEqual(parseThemePreference("auto"), { family: "cave", mode: "system" });
  assert.deepEqual(parseThemePreference("cave"), { family: "cave", mode: "dark" });
  assert.deepEqual(parseThemePreference("aurora"), { family: "aurora", mode: "dark" });
  assert.deepEqual(parseThemePreference("ember"), { family: "ember", mode: "dark" });
  assert.deepEqual(parseThemePreference("daylight"), { family: "cave", mode: "light" });
});

test("theme parsing rejects invalid and ambiguous combinations", () => {
  for (const value of [
    null,
    "",
    "canopy",
    "daylight:light",
    "cave:auto",
    "cave:system:dark",
    "Cave:dark",
    ":dark",
    "cave:",
  ]) {
    assert.equal(parseThemePreference(value), null, String(value));
  }
});

test("system mode resolves without changing the selected family", () => {
  assert.deepEqual(resolveThemePreference({ family: "canopy", mode: "system" }, "light"), {
    family: "canopy",
    mode: "light",
  });
  assert.deepEqual(resolveThemePreference({ family: "ember", mode: "dark" }, "light"), {
    family: "ember",
    mode: "dark",
  });
  assert.equal(
    serializeResolvedTheme(resolveThemePreference({ family: "aurora", mode: "system" }, "dark")),
    "aurora:dark",
  );
});

test("every resolved theme has a chart palette", () => {
  for (const family of families) {
    for (const mode of resolvedModes) {
      const name = serializeResolvedTheme({ family, mode });
      assert.ok(chartPalettes[name], name);
    }
  }
});

test("local fallback clears only for the latest exact durable runtime echo", () => {
  const sequence = new UiPreferencePersistenceSequence();
  const first = sequence.begin({ theme: "cave:dark", history_point_limit: 72 });
  const latest = sequence.begin({ theme: "canopy:system", history_point_limit: 180 });

  assert.equal(sequence.isLatestDurable(first, durableSnapshot("cave:dark", 72)), false);
  assert.equal(sequence.isLatestDurable(latest, durableSnapshot("canopy:system", 72)), false);
  assert.equal(
    sequence.isLatestDurable(latest, durableSnapshot("canopy:dark", 180)),
    false,
    "the resolved native value is not the persisted preference",
  );
  assert.equal(
    sequence.isLatestDurable(latest, durableSnapshot("canopy:system", 180, "degraded")),
    false,
  );
  assert.equal(sequence.isLatestDurable(latest, durableSnapshot("canopy:system", 180)), true);
});

function durableSnapshot(
  theme: string,
  historyPointLimit: number,
  state: "healthy" | "degraded" = "healthy",
): RuntimeSnapshot {
  return {
    settings: {
      ui_preferences: {
        theme,
        history_point_limit: historyPointLimit,
      },
    },
    persistence: {
      components: [
        {
          owner: "current_user",
          kind: "settings",
          state,
          durability: "durable",
          active_failure: null,
        },
      ],
    },
  } as RuntimeSnapshot;
}
