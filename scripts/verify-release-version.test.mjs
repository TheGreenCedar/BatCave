import assert from "node:assert/strict";
import fs from "node:fs";
import test from "node:test";
import { fileURLToPath } from "node:url";
import { readCargoVersion, verifyReleaseVersion } from "./verify-release-version.mjs";

test("accepts an aligned stable tag", () => {
  assert.deepEqual(verifyReleaseVersion("v0.2.0", "0.2.0"), {
    version: "0.2.0",
    prerelease: false,
  });
});

test("accepts an aligned prerelease tag", () => {
  assert.deepEqual(verifyReleaseVersion("v0.2.0-rc.1", "0.2.0-rc.1"), {
    version: "0.2.0-rc.1",
    prerelease: true,
  });
});

test("rejects version drift", () => {
  assert.throws(
    () => verifyReleaseVersion("v0.2.0", "0.1.0"),
    /Cargo.toml: 0.1.0/,
  );
});

test("rejects a non-version tag", () => {
  assert.throws(() => verifyReleaseVersion("latest", "0.2.0"), /must be v<semver>/);
});

test("workspace authors the app version only in Cargo metadata", () => {
  const repoRoot = fileURLToPath(new URL("../", import.meta.url));
  assert.match(readCargoVersion(repoRoot), /^\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?$/);

  const appRoot = new URL("../src/BatCave.App/", import.meta.url);
  const packageJson = JSON.parse(fs.readFileSync(new URL("package.json", appRoot), "utf8"));
  const packageLock = JSON.parse(fs.readFileSync(new URL("package-lock.json", appRoot), "utf8"));
  const tauriConfig = JSON.parse(
    fs.readFileSync(new URL("src-tauri/tauri.conf.json", appRoot), "utf8"),
  );
  assert.equal(Object.hasOwn(packageJson, "version"), false);
  assert.equal(Object.hasOwn(packageLock, "version"), false);
  assert.equal(Object.hasOwn(packageLock.packages[""], "version"), false);
  assert.equal(Object.hasOwn(tauriConfig, "version"), false);
});
