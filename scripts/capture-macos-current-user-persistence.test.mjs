import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import test from "node:test";

import { macosPersistenceCaptureInternals } from "./capture-macos-current-user-persistence.mjs";

const { hashBundleTree, inspectRoot, parseArgs, regularFileInside } =
  macosPersistenceCaptureInternals;

test("requires a fixed app and exact source identity", () => {
  assert.deepEqual(parseArgs(["--app", "BatCave Monitor.app", "--source-sha", "a".repeat(40)]), {
    app: path.resolve("BatCave Monitor.app"),
    sourceSha: "a".repeat(40),
    output: undefined,
  });
  assert.throws(
    () => parseArgs(["--app", "BatCave Monitor.app", "--source-sha", "main"]),
    /exact lowercase 40-character Git SHA-1/u,
  );
  assert.throws(
    () =>
      parseArgs([
        "--app",
        "BatCave Monitor.app",
        "--source-sha",
        "a".repeat(40),
        "--command",
        "rm -rf",
      ]),
    /unknown argument/u,
  );
});

test("canonical app-bundle digest binds relative names and bytes", () => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "batcave-bundle-hash-"));
  fs.mkdirSync(path.join(root, "Contents"));
  fs.writeFileSync(path.join(root, "Contents", "Info.plist"), "one");
  const first = hashBundleTree(root);
  assert.match(first, /^sha256:[0-9a-f]{64}$/u);
  assert.equal(hashBundleTree(root), first);
  fs.writeFileSync(path.join(root, "Contents", "Info.plist"), "two");
  assert.notEqual(hashBundleTree(root), first);
  fs.rmSync(root, { recursive: true, force: true });
});

test("canonical app-bundle digest separates hostile record-shaped file bytes", () => {
  const first = fs.mkdtempSync(path.join(os.tmpdir(), "batcave-bundle-collision-a-"));
  const second = fs.mkdtempSync(path.join(os.tmpdir(), "batcave-bundle-collision-b-"));
  fs.writeFileSync(path.join(first, "a"), Buffer.from("X\0file\0b\0Y"));
  fs.writeFileSync(path.join(second, "a"), "X");
  fs.writeFileSync(path.join(second, "b"), "Y");

  assert.notEqual(hashBundleTree(first), hashBundleTree(second));

  fs.rmSync(first, { recursive: true, force: true });
  fs.rmSync(second, { recursive: true, force: true });
});

test(
  "executable authority rejects file and parent-directory links",
  { skip: process.platform === "win32" },
  () => {
    const root = fs.mkdtempSync(path.join(os.tmpdir(), "batcave-executable-root-"));
    const outside = fs.mkdtempSync(path.join(os.tmpdir(), "batcave-executable-outside-"));
    const outsideExecutable = path.join(outside, "batcave-monitor");
    fs.writeFileSync(outsideExecutable, "external", { mode: 0o700 });
    const linkedFile = path.join(root, "linked-file");
    fs.symlinkSync(outsideExecutable, linkedFile, "file");
    assert.throws(
      () => regularFileInside(root, linkedFile, "app bundle GUI executable"),
      /regular non-link file/u,
    );

    const linkedDirectory = path.join(root, "linked-directory");
    fs.symlinkSync(outside, linkedDirectory, process.platform === "win32" ? "junction" : "dir");
    assert.throws(
      () =>
        regularFileInside(
          root,
          path.join(linkedDirectory, "batcave-monitor"),
          "app bundle GUI executable",
        ),
      /must not traverse a linked app-bundle path/u,
    );

    fs.rmSync(root, { recursive: true, force: true });
    fs.rmSync(outside, { recursive: true, force: true });
  },
);

test(
  "root inspection emits modes and ownership without local paths",
  { skip: process.platform === "win32" },
  () => {
    const root = fs.mkdtempSync(path.join(os.tmpdir(), "batcave-root-inspection-"));
    fs.chmodSync(root, 0o700);
    fs.writeFileSync(path.join(root, "settings.json"), "{}", { mode: 0o600 });
    const evidence = inspectRoot(root);
    assert.deepEqual(evidence, {
      canonical_location: "application_support",
      owner_verified: true,
      permission_model: "unix_mode",
      private_permissions_verified: true,
      directory_mode: "0700",
      files: [
        {
          component: "settings",
          private_permissions_verified: true,
          mode: "0600",
        },
      ],
    });
    assert.ok(!JSON.stringify(evidence).includes(root));
    fs.rmSync(root, { recursive: true, force: true });
  },
);
