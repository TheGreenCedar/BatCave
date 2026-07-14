import {
  createInstallSmokePlan,
  installSmokeContractInternals,
  validateInstallSmokePlan,
} from "./install-smoke-contract.mjs";
import { buildInstallSmokeResult, validateInstallSmokeResult } from "./install-smoke-evidence.mjs";
import { validateSanitizedReleaseEvidenceValue } from "./validate-release-evidence-packet.mjs";

export {
  INSTALL_SMOKE_DISPOSITIONS,
  INSTALL_SMOKE_SCHEMA_VERSION,
  createInstallSmokePlan,
  validateInstallSmokePlan,
} from "./install-smoke-contract.mjs";
export {
  shapeInstallSmokeEvidence,
  validateInstallSmokeResult,
} from "./install-smoke-evidence.mjs";

const {
  ADAPTER_STATUSES,
  QUALITY_STATES,
  deepFreeze,
  exactKeys,
  fail,
  object,
  platformContract,
  string,
  validateInput,
} = installSmokeContractInternals;

function validateAdapter(adapter, plan) {
  exactKeys(adapter, "adapter", ["kind", "executor", "actions"]);
  const expectedKind = plan.execution_kind;
  if (adapter.kind !== expectedKind || !new Set(["fixture", "native"]).has(adapter.kind)) {
    fail("adapter.kind", `must equal ${expectedKind}`);
  }
  object(adapter.actions, "adapter.actions");
  exactKeys(adapter.executor, "adapter.executor", [
    "tokenized_argv",
    "shell",
    "minimal_environment",
    "bounded_output",
    "timeout_tree_cleanup",
  ]);
  if (
    adapter.executor.tokenized_argv !== true ||
    adapter.executor.shell !== false ||
    adapter.executor.minimal_environment !== true ||
    adapter.executor.bounded_output !== true ||
    adapter.executor.timeout_tree_cleanup !== true
  ) {
    fail("adapter.executor", "must enforce the bounded non-shell executor contract");
  }
  const requiredActions = new Set(
    plan.steps.filter(({ source }) => source === "adapter").map(({ action }) => action),
  );
  const validatedActions = Object.create(null);
  for (const action of requiredActions) {
    const descriptor = Object.getOwnPropertyDescriptor(adapter.actions, action);
    if (!descriptor || typeof descriptor.value !== "function") {
      fail(
        `adapter.actions.${action}`,
        "must be an explicit action function before execution starts",
      );
    }
    validatedActions[action] = descriptor.value;
  }
  const unexpected = Object.keys(adapter.actions).filter((action) => !requiredActions.has(action));
  if (unexpected.length) fail(`adapter.actions.${unexpected[0]}`, "is not used by this plan");
  return Object.freeze(validatedActions);
}

function adapterContext(plan) {
  return deepFreeze({
    release: structuredClone(plan.release),
    platform: structuredClone(plan.platform),
    asset: structuredClone(plan.asset),
    isolation: structuredClone(plan.isolation),
    constraints: structuredClone(plan.constraints),
  });
}

function responseSummary(value, field) {
  const summary = string(value, field, { max: 200 });
  validateSanitizedReleaseEvidenceValue(summary, field);
  return summary;
}

function emptyObservations(observations, field) {
  exactKeys(observations, field, []);
}

function validatePassedObservations(step, observations, plan) {
  const field = `adapter.${step.action}.observations`;
  switch (step.id) {
    case "preflight.asset_rehash":
      exactKeys(observations, field, ["sha256", "regular_file", "symlink", "contained"]);
      if (
        observations.sha256 !== plan.asset.sha256 ||
        observations.regular_file !== true ||
        observations.symlink !== false ||
        observations.contained !== true
      ) {
        fail(field, "must prove the exact regular asset bytes remain inside the adapter root");
      }
      break;
    case "preflight.package_trust":
      exactKeys(observations, field, ["signatures", "trust_basis"]);
      if (
        JSON.stringify(observations.signatures) !== JSON.stringify(plan.asset.expected_signatures)
      ) {
        fail(
          `${field}.signatures`,
          "must match the package trust identities in the evidence template",
        );
      }
      if (observations.trust_basis !== plan.asset.expected_trust_basis) {
        fail(`${field}.trust_basis`, "must match the closed package trust path");
      }
      break;
    case "install.package_install":
      exactKeys(observations, field, ["package_ready"]);
      if (observations.package_ready !== true) fail(`${field}.package_ready`, "must be true");
      break;
    case "runtime.launch":
      exactKeys(observations, field, ["launched"]);
      if (observations.launched !== true) fail(`${field}.launched`, "must be true");
      break;
    case "runtime.release_identity": {
      exactKeys(observations, field, ["app_version", "source_commit_sha", "install_kind"]);
      const contract = platformContract(plan.platform);
      if (
        observations.app_version !== plan.release.app_version ||
        observations.source_commit_sha !== plan.release.source_sha ||
        observations.install_kind !== contract.installKind
      ) {
        fail(field, "must match the exact verified release and installed package kind");
      }
      break;
    }
    case "runtime.settings":
      exactKeys(observations, field, ["restarted", "preserved"]);
      if (observations.restarted !== true) fail(`${field}.restarted`, "must be true");
      if (observations.preserved !== true) fail(`${field}.preserved`, "must be true");
      break;
    case "runtime.degradation":
      exactKeys(observations, field, ["reported", "windows_service_behavior"]);
      if (observations.reported !== true) fail(`${field}.reported`, "must be true");
      if (observations.windows_service_behavior !== "not_assumed") {
        fail(`${field}.windows_service_behavior`, "must be not_assumed");
      }
      break;
    case "runtime.telemetry":
      exactKeys(observations, field, ["sample_observed", "quality_state"]);
      if (observations.sample_observed !== true) fail(`${field}.sample_observed`, "must be true");
      if (!QUALITY_STATES.has(observations.quality_state)) {
        fail(`${field}.quality_state`, "must be native or limited; unavailable cannot pass");
      }
      break;
    case "cleanup.application_removed":
      exactKeys(observations, field, ["removed"]);
      if (observations.removed !== true) fail(`${field}.removed`, "must be true");
      break;
    case "cleanup.owned_runtime_cleanup":
      exactKeys(observations, field, ["residue_count"]);
      if (observations.residue_count !== 0) fail(`${field}.residue_count`, "must equal 0");
      break;
    case "cleanup.user_state_policy": {
      exactKeys(observations, field, ["policy"]);
      const expected = plan.isolation.user_state_policy === "preserve" ? "preserved" : "removed";
      if (observations.policy !== expected) fail(`${field}.policy`, `must equal ${expected}`);
      break;
    }
    default:
      fail("adapter", `has no observation contract for ${step.id}`);
  }
}

function validateAdapterResponse(response, step, plan) {
  exactKeys(response, `adapter.${step.action}`, ["status", "outcome", "observations"]);
  if (!ADAPTER_STATUSES.has(response.status)) {
    fail(`adapter.${step.action}.status`, "is not supported");
  }
  const outcome = responseSummary(response.outcome, `adapter.${step.action}.outcome`);
  if (response.status === "passed") {
    validatePassedObservations(step, response.observations, plan);
  } else {
    emptyObservations(response.observations, `adapter.${step.action}.observations`);
  }
  return { status: response.status, outcome };
}

async function executeWithTimeout(action, step, context) {
  const controller = new AbortController();
  let timer;
  try {
    const timeout = new Promise((resolve) => {
      timer = setTimeout(() => {
        controller.abort();
        resolve({
          status: "timeout",
          outcome: "Adapter action exceeded its bounded timeout.",
          observations: {},
        });
      }, step.timeout_ms);
    });
    const execution = Promise.resolve().then(() =>
      action({ step, context, signal: controller.signal }),
    );
    return await Promise.race([execution, timeout]);
  } finally {
    clearTimeout(timer);
  }
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

function shouldExecuteStep(step, priorResults, packageAttempted) {
  if (step.group === "cleanup") return packageAttempted;
  return !priorResults.some(
    ({ group, status }) =>
      group !== "cleanup" &&
      new Set(["failed", "timeout", "unsupported", "skipped", "blocked"]).has(status),
  );
}

export async function runInstallSmoke(input, adapter) {
  const { template } = validateInput(input);
  const plan = createInstallSmokePlan(input);
  const publicSteps = plan.steps
    .filter(({ source }) => source === "public_verifier")
    .map((step) =>
      fixedStep(
        step,
        plan.execution_kind === "fixture" ? "not_applicable" : "passed",
        plan.execution_kind === "fixture"
          ? "Synthetic fixture; no public release verification is claimed."
          : step.check_id === "anonymous_download"
            ? "Anonymous public asset verification passed before install planning."
            : "Public asset checksum matched the exact release inventory.",
      ),
    );

  if (plan.execution_kind === "plan") {
    const planned = plan.steps
      .filter(({ source }) => source === "adapter")
      .map((step) =>
        fixedStep(step, "planned", "Required adapter action is planned but not executed."),
      );
    return buildInstallSmokeResult(plan, template, [...publicSteps, ...planned]);
  }

  const actions = validateAdapter(adapter, plan);
  const context = adapterContext(plan);
  const results = [...publicSteps];
  let packageAttempted = false;
  for (const step of plan.steps.filter(({ source }) => source === "adapter")) {
    if (!shouldExecuteStep(step, results, packageAttempted)) {
      results.push(
        fixedStep(step, "blocked", "Required action was blocked by an earlier smoke-step result."),
      );
      continue;
    }
    if (step.id === "install.package_install") packageAttempted = true;
    let response;
    try {
      response = await executeWithTimeout(actions[step.action], step, context);
      response = validateAdapterResponse(response, step, plan);
    } catch {
      response = {
        status: "failed",
        outcome: "Adapter response failed the bounded smoke contract.",
      };
    }
    results.push(fixedStep(step, response.status, response.outcome));
  }
  return buildInstallSmokeResult(plan, template, results);
}
