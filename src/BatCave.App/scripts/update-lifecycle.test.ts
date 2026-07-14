import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import {
  checkAfterClosingUpdate,
  downloadInstallAndClose,
  type InstallableUpdateResource,
  UpdateResourceCleanupError,
} from "../src/lib/updateLifecycle.ts";

class FixtureUpdate implements InstallableUpdateResource {
  closeCalls = 0;
  installCalls = 0;
  private readonly events: string[];
  private readonly installError: Error | null;
  private readonly closeError: Error | null;

  constructor(
    events: string[],
    installError: Error | null = null,
    closeError: Error | null = null,
  ) {
    this.events = events;
    this.installError = installError;
    this.closeError = closeError;
  }

  async close(): Promise<void> {
    this.closeCalls += 1;
    this.events.push("close");
    if (this.closeError) throw this.closeError;
  }

  async downloadAndInstall(): Promise<void> {
    this.installCalls += 1;
    this.events.push("install");
    if (this.installError) throw this.installError;
  }
}

test("a new check closes the prior Tauri update before requesting another", async () => {
  const events: string[] = [];
  const previous = new FixtureUpdate(events);
  const next = new FixtureUpdate(events);

  const checked = await checkAfterClosingUpdate(previous, async () => {
    events.push("check");
    return next;
  });

  assert.equal(checked, next);
  assert.equal(previous.closeCalls, 1);
  assert.deepEqual(events, ["close", "check"]);
});

test("an already-closed prior resource cannot block an explicit retry", async () => {
  const events: string[] = [];
  const previous = new FixtureUpdate(events, null, new Error("The resource id 42 is invalid."));

  const checked = await checkAfterClosingUpdate(previous, async () => {
    events.push("retry");
    return null;
  });

  assert.equal(checked, null);
  assert.equal(previous.closeCalls, 1);
  assert.deepEqual(events, ["close", "retry"]);
});

test("an arbitrary cleanup failure blocks replacement and remains observable", async () => {
  const events: string[] = [];
  const closeError = new Error("resource table unavailable");
  const previous = new FixtureUpdate(events, null, closeError);

  await assert.rejects(
    () =>
      checkAfterClosingUpdate(previous, async () => {
        events.push("check");
        return null;
      }),
    (error) => error === closeError,
  );
  assert.equal(previous.closeCalls, 1);
  assert.deepEqual(events, ["close"]);
});

test("failed replacement retains the prior handle for a later cleanup retry", async () => {
  const events: string[] = [];
  const previous = new FixtureUpdate(events, null, new Error("resource table unavailable"));
  let pending: FixtureUpdate | null = previous;

  await assert.rejects(async () => {
    pending = await checkAfterClosingUpdate(pending, async () => null);
  });
  assert.equal(pending, previous);
});

test("failed verification or installation closes the abandoned update", async () => {
  const events: string[] = [];
  const update = new FixtureUpdate(events, new Error("signature rejected"));

  await assert.rejects(() => downloadInstallAndClose(update), /signature rejected/);
  assert.equal(update.installCalls, 1);
  assert.equal(update.closeCalls, 1);
  assert.deepEqual(events, ["install", "close"]);
});

test("a completed install also releases the in-process update selection", async () => {
  const events: string[] = [];
  const update = new FixtureUpdate(events);

  await downloadInstallAndClose(update);
  assert.equal(update.installCalls, 1);
  assert.equal(update.closeCalls, 1);
  assert.deepEqual(events, ["install", "close"]);
});

test("combined install and cleanup failure preserves both errors", async () => {
  const events: string[] = [];
  const installError = new Error("signature rejected");
  const closeError = new Error("resource table unavailable");
  const update = new FixtureUpdate(events, installError, closeError);

  await assert.rejects(
    () => downloadInstallAndClose(update),
    (error) => {
      assert.ok(error instanceof UpdateResourceCleanupError);
      assert.equal(error.operationError, installError);
      assert.equal(error.cleanupError, closeError);
      assert.deepEqual(error.errors, [installError, closeError]);
      return true;
    },
  );
  assert.deepEqual(events, ["install", "close"]);
});

test("App keeps cleanup-failed handles owned and updater checks user-triggered", () => {
  const appSource = readFileSync(new URL("../src/App.svelte", import.meta.url), "utf8");
  const settingsSource = readFileSync(
    new URL("../src/lib/components/shell/SettingsDrawer.svelte", import.meta.url),
    "utf8",
  );
  const checkFunction = appSource.slice(
    appSource.indexOf("async function checkForStableUpdate"),
    appSource.indexOf("async function installStableUpdate"),
  );
  const installFunction = appSource.slice(
    appSource.indexOf("async function installStableUpdate"),
    appSource.indexOf("function setSortKey"),
  );
  const startup = appSource.slice(
    appSource.indexOf("onMount(() =>"),
    appSource.indexOf("async function checkForStableUpdate"),
  );

  assert.doesNotMatch(checkFunction, /pendingUpdate = null/);
  assert.doesNotMatch(installFunction, /pendingUpdate = null;[\s\S]*downloadInstallAndClose/);
  assert.match(installFunction, /error instanceof UpdateResourceCleanupError/);
  assert.match(installFunction, /else \{\s+pendingUpdate = null;/);
  assert.doesNotMatch(startup, /checkForStableUpdate/);
  assert.match(appSource, /onCheckForUpdates=\{\(\) => void checkForStableUpdate\(\)\}/);
  assert.match(appSource, /Monitoring remains available offline/);
  assert.match(settingsSource, /updateStatus === "error"\s+\? "Retry"/);
  assert.match(settingsSource, /only when you ask/);
});
