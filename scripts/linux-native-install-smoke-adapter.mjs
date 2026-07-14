import { spawn } from "node:child_process";
import fs from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import process from "node:process";

import {
  installSmokeContractInternals,
  validateInstallSmokePlan,
} from "./install-smoke-contract.mjs";

export const LINUX_NATIVE_ADAPTER_SCHEMA_VERSION = 1;

const ADAPTER_MODULE = "scripts/linux-native-install-smoke-adapter.mjs";
const MAX_OUTPUT_BYTES = 4 * 1024;
const PROBE_TIMEOUT_MS = 120;
const TERMINATION_TIMEOUT_MS = 1_500;
const NORMAL_EXIT_GROUP_GRACE_MS = 50;
const POLL_INTERVAL_MS = 10;
const descriptorStates = new WeakMap();
const settlementResults = new WeakSet();
const { PLAN_STEP_ORDER, deepFreeze, exactKeys, fail, platformContract } =
  installSmokeContractInternals;

const LINUX_PROFILES = Object.freeze({
  "linux:appimage": Object.freeze({
    package_operation: "stage",
    install_kind: "appimage",
    trust_basis: "tauri_updater",
    required_limitations: Object.freeze([]),
  }),
  "linux:deb": Object.freeze({
    package_operation: "install",
    install_kind: "deb",
    trust_basis: "public_checksum_and_source_attestation",
    required_limitations: Object.freeze(["deb_checksum_attestation_only"]),
  }),
});

const LEAF_PROGRAM = 'process.on("SIGTERM",()=>{});setInterval(()=>{},60000);';
const PROBE_PROGRAMS = Object.freeze({
  normal: 'process.stdout.write("settled\\n");',
  overflow: `process.stdout.write("x".repeat(${MAX_OUTPUT_BYTES + 1}));setInterval(()=>{},60000);`,
  "stubborn-tree": [
    'const {spawn}=require("node:child_process");',
    `const leaf=spawn(process.execPath,["--eval",${JSON.stringify(LEAF_PROGRAM)}],{env:{},shell:false,stdio:"ignore"});`,
    "leaf.unref();",
    'process.on("SIGTERM",()=>{});',
    "setInterval(()=>{},60000);",
  ].join(""),
  "orphan-tree": [
    'const {spawn}=require("node:child_process");',
    `const leaf=spawn(process.execPath,["--eval",${JSON.stringify(LEAF_PROGRAM)}],{env:{},shell:false,stdio:"ignore"});`,
    'leaf.once("spawn",()=>{leaf.unref();process.exit(0);});',
    'leaf.once("error",()=>process.exit(3));',
  ].join(""),
});

const PROBES = Object.freeze([
  Object.freeze({ id: "normal_exit", mode: "normal", expected_trigger: "exit" }),
  Object.freeze({
    id: "descendant_after_parent_exit",
    mode: "orphan-tree",
    expected_trigger: "exit",
  }),
  Object.freeze({
    id: "stubborn_process_tree",
    mode: "stubborn-tree",
    expected_trigger: "timeout",
  }),
  Object.freeze({
    id: "bounded_output",
    mode: "overflow",
    expected_trigger: "output_limit",
  }),
]);

function linuxProfile(plan) {
  const key = `${plan.platform.os}:${plan.platform.package.kind}`;
  const profile = LINUX_PROFILES[key];
  if (!profile) {
    fail("linux_adapter.plan", "must select the closed linux:deb or linux:appimage profile");
  }
  const contract = platformContract(plan.platform);
  const expected = {
    package_operation: contract.packageOperation,
    install_kind: contract.installKind,
    trust_basis: contract.trustBasis,
    required_limitations: [...contract.requiredLimitations],
  };
  if (JSON.stringify(plan.profile) !== JSON.stringify(expected)) {
    fail("linux_adapter.plan.profile", "must match the existing closed package profile");
  }
  if (JSON.stringify(profile) !== JSON.stringify(expected)) {
    fail("linux_adapter.profile", "does not match the install-smoke platform contract");
  }
  return { key, profile };
}

export function createClosedLinuxAdapterSourceDescriptor(plan, ...unexpectedArguments) {
  if (unexpectedArguments.length) {
    fail(
      "linux_adapter.arguments",
      "does not accept caller commands, environment, paths, callbacks, statuses, or evidence",
    );
  }
  validateInstallSmokePlan(plan);
  if (plan.execution_kind !== "plan") {
    fail("linux_adapter.plan", "must be a contract-only plan; fixtures cannot register adapters");
  }
  const { key, profile } = linuxProfile(plan);
  const descriptor = deepFreeze({
    schema_version: LINUX_NATIVE_ADAPTER_SCHEMA_VERSION,
    adapter: ADAPTER_MODULE,
    proof_scope: "linux_adapter_source_contract_only",
    plan_id: plan.plan_id,
    profile_id: key,
    profile: structuredClone(profile),
    required_gate_order: [...PLAN_STEP_ORDER],
    artifact_boundary: "opaque_process_local_capability_required",
    process_boundary: {
      shell: false,
      caller_environment: false,
      caller_paths: false,
      caller_commands: false,
      bounded_output_bytes: MAX_OUTPUT_BYTES,
      process_group_ownership: "required",
      termination_settlement: "required_before_cleanup",
    },
    execution_state: "package_bytes_not_executed",
    native_proof_eligibility: "none",
  });
  descriptorStates.set(descriptor, {
    identity_receipt: plan.identity_receipt,
    plan_id: plan.plan_id,
    profile_id: key,
  });
  return descriptor;
}

export function requireClosedLinuxAdapterSourceDescriptor(descriptor, plan) {
  validateInstallSmokePlan(plan);
  const state = descriptorStates.get(descriptor);
  if (!state) {
    fail(
      "linux_adapter.descriptor",
      "must be the process-local built-in descriptor returned by the Linux adapter module",
    );
  }
  const { key } = linuxProfile(plan);
  if (
    state.identity_receipt !== plan.identity_receipt ||
    state.plan_id !== plan.plan_id ||
    state.profile_id !== key
  ) {
    fail("linux_adapter.descriptor", "must bind the exact plan identity and Linux profile");
  }
  return descriptor;
}

function delay(milliseconds) {
  return new Promise((resolve) => setTimeout(resolve, milliseconds));
}

function groupAlive(pid) {
  if (!Number.isSafeInteger(pid) || pid <= 0) return false;
  try {
    process.kill(-pid, 0);
    return true;
  } catch (error) {
    if (error?.code === "ESRCH") return false;
    return true;
  }
}

async function waitForGroupSettlement(pid, deadline) {
  while (Date.now() < deadline) {
    if (!groupAlive(pid)) return true;
    await delay(POLL_INTERVAL_MS);
  }
  return !groupAlive(pid);
}

function signalOwnedProcessGroup(pid, signal) {
  try {
    process.kill(-pid, signal);
    return true;
  } catch (error) {
    return error?.code === "ESRCH";
  }
}

async function terminateAndSettle(child, closed) {
  const pid = child.pid;
  const softDeadline = Date.now() + Math.floor(TERMINATION_TIMEOUT_MS / 2);
  signalOwnedProcessGroup(pid, "SIGTERM");
  const soft = await Promise.race([
    Promise.all([closed, waitForGroupSettlement(pid, softDeadline)]).then(([, settled]) => settled),
    delay(Math.max(0, softDeadline - Date.now())).then(() => false),
  ]);
  if (soft && !groupAlive(pid)) return { close_confirmed: true, settled: true };

  const hardDeadline = Date.now() + Math.ceil(TERMINATION_TIMEOUT_MS / 2);
  signalOwnedProcessGroup(pid, "SIGKILL");
  const hard = await Promise.race([
    Promise.all([closed, waitForGroupSettlement(pid, hardDeadline)]).then(([, settled]) => settled),
    delay(Math.max(0, hardDeadline - Date.now())).then(() => false),
  ]);
  return { close_confirmed: hard, settled: hard && !groupAlive(pid) };
}

async function runFixedProbe(root, probe) {
  const child = spawn(process.execPath, ["--eval", PROBE_PROGRAMS[probe.mode]], {
    cwd: root,
    detached: true,
    env: {
      HOME: root,
      LANG: "C",
      LC_ALL: "C",
      NO_COLOR: "1",
      TMPDIR: root,
    },
    shell: false,
    stdio: ["ignore", "pipe", "pipe"],
    windowsHide: true,
  });
  let outputBytes = 0;
  let outputLimitReached = false;
  let spawnFailed = false;
  let resolveOutputLimit;
  const outputLimit = new Promise((resolve) => {
    resolveOutputLimit = resolve;
  });
  const observeOutput = (chunk) => {
    outputBytes += chunk.length;
    if (!outputLimitReached && outputBytes > MAX_OUTPUT_BYTES) {
      outputLimitReached = true;
      resolveOutputLimit("output_limit");
    }
  };
  child.stdout.on("data", observeOutput);
  child.stderr.on("data", observeOutput);
  const closed = new Promise((resolve) => {
    let resolved = false;
    const finish = (value) => {
      if (resolved) return;
      resolved = true;
      resolve(value);
    };
    child.once("error", () => {
      spawnFailed = true;
      finish({ code: null, signal: null });
    });
    child.once("close", (code, signal) => finish({ code, signal }));
  });
  const trigger = await Promise.race([
    closed.then(() => "exit"),
    outputLimit,
    delay(PROBE_TIMEOUT_MS).then(() => "timeout"),
  ]);
  let processTreeSettled = false;
  let exit = null;
  let observedBoundary;
  if (trigger === "exit") {
    exit = await closed;
    const deadline = Date.now() + NORMAL_EXIT_GROUP_GRACE_MS;
    processTreeSettled = await waitForGroupSettlement(child.pid, deadline);
    if (!processTreeSettled) {
      const termination = await terminateAndSettle(child, closed);
      processTreeSettled = termination.settled;
      observedBoundary = processTreeSettled
        ? "parent_exit_descendants_terminated_and_settled"
        : "parent_exit_descendant_settlement_unconfirmed";
    } else {
      observedBoundary = "normal_exit_settled";
    }
  } else {
    const termination = await terminateAndSettle(child, closed);
    processTreeSettled = termination.settled;
    observedBoundary = processTreeSettled
      ? trigger === "output_limit"
        ? "output_limit_terminated_and_settled"
        : "timeout_terminated_and_settled"
      : trigger === "output_limit"
        ? "output_limit_settlement_unconfirmed"
        : "timeout_settlement_unconfirmed";
  }
  const contractPassed =
    !spawnFailed &&
    trigger === probe.expected_trigger &&
    processTreeSettled &&
    ((trigger === "exit" && exit?.code === 0 && !outputLimitReached) || trigger !== "exit");
  return {
    id: probe.id,
    contract_result: contractPassed ? "passed" : "failed",
    observed_boundary: observedBoundary,
    process_tree_settled: processTreeSettled,
  };
}

function processSettlementResult(disposition, probes, cleanup) {
  const result = deepFreeze({
    schema_version: LINUX_NATIVE_ADAPTER_SCHEMA_VERSION,
    adapter: ADAPTER_MODULE,
    proof_scope: "fixed_process_settlement_contract_only",
    host: process.platform === "linux" ? "linux" : "unsupported",
    disposition,
    probes,
    cleanup,
    package_bytes_executed: false,
    native_execution_receipt: null,
    evidence_packet: null,
  });
  settlementResults.add(result);
  return result;
}

export async function runClosedLinuxProcessSettlementContract(...unexpectedArguments) {
  if (unexpectedArguments.length) {
    fail(
      "linux_adapter.process_contract.arguments",
      "does not accept caller commands, environment, paths, statuses, or evidence",
    );
  }
  if (process.platform !== "linux") {
    return processSettlementResult("unsupported", [], "not_run");
  }
  let root;
  const probes = [];
  let cleanup = "passed";
  try {
    root = await fs.mkdtemp(path.join(os.tmpdir(), "batcave-linux-adapter-contract-"));
    await fs.chmod(root, 0o700);
    for (const probe of PROBES) {
      const result = await runFixedProbe(root, probe);
      probes.push(result);
      if (!result.process_tree_settled) break;
    }
  } catch {
    probes.push({
      id: "process_contract",
      contract_result: "failed",
      observed_boundary: "process_contract_failed_closed",
      process_tree_settled: false,
    });
  } finally {
    if (root) {
      if (probes.some(({ process_tree_settled }) => process_tree_settled !== true)) {
        cleanup = "retained_unsettled";
      } else {
        try {
          await fs.rm(root, { recursive: true, force: true });
        } catch {
          cleanup = "failed";
        }
      }
    }
  }
  const disposition =
    cleanup === "passed" &&
    probes.length === PROBES.length &&
    probes.every(
      ({ contract_result, process_tree_settled }) =>
        contract_result === "passed" && process_tree_settled,
    )
      ? "source_contract_verified"
      : "failed";
  return processSettlementResult(disposition, probes, cleanup);
}

export function validateClosedLinuxProcessSettlementResult(result) {
  exactKeys(result, "linux_adapter.process_contract", [
    "schema_version",
    "adapter",
    "proof_scope",
    "host",
    "disposition",
    "probes",
    "cleanup",
    "package_bytes_executed",
    "native_execution_receipt",
    "evidence_packet",
  ]);
  if (!settlementResults.has(result)) {
    fail(
      "linux_adapter.process_contract",
      "must be the process-local result of the fixed Linux settlement contract",
    );
  }
  if (
    result.schema_version !== LINUX_NATIVE_ADAPTER_SCHEMA_VERSION ||
    result.adapter !== ADAPTER_MODULE ||
    result.proof_scope !== "fixed_process_settlement_contract_only" ||
    result.package_bytes_executed !== false ||
    result.native_execution_receipt !== null ||
    result.evidence_packet !== null
  ) {
    fail("linux_adapter.process_contract", "does not preserve the source-only proof boundary");
  }
  if (result.host === "unsupported") {
    if (
      result.disposition !== "unsupported" ||
      result.probes.length !== 0 ||
      result.cleanup !== "not_run"
    ) {
      fail("linux_adapter.process_contract", "has a contradictory unsupported-host result");
    }
  } else if (result.host === "linux") {
    if (!new Set(["source_contract_verified", "failed"]).has(result.disposition)) {
      fail("linux_adapter.process_contract.disposition", "is not supported on Linux");
    }
    if (!new Set(["passed", "failed", "retained_unsettled"]).has(result.cleanup)) {
      fail("linux_adapter.process_contract.cleanup", "is not supported on Linux");
    }
    if (result.disposition === "source_contract_verified") {
      if (
        result.cleanup !== "passed" ||
        result.probes.length !== PROBES.length ||
        result.probes.some(
          ({ contract_result, process_tree_settled }) =>
            contract_result !== "passed" || process_tree_settled !== true,
        )
      ) {
        fail("linux_adapter.process_contract", "verified disposition requires every boundary");
      }
    }
  } else {
    fail("linux_adapter.process_contract.host", "is not supported");
  }
  return result;
}
