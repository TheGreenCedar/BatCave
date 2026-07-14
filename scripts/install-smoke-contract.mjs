import { validateReleaseEvidencePacket } from "./validate-release-evidence-packet.mjs";
import {
  RELEASE_REPOSITORY,
  requireVerifiedPublicReleaseReceipt,
} from "./verify-public-release.mjs";
import { parseReleaseTag } from "./verify-release-version.mjs";

export const INSTALL_SMOKE_SCHEMA_VERSION = 1;
export const INSTALL_SMOKE_DISPOSITIONS = Object.freeze([
  "planned",
  "fixture",
  "skipped",
  "failed",
  "native_proven",
]);

const EXECUTION_KINDS = new Set(["plan", "fixture", "native"]);
const ADAPTER_STATUSES = new Set(["passed", "failed", "timeout", "unsupported", "skipped"]);
const RESULT_STATUSES = new Set([
  "planned",
  "passed",
  "failed",
  "timeout",
  "unsupported",
  "skipped",
  "blocked",
  "not_applicable",
]);
const SLUG = /^[a-z0-9]+(?:[._-][a-z0-9]+)*$/u;
const COMMIT_SHA = /^[0-9a-f]{40}$/u;
const SHA256_DIGEST = /^sha256:[0-9a-f]{64}$/u;
const PUBLIC_VERIFIER = "scripts/verify-public-release.mjs";
const PUBLIC_ASSET_URL_PREFIX = `https://github.com/${RELEASE_REPOSITORY}/releases/download/`;
const QUALITY_STATES = new Set(["native", "limited"]);
const PLATFORM_CONTRACTS = Object.freeze({
  "linux:appimage": Object.freeze({
    prepareAction: "stage_appimage",
    installKind: "appimage",
    removeAction: "remove_appimage",
    degradationScenario: "permission-limited-telemetry",
    trustBasis: "tauri_updater",
  }),
  "linux:deb": Object.freeze({
    prepareAction: "install_deb",
    installKind: "deb",
    removeAction: "remove_deb",
    degradationScenario: "permission-limited-telemetry",
    trustBasis: "public_checksum_and_source_attestation",
  }),
  "macos:dmg": Object.freeze({
    prepareAction: "install_dmg_app",
    installKind: "app_bundle",
    removeAction: "remove_macos_app",
    degradationScenario: "permission-limited-telemetry",
    trustBasis: "developer_id_notarization_and_staple",
  }),
  "macos:macos_updater": Object.freeze({
    prepareAction: "stage_updater_archive_app",
    installKind: "app_bundle",
    removeAction: "remove_macos_app",
    degradationScenario: "permission-limited-telemetry",
    trustBasis: "contained_app_trust_and_tauri_updater",
  }),
  "windows:nsis": Object.freeze({
    prepareAction: "install_nsis",
    installKind: "nsis",
    removeAction: "remove_nsis",
    degradationScenario: "standard-access-visibility",
    trustBasis: "authenticode_and_tauri_updater",
  }),
});
const CHECK_ORDER = Object.freeze([
  ["install", "anonymous_download"],
  ["install", "checksum"],
  ["install", "package_install"],
  ["runtime", "launch"],
  ["runtime", "release_identity"],
  ["runtime", "settings"],
  ["runtime", "degradation"],
  ["runtime", "telemetry"],
  ["cleanup", "application_removed"],
  ["cleanup", "owned_runtime_cleanup"],
  ["cleanup", "user_state_policy"],
]);
const CHECK_IDS = new Set(CHECK_ORDER.map(([group, id]) => `${group}.${id}`));
const HARNESS_STEP_IDS = new Set(["preflight.asset_rehash", "preflight.package_trust"]);
const PLAN_STEP_IDS = new Set([...CHECK_IDS, ...HARNESS_STEP_IDS]);
const PLAN_STEP_ORDER = Object.freeze([
  "install.anonymous_download",
  "install.checksum",
  "preflight.package_trust",
  "preflight.asset_rehash",
  "install.package_install",
  "runtime.launch",
  "runtime.release_identity",
  "runtime.settings",
  "runtime.degradation",
  "runtime.telemetry",
  "cleanup.application_removed",
  "cleanup.owned_runtime_cleanup",
  "cleanup.user_state_policy",
]);

function fail(field, message) {
  throw new Error(`${field}: ${message}`);
}

function object(value, field) {
  if (value === null || typeof value !== "object" || Array.isArray(value)) {
    fail(field, "must be an object");
  }
  return value;
}

function exactKeys(value, field, keys) {
  object(value, field);
  const actual = Object.keys(value);
  const missing = keys.filter((key) => !actual.includes(key));
  const extra = actual.filter((key) => !keys.includes(key));
  if (missing.length) fail(`${field}.${missing[0]}`, "is required");
  if (extra.length) fail(`${field}.${extra[0]}`, "is not allowed");
}

function string(value, field, { max = 200, pattern } = {}) {
  const control =
    typeof value === "string" &&
    [...value].some((character) => {
      const codePoint = character.codePointAt(0);
      return codePoint <= 0x1f || codePoint === 0x7f;
    });
  if (
    typeof value !== "string" ||
    value.length === 0 ||
    value.length > max ||
    value !== value.normalize("NFC") ||
    control
  ) {
    fail(field, `must be a normalized single-line string of at most ${max} characters`);
  }
  if (pattern && !pattern.test(value)) fail(field, "has an invalid format");
  return value;
}

function slug(value, field) {
  return string(value, field, { max: 120, pattern: SLUG });
}

function positiveInteger(value, field, maximum = Number.MAX_SAFE_INTEGER) {
  if (!Number.isSafeInteger(value) || value <= 0 || value > maximum) {
    fail(field, `must be a positive safe integer no greater than ${maximum}`);
  }
  return value;
}

function sortedObject(entries) {
  return Object.fromEntries(entries.sort(([left], [right]) => left.localeCompare(right)));
}

function deepFreeze(value) {
  if (value && typeof value === "object" && !Object.isFrozen(value)) {
    Object.freeze(value);
    for (const child of Object.values(value)) deepFreeze(child);
  }
  return value;
}

function publicAssetUrl(tag, name) {
  return `${PUBLIC_ASSET_URL_PREFIX}${encodeURIComponent(tag)}/${encodeURIComponent(name)}`;
}

function platformContract(platform) {
  const key = `${platform.os}:${platform.package.kind}`;
  const contract = PLATFORM_CONTRACTS[key];
  if (!contract)
    fail("input.evidence_template.platform", `has no install-smoke contract for ${key}`);
  return contract;
}

function validatePublicVerification(input, template, appVersion, executionKind) {
  if (executionKind !== "fixture") requireVerifiedPublicReleaseReceipt(input);
  exactKeys(input, "input.public_verification", [
    "schema_version",
    "verifier",
    "disposition",
    "repository",
    "tag",
    "source_sha",
    "app_version",
    "assets",
  ]);
  if (input.schema_version !== INSTALL_SMOKE_SCHEMA_VERSION) {
    fail("input.public_verification.schema_version", `must equal ${INSTALL_SMOKE_SCHEMA_VERSION}`);
  }
  if (input.verifier !== PUBLIC_VERIFIER) {
    fail("input.public_verification.verifier", `must equal ${PUBLIC_VERIFIER}`);
  }
  const expectedDisposition = executionKind === "fixture" ? "fixture" : "passed";
  if (input.disposition !== expectedDisposition) {
    fail(
      "input.public_verification.disposition",
      `must equal ${expectedDisposition} for ${executionKind} execution`,
    );
  }
  if (input.repository !== RELEASE_REPOSITORY || input.repository !== template.release.repository) {
    fail("input.public_verification.repository", `must equal ${RELEASE_REPOSITORY}`);
  }
  if (input.tag !== template.release.tag)
    fail("input.public_verification.tag", "must match the evidence template");
  if (!COMMIT_SHA.test(input.source_sha) || input.source_sha !== template.release.source_sha) {
    fail("input.public_verification.source_sha", "must match the exact evidence source commit");
  }
  if (input.app_version !== appVersion) {
    fail("input.public_verification.app_version", "must match the tag-derived app version");
  }

  const packageAsset = template.assets.find(
    ({ name }) => name === template.platform.package.asset_name,
  );
  if (!packageAsset) fail("input.evidence_template", "must contain the selected package asset");
  if (!Array.isArray(input.assets) || input.assets.length === 0) {
    fail("input.public_verification.assets", "must contain verified public assets");
  }
  const verifiedAsset = input.assets.find(({ name }) => name === packageAsset.name);
  if (!verifiedAsset) {
    fail("input.public_verification.assets", "must contain the selected package asset");
  }
  exactKeys(verifiedAsset, "input.public_verification.assets.package", [
    "name",
    "size_bytes",
    "sha256",
    "public_url",
  ]);
  positiveInteger(verifiedAsset.size_bytes, "input.public_verification.assets.package.size_bytes");
  if (verifiedAsset.size_bytes !== packageAsset.size_bytes) {
    fail(
      "input.public_verification.assets.package.size_bytes",
      "must match the evidence asset size",
    );
  }
  string(verifiedAsset.sha256, "input.public_verification.assets.package.sha256", {
    max: 71,
    pattern: SHA256_DIGEST,
  });
  if (
    verifiedAsset.sha256 !== packageAsset.sha256 ||
    verifiedAsset.sha256 !== packageAsset.api_digest
  ) {
    fail(
      "input.public_verification.assets.package.sha256",
      "must match the verified public asset digest",
    );
  }
  const expectedUrl = publicAssetUrl(input.tag, verifiedAsset.name);
  if (verifiedAsset.public_url !== expectedUrl || packageAsset.public_url !== expectedUrl) {
    fail(
      "input.public_verification.assets.package.public_url",
      "must be the exact anonymous public asset URL",
    );
  }
  return packageAsset;
}

function validateIsolation(isolation, contract) {
  exactKeys(isolation, "input.isolation", [
    "scope_id",
    "install_root_id",
    "user_state_root_id",
    "user_state_policy",
    "step_timeout_ms",
    "settings_probe",
    "degradation_scenario",
  ]);
  slug(isolation.scope_id, "input.isolation.scope_id");
  slug(isolation.install_root_id, "input.isolation.install_root_id");
  slug(isolation.user_state_root_id, "input.isolation.user_state_root_id");
  if (isolation.install_root_id === isolation.user_state_root_id) {
    fail("input.isolation", "install and user-state root identifiers must be distinct");
  }
  if (!new Set(["preserve", "remove"]).has(isolation.user_state_policy)) {
    fail("input.isolation.user_state_policy", "must be preserve or remove");
  }
  positiveInteger(isolation.step_timeout_ms, "input.isolation.step_timeout_ms", 600_000);
  exactKeys(isolation.settings_probe, "input.isolation.settings_probe", [
    "theme",
    "sample_interval_ms",
  ]);
  if (!new Set(["cave", "daylight"]).has(isolation.settings_probe.theme)) {
    fail("input.isolation.settings_probe.theme", "must be cave or daylight");
  }
  const interval = positiveInteger(
    isolation.settings_probe.sample_interval_ms,
    "input.isolation.settings_probe.sample_interval_ms",
    5_000,
  );
  if (interval < 500)
    fail("input.isolation.settings_probe.sample_interval_ms", "must be at least 500");
  if (isolation.degradation_scenario !== contract.degradationScenario) {
    fail(
      "input.isolation.degradation_scenario",
      `must equal ${contract.degradationScenario} for this package`,
    );
  }
  return structuredClone(isolation);
}

function requireBlankNativeChecks(template, executionKind) {
  if (executionKind === "fixture") return;
  for (const [group, id] of CHECK_ORDER) {
    if (template.checks[group][id].status !== "blocked") {
      fail(
        `input.evidence_template.checks.${group}.${id}.status`,
        "must be blocked before install-smoke execution",
      );
    }
  }
}

function validateInput(input) {
  exactKeys(input, "input", [
    "schema_version",
    "execution_kind",
    "app_version",
    "evidence_template",
    "public_verification",
    "isolation",
  ]);
  if (input.schema_version !== INSTALL_SMOKE_SCHEMA_VERSION) {
    fail("input.schema_version", `must equal ${INSTALL_SMOKE_SCHEMA_VERSION}`);
  }
  if (!EXECUTION_KINDS.has(input.execution_kind)) {
    fail("input.execution_kind", "must be plan, fixture, or native");
  }
  const template = structuredClone(input.evidence_template);
  validateReleaseEvidencePacket(template);
  const expectedPacketKind =
    input.execution_kind === "fixture" ? "schema_fixture" : "release_evidence";
  if (template.packet_kind !== expectedPacketKind) {
    fail("input.evidence_template.packet_kind", `must equal ${expectedPacketKind}`);
  }
  const parsed = parseReleaseTag(template.release.tag);
  if (input.app_version !== parsed.version) {
    fail("input.app_version", `must equal ${parsed.version} from the release tag`);
  }
  requireBlankNativeChecks(template, input.execution_kind);
  const contract = platformContract(template.platform);
  const packageAsset = validatePublicVerification(
    input.public_verification,
    template,
    input.app_version,
    input.execution_kind,
  );
  const isolation = validateIsolation(input.isolation, contract);
  return { template, contract, packageAsset, isolation };
}

function adapterStep(group, id, action, timeoutMs) {
  return {
    id: `${group}.${id}`,
    group,
    check_id: id,
    source: "adapter",
    action,
    required: true,
    timeout_ms: timeoutMs,
  };
}

function harnessStep(id, action, timeoutMs) {
  return {
    id,
    group: "preflight",
    check_id: id.slice("preflight.".length),
    source: "adapter",
    action,
    required: true,
    timeout_ms: timeoutMs,
  };
}

export function createInstallSmokePlan(input) {
  const { template, contract, packageAsset, isolation } = validateInput(input);
  const timeout = isolation.step_timeout_ms;
  const steps = [
    {
      id: "install.anonymous_download",
      group: "install",
      check_id: "anonymous_download",
      source: "public_verifier",
      action: "verified_anonymous_download",
      required: true,
      timeout_ms: 0,
    },
    {
      id: "install.checksum",
      group: "install",
      check_id: "checksum",
      source: "public_verifier",
      action: "verified_public_checksum",
      required: true,
      timeout_ms: 0,
    },
    harnessStep("preflight.package_trust", "verify_package_trust", timeout),
    harnessStep("preflight.asset_rehash", "rehash_public_asset", timeout),
    adapterStep("install", "package_install", contract.prepareAction, timeout),
    adapterStep("runtime", "launch", "launch_app", timeout),
    adapterStep("runtime", "release_identity", "verify_release_identity", timeout),
    adapterStep("runtime", "settings", "restart_and_verify_settings", timeout),
    adapterStep("runtime", "degradation", "probe_supported_degradation", timeout),
    adapterStep("runtime", "telemetry", "verify_telemetry", timeout),
    adapterStep("cleanup", "application_removed", contract.removeAction, timeout),
    adapterStep("cleanup", "owned_runtime_cleanup", "verify_owned_runtime_cleanup", timeout),
    adapterStep("cleanup", "user_state_policy", "verify_user_state_policy", timeout),
  ];
  const plan = {
    schema_version: INSTALL_SMOKE_SCHEMA_VERSION,
    plan_id: template.packet_id,
    execution_kind: input.execution_kind,
    release: {
      repository: template.release.repository,
      tag: template.release.tag,
      app_version: input.app_version,
      source_sha: template.release.source_sha,
    },
    platform: structuredClone(template.platform),
    asset: {
      name: packageAsset.name,
      size_bytes: packageAsset.size_bytes,
      sha256: packageAsset.sha256,
      public_url: packageAsset.public_url,
      expected_signatures: structuredClone(packageAsset.signatures),
      expected_trust_basis: contract.trustBasis,
    },
    public_verification: structuredClone(input.public_verification),
    isolation,
    constraints: {
      adapter_owns_local_paths: true,
      bounded_output_required: true,
      minimal_environment_required: true,
      requires_isolated_roots: true,
      root_escape_allowed: false,
      shell_execution_allowed: false,
      symlinks_allowed: false,
      timeout_tree_cleanup_required: true,
      tokenized_argv_required: true,
      windows_service_behavior_assumed: false,
    },
    steps,
  };
  return validateInstallSmokePlan(deepFreeze(plan));
}

export function validateInstallSmokePlan(plan) {
  exactKeys(plan, "plan", [
    "schema_version",
    "plan_id",
    "execution_kind",
    "release",
    "platform",
    "asset",
    "public_verification",
    "isolation",
    "constraints",
    "steps",
  ]);
  if (plan.schema_version !== INSTALL_SMOKE_SCHEMA_VERSION)
    fail("plan.schema_version", "must equal 1");
  slug(plan.plan_id, "plan.plan_id");
  if (!EXECUTION_KINDS.has(plan.execution_kind)) fail("plan.execution_kind", "is not supported");
  exactKeys(plan.release, "plan.release", ["repository", "tag", "app_version", "source_sha"]);
  if (plan.release.repository !== RELEASE_REPOSITORY)
    fail("plan.release.repository", "is not supported");
  const parsedRelease = parseReleaseTag(plan.release.tag);
  string(plan.release.app_version, "plan.release.app_version", { max: 80 });
  if (plan.release.app_version !== parsedRelease.version) {
    fail("plan.release.app_version", "must match the release tag");
  }
  string(plan.release.source_sha, "plan.release.source_sha", { max: 40, pattern: COMMIT_SHA });
  exactKeys(plan.platform, "plan.platform", ["os", "os_version", "architecture", "package"]);
  exactKeys(plan.platform.package, "plan.platform.package", ["kind", "architecture", "asset_name"]);
  exactKeys(plan.asset, "plan.asset", [
    "name",
    "size_bytes",
    "sha256",
    "public_url",
    "expected_signatures",
    "expected_trust_basis",
  ]);
  string(plan.asset.name, "plan.asset.name", { max: 180 });
  positiveInteger(plan.asset.size_bytes, "plan.asset.size_bytes");
  string(plan.asset.sha256, "plan.asset.sha256", { max: 71, pattern: SHA256_DIGEST });
  if (plan.asset.public_url !== publicAssetUrl(plan.release.tag, plan.asset.name)) {
    fail("plan.asset.public_url", "must be the exact anonymous public asset URL");
  }
  object(plan.asset.expected_signatures, "plan.asset.expected_signatures");
  slug(plan.asset.expected_trust_basis, "plan.asset.expected_trust_basis");
  const contract = platformContract(plan.platform);
  validateIsolation(plan.isolation, contract);
  exactKeys(plan.public_verification, "plan.public_verification", [
    "schema_version",
    "verifier",
    "disposition",
    "repository",
    "tag",
    "source_sha",
    "app_version",
    "assets",
  ]);
  const expectedDisposition = plan.execution_kind === "fixture" ? "fixture" : "passed";
  if (
    plan.public_verification.schema_version !== INSTALL_SMOKE_SCHEMA_VERSION ||
    plan.public_verification.verifier !== PUBLIC_VERIFIER ||
    plan.public_verification.disposition !== expectedDisposition ||
    plan.public_verification.repository !== plan.release.repository ||
    plan.public_verification.tag !== plan.release.tag ||
    plan.public_verification.source_sha !== plan.release.source_sha ||
    plan.public_verification.app_version !== plan.release.app_version
  ) {
    fail("plan.public_verification", "must match the exact verified release identity");
  }
  if (!Array.isArray(plan.public_verification.assets) || !plan.public_verification.assets.length) {
    fail("plan.public_verification.assets", "must contain verified public assets");
  }
  const receiptNames = new Set();
  for (const [index, asset] of plan.public_verification.assets.entries()) {
    exactKeys(asset, `plan.public_verification.assets[${index}]`, [
      "name",
      "size_bytes",
      "sha256",
      "public_url",
    ]);
    if (receiptNames.has(asset.name)) {
      fail(`plan.public_verification.assets[${index}].name`, "must not be duplicated");
    }
    receiptNames.add(asset.name);
  }
  const receiptAsset = plan.public_verification.assets.find(({ name }) => name === plan.asset.name);
  if (
    !receiptAsset ||
    receiptAsset.size_bytes !== plan.asset.size_bytes ||
    receiptAsset.sha256 !== plan.asset.sha256 ||
    receiptAsset.public_url !== plan.asset.public_url
  ) {
    fail("plan.public_verification.assets", "must contain the exact selected package asset");
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
  exactKeys(plan.constraints, "plan.constraints", [
    "adapter_owns_local_paths",
    "bounded_output_required",
    "minimal_environment_required",
    "requires_isolated_roots",
    "root_escape_allowed",
    "shell_execution_allowed",
    "symlinks_allowed",
    "timeout_tree_cleanup_required",
    "tokenized_argv_required",
    "windows_service_behavior_assumed",
  ]);
  if (
    plan.constraints.adapter_owns_local_paths !== true ||
    plan.constraints.bounded_output_required !== true ||
    plan.constraints.minimal_environment_required !== true ||
    plan.constraints.requires_isolated_roots !== true ||
    plan.constraints.root_escape_allowed !== false ||
    plan.constraints.shell_execution_allowed !== false ||
    plan.constraints.symlinks_allowed !== false ||
    plan.constraints.timeout_tree_cleanup_required !== true ||
    plan.constraints.tokenized_argv_required !== true ||
    plan.constraints.windows_service_behavior_assumed !== false
  ) {
    fail("plan.constraints", "must preserve isolation and the Windows service non-assumption");
  }
  if (!Array.isArray(plan.steps) || plan.steps.length !== PLAN_STEP_IDS.size) {
    fail("plan.steps", `must contain all ${PLAN_STEP_IDS.size} required gates`);
  }
  const actualIds = new Set();
  for (const [index, step] of plan.steps.entries()) {
    exactKeys(step, `plan.steps[${index}]`, [
      "id",
      "group",
      "check_id",
      "source",
      "action",
      "required",
      "timeout_ms",
    ]);
    if (step.id !== `${step.group}.${step.check_id}` || !PLAN_STEP_IDS.has(step.id)) {
      fail(`plan.steps[${index}].id`, "must identify one exact smoke gate");
    }
    if (actualIds.has(step.id)) fail(`plan.steps[${index}].id`, "must not be duplicated");
    actualIds.add(step.id);
    if (step.id !== PLAN_STEP_ORDER[index]) {
      fail(`plan.steps[${index}].id`, `must equal ordered gate ${PLAN_STEP_ORDER[index]}`);
    }
    if (step.action !== expectedActions[index]) {
      fail(`plan.steps[${index}].action`, `must equal ${expectedActions[index]}`);
    }
    const expectedSource = index < 2 ? "public_verifier" : "adapter";
    if (step.source !== expectedSource) {
      fail(`plan.steps[${index}].source`, `must equal ${expectedSource}`);
    }
    if (!new Set(["public_verifier", "adapter"]).has(step.source)) {
      fail(`plan.steps[${index}].source`, "is not supported");
    }
    slug(step.action, `plan.steps[${index}].action`);
    if (step.required !== true) fail(`plan.steps[${index}].required`, "must be true");
    if (
      (step.source === "public_verifier" && step.timeout_ms !== 0) ||
      (step.source === "adapter" &&
        (!Number.isSafeInteger(step.timeout_ms) || step.timeout_ms <= 0))
    ) {
      fail(`plan.steps[${index}].timeout_ms`, "does not match the step source");
    }
  }
  return plan;
}

export const installSmokeContractInternals = Object.freeze({
  ADAPTER_STATUSES,
  CHECK_ORDER,
  EXECUTION_KINDS,
  PLAN_STEP_IDS,
  PLAN_STEP_ORDER,
  QUALITY_STATES,
  RESULT_STATUSES,
  deepFreeze,
  exactKeys,
  fail,
  object,
  platformContract,
  slug,
  sortedObject,
  string,
  validateInput,
});
