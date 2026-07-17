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
import {
  createInstallSmokePlan,
  validateInstallSmokePlan,
} from "./install-smoke-contract.mjs";
import {
  runNativeInstallSmokeSourceSlice,
  validateNativeInstallSmokeResult,
} from "./native-install-smoke-executor.mjs";
import {
  bindMacosNativeAdapterSource,
  requireMacosNativeAdapterSourceReceipt,
} from "./macos-native-install-smoke-adapter.mjs";
import { expectedReleaseAssetRoles } from "./release-asset-contract.mjs";
import { validateReleaseEvidenceTemplatePacket } from "./validate-release-evidence-packet.mjs";
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
const DEVELOPER_ID = "Developer ID Application: BatCave Monitor (ABCDEFGHIJ)";
const NOTARIZATION_ID = "submission-id:12345678-1234-4234-8234-123456789abc";
const STAPLE_ID = `ticket-sha256:${"a".repeat(64)}`;

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
  packet.platform.os_version = "ubuntu-22.04";
  packet.platform.proof.source = "source_enforced";
  packet.platform.proof.native = "pending";
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
  validateReleaseEvidenceTemplatePacket(packet);
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

function macosReleaseTemplate(packageKind) {
  const packet = releaseTemplate();
  const role =
    packageKind === "dmg" ? "macOS universal DMG" : "macOS universal updater payload";
  const assetName = expectedReleaseAssetRoles(TAG).roles.find(
    ({ role: candidateRole }) => candidateRole === role,
  ).name;
  const verified = contractReceipt.assets.find(({ name }) => name === assetName);
  packet.packet_id = `macos-${packageKind}-native-source-slice`;
  packet.platform = {
    support_contract_version: 1,
    profile_id: "macos-12-universal",
    proof: {
      declaration: "declared",
      source: "source_enforced",
      native: "pending",
    },
    os: "macos",
    os_version: "macos-15.0",
    architecture: "arm64",
    runtime: { libc_family: "not_applicable" },
    package: { kind: packageKind, architecture: "universal", asset_name: assetName },
  };
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
    signatures:
      packageKind === "dmg"
        ? {
            apple_notarization: { identity: NOTARIZATION_ID, verified: true },
            apple_staple: { identity: STAPLE_ID, verified: true },
            contained_app_developer_id: { identity: DEVELOPER_ID, verified: true },
            contained_app_notarization: { identity: NOTARIZATION_ID, verified: true },
            contained_app_staple: { identity: STAPLE_ID, verified: true },
            developer_id: { identity: DEVELOPER_ID, verified: true },
          }
        : {
            contained_app_developer_id: { identity: DEVELOPER_ID, verified: true },
            contained_app_notarization: { identity: NOTARIZATION_ID, verified: true },
            contained_app_staple: { identity: STAPLE_ID, verified: true },
            tauri_updater: { identity: UPDATER_KEY, verified: true },
          },
  };
  packet.limitations =
    packageKind === "macos_updater"
      ? {
          macos_updater_staging_only: {
            disposition: "accepted",
            summary: "Archive extraction is staging only and does not prove A-to-B installation.",
          },
        }
      : {};
  validateReleaseEvidenceTemplatePacket(packet);
  return packet;
}

function macosPlanInput(packageKind) {
  const input = planInput();
  input.evidence_template = macosReleaseTemplate(packageKind);
  input.isolation.scope_id = `macos-${packageKind}-native-source-slice`;
  return input;
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

test("closed macOS source binding is verified-identity-bound and cannot mint proof", async () => {
  for (const packageKind of ["dmg", "macos_updater"]) {
    const plan = createInstallSmokePlan(macosPlanInput(packageKind));
    const root = await verifiedRootFor(plan);
    const capability = await acquireNativeArtifactCapability(plan, { verified_root: root });
    const artifactReceipt = await verifyOwnedNativeArtifactCapability(capability);
    const sourceReceipt = bindMacosNativeAdapterSource(plan, artifactReceipt);

    assert.equal(
      requireMacosNativeAdapterSourceReceipt(sourceReceipt, plan, artifactReceipt),
      sourceReceipt,
    );
    assert.equal(sourceReceipt.profile.package_operation, packageKind === "dmg" ? "install" : "stage");
    assert.deepEqual(
      sourceReceipt.profile.required_limitations,
      packageKind === "dmg" ? [] : ["macos_updater_staging_only"],
    );
    assert.equal(
      sourceReceipt.profile.artifact_flow,
      packageKind === "dmg"
        ? "owned_dmg_mount_copy_required"
        : "rust_owned_updater_archive_stream_required",
    );
    assert.equal(
      sourceReceipt.profile.source_descriptor_tool_ids.includes(
        "rust_owned_stream_extractor",
      ),
      packageKind === "macos_updater",
    );
    assert.equal(
      sourceReceipt.profile.source_descriptor_tool_ids.includes(
        "python_archive_extractor",
      ),
      false,
    );
    assert.deepEqual(sourceReceipt.profile.destination_revalidation, {
      boundary: "consumed_destination_only",
      bundle_id_source: "compiled_tauri_identifier",
      version_source: "verified_release_version",
      required_architectures: ["arm64", "x86_64"],
      required_gate_ids: [
        "bundle_id",
        "version",
        "architectures",
        "signature_integrity",
        "developer_id_authority",
        "notarization",
        "staple",
      ],
      release_evidence_signature_role_ids: [
        "contained_app_developer_id",
        "contained_app_notarization",
        "contained_app_staple",
      ],
      source_fixture_can_prove_destination_binding: false,
      source_fixture_can_mint_proof: false,
    });
    assert.deepEqual(sourceReceipt.claims, {
      verified_asset_identity_bound: true,
      live_capability_held: false,
      descriptor_only: true,
      package_consumed: false,
      process_executed: false,
      trust_verified: false,
      runtime_executed: false,
      cleanup_proven: false,
      native_proven: false,
      release_evidence_emitted: false,
    });
    assert.doesNotMatch(
      JSON.stringify(sourceReceipt),
      /(?:verified_root|selected_path|command|status|trust_identity|evidence_packet|\/private\/|[A-Za-z]:\\)/u,
    );
    await assert.rejects(
      async () => bindMacosNativeAdapterSource(plan, artifactReceipt, { command: "injected" }),
      /does not accept caller commands, paths, statuses, trust, cleanup, or evidence/u,
    );
    assert.throws(
      () => requireMacosNativeAdapterSourceReceipt({ ...sourceReceipt }, plan, artifactReceipt),
      /process-local closed macOS source adapter/u,
    );
    const secondCapability = await acquireNativeArtifactCapability(plan, { verified_root: root });
    const secondArtifactReceipt = await verifyOwnedNativeArtifactCapability(secondCapability);
    assert.throws(
      () => requireMacosNativeAdapterSourceReceipt(sourceReceipt, plan, secondArtifactReceipt),
      /exact process-local plan and artifact verification receipt identities/u,
    );
    await closeNativeArtifactCapability(secondCapability);
    await closeNativeArtifactCapability(capability);
  }
});

test("macOS source receipts cannot replay across an equivalent process-local plan", async () => {
  const firstPlan = createInstallSmokePlan(macosPlanInput("dmg"));
  const secondPlan = createInstallSmokePlan(macosPlanInput("dmg"));
  assert.equal(firstPlan.plan_id, secondPlan.plan_id);
  assert.deepEqual(firstPlan.asset, secondPlan.asset);
  assert.notEqual(firstPlan.identity_receipt, secondPlan.identity_receipt);

  const firstRoot = await verifiedRootFor(firstPlan);
  const secondRoot = await verifiedRootFor(secondPlan);
  const firstCapability = await acquireNativeArtifactCapability(firstPlan, {
    verified_root: firstRoot,
  });
  const secondCapability = await acquireNativeArtifactCapability(secondPlan, {
    verified_root: secondRoot,
  });
  const firstArtifactReceipt = await verifyOwnedNativeArtifactCapability(firstCapability);
  const secondArtifactReceipt = await verifyOwnedNativeArtifactCapability(secondCapability);
  const sourceReceipt = bindMacosNativeAdapterSource(firstPlan, firstArtifactReceipt);

  assert.throws(
    () =>
      requireMacosNativeAdapterSourceReceipt(
        sourceReceipt,
        secondPlan,
        secondArtifactReceipt,
      ),
    /exact process-local plan and artifact verification receipt identities/u,
  );
  await closeNativeArtifactCapability(secondCapability);
  await closeNativeArtifactCapability(firstCapability);
});

test("macOS source execution stays skipped and preserves updater staging-only", async () => {
  for (const packageKind of ["dmg", "macos_updater"]) {
    const plan = createInstallSmokePlan(macosPlanInput(packageKind));
    const root = await verifiedRootFor(plan);
    const result = await runNativeInstallSmokeSourceSlice(plan, { verified_root: root });

    assert.equal(result.disposition, "skipped");
    assert.equal(result.native_execution_receipt, null);
    assert.equal(result.evidence_packet, null);
    assert.equal(result.steps.find(({ id }) => id === "preflight.asset_rehash").status, "passed");
    assert.equal(
      result.steps.find(({ id }) => id === "preflight.package_trust").status,
      "unsupported",
    );
    assert.equal(result.steps.find(({ id }) => id === "install.package_install").status, "blocked");
    assert.deepEqual(
      result.profile.required_limitations,
      packageKind === "dmg" ? [] : ["macos_updater_staging_only"],
    );
    assert.equal(validateNativeInstallSmokeResult(result), result);
  }
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

test("serialized plans cannot authorize a separate Rust composition root", () => {
  const plan = createInstallSmokePlan(planInput());
  const structuredCopy = structuredClone(plan);
  const jsonCopy = JSON.parse(JSON.stringify(plan));

  assert.throws(
    () => validateInstallSmokePlan(structuredCopy),
    /must retain the process-local selected-asset identity/u,
  );
  assert.throws(
    () => validateInstallSmokePlan(jsonCopy),
    /must retain the process-local selected-asset identity/u,
  );
});

test("superseded no-entry decision preserves the rejected bridge boundary", () => {
  const decision = fs.readFileSync(
    path.join(
      ROOT,
      "docs",
      "decisions",
      "0004-rust-install-smoke-complete-operation-entry.md",
    ),
    "utf8",
  );
  const rustSourceRoot = path.join(ROOT, "src", "BatCave.App", "src-tauri", "src");
  const rustSourceFiles = fs
    .readdirSync(rustSourceRoot, { recursive: true, withFileTypes: true })
    .filter((entry) => entry.isFile() && entry.name.endsWith(".rs"))
    .map((entry) => path.join(entry.parentPath, entry.name));
  const productionRust = rustSourceFiles
    .map((file) => fs.readFileSync(file, "utf8"))
    .join("\n");
  const cargoManifest = fs.readFileSync(
    path.join(ROOT, "src", "BatCave.App", "src-tauri", "Cargo.toml"),
    "utf8",
  );
  const executor = fs.readFileSync(
    path.join(ROOT, "scripts", "native-install-smoke-executor.mjs"),
    "utf8",
  );

  assert.match(
    decision,
    /Status: superseded; the Rust-owned `batcave-install-smoke` entry now preserves the accepted boundary/u,
  );
  assert.match(
    decision,
    /historical record for rejecting a serialized JavaScript-to-Rust authority bridge/u,
  );
  assert.match(decision, /Do not add a production Rust install-smoke composition root yet/u);
  assert.match(decision, /Rust independently reads the immutable public release/u);
  assert.match(
    cargoManifest,
    /\[\[bin\]\]\s*name = "batcave-install-smoke"\s*path = "src\/bin\/batcave-install-smoke\.rs"\s*required-features = \["private-release-verifier"\]/u,
  );
  assert.equal(
    rustSourceFiles.filter((file) => /native[_-]install[_-]smoke/iu.test(file)).length,
    0,
  );
  assert.doesNotMatch(productionRust, /native[_-]install[_-]smoke/iu);
  assert.doesNotMatch(cargoManifest, /(?:^|\n)\s*\[\[bin\]\][\s\S]*native[_-]install[_-]smoke/iu);
  assert.doesNotMatch(cargoManifest, /\b(?:napi|neon)\b/iu);
  assert.equal(executor.match(/nativeExecutionReceipts\.add\s*\(/gu)?.length ?? 0, 0);
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
