import assert from "node:assert/strict";
import crypto from "node:crypto";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import test from "node:test";
import {
  buildReleaseInventory,
  stageReleaseAssets,
  verifyReleaseCandidateIdentity,
  verifyReleaseReadback,
} from "./verify-release-candidate.mjs";

const sourceSha = "0123456789abcdef0123456789abcdef01234567";

test("accepts an explicitly approved main-tip candidate", () => {
  assert.deepEqual(
    verifyReleaseCandidateIdentity({
      tag: "v0.3.0-rc.1",
      channel: "prerelease",
      sourceSha,
      mainSha: sourceSha,
      approvedSourceSha: sourceSha,
    }),
    { tag: "v0.3.0-rc.1", sourceSha, prerelease: true },
  );
});

test("rejects candidates that are only ancestors of main or were not explicitly approved", () => {
  const otherSha = "abcdef0123456789abcdef0123456789abcdef01";
  assert.throws(
    () =>
      verifyReleaseCandidateIdentity({
        tag: "v0.3.0",
        channel: "stable",
        sourceSha,
        mainSha: otherSha,
        approvedSourceSha: sourceSha,
      }),
    /must equal origin\/main/,
  );
  assert.throws(
    () =>
      verifyReleaseCandidateIdentity({
        tag: "v0.3.0",
        channel: "stable",
        sourceSha,
        mainSha: sourceSha,
        approvedSourceSha: otherSha,
      }),
    /must equal approved source/,
  );
});

test("rejects a release channel that disagrees with the tag", () => {
  assert.throws(
    () =>
      verifyReleaseCandidateIdentity({
        tag: "v0.3.0-rc.1",
        channel: "stable",
        sourceSha,
        mainSha: sourceSha,
        approvedSourceSha: sourceSha,
      }),
    /expected prerelease/,
  );
});

test("stages nested artifact downloads without silent basename collisions", () => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "batcave-release-stage-"));
  try {
    const input = path.join(root, "input");
    const output = path.join(root, "output");
    fs.mkdirSync(path.join(input, "windows"), { recursive: true });
    fs.mkdirSync(path.join(input, "macos"), { recursive: true });
    fs.writeFileSync(path.join(input, "windows", "BatCave Setup.exe"), "windows");
    fs.writeFileSync(path.join(input, "macos", "BatCave.dmg"), "macos");

    assert.deepEqual(stageReleaseAssets(input, output), ["BatCave.dmg", "BatCave.Setup.exe"]);
    assert.equal(fs.readFileSync(path.join(output, "BatCave.Setup.exe"), "utf8"), "windows");

    const collisionInput = path.join(root, "collision");
    fs.mkdirSync(path.join(collisionInput, "one"), { recursive: true });
    fs.mkdirSync(path.join(collisionInput, "two"), { recursive: true });
    fs.writeFileSync(path.join(collisionInput, "one", "same name"), "one");
    fs.writeFileSync(path.join(collisionInput, "two", "same.name"), "two");
    assert.throws(
      () => stageReleaseAssets(collisionInput, path.join(root, "collision-output")),
      /both normalize to same\.name/,
    );
  } finally {
    fs.rmSync(root, { recursive: true, force: true });
  }
});

test("builds a deterministic name, size, and digest inventory", () => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "batcave-release-inventory-"));
  try {
    fs.writeFileSync(path.join(root, "asset.bin"), "abc");
    assert.deepEqual(buildReleaseInventory("v0.3.0", sourceSha, false, root), {
      tag: "v0.3.0",
      source_sha: sourceSha,
      prerelease: false,
      assets: [
        {
          name: "asset.bin",
          size: 3,
          digest: `sha256:${crypto.createHash("sha256").update("abc").digest("hex")}`,
        },
      ],
    });
  } finally {
    fs.rmSync(root, { recursive: true, force: true });
  }
});

test("verifies draft and published release readback against the same candidate", () => {
  const expected = {
    tag: "v0.3.0",
    source_sha: sourceSha,
    prerelease: false,
    assets: [{ name: "asset.bin", size: 3, digest: "sha256:abc" }],
  };
  const actual = {
    tag_name: "v0.3.0",
    target_commitish: sourceSha,
    draft: true,
    prerelease: false,
    assets: [{ name: "asset.bin", size: 3, digest: "sha256:abc", id: 12 }],
  };
  assert.equal(verifyReleaseReadback(expected, actual, true), true);
  assert.equal(verifyReleaseReadback(expected, { ...actual, draft: false }, false), true);
});

test("rejects source, state, and asset drift in release readback", () => {
  const expected = {
    tag: "v0.3.0",
    source_sha: sourceSha,
    prerelease: false,
    assets: [{ name: "asset.bin", size: 3, digest: "sha256:abc" }],
  };
  const actual = {
    tag_name: "v0.3.0",
    target_commitish: sourceSha,
    draft: true,
    prerelease: false,
    assets: [{ name: "asset.bin", size: 3, digest: "sha256:abc" }],
  };
  assert.throws(
    () => verifyReleaseReadback(expected, { ...actual, target_commitish: "main" }, true),
    /source readback mismatch/,
  );
  assert.throws(() => verifyReleaseReadback(expected, actual, false), /draft readback mismatch/);
  assert.throws(
    () =>
      verifyReleaseReadback(
        expected,
        { ...actual, assets: [{ name: "asset.bin", size: 4, digest: "sha256:def" }] },
        true,
      ),
    /asset readback mismatch/,
  );
});
