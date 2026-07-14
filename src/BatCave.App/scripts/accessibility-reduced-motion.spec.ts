import { expect, test } from "@playwright/test";

function cssTimeMilliseconds(value: string): number {
  const durations = value.split(",").map((entry) => {
    const duration = entry.trim();
    if (duration.endsWith("ms")) {
      return Number.parseFloat(duration);
    }
    if (duration.endsWith("s")) {
      return Number.parseFloat(duration) * 1_000;
    }
    return Number.NaN;
  });

  expect(durations.every(Number.isFinite), `invalid CSS duration: ${value}`).toBe(true);
  return Math.max(...durations);
}

test("accessibility runtime emulates reduced motion and applies bounded motion styles", async ({
  page,
}) => {
  await page.goto("/?a11y=overview");
  await expect(page.locator('[data-accessibility-fixture="overview"]')).toBeVisible();

  const result = await page.evaluate(() => {
    const style = document.createElement("style");
    style.textContent = `
      @keyframes batcave-reduced-motion-probe {
        from { opacity: 0; }
        to { opacity: 1; }
      }
    `;
    const probe = document.createElement("div");
    probe.style.animationDuration = "2s";
    probe.style.animationIterationCount = "infinite";
    probe.style.animationName = "batcave-reduced-motion-probe";
    probe.style.scrollBehavior = "smooth";
    probe.style.transitionDuration = "2s";
    probe.style.transitionProperty = "opacity";
    document.head.append(style);
    document.body.append(probe);

    const computed = getComputedStyle(probe);
    const observation = {
      animationDuration: computed.animationDuration,
      animationIterationCount: computed.animationIterationCount,
      mediaMatches: window.matchMedia("(prefers-reduced-motion: reduce)").matches,
      scrollBehavior: computed.scrollBehavior,
      transitionDuration: computed.transitionDuration,
    };

    probe.remove();
    style.remove();
    return observation;
  });

  expect(result.mediaMatches).toBe(true);
  expect(cssTimeMilliseconds(result.animationDuration)).toBeLessThanOrEqual(0.01);
  expect(cssTimeMilliseconds(result.transitionDuration)).toBeLessThanOrEqual(0.01);
  expect(result.animationIterationCount).toBe("1");
  expect(result.scrollBehavior).toBe("auto");
});
