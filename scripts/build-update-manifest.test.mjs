import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";
import { buildUpdateManifest } from "./build-update-manifest.mjs";

const artifacts = {
  "BatCave.Monitor_0.3.0_x64-setup.exe": "",
  "BatCave.Monitor_0.3.0_x64-setup.exe.sig": "windows-signature\n",
  "BatCave.Monitor_0.3.0_amd64.AppImage": "",
  "BatCave.Monitor_0.3.0_amd64.AppImage.sig": "linux-signature\n",
  "BatCave.Monitor.app.tar.gz": "",
  "BatCave.Monitor.app.tar.gz.sig": "macos-signature\n",
};

function artifactsForVersion(version) {
  return Object.fromEntries(
    Object.entries(artifacts).map(([name, contents]) => [name.replace("0.3.0", version), contents]),
  );
}

test("builds stable signed update entries for Windows, Linux, and Apple Silicon macOS", () => {
  assert.deepEqual(buildUpdateManifest("v0.3.0", "TheGreenCedar/BatCave", artifacts), {
    version: "0.3.0",
    notes: "BatCave v0.3.0",
    platforms: {
      "windows-x86_64": {
        signature: "windows-signature",
        url: "https://github.com/TheGreenCedar/BatCave/releases/download/v0.3.0/BatCave.Monitor_0.3.0_x64-setup.exe",
      },
      "linux-x86_64": {
        signature: "linux-signature",
        url: "https://github.com/TheGreenCedar/BatCave/releases/download/v0.3.0/BatCave.Monitor_0.3.0_amd64.AppImage",
      },
      "darwin-aarch64": {
        signature: "macos-signature",
        url: "https://github.com/TheGreenCedar/BatCave/releases/download/v0.3.0/BatCave.Monitor.app.tar.gz",
      },
    },
  });
});

test("keeps prerelease versions explicit", () => {
  const manifest = buildUpdateManifest(
    "v0.3.0-rc.1",
    "owner/repo",
    artifactsForVersion("0.3.0-rc.1"),
  );
  assert.equal(manifest.version, "0.3.0-rc.1");
  assert.match(manifest.platforms["windows-x86_64"].url, /0\.3\.0-rc\.1_x64-setup/);
});

test("rejects missing signatures", () => {
  assert.throws(
    () =>
      buildUpdateManifest("v0.3.0", "owner/repo", {
        ...artifacts,
        "BatCave.Monitor_0.3.0_x64-setup.exe.sig": "",
      }),
    /missing or empty/,
  );
});

test("requires exactly one Apple Silicon macOS updater archive", () => {
  assert.throws(
    () =>
      buildUpdateManifest("v0.3.0", "owner/repo", {
        ...artifacts,
        "another.app.tar.gz": "",
        "another.app.tar.gz.sig": "other-signature",
      }),
    /macOS Apple Silicon updater payload requires exactly one asset; found 2/,
  );
});

test("binds updater payload and signature names to the exact tag version", () => {
  assert.throws(
    () => buildUpdateManifest("v0.3.0", "owner/repo", artifactsForVersion("9.9.9")),
    /must be named BatCave\.Monitor_0\.3\.0_x64-setup\.exe/,
  );
});

test("rejects invalid repository names", () => {
  assert.throws(
    () => buildUpdateManifest("v0.3.0", "not-a-repository", artifacts),
    /invalid GitHub repository/,
  );
});

test("executable manifest generation binds the tag to Cargo before writing", () => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "batcave-update-version-"));
  try {
    const result = spawnSync(
      process.execPath,
      [
        fileURLToPath(new URL("./build-update-manifest.mjs", import.meta.url)),
        "v9.9.9",
        "owner/repo",
        root,
      ],
      { encoding: "utf8" },
    );
    assert.equal(result.status, 1);
    assert.match(result.stderr, /release tag v9\.9\.9 expects version 9\.9\.9/u);
    assert.match(result.stderr, /Cargo\.toml:/u);
    assert.equal(fs.existsSync(path.join(root, "latest.json")), false);
  } finally {
    fs.rmSync(root, { recursive: true, force: true });
  }
});
