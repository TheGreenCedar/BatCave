import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import test from "node:test";

import { macosDmgPersistenceCaptureInternals } from "./capture-macos-dmg-current-user-persistence.mjs";

const { parseArgs, readStableRegularFile, sha256, withDiskImagesLock, writePrivateArtifact } =
  macosDmgPersistenceCaptureInternals;

test("requires a fixed DMG and exact source identity", () => {
  assert.deepEqual(parseArgs(["--dmg", "BatCave.dmg", "--source-sha", "a".repeat(40)]), {
    dmg: path.resolve("BatCave.dmg"),
    sourceSha: "a".repeat(40),
    output: undefined,
  });
  assert.throws(
    () => parseArgs(["--dmg", "BatCave.dmg", "--source-sha", "main"]),
    /exact lowercase 40-character Git SHA-1/u,
  );
  assert.throws(
    () =>
      parseArgs(["--dmg", "BatCave.dmg", "--source-sha", "a".repeat(40), "--command", "hdiutil"]),
    /unknown argument/u,
  );
});

test(
  "private artifact copy is fixed, private, and byte-identical",
  { skip: process.platform === "win32" },
  () => {
    const root = fs.mkdtempSync(path.join(os.tmpdir(), "batcave-dmg-copy-test-"));
    const file = path.join(root, "candidate.dmg");
    const bytes = Buffer.from("fixed candidate bytes", "utf8");
    writePrivateArtifact(file, bytes);
    assert.ok(readStableRegularFile(file, "candidate").equals(bytes));
    assert.equal(fs.lstatSync(file).mode & 0o777, 0o400);
    assert.equal(sha256(readStableRegularFile(file, "candidate")), sha256(bytes));
    fs.rmSync(root, { recursive: true, force: true });
  },
);

test("stable artifact reader rejects symlinks", { skip: process.platform === "win32" }, () => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "batcave-dmg-link-test-"));
  const target = path.join(root, "target.dmg");
  const link = path.join(root, "candidate.dmg");
  fs.writeFileSync(target, "bytes");
  fs.symlinkSync(target, link);
  assert.throws(() => readStableRegularFile(link, "candidate"), /regular non-link file/u);
  fs.rmSync(root, { recursive: true, force: true });
});

test(
  "DiskImages lock is atomic and always released",
  { skip: process.platform === "win32" },
  () => {
    const root = fs.mkdtempSync(path.join(os.tmpdir(), "batcave-diskimages-lock-test-"));
    const lock = path.join(root, "proof.lock");
    assert.equal(
      withDiskImagesLock(() => {
        assert.equal(fs.lstatSync(lock).mode & 0o777, 0o600);
        assert.throws(() => withDiskImagesLock(() => {}, lock), /lock is busy/u);
        return "complete";
      }, lock),
      "complete",
    );
    assert.equal(fs.existsSync(lock), false);
    assert.throws(
      () =>
        withDiskImagesLock(() => {
          throw new Error("hostile helper failure");
        }, lock),
      /hostile helper failure/u,
    );
    assert.equal(fs.existsSync(lock), false);
    fs.rmSync(root, { recursive: true, force: true });
  },
);
