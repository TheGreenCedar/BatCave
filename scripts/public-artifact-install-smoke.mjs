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

const { ADAPTER_STATUSES, QUALITY_STATES, deepFreeze, exactKeys, fail, object, string } =
  installSmokeContractInternals;
const HARNESS_TIMEOUT_RESPONSE = Symbol("harness-timeout-response");

function ownFunction(owner, key, field) {
  const descriptor = Object.getOwnPropertyDescriptor(owner, key);
  if (!descriptor || typeof descriptor.value !== "function") {
    fail(field, "must be an explicit data function before execution starts");
  }
  return descriptor.value;
}

function validateAdapter(adapter, plan) {
  exactKeys(adapter, "adapter", ["kind", "executor", "actions"]);
  if (plan.execution_kind !== "fixture" || adapter.kind !== "fixture") {
    fail("adapter.kind", "must be fixture; injected adapters cannot produce native evidence");
  }
  object(adapter.actions, "adapter.actions");
  exactKeys(adapter.executor, "adapter.executor", [
    "tokenized_argv",
    "shell",
    "minimal_environment",
    "bounded_output",
    "timeout_tree_cleanup",
    "confirm_terminated",
  ]);
  if (
    adapter.executor.tokenized_argv !== true ||
    adapter.executor.shell !== false ||
    adapter.executor.minimal_environment !== true ||
    adapter.executor.bounded_output !== true ||
    adapter.executor.timeout_tree_cleanup !== true
  ) {
    fail("adapter.executor", "must enforce the bounded non-shell fixture executor contract");
  }
  const confirmTerminated = ownFunction(
    adapter.executor,
    "confirm_terminated",
    "adapter.executor.confirm_terminated",
  );
  const requiredActions = new Set(
    plan.steps.filter(({ source }) => source === "adapter").map(({ action }) => action),
  );
  const validatedActions = Object.create(null);
  for (const action of requiredActions) {
    validatedActions[action] = ownFunction(adapter.actions, action, `adapter.actions.${action}`);
  }
  const unexpected = Object.keys(adapter.actions).filter((action) => !requiredActions.has(action));
  if (unexpected.length) fail(`adapter.actions.${unexpected[0]}`, "is not used by this plan");
  return { actions: Object.freeze(validatedActions), confirmTerminated };
}

function adapterContext(plan) {
  return deepFreeze({
    observed_at_utc: plan.observed_at_utc,
    release: structuredClone(plan.release),
    platform: structuredClone(plan.platform),
    asset: structuredClone(plan.asset),
    public_verification: structuredClone(plan.public_verification),
    isolation: structuredClone(plan.isolation),
    profile: structuredClone(plan.profile),
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
        fail(field, "must match the fixture contract for the selected regular asset");
      }
      break;
    case "preflight.package_trust":
      exactKeys(observations, field, ["trust_basis", "fixture_only"]);
      if (
        observations.trust_basis !== plan.profile.trust_basis ||
        observations.fixture_only !== true
      ) {
        fail(field, "must match the closed fixture-only package trust path");
      }
      break;
    case "install.package_install":
      exactKeys(observations, field, ["package_ready", "package_operation"]);
      if (
        observations.package_ready !== true ||
        observations.package_operation !== plan.profile.package_operation
      ) {
        fail(field, "must match the package preparation operation");
      }
      break;
    case "runtime.launch":
      exactKeys(observations, field, ["launched"]);
      if (observations.launched !== true) fail(`${field}.launched`, "must be true");
      break;
    case "runtime.release_identity":
      exactKeys(observations, field, ["app_version", "source_commit_sha", "install_kind"]);
      if (
        observations.app_version !== plan.release.app_version ||
        observations.source_commit_sha !== plan.release.source_sha ||
        observations.install_kind !== plan.profile.install_kind
      ) {
        fail(field, "must match the exact fixture release and package kind");
      }
      break;
    case "runtime.settings":
      exactKeys(observations, field, ["restarted", "preserved"]);
      if (observations.restarted !== true || observations.preserved !== true) {
        fail(field, "must show a restart with settings preserved");
      }
      break;
    case "runtime.degradation":
      exactKeys(observations, field, ["reported", "windows_service_behavior"]);
      if (
        observations.reported !== true ||
        observations.windows_service_behavior !== "not_assumed"
      ) {
        fail(field, "must report degradation without assuming Windows service behavior");
      }
      break;
    case "runtime.telemetry":
      exactKeys(observations, field, ["sample_observed", "quality_state"]);
      if (
        observations.sample_observed !== true ||
        !QUALITY_STATES.has(observations.quality_state)
      ) {
        fail(field, "must observe a fixture sample with native or limited quality");
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
    fail(`adapter.${step.action}.status`, "is not supported; timeout is owned by the harness");
  }
  const outcome = responseSummary(response.outcome, `adapter.${step.action}.outcome`);
  if (response.status === "passed") {
    validatePassedObservations(step, response.observations, plan);
  } else {
    emptyObservations(response.observations, `adapter.${step.action}.observations`);
  }
  return { status: response.status, outcome };
}

function timeoutAfter(milliseconds, value) {
  let timer;
  const promise = new Promise((resolve) => {
    timer = setTimeout(() => resolve(value), milliseconds);
  });
  return { promise, cancel: () => clearTimeout(timer) };
}

async function confirmTimeoutTermination(confirmTerminated, step, context) {
  const bounded = timeoutAfter(context.isolation.termination_timeout_ms, null);
  try {
    const response = await Promise.race([
      Promise.resolve().then(() => confirmTerminated({ step, context })),
      bounded.promise,
    ]);
    if (!response) return false;
    exactKeys(response, "adapter.executor.confirm_terminated", [
      "action_settled",
      "process_tree_settled",
      "outcome",
    ]);
    responseSummary(response.outcome, "adapter.executor.confirm_terminated.outcome");
    return response.action_settled === true && response.process_tree_settled === true;
  } catch {
    return false;
  } finally {
    bounded.cancel();
  }
}

async function executeWithTimeout(action, confirmTerminated, step, context) {
  const controller = new AbortController();
  const timeoutMarker = Symbol("timeout");
  const bounded = timeoutAfter(step.timeout_ms, timeoutMarker);
  try {
    const execution = Promise.resolve().then(() =>
      action({ step, context, signal: controller.signal }),
    );
    const response = await Promise.race([execution, bounded.promise]);
    if (response !== timeoutMarker) return response;
    controller.abort();
    const settled = await confirmTimeoutTermination(confirmTerminated, step, context);
    return settled
      ? {
          [HARNESS_TIMEOUT_RESPONSE]: true,
          status: "timeout",
          outcome: "Fixture action timed out and its termination handshake settled.",
          observations: {},
        }
      : {
          [HARNESS_TIMEOUT_RESPONSE]: true,
          status: "partial",
          outcome: "Fixture action timed out without a settled termination handshake.",
          observations: {},
        };
  } finally {
    bounded.cancel();
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
  const unsettled = priorResults.some(({ status }) => status === "partial");
  if (unsettled) return false;
  if (step.group === "cleanup") return packageAttempted;
  return !priorResults.some(
    ({ group, status }) =>
      group !== "cleanup" &&
      new Set(["failed", "timeout", "unsupported", "skipped", "blocked"]).has(status),
  );
}

export async function runInstallSmoke(input, adapter) {
  const plan = createInstallSmokePlan(input);
  const publicSteps = plan.steps
    .filter(({ source }) => source === "public_verifier")
    .map((step) =>
      fixedStep(
        step,
        plan.execution_kind === "fixture" ? "not_applicable" : "passed",
        plan.execution_kind === "fixture"
          ? "Synthetic fixture; no public release verification is claimed."
          : "The contract-only public verification prerequisite passed for planning.",
      ),
    );

  if (plan.execution_kind === "plan") {
    const planned = plan.steps
      .filter(({ source }) => source === "adapter")
      .map((step) =>
        fixedStep(step, "planned", "Required native action is planned but unavailable."),
      );
    return buildInstallSmokeResult(plan, [...publicSteps, ...planned]);
  }

  const { actions, confirmTerminated } = validateAdapter(adapter, plan);
  const context = adapterContext(plan);
  const results = [...publicSteps];
  let packageAttempted = false;
  for (const step of plan.steps.filter(({ source }) => source === "adapter")) {
    if (!shouldExecuteStep(step, results, packageAttempted)) {
      const partial = results.some(({ status }) => status === "partial");
      results.push(
        fixedStep(
          step,
          "blocked",
          partial
            ? "Action blocked because prior process-tree settlement is unconfirmed."
            : "Required action was blocked by an earlier fixture-step result.",
        ),
      );
      continue;
    }
    if (step.id === "install.package_install") packageAttempted = true;
    let response;
    try {
      response = await executeWithTimeout(actions[step.action], confirmTerminated, step, context);
      if (response?.[HARNESS_TIMEOUT_RESPONSE] !== true) {
        response = new Set(["timeout", "partial"]).has(response?.status)
          ? {
              status: "partial",
              outcome: "Adapter-authored timeout state is untrusted and has no settlement proof.",
            }
          : validateAdapterResponse(response, step, plan);
      }
    } catch {
      response = {
        status: "failed",
        outcome: "Fixture adapter response failed the bounded smoke contract.",
      };
    }
    results.push(fixedStep(step, response.status, response.outcome));
  }
  return buildInstallSmokeResult(plan, results);
}
