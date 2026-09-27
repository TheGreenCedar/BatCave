import AxeBuilder from "@axe-core/playwright";
import { expect, test, type Page } from "@playwright/test";
import { readFileSync } from "node:fs";

type RateFixture = {
  event: {
    kind: string;
    payload: {
      descriptors: { semantic: string }[];
      system: { metrics: [number, number | null, number, number | null, number | null][] };
      quality_codes: string[];
      limitations: { message: string }[];
      sampled_at_ms: number | null;
    };
  };
};

type GroupedHeroFixture = RateFixture & {
  event: {
    payload: {
      limitations: { code: string; message: string }[];
      workloads: (
        | {
            kind: "group";
            detail: {
              stable_id: string;
              group_key: string;
              label: string;
              metrics: RateFixture["event"]["payload"]["system"]["metrics"];
              coverage: {
                descriptor_index: number;
                available_contributors: number;
                limitation_index: number | null;
              }[];
            };
          }
        | {
            kind: "process";
            detail: {
              presentation: { group_id: string | null; group_key: string; group_label: string };
              metrics: RateFixture["event"]["payload"]["system"]["metrics"];
            };
          }
      )[];
    };
  };
};

type FixtureState =
  | "overview"
  | "process"
  | "group"
  | "settings"
  | "diagnostics"
  | "stale"
  | "degraded"
  | "compact"
  | "exited";

const wcagTags = ["wcag2a", "wcag2aa", "wcag21a", "wcag21aa"];

async function openFixture(page: Page, state: FixtureState): Promise<void> {
  await page.goto(`/?a11y=${state}`);
  await expect(page.locator(`[data-accessibility-fixture="${state}"]`)).toBeVisible();
  await expect(page.getByRole("heading", { name: "BatCave", exact: true })).toBeVisible();

  if (state === "overview") {
    await expect(page.getByRole("heading", { name: /Machine CPU is/i })).toBeVisible();
    await expect(page.getByRole("region", { name: "System resources" })).toBeVisible();
  } else if (state === "process") {
    await expect(page.locator('[aria-label="Workload inspector"]')).toBeVisible();
  } else if (state === "group") {
    await expect(page.locator('[aria-label="Workload group inspector"]')).toBeVisible();
  } else if (state === "settings" || state === "diagnostics") {
    const name = state === "settings" ? "Settings" : "Diagnostics";
    await expect(page.getByRole("dialog", { name })).toBeVisible();
  } else if (state === "stale") {
    await expect(
      page.getByRole("button", {
        name: "Last sample. Open diagnostics.",
        exact: true,
      }),
    ).toBeVisible();
  } else if (state === "degraded") {
    await expect(page.getByRole("button", { name: /Open diagnostics/ })).toBeVisible();
    await expect(page.locator(".overview-attention")).toBeVisible();
  }
}

async function expectNoAxeViolations(page: Page): Promise<void> {
  const result = await new AxeBuilder({ page }).withTags(wcagTags).analyze();
  expect(result.violations, formatViolations(result.violations)).toEqual([]);
}

async function expectLogicalControlFocused(
  page: Page,
  attribute: "data-workload-id" | "data-resource-mode",
  identity: string,
): Promise<void> {
  await expect
    .poll(() =>
      page.evaluate(
        ({ attribute, identity }) => document.activeElement?.getAttribute(attribute) === identity,
        { attribute, identity },
      ),
    )
    .toBe(true);
}

function formatViolations(
  violations: Awaited<ReturnType<AxeBuilder["analyze"]>>["violations"],
): string {
  return violations
    .map(
      (violation) =>
        `${violation.id}: ${violation.help}\n${violation.nodes
          .map((node) => `  ${node.target.join(" ")}\n    ${node.failureSummary ?? ""}`)
          .join("\n")}`,
    )
    .join("\n\n");
}

for (const state of [
  "overview",
  "process",
  "group",
  "settings",
  "diagnostics",
  "stale",
  "degraded",
] as const) {
  test(`${state} fixture has no automated WCAG A/AA violations`, async ({ page }) => {
    await page.setViewportSize({ width: 1440, height: 900 });
    await openFixture(page, state);
    await expectNoAxeViolations(page);
  });
}

test("compact workload detail has no automated WCAG A/AA violations", async ({ page }) => {
  await page.setViewportSize({ width: 760, height: 900 });
  await openFixture(page, "compact");
  await expect(page.getByRole("dialog", { name: "Resource detail" })).toBeVisible();
  await expectNoAxeViolations(page);
});

test("diagnostics exposes collector-service identity without a helper action", async ({ page }) => {
  await openFixture(page, "diagnostics");
  const dialog = page.getByRole("dialog", { name: "Diagnostics" });
  await page.getByText("Technical details", { exact: true }).click();

  await expect(dialog.getByText("Collector service active", { exact: true }).first()).toBeVisible();
  await expect(dialog.getByText("Installed collector service", { exact: true })).toBeVisible();
  await expect(dialog.getByText("accessibility-fixture-service", { exact: true })).toBeVisible();
  await expect(page.getByRole("button", { name: /helper/i })).toHaveCount(0);
});

test("every theme family renders in both modes and System follows the OS", async ({ page }) => {
  await page.emulateMedia({ colorScheme: "dark" });
  await openFixture(page, "settings");
  const shell = page.locator(".app-shell");
  const families = ["Cave", "Aurora", "Ember", "Canopy"] as const;

  for (const family of families) {
    await page.getByRole("button", { name: `Use the ${family} theme family` }).click();
    for (const mode of ["light", "dark"] as const) {
      await page.getByRole("button", { name: `Use the ${mode} appearance` }).click();
      await expect(shell).toHaveAttribute("data-theme", family.toLocaleLowerCase());
      await expect(shell).toHaveAttribute("data-mode", mode);
    }
  }

  await page.getByRole("button", { name: "Follow the system appearance" }).click();
  await expect(shell).toHaveAttribute("data-theme", "canopy");
  await expect(shell).toHaveAttribute("data-mode", "dark");
  await page.emulateMedia({ colorScheme: "light" });
  await expect(shell).toHaveAttribute("data-theme", "canopy");
  await expect(shell).toHaveAttribute("data-mode", "light");
});

test("enhanced explanations are an explicit local opt-in with a deterministic fallback", async ({
  page,
}) => {
  await openFixture(page, "settings");
  const dialog = page.getByRole("dialog", { name: "Settings" });
  await expect(dialog.getByText("Enhanced explanations — not available on this Mac")).toBeVisible();
  await dialog.getByText("Enhanced explanations — not available on this Mac").click();
  const toggle = dialog.getByRole("switch", {
    name: "Use local AI to choose explanations",
  });
  await expect(toggle).not.toBeChecked();
  await expect(
    dialog.getByText("Off by default. Deterministic explanations always remain available."),
  ).toBeVisible();
  await expect(
    dialog.getByText(/paths, process IDs, diagnostics, and other workloads are excluded/i),
  ).toBeVisible();

  await page.keyboard.press("Escape");
  await page.getByRole("button", { name: "Overview" }).click();
  await expect(page.locator(".narrative-origin")).toHaveCount(0);
});

test("matched icon provenance is consistent across Overview, Explore, compact cards, and inspectors", async ({
  page,
}) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  await openFixture(page, "overview");

  const overviewMatch = page.locator(".overview-workload-list .process-icon.matched").first();
  await expect(overviewMatch).toBeVisible();
  await expect(overviewMatch).toHaveAttribute("title", "Icon matched from a related process");
  await expect(page.locator('.process-icon:not(.matched)[title*="matched"]')).toHaveCount(0);
  await expect(
    page.locator(".overview-workload-list .process-icon.has-image:not(.matched)").first(),
  ).toBeVisible();
  await expect(page.locator(".process-icon-batcave:not(.matched)").first()).toBeVisible();

  const workloadButton = overviewMatch.locator("..");
  const workloadId = await workloadButton.getAttribute("data-workload-id");
  expect(workloadId).not.toBeNull();
  await workloadButton.click();

  const selectedDesktopRow = page.locator(
    `[data-workload-id="${workloadId}"] .process-icon.matched:visible`,
  );
  await expect(selectedDesktopRow).toBeVisible();
  await expect(page.locator(".process-inspector .process-icon.matched")).toBeVisible();

  await page.setViewportSize({ width: 760, height: 900 });
  await expect(
    page.locator(`[data-workload-id="${workloadId}"] .process-icon.matched:visible`),
  ).toBeVisible();
});

test("the neutral matched marker remains distinct in every family and mode", async ({ page }) => {
  await openFixture(page, "settings");
  const marker = page.locator(".process-icon.matched").first();
  const families = ["Cave", "Aurora", "Ember", "Canopy"] as const;

  for (const family of families) {
    await page.getByRole("button", { name: `Use the ${family} theme family` }).click();
    for (const mode of ["light", "dark"] as const) {
      await page.getByRole("button", { name: `Use the ${mode} appearance` }).click();
      const markerStyle = await marker.evaluate((element) => {
        const style = getComputedStyle(element, "::after");
        return {
          background: style.backgroundColor,
          border: style.borderBottomColor,
          height: Number.parseFloat(style.height),
          width: Number.parseFloat(style.width),
        };
      });
      expect(markerStyle.width).toBeGreaterThanOrEqual(6);
      expect(markerStyle.width).toBeLessThanOrEqual(8);
      expect(markerStyle.height).toBe(markerStyle.width);
      expect(markerStyle.background).not.toBe(markerStyle.border);
    }
  }
});

for (const drawer of ["Settings", "Diagnostics"] as const) {
  test(`${drawer} dialog closes with Escape, contains focus, and restores its opener`, async ({
    page,
  }) => {
    await openFixture(page, drawer === "Settings" ? "overview" : "stale");
    const opener = page
      .getByRole("button", {
        name: drawer === "Settings" ? "Settings" : /Open diagnostics/,
      })
      .first();
    await opener.focus();
    await opener.click();

    const dialog = page.getByRole("dialog", { name: drawer });
    await expect(dialog).toBeVisible();
    await expect
      .poll(() => page.evaluate(() => document.activeElement?.closest("dialog") !== null))
      .toBe(true);

    await page.keyboard.press("Shift+Tab");
    await expect
      .poll(() => page.evaluate(() => document.activeElement?.closest("dialog") !== null))
      .toBe(true);
    await page.keyboard.press("Tab");
    await expect(
      page.getByRole("button", { name: `Close ${drawer.toLocaleLowerCase()}` }),
    ).toBeFocused();
    await page.keyboard.press("Escape");

    await expect(dialog).not.toBeVisible();
    await expect(opener).toBeFocused();
  });
}

test("only the active workload layout is mounted and resizing preserves exploration state", async ({
  page,
}) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  await openFixture(page, "group");
  await page.getByRole("button", { name: "Close resource detail" }).click();
  await page.setViewportSize({ width: 1000, height: 900 });
  await expect(page.locator(".attention-table-wrap")).toHaveCount(1);
  await expect(page.locator(".mobile-process-list")).toHaveCount(0);

  const expand = page.locator(".group-expand").first();
  await expand.click();
  const groupKey = await expand.getAttribute("data-workload-group-key");
  expect(groupKey).toBeTruthy();
  await page.setViewportSize({ width: 899, height: 900 });
  await expect(page.locator(".attention-table-wrap")).toHaveCount(0);
  await expect(page.locator(".mobile-process-list")).toHaveCount(1);
  const mobileExpand = page.locator(`[data-workload-group-key="${groupKey}"]`);
  await expect(mobileExpand).toHaveAttribute("aria-expanded", "true");
  await expect(mobileExpand).toBeFocused();

  const workload = page.locator(".mobile-card-select[data-workload-id]").first();
  const workloadId = await workload.getAttribute("data-workload-id");
  await workload.focus();
  await page.setViewportSize({ width: 900, height: 900 });
  await expect(page.locator(".attention-table-wrap")).toHaveCount(1);
  await expect(page.locator(".mobile-process-list")).toHaveCount(0);
  await expect(page.locator(`[data-workload-group-key="${groupKey}"]`)).toHaveAttribute(
    "aria-expanded",
    "true",
  );
  await expectLogicalControlFocused(page, "data-workload-id", workloadId ?? "");

  await page.locator('[data-view="overview"]').click();
  await page.setViewportSize({ width: 899, height: 900 });
  await page.locator('[data-view="explore"]').click();
  await expect(page.locator(".attention-table-wrap")).toHaveCount(0);
  await expect(page.locator(".mobile-process-list")).toHaveCount(1);
});

test("compact resource detail closes with Escape and restores the selected workload", async ({
  page,
}) => {
  await page.setViewportSize({ width: 760, height: 900 });
  await openFixture(page, "overview");
  const opener = page.locator(".overview-workload-list [data-workload-id]").first();
  const workloadId = await opener.getAttribute("data-workload-id");
  expect(workloadId).not.toBeNull();
  await opener.focus();
  await opener.evaluate((button) => (button as HTMLButtonElement).click());

  const dialog = page.getByRole("dialog", { name: "Resource detail" });
  await expect(dialog).toBeVisible();
  await expect
    .poll(() => page.evaluate(() => document.activeElement?.closest("dialog") !== null))
    .toBe(true);
  const firstControl = dialog.getByRole("button", { name: "System detail" });
  await firstControl.focus();
  await page.keyboard.press("Shift+Tab");
  await expect
    .poll(() => page.evaluate(() => document.activeElement?.closest("dialog") !== null))
    .toBe(true);
  await page.keyboard.press("Tab");
  await expect(firstControl).toBeFocused();
  await page.keyboard.press("Escape");

  await expect(dialog).not.toBeVisible();
  await expectLogicalControlFocused(page, "data-workload-id", workloadId ?? "");
});

test("workload detail keeps its dialog and focus after expanding to desktop", async ({ page }) => {
  await page.setViewportSize({ width: 760, height: 900 });
  await openFixture(page, "overview");
  const workloadControl = page
    .locator(".overview-workload-list [data-workload-id]:visible")
    .first();
  const workloadId = await workloadControl.getAttribute("data-workload-id");
  expect(workloadId).not.toBeNull();
  await workloadControl.evaluate((button) => (button as HTMLButtonElement).click());
  await expect(page.getByRole("dialog", { name: "Resource detail" })).toBeVisible();

  await page.setViewportSize({ width: 1440, height: 900 });

  await expect(page.getByRole("dialog", { name: "Resource detail" })).toBeVisible();
  await expect(page.getByRole("button", { name: "Close resource detail" })).toBeFocused();
  await page.keyboard.press("Escape");
  await expectLogicalControlFocused(page, "data-workload-id", workloadId ?? "");
});

test("workload detail keeps its dialog and focused control after collapsing to compact", async ({
  page,
}) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  await openFixture(page, "process");
  const workloadControl = page.locator('[data-workload-id][aria-pressed="true"]:visible').first();
  const workloadId = await workloadControl.getAttribute("data-workload-id");
  expect(workloadId).not.toBeNull();
  await page.getByRole("button", { name: "Copy workload summary" }).focus();

  await page.setViewportSize({ width: 760, height: 900 });

  await expect(page.getByRole("dialog", { name: "Resource detail" })).toBeVisible();
  await expect(page.getByRole("button", { name: "Copy workload summary" })).toBeFocused();
  await page.keyboard.press("Escape");
  await expectLogicalControlFocused(page, "data-workload-id", workloadId ?? "");
});

test("system detail stays open after expanding and restores navigation on dismissal", async ({
  page,
}) => {
  await page.setViewportSize({ width: 760, height: 900 });
  await openFixture(page, "overview");
  const resourceControl = page.locator('.overview-resource-card[data-resource-mode="memory"]');
  await resourceControl.click();
  await expect(resourceControl).toHaveAttribute("aria-pressed", "true");
  await page.getByRole("button", { name: "Inspect resource", exact: true }).click();
  await expect(page.getByRole("dialog", { name: "Resource detail" })).toBeVisible();

  await page.setViewportSize({ width: 1440, height: 900 });

  await expect(page.getByRole("dialog", { name: "Resource detail" })).toBeVisible();
  await expect(page.getByRole("button", { name: "Close resource detail" })).toBeFocused();
  await page.keyboard.press("Escape");
  await expect(page.locator('[data-view="explore"]')).toBeFocused();
});

test("system detail stays open after collapsing and restores navigation on dismissal", async ({
  page,
}) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  await openFixture(page, "overview");
  const resourceControl = page.locator('.overview-resource-card[data-resource-mode="memory"]');
  await resourceControl.click();
  await expect(resourceControl).toHaveAttribute("aria-pressed", "true");
  await page.getByRole("button", { name: "Inspect resource", exact: true }).click();
  await page.getByRole("button", { name: "Close resource detail" }).focus();

  await page.setViewportSize({ width: 760, height: 900 });

  await expect(page.getByRole("dialog", { name: "Resource detail" })).toBeVisible();
  await expect(page.getByRole("button", { name: "Close resource detail" })).toBeFocused();
  await page.keyboard.press("Escape");
  await expect(page.locator('[data-view="explore"]')).toBeFocused();
});

test("Overview selection and leading rows survive Explore query controls", async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  await openFixture(page, "overview");
  const memory = page.locator('.overview-resource-card[data-resource-mode="memory"]');
  await memory.click();
  await expect(memory).toHaveAttribute("aria-pressed", "true");
  await expect(page.locator('[data-view="overview"]')).toHaveAttribute("aria-current", "page");
  const leadingIds = await page
    .locator(".overview-workload-list [data-workload-id]")
    .evaluateAll((rows) => rows.map((row) => row.getAttribute("data-workload-id")));
  expect(leadingIds.length).toBeGreaterThan(0);
  await page.locator('[data-view="explore"]').click();
  await page
    .getByRole("textbox", { name: "Search apps and processes" })
    .fill("no matching workload for overview independence");
  await page.getByRole("textbox", { name: "Search apps and processes" }).press("Enter");
  await page.locator('[data-view="overview"]').click();
  await expect(memory).toHaveAttribute("aria-pressed", "true");
  await expect
    .poll(() =>
      page
        .locator(".overview-workload-list [data-workload-id]")
        .evaluateAll((rows) => rows.map((row) => row.getAttribute("data-workload-id"))),
    )
    .toEqual(leadingIds);
});

async function openGroupedHeroFixture(page: Page): Promise<string> {
  const fixture = JSON.parse(
    readFileSync(
      new URL("../src-tauri/src/fixtures/runtime-protocol-v4/browser-macos.json", import.meta.url),
      "utf8",
    ),
  ) as GroupedHeroFixture;
  if (fixture.event.kind !== "runtime_snapshot")
    throw new Error("Expected runtime snapshot fixture");
  const payload = fixture.event.payload;
  const group = payload.workloads.find((row) => row.kind === "group");
  if (!group || group.kind !== "group") throw new Error("Expected workload group");
  const oldId = group.detail.stable_id;
  const key = `scope:${"a".repeat(64)}`;
  const id = `group:${key}`;
  group.detail.stable_id = id;
  group.detail.group_key = key;
  group.detail.label = "BatCave";
  const partialLimitation =
    payload.limitations.push({
      code: "group_partial_coverage",
      message: "1 of 2 processes contribute to this aggregate.",
    }) - 1;
  const groupMemory = group.detail.metrics.find(
    (metric) => payload.descriptors[metric[0]].semantic === "resident_memory",
  )!;
  groupMemory[1] = 2 * 1024 ** 3;
  groupMemory[2] = payload.quality_codes.indexOf("partial");
  groupMemory[4] = partialLimitation;
  const memoryCoverage = group.detail.coverage.find(
    (coverage) => coverage.descriptor_index === groupMemory[0],
  )!;
  memoryCoverage.available_contributors = 1;
  memoryCoverage.limitation_index = partialLimitation;
  let memberIndex = 0;
  for (const row of payload.workloads) {
    if (row.kind !== "process" || row.detail.presentation.group_id !== oldId) continue;
    row.detail.presentation.group_id = id;
    row.detail.presentation.group_key = key;
    row.detail.presentation.group_label = "BatCave";
    const memory = row.detail.metrics.find(
      (metric) => payload.descriptors[metric[0]].semantic === "resident_memory",
    )!;
    memory[1] = memberIndex === 0 ? 2 * 1024 ** 3 : null;
    memory[2] = payload.quality_codes.indexOf(memberIndex === 0 ? "native" : "unavailable");
    memory[3] = memberIndex === 0 ? payload.sampled_at_ms : null;
    memory[4] = memberIndex === 0 ? null : partialLimitation;
    memberIndex += 1;
  }
  expect(memberIndex).toBe(2);
  // Replace only protocol data; the real decoder, ranking, selection and inspector run.
  await page.route("**/browser-macos.json?import", (route) =>
    route.fulfill({
      contentType: "text/javascript",
      body: `export default ${JSON.stringify(fixture)};`,
    }),
  );
  await page.setViewportSize({ width: 1440, height: 900 });
  await openFixture(page, "overview");
  return id;
}

test("Overview hero opens the same scoped group and aggregate shown in leading workloads", async ({
  page,
}) => {
  const id = await openGroupedHeroFixture(page);
  const hero = page.locator("[data-overview-contributor-id]");
  await expect(hero).toHaveAttribute("data-overview-contributor-id", id);
  await expect(hero.locator("strong")).toHaveText("BatCave");
  await expect(hero.locator("small")).toHaveText("2 processes · 18% of one core");
  await expect(page.locator(".overview-workload-list [data-workload-id]").first()).toHaveAttribute(
    "data-workload-id",
    id,
  );
  await expect(page.locator(".overview-contributor .narrative-origin")).toHaveCount(0);
  await hero.click();
  const inspector = page.getByRole("dialog", { name: "Resource detail" });
  await expect(inspector).toBeVisible();
  await expect(inspector.locator('[aria-label="Workload group inspector"]')).toBeVisible();
  await expect(inspector).toContainText("BatCave");
  await expect(inspector).toContainText("18%");
  await expect(page.locator(`.attention-table [data-workload-id="${id}"]`)).toHaveAttribute(
    "aria-pressed",
    "true",
  );
  await page.keyboard.press("Escape");
  await expect(inspector).toHaveCount(0);
});

test("Overview hero keeps group coverage and physical disk attribution separate", async ({
  page,
}) => {
  const id = await openGroupedHeroFixture(page);
  await page.locator('.overview-resource-card[data-resource-mode="memory"]').click();
  const hero = page.locator("[data-overview-contributor-id]");
  await expect(hero).toHaveAttribute("data-overview-contributor-id", id);
  await expect(hero.locator("small")).toHaveText(
    "2 processes · 2.0 GB resident memory · 1/2 · limited",
  );
  await expect(page.locator(".overview-workload-list [data-workload-id]").first()).toHaveAttribute(
    "data-workload-id",
    id,
  );
  await page.locator('[data-view="explore"]').click();
  await page
    .getByRole("textbox", { name: "Search apps and processes" })
    .fill("no matching grouped workload");
  await page.getByRole("textbox", { name: "Search apps and processes" }).press("Enter");
  await page.locator('[data-view="overview"]').click();
  await expect(hero).toHaveAttribute("data-overview-contributor-id", id);
  await expect(hero.locator("small")).toContainText("1/2 · limited");
  await page.locator('.overview-resource-card[data-resource-mode="network"]').click();
  await expect(hero).toHaveCount(0);
  await expect(page.locator(".overview-contributor")).toContainText(
    "No available workload attribution for this resource",
  );
  await page.locator('.overview-resource-card[data-resource-mode="disk"]').click();
  await expect(hero).toHaveCount(0);
  await expect(page.locator(".overview-contributor")).toContainText(
    "No compatible process attribution",
  );
  await expect(page.locator(".overview-workloads")).toContainText(
    "Sorted by process read/write I/O",
  );
});

for (const width of [1440, 760]) {
  test(`group inspection actions expose exact selection state at ${width}px`, async ({ page }) => {
    await page.setViewportSize({ width: 1440, height: 900 });
    await openFixture(page, "group");
    await page.getByRole("button", { name: "Close resource detail" }).click();
    await page.setViewportSize({ width, height: 900 });
    const surface = page.locator(width === 1440 ? ".attention-table" : ".mobile-process-list");
    const group = surface.locator('[data-workload-id="group:batcave.app.exe"]');
    await expect(group).toHaveAttribute("aria-pressed", "true");
    await surface.locator('[data-workload-group-key="batcave.app.exe"]').click();
    const child = surface.locator('[data-workload-id="process:1234:1699999999000"]');
    await child.click();
    await page.getByRole("button", { name: "Close resource detail" }).click();
    await expect(child).toHaveAttribute("aria-pressed", "true");
    await expect(group).toHaveAttribute("aria-pressed", "false");
    await group.click();
    await page.getByRole("button", { name: "Close resource detail" }).click();
    await expect(group).toHaveAttribute("aria-pressed", "true");
    await expect(child).toHaveAttribute("aria-pressed", "false");
    if (width === 760) {
      await expect(group.locator(".card-metrics")).toContainText("Resident memory");
      await expect(group.locator(".card-metrics")).not.toContainText("Working set");
    }
  });
}

test("compact workload controls change the active sort direction", async ({ page }) => {
  await page.setViewportSize({ width: 760, height: 900 });
  await openFixture(page, "overview");
  await page.locator('[data-view="explore"]').click();
  const descending = page.getByRole("button", {
    name: "Sort direction: descending. Change to ascending.",
    exact: true,
  });
  await expect(descending).toHaveText("Desc");
  await descending.click();
  const ascending = page.getByRole("button", {
    name: "Sort direction: ascending. Change to descending.",
    exact: true,
  });
  await expect(ascending).toHaveText("Asc");
  await ascending.click();
  await expect(descending).toHaveText("Desc");
});

test("desktop workload table renders unavailable network attribution", async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  await openFixture(page, "group");
  await page.getByRole("button", { name: "Close resource detail" }).click();
  const table = page.locator(".attention-table");
  await expect(table.getByRole("columnheader", { name: /Network/ })).toBeVisible();
  const row = table.locator("tr").filter({
    has: page.locator('[data-workload-id="group:batcave.app.exe"]'),
  });
  const networkColumn = await table
    .getByRole("columnheader", { name: /Network/ })
    .evaluate((header) => Array.from(header.parentElement!.children).indexOf(header));
  const qualityReason = await row.locator("td").nth(networkColumn).getAttribute("title");
  expect(qualityReason).toBeTruthy();
  await expect(row.locator("td").nth(networkColumn)).toHaveAttribute(
    "aria-label",
    `Unavailable. ${qualityReason}`,
  );
  await expect(row.locator("td").nth(networkColumn)).toHaveText("—");
});

test("updater checks start only from the explicit Settings action and retry", async ({ page }) => {
  let checks = 0;
  await page.exposeFunction("batcaveUpdaterCheck", () => {
    checks += 1;
  });
  await page.route("**/@tauri-apps_plugin-updater.js*", (route) =>
    route.fulfill({
      contentType: "text/javascript",
      body: 'export async function check() { await window.batcaveUpdaterCheck(); throw new Error("fixture updater unavailable"); }',
    }),
  );
  await openFixture(page, "settings");
  expect(checks).toBe(0);
  const dialog = page.getByRole("dialog", { name: "Settings" });
  await dialog.getByRole("button", { name: "Check now", exact: true }).click();
  await expect.poll(() => checks).toBe(1);
  await expect(dialog).toContainText(
    "Unable to check for updates. Monitoring remains available offline.",
  );
  await dialog.getByRole("button", { name: "Retry", exact: true }).click();
  await expect.poll(() => checks).toBe(2);
});

for (const viewport of [
  { width: 1280, textScale: 100 },
  { width: 1280, textScale: 200 },
  { width: 760, textScale: 100 },
  { width: 360, textScale: 200 },
] as const) {
  test(`Overview rate pairs remain readable as samples change at ${viewport.width}px and ${viewport.textScale}% text`, async ({
    page,
  }) => {
    const fixtureUrl = "/src-tauri/src/fixtures/runtime-protocol-v4/browser-macos.json?import";
    const fixture: unknown = JSON.parse(
      readFileSync(
        new URL(
          "../src-tauri/src/fixtures/runtime-protocol-v4/browser-macos.json",
          import.meta.url,
        ),
        "utf8",
      ),
    );
    // Substitute data only. The real fixture decoder, sampling action, formatter, and UI still run.
    await page.route("**/browser-macos.json?import", (route) =>
      route.fulfill({
        contentType: "text/javascript",
        body: `export default ${JSON.stringify(fixture)};`,
      }),
    );
    await page.setViewportSize({ width: viewport.width, height: 1000 });
    await openFixture(page, "overview");
    if (viewport.textScale !== 100) {
      await page.addStyleTag({
        content: `:root { font-size: ${viewport.textScale}% !important; }`,
      });
    }

    const heights = new Map<string, number>();
    const samples = [
      {
        rates: [12 * 1024, 4 * 1024, 8 * 1024, 3 * 1024],
        disk: ["18 KB/s", "6.1 KB/s"],
        network: ["12 KB/s", "4.6 KB/s"],
      },
      {
        rates: [196 * 1024 ** 2, 131 * 1024 ** 2, 123 * 1024 ** 2, 87 * 1024 ** 2],
        disk: ["314 MB/s", "210 MB/s"],
        network: ["197 MB/s", "139 MB/s"],
      },
      {
        rates: [0, 1024 ** 2, 99.4 * 1024 ** 2, 999 * 1024],
        disk: ["0 B/s", "1.7 MB/s"],
        network: ["167 MB/s", "1.6 MB/s"],
      },
    ];

    for (const [index, sample] of samples.entries()) {
      await page.evaluate(
        async ({ fixtureUrl, rates }) => {
          const imported = (await import(fixtureUrl)) as { default: RateFixture };
          if (imported.default.event.kind !== "runtime_snapshot")
            throw new Error("Expected snapshot fixture");
          const payload = imported.default.event.payload;
          const semantics = [
            "physical_disk_read_rate",
            "physical_disk_write_rate",
            "network_receive_rate",
            "network_transmit_rate",
          ];
          for (const observation of payload.system.metrics) {
            const semantic = payload.descriptors[observation[0]].semantic;
            const rateIndex = semantics.indexOf(semantic);
            const diskTotal =
              semantic === "physical_disk_read_total" || semantic === "physical_disk_write_total";
            if (rateIndex < 0 && !diskTotal) continue;
            observation[1] = diskTotal ? 1024 ** 3 : rates[rateIndex];
            observation[2] = payload.quality_codes.indexOf("native");
            observation[3] = payload.sampled_at_ms;
            observation[4] = null;
          }
        },
        { fixtureUrl, rates: sample.rates },
      );
      await page.locator(".settings-action").click();
      await page
        .getByRole("dialog", { name: "Settings" })
        .getByRole("button", { name: "Refresh now", exact: true })
        .click();
      await page.keyboard.press("Escape");

      // The explicit oracle covers the fixture's next three animated samples (ticks 9, 10, 11).
      for (const [mode, expectedRates, labels] of [
        ["disk", sample.disk, ["Read", "Write"]],
        ["network", sample.network, ["Down", "Up"]],
      ] as const) {
        const card = page.locator(`.overview-resource-card[data-resource-mode="${mode}"]`);
        for (const rate of expectedRates) {
          await expect(card.locator(".resource-card-copy")).toContainText(rate);
        }
        for (const label of labels)
          await expect(card.locator(".resource-card-copy")).toContainText(new RegExp(label, "i"));

        const geometry = await card.evaluate((element, expectedRates) => {
          const copy = element.querySelector(".resource-card-copy")!;
          const rect = (value: DOMRect) => ({
            left: value.left,
            right: value.right,
            top: value.top,
            bottom: value.bottom,
            height: value.height,
          });
          const textNodes: Text[] = [];
          const walker = document.createTreeWalker(copy, NodeFilter.SHOW_TEXT);
          while (walker.nextNode()) textNodes.push(walker.currentNode as Text);
          const text = textNodes
            .map((node) => node.textContent ?? "")
            .join("")
            .replaceAll("\u00a0", " ");
          function rangeRects(start: number, end: number) {
            const range = document.createRange();
            let position = 0;
            let started = false;
            for (const node of textNodes) {
              const next = position + node.length;
              if (!started && start < next) {
                range.setStart(node, start - position);
                started = true;
              }
              if (started && end <= next) {
                range.setEnd(node, end - position);
                break;
              }
              position = next;
            }
            return Array.from(range.getClientRects())
              .filter((item) => item.width > 0 && item.height > 0)
              .map(rect);
          }
          return {
            card: rect(element.getBoundingClientRect()),
            copy: rect(copy.getBoundingClientRect()),
            obstacles: [".resource-icon", ".resource-card-chart", ".resource-card-value"].flatMap(
              (selector) => {
                const node = element.querySelector(selector);
                return node && getComputedStyle(node).display !== "none"
                  ? [rect(node.getBoundingClientRect())]
                  : [];
              },
            ),
            textRects: textNodes.flatMap((node) => {
              const range = document.createRange();
              range.selectNodeContents(node);
              return Array.from(range.getClientRects())
                .filter((item) => item.width > 0 && item.height > 0)
                .map(rect);
            }),
            rates: expectedRates.map((value) => {
              const start = text.indexOf(value);
              return { value, rects: start < 0 ? [] : rangeRects(start, start + value.length) };
            }),
          };
        }, expectedRates);
        const context = `${mode}, sample ${index}, ${viewport.width}px, ${viewport.textScale}% text`;
        for (const rate of geometry.rates) {
          expect
            .soft(rate.rects.length, `${context}: ${rate.value} must render`)
            .toBeGreaterThan(0);
          const lines = new Set(rate.rects.map((item) => Math.round(item.top)));
          expect
            .soft(lines.size, `${context}: ${rate.value} must keep its value and unit together`)
            .toBe(1);
        }
        for (const rect of geometry.textRects) {
          expect
            .soft(rect.left, `${context}: text leaves card left`)
            .toBeGreaterThanOrEqual(geometry.card.left - 1);
          expect
            .soft(rect.right, `${context}: text leaves card right`)
            .toBeLessThanOrEqual(geometry.card.right + 1);
          expect
            .soft(rect.left, `${context}: text clips left`)
            .toBeGreaterThanOrEqual(geometry.copy.left - 1);
          expect
            .soft(rect.right, `${context}: text clips right`)
            .toBeLessThanOrEqual(geometry.copy.right + 1);
          expect
            .soft(rect.top, `${context}: text clips top`)
            .toBeGreaterThanOrEqual(geometry.card.top - 1);
          expect
            .soft(rect.bottom, `${context}: text clips bottom`)
            .toBeLessThanOrEqual(geometry.card.bottom + 1);
          for (const obstacle of geometry.obstacles) {
            const overlaps =
              Math.min(rect.right, obstacle.right) - Math.max(rect.left, obstacle.left) > 1 &&
              Math.min(rect.bottom, obstacle.bottom) - Math.max(rect.top, obstacle.top) > 1;
            expect
              .soft(overlaps, `${context}: supporting text overlaps icon, chart, or total`)
              .toBe(false);
          }
        }
        if (heights.has(mode)) {
          expect
            .soft(geometry.card.height, `${context}: changing rates must not resize the card`)
            .toBeCloseTo(heights.get(mode)!, 0);
        } else {
          heights.set(mode, geometry.card.height);
        }
      }
    }
  });
}

test("Overview drill-down and Explore controls preserve the workload task", async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  await openFixture(page, "overview");
  const workloadControl = page
    .locator(".overview-workload-list [data-workload-id]:visible")
    .first();
  const workloadId = await workloadControl.getAttribute("data-workload-id");
  expect(workloadId).not.toBeNull();
  await workloadControl.evaluate((button) => (button as HTMLButtonElement).click());

  await expect(page.getByRole("dialog", { name: "Resource detail" })).toBeVisible();
  await page.getByRole("button", { name: "Close resource detail" }).click();
  await expect(page.getByRole("heading", { name: "Workloads" })).toBeVisible();
  await expect(
    page.locator(`[data-workload-id="${workloadId}"][aria-pressed="true"]:visible`).first(),
  ).toBeVisible();

  const search = page.getByRole("textbox", {
    name: "Search apps and processes",
  });
  await search.fill("BatCave");
  await expect(search).toHaveValue("BatCave");
  await page.getByRole("button", { name: "I/O active", exact: true }).click();
  await expect(page.getByRole("combobox", { name: "Process sort" })).toBeVisible();
});

test("diagnostics stays horizontally contained and vertically reachable with dense text", async ({
  page,
}) => {
  await page.setViewportSize({ width: 360, height: 640 });
  await openFixture(page, "diagnostics");
  await page.getByText("Technical details", { exact: true }).click();
  await page.addStyleTag({ content: ":root { font-size: 200% !important; }" });

  const scrollPane = page.locator(".diagnostics-drawer .drawer-scroll");
  await expect(scrollPane).toBeVisible();
  const bounds = await scrollPane.evaluate((element) => {
    const node = element as HTMLElement;
    node.scrollTop = node.scrollHeight;
    return {
      clientWidth: node.clientWidth,
      scrollWidth: node.scrollWidth,
      clientHeight: node.clientHeight,
      scrollHeight: node.scrollHeight,
      scrollTop: node.scrollTop,
    };
  });

  expect(bounds.scrollWidth).toBeLessThanOrEqual(bounds.clientWidth + 1);
  expect(bounds.scrollHeight).toBeGreaterThan(bounds.clientHeight);
  expect(bounds.scrollTop).toBeGreaterThan(0);
  expect(bounds.scrollTop + bounds.clientHeight).toBeGreaterThanOrEqual(bounds.scrollHeight - 1);
});

test("independent inspection keeps identity and timestamp when switching A to B to A", async ({
  page,
}) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  await openFixture(page, "process");
  const pane = page.getByRole("dialog", { name: "Resource detail" });
  await pane.getByRole("button", { name: "Memory history" }).click();
  const first = page.locator('[data-workload-id][aria-pressed="true"]:visible').first();
  const id = await first.getAttribute("data-workload-id");
  const identity = await pane.locator(".identity-title-row strong").textContent();
  const timestamp = await pane.locator(".history-readout time").getAttribute("datetime");
  await pane.getByRole("button", { name: "Close resource detail" }).click();
  await first.click();
  await expect(pane.locator(".identity-title-row strong")).toHaveText(identity ?? "");
  await pane.getByRole("button", { name: "Memory history" }).click();
  await expect(pane.locator(".history-readout time")).toHaveAttribute("datetime", timestamp ?? "");
  await pane.getByRole("button", { name: "Close resource detail" }).click();
  await page.locator('[data-view="overview"]').click();
  await expect(pane).not.toBeVisible();
  await page.locator('[data-view="explore"]').click();
  await expect(pane).not.toBeVisible();
  await page.locator(`[data-workload-id="${id}"]:visible`).first().click();
  await expect(pane.locator(".identity-title-row strong")).toHaveText(identity ?? "");
  await pane.getByRole("button", { name: "Memory history" }).click();
  await expect(pane.locator(".history-readout time")).toHaveAttribute("datetime", timestamp ?? "");
  const second = page.locator('[data-workload-id][aria-pressed="false"]:visible').first();
  await pane.getByRole("button", { name: "Close resource detail" }).click();
  await second.click();
  await expect(pane.locator(".identity-title-row strong")).not.toHaveText(identity ?? "");
  await pane.getByRole("button", { name: "Close resource detail" }).click();
  await page.locator(`[data-workload-id="${id}"]:visible`).first().click();
  await expect(pane.locator(".identity-title-row strong")).toHaveText(identity ?? "");
  const memoryHistory = pane.getByRole("button", { name: "Memory history" });
  if ((await memoryHistory.getAttribute("aria-expanded")) !== "true") await memoryHistory.click();
  await expect(pane.locator(".history-readout time")).toHaveAttribute("datetime", timestamp ?? "");
  await expect(pane.locator(".history-readout strong")).toContainText(/[KMGT]?B$/);
  const slider = pane.getByRole("slider", { name: "Recorded sample" });
  await slider.focus();
  await expect(slider).toBeFocused();
  await expectNoAxeViolations(page);
  await page.setViewportSize({ width: 760, height: 900 });
  const dialog = page.getByRole("dialog", { name: "Resource detail" });
  await expect(dialog).toBeVisible();
  await expect(slider).toBeFocused();
  await dialog.getByRole("button", { name: "Close resource detail" }).click();
  const compactSelection = page.locator(`[data-workload-id="${id}"]:visible`).first();
  await compactSelection.click();
  await expect(dialog.locator(".identity-title-row strong")).toHaveText(identity ?? "");
  await dialog.getByRole("button", { name: "Close resource detail" }).click();
  await expect(dialog).not.toBeVisible();
  await compactSelection.click();
  await expect(dialog.locator(".identity-title-row strong")).toHaveText(identity ?? "");
  await dialog.getByRole("button", { name: "Memory history" }).click();
  await expect(dialog.locator(".history-readout time")).toHaveAttribute(
    "datetime",
    timestamp ?? "",
  );
});

test("exited inspection retains exact identity and last sample on desktop and compact layouts", async ({
  page,
}) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  await openFixture(page, "exited");
  const pane = page.getByRole("dialog", { name: "Resource detail" });
  await expect(
    pane.getByText(
      "This process is no longer in the latest sample. Showing its last recorded activity.",
    ),
  ).toBeVisible();
  await expect(pane.getByText("Last recorded activity", { exact: true })).toBeVisible();
  await pane.getByRole("button", { name: "CPU history" }).click();
  await expect(pane.locator(".history-readout time")).toHaveAttribute("datetime", /T/);
  await expectNoAxeViolations(page);
  await page.setViewportSize({ width: 760, height: 900 });
  await expect(page.locator("body")).toHaveJSProperty("scrollWidth", 760);
});

test("Explore stays horizontally contained at the 720px minimum window width", async ({ page }) => {
  await page.setViewportSize({ width: 720, height: 800 });
  await openFixture(page, "overview");
  await page.getByRole("button", { name: "Explore", exact: true }).click();
  await expect(page.locator(".mobile-process-card").first()).toBeVisible();
  await expect(page.locator("html")).toHaveJSProperty("scrollWidth", 720);
});

for (const width of [1440, 1280, 1000, 760, 720]) {
  test(`Explore keeps its full width and inspector dismissal restores its opener at ${width}px`, async ({
    page,
  }) => {
    await page.setViewportSize({ width, height: 900 });
    await openFixture(page, "overview");
    await page.getByRole("button", { name: "Explore", exact: true }).click();
    const dialog = page.getByRole("dialog", { name: "Resource detail" });
    await expect(dialog).toHaveCount(0);
    const queue = page.locator(".explore-queue");
    const workspace = await page.locator(".explore-workspace").boundingBox();
    const before = await queue.boundingBox();
    expect(workspace).not.toBeNull();
    expect(before).not.toBeNull();
    expect(before!.width).toBeCloseTo(workspace!.width, 0);
    const opener = page.locator(".explore-queue button[data-workload-id]").first();

    for (const dismissal of ["button", "escape", "backdrop"]) {
      await opener.focus();
      await opener.click();
      await expect(dialog).toBeVisible();
      await expect(dialog.getByRole("button", { name: "Close resource detail" })).toBeFocused();
      const during = await queue.boundingBox();
      expect(during).toEqual(before);
      if (dismissal === "button") {
        await dialog.getByRole("button", { name: "Close resource detail" }).click();
      } else if (dismissal === "escape") {
        await page.keyboard.press("Escape");
      } else {
        const bounds = await dialog.boundingBox();
        expect(bounds!.x).toBeGreaterThan(8);
        await page.mouse.click(4, 450);
      }
      await expect(dialog).toHaveCount(0);
      await expect(opener).toBeFocused();
      await expect(opener).toHaveAttribute("aria-pressed", "true");
      expect(await queue.boundingBox()).toEqual(before);
    }
  });
}

test("compact unavailable readings expose the actual reason without repeating visible text", async ({
  page,
}) => {
  await page.setViewportSize({ width: 760, height: 900 });
  await openFixture(page, "group");
  await page.getByRole("button", { name: "Close resource detail" }).click();
  const group = page.locator('.mobile-card-select[data-workload-id="group:batcave.app.exe"]');
  await expect(group).toHaveAccessibleDescription(
    /Network Unavailable\. 0 of 2 processes contribute to this aggregate\./,
  );
  const reading = group
    .locator(".card-metrics > span")
    .filter({ has: page.getByText("Network", { exact: true }) })
    .locator("b");
  await expect(reading.locator('[aria-hidden="true"]')).toHaveText("—");
  await expect(reading.locator(".visually-hidden")).toHaveText(
    "Unavailable. 0 of 2 processes contribute to this aggregate.",
  );
});

test("resource detail remains contained and dismissible at minimum width with 200% text", async ({
  page,
}) => {
  await page.setViewportSize({ width: 720, height: 900 });
  await openFixture(page, "overview");
  await page.addStyleTag({ content: ":root { font-size: 200% !important; }" });
  await page.locator('.overview-resource-card[data-resource-mode="memory"]').click();
  await page.getByRole("button", { name: "Inspect resource", exact: true }).click();
  const dialog = page.getByRole("dialog", { name: "Resource detail" });
  const close = dialog.getByRole("button", { name: "Close resource detail" });
  await expect(close).toBeVisible();
  await expect(close).toBeFocused();
  await dialog.getByText("Memory accounting", { exact: true }).click();
  const bounds = await dialog.boundingBox();
  expect(bounds!.x).toBeGreaterThanOrEqual(0);
  expect(bounds!.x + bounds!.width).toBeLessThanOrEqual(720);
  const content = await dialog.locator(".detail-pane-scroll").evaluate((element) => ({
    width: element.clientWidth,
    scrollWidth: element.scrollWidth,
    height: element.clientHeight,
    scrollHeight: element.scrollHeight,
    overflow: [...element.querySelectorAll("*")]
      .filter(
        (child) => child.getBoundingClientRect().right > element.getBoundingClientRect().right + 1,
      )
      .slice(0, 6)
      .map((child) => ({
        tag: child.tagName,
        class: child.className,
        text: child.textContent?.slice(0, 100),
      })),
  }));
  expect(content.scrollWidth, JSON.stringify(content.overflow)).toBeLessThanOrEqual(
    content.width + 1,
  );
  expect(content.scrollHeight).toBeGreaterThan(content.height);
  await expectNoAxeViolations(page);
  await page.keyboard.press("Escape");
  await expect(dialog).toHaveCount(0);
  await expect(page.locator('[data-view="explore"]')).toBeFocused();
});

for (const viewport of [
  { width: 1440, textScale: 100 },
  { width: 720, textScale: 200 },
  { width: 360, textScale: 100 },
  { width: 360, textScale: 200 },
]) {
  test(`Overview status remains readable without moving metrics at ${viewport.width}px and ${viewport.textScale}% text`, async ({
    page,
  }) => {
    const fixtureUrl = "/src-tauri/src/fixtures/runtime-protocol-v4/browser-macos.json?import";
    const fixture: unknown = JSON.parse(
      readFileSync(
        new URL(
          "../src-tauri/src/fixtures/runtime-protocol-v4/browser-macos.json",
          import.meta.url,
        ),
        "utf8",
      ),
    );
    await page.route("**/browser-macos.json?import", (route) =>
      route.fulfill({
        contentType: "text/javascript",
        body: `export default ${JSON.stringify(fixture)};`,
      }),
    );
    await page.setViewportSize({ width: viewport.width, height: 1000 });
    await openFixture(page, "overview");
    await page.addStyleTag({ content: `:root { font-size: ${viewport.textScale}% !important; }` });
    await expect
      .poll(() =>
        page.evaluate(() => parseFloat(getComputedStyle(document.documentElement).fontSize)),
      )
      .toBe((16 * viewport.textScale) / 100);
    await page.evaluate(
      () =>
        new Promise<void>((resolve) =>
          requestAnimationFrame(() => requestAnimationFrame(() => resolve())),
        ),
    );
    const header = page.locator(".app-header");
    const headerControls = [
      page.getByRole("heading", { name: "BatCave", exact: true }),
      header.locator(".view-navigation"),
      header.locator(".header-status-actions"),
    ];
    const headerBoxes = await Promise.all(headerControls.map((control) => control.boundingBox()));
    for (const box of headerBoxes) {
      expect(box).not.toBeNull();
      expect(box!.x).toBeGreaterThanOrEqual(0);
      expect(box!.x + box!.width).toBeLessThanOrEqual(viewport.width + 1);
    }
    for (let index = 0; index < headerBoxes.length; index += 1) {
      for (let other = index + 1; other < headerBoxes.length; other += 1) {
        const left = headerBoxes[index]!;
        const right = headerBoxes[other]!;
        const disjoint =
          left.x + left.width <= right.x ||
          right.x + right.width <= left.x ||
          left.y + left.height <= right.y ||
          right.y + right.height <= left.y;
        expect(disjoint, "Header title, navigation and status controls must not overlap").toBe(
          true,
        );
      }
    }
    const resources = page.locator(".overview-resources");
    const workloads = page.locator(".overview-workloads");
    const beforeResources = await resources.boundingBox();
    const beforeWorkloads = await workloads.boundingBox();
    const reason =
      "A current machine CPU sample is unavailable. " +
      "The detailed local collector reason remains readable while workload controls keep their position. ".repeat(
        20,
      ) +
      "End of the collector reason.";
    await page.evaluate(
      async ({ fixtureUrl, reason }) => {
        const imported = (await import(fixtureUrl)) as {
          default: RateFixture;
        };
        if (imported.default.event.kind !== "runtime_snapshot")
          throw new Error("Expected snapshot fixture");
        const payload = imported.default.event.payload;
        const cpu = payload.system.metrics.find(
          (metric) => payload.descriptors[metric[0]].semantic === "cpu_usage",
        );
        if (!cpu || !payload.limitations[0]) throw new Error("Expected CPU and limitation fixture");
        payload.limitations[0].message = reason;
        cpu[1] = null;
        cpu[2] = payload.quality_codes.indexOf("unavailable");
        cpu[3] = null;
        cpu[4] = 0;
      },
      { fixtureUrl, reason },
    );
    await page.locator(".settings-action").click();
    await page.getByRole("button", { name: "Refresh now", exact: true }).click();
    await page.keyboard.press("Escape");
    const status = page.locator(".overview-status-copy");
    await expect(status).toContainText(reason);
    expect(await resources.boundingBox()).toEqual(beforeResources);
    expect(await workloads.boundingBox()).toEqual(beforeWorkloads);
    const content = await status.evaluate((element) => ({
      width: element.clientWidth,
      scrollWidth: element.scrollWidth,
      height: element.clientHeight,
      scrollHeight: element.scrollHeight,
    }));
    expect(content.scrollWidth).toBeLessThanOrEqual(content.width + 1);
    expect(content.scrollHeight).toBeGreaterThan(content.height);
    await status.focus();
    await status.press("End");
    await expect.poll(() => status.evaluate((element) => element.scrollTop)).toBeGreaterThan(0);
    await expect(page.getByRole("button", { name: "View diagnostics", exact: true })).toBeVisible();
    await expectNoAxeViolations(page);
  });
}
