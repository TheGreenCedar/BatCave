import assert from "node:assert/strict";
import test from "node:test";
import { buildUpdateManifest } from "./build-update-manifest.mjs";

const artifacts = {
  "BatCave.Monitor_0.3.0_x64-setup.exe": "",
  "BatCave.Monitor_0.3.0_x64-setup.exe.sig": "windows-signature\n",
  "BatCave.Monitor_0.3.0_amd64.AppImage": "",
  "BatCave.Monitor_0.3.0_amd64.AppImage.sig": "linux-signature\n",
  "BatCave.Monitor.app.tar.gz": "",
  "BatCave.Monitor.app.tar.gz.sig": "macos-signature\n",
};

test("builds stable signed update entries for Windows, Linux, and universal macOS", () => {
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
      "darwin-x86_64": {
        signature: "macos-signature",
        url: "https://github.com/TheGreenCedar/BatCave/releases/download/v0.3.0/BatCave.Monitor.app.tar.gz",
      },
    },
  });
});

test("keeps prerelease versions explicit", () => {
  assert.equal(buildUpdateManifest("v0.3.0-rc.1", "owner/repo", artifacts).version, "0.3.0-rc.1");
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

test("requires exactly one universal macOS updater archive", () => {
  assert.throws(
    () =>
      buildUpdateManifest("v0.3.0", "owner/repo", {
        ...artifacts,
        "another.app.tar.gz": "",
        "another.app.tar.gz.sig": "other-signature",
      }),
    /darwin-aarch64 requires exactly one \.app\.tar\.gz asset/,
  );
});

test("rejects invalid repository names", () => {
  assert.throws(
    () => buildUpdateManifest("v0.3.0", "not-a-repository", artifacts),
    /invalid GitHub repository/,
  );
});
