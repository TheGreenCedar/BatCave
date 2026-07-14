import {
  NativeArtifactCapabilityCleanupError,
  closeNativeArtifactCapability,
  acquireNativeArtifactCapability,
  requireOwnedNativeArtifactVerificationReceipt,
  verifyOwnedNativeArtifactCapability,
} from "./native-artifact-capability.mjs";
import {
  INSTALL_SMOKE_SCHEMA_VERSION,
  installSmokeContractInternals,
  validateInstallSmokePlan,
} from "./install-smoke-contract.mjs";
import {
  validateReleaseEvidencePacket,
  validateSanitizedReleaseEvidenceValue,
} from "./validate-release-evidence-packet.mjs";
import {
  bindMacosNativeAdapterSource,
  requireMacosNativeAdapterSourceReceipt,
} from "./macos-native-install-smoke-adapter.mjs";

export const NATIVE_INSTALL_SMOKE_DISPOSITIONS = Object.freeze([
  "skipped",
  "failed",
  "native_proven",
]);

const {
  PLAN_STEP_IDS,
  PLAN_STEP_ORDER,
  exactKeys,
  fail,
  platformContract,
  string,
  validateInstallSmokeIdentity,
} = installSmokeContractInternals;
const nativeResults = new WeakSet();
const nativeExecutionReceipts = new WeakSet();
const RESULT_STATUSES = new Set([
  "passed",
  "failed",
  "timeout",
  "partial",
  "unsupported",
  "skipped",
  "blocked",
]);

function deepFreeze(value) {
  if (value && typeof value === "object" && !Object.isFrozen(value)) {
    Object.freeze(value);
    for (const child of Object.values(value)) deepFreeze(child);
  }
  return value;
}

function responseSummary(value, field) {
  const summary = string(value, field, { max: 200 });
  validateSanitizedReleaseEvidenceValue(summary, field);
  return summary;
}

function expectedActions(platform) {
  const contract = platformContract(platform);
  return [
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
}

function fixedStep(step, status, outcome) {
  return {
    id: step.id,
    group: step.group,
    check_id: step.check_id,
    action: step.action,
    status,
    outcome,
  };
}

function prerequisiteSteps(plan) {
  return plan.steps
    .slice(0, 2)
    .map((step) =>
      fixedStep(
        step,
        "passed",
        step.check_id === "anonymous_download"
          ? "Contract-only anonymous public verification passed."
          : "Contract-only public checksum verification passed.",
      ),
    );
}

function unavailableSteps(plan, capabilityFailed, macosSourceReceipt = null) {
  const steps = prerequisiteSteps(plan);
  for (const step of plan.steps.slice(2)) {
    if (capabilityFailed && step.id === "preflight.asset_rehash") {
      steps.push(
        fixedStep(step, "failed", "Selected artifact capability acquisition failed closed."),
      );
    } else if (macosSourceReceipt && step.id === "preflight.asset_rehash") {
      steps.push(
        fixedStep(
          step,
          "passed",
          "Verified asset identity is bound to the macOS adapter source descriptor.",
        ),
      );
    } else if (!capabilityFailed && step.id === "preflight.package_trust") {
      steps.push(
        fixedStep(
          step,
          "unsupported",
          macosSourceReceipt
            ? "Signed destination trust remains blocked on exact native macOS execution."
            : "No reviewed native platform adapter is registered in this source slice.",
        ),
      );
    } else {
      steps.push(
        fixedStep(
          step,
          "blocked",
          capabilityFailed
            ? "Native action blocked by artifact capability acquisition failure."
            : "Native action blocked because the reviewed platform adapter is unavailable.",
        ),
      );
    }
  }
  return steps;
}

function withCapabilityCleanupFailure(steps) {
  return steps.map((step) =>
    step.id === "cleanup.owned_runtime_cleanup"
      ? {
          ...step,
          status: "failed",
          outcome: "Owned artifact capability cleanup failed closed.",
        }
      : step,
  );
}

function derivedDisposition(steps) {
  const statuses = new Set(steps.map(({ status }) => status));
  if (statuses.has("failed") || statuses.has("timeout") || statuses.has("partial")) return "failed";
  if (statuses.has("unsupported") || statuses.has("skipped") || statuses.has("blocked")) {
    return "skipped";
  }
  return "native_proven";
}

function evidenceRelease(release) {
  const { app_version: _appVersion, ...packetRelease } = release;
  return structuredClone(packetRelease);
}

function buildNativeEvidence(result) {
  if (result.disposition !== "native_proven") return null;
  const receipt = result.native_execution_receipt;
  if (!receipt || !nativeExecutionReceipts.has(receipt)) {
    fail(
      "native_result.native_execution_receipt",
      "native_proven requires a process-local closed-adapter execution receipt",
    );
  }
  const byId = new Map(result.steps.map((step) => [step.id, step]));
  const check = (group, id) => ({
    status: "passed",
    outcome: byId.get(`${group}.${id}`).outcome,
  });
  const checks = {
    cleanup: {
      application_removed: check("cleanup", "application_removed"),
      owned_runtime_cleanup: check("cleanup", "owned_runtime_cleanup"),
      user_state_policy: check("cleanup", "user_state_policy"),
    },
    install: {
      anonymous_download: check("install", "anonymous_download"),
      checksum: check("install", "checksum"),
      package_install: check("install", "package_install"),
    },
    runtime: {
      degradation: check("runtime", "degradation"),
      launch: check("runtime", "launch"),
      release_identity: check("runtime", "release_identity"),
      settings: check("runtime", "settings"),
      telemetry: check("runtime", "telemetry"),
    },
  };
  const packet = {
    schema_version: 1,
    packet_kind: "release_evidence",
    packet_id: result.plan_id,
    observed_at_utc: result.observed_at_utc,
    release: evidenceRelease(result.release),
    platform: structuredClone(result.platform),
    assets: [structuredClone(receipt.asset_evidence)],
    checks,
    limitations: structuredClone(receipt.limitations),
  };
  validateReleaseEvidencePacket(packet);
  return packet;
}

function validateSteps(result) {
  if (!Array.isArray(result.steps) || result.steps.length !== PLAN_STEP_IDS.size) {
    fail("native_result.steps", `must contain all ${PLAN_STEP_IDS.size} required gates`);
  }
  const actions = expectedActions(result.platform);
  const seen = new Set();
  for (const [index, step] of result.steps.entries()) {
    exactKeys(step, `native_result.steps[${index}]`, [
      "id",
      "group",
      "check_id",
      "action",
      "status",
      "outcome",
    ]);
    if (step.id !== PLAN_STEP_ORDER[index] || step.id !== `${step.group}.${step.check_id}`) {
      fail(`native_result.steps[${index}].id`, `must equal ordered gate ${PLAN_STEP_ORDER[index]}`);
    }
    if (seen.has(step.id)) fail(`native_result.steps[${index}].id`, "must not be duplicated");
    seen.add(step.id);
    if (step.action !== actions[index]) {
      fail(`native_result.steps[${index}].action`, `must equal ${actions[index]}`);
    }
    if (!RESULT_STATUSES.has(step.status)) {
      fail(`native_result.steps[${index}].status`, "is not supported");
    }
    responseSummary(step.outcome, `native_result.steps[${index}].outcome`);
  }
}

export function validateNativeInstallSmokeResult(result) {
  exactKeys(result, "native_result", [
    "schema_version",
    "executor",
    "plan_id",
    "disposition",
    "observed_at_utc",
    "release",
    "platform",
    "asset",
    "identity_receipt",
    "profile",
    "steps",
    "artifact_verification_receipt",
    "native_execution_receipt",
    "evidence_packet",
  ]);
  if (result.schema_version !== INSTALL_SMOKE_SCHEMA_VERSION) {
    fail("native_result.schema_version", `must equal ${INSTALL_SMOKE_SCHEMA_VERSION}`);
  }
  if (result.executor !== "scripts/native-install-smoke-executor.mjs") {
    fail("native_result.executor", "is not supported");
  }
  validateInstallSmokeIdentity(result);
  const contract = platformContract(result.platform);
  const expectedProfile = {
    package_operation: contract.packageOperation,
    install_kind: contract.installKind,
    trust_basis: contract.trustBasis,
    required_limitations: [...contract.requiredLimitations],
  };
  if (JSON.stringify(result.profile) !== JSON.stringify(expectedProfile)) {
    fail("native_result.profile", "must match the closed platform profile");
  }
  if (!NATIVE_INSTALL_SMOKE_DISPOSITIONS.includes(result.disposition)) {
    fail("native_result.disposition", "is not supported");
  }
  validateSteps(result);
  const derived = derivedDisposition(result.steps);
  if (result.disposition !== derived) {
    fail("native_result.disposition", `must equal derived state ${derived}`);
  }
  if (result.artifact_verification_receipt !== null) {
    requireOwnedNativeArtifactVerificationReceipt(result.artifact_verification_receipt, result);
  }
  if (result.disposition === "native_proven") {
    if (
      !result.native_execution_receipt ||
      !nativeExecutionReceipts.has(result.native_execution_receipt)
    ) {
      fail(
        "native_result.native_execution_receipt",
        "native_proven requires a process-local closed-adapter execution receipt",
      );
    }
    if (result.steps.some(({ status }) => status !== "passed")) {
      fail("native_result.disposition", "native_proven requires every ordered gate to pass");
    }
  } else if (result.native_execution_receipt !== null) {
    fail("native_result.native_execution_receipt", "must be null unless native_proven");
  }
  const expectedEvidence = buildNativeEvidence(result);
  if (JSON.stringify(result.evidence_packet) !== JSON.stringify(expectedEvidence)) {
    fail("native_result.evidence_packet", "must exactly equal the executor-derived #98 packet");
  }
  if (!nativeResults.has(result)) {
    fail("native_result", "must be created by the production native executor");
  }
  return result;
}

function buildSourceSliceResult(plan, steps, artifactReceipt) {
  const result = {
    schema_version: INSTALL_SMOKE_SCHEMA_VERSION,
    executor: "scripts/native-install-smoke-executor.mjs",
    plan_id: plan.plan_id,
    disposition: derivedDisposition(steps),
    observed_at_utc: plan.observed_at_utc,
    release: structuredClone(plan.release),
    platform: structuredClone(plan.platform),
    asset: structuredClone(plan.asset),
    identity_receipt: plan.identity_receipt,
    profile: structuredClone(plan.profile),
    steps,
    artifact_verification_receipt: artifactReceipt,
    native_execution_receipt: null,
    evidence_packet: null,
  };
  const frozenResult = deepFreeze(result);
  nativeResults.add(frozenResult);
  return validateNativeInstallSmokeResult(frozenResult);
}

export async function runNativeInstallSmokeSourceSlice(plan, options) {
  validateInstallSmokePlan(plan);
  if (plan.execution_kind !== "plan") {
    fail("native_executor.plan", "must be a contract-only plan; fixtures cannot execute natively");
  }
  exactKeys(options, "native_executor.options", ["verified_root"]);
  let capability;
  let artifactReceipt = null;
  let macosSourceReceipt = null;
  let steps;
  try {
    capability = await acquireNativeArtifactCapability(plan, options);
    artifactReceipt = await verifyOwnedNativeArtifactCapability(capability);
    if (plan.platform.os === "macos") {
      macosSourceReceipt = bindMacosNativeAdapterSource(plan, artifactReceipt);
      requireMacosNativeAdapterSourceReceipt(macosSourceReceipt, plan, artifactReceipt);
    }
    steps = unavailableSteps(plan, false, macosSourceReceipt);
  } catch (error) {
    steps = unavailableSteps(plan, true);
    if (error instanceof NativeArtifactCapabilityCleanupError) {
      steps = withCapabilityCleanupFailure(steps);
    }
    artifactReceipt = null;
  }
  if (capability) {
    try {
      await closeNativeArtifactCapability(capability);
    } catch {
      steps = withCapabilityCleanupFailure(steps);
    }
  }
  return buildSourceSliceResult(plan, steps, artifactReceipt);
}
