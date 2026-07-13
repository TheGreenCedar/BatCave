import assert from "node:assert/strict";
import test from "node:test";
import { resolveAccessibilityFixtureState } from "../src/lib/accessibilityFixtures.ts";
import { accessibilityServerSettings } from "./accessibilityServer.ts";

test("accessibility fixtures require a development browser runtime", () => {
  assert.equal(resolveAccessibilityFixtureState("?a11y=overview", true, false), "overview");
  assert.equal(resolveAccessibilityFixtureState("?a11y=overview", false, false), null);
  assert.equal(resolveAccessibilityFixtureState("?a11y=overview", true, true), null);
});

test("accessibility fixtures reject unknown and missing states", () => {
  assert.equal(resolveAccessibilityFixtureState("?a11y=unknown", true, false), null);
  assert.equal(resolveAccessibilityFixtureState("", true, false), null);
});

test("accessibility server owns a strict worktree-derived or overridden port", () => {
  const first = accessibilityServerSettings("/workspace/one", undefined);
  const second = accessibilityServerSettings("/workspace/two", undefined);
  assert.notEqual(first.port, second.port);
  assert.equal(first.reuseExistingServer, false);
  assert.equal(first.baseUrl, `http://127.0.0.1:${first.port}`);
  assert.equal(first.command, `npm run dev -- --host 127.0.0.1 --strictPort --port ${first.port}`);

  const overridden = accessibilityServerSettings("/workspace/one", "32123");
  assert.equal(overridden.port, 32123);
  assert.throws(() => accessibilityServerSettings("/workspace/one", "not-a-port"));
});
