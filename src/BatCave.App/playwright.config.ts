import { defineConfig, devices } from "@playwright/test";
import { fileURLToPath } from "node:url";
import { accessibilityServerSettings } from "./scripts/accessibilityServer";

const server = accessibilityServerSettings(
  fileURLToPath(new URL(".", import.meta.url)),
  process.env.BATCAVE_ACCESSIBILITY_TEST_PORT,
);

export default defineConfig({
  testDir: "./scripts",
  testMatch: "accessibility.spec.ts",
  fullyParallel: false,
  forbidOnly: true,
  retries: 0,
  reporter: "line",
  outputDir: "../../artifacts/accessibility/test-results",
  use: {
    baseURL: server.baseUrl,
    colorScheme: "dark",
    reducedMotion: "reduce",
    screenshot: "off",
    trace: "off",
    video: "off",
  },
  projects: [
    {
      name: "chromium",
      use: { ...devices["Desktop Chrome"] },
    },
  ],
  webServer: {
    command: server.command,
    url: server.baseUrl,
    reuseExistingServer: server.reuseExistingServer,
    timeout: 30_000,
  },
});
