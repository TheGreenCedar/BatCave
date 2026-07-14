import assert from "node:assert/strict";
import crypto from "node:crypto";
import fs from "node:fs";
import fsp from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import process from "node:process";
import { after, before, test } from "node:test";
import { fileURLToPath } from "node:url";

import { createInstallSmokePlan } from "./install-smoke-contract.mjs";
import {
  createClosedLinuxAdapterSourceDescriptor,
  requireClosedLinuxAdapterSourceDescriptor,
  runClosedLinuxProcessSettlementContract,
  validateClosedLinuxProcessSettlementResult,
} from "./linux-native-install-smoke-adapter.mjs";
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
const FIXTURE_ROOT = path.join(ROOT, "docs", "evidence", "releases", "fixtures", "v1");
const SOURCE_SHA = "0123456789abcdef0123456789abcdef01234567";
const TAG = "v9.9.9-rc.1";
const UPDATER_KEY = "sha256:0dad0009cf5cc87a778f2e951cefaa0faaba637b95a22f6f3064f12cd4136545";

let contractReceipt;
let payloads;
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
      .map((name) => [name, `verified public linux-adapter bytes for ${name}\n`]),
  );
  const manifest = [...subjects]
    .map(([name, contents]) => `${digest(contents).slice("sha256:".length)}  ./${name}\n`)
    .join("");
  const fixturePayloads = new Map([
    ...subjects,
    [CHECKSUM_MANIFEST, manifest],
    [provenance, '{"bundle":"linux-adapter-fixture"}\n'],
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

function roleFor(kind) {
  return kind === "deb" ? "Linux deb package" : "Linux AppImage package and updater payload";
}

function releaseTemplate(kind) {
  const packet = JSON.parse(fs.readFileSync(path.join(FIXTURE_ROOT, `linux-${kind}.json`), "utf8"));
  packet.packet_kind = "release_evidence";
  packet.packet_id = `linux-${kind}-adapter-source`;
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
  if (kind === "deb") {
    packet.limitations.deb_checksum_attestation_only.disposition = "accepted";
  }
  const assetName = expectedReleaseAssetRoles(TAG).roles.find(
    ({ role }) => role === roleFor(kind),
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
    signatures:
      kind === "appimage" ? { tauri_updater: { identity: UPDATER_KEY, verified: true } } : {},
  };
  for (const checks of Object.values(packet.checks)) {
    for (const check of Object.values(checks)) {
      check.status = "blocked";
      check.outcome = "Awaiting exact public-artifact Linux execution.";
    }
  }
  validateReleaseEvidencePacket(packet);
  return packet;
}

function plan(kind) {
  return createInstallSmokePlan({
    schema_version: 1,
    execution_kind: "plan",
    app_version: "9.9.9-rc.1",
    evidence_template: releaseTemplate(kind),
    public_verification: contractReceipt,
    isolation: {
      scope_id: `linux-${kind}-adapter-source`,
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

function fixturePlan() {
  const packet = JSON.parse(
    fs.readFileSync(path.join(FIXTURE_ROOT, "linux-appimage.json"), "utf8"),
  );
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
      scope_id: "linux-adapter-fixture",
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

async function verifiedRootFor(selectedPlan) {
  const root = await fsp.mkdtemp(path.join(scratchRoot, "verified-"));
  await fsp.writeFile(
    path.join(root, selectedPlan.asset.name),
    payloads.get(selectedPlan.asset.name),
    { mode: 0o600 },
  );
  return root;
}

before(async () => {
  scratchRoot = await fsp.mkdtemp(path.join(os.tmpdir(), "batcave-linux-adapter-test-"));
  const fixture = publicReleaseFixture();
  payloads = fixture.payloads;
  const result = await verifyPublicRelease(
    fixture.candidate,
    fixture.release,
    path.join(scratchRoot, "public"),
    {
      fetchImpl: async (url) => {
        const name = decodeURIComponent(new URL(url).pathname.split("/").at(-1));
        return payloads.has(name)
          ? new Response(payloads.get(name), { status: 200 })
          : new Response("not found", { status: 404 });
      },
      ghRunner: () => {},
    },
  );
  contractReceipt = result.receipt;
});

after(async () => {
  await fsp.rm(scratchRoot, { recursive: true, force: true });
});

test("deb and AppImage descriptors preserve their exact closed profiles", () => {
  const debPlan = plan("deb");
  const appImagePlan = plan("appimage");
  const deb = createClosedLinuxAdapterSourceDescriptor(debPlan);
  const appImage = createClosedLinuxAdapterSourceDescriptor(appImagePlan);

  assert.equal(requireClosedLinuxAdapterSourceDescriptor(deb, debPlan), deb);
  assert.equal(requireClosedLinuxAdapterSourceDescriptor(appImage, appImagePlan), appImage);
  assert.equal(deb.profile_id, "linux:deb");
  assert.equal(deb.profile.package_operation, "install");
  assert.deepEqual(deb.profile.required_limitations, ["deb_checksum_attestation_only"]);
  assert.equal(appImage.profile_id, "linux:appimage");
  assert.equal(appImage.profile.package_operation, "stage");
  assert.deepEqual(appImage.profile.required_limitations, []);
  assert.equal(deb.execution_state, "package_bytes_not_executed");
  assert.equal(appImage.native_proof_eligibility, "none");
  assert.equal(deb.required_gate_order.length, 13);
});

test("descriptor input exposes no command, environment, path, callback, status, or evidence seam", () => {
  const selectedPlan = plan("deb");
  let called = false;
  const forbidden = [
    { command: ["dpkg", "--install"] },
    { env: process.env },
    { path: "/tmp/caller-package.deb" },
    { status: "passed" },
    { evidence_packet: { packet_kind: "release_evidence" } },
    () => {
      called = true;
    },
  ];
  for (const value of forbidden) {
    assert.throws(
      () => createClosedLinuxAdapterSourceDescriptor(selectedPlan, value),
      /does not accept caller commands, environment, paths, callbacks, statuses, or evidence/u,
    );
  }
  assert.equal(called, false);

  const descriptor = createClosedLinuxAdapterSourceDescriptor(selectedPlan);
  const serialized = JSON.stringify(descriptor);
  assert.doesNotMatch(serialized, /(?:\/private\/|\/Users\/|[A-Za-z]:\\)/u);
  assert.doesNotMatch(serialized, /evidence_packet|native_execution_receipt/u);
  assert.equal(descriptor.process_boundary.caller_commands, false);
  assert.equal(descriptor.process_boundary.caller_environment, false);
  assert.equal(descriptor.process_boundary.caller_paths, false);
});

test("fixtures, forged descriptors, and cross-profile reuse fail before execution", () => {
  assert.throws(
    () => createClosedLinuxAdapterSourceDescriptor(fixturePlan()),
    /fixtures cannot register adapters/u,
  );
  const debPlan = plan("deb");
  const appImagePlan = plan("appimage");
  const descriptor = createClosedLinuxAdapterSourceDescriptor(debPlan);
  assert.throws(
    () => requireClosedLinuxAdapterSourceDescriptor({ ...descriptor }, debPlan),
    /process-local built-in descriptor/u,
  );
  assert.throws(
    () => requireClosedLinuxAdapterSourceDescriptor(descriptor, appImagePlan),
    /exact plan identity and Linux profile/u,
  );
});

test("native source execution registers Linux source only and cannot mint proof", async () => {
  const selectedPlan = plan("appimage");
  const root = await verifiedRootFor(selectedPlan);
  const result = await runNativeInstallSmokeSourceSlice(selectedPlan, { verified_root: root });
  assert.equal(result.disposition, "skipped");
  assert.equal(result.native_execution_receipt, null);
  assert.equal(result.evidence_packet, null);
  assert.match(
    result.steps.find(({ id }) => id === "preflight.package_trust").outcome,
    /closed Linux source adapter is registered/u,
  );
  assert.equal(validateNativeInstallSmokeResult(result), result);
  await assert.rejects(
    () =>
      runNativeInstallSmokeSourceSlice(selectedPlan, {
        verified_root: root,
        adapter: createClosedLinuxAdapterSourceDescriptor(selectedPlan),
      }),
    /native_executor.options.adapter.*not allowed/u,
  );
});

test("fixed Linux process contract settles descendants and bounded output without package bytes", async () => {
  const result = await runClosedLinuxProcessSettlementContract();
  assert.equal(validateClosedLinuxProcessSettlementResult(result), result);
  assert.equal(result.package_bytes_executed, false);
  assert.equal(result.native_execution_receipt, null);
  assert.equal(result.evidence_packet, null);
  if (process.platform === "linux") {
    assert.equal(result.disposition, "source_contract_verified");
    assert.equal(result.cleanup, "passed");
    assert.deepEqual(
      result.probes.map(({ id }) => id),
      ["normal_exit", "descendant_after_parent_exit", "stubborn_process_tree", "bounded_output"],
    );
    assert.ok(result.probes.every(({ process_tree_settled }) => process_tree_settled));
  } else {
    assert.equal(result.disposition, "unsupported");
    assert.deepEqual(result.probes, []);
    assert.equal(result.cleanup, "not_run");
  }
});

test("process contract rejects caller control and forged settlement results", async () => {
  await assert.rejects(
    () => runClosedLinuxProcessSettlementContract({ command: "caller" }),
    /does not accept caller commands, environment, paths, statuses, or evidence/u,
  );
  const result = await runClosedLinuxProcessSettlementContract();
  assert.throws(
    () =>
      validateClosedLinuxProcessSettlementResult({
        ...result,
        disposition: "source_contract_verified",
        native_execution_receipt: { native_proven: true },
      }),
    /process-local result/u,
  );
});

test(
  "unconfirmed hard settlement retains the owned root and fails closed",
  { skip: process.platform !== "linux" },
  async () => {
    const beforeRoots = new Set(
      (await fsp.readdir(os.tmpdir())).filter((name) =>
        name.startsWith("batcave-linux-adapter-contract-"),
      ),
    );
    const originalKill = process.kill;
    const forcedUncertainGroups = new Set();
    process.kill = (pid, signal) => {
      if (signal === "SIGKILL" && Number.isSafeInteger(pid) && pid < 0) {
        const outcome = originalKill(pid, signal);
        forcedUncertainGroups.add(pid);
        return outcome;
      }
      if (signal === 0 && forcedUncertainGroups.has(pid)) return true;
      return originalKill(pid, signal);
    };
    let result;
    try {
      result = await runClosedLinuxProcessSettlementContract();
    } finally {
      process.kill = originalKill;
      const retained = (await fsp.readdir(os.tmpdir())).filter(
        (name) => name.startsWith("batcave-linux-adapter-contract-") && !beforeRoots.has(name),
      );
      await Promise.all(
        retained.map((name) =>
          fsp.rm(path.join(os.tmpdir(), name), { recursive: true, force: true }),
        ),
      );
    }
    assert.equal(result.disposition, "failed");
    assert.equal(result.cleanup, "retained_unsettled");
    assert.ok(result.probes.some(({ process_tree_settled }) => !process_tree_settled));
    assert.ok(!result.probes.some(({ id }) => id === "bounded_output"));
    assert.equal(result.native_execution_receipt, null);
    assert.equal(result.evidence_packet, null);
    assert.equal(validateClosedLinuxProcessSettlementResult(result), result);
  },
);

test(
  "cleanup failure is a failed source contract with no proof",
  { skip: process.platform !== "linux" },
  async () => {
    const beforeRoots = new Set(
      (await fsp.readdir(os.tmpdir())).filter((name) =>
        name.startsWith("batcave-linux-adapter-contract-"),
      ),
    );
    const originalRm = fsp.rm;
    fsp.rm = async (target, options) => {
      if (path.basename(String(target)).startsWith("batcave-linux-adapter-contract-")) {
        throw new Error("simulated cleanup failure");
      }
      return originalRm(target, options);
    };
    let result;
    try {
      result = await runClosedLinuxProcessSettlementContract();
    } finally {
      fsp.rm = originalRm;
      const leaked = (await fsp.readdir(os.tmpdir())).filter(
        (name) => name.startsWith("batcave-linux-adapter-contract-") && !beforeRoots.has(name),
      );
      await Promise.all(
        leaked.map((name) =>
          originalRm(path.join(os.tmpdir(), name), { recursive: true, force: true }),
        ),
      );
    }
    assert.equal(result.disposition, "failed");
    assert.equal(result.cleanup, "failed");
    assert.equal(result.native_execution_receipt, null);
    assert.equal(result.evidence_packet, null);
    assert.equal(validateClosedLinuxProcessSettlementResult(result), result);
  },
);

test("hosted workflows run the Linux adapter contract on every validation host", () => {
  const releaseWorkflow = fs.readFileSync(
    path.join(ROOT, ".github", "workflows", "release.yml"),
    "utf8",
  );
  const validationWorkflow = fs.readFileSync(
    path.join(ROOT, ".github", "workflows", "validation.yml"),
    "utf8",
  );
  assert.equal(
    releaseWorkflow.match(/scripts\/linux-native-install-smoke-adapter\.test\.mjs/gu)?.length,
    1,
  );
  assert.equal(
    validationWorkflow.match(/scripts\/linux-native-install-smoke-adapter\.test\.mjs/gu)?.length,
    3,
  );
});
