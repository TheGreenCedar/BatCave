import assert from "node:assert/strict";
import crypto from "node:crypto";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import test from "node:test";

import { linuxPersistenceCaptureInternals } from "./capture-linux-current-user-persistence.mjs";
import { linuxDebPostPublicSmokeInternals } from "./linux-deb-post-public-smoke.mjs";
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
  const subjects = roles.map(({ name }) => name).filter((name) => ![CHECKSUM_MANIFEST, provenance].includes(name));
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
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "batcave-deb-brand-test-"));
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
  assert.deepEqual(linuxDebPostPublicSmokeInternals.parseSelectors(["v0.3.0", sourceSha]), {
    sourceSha,
    tag: "v0.3.0",
  });
  for (const argv of [
    [],
    ["v0.3.0"],
    ["v0.3.0", sourceSha, "--deb"],
    ["--deb", sourceSha],
    ["v0.3.0", sourceSha.toUpperCase()],
    ["v0.3.0", "a".repeat(39)],
  ]) {
    assert.throws(() => linuxDebPostPublicSmokeInternals.parseSelectors(argv));
  }
});

test("requires the independent pre-publication inventory to match both workflow selectors", () => {
  const candidate = {
    tag: "v0.3.0",
    source_sha: sourceSha,
    prerelease: false,
    assets: [
      {
        name: "BatCave.Monitor_0.3.0_amd64.deb",
        size: 42,
        digest: `sha256:${"a".repeat(64)}`,
      },
    ],
  };
  assert.equal(
    linuxDebPostPublicSmokeInternals.validateCandidateSelectors(candidate, "v0.3.0", sourceSha),
    candidate,
  );
  assert.throws(
    () => linuxDebPostPublicSmokeInternals.validateCandidateSelectors(candidate, "v0.3.1", sourceSha),
    /does not match/u,
  );
  assert.throws(
    () =>
      linuxDebPostPublicSmokeInternals.validateCandidateSelectors(
        candidate,
        "v0.3.0",
        "f".repeat(40),
      ),
    /does not match/u,
  );
});

test("rejects fixture or cloned native results at the native process-local brand", async () => {
  const receipt = await genuinePublicReceipt();
  for (const result of [{}, structuredClone({ disposition: "passed" })]) {
    assert.throws(
      () => linuxPersistenceCaptureInternals.requireVerifiedPublicDebCaptureResult(result, receipt),
      /matching in-process verified native result/u,
    );
  }
});

test("keeps privileged commands, artifact selection, cleanup, and evidence fixed in source", () => {
  const capture = fs.readFileSync(
    new URL("./capture-linux-current-user-persistence.mjs", import.meta.url),
    "utf8",
  );
  const smoke = fs.readFileSync(new URL("./linux-deb-post-public-smoke.mjs", import.meta.url), "utf8");

  assert.match(capture, /operation === "install"\) return \["\/usr\/bin\/dpkg", "--install", value\]/u);
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
    assert.ok(capture.includes(`"${runtimePackage}"`), `runtime prerequisite must fix ${runtimePackage}`);
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
  assert.doesNotMatch(smoke, /--(?:deb|output-dir|command|status|evidence|env)/u);
  assert.doesNotMatch(smoke, /process\.env/u);
  assert.doesNotMatch(smoke, /candidateFromPublicRelease/u);
  assert.match(smoke, /post-public-input\/release-candidate\.json/u);
  assert.match(smoke, /verifyPublicRelease\(candidate, release, downloads\)/u);
  assert.doesNotMatch(smoke, /buildEvidence[,\s]*\n/u);
  assert.doesNotMatch(smoke, /root_descendant_settlement/u);
  assert.match(smoke, /root_process_settlement/u);
  assert.match(smoke, /finally \{[\s\S]*fs\.rmSync\(workspace/u);
  assert.match(smoke, /post-public-output[\s\S]*linux-deb-observation\.json/u);
  assert.match(smoke, /flag: "wx"[\s\S]*mode: 0o600/u);
  assert.match(smoke, /console\.log\(JSON\.stringify\(evidence\)\)/u);
});
