import assert from "node:assert/strict";
import crypto from "node:crypto";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import test from "node:test";

import { linuxPersistenceCaptureInternals } from "./capture-linux-current-user-persistence.mjs";
import { linuxAppImagePostPublicSmokeInternals } from "./linux-appimage-post-public-smoke.mjs";
import { linuxDebPostPublicSmokeInternals } from "./linux-deb-post-public-smoke.mjs";
import { expectedReleaseAssetRoles } from "./release-asset-contract.mjs";
import {
  CHECKSUM_MANIFEST,
  RELEASE_REPOSITORY,
  verifyPublicRelease,
} from "./verify-public-release.mjs";

const sourceSha = "0123456789abcdef0123456789abcdef01234567";

const profiles = [
  {
    forbiddenSelector: "--deb",
    internals: linuxDebPostPublicSmokeInternals,
    name: "deb",
  },
  {
    forbiddenSelector: "--appimage",
    internals: linuxAppImagePostPublicSmokeInternals,
    name: "AppImage",
  },
];

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
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "batcave-linux-post-public-brand-test-"));
  try {
    const result = await verifyPublicRelease(candidate, release, path.join(root, "downloads"), {
      fetchImpl: async (url) => {
        const name = decodeURIComponent(new URL(url).pathname.split("/").at(-1));
        return new Response(payloads.get(name), { status: 200 });
      },
      ghRunner: () => {},
    });
    return result.receipt;
  } finally {
    fs.rmSync(root, { force: true, recursive: true });
  }
}

for (const { forbiddenSelector, internals, name } of profiles) {
  test(`${name} accepts only exact workflow-owned tag and source selectors`, () => {
    assert.deepEqual(internals.parseSelectors(["v0.3.0", sourceSha]), {
      sourceSha,
      tag: "v0.3.0",
    });
    for (const argv of [
      [],
      ["v0.3.0"],
      ["v0.3.0", sourceSha, forbiddenSelector],
      [forbiddenSelector, sourceSha],
      ["v0.3.0", sourceSha.toUpperCase()],
      ["v0.3.0", "a".repeat(39)],
    ]) {
      assert.throws(() => internals.parseSelectors(argv));
    }
  });
}

test("both profiles require the candidate inventory to match both workflow selectors", () => {
  const candidate = {
    tag: "v0.3.0",
    source_sha: sourceSha,
    assets: [
      {
        name: "BatCave.Monitor_0.3.0_amd64.deb",
        size: 42,
        digest: `sha256:${"a".repeat(64)}`,
      },
    ],
  };
  for (const { internals } of profiles) {
    assert.equal(internals.validateCandidateSelectors(candidate, "v0.3.0", sourceSha), candidate);
    assert.throws(
      () => internals.validateCandidateSelectors(candidate, "v0.3.1", sourceSha),
      /does not match/u,
    );
    assert.throws(
      () => internals.validateCandidateSelectors(candidate, "v0.3.0", "f".repeat(40)),
      /does not match/u,
    );
  }
});

test("both profiles reject fixture or cloned results at their process-local brands", async () => {
  const receipt = await genuinePublicReceipt();
  for (const requireResult of [
    linuxPersistenceCaptureInternals.requireVerifiedPublicDebCaptureResult,
    linuxPersistenceCaptureInternals.requireVerifiedPublicAppImageCaptureResult,
  ]) {
    for (const result of [{}, structuredClone({ disposition: "passed" })]) {
      assert.throws(
        () => requireResult(result, receipt),
        /matching in-process verified native result/u,
      );
    }
  }
});

test("keeps both native profiles, public selection, cleanup, and evidence fixed in source", () => {
  const capture = fs.readFileSync(
    new URL("./capture-linux-current-user-persistence.mjs", import.meta.url),
    "utf8",
  );
  const shared = fs.readFileSync(new URL("./linux-post-public-smoke.mjs", import.meta.url), "utf8");
  const debEntry = fs.readFileSync(
    new URL("./linux-deb-post-public-smoke.mjs", import.meta.url),
    "utf8",
  );
  const appImageEntry = fs.readFileSync(
    new URL("./linux-appimage-post-public-smoke.mjs", import.meta.url),
    "utf8",
  );

  assert.match(
    capture,
    /operation === "install"\) return \["\/usr\/bin\/dpkg", "--install", value\]/u,
  );
  assert.match(capture, /runFixedRootUnit\("install", deb\)/u);
  assert.match(capture, /runFixedRootUnit\("purge"\)/u);
  assert.match(capture, /runFixedRootUnit\("apt-update"\)/u);
  assert.match(capture, /runFixedRootUnit\("apt-install"\)/u);
  for (const runtimePackage of [
    "libgtk-3-0",
    "libwebkit2gtk-4.1-0",
    "libayatana-appindicator3-1",
    "librsvg2-2",
    "libxdo3",
  ]) {
    assert.ok(
      capture.includes(`"${runtimePackage}"`),
      `runtime prerequisite must fix ${runtimePackage}`,
    );
  }
  for (const property of [
    "KillMode=control-group",
    "SendSIGKILL=yes",
    "TimeoutStopSec=10s",
    "RuntimeMaxSec=120s",
    "TasksMax=256",
    "ProtectControlGroups=yes",
    "Delegate=no",
  ]) {
    assert.ok(capture.includes(property), `root unit must set ${property}`);
  }
  assert.match(capture, /runRootSettlementHostileProof\(\)/u);
  assert.match(capture, /requireRootUnitSettlementReceipt\(receipt\)\.process_tree_settled/u);
  assert.match(capture, /process\.on\(signal, handler\)/u);
  assert.doesNotMatch(capture, /process\.once\(signal, handler\)/u);
  assert.match(capture, /interrupted \?\?= signal;\s*resolveSignal\(signal\)/u);
  assert.match(
    capture,
    /finally \{\s*try \{\s*settlement = await settleRootUnit\(unit\)[\s\S]*finally \{\s*for \(const \[signal, handler\] of handlers\) process\.removeListener/u,
  );
  assert.match(capture, /requireVerifiedPublicReleaseReceipt\(receipt\)/u);
  assert.match(capture, /expectedReleaseAssetRoles\(verified\.tag\)/u);
  assert.match(capture, /copyOwnedArtifact\(source, artifact, "deb artifact", expectedArtifact\)/u);
  assert.match(
    capture,
    /finally \{[\s\S]*await purgeDeb\(packageName, ownedFiles\)[\s\S]*fs\.rmSync\(workspace/u,
  );
  assert.match(capture, /captureVerifiedPublicAppImage\(receipt\)/u);
  assert.match(capture, /signatureFor === role\.role/u);
  assert.match(capture, /batcave-verify-updater-signature/u);
  assert.match(capture, /fixedCommand\(UPDATER_VERIFIER/u);
  assert.match(capture, /expectedArtifact: \{ \.\.\.asset, app_version: verified\.app_version \}/u);
  assert.match(capture, /telemetryRequired: true/u);
  assert.match(capture, /appimage: true/u);

  assert.match(debEntry, /runLinuxDebPostPublicSmoke\(process\.argv\.slice\(2\)\)/u);
  assert.match(appImageEntry, /runLinuxAppImagePostPublicSmoke\(process\.argv\.slice\(2\)\)/u);
  for (const source of [shared, debEntry, appImageEntry]) {
    assert.doesNotMatch(source, /--(?:deb|appimage|output-dir|command|status|evidence|env)/u);
    assert.doesNotMatch(source, /process\.env/u);
  }
  assert.doesNotMatch(shared, /candidateFromPublicRelease/u);
  assert.match(shared, /post-public-input\/release-candidate\.json/u);
  assert.match(shared, /verifyPublicRelease\(candidate, release, downloads\)/u);
  assert.match(shared, /captureVerifiedPublicDeb/u);
  assert.match(shared, /captureVerifiedPublicAppImage/u);
  assert.equal(shared.match(/release_evidence_eligible: false/gu)?.length, 2);
  for (const profileContract of [
    'result_kind: "linux_deb_post_public_observation"',
    'proof_scope: "post_public_deb_smoke_observation_only"',
    "package_owned_files_removed:",
    "root_process_settlement:",
    'result_kind: "linux_appimage_post_public_observation"',
    'proof_scope: "post_public_appimage_smoke_observation_only"',
    "updater_signature_name:",
    "updater_signature_sha256:",
    "updater_key_fingerprint:",
    "appimage_removed:",
    'invocation_process_groups_settled: "passed"',
  ]) {
    assert.ok(shared.includes(profileContract), `shared driver must retain ${profileContract}`);
  }
  assert.doesNotMatch(shared, /export function runLinuxPostPublicSmoke/u);
  assert.match(shared, /export const runLinuxDebPostPublicSmoke/u);
  assert.match(shared, /export const runLinuxAppImagePostPublicSmoke/u);
  assert.doesNotMatch(shared, /root_descendant_settlement/u);
  assert.match(shared, /root_process_settlement/u);
  assert.match(shared, /network_isolation_not_enforced/u);
  assert.match(shared, /updater_a_to_b_not_exercised/u);
  assert.match(shared, /finally \{[\s\S]*fs\.rmSync\(workspace/u);
  assert.match(shared, /outputName: "linux-deb-observation\.json"/u);
  assert.match(shared, /outputName: "linux-appimage-observation\.json"/u);
  assert.match(shared, /flag: "wx"[\s\S]*mode: 0o600/u);
  assert.match(shared, /console\.log\(JSON\.stringify\(evidence\)\)/u);
});
