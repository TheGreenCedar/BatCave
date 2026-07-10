import assert from "node:assert/strict";
import test from "node:test";
import { verifyReleaseVersion } from "./verify-release-version.mjs";

const aligned = {
  "package.json": "0.2.0",
  "package-lock.json": "0.2.0",
  "package-lock.json workspace": "0.2.0",
  "Cargo.toml": "0.2.0",
  "tauri.conf.json": "0.2.0",
};

test("accepts an aligned stable tag", () => {
  assert.deepEqual(verifyReleaseVersion("v0.2.0", aligned), {
    version: "0.2.0",
    prerelease: false,
  });
});

test("accepts an aligned prerelease tag", () => {
  const versions = Object.fromEntries(Object.keys(aligned).map((file) => [file, "0.2.0-rc.1"]));
  assert.deepEqual(verifyReleaseVersion("v0.2.0-rc.1", versions), {
    version: "0.2.0-rc.1",
    prerelease: true,
  });
});

test("rejects version drift", () => {
  assert.throws(
    () => verifyReleaseVersion("v0.2.0", { ...aligned, "Cargo.toml": "0.1.0" }),
    /Cargo.toml: 0.1.0/,
  );
});

test("rejects a non-version tag", () => {
  assert.throws(() => verifyReleaseVersion("latest", aligned), /must be v<semver>/);
});
