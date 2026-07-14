import {
  validateReleaseEvidencePacket,
  validateSanitizedReleaseEvidenceValue,
} from "./validate-release-evidence-packet.mjs";
import {
  INSTALL_SMOKE_DISPOSITIONS,
  INSTALL_SMOKE_SCHEMA_VERSION,
  installSmokeContractInternals,
} from "./install-smoke-contract.mjs";

const {
  CHECK_ORDER,
  EXECUTION_KINDS,
  PLAN_STEP_IDS,
  PLAN_STEP_ORDER,
  RELEASE_SIGNER_WORKFLOW,
  RELEASE_SOURCE_REF,
  RESULT_STATUSES,
  exactKeys,
  fail,
  slug,
  sortedObject,
  string,
  validateInstallSmokeIdentity,
} = installSmokeContractInternals;

const FIXTURE_OUTCOME = "Synthetic install-smoke fixture only.";
const FIXTURE_SIGNATURES = Object.freeze({
  appimage: Object.freeze({
    tauri_updater: Object.freeze({
      identity: "synthetic updater key fingerprint fixture",
      verified: true,
    }),
  }),
  deb: Object.freeze({}),
  dmg: Object.freeze({
    apple_notarization: Object.freeze({
      identity: "synthetic Apple notarization fixture",
      verified: true,
    }),
    apple_staple: Object.freeze({ identity: "synthetic stapled ticket fixture", verified: true }),
    contained_app_developer_id: Object.freeze({
      identity: "synthetic Developer ID signer fixture",
      verified: true,
    }),
    contained_app_notarization: Object.freeze({
      identity: "synthetic contained-app notarization fixture",
      verified: true,
    }),
    contained_app_staple: Object.freeze({
      identity: "synthetic contained-app stapled ticket fixture",
      verified: true,
    }),
    developer_id: Object.freeze({
      identity: "synthetic Developer ID signer fixture",
      verified: true,
    }),
  }),
  macos_updater: Object.freeze({
    contained_app_developer_id: Object.freeze({
      identity: "synthetic Developer ID signer fixture",
      verified: true,
    }),
    contained_app_notarization: Object.freeze({
      identity: "synthetic contained-app notarization fixture",
      verified: true,
    }),
    contained_app_staple: Object.freeze({
      identity: "synthetic contained-app stapled ticket fixture",
      verified: true,
    }),
    tauri_updater: Object.freeze({
      identity: "synthetic updater key fingerprint fixture",
      verified: true,
    }),
  }),
  nsis: Object.freeze({
    authenticode: Object.freeze({
      identity: "synthetic Authenticode signer fixture",
      verified: true,
    }),
    tauri_updater: Object.freeze({
      identity: "synthetic updater key fingerprint fixture",
      verified: true,
    }),
  }),
});
const LIMITATIONS = Object.freeze({
  deb_checksum_attestation_only: Object.freeze({
    disposition: "not_applicable",
    summary: "Debian trust uses the public checksum and source-bound attestation.",
  }),
  macos_updater_staging_only: Object.freeze({
    disposition: "not_applicable",
    summary: "The updater archive is staged only; no normal package installation is claimed.",
  }),
  synthetic_fixture_no_release_claim: Object.freeze({
    disposition: "not_applicable",
    summary: "Schema exercise only; this packet is not release evidence.",
  }),
  windows_service_etw_out_of_scope: Object.freeze({
    disposition: "not_applicable",
    summary: "Windows service and ETW behavior are outside this fixture contract.",
  }),
});

function responseSummary(value, field) {
  const summary = string(value, field, { max: 200 });
  validateSanitizedReleaseEvidenceValue(summary, field);
  return summary;
}

function canonical(value) {
  if (Array.isArray(value)) return value.map(canonical);
  if (value && typeof value === "object") {
    return Object.fromEntries(
      Object.entries(value)
        .sort(([left], [right]) => left.localeCompare(right))
        .map(([key, child]) => [key, canonical(child)]),
    );
  }
  return value;
}

function equal(left, right) {
  return JSON.stringify(canonical(left)) === JSON.stringify(canonical(right));
}

export function installSmokeResultDisposition(executionKind, steps) {
  if (executionKind === "plan") return "planned";
  if (steps.some(({ status }) => status === "partial")) return "partial";
  return "fixture";
}

function fixtureChecks() {
  return {
    cleanup: {
      application_removed: { status: "not_applicable", outcome: FIXTURE_OUTCOME },
      owned_runtime_cleanup: { status: "not_applicable", outcome: FIXTURE_OUTCOME },
      user_state_policy: { status: "not_applicable", outcome: FIXTURE_OUTCOME },
    },
    install: {
      anonymous_download: { status: "not_applicable", outcome: FIXTURE_OUTCOME },
      checksum: { status: "not_applicable", outcome: FIXTURE_OUTCOME },
      package_install: { status: "not_applicable", outcome: FIXTURE_OUTCOME },
    },
    runtime: {
      degradation: { status: "not_applicable", outcome: FIXTURE_OUTCOME },
      launch: { status: "not_applicable", outcome: FIXTURE_OUTCOME },
      release_identity: { status: "not_applicable", outcome: FIXTURE_OUTCOME },
      settings: { status: "not_applicable", outcome: FIXTURE_OUTCOME },
      telemetry: { status: "not_applicable", outcome: FIXTURE_OUTCOME },
    },
  };
}

function fixtureLimitations(profile) {
  const keys = ["synthetic_fixture_no_release_claim", ...profile.required_limitations];
  return sortedObject(keys.map((key) => [key, structuredClone(LIMITATIONS[key])]));
}

function evidenceRelease(release) {
  return {
    repository: release.repository,
    tag: release.tag,
    channel: release.channel,
    source_sha: release.source_sha,
    main_sha: release.main_sha,
    release_target_sha: release.release_target_sha,
    release_url: release.release_url,
    workflow_run: structuredClone(release.workflow_run),
  };
}

export function shapeInstallSmokeEvidence(plan, disposition) {
  if (plan.execution_kind === "plan" || disposition === "partial") return null;
  if (plan.execution_kind !== "fixture" || disposition !== "fixture") {
    fail(
      "result.evidence_packet",
      "release evidence is unreachable without a reviewed branded native executor",
    );
  }
  const signatures = FIXTURE_SIGNATURES[plan.platform.package.kind];
  if (!signatures) fail("result.platform.package.kind", "has no normalized fixture signatures");
  const packet = {
    schema_version: 1,
    packet_kind: "schema_fixture",
    packet_id: plan.plan_id,
    observed_at_utc: plan.observed_at_utc,
    release: evidenceRelease(plan.release),
    platform: structuredClone(plan.platform),
    assets: [
      {
        name: plan.asset.name,
        size_bytes: plan.asset.size_bytes,
        sha256: plan.asset.sha256,
        api_digest: plan.asset.sha256,
        public_url: plan.asset.public_url,
        attestation: {
          verified: true,
          repository: plan.release.repository,
          source_sha: plan.release.source_sha,
          source_ref: RELEASE_SOURCE_REF,
          signer_workflow: RELEASE_SIGNER_WORKFLOW,
        },
        signatures: structuredClone(signatures),
      },
    ],
    checks: fixtureChecks(),
    limitations: fixtureLimitations(plan.profile),
  };
  validateReleaseEvidencePacket(packet);
  return packet;
}

function validatePlannedStatuses(result) {
  if (result.execution_kind !== "plan") return;
  for (const [index, step] of result.steps.entries()) {
    const expected = index < 2 ? "passed" : "planned";
    if (step.status !== expected) fail(`result.steps.${step.id}.status`, `must equal ${expected}`);
  }
}

function validateFixtureStatuses(result) {
  if (result.execution_kind !== "fixture") return;
  for (const [index, step] of result.steps.entries()) {
    if (index < 2) {
      if (step.status !== "not_applicable") {
        fail(`result.steps.${step.id}.status`, "must equal not_applicable");
      }
    } else if (step.status === "planned" || step.status === "not_applicable") {
      fail(`result.steps.${step.id}.status`, "is not valid for fixture execution");
    }
  }
  const partialIndex = result.steps.findIndex(({ status }) => status === "partial");
  if (
    partialIndex >= 0 &&
    result.steps.slice(partialIndex + 1).some(({ status }) => status !== "blocked")
  ) {
    fail("result.steps", "all actions after an unsettled timeout must remain blocked");
  }
}

function validateProfile(result, contract) {
  exactKeys(result.profile, "result.profile", [
    "package_operation",
    "install_kind",
    "trust_basis",
    "required_limitations",
  ]);
  const expected = {
    package_operation: contract.packageOperation,
    install_kind: contract.installKind,
    trust_basis: contract.trustBasis,
    required_limitations: [...contract.requiredLimitations],
  };
  if (!equal(result.profile, expected))
    fail("result.profile", "must match the closed package profile");
}

export function validateInstallSmokeResult(result) {
  exactKeys(result, "result", [
    "schema_version",
    "plan_id",
    "execution_kind",
    "disposition",
    "observed_at_utc",
    "platform",
    "release",
    "asset",
    "identity_receipt",
    "profile",
    "steps",
    "evidence_packet",
  ]);
  if (result.schema_version !== INSTALL_SMOKE_SCHEMA_VERSION)
    fail("result.schema_version", "must equal 1");
  slug(result.plan_id, "result.plan_id");
  if (!EXECUTION_KINDS.has(result.execution_kind))
    fail("result.execution_kind", "is not supported");
  if (!INSTALL_SMOKE_DISPOSITIONS.includes(result.disposition))
    fail("result.disposition", "is not supported");
  const contract = validateInstallSmokeIdentity(result);
  validateProfile(result, contract);
  if (!Array.isArray(result.steps) || result.steps.length !== PLAN_STEP_IDS.size) {
    fail("result.steps", `must contain all ${PLAN_STEP_IDS.size} required gates`);
  }
  const expectedActions = [
    "verified_anonymous_download",
    "verified_public_checksum",
    "verify_package_trust",
    "rehash_public_asset",
    contract.prepareAction,
    "launch_app",
    "verify_release_identity",
    "restart_and_verify_settings",
    "probe_supported_degradation",
    "verify_telemetry",
    contract.removeAction,
    "verify_owned_runtime_cleanup",
    "verify_user_state_policy",
  ];
  const seen = new Set();
  for (const [index, step] of result.steps.entries()) {
    exactKeys(step, `result.steps[${index}]`, [
      "id",
      "group",
      "check_id",
      "action",
      "status",
      "outcome",
    ]);
    if (!PLAN_STEP_IDS.has(step.id) || step.id !== `${step.group}.${step.check_id}`) {
      fail(`result.steps[${index}].id`, "must identify one exact smoke gate");
    }
    if (seen.has(step.id)) fail(`result.steps[${index}].id`, "must not be duplicated");
    seen.add(step.id);
    if (step.id !== PLAN_STEP_ORDER[index])
      fail(`result.steps[${index}].id`, `must equal ordered gate ${PLAN_STEP_ORDER[index]}`);
    if (step.action !== expectedActions[index])
      fail(`result.steps[${index}].action`, `must equal ${expectedActions[index]}`);
    if (!RESULT_STATUSES.has(step.status))
      fail(`result.steps[${index}].status`, "is not supported");
    responseSummary(step.outcome, `result.steps[${index}].outcome`);
  }
  validatePlannedStatuses(result);
  validateFixtureStatuses(result);
  const derived = installSmokeResultDisposition(result.execution_kind, result.steps);
  if (result.disposition !== derived)
    fail("result.disposition", `must equal derived state ${derived}`);
  const expectedPacket = shapeInstallSmokeEvidence(result, derived);
  if (!equal(result.evidence_packet, expectedPacket)) {
    fail(
      "result.evidence_packet",
      "must exactly match the derived packet id, checks, outcomes, limitations, disposition, release, platform, and selected asset",
    );
  }
  if (result.evidence_packet !== null) validateReleaseEvidencePacket(result.evidence_packet);
  return result;
}

export function buildInstallSmokeResult(plan, steps) {
  const disposition = installSmokeResultDisposition(plan.execution_kind, steps);
  return validateInstallSmokeResult({
    schema_version: INSTALL_SMOKE_SCHEMA_VERSION,
    plan_id: plan.plan_id,
    execution_kind: plan.execution_kind,
    disposition,
    observed_at_utc: plan.observed_at_utc,
    platform: structuredClone(plan.platform),
    release: structuredClone(plan.release),
    asset: structuredClone(plan.asset),
    identity_receipt: plan.identity_receipt,
    profile: structuredClone(plan.profile),
    steps,
    evidence_packet: shapeInstallSmokeEvidence(plan, disposition),
  });
}
