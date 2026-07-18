import assert from "node:assert/strict";
import crypto from "node:crypto";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { after, before, test } from "node:test";
import { fileURLToPath } from "node:url";

import { expectedReleaseAssetRoles } from "./release-asset-contract.mjs";
import {
  createInstallSmokePlan,
  runInstallSmoke,
  shapeInstallSmokeEvidence,
  validateInstallSmokePlan,
  validateInstallSmokeResult,
} from "./public-artifact-install-smoke.mjs";
import { validateReleaseEvidenceTemplatePacket } from "./validate-release-evidence-packet.mjs";
import {
  CHECKSUM_MANIFEST,
  RELEASE_REPOSITORY,
  verifyPublicRelease,
} from "./verify-public-release.mjs";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const FIXTURE_DIR = path.join(ROOT, "docs", "evidence", "releases", "fixtures", "v1");
const SOURCE_SHA = "0123456789abcdef0123456789abcdef01234567";
const TAG = "v9.9.9-rc.1";
const FIXTURES = [
  ["windows-nsis.json", "windows", "nsis"],
  ["linux-deb.json", "linux", "deb"],
  ["linux-appimage.json", "linux", "appimage"],
  ["macos-dmg.json", "macos", "dmg"],
  ["macos-updater.json", "macos", "macos_updater"],
];
const PACKAGE_ROLES = {
  appimage: "Linux AppImage package and updater payload",
  deb: "Linux deb package",
  dmg: "macOS Apple Silicon DMG",
  macos_updater: "macOS Apple Silicon updater payload",
  nsis: "Windows NSIS installer and updater payload",
};
const REAL_SIGNATURE_IDENTITIES = {
  apple_notarization: "submission-id:123e4567-e89b-42d3-a456-426614174000",
  apple_staple: `ticket-sha256:${"a".repeat(64)}`,
  authenticode: `sha256:${"b".repeat(64)}`,
  contained_app_developer_id: "Developer ID Application: BatCave Monitor (ABCDEFGHIJ)",
  contained_app_notarization: "submission-id:123e4567-e89b-42d3-a456-426614174001",
  contained_app_staple: `ticket-sha256:${"c".repeat(64)}`,
  developer_id: "Developer ID Application: BatCave Monitor (ABCDEFGHIJ)",
  tauri_updater: "sha256:0dad0009cf5cc87a778f2e951cefaa0faaba637b95a22f6f3064f12cd4136545",
};

let contractReceipt;
let publicRoot;

function readJson(file) {
  return JSON.parse(fs.readFileSync(file, "utf8"));
}

function digest(contents) {
  return `sha256:${crypto.createHash("sha256").update(contents).digest("hex")}`;
}

function mutableIdentityCopy(value) {
  const { identity_receipt: identityReceipt, ...serializable } = value;
  return { ...structuredClone(serializable), identity_receipt: identityReceipt };
}

function publicReleaseFixture() {
  const contract = expectedReleaseAssetRoles(TAG);
  const provenance = contract.roles.find(({ role }) => role === "build provenance bundle").name;
  const subjects = new Map(
    contract.roles
      .map(({ name }) => name)
      .filter((name) => name !== CHECKSUM_MANIFEST && name !== provenance)
      .map((name) => [name, `verified public fixture bytes for ${name}\n`]),
  );
  const manifest = [...subjects]
    .map(([name, contents]) => `${digest(contents).slice("sha256:".length)}  ./${name}\n`)
    .join("");
  const payloads = new Map([
    ...subjects,
    [CHECKSUM_MANIFEST, manifest],
    [provenance, '{"bundle":"fixture"}\n'],
  ]);
  const assets = [...payloads]
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
    payloads,
  };
}

before(async () => {
  publicRoot = fs.mkdtempSync(path.join(os.tmpdir(), "batcave-install-smoke-test-"));
  const { candidate, release, payloads } = publicReleaseFixture();
  const result = await verifyPublicRelease(candidate, release, path.join(publicRoot, "public"), {
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

after(() => {
  fs.rmSync(publicRoot, { recursive: true, force: true });
});

function schemaFixture(name) {
  return readJson(path.join(FIXTURE_DIR, name));
}

function releaseTemplate(name) {
  const packet = schemaFixture(name);
  packet.packet_kind = "release_evidence";
  packet.packet_id = packet.packet_id.replace("schema-fixture", "install-smoke-plan");
  packet.release.tag = TAG;
  packet.release.channel = "prerelease";
  packet.release.source_sha = SOURCE_SHA;
  packet.release.main_sha = SOURCE_SHA;
  packet.release.release_target_sha = SOURCE_SHA;
  packet.release.release_url = `https://github.com/${RELEASE_REPOSITORY}/releases/tag/${TAG}`;
  packet.release.workflow_run = {
    workflow_file: ".github/workflows/release.yml",
    run_id: 123456789,
    run_attempt: 2,
    url: `https://github.com/${RELEASE_REPOSITORY}/actions/runs/123456789/attempts/2`,
  };
  packet.platform.os_version = {
    "debian-12-x86_64-glibc": "debian-12",
    "macos-12-arm64": "macos-12.0",
    "ubuntu-22.04-x86_64-glibc": "ubuntu-22.04",
    "windows-client-10-x86_64": "windows-client-10.0.16299",
  }[packet.platform.profile_id];
  packet.platform.proof.source = "source_enforced";
  packet.platform.proof.native = "pending";
  delete packet.limitations.synthetic_fixture_no_release_claim;
  if (packet.platform.package.kind === "deb") {
    packet.limitations.deb_checksum_attestation_only.disposition = "accepted";
  }
  const role = PACKAGE_ROLES[packet.platform.package.kind];
  const assetName = expectedReleaseAssetRoles(TAG).roles.find(
    (candidate) => candidate.role === role,
  ).name;
  const verified = contractReceipt.assets.find(({ name: candidate }) => candidate === assetName);
  packet.platform.package.asset_name = assetName;
  packet.assets[0].name = assetName;
  packet.assets[0].size_bytes = verified.size_bytes;
  packet.assets[0].sha256 = verified.sha256;
  packet.assets[0].api_digest = verified.sha256;
  packet.assets[0].public_url = verified.public_url;
  packet.assets[0].attestation.source_sha = SOURCE_SHA;
  for (const [kind, signature] of Object.entries(packet.assets[0].signatures)) {
    signature.identity = REAL_SIGNATURE_IDENTITIES[kind];
  }
  for (const checks of Object.values(packet.checks)) {
    for (const check of Object.values(checks)) {
      check.status = "blocked";
      check.outcome = "Awaiting a future reviewed native install-smoke executor.";
    }
  }
  validateReleaseEvidenceTemplatePacket(packet);
  return packet;
}

function fixtureReceipt(packet) {
  const asset = packet.assets.find(({ name }) => name === packet.platform.package.asset_name);
  return {
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
  };
}

function inputFor(name, executionKind = "fixture") {
  const packet = executionKind === "fixture" ? schemaFixture(name) : releaseTemplate(name);
  const scenario =
    packet.platform.os === "windows"
      ? "standard-access-visibility"
      : "permission-limited-telemetry";
  return {
    schema_version: 1,
    execution_kind: executionKind,
    app_version: executionKind === "fixture" ? "0.0.0-evidence.1" : "9.9.9-rc.1",
    evidence_template: packet,
    public_verification: executionKind === "fixture" ? fixtureReceipt(packet) : contractReceipt,
    isolation: {
      scope_id: `${packet.platform.os}-${packet.platform.package.kind}-smoke`,
      install_root_id: "isolated-install-root",
      user_state_root_id: "isolated-user-state-root",
      user_state_policy: "preserve",
      step_timeout_ms: 1_000,
      termination_timeout_ms: 100,
      settings_probe: { theme: "cave", sample_interval_ms: 1_500 },
      degradation_scenario: scenario,
    },
  };
}

function passedResponse(step, plan) {
  const observations = {
    "preflight.asset_rehash": {
      sha256: plan.asset.sha256,
      regular_file: true,
      symlink: false,
      contained: true,
    },
    "preflight.package_trust": {
      trust_basis: plan.profile.trust_basis,
      fixture_only: true,
    },
    "install.package_install": {
      package_ready: true,
      package_operation: plan.profile.package_operation,
    },
    "runtime.launch": { launched: true },
    "runtime.release_identity": {
      app_version: plan.release.app_version,
      source_commit_sha: plan.release.source_sha,
      install_kind: plan.profile.install_kind,
    },
    "runtime.settings": { restarted: true, preserved: true },
    "runtime.degradation": { reported: true, windows_service_behavior: "not_assumed" },
    "runtime.telemetry": { sample_observed: true, quality_state: "limited" },
    "cleanup.application_removed": { removed: true },
    "cleanup.owned_runtime_cleanup": { residue_count: 0 },
    "cleanup.user_state_policy": { policy: "preserved" },
  }[step.id];
  return { status: "passed", outcome: `Bounded ${step.id} fixture completed.`, observations };
}

function adapterFor(
  input,
  overrides = {},
  confirmTerminated = async () => ({
    action_settled: true,
    process_tree_settled: true,
    outcome: "Fixture process tree settled.",
  }),
) {
  const plan = createInstallSmokePlan(input);
  const calls = [];
  const actions = {};
  for (const step of plan.steps.filter(({ source }) => source === "adapter")) {
    actions[step.action] = async (request) => {
      calls.push(request.step.id);
      return overrides[request.step.id]
        ? overrides[request.step.id](request, plan)
        : passedResponse(request.step, plan);
    };
  }
  return {
    plan,
    calls,
    adapter: {
      kind: "fixture",
      executor: {
        tokenized_argv: true,
        shell: false,
        minimal_environment: true,
        bounded_output: true,
        timeout_tree_cleanup: true,
        confirm_terminated: confirmTerminated,
      },
      actions,
    },
  };
}

for (const [file, os, packageKind] of FIXTURES) {
  test(`runs a non-claiming ${os} ${packageKind} fixture`, async () => {
    const input = inputFor(file);
    const { plan, calls, adapter } = adapterFor(input);
    const result = await runInstallSmoke(input, adapter);
    assert.equal(result.disposition, "fixture");
    assert.equal(result.evidence_packet.packet_kind, "schema_fixture");
    assert.ok(
      Object.values(result.evidence_packet.checks).every((checks) =>
        Object.values(checks).every(
          ({ status, outcome }) =>
            status === "not_applicable" && outcome === "Synthetic install-smoke fixture only.",
        ),
      ),
    );
    assert.deepEqual(
      calls,
      plan.steps.filter(({ source }) => source === "adapter").map(({ id }) => id),
    );
    assert.deepEqual(
      result.evidence_packet.assets.map(({ name }) => name),
      [plan.asset.name],
    );
    assert.doesNotMatch(JSON.stringify(result), /(?:\/private\/|[A-Za-z]:\\)/u);
    if (packageKind === "macos_updater") {
      assert.equal(plan.profile.package_operation, "stage");
      assert.equal(
        result.evidence_packet.limitations.macos_updater_staging_only.disposition,
        "not_applicable",
      );
    }
    if (packageKind === "nsis") {
      assert.equal(
        result.evidence_packet.limitations.windows_service_etw_out_of_scope.disposition,
        "not_applicable",
      );
    }
  });
}

test("returns a workflow-bound plan with one receipt-bound asset and no packet", async () => {
  const input = inputFor("linux-appimage.json", "plan");
  const result = await runInstallSmoke(input);
  const plan = createInstallSmokePlan(input);
  assert.equal(result.disposition, "planned");
  assert.equal(result.evidence_packet, null);
  assert.equal(result.release.workflow_run.run_id, 123456789);
  assert.deepEqual(result.platform.proof, {
    declaration: "declared",
    source: "source_enforced",
    native: "pending",
  });
  assert.equal(result.observed_at_utc, input.evidence_template.observed_at_utc);
  assert.deepEqual(Object.keys(plan.public_verification).sort(), [
    "app_version",
    "asset",
    "disposition",
    "proof_scope",
    "repository",
    "schema_version",
    "source_sha",
    "tag",
    "verifier",
  ]);
  assert.deepEqual(Object.keys(plan.asset).sort(), ["name", "public_url", "sha256", "size_bytes"]);
  assert.ok(result.steps.slice(2).every(({ status }) => status === "planned"));
});

test("rejects a plan template that claims a native observation", () => {
  const input = inputFor("linux-appimage.json", "plan");
  input.evidence_template.platform.proof.native = "observed";
  assert.throws(() => createInstallSmokePlan(input), /proof.native: must equal pending/u);
});

test("plan validation rejects malformed or contradictory release identity", () => {
  const source = createInstallSmokePlan(inputFor("linux-appimage.json", "plan"));
  const mutations = [
    (plan) => {
      plan.observed_at_utc = "2000-02-30T00:00:00Z";
    },
    (plan) => {
      plan.release.repository = "example/BatCave";
    },
    (plan) => {
      plan.release.channel = "stable";
    },
    (plan) => {
      plan.release.app_version = "9.9.8";
    },
    (plan) => {
      plan.release.release_url = "https://github.com/TheGreenCedar/BatCave/releases/tag/v9.9.8";
    },
    (plan) => {
      plan.release.main_sha = "f".repeat(40);
    },
    (plan) => {
      plan.release.workflow_run.url =
        "https://github.com/TheGreenCedar/BatCave/actions/runs/123456789/attempts/3";
    },
    (plan) => {
      plan.asset.name = "BatCave.Monitor_9.9.9-rc.1_unknown.AppImage";
    },
    (plan) => {
      plan.asset.sha256 = `sha256:${"F".repeat(64)}`;
    },
    (plan) => {
      plan.platform.architecture = "sparc64";
    },
    (plan) => {
      plan.platform.package.architecture = "universal";
    },
  ];
  for (const mutate of mutations) {
    const plan = mutableIdentityCopy(source);
    mutate(plan);
    assert.throws(
      () => validateInstallSmokePlan(plan),
      /packet|identity|release|platform|asset|app_version|source_sha/u,
    );
  }

  const pairedDigestRewrite = mutableIdentityCopy(source);
  pairedDigestRewrite.asset.sha256 = `sha256:${"f".repeat(64)}`;
  pairedDigestRewrite.public_verification.asset.sha256 = pairedDigestRewrite.asset.sha256;
  assert.throws(
    () => validateInstallSmokePlan(pairedDigestRewrite),
    /process-local selected-asset identity|bind the exact/u,
  );

  const pairedSizeRewrite = mutableIdentityCopy(source);
  pairedSizeRewrite.asset.size_bytes += 1;
  pairedSizeRewrite.public_verification.asset.size_bytes = pairedSizeRewrite.asset.size_bytes;
  assert.throws(
    () => validateInstallSmokePlan(pairedSizeRewrite),
    /process-local selected-asset identity|bind the exact/u,
  );

  const pairedWorkflowRewrite = mutableIdentityCopy(source);
  pairedWorkflowRewrite.release.workflow_run.run_id = 987654321;
  pairedWorkflowRewrite.release.workflow_run.run_attempt = 4;
  pairedWorkflowRewrite.release.workflow_run.url =
    "https://github.com/TheGreenCedar/BatCave/actions/runs/987654321/attempts/4";
  assert.throws(
    () => validateInstallSmokePlan(pairedWorkflowRewrite),
    /process-local selected-asset identity|bind the exact/u,
  );
});

test("caller-authored native flags and injected functions cannot mint proof", async () => {
  const input = inputFor("linux-appimage.json", "plan");
  input.execution_kind = "native";
  let calls = 0;
  const attacker = {
    kind: "native",
    native_proven: true,
    executor: {
      tokenized_argv: true,
      shell: false,
      minimal_environment: true,
      bounded_output: true,
      timeout_tree_cleanup: true,
      confirm_terminated: async () => {
        calls += 1;
        return { action_settled: true, process_tree_settled: true, outcome: "forged" };
      },
    },
    actions: new Proxy(
      {},
      {
        get: () => async () => {
          calls += 1;
        },
      },
    ),
  };
  await assert.rejects(
    () => runInstallSmoke(input, attacker),
    /native proof is unavailable without a reviewed branded executor/u,
  );
  assert.equal(calls, 0);
  assert.throws(
    () => shapeInstallSmokeEvidence({ execution_kind: "native" }, "native_proven"),
    /release evidence is unreachable/u,
  );
});

test("a copied contract receipt is rejected before any adapter action", async () => {
  const input = inputFor("linux-appimage.json", "plan");
  input.public_verification = structuredClone(contractReceipt);
  await assert.rejects(
    () => runInstallSmoke(input),
    /must come from a successful in-process verifyPublicRelease call/u,
  );
});

test("rejects missing, inherited, accessor, and unsafe fixture executor capabilities", async () => {
  const input = inputFor("linux-deb.json");
  const missing = adapterFor(input);
  delete missing.adapter.actions.install_deb;
  await assert.rejects(
    () => runInstallSmoke(input, missing.adapter),
    /install_deb.*explicit data function/u,
  );
  assert.deepEqual(missing.calls, []);

  const inherited = adapterFor(input);
  const install = inherited.adapter.actions.install_deb;
  delete inherited.adapter.actions.install_deb;
  Object.setPrototypeOf(inherited.adapter.actions, { install_deb: install });
  await assert.rejects(
    () => runInstallSmoke(input, inherited.adapter),
    /install_deb.*explicit data function/u,
  );
  assert.deepEqual(inherited.calls, []);

  const accessor = adapterFor(input);
  delete accessor.adapter.actions.install_deb;
  Object.defineProperty(accessor.adapter.actions, "install_deb", {
    enumerable: true,
    get: () => install,
  });
  await assert.rejects(
    () => runInstallSmoke(input, accessor.adapter),
    /install_deb.*explicit data function/u,
  );
  assert.deepEqual(accessor.calls, []);

  const unsafe = adapterFor(input);
  unsafe.adapter.executor.shell = true;
  await assert.rejects(
    () => runInstallSmoke(input, unsafe.adapter),
    /bounded non-shell fixture executor/u,
  );
  assert.deepEqual(unsafe.calls, []);
});

test("adapter hash and trust self-attestations remain fixture-only", async () => {
  const input = inputFor("linux-appimage.json");
  const { adapter, calls } = adapterFor(input, {
    "preflight.asset_rehash": (_request, plan) => ({
      status: "passed",
      outcome: "Synthetic file probe returned.",
      observations: {
        sha256: plan.asset.sha256,
        regular_file: true,
        symlink: true,
        contained: true,
      },
    }),
  });
  const result = await runInstallSmoke(input, adapter);
  assert.equal(result.disposition, "fixture");
  assert.equal(result.steps.find(({ id }) => id === "preflight.asset_rehash").status, "failed");
  assert.ok(!calls.includes("install.package_install"));
  assert.equal(result.evidence_packet.checks.install.package_install.status, "not_applicable");
  assert.doesNotMatch(JSON.stringify(result), /native_proven|release_evidence/u);
});

test("confirmed timeout waits for settlement before bounded cleanup", async () => {
  const input = inputFor("linux-deb.json");
  input.isolation.step_timeout_ms = 5;
  let aborted = false;
  let handshake = 0;
  const { adapter, calls } = adapterFor(
    input,
    {
      "install.package_install": ({ signal }) =>
        new Promise(() => {
          signal.addEventListener("abort", () => {
            aborted = true;
          });
        }),
    },
    async () => {
      handshake += 1;
      return {
        action_settled: true,
        process_tree_settled: true,
        outcome: "Timed-out fixture tree settled.",
      };
    },
  );
  const result = await runInstallSmoke(input, adapter);
  assert.equal(result.disposition, "fixture");
  assert.equal(result.steps.find(({ id }) => id === "install.package_install").status, "timeout");
  assert.equal(aborted, true);
  assert.equal(handshake, 1);
  assert.ok(calls.includes("cleanup.application_removed"));
  assert.equal(result.evidence_packet.packet_kind, "schema_fixture");
});

test("unconfirmed timeout is partial, emits no packet, and runs no later action", async () => {
  const input = inputFor("linux-deb.json");
  input.isolation.step_timeout_ms = 5;
  input.isolation.termination_timeout_ms = 5;
  const { adapter, calls } = adapterFor(
    input,
    { "install.package_install": () => new Promise(() => {}) },
    () => new Promise(() => {}),
  );
  const result = await runInstallSmoke(input, adapter);
  assert.equal(result.disposition, "partial");
  assert.equal(result.steps.find(({ id }) => id === "install.package_install").status, "partial");
  assert.equal(result.evidence_packet, null);
  assert.ok(!calls.includes("runtime.launch"));
  assert.ok(!calls.includes("cleanup.application_removed"));
});

test("adapter-authored timeout states cannot bypass settlement or trigger cleanup", async () => {
  for (const forgedStatus of ["timeout", "partial"]) {
    const input = inputFor("linux-deb.json");
    let handshakes = 0;
    const { adapter, calls } = adapterFor(
      input,
      {
        "install.package_install": () => ({
          status: forgedStatus,
          outcome: "Caller-authored timeout state.",
          observations: {},
        }),
      },
      async () => {
        handshakes += 1;
        return {
          action_settled: true,
          process_tree_settled: true,
          outcome: "Unexpected handshake.",
        };
      },
    );
    const result = await runInstallSmoke(input, adapter);
    assert.equal(result.disposition, "partial");
    assert.equal(result.steps.find(({ id }) => id === "install.package_install").status, "partial");
    assert.equal(result.evidence_packet, null);
    assert.equal(handshakes, 0);
    assert.ok(!calls.includes("runtime.launch"));
    assert.ok(!calls.includes("cleanup.application_removed"));
  }
});

test("fixture failures, unsupported states, identity drift, and unsafe output fail closed", async () => {
  const cases = [
    [
      "runtime.launch",
      () => ({ status: "failed", outcome: "Synthetic launch failure.", observations: {} }),
      "failed",
    ],
    [
      "install.package_install",
      () => ({
        status: "unsupported",
        outcome: "Fixture capability unavailable.",
        observations: {},
      }),
      "unsupported",
    ],
    [
      "runtime.release_identity",
      (_request, plan) => ({
        status: "passed",
        outcome: "Synthetic identity returned.",
        observations: {
          app_version: "9.9.8",
          source_commit_sha: plan.release.source_sha,
          install_kind: plan.profile.install_kind,
        },
      }),
      "failed",
    ],
    [
      "runtime.telemetry",
      () => ({
        status: "passed",
        outcome: "Telemetry unavailable.",
        observations: { sample_observed: true, quality_state: "unavailable" },
      }),
      "failed",
    ],
    [
      "preflight.asset_rehash",
      (_request, plan) => ({
        status: "passed",
        outcome: "Evidence saved at /private/tmp/release.log",
        observations: {
          sha256: plan.asset.sha256,
          regular_file: true,
          symlink: false,
          contained: true,
        },
      }),
      "failed",
    ],
  ];
  for (const [stepId, override, expectedStatus] of cases) {
    const input = inputFor("linux-appimage.json");
    const { adapter } = adapterFor(input, { [stepId]: override });
    const result = await runInstallSmoke(input, adapter);
    assert.equal(result.disposition, "fixture");
    assert.equal(result.steps.find(({ id }) => id === stepId).status, expectedStatus);
    assert.equal(result.evidence_packet.packet_kind, "schema_fixture");
    assert.doesNotMatch(JSON.stringify(result), /\/private\/tmp/u);
  }
});

test("result validation rederives packet id, checks, limitations, and disposition", async () => {
  const input = inputFor("windows-nsis.json");
  const { adapter } = adapterFor(input);
  const result = await runInstallSmoke(input, adapter);

  const mutations = [
    (candidate) => {
      candidate.evidence_packet.packet_id = "contradictory-packet";
    },
    (candidate) => {
      candidate.evidence_packet.checks.runtime.launch.status = "passed";
    },
    (candidate) => {
      candidate.evidence_packet.checks.runtime.launch.outcome = "Contradictory.";
    },
    (candidate) => {
      delete candidate.evidence_packet.limitations.windows_service_etw_out_of_scope;
    },
    (candidate) => {
      candidate.evidence_packet.limitations.synthetic_fixture_no_release_claim.summary = "Changed.";
    },
    (candidate) => {
      candidate.disposition = "partial";
    },
  ];
  for (const mutate of mutations) {
    const candidate = mutableIdentityCopy(result);
    mutate(candidate);
    assert.throws(
      () => validateInstallSmokeResult(candidate),
      /derived|exactly match|fixture|packet|limitation|disposition/u,
    );
  }

  const reordered = mutableIdentityCopy(result);
  [reordered.steps[2], reordered.steps[3]] = [reordered.steps[3], reordered.steps[2]];
  assert.throws(() => validateInstallSmokeResult(reordered), /ordered gate/u);
});

test("result validation rejects malformed identity for planned, fixture, and partial states", async () => {
  const planned = await runInstallSmoke(inputFor("linux-appimage.json", "plan"));
  const fixtureInput = inputFor("linux-appimage.json");
  const fixture = await runInstallSmoke(fixtureInput, adapterFor(fixtureInput).adapter);
  const partialInput = inputFor("linux-appimage.json");
  partialInput.isolation.step_timeout_ms = 5;
  partialInput.isolation.termination_timeout_ms = 5;
  const partialAdapter = adapterFor(
    partialInput,
    { "install.package_install": () => new Promise(() => {}) },
    () => new Promise(() => {}),
  ).adapter;
  const partial = await runInstallSmoke(partialInput, partialAdapter);

  for (const source of [planned, fixture, partial]) {
    const appVersionMismatch = mutableIdentityCopy(source);
    appVersionMismatch.release.app_version = "0.0.1";
    assert.throws(() => validateInstallSmokeResult(appVersionMismatch), /app_version.*must equal/u);

    const impossibleTime = mutableIdentityCopy(source);
    impossibleTime.observed_at_utc = "2000-02-30T00:00:00Z";
    assert.throws(() => validateInstallSmokeResult(impossibleTime), /real UTC time/u);
  }

  const mutations = [
    (result) => {
      result.release.channel = "stable";
    },
    (result) => {
      result.release.main_sha = "f".repeat(40);
    },
    (result) => {
      result.release.workflow_run.run_attempt += 1;
    },
    (result) => {
      result.asset.name = "BatCave.Monitor_0.0.0-evidence.1_unknown.AppImage";
    },
    (result) => {
      result.platform.package.architecture = "universal";
    },
  ];
  for (const mutate of mutations) {
    const result = mutableIdentityCopy(fixture);
    mutate(result);
    assert.throws(
      () => validateInstallSmokeResult(result),
      /packet|identity|release|platform|asset|source_sha/u,
    );
  }

  const pairedDigestRewrite = mutableIdentityCopy(fixture);
  pairedDigestRewrite.asset.sha256 = `sha256:${"f".repeat(64)}`;
  pairedDigestRewrite.evidence_packet.assets[0].sha256 = pairedDigestRewrite.asset.sha256;
  pairedDigestRewrite.evidence_packet.assets[0].api_digest = pairedDigestRewrite.asset.sha256;
  assert.throws(
    () => validateInstallSmokeResult(pairedDigestRewrite),
    /process-local selected-asset identity|bind the exact/u,
  );

  const pairedSizeRewrite = mutableIdentityCopy(fixture);
  pairedSizeRewrite.asset.size_bytes += 1;
  pairedSizeRewrite.evidence_packet.assets[0].size_bytes = pairedSizeRewrite.asset.size_bytes;
  assert.throws(
    () => validateInstallSmokeResult(pairedSizeRewrite),
    /process-local selected-asset identity|bind the exact/u,
  );

  const pairedWorkflowRewrite = mutableIdentityCopy(fixture);
  pairedWorkflowRewrite.release.workflow_run.run_id = 987654321;
  pairedWorkflowRewrite.release.workflow_run.run_attempt = 4;
  pairedWorkflowRewrite.release.workflow_run.url =
    "https://github.com/TheGreenCedar/BatCave/actions/runs/987654321/attempts/4";
  pairedWorkflowRewrite.evidence_packet.release.workflow_run = structuredClone(
    pairedWorkflowRewrite.release.workflow_run,
  );
  assert.throws(
    () => validateInstallSmokeResult(pairedWorkflowRewrite),
    /process-local selected-asset identity|bind the exact/u,
  );
});

test("hosted release-contract jobs run the install-smoke suite", () => {
  const releaseWorkflow = fs.readFileSync(
    path.join(ROOT, ".github", "workflows", "release.yml"),
    "utf8",
  );
  const validationWorkflow = fs.readFileSync(
    path.join(ROOT, ".github", "workflows", "validation.yml"),
    "utf8",
  );
  assert.equal(
    releaseWorkflow.match(/scripts\/public-artifact-install-smoke\.test\.mjs/gu)?.length,
    1,
  );
  assert.equal(
    validationWorkflow.match(/scripts\/public-artifact-install-smoke\.test\.mjs/gu)?.length,
    2,
  );
});
