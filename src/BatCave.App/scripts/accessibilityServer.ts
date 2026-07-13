import { createHash } from "node:crypto";

interface AccessibilityServerSettings {
  baseUrl: string;
  command: string;
  port: number;
  reuseExistingServer: false;
}

export function accessibilityServerSettings(
  workspacePath: string,
  portOverride: string | undefined,
): AccessibilityServerSettings {
  const port = portOverride
    ? Number(portOverride)
    : 20_000 +
      (Number.parseInt(createHash("sha256").update(workspacePath).digest("hex").slice(0, 8), 16) %
        25_000);

  if (!Number.isInteger(port) || port < 1024 || port > 65_535) {
    throw new Error("BATCAVE_ACCESSIBILITY_TEST_PORT must be an integer from 1024 through 65535.");
  }

  return {
    baseUrl: `http://127.0.0.1:${port}`,
    command: `npm run dev -- --host 127.0.0.1 --strictPort --port ${port}`,
    port,
    reuseExistingServer: false,
  };
}
