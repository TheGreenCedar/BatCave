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
      sampled_at_ms: number | null;
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
  const firstControl = dialog.getByRole("button", { name: "System overview" });
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

test("compact workload detail restores its live workload control after expanding to desktop", async ({
  page,
}) => {
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

  await expect(page.getByRole("complementary", { name: "Resource detail" })).toBeVisible();
  await expectLogicalControlFocused(page, "data-workload-id", workloadId ?? "");
});

test("desktop workload detail restores its live workload control after collapsing to compact", async ({
  page,
}) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  await openFixture(page, "process");
  const workloadControl = page.locator('[data-workload-id][aria-pressed="true"]:visible').first();
  const workloadId = await workloadControl.getAttribute("data-workload-id");
  expect(workloadId).not.toBeNull();
  await page.getByRole("button", { name: "Copy workload summary" }).focus();

  await page.setViewportSize({ width: 760, height: 900 });

  await expect(page.getByRole("dialog", { name: "Resource detail" })).not.toBeVisible();
  await expectLogicalControlFocused(page, "data-workload-id", workloadId ?? "");
});

test("compact system detail restores its resource control after expanding to desktop", async ({
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

  await expect(page.getByRole("complementary", { name: "Resource detail" })).toBeVisible();
  await expect(page.locator('[data-view="explore"]')).toBeFocused();
});

test("desktop system detail restores its resource control after collapsing to compact", async ({
  page,
}) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  await openFixture(page, "overview");
  const resourceControl = page.locator('.overview-resource-card[data-resource-mode="memory"]');
  await resourceControl.click();
  await expect(resourceControl).toHaveAttribute("aria-pressed", "true");
  await page.getByRole("button", { name: "Inspect resource", exact: true }).click();
  await page.getByText("Memory accounting", { exact: true }).focus();

  await page.setViewportSize({ width: 760, height: 900 });

  await expect(page.getByRole("dialog", { name: "Resource detail" })).not.toBeVisible();
  await expect(page.locator('[data-view="explore"]')).toBeFocused();
});

test("Overview resource selection and leading rows survive an Explore filter", async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  await openFixture(page, "overview");
  const memory = page.locator('.overview-resource-card[data-resource-mode="memory"]');
  await memory.click();
  await expect(memory).toHaveAttribute("aria-pressed", "true");
  await expect(page.locator('[data-view="overview"]')).toHaveAttribute("aria-current", "page");
  const leadingIds = await page
    .locator(".overview-workload-list [data-workload-id]")
    .evaluateAll((rows) => rows.map((row) => row.getAttribute("data-workload-id")));
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

  await expect(page.getByRole("heading", { name: "Workloads" })).toBeVisible();
  await expect(page.getByRole("complementary", { name: "Resource detail" })).toBeVisible();
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
  const pane = page.getByRole("complementary", { name: "Resource detail" });
  const first = page.locator('[data-workload-id][aria-pressed="true"]:visible').first();
  const id = await first.getAttribute("data-workload-id");
  const identity = await pane.locator(".identity-title-row strong").textContent();
  const timestamp = await pane.locator(".history-readout time").getAttribute("datetime");
  await first.click();
  await expect(pane.locator(".identity-title-row strong")).toHaveText(identity ?? "");
  await expect(pane.locator(".history-readout time")).toHaveAttribute("datetime", timestamp ?? "");
  await page.locator('[data-view="overview"]').click();
  await expect(pane).not.toBeVisible();
  await page.locator('[data-view="explore"]').click();
  await expect(pane.locator(".identity-title-row strong")).toHaveText(identity ?? "");
  await expect(pane.locator(".history-readout time")).toHaveAttribute("datetime", timestamp ?? "");
  const second = page.locator('[data-workload-id][aria-pressed="false"]:visible').first();
  await second.click();
  await expect(pane.locator(".identity-title-row strong")).not.toHaveText(identity ?? "");
  await page.locator(`[data-workload-id="${id}"]:visible`).first().click();
  await expect(pane.locator(".identity-title-row strong")).toHaveText(identity ?? "");
  await expect(pane.locator(".history-readout time")).toHaveAttribute("datetime", timestamp ?? "");
  await pane.getByRole("combobox", { name: "History resource" }).selectOption("memory");
  await expect(pane.locator(".history-readout strong")).toContainText("bytes");
  const slider = pane.getByRole("slider", { name: "Recorded sample" });
  await slider.focus();
  await expect(slider).toBeFocused();
  await expectNoAxeViolations(page);
  await page.setViewportSize({ width: 760, height: 900 });
  const dialog = page.getByRole("dialog", { name: "Resource detail" });
  await expect(dialog).not.toBeVisible();
  const compactSelection = page.locator(`[data-workload-id="${id}"]:visible`).first();
  await compactSelection.click();
  await expect(dialog.locator(".identity-title-row strong")).toHaveText(identity ?? "");
  await dialog.getByRole("button", { name: "Close resource detail" }).click();
  await expect(dialog).not.toBeVisible();
  await compactSelection.click();
  await expect(dialog.locator(".identity-title-row strong")).toHaveText(identity ?? "");
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
  const pane = page.getByRole("complementary", { name: "Resource detail" });
  await expect(
    pane.getByText(
      "This identity is no longer in the latest sample. Showing its last recorded activity.",
    ),
  ).toBeVisible();
  await expect(pane.getByText("Last recorded activity", { exact: true })).toBeVisible();
  await expect(pane.locator(".history-readout time")).toHaveAttribute("datetime", /T/);
  await expectNoAxeViolations(page);
  await page.setViewportSize({ width: 760, height: 900 });
  await expect(page.locator("body")).toHaveJSProperty("scrollWidth", 760);
});
