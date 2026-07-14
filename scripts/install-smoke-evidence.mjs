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
  RESULT_STATUSES,
  exactKeys,
  fail,
  platformContract,
  slug,
  sortedObject,
  string,
} = installSmokeContractInternals;

function responseSummary(value, field) {
  const summary = string(value, field, { max: 200 });
  validateSanitizedReleaseEvidenceValue(summary, field);
  return summary;
}

export function installSmokeResultDisposition(executionKind, steps) {
  if (executionKind === "plan") return "planned";
  if (executionKind === "fixture") return "fixture";
  const statuses = new Set(steps.map(({ status }) => status));
  if (statuses.has("failed") || statuses.has("timeout")) return "failed";
  if (statuses.has("unsupported") || statuses.has("skipped") || statuses.has("blocked")) {
    return "skipped";
  }
  return "native_proven";
}

function evidenceStatus(status) {
  if (status === "passed") return "passed";
  if (status === "failed" || status === "timeout") return "failed";
  if (status === "not_applicable") return "not_applicable";
  return "blocked";
}

function evidenceLimitations(template, disposition) {
  const limitations = structuredClone(template.limitations);
  const additions = {
    skipped: [
      "install_smoke_required_step_skipped",
      { disposition: "blocked", summary: "One or more required native smoke steps did not run." },
    ],
    failed: [
      "install_smoke_failed",
      { disposition: "blocked", summary: "One or more required native smoke steps failed." },
    ],
  };
  if (additions[disposition]) limitations[additions[disposition][0]] = additions[disposition][1];
  return sortedObject(Object.entries(limitations));
}

export function shapeInstallSmokeEvidence(template, executionKind, disposition, steps) {
  if (executionKind === "plan") return null;
  const packet = structuredClone(template);
  if (executionKind === "fixture") {
    validateReleaseEvidencePacket(packet);
    return packet;
  }
  const byId = new Map(steps.map((step) => [step.id, step]));
  for (const [group, id] of CHECK_ORDER) {
    const step = byId.get(`${group}.${id}`);
    packet.checks[group][id] = {
      status: evidenceStatus(step.status),
      outcome: step.outcome,
    };
  }
  packet.limitations = evidenceLimitations(packet, disposition);
  validateReleaseEvidencePacket(packet);
  return packet;
}

function validatePlannedStatuses(result) {
  if (result.execution_kind !== "plan") return;
  for (const step of result.steps) {
    const expected =
      step.id.startsWith("install.") && step.action.startsWith("verified_") ? "passed" : "planned";
    if (step.status !== expected) fail(`result.steps.${step.id}.status`, `must equal ${expected}`);
  }
}

function validateExecutionStatuses(result) {
  if (result.execution_kind === "plan") return;
  for (const [index, step] of result.steps.entries()) {
    if (index < 2) {
      const expected = result.execution_kind === "fixture" ? "not_applicable" : "passed";
      if (step.status !== expected)
        fail(`result.steps.${step.id}.status`, `must equal ${expected}`);
      continue;
    }
    if (step.status === "planned" || step.status === "not_applicable") {
      fail(`result.steps.${step.id}.status`, `is not valid for ${result.execution_kind} execution`);
    }
  }
  if (
    result.execution_kind === "native" &&
    result.disposition === "native_proven" &&
    result.steps.some(({ status }) => status !== "passed")
  ) {
    fail("result.disposition", "native_proven requires every gate to pass");
  }
}

export function validateInstallSmokeResult(result) {
  exactKeys(result, "result", [
    "schema_version",
    "plan_id",
    "execution_kind",
    "disposition",
    "platform",
    "release",
    "steps",
    "evidence_packet",
  ]);
  if (result.schema_version !== INSTALL_SMOKE_SCHEMA_VERSION)
    fail("result.schema_version", "must equal 1");
  slug(result.plan_id, "result.plan_id");
  if (!EXECUTION_KINDS.has(result.execution_kind))
    fail("result.execution_kind", "is not supported");
  if (!INSTALL_SMOKE_DISPOSITIONS.includes(result.disposition)) {
    fail("result.disposition", "is not supported");
  }
  if (!Array.isArray(result.steps) || result.steps.length !== PLAN_STEP_IDS.size) {
    fail("result.steps", `must contain all ${PLAN_STEP_IDS.size} required gates`);
  }
  const contract = platformContract(result.platform);
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
    if (step.id !== PLAN_STEP_ORDER[index]) {
      fail(`result.steps[${index}].id`, `must equal ordered gate ${PLAN_STEP_ORDER[index]}`);
    }
    if (step.action !== expectedActions[index]) {
      fail(`result.steps[${index}].action`, `must equal ${expectedActions[index]}`);
    }
    if (!RESULT_STATUSES.has(step.status))
      fail(`result.steps[${index}].status`, "is not supported");
    responseSummary(step.outcome, `result.steps[${index}].outcome`);
  }
  validatePlannedStatuses(result);
  validateExecutionStatuses(result);
  if (result.disposition === "planned") {
    if (result.evidence_packet !== null) fail("result.evidence_packet", "must be null for a plan");
  } else {
    validateReleaseEvidencePacket(result.evidence_packet);
    const expectedKind =
      result.execution_kind === "fixture" ? "schema_fixture" : "release_evidence";
    if (result.evidence_packet.packet_kind !== expectedKind) {
      fail("result.evidence_packet.packet_kind", `must equal ${expectedKind}`);
    }
    if (
      result.evidence_packet.release.tag !== result.release.tag ||
      result.evidence_packet.release.source_sha !== result.release.source_sha ||
      JSON.stringify(result.evidence_packet.platform) !== JSON.stringify(result.platform)
    ) {
      fail("result.evidence_packet", "must match the result release and platform identity");
    }
  }
  const derived = installSmokeResultDisposition(result.execution_kind, result.steps);
  if (result.disposition !== derived)
    fail("result.disposition", `must equal derived state ${derived}`);
  return result;
}

export function buildInstallSmokeResult(plan, template, steps) {
  const disposition = installSmokeResultDisposition(plan.execution_kind, steps);
  return validateInstallSmokeResult({
    schema_version: INSTALL_SMOKE_SCHEMA_VERSION,
    plan_id: plan.plan_id,
    execution_kind: plan.execution_kind,
    disposition,
    platform: structuredClone(plan.platform),
    release: structuredClone(plan.release),
    steps,
    evidence_packet: shapeInstallSmokeEvidence(template, plan.execution_kind, disposition, steps),
  });
}
