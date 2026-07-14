import assert from "node:assert/strict";
import crypto from "node:crypto";
import fs from "node:fs";
import fsp from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import { after, before, test } from "node:test";
import { fileURLToPath } from "node:url";

import {
  NativeArtifactCapabilityCleanupError,
  acquireNativeArtifactCapability,
  closeNativeArtifactCapability,
  requireOwnedNativeArtifactVerificationReceipt,
  verifyOwnedNativeArtifactCapability,
} from "./native-artifact-capability.mjs";
import { createInstallSmokePlan } from "./install-smoke-contract.mjs";
import {
  runNativeInstallSmokeSourceSlice,
  validateNativeInstallSmokeResult,
} from "./native-install-smoke-executor.mjs";
import { expectedReleaseAssetRoles } from "./release-asset-contract.mjs";
import { validateReleaseEvidencePacket } from "./validate-release-evidence-packet.mjs";
import {
  CHECKSUM_MANIFEST,
  RELEASE_REPOSITORY,
  verifyPublicRelease,
} from "./verify-public-release.mjs";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const FIXTURE = path.join(
  ROOT,
  "docs",
  "evidence",
  "releases",
  "fixtures",
  "v1",
  "linux-appimage.json",
);
const SOURCE_SHA = "0123456789abcdef0123456789abcdef01234567";
const TAG = "v9.9.9-rc.1";
const UPDATER_KEY = "sha256:0dad0009cf5cc87a778f2e951cefaa0faaba637b95a22f6f3064f12cd4136545";

let contractReceipt;
let payloads;
let publicRoot;
let scratchRoot;

function digest(contents) {
  return `sha256:${crypto.createHash("sha256").update(contents).digest("hex")}`;
}

function publicReleaseFixture() {
  const contract = expectedReleaseAssetRoles(TAG);
  const provenance = contract.roles.find(({ role }) => role === "build provenance bundle").name;
  const subjects = new Map(
    contract.roles
      .map(({ name }) => name)
      .filter((name) => name !== CHECKSUM_MANIFEST && name !== provenance)
      .map((name) => [name, `verified public native-capability bytes for ${name}\n`]),
  );
  const manifest = [...subjects]
    .map(([name, contents]) => `${digest(contents).slice("sha256:".length)}  ./${name}\n`)
    .join("");
  const fixturePayloads = new Map([
    ...subjects,
    [CHECKSUM_MANIFEST, manifest],
    [provenance, '{"bundle":"native-capability-fixture"}\n'],
  ]);
  const assets = [...fixturePayloads]
    .map(([name, contents]) => ({
      name,
      size: Buffer.byteLength(contents),
      digest: digest(contents),
    }))
    .sort((left, right) => left.name.localeCompare(right.name));
  return {
    candidate: { tag: TAG, source_sha: SOURCE_SHA, prerelease: true, assets },
    release: {
      tag_name: TAG,
      target_commitish: SOURCE_SHA,
      draft: false,
      prerelease: true,
      immutable: true,
      assets: assets.map((asset) => ({
        ...asset,
        browser_download_url: `https://github.com/${RELEASE_REPOSITORY}/releases/download/${TAG}/${encodeURIComponent(asset.name)}`,
      })),
    },
    payloads: fixturePayloads,
  };
}

function releaseTemplate() {
  const packet = JSON.parse(fs.readFileSync(FIXTURE, "utf8"));
  packet.packet_kind = "release_evidence";
  packet.packet_id = "linux-appimage-native-source-slice";
  packet.release.tag = TAG;
  packet.release.channel = "prerelease";
  packet.release.source_sha = SOURCE_SHA;
  packet.release.main_sha = SOURCE_SHA;
  packet.release.release_target_sha = SOURCE_SHA;
  packet.release.release_url = `https://github.com/${RELEASE_REPOSITORY}/releases/tag/${TAG}`;
  packet.release.workflow_run = {
    workflow_file: ".github/workflows/release.yml",
    run_id: 123456789,
    run_attempt: 1,
    url: `https://github.com/${RELEASE_REPOSITORY}/actions/runs/123456789/attempts/1`,
  };
  delete packet.limitations.synthetic_fixture_no_release_claim;
  const assetName = expectedReleaseAssetRoles(TAG).roles.find(
    ({ role }) => role === "Linux AppImage package and updater payload",
  ).name;
  const verified = contractReceipt.assets.find(({ name }) => name === assetName);
  packet.platform.package.asset_name = assetName;
  packet.assets[0] = {
    name: assetName,
    size_bytes: verified.size_bytes,
    sha256: verified.sha256,
    api_digest: verified.sha256,
    public_url: verified.public_url,
    attestation: {
      verified: true,
      repository: RELEASE_REPOSITORY,
      source_sha: SOURCE_SHA,
      source_ref: "refs/heads/main",
      signer_workflow: "TheGreenCedar/BatCave/.github/workflows/release.yml",
    },
    signatures: { tauri_updater: { identity: UPDATER_KEY, verified: true } },
  };
  for (const checks of Object.values(packet.checks)) {
    for (const check of Object.values(checks)) {
      check.status = "blocked";
      check.outcome = "Awaiting the reviewed native adapter.";
    }
  }
  validateReleaseEvidencePacket(packet);
  return packet;
}

function planInput() {
  const packet = releaseTemplate();
  return {
    schema_version: 1,
    execution_kind: "plan",
    app_version: "9.9.9-rc.1",
    evidence_template: packet,
    public_verification: contractReceipt,
    isolation: {
      scope_id: "linux-appimage-native-source-slice",
      install_root_id: "isolated-install-root",
      user_state_root_id: "isolated-user-state-root",
      user_state_policy: "preserve",
      step_timeout_ms: 1_000,
      termination_timeout_ms: 100,
      settings_probe: { theme: "cave", sample_interval_ms: 1_500 },
      degradation_scenario: "permission-limited-telemetry",
    },
  };
}

function fixturePlan() {
  const packet = JSON.parse(fs.readFileSync(FIXTURE, "utf8"));
  const asset = packet.assets[0];
  return createInstallSmokePlan({
    schema_version: 1,
    execution_kind: "fixture",
    app_version: "0.0.0-evidence.1",
    evidence_template: packet,
    public_verification: {
      schema_version: 1,
      verifier: "scripts/verify-public-release.mjs",
      disposition: "fixture",
      proof_scope: "fixture_only",
      repository: RELEASE_REPOSITORY,
      tag: packet.release.tag,
      source_sha: packet.release.source_sha,
      app_version: "0.0.0-evidence.1",
      assets: [
        {
          name: asset.name,
          size_bytes: asset.size_bytes,
          sha256: asset.sha256,
          public_url: asset.public_url,
        },
      ],
    },
    isolation: {
      scope_id: "linux-appimage-fixture",
      install_root_id: "isolated-install-root",
      user_state_root_id: "isolated-user-state-root",
      user_state_policy: "preserve",
      step_timeout_ms: 1_000,
      termination_timeout_ms: 100,
      settings_probe: { theme: "cave", sample_interval_ms: 1_500 },
      degradation_scenario: "permission-limited-telemetry",
    },
  });
}

async function verifiedRootFor(plan) {
  const root = await fsp.mkdtemp(path.join(scratchRoot, "verified-"));
  await fsp.writeFile(path.join(root, plan.asset.name), payloads.get(plan.asset.name), {
    mode: 0o600,
  });
  return root;
}

before(async () => {
  scratchRoot = await fsp.mkdtemp(path.join(os.tmpdir(), "batcave-native-executor-test-"));
  publicRoot = path.join(scratchRoot, "public");
  const fixture = publicReleaseFixture();
  payloads = fixture.payloads;
  const result = await verifyPublicRelease(fixture.candidate, fixture.release, publicRoot, {
    fetchImpl: async (url) => {
      const name = decodeURIComponent(new URL(url).pathname.split("/").at(-1));
      return payloads.has(name)
        ? new Response(payloads.get(name), { status: 200 })
        : new Response("not found", { status: 404 });
    },
    ghRunner: () => {},
  });
  contractReceipt = result.receipt;
});

after(async () => {
  await fsp.rm(scratchRoot, { recursive: true, force: true });
});

test("capability owns verified bytes after the public path is replaced", async () => {
  const plan = createInstallSmokePlan(planInput());
  const root = await verifiedRootFor(plan);
  const source = path.join(root, plan.asset.name);
  const capability = await acquireNativeArtifactCapability(plan, { verified_root: root });
  await fsp.rename(source, `${source}.replaced`);
  await fsp.writeFile(source, "hostile replacement bytes", { mode: 0o600 });

  const receipt = await verifyOwnedNativeArtifactCapability(capability);
  assert.equal(receipt.proof_scope, "artifact_owned_bytes_verified");
  assert.equal(receipt.asset.sha256, digest(payloads.get(plan.asset.name)));
  assert.equal(requireOwnedNativeArtifactVerificationReceipt(receipt, plan), receipt);
  assert.deepEqual(Object.keys(capability).sort(), [
    "asset",
    "plan_id",
    "proof_scope",
    "schema_version",
  ]);
  assert.doesNotMatch(JSON.stringify(capability), /(?:\/private\/|[A-Za-z]:\\)/u);
  await closeNativeArtifactCapability(capability);
});

test("rejects source links and verified-root links before capability creation", async (t) => {
  const plan = createInstallSmokePlan(planInput());
  const outside = await fsp.mkdtemp(path.join(scratchRoot, "outside-"));
  await fsp.writeFile(path.join(outside, plan.asset.name), payloads.get(plan.asset.name));
  const root = await fsp.mkdtemp(path.join(scratchRoot, "links-"));
  try {
    await fsp.symlink(
      path.join(outside, plan.asset.name),
      path.join(root, plan.asset.name),
      "file",
    );
    await assert.rejects(
      () => acquireNativeArtifactCapability(plan, { verified_root: root }),
      /regular non-link file|too many levels|symbolic link/u,
    );
  } catch (error) {
    if (error.code === "EPERM") t.diagnostic("file symlink creation is unavailable on this host");
    else throw error;
  }

  const rootLink = path.join(scratchRoot, "root-link");
  try {
    await fsp.symlink(outside, rootLink, process.platform === "win32" ? "junction" : "dir");
    await assert.rejects(
      () => acquireNativeArtifactCapability(plan, { verified_root: rootLink }),
      /non-link directory/u,
    );
  } catch (error) {
    if (error.code === "EPERM") t.diagnostic("directory link creation is unavailable on this host");
    else throw error;
  }
});

test("rejects verified-root replacement between inspection and resolution", async () => {
  const plan = createInstallSmokePlan(planInput());
  const root = await verifiedRootFor(plan);
  const replacement = await verifiedRootFor(plan);
  const originalRealpath = fsp.realpath;
  let swapped = false;
  fsp.realpath = async (target) => {
    if (target === root && !swapped) {
      swapped = true;
      await fsp.rename(root, `${root}.inspected`);
      await fsp.rename(replacement, root);
    }
    return originalRealpath(target);
  };
  try {
    await assert.rejects(
      () => acquireNativeArtifactCapability(plan, { verified_root: root }),
      /verified_root.*identity changed while being resolved/u,
    );
  } finally {
    fsp.realpath = originalRealpath;
  }
  assert.equal(swapped, true);
});

test("rejects source substitution between inspection and open", async () => {
  const plan = createInstallSmokePlan(planInput());
  const root = await verifiedRootFor(plan);
  const source = path.join(root, plan.asset.name);
  const sourceReal = await fsp.realpath(source);
  const replacement = `${source}.replacement`;
  await fsp.writeFile(replacement, payloads.get(plan.asset.name), { mode: 0o600 });
  const originalOpen = fsp.open;
  let swapped = false;
  fsp.open = async (...arguments_) => {
    if (arguments_[0] === sourceReal && !swapped) {
      swapped = true;
      await fsp.rename(source, `${source}.inspected`);
      await fsp.rename(replacement, source);
    }
    return originalOpen(...arguments_);
  };
  try {
    await assert.rejects(
      () => acquireNativeArtifactCapability(plan, { verified_root: root }),
      /was replaced between inspection and open/u,
    );
  } finally {
    fsp.open = originalOpen;
  }
  assert.equal(swapped, true);
});

test("rejects source-path replacement during capability copying", async () => {
  const plan = createInstallSmokePlan(planInput());
  const root = await verifiedRootFor(plan);
  const source = path.join(root, plan.asset.name);
  const sourceReal = await fsp.realpath(source);
  const replacement = `${source}.replacement`;
  await fsp.writeFile(replacement, payloads.get(plan.asset.name), { mode: 0o600 });
  const originalOpen = fsp.open;
  let swapped = false;
  fsp.open = async (...arguments_) => {
    const handle = await originalOpen(...arguments_);
    if (arguments_[0] !== sourceReal) return handle;
    const originalRead = handle.read.bind(handle);
    handle.read = async (...readArguments) => {
      const result = await originalRead(...readArguments);
      if (!swapped) {
        swapped = true;
        await fsp.rename(source, `${source}.opened`);
        await fsp.rename(replacement, source);
      }
      return result;
    };
    return handle;
  };
  try {
    await assert.rejects(
      () => acquireNativeArtifactCapability(plan, { verified_root: root }),
      /was replaced while the capability acquired it/u,
    );
  } finally {
    fsp.open = originalOpen;
  }
  assert.equal(swapped, true);
});

test("rejects mismatched and substituted selected artifacts", async () => {
  const plan = createInstallSmokePlan(planInput());
  const root = await verifiedRootFor(plan);
  const source = path.join(root, plan.asset.name);
  await fsp.writeFile(source, "same path, wrong bytes", { mode: 0o600 });
  await assert.rejects(
    () => acquireNativeArtifactCapability(plan, { verified_root: root }),
    /size does not match|digest does not match/u,
  );

  await fsp.writeFile(source, payloads.get(plan.asset.name), { mode: 0o600 });
  const temporary = `${source}.next`;
  await fsp.writeFile(temporary, payloads.get(plan.asset.name), { mode: 0o600 });
  await fsp.rename(temporary, source);
  const capability = await acquireNativeArtifactCapability(plan, { verified_root: root });
  const receipt = await verifyOwnedNativeArtifactCapability(capability);
  assert.equal(requireOwnedNativeArtifactVerificationReceipt(receipt, plan), receipt);
  await closeNativeArtifactCapability(capability);
});

test("caller readers, callbacks, adapters, and options cannot reach owned bytes", async () => {
  const plan = createInstallSmokePlan(planInput());
  const root = await verifiedRootFor(plan);
  const capability = await acquireNativeArtifactCapability(plan, { verified_root: root });
  let called = false;
  const forbiddenArguments = [
    async () => {
      called = true;
    },
    { read: "partial", offset: 1 },
    { concurrent: true },
    { adapter: "injected" },
  ];
  for (const argument of forbiddenArguments) {
    await assert.rejects(
      () => verifyOwnedNativeArtifactCapability(capability, argument),
      /does not accept a caller-supplied reader, callback, adapter, or options/u,
    );
  }
  assert.equal(called, false);
  const receipt = await verifyOwnedNativeArtifactCapability(capability);
  assert.equal(requireOwnedNativeArtifactVerificationReceipt(receipt, plan), receipt);
  await closeNativeArtifactCapability(capability);
});

test("concurrent and repeated owned-byte verification fails closed", async () => {
  const plan = createInstallSmokePlan(planInput());
  const root = await verifiedRootFor(plan);
  const capability = await acquireNativeArtifactCapability(plan, { verified_root: root });
  const firstVerification = verifyOwnedNativeArtifactCapability(capability);
  await assert.rejects(
    () => verifyOwnedNativeArtifactCapability(capability),
    /already being verified|already been verified/u,
  );
  const receipt = await firstVerification;
  assert.equal(requireOwnedNativeArtifactVerificationReceipt(receipt, plan), receipt);
  await assert.rejects(
    () => verifyOwnedNativeArtifactCapability(capability),
    /already been verified/u,
  );
  await closeNativeArtifactCapability(capability);
});

test("fixture, fake capability, and forged verification receipt cannot reach native bytes", async () => {
  const plan = fixturePlan();
  await assert.rejects(
    () => acquireNativeArtifactCapability(plan, { verified_root: scratchRoot }),
    /fixtures cannot acquire native bytes/u,
  );
  await assert.rejects(
    () => verifyOwnedNativeArtifactCapability({ ...plan.identity_receipt }),
    /process-local capability/u,
  );
  assert.throws(
    () =>
      requireOwnedNativeArtifactVerificationReceipt(
        {
          schema_version: 1,
          proof_scope: "artifact_owned_bytes_verified",
          plan_id: plan.plan_id,
          verification_sequence: 1,
          asset: plan.asset,
        },
        plan,
      ),
    /process-local owned-byte verification/u,
  );
});

test("source slice distinguishes skipped and failed without release evidence", async () => {
  const plan = createInstallSmokePlan(planInput());
  const root = await verifiedRootFor(plan);
  const skipped = await runNativeInstallSmokeSourceSlice(plan, { verified_root: root });
  assert.equal(skipped.disposition, "skipped");
  assert.equal(skipped.evidence_packet, null);
  assert.equal(skipped.artifact_verification_receipt.proof_scope, "artifact_owned_bytes_verified");
  assert.equal(
    skipped.steps.find(({ id }) => id === "preflight.package_trust").status,
    "unsupported",
  );
  assert.equal(validateNativeInstallSmokeResult(skipped), skipped);

  await fsp.writeFile(path.join(root, plan.asset.name), "wrong bytes", { mode: 0o600 });
  const failed = await runNativeInstallSmokeSourceSlice(plan, { verified_root: root });
  assert.equal(failed.disposition, "failed");
  assert.equal(failed.evidence_packet, null);
  assert.equal(failed.artifact_verification_receipt, null);
  assert.equal(failed.steps.find(({ id }) => id === "preflight.asset_rehash").status, "failed");
  assert.equal(validateNativeInstallSmokeResult(failed), failed);
  assert.doesNotMatch(JSON.stringify(failed), /(?:\/private\/|[A-Za-z]:\\)/u);
});

test("owned capability cleanup failure is represented as a failed gate", async () => {
  const plan = createInstallSmokePlan(planInput());
  const root = await verifiedRootFor(plan);
  const before = new Set(
    (await fsp.readdir(os.tmpdir())).filter((name) => name.startsWith("batcave-native-artifact-")),
  );
  const originalRm = fsp.rm;
  fsp.rm = async (target, options) => {
    if (path.basename(String(target)).startsWith("batcave-native-artifact-")) {
      throw new Error("simulated owned capability cleanup failure");
    }
    return originalRm(target, options);
  };
  let result;
  try {
    result = await runNativeInstallSmokeSourceSlice(plan, { verified_root: root });
  } finally {
    fsp.rm = originalRm;
    const after = (await fsp.readdir(os.tmpdir())).filter(
      (name) => name.startsWith("batcave-native-artifact-") && !before.has(name),
    );
    await Promise.all(
      after.map((name) =>
        originalRm(path.join(os.tmpdir(), name), { recursive: true, force: true }),
      ),
    );
  }
  assert.equal(result.disposition, "failed");
  assert.equal(result.evidence_packet, null);
  assert.equal(
    result.steps.find(({ id }) => id === "cleanup.owned_runtime_cleanup").status,
    "failed",
  );
  assert.match(
    result.steps.find(({ id }) => id === "cleanup.owned_runtime_cleanup").outcome,
    /cleanup failed closed/u,
  );
});

test("acquisition preserves cleanup errors and reports both failed boundaries", async () => {
  const plan = createInstallSmokePlan(planInput());
  const root = await verifiedRootFor(plan);
  await fsp.writeFile(path.join(root, plan.asset.name), "wrong bytes", { mode: 0o600 });
  const before = new Set(
    (await fsp.readdir(os.tmpdir())).filter((name) => name.startsWith("batcave-native-artifact-")),
  );
  const originalRm = fsp.rm;
  fsp.rm = async (target, options) => {
    if (path.basename(String(target)).startsWith("batcave-native-artifact-")) {
      throw new Error("simulated acquisition cleanup failure");
    }
    return originalRm(target, options);
  };
  let result;
  let directError;
  try {
    try {
      await acquireNativeArtifactCapability(plan, { verified_root: root });
    } catch (error) {
      directError = error;
    }
    result = await runNativeInstallSmokeSourceSlice(plan, { verified_root: root });
  } finally {
    fsp.rm = originalRm;
    const after = (await fsp.readdir(os.tmpdir())).filter(
      (name) => name.startsWith("batcave-native-artifact-") && !before.has(name),
    );
    await Promise.all(
      after.map((name) =>
        originalRm(path.join(os.tmpdir(), name), { recursive: true, force: true }),
      ),
    );
  }
  assert.ok(directError instanceof NativeArtifactCapabilityCleanupError);
  assert.equal(directError.errors.length, 2);
  assert.equal(result.disposition, "failed");
  assert.equal(result.evidence_packet, null);
  assert.equal(result.steps.find(({ id }) => id === "preflight.asset_rehash").status, "failed");
  assert.equal(
    result.steps.find(({ id }) => id === "cleanup.owned_runtime_cleanup").status,
    "failed",
  );
});

test("native disposition and executor seams cannot be caller-authored", async () => {
  const plan = createInstallSmokePlan(planInput());
  const root = await verifiedRootFor(plan);
  await assert.rejects(
    () =>
      runNativeInstallSmokeSourceSlice(plan, {
        verified_root: root,
        adapter: { native_proven: true, actions: {} },
      }),
    /native_executor.options.adapter.*not allowed/u,
  );

  const skipped = await runNativeInstallSmokeSourceSlice(plan, { verified_root: root });
  const forged = {
    ...skipped,
    disposition: "native_proven",
    steps: skipped.steps.map((step) => ({ ...step, status: "passed" })),
    native_execution_receipt: { native_proven: true },
  };
  assert.throws(
    () => validateNativeInstallSmokeResult(forged),
    /process-local closed-adapter execution receipt/u,
  );
});

test("hosted release-contract jobs include the native executor hostile suite", () => {
  const releaseWorkflow = fs.readFileSync(
    path.join(ROOT, ".github", "workflows", "release.yml"),
    "utf8",
  );
  const validationWorkflow = fs.readFileSync(
    path.join(ROOT, ".github", "workflows", "validation.yml"),
    "utf8",
  );
  assert.equal(
    releaseWorkflow.match(/scripts\/native-install-smoke-executor\.test\.mjs/gu)?.length,
    1,
  );
  assert.equal(
    validationWorkflow.match(/scripts\/native-install-smoke-executor\.test\.mjs/gu)?.length,
    3,
  );
});
