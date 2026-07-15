import assert from "node:assert/strict";
import crypto from "node:crypto";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import test from "node:test";

import { linuxPersistenceCaptureInternals } from "./capture-linux-current-user-persistence.mjs";
import { linuxAppImagePostPublicSmokeInternals } from "./linux-appimage-post-public-smoke.mjs";
import { expectedReleaseAssetRoles } from "./release-asset-contract.mjs";
import {
  CHECKSUM_MANIFEST,
  RELEASE_REPOSITORY,
  verifyPublicRelease,
} from "./verify-public-release.mjs";

const sourceSha = "0123456789abcdef0123456789abcdef01234567";

function digest(contents) {
  return `sha256:${crypto.createHash("sha256").update(contents).digest("hex")}`;
}

async function genuinePublicReceipt() {
  const tag = "v0.3.0";
  const roles = expectedReleaseAssetRoles(tag).roles;
  const provenance = roles.find(({ role }) => role === "build provenance bundle").name;
  const subjects = roles
    .map(({ name }) => name)
    .filter((name) => ![CHECKSUM_MANIFEST, provenance].includes(name));
  const payloads = new Map(subjects.map((name) => [name, `fixture bytes for ${name}\n`]));
  payloads.set(
    CHECKSUM_MANIFEST,
    subjects.map((name) => `${digest(payloads.get(name)).slice(7)}  ./${name}\n`).join(""),
  );
  payloads.set(provenance, '{"bundle":"fixture"}\n');
  const assets = [...payloads].map(([name, contents]) => ({
    name,
    size: Buffer.byteLength(contents),
    digest: digest(contents),
  }));
  const candidate = { tag, source_sha: sourceSha, prerelease: false, assets };
  const release = {
    tag_name: tag,
    target_commitish: sourceSha,
    draft: false,
    prerelease: false,
    immutable: true,
    assets: assets.map((asset) => ({
      ...asset,
      browser_download_url: `https://github.com/${RELEASE_REPOSITORY}/releases/download/${tag}/${asset.name}`,
    })),
  };
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "batcave-appimage-brand-test-"));
  const result = await verifyPublicRelease(candidate, release, path.join(root, "downloads"), {
    fetchImpl: async (url) => {
      const name = decodeURIComponent(new URL(url).pathname.split("/").at(-1));
      return new Response(payloads.get(name), { status: 200 });
    },
    ghRunner: () => {},
  });
  fs.rmSync(root, { force: true, recursive: true });
  return result.receipt;
}

test("accepts only exact workflow-owned tag and source selectors", () => {
  assert.deepEqual(linuxAppImagePostPublicSmokeInternals.parseSelectors(["v0.3.0", sourceSha]), {
    sourceSha,
    tag: "v0.3.0",
  });
  for (const argv of [
    [],
    ["v0.3.0"],
    ["v0.3.0", sourceSha, "--appimage"],
    ["--appimage", sourceSha],
    ["v0.3.0", sourceSha.toUpperCase()],
    ["v0.3.0", "a".repeat(39)],
  ]) {
    assert.throws(() => linuxAppImagePostPublicSmokeInternals.parseSelectors(argv));
  }
});

test("requires the independent pre-publication inventory to match workflow selectors", () => {
  const candidate = { tag: "v0.3.0", source_sha: sourceSha, assets: [] };
  assert.equal(
    linuxAppImagePostPublicSmokeInternals.validateCandidateSelectors(
      candidate,
      "v0.3.0",
      sourceSha,
    ),
    candidate,
  );
  assert.throws(
    () =>
      linuxAppImagePostPublicSmokeInternals.validateCandidateSelectors(
        candidate,
        "v0.3.1",
        sourceSha,
      ),
    /does not match/u,
  );
});

test("rejects fixture or cloned native results at the process-local brand", async () => {
  const receipt = await genuinePublicReceipt();
  for (const result of [{}, structuredClone({ disposition: "passed" })]) {
    assert.throws(
      () =>
        linuxPersistenceCaptureInternals.requireVerifiedPublicAppImageCaptureResult(
          result,
          receipt,
        ),
      /matching in-process verified native result/u,
    );
  }
});

test("keeps public selection, updater verification, execution, and output fixed in source", () => {
  const capture = fs.readFileSync(
    new URL("./capture-linux-current-user-persistence.mjs", import.meta.url),
    "utf8",
  );
  const smoke = fs.readFileSync(
    new URL("./linux-appimage-post-public-smoke.mjs", import.meta.url),
    "utf8",
  );
  assert.match(capture, /captureVerifiedPublicAppImage\(receipt\)/u);
  assert.match(capture, /signatureFor === role\.role/u);
  assert.match(capture, /batcave-verify-updater-signature/u);
  assert.match(capture, /fixedCommand\(UPDATER_VERIFIER/u);
  assert.match(capture, /expectedArtifact: \{ \.\.\.asset, app_version: verified\.app_version \}/u);
  assert.match(capture, /telemetryRequired: true/u);
  assert.match(capture, /appimage: true/u);
  assert.doesNotMatch(smoke, /--(?:appimage|output-dir|command|status|evidence|env)/u);
  assert.doesNotMatch(smoke, /process\.env/u);
  assert.match(smoke, /post-public-input\/release-candidate\.json/u);
  assert.match(smoke, /verifyPublicRelease\(candidate, release, downloads\)/u);
  assert.match(smoke, /captureVerifiedPublicAppImage/u);
  assert.match(smoke, /release_evidence_eligible: false/u);
  assert.match(smoke, /network_isolation_not_enforced/u);
  assert.match(smoke, /updater_a_to_b_not_exercised/u);
  assert.match(smoke, /post-public-output[\s\S]*linux-appimage-observation\.json/u);
  assert.match(smoke, /flag: "wx"[\s\S]*mode: 0o600/u);
});
