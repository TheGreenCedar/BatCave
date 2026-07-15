import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import test from "node:test";

import { macosDmgPersistenceCaptureInternals } from "./capture-macos-dmg-current-user-persistence.mjs";

const {
  cleanupProofWorkspace,
  copyMountedApplication,
  directoryIdentity,
  helperProcessIds,
  mountInventoryContains,
  parseArgs,
  readStableRegularFile,
  sha256,
  UnsettledDiskImagesError,
  withDiskImagesLock,
  writePrivateArtifact,
} = macosDmgPersistenceCaptureInternals;

function copyFixture(label) {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), `batcave-dmg-${label}-`));
  fs.chmodSync(root, 0o700);
  const privateDmg = path.join(root, "injected-verifier-input.dmg");
  const mountPoint = path.join(root, "mount");
  const installedApp = path.join(root, "Applications", "BatCave Monitor.app");
  const lockPath = path.join(root, "proof.lock");
  const bytes = Buffer.from("opaque test-only verifier input", "utf8");
  fs.mkdirSync(mountPoint, { mode: 0o700 });
  fs.mkdirSync(path.dirname(installedApp), { recursive: true, mode: 0o700 });
  writePrivateArtifact(privateDmg, bytes);
  return {
    root,
    privateDmg,
    expectedDmgDigest: sha256(bytes),
    mountPoint,
    installedApp,
    lockPath,
  };
}

function postVerifyAttachFailureHooks(fixture, overrides = {}) {
  const events = [];
  const helperBaseline = new Set(["101"]);
  const mountBaseline = new Set(["system-volume"]);
  return {
    events,
    hooks: {
      lockPath: fixture.lockPath,
      helperProcessIds: () => helperBaseline,
      mountInventory: () => mountBaseline,
      runHdiutil: (args) => {
        events.push(args[0]);
        if (args[0] === "verify") return;
        throw new Error("injected post-verify attach failure");
      },
      tryHdiutil: (args) => {
        events.push(args[0]);
        return { error: undefined, status: 1 };
      },
      waitMilliseconds: () => {},
      ...overrides,
    },
  };
}

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

test(
  "failed native DMG verification settles without mount helper lock or root residue",
  { skip: process.platform !== "darwin", timeout: 30_000 },
  () => {
    const root = fs.mkdtempSync(path.join(os.tmpdir(), "batcave-dmg-failed-attach-test-"));
    fs.chmodSync(root, 0o700);
    const privateDmg = path.join(root, "invalid.dmg");
    const mountPoint = path.join(root, "mount");
    const installedApp = path.join(root, "Applications", "BatCave Monitor.app");
    fs.mkdirSync(mountPoint, { mode: 0o700 });
    fs.mkdirSync(path.dirname(installedApp), { recursive: true, mode: 0o700 });
    const invalidBytes = Buffer.from("not a disk image", "utf8");
    writePrivateArtifact(privateDmg, invalidBytes);
    const mountIdentity = directoryIdentity(mountPoint);
    const helperBaseline = helperProcessIds();

    assert.throws(
      () =>
        copyMountedApplication({
          privateDmg,
          expectedDmgDigest: sha256(invalidBytes),
          mountPoint,
          installedApp,
        }),
      /fixed hdiutil operation exited/u,
    );
    assert.deepEqual(directoryIdentity(mountPoint), mountIdentity);
    assert.equal(mountInventoryContains(mountPoint), false);
    assert.deepEqual(helperProcessIds(), helperBaseline);
    assert.equal(fs.existsSync("/tmp/batcave-diskimages-proof.lock"), false);
    assert.equal(fs.existsSync(installedApp), false);

    fs.rmSync(root, { recursive: true, force: true });
    assert.equal(fs.existsSync(root), false);
  },
);

test(
  "unproven settlement retains the atomic lock and private root for manual recovery",
  { skip: process.platform === "win32" },
  () => {
    const root = fs.mkdtempSync(path.join(os.tmpdir(), "batcave-dmg-retained-test-"));
    const lock = path.join(root, "proof.lock");
    const workspace = path.join(root, "workspace");
    fs.mkdirSync(workspace, { mode: 0o700 });
    const unsettled = new UnsettledDiskImagesError();

    assert.throws(
      () =>
        withDiskImagesLock(() => {
          throw unsettled;
        }, lock),
      (error) => error === unsettled && error.retainDiskImagesAuthority === true,
    );
    cleanupProofWorkspace(workspace, unsettled);
    assert.equal(fs.existsSync(lock), true);
    assert.equal(fs.existsSync(workspace), true);

    fs.rmSync(lock);
    fs.rmSync(root, { recursive: true, force: true });
    assert.equal(fs.existsSync(root), false);
  },
);

test(
  "post-verify attach failure settles against global mount and helper baselines",
  { skip: process.platform === "win32" },
  () => {
    const fixture = copyFixture("post-verify-attach");
    const { events, hooks } = postVerifyAttachFailureHooks(fixture);
    assert.throws(
      () => copyMountedApplication(fixture, hooks),
      /injected post-verify attach failure/u,
    );
    assert.deepEqual(events.slice(0, 3), ["verify", "attach", "detach"]);
    assert.equal(fs.existsSync(fixture.lockPath), false);
    fs.rmSync(fixture.root, { recursive: true, force: true });
  },
);

test(
  "a global mount delta retains authority after post-verify attach failure",
  { skip: process.platform === "win32" },
  () => {
    const fixture = copyFixture("global-mount-delta");
    let mountObservation = 0;
    const { hooks } = postVerifyAttachFailureHooks(fixture, {
      mountInventory: () => {
        mountObservation += 1;
        return mountObservation === 1
          ? new Set(["system-volume"])
          : new Set(["system-volume", "unexpected-new-volume"]);
      },
    });
    let error;
    assert.throws(
      () => copyMountedApplication(fixture, hooks),
      (candidate) => {
        error = candidate;
        return true;
      },
    );
    assert.equal(error instanceof UnsettledDiskImagesError, true);
    assert.match(error.cause.message, /bounded local settlement observations/u);
    assert.equal(fs.existsSync(fixture.lockPath), true);
    assert.equal(fs.existsSync(fixture.root), true);
    fs.rmSync(fixture.root, { recursive: true, force: true });
  },
);

for (const observer of ["helperProcessIds", "mountInventory"]) {
  test(
    `${observer} failure after a DiskImages attempt retains authority with its cause`,
    { skip: process.platform === "win32" },
    () => {
      const fixture = copyFixture(`${observer}-failure`);
      let observation = 0;
      const { hooks } = postVerifyAttachFailureHooks(fixture, {
        [observer]: () => {
          observation += 1;
          if (observation > 1) throw new Error(`${observer} unavailable`);
          return observer === "helperProcessIds" ? new Set(["101"]) : new Set(["system-volume"]);
        },
      });
      let error;
      assert.throws(
        () => copyMountedApplication(fixture, hooks),
        (candidate) => {
          error = candidate;
          return true;
        },
      );
      assert.equal(error instanceof UnsettledDiskImagesError, true);
      assert.equal(error.cause.message, `${observer} unavailable`);
      assert.equal(fs.existsSync(fixture.lockPath), true);
      assert.equal(fs.existsSync(fixture.root), true);
      fs.rmSync(fixture.root, { recursive: true, force: true });
    },
  );
}
