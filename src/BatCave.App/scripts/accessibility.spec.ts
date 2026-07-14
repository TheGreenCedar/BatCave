import AxeBuilder from "@axe-core/playwright";
import { expect, test, type Page } from "@playwright/test";

type FixtureState =
  | "overview"
  | "process"
  | "group"
  | "settings"
  | "diagnostics"
  | "stale"
  | "degraded"
  | "compact";

const wcagTags = ["wcag2a", "wcag2aa", "wcag21a", "wcag21aa"];

async function openFixture(page: Page, state: FixtureState): Promise<void> {
  await page.goto(`/?a11y=${state}`);
  await expect(page.locator(`[data-accessibility-fixture="${state}"]`)).toBeVisible();
  await expect(page.getByRole("heading", { name: "BatCave", exact: true })).toBeVisible();

  if (state === "overview" && (page.viewportSize()?.width ?? 0) >= 1280) {
    await expect(page.getByRole("complementary", { name: "Resource detail" })).toBeVisible();
    await expect(page.getByRole("heading", { name: "Logical cores", exact: true })).toBeVisible();
  } else if (state === "process") {
    await expect(page.locator('[aria-label="Workload inspector"]')).toBeVisible();
  } else if (state === "group") {
    await expect(page.locator('[aria-label="Workload group inspector"]')).toBeVisible();
  } else if (state === "settings" || state === "diagnostics") {
    const name = state === "settings" ? "Settings" : "Diagnostics";
    await expect(page.getByRole("dialog", { name })).toBeVisible();
  } else if (state === "stale") {
    await expect(
      page.getByRole("button", { name: /Telemetry stale\. Open diagnostics\./ }),
    ).toBeVisible();
  } else if (state === "degraded") {
    await expect(
      page.getByRole("button", { name: /App resource warning\. Open diagnostics\./ }),
    ).toBeVisible();
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

for (const drawer of ["Settings", "Diagnostics"] as const) {
  test(`${drawer} dialog closes with Escape, contains focus, and restores its opener`, async ({
    page,
  }) => {
    await openFixture(page, "overview");
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

test("compact resource detail closes with Escape and restores the selected workload", async ({
  page,
}) => {
  await page.setViewportSize({ width: 760, height: 900 });
  await openFixture(page, "overview");
  const opener = page.getByRole("button", { name: /Inspect .+, PID/ }).first();
  await opener.focus();
  await opener.click();

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
  await expect(opener).toBeFocused();
});

test("compact workload detail restores its live workload control after expanding to desktop", async ({
  page,
}) => {
  await page.setViewportSize({ width: 760, height: 900 });
  await openFixture(page, "overview");
  const workloadControl = page.locator("[data-workload-id]:visible").first();
  const workloadId = await workloadControl.getAttribute("data-workload-id");
  expect(workloadId).not.toBeNull();
  await workloadControl.click();
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
  const resourceControl = page.locator('.resource-rail [data-resource-mode="memory"]');
  await resourceControl.click();
  await expect(page.getByRole("dialog", { name: "Resource detail" })).toBeVisible();

  await page.setViewportSize({ width: 1440, height: 900 });

  await expect(page.getByRole("complementary", { name: "Resource detail" })).toBeVisible();
  await expectLogicalControlFocused(page, "data-resource-mode", "memory");
});

test("desktop system detail restores its resource control after collapsing to compact", async ({
  page,
}) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  await openFixture(page, "overview");
  const resourceControl = page.locator('.resource-rail [data-resource-mode="memory"]');
  await resourceControl.click();
  await page.getByText("Memory accounting", { exact: true }).focus();

  await page.setViewportSize({ width: 760, height: 900 });

  await expect(page.getByRole("dialog", { name: "Resource detail" })).not.toBeVisible();
  await expectLogicalControlFocused(page, "data-resource-mode", "memory");
});

test("live refresh holds the focused workload order until keyboard confirmation", async ({
  page,
}) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  await openFixture(page, "overview");
  const workloadControl = page.locator("[data-workload-id]:visible").first();
  const workloadId = await workloadControl.getAttribute("data-workload-id");
  expect(workloadId).not.toBeNull();
  const initialOrder = await page
    .locator(".attention-table [data-workload-id]:visible")
    .evaluateAll((controls) => controls.map((control) => control.getAttribute("data-workload-id")));
  const workloadRow = workloadControl.locator("xpath=ancestor::tr");
  const initialContent = await workloadRow.textContent();
  await workloadControl.focus();

  await page
    .getByRole("button", { name: "Refresh" })
    .evaluate((button) => (button as HTMLButtonElement).click());

  const updateOrder = page.getByRole("button", { name: "Update workload order" });
  await expect(updateOrder).toBeVisible();
  await expectLogicalControlFocused(page, "data-workload-id", workloadId ?? "");
  await expect.poll(() => workloadRow.textContent()).not.toBe(initialContent);
  await expect
    .poll(() =>
      page
        .locator(".attention-table [data-workload-id]:visible")
        .evaluateAll(
          (controls, length) =>
            controls.slice(0, length).map((control) => control.getAttribute("data-workload-id")),
          initialOrder.length,
        ),
    )
    .toEqual(initialOrder);

  await updateOrder.focus();
  await expect(updateOrder).toBeFocused();
  const firstRefreshContent = await workloadRow.textContent();

  await page
    .getByRole("button", { name: "Refresh" })
    .evaluate((button) => (button as HTMLButtonElement).click());

  await expect(updateOrder).toBeVisible();
  await expect(updateOrder).toBeFocused();
  await expect.poll(() => workloadRow.textContent()).not.toBe(firstRefreshContent);
  await expect
    .poll(() =>
      page
        .locator(".attention-table [data-workload-id]:visible")
        .evaluateAll(
          (controls, length) =>
            controls.slice(0, length).map((control) => control.getAttribute("data-workload-id")),
          initialOrder.length,
        ),
    )
    .toEqual(initialOrder);

  await page.keyboard.press("Enter");

  await expect(updateOrder).not.toBeVisible();
  await expect(page.getByRole("combobox", { name: "Process sort" })).toBeFocused();
  await expect
    .poll(() =>
      page
        .locator(".attention-table [data-workload-id]:visible")
        .evaluateAll(
          (controls, length) =>
            controls.slice(0, length).map((control) => control.getAttribute("data-workload-id")),
          initialOrder.length,
        ),
    )
    .not.toEqual(initialOrder);
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
