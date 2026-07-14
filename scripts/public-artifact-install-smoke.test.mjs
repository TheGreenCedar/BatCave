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
  validateInstallSmokeResult,
} from "./public-artifact-install-smoke.mjs";
import { validateReleaseEvidencePacket } from "./validate-release-evidence-packet.mjs";
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
  dmg: "macOS universal DMG",
  macos_updater: "macOS universal updater payload",
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
const EXECUTOR = Object.freeze({
  tokenized_argv: true,
  shell: false,
  minimal_environment: true,
  bounded_output: true,
  timeout_tree_cleanup: true,
});

let nativeReceipt;
let publicRoot;

function readJson(file) {
  return JSON.parse(fs.readFileSync(file, "utf8"));
}

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
  nativeReceipt = result.receipt;
});

after(() => {
  fs.rmSync(publicRoot, { recursive: true, force: true });
});

function schemaFixture(name) {
  return readJson(path.join(FIXTURE_DIR, name));
}

function nativeTemplate(name) {
  const packet = schemaFixture(name);
  packet.packet_kind = "release_evidence";
  packet.packet_id = packet.packet_id.replace("schema-fixture", "install-smoke");
  packet.release.tag = TAG;
  packet.release.channel = "prerelease";
  packet.release.source_sha = SOURCE_SHA;
  packet.release.main_sha = SOURCE_SHA;
  packet.release.release_target_sha = SOURCE_SHA;
  packet.release.release_url = `https://github.com/${RELEASE_REPOSITORY}/releases/tag/${TAG}`;
  delete packet.limitations.synthetic_fixture_no_release_claim;
  if (packet.platform.package.kind === "deb") {
    packet.limitations.deb_checksum_attestation_only.disposition = "accepted";
  }

  const role = PACKAGE_ROLES[packet.platform.package.kind];
  const assetName = expectedReleaseAssetRoles(TAG).roles.find(
    (candidate) => candidate.role === role,
  ).name;
  const verified = nativeReceipt.assets.find(({ name: candidate }) => candidate === assetName);
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
      check.outcome = "Awaiting bounded native install smoke execution.";
    }
  }
  validateReleaseEvidencePacket(packet);
  return packet;
}

function fixtureReceipt(packet) {
  const asset = packet.assets.find(({ name }) => name === packet.platform.package.asset_name);
  return {
    schema_version: 1,
    verifier: "scripts/verify-public-release.mjs",
    disposition: "fixture",
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
  const packet = executionKind === "fixture" ? schemaFixture(name) : nativeTemplate(name);
  const scenario =
    packet.platform.os === "windows"
      ? "standard-access-visibility"
      : "permission-limited-telemetry";
  return {
    schema_version: 1,
    execution_kind: executionKind,
    app_version: executionKind === "fixture" ? "0.0.0-evidence.1" : "9.9.9-rc.1",
    evidence_template: packet,
    public_verification: executionKind === "fixture" ? fixtureReceipt(packet) : nativeReceipt,
    isolation: {
      scope_id: `${packet.platform.os}-${packet.platform.package.kind}-smoke`,
      install_root_id: "isolated-install-root",
      user_state_root_id: "isolated-user-state-root",
      user_state_policy: "preserve",
      step_timeout_ms: 1_000,
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
      signatures: structuredClone(plan.asset.expected_signatures),
      trust_basis: plan.asset.expected_trust_basis,
    },
    "install.package_install": { package_ready: true },
    "runtime.launch": { launched: true },
    "runtime.release_identity": {
      app_version: plan.release.app_version,
      source_commit_sha: plan.release.source_sha,
      install_kind:
        plan.platform.package.kind === "macos_updater" || plan.platform.package.kind === "dmg"
          ? "app_bundle"
          : plan.platform.package.kind,
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

function adapterFor(input, overrides = {}) {
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
    adapter: { kind: input.execution_kind, executor: structuredClone(EXECUTOR), actions },
  };
}

for (const [file, os, packageKind] of FIXTURES) {
  test(`plans and runs a non-claiming ${os} ${packageKind} fixture`, async () => {
    const input = inputFor(file);
    const { plan, calls, adapter } = adapterFor(input);
    const result = await runInstallSmoke(input, adapter);
    assert.equal(result.disposition, "fixture");
    assert.equal(result.execution_kind, "fixture");
    assert.equal(result.evidence_packet.packet_kind, "schema_fixture");
    assert.ok(
      Object.values(result.evidence_packet.checks).every((checks) =>
        Object.values(checks).every(({ status }) => status === "not_applicable"),
      ),
    );
    assert.deepEqual(
      calls,
      plan.steps.filter(({ source }) => source === "adapter").map(({ id }) => id),
    );
    assert.doesNotMatch(JSON.stringify(result), /(?:\/private\/|[A-Za-z]:\\)/u);
    if (packageKind === "macos_updater") {
      assert.equal(
        plan.steps.find(({ id }) => id === "install.package_install").action,
        "stage_updater_archive_app",
      );
    }
  });
}

test("a failing synthetic adapter remains fixture-only evidence", async () => {
  const input = inputFor("windows-nsis.json");
  const { adapter } = adapterFor(input, {
    "runtime.launch": () => ({
      status: "failed",
      outcome: "Synthetic launch failure.",
      observations: {},
    }),
  });
  const result = await runInstallSmoke(input, adapter);
  assert.equal(result.disposition, "fixture");
  assert.equal(result.evidence_packet.packet_kind, "schema_fixture");
  assert.ok(
    Object.values(result.evidence_packet.checks).every((checks) =>
      Object.values(checks).every(({ status }) => status === "not_applicable"),
    ),
  );
});

test("returns a pure plan with no evidence packet and invokes no adapter", async () => {
  const input = inputFor("linux-appimage.json", "plan");
  const result = await runInstallSmoke(input);
  assert.equal(result.disposition, "planned");
  assert.equal(result.evidence_packet, null);
  assert.equal(result.steps[0].status, "passed");
  assert.ok(result.steps.slice(2).every(({ status }) => status === "planned"));
});

test("maps a complete native adapter run to schema-valid release evidence", async () => {
  const input = inputFor("linux-appimage.json", "native");
  const { plan, calls, adapter } = adapterFor(input);
  const result = await runInstallSmoke(input, adapter);
  assert.equal(result.disposition, "native_proven");
  assert.equal(result.evidence_packet.packet_kind, "release_evidence");
  assert.equal(validateReleaseEvidencePacket(result.evidence_packet), result.evidence_packet);
  assert.ok(
    Object.values(result.evidence_packet.checks).every((checks) =>
      Object.values(checks).every(({ status }) => status === "passed"),
    ),
  );
  assert.ok(
    calls.indexOf("preflight.asset_rehash") < calls.indexOf("install.package_install") &&
      calls.indexOf("preflight.package_trust") < calls.indexOf("install.package_install"),
  );
  assert.equal(plan.constraints.windows_service_behavior_assumed, false);
});

test("rejects a caller-authored public-verifier pass before any adapter action", async () => {
  const input = inputFor("linux-appimage.json", "native");
  input.public_verification = structuredClone(nativeReceipt);
  let calls = 0;
  await assert.rejects(
    () =>
      runInstallSmoke(input, {
        kind: "native",
        executor: structuredClone(EXECUTOR),
        actions: new Proxy(
          {},
          {
            get: () => async () => {
              calls += 1;
            },
          },
        ),
      }),
    /must come from a successful in-process verifyPublicRelease call/u,
  );
  assert.equal(calls, 0);
});

test("rejects a missing adapter action before package mutation", async () => {
  const input = inputFor("linux-deb.json", "native");
  const { adapter, calls } = adapterFor(input);
  delete adapter.actions.install_deb;
  await assert.rejects(() => runInstallSmoke(input, adapter), /install_deb.*explicit action/u);
  assert.deepEqual(calls, []);
});

test("rejects inherited or accessor adapter actions before package mutation", async () => {
  const input = inputFor("linux-deb.json", "native");
  const inherited = adapterFor(input);
  const install = inherited.adapter.actions.install_deb;
  delete inherited.adapter.actions.install_deb;
  Object.setPrototypeOf(inherited.adapter.actions, { install_deb: install });
  await assert.rejects(
    () => runInstallSmoke(input, inherited.adapter),
    /install_deb.*explicit action/u,
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
    /install_deb.*explicit action/u,
  );
  assert.deepEqual(accessor.calls, []);
});

test("rejects an unsafe executor contract before package mutation", async () => {
  const input = inputFor("linux-deb.json", "native");
  const { adapter, calls } = adapterFor(input);
  adapter.executor.shell = true;
  await assert.rejects(() => runInstallSmoke(input, adapter), /bounded non-shell executor/u);
  assert.deepEqual(calls, []);
});

test("fails closed on rehash mismatch or a symlink before install", async () => {
  const input = inputFor("linux-appimage.json", "native");
  const { adapter, calls } = adapterFor(input, {
    "preflight.asset_rehash": (_request, plan) => ({
      status: "passed",
      outcome: "Asset rehash attempted.",
      observations: {
        sha256: plan.asset.sha256,
        regular_file: true,
        symlink: true,
        contained: true,
      },
    }),
  });
  const result = await runInstallSmoke(input, adapter);
  assert.equal(result.disposition, "failed");
  assert.equal(result.steps.find(({ id }) => id === "preflight.asset_rehash").status, "failed");
  assert.ok(!calls.includes("install.package_install"));
  assert.equal(result.evidence_packet.checks.install.package_install.status, "blocked");
});

test("fails closed on digest drift or adapter-root escape before install", async () => {
  for (const observations of [
    {
      sha256: `sha256:${"f".repeat(64)}`,
      regular_file: true,
      symlink: false,
      contained: true,
    },
    {
      sha256: nativeReceipt.assets.find(({ name }) => name.endsWith(".AppImage")).sha256,
      regular_file: true,
      symlink: false,
      contained: false,
    },
  ]) {
    const input = inputFor("linux-appimage.json", "native");
    const { adapter, calls } = adapterFor(input, {
      "preflight.asset_rehash": () => ({
        status: "passed",
        outcome: "Asset containment probe completed.",
        observations,
      }),
    });
    const result = await runInstallSmoke(input, adapter);
    assert.equal(result.disposition, "failed");
    assert.ok(!calls.includes("install.package_install"));
  }
});

test("fails closed when observed package trust differs from the evidence template", async () => {
  const input = inputFor("windows-nsis.json", "native");
  const { adapter, calls } = adapterFor(input, {
    "preflight.package_trust": (_request, plan) => ({
      status: "passed",
      outcome: "Package trust probe completed.",
      observations: {
        signatures: {},
        trust_basis: plan.asset.expected_trust_basis,
      },
    }),
  });
  const result = await runInstallSmoke(input, adapter);
  assert.equal(result.disposition, "failed");
  assert.ok(!calls.includes("install.package_install"));
  assert.equal(result.evidence_packet.checks.install.package_install.status, "blocked");
});

test("keeps timeout distinct, aborts the action, and still runs bounded cleanup", async () => {
  const input = inputFor("linux-deb.json", "native");
  input.isolation.step_timeout_ms = 5;
  let aborted = false;
  const { adapter, calls } = adapterFor(input, {
    "install.package_install": ({ signal }) =>
      new Promise(() => {
        signal.addEventListener("abort", () => {
          aborted = true;
        });
      }),
  });
  const result = await runInstallSmoke(input, adapter);
  assert.equal(result.disposition, "failed");
  assert.equal(result.steps.find(({ id }) => id === "install.package_install").status, "timeout");
  assert.equal(aborted, true);
  assert.ok(calls.includes("cleanup.application_removed"));
  assert.equal(result.evidence_packet.checks.install.package_install.status, "failed");
});

test("keeps unsupported and downstream blocked states distinct", async () => {
  const input = inputFor("macos-dmg.json", "native");
  const { adapter } = adapterFor(input, {
    "install.package_install": () => ({
      status: "unsupported",
      outcome: "Native package preparation capability is unavailable.",
      observations: {},
    }),
  });
  const result = await runInstallSmoke(input, adapter);
  assert.equal(result.disposition, "skipped");
  assert.equal(
    result.steps.find(({ id }) => id === "install.package_install").status,
    "unsupported",
  );
  assert.equal(result.steps.find(({ id }) => id === "runtime.launch").status, "blocked");
  assert.equal(result.evidence_packet.checks.install.package_install.status, "blocked");
});

test("keeps an explicit adapter skip distinct from a failure", async () => {
  const input = inputFor("macos-dmg.json", "native");
  const { adapter } = adapterFor(input, {
    "install.package_install": () => ({
      status: "skipped",
      outcome: "Required native package preparation was explicitly skipped.",
      observations: {},
    }),
  });
  const result = await runInstallSmoke(input, adapter);
  assert.equal(result.disposition, "skipped");
  assert.equal(result.steps.find(({ id }) => id === "install.package_install").status, "skipped");
  assert.equal(result.evidence_packet.checks.install.package_install.status, "blocked");
});

test("fails exact runtime identity mismatch without trusting adapter prose", async () => {
  const input = inputFor("linux-appimage.json", "native");
  const { adapter } = adapterFor(input, {
    "runtime.release_identity": (_request, plan) => ({
      status: "passed",
      outcome: "Runtime reported an identity.",
      observations: {
        app_version: "9.9.8",
        source_commit_sha: plan.release.source_sha,
        install_kind: "appimage",
      },
    }),
  });
  const result = await runInstallSmoke(input, adapter);
  assert.equal(result.disposition, "failed");
  assert.equal(result.evidence_packet.checks.runtime.release_identity.status, "failed");
  assert.equal(result.evidence_packet.checks.runtime.settings.status, "blocked");
});

test("does not let unavailable telemetry satisfy native proof", async () => {
  const input = inputFor("linux-appimage.json", "native");
  const { adapter } = adapterFor(input, {
    "runtime.telemetry": () => ({
      status: "passed",
      outcome: "Telemetry was unavailable.",
      observations: { sample_observed: true, quality_state: "unavailable" },
    }),
  });
  const result = await runInstallSmoke(input, adapter);
  assert.equal(result.disposition, "failed");
  assert.equal(result.evidence_packet.checks.runtime.telemetry.status, "failed");
});

test("fails a partial adapter response and rejects unsafe output from evidence", async () => {
  const partialInput = inputFor("linux-appimage.json", "native");
  const partial = adapterFor(partialInput, {
    "runtime.launch": () => ({ status: "passed", outcome: "Launch returned." }),
  });
  const partialResult = await runInstallSmoke(partialInput, partial.adapter);
  assert.equal(partialResult.disposition, "failed");
  assert.equal(partialResult.evidence_packet.checks.runtime.launch.status, "failed");

  const unsafeInput = inputFor("linux-appimage.json", "native");
  const unsafe = adapterFor(unsafeInput, {
    "preflight.asset_rehash": (_request, plan) => ({
      status: "passed",
      outcome: "Evidence saved at /private/tmp/release.log",
      observations: {
        sha256: plan.asset.sha256,
        regular_file: true,
        symlink: false,
        contained: true,
      },
    }),
  });
  const unsafeResult = await runInstallSmoke(unsafeInput, unsafe.adapter);
  assert.equal(unsafeResult.disposition, "failed");
  assert.doesNotMatch(JSON.stringify(unsafeResult), /\/private\/tmp/u);
});

test("fails unexpected owned residue and keeps user-state policy separately visible", async () => {
  const input = inputFor("macos-updater.json", "native");
  const { adapter } = adapterFor(input, {
    "cleanup.owned_runtime_cleanup": () => ({
      status: "passed",
      outcome: "Cleanup probe found residue.",
      observations: { residue_count: 1 },
    }),
  });
  const result = await runInstallSmoke(input, adapter);
  assert.equal(result.disposition, "failed");
  assert.equal(result.evidence_packet.checks.cleanup.owned_runtime_cleanup.status, "failed");
  assert.equal(result.evidence_packet.checks.cleanup.user_state_policy.status, "passed");
});

test("rejects duplicate, out-of-order, and contradictory result states", async () => {
  const input = inputFor("linux-appimage.json");
  const { adapter } = adapterFor(input);
  const result = await runInstallSmoke(input, adapter);

  const duplicate = structuredClone(result);
  duplicate.steps[3] = structuredClone(duplicate.steps[2]);
  assert.throws(() => validateInstallSmokeResult(duplicate), /duplicated|ordered gate/u);

  const reordered = structuredClone(result);
  [reordered.steps[2], reordered.steps[3]] = [reordered.steps[3], reordered.steps[2]];
  assert.throws(() => validateInstallSmokeResult(reordered), /ordered gate/u);

  const contradiction = structuredClone(result);
  contradiction.disposition = "native_proven";
  assert.throws(
    () => validateInstallSmokeResult(contradiction),
    /native_proven requires every gate to pass|derived state fixture/u,
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
