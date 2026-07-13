import { defineConfig, devices } from "@playwright/test";

export default defineConfig({
  testDir: "./scripts",
  testMatch: "accessibility.spec.ts",
  fullyParallel: false,
  forbidOnly: true,
  retries: 0,
  reporter: "line",
  outputDir: "../../artifacts/accessibility/test-results",
  use: {
    baseURL: "http://127.0.0.1:1420",
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
    command: "npm run dev",
    url: "http://127.0.0.1:1420",
    reuseExistingServer: !process.env.CI,
    timeout: 30_000,
  },
});
