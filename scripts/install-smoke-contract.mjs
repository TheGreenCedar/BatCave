import {
  validateReleaseEvidenceIdentity,
  validateReleaseEvidencePacket,
  validateReleaseEvidenceTemplatePacket,
} from "./validate-release-evidence-packet.mjs";
import {
  RELEASE_REPOSITORY,
  RELEASE_SIGNER_WORKFLOW,
  RELEASE_SOURCE_REF,
  requireVerifiedPublicReleaseReceipt,
} from "./verify-public-release.mjs";
import { parseReleaseTag } from "./verify-release-version.mjs";

export const INSTALL_SMOKE_SCHEMA_VERSION = 1;
export const INSTALL_SMOKE_DISPOSITIONS = Object.freeze(["planned", "fixture", "partial"]);

const EXECUTION_KINDS = new Set(["plan", "fixture"]);
const ADAPTER_STATUSES = new Set(["passed", "failed", "unsupported", "skipped"]);
const RESULT_STATUSES = new Set([
  "planned",
  "passed",
  "failed",
  "timeout",
  "partial",
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
const installSmokeIdentityReceipts = new WeakSet();
const PLATFORM_CONTRACTS = Object.freeze({
  "linux:appimage": Object.freeze({
    prepareAction: "stage_appimage",
    installKind: "appimage",
    packageOperation: "stage",
    removeAction: "remove_appimage",
    degradationScenario: "permission-limited-telemetry",
    trustBasis: "tauri_updater",
    requiredLimitations: Object.freeze([]),
  }),
  "linux:deb": Object.freeze({
    prepareAction: "install_deb",
    installKind: "deb",
    packageOperation: "install",
    removeAction: "remove_deb",
    degradationScenario: "permission-limited-telemetry",
    trustBasis: "public_checksum_and_source_attestation",
    requiredLimitations: Object.freeze(["deb_checksum_attestation_only"]),
  }),
  "macos:dmg": Object.freeze({
    prepareAction: "install_dmg_app",
    installKind: "app_bundle",
    packageOperation: "install",
    removeAction: "remove_macos_app",
    degradationScenario: "permission-limited-telemetry",
    trustBasis: "developer_id_notarization_and_staple",
    requiredLimitations: Object.freeze([]),
  }),
  "macos:macos_updater": Object.freeze({
    prepareAction: "stage_updater_archive_app",
    installKind: "app_bundle",
    packageOperation: "stage",
    removeAction: "remove_macos_app",
    degradationScenario: "permission-limited-telemetry",
    trustBasis: "contained_app_trust_and_tauri_updater",
    requiredLimitations: Object.freeze(["macos_updater_staging_only"]),
  }),
  "windows:nsis": Object.freeze({
    prepareAction: "install_nsis",
    installKind: "nsis",
    packageOperation: "install",
    removeAction: "remove_nsis",
    degradationScenario: "standard-access-visibility",
    trustBasis: "authenticode_and_tauri_updater",
    requiredLimitations: Object.freeze(["windows_service_etw_out_of_scope"]),
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
  if (!contract) fail("platform", `has no install-smoke contract for ${key}`);
  return contract;
}

function createInstallSmokeIdentityReceipt(planId, observedAtUtc, release, platform, asset) {
  const receipt = deepFreeze({
    schema_version: INSTALL_SMOKE_SCHEMA_VERSION,
    plan_id: planId,
    observed_at_utc: observedAtUtc,
    release: structuredClone(release),
    platform: structuredClone(platform),
    asset: structuredClone(asset),
  });
  installSmokeIdentityReceipts.add(receipt);
  return receipt;
}

function validateInstallSmokeIdentityReceipt(value) {
  exactKeys(value.identity_receipt, "identity.identity_receipt", [
    "schema_version",
    "plan_id",
    "observed_at_utc",
    "release",
    "platform",
    "asset",
  ]);
  const expected = {
    schema_version: INSTALL_SMOKE_SCHEMA_VERSION,
    plan_id: value.plan_id,
    observed_at_utc: value.observed_at_utc,
    release: value.release,
    platform: value.platform,
    asset: value.asset,
  };
  if (JSON.stringify(value.identity_receipt) !== JSON.stringify(expected)) {
    fail(
      "identity.identity_receipt",
      "must bind the exact plan, release, platform, selected asset name, size, digest, and URL",
    );
  }
  if (!installSmokeIdentityReceipts.has(value.identity_receipt)) {
    fail(
      "identity.identity_receipt",
      "must retain the process-local selected-asset identity created by the harness",
    );
  }
}

function validateInstallSmokeIdentity(value) {
  exactKeys(value.release, "identity.release", [
    "repository",
    "tag",
    "channel",
    "source_sha",
    "main_sha",
    "release_target_sha",
    "release_url",
    "app_version",
    "workflow_run",
  ]);
  const { app_version: appVersion, ...release } = value.release;
  validateReleaseEvidenceIdentity(
    {
      observed_at_utc: value.observed_at_utc,
      release,
      app_version: appVersion,
      platform: value.platform,
      asset: value.asset,
    },
    value.execution_kind === "fixture" ? "schema_fixture" : "release_plan",
  );
  validateInstallSmokeIdentityReceipt(value);
  return platformContract(value.platform);
}

function validateReleaseIdentity(template) {
  const release = template.release;
  exactKeys(release.workflow_run, "input.evidence_template.release.workflow_run", [
    "workflow_file",
    "run_id",
    "run_attempt",
    "url",
  ]);
  return {
    repository: release.repository,
    tag: release.tag,
    channel: release.channel,
    source_sha: release.source_sha,
    main_sha: release.main_sha,
    release_target_sha: release.release_target_sha,
    release_url: release.release_url,
    app_version: parseReleaseTag(release.tag).version,
    workflow_run: structuredClone(release.workflow_run),
  };
}

function validatePublicVerification(input, template, appVersion, executionKind) {
  if (executionKind === "plan") requireVerifiedPublicReleaseReceipt(input);
  exactKeys(input, "input.public_verification", [
    "schema_version",
    "verifier",
    "disposition",
    "proof_scope",
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
  const expectedScope = executionKind === "fixture" ? "fixture_only" : "contract_only";
  if (input.disposition !== expectedDisposition || input.proof_scope !== expectedScope) {
    fail(
      "input.public_verification",
      `must be ${expectedDisposition}/${expectedScope} for ${executionKind} execution`,
    );
  }
  if (input.repository !== RELEASE_REPOSITORY || input.repository !== template.release.repository) {
    fail("input.public_verification.repository", `must equal ${RELEASE_REPOSITORY}`);
  }
  if (input.tag !== template.release.tag) {
    fail("input.public_verification.tag", "must match the evidence template");
  }
  if (!COMMIT_SHA.test(input.source_sha) || input.source_sha !== template.release.source_sha) {
    fail("input.public_verification.source_sha", "must match the exact evidence source commit");
  }
  if (input.app_version !== appVersion) {
    fail("input.public_verification.app_version", "must match the tag-derived app version");
  }

  const templateAsset = template.assets.find(
    ({ name }) => name === template.platform.package.asset_name,
  );
  if (!templateAsset) fail("input.evidence_template", "must contain the selected package asset");
  if (!Array.isArray(input.assets) || input.assets.length === 0) {
    fail("input.public_verification.assets", "must contain verified public assets");
  }
  const verifiedAsset = input.assets.find(({ name }) => name === templateAsset.name);
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
  string(verifiedAsset.sha256, "input.public_verification.assets.package.sha256", {
    max: 71,
    pattern: SHA256_DIGEST,
  });
  const expectedUrl = publicAssetUrl(input.tag, verifiedAsset.name);
  if (
    verifiedAsset.size_bytes !== templateAsset.size_bytes ||
    verifiedAsset.sha256 !== templateAsset.sha256 ||
    verifiedAsset.sha256 !== templateAsset.api_digest ||
    verifiedAsset.public_url !== expectedUrl ||
    templateAsset.public_url !== expectedUrl
  ) {
    fail("input.public_verification.assets.package", "must match the selected public asset");
  }
  return {
    name: verifiedAsset.name,
    size_bytes: verifiedAsset.size_bytes,
    sha256: verifiedAsset.sha256,
    public_url: verifiedAsset.public_url,
  };
}

function validateIsolation(isolation, contract) {
  exactKeys(isolation, "input.isolation", [
    "scope_id",
    "install_root_id",
    "user_state_root_id",
    "user_state_policy",
    "step_timeout_ms",
    "termination_timeout_ms",
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
  positiveInteger(
    isolation.termination_timeout_ms,
    "input.isolation.termination_timeout_ms",
    60_000,
  );
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
  if (interval < 500) {
    fail("input.isolation.settings_probe.sample_interval_ms", "must be at least 500");
  }
  if (isolation.degradation_scenario !== contract.degradationScenario) {
    fail(
      "input.isolation.degradation_scenario",
      `must equal ${contract.degradationScenario} for this package`,
    );
  }
  return structuredClone(isolation);
}

function requireBlankPlanChecks(template, executionKind) {
  if (executionKind !== "plan") return;
  for (const [group, id] of CHECK_ORDER) {
    if (template.checks[group][id].status !== "blocked") {
      fail(
        `input.evidence_template.checks.${group}.${id}.status`,
        "must be blocked before install-smoke planning",
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
    fail(
      "input.execution_kind",
      "must be plan or fixture; native proof is unavailable without a reviewed branded executor",
    );
  }
  const template = structuredClone(input.evidence_template);
  const expectedPacketKind =
    input.execution_kind === "fixture" ? "schema_fixture" : "release_evidence";
  if (template.packet_kind !== expectedPacketKind) {
    fail("input.evidence_template.packet_kind", `must equal ${expectedPacketKind}`);
  }
  if (input.execution_kind === "fixture") validateReleaseEvidencePacket(template);
  else validateReleaseEvidenceTemplatePacket(template);
  const release = validateReleaseIdentity(template);
  if (input.app_version !== release.app_version) {
    fail("input.app_version", `must equal ${release.app_version} from the release tag`);
  }
  requireBlankPlanChecks(template, input.execution_kind);
  const contract = platformContract(template.platform);
  const asset = validatePublicVerification(
    input.public_verification,
    template,
    input.app_version,
    input.execution_kind,
  );
  const isolation = validateIsolation(input.isolation, contract);
  return { template, release, contract, asset, isolation };
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
  const { template, release, contract, asset, isolation } = validateInput(input);
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
    observed_at_utc: template.observed_at_utc,
    release,
    platform: structuredClone(template.platform),
    asset,
    identity_receipt: createInstallSmokeIdentityReceipt(
      template.packet_id,
      template.observed_at_utc,
      release,
      template.platform,
      asset,
    ),
    public_verification: {
      schema_version: INSTALL_SMOKE_SCHEMA_VERSION,
      verifier: PUBLIC_VERIFIER,
      disposition: input.public_verification.disposition,
      proof_scope: input.public_verification.proof_scope,
      repository: input.public_verification.repository,
      tag: input.public_verification.tag,
      source_sha: input.public_verification.source_sha,
      app_version: input.public_verification.app_version,
      asset: structuredClone(asset),
    },
    isolation,
    profile: {
      package_operation: contract.packageOperation,
      install_kind: contract.installKind,
      trust_basis: contract.trustBasis,
      required_limitations: [...contract.requiredLimitations],
    },
    constraints: {
      adapter_owns_local_paths: true,
      bounded_output_required: true,
      minimal_environment_required: true,
      requires_isolated_roots: true,
      root_escape_allowed: false,
      shell_execution_allowed: false,
      symlinks_allowed: false,
      termination_handshake_required: true,
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
    "observed_at_utc",
    "release",
    "platform",
    "asset",
    "identity_receipt",
    "public_verification",
    "isolation",
    "profile",
    "constraints",
    "steps",
  ]);
  if (plan.schema_version !== INSTALL_SMOKE_SCHEMA_VERSION)
    fail("plan.schema_version", "must equal 1");
  slug(plan.plan_id, "plan.plan_id");
  if (!EXECUTION_KINDS.has(plan.execution_kind)) fail("plan.execution_kind", "is not supported");
  const contract = validateInstallSmokeIdentity(plan);
  validateIsolation(plan.isolation, contract);
  exactKeys(plan.public_verification, "plan.public_verification", [
    "schema_version",
    "verifier",
    "disposition",
    "proof_scope",
    "repository",
    "tag",
    "source_sha",
    "app_version",
    "asset",
  ]);
  const expectedDisposition = plan.execution_kind === "fixture" ? "fixture" : "passed";
  const expectedScope = plan.execution_kind === "fixture" ? "fixture_only" : "contract_only";
  if (
    plan.public_verification.schema_version !== INSTALL_SMOKE_SCHEMA_VERSION ||
    plan.public_verification.verifier !== PUBLIC_VERIFIER ||
    plan.public_verification.disposition !== expectedDisposition ||
    plan.public_verification.proof_scope !== expectedScope ||
    plan.public_verification.repository !== plan.release.repository ||
    plan.public_verification.tag !== plan.release.tag ||
    plan.public_verification.source_sha !== plan.release.source_sha ||
    plan.public_verification.app_version !== plan.release.app_version ||
    JSON.stringify(plan.public_verification.asset) !== JSON.stringify(plan.asset)
  ) {
    fail(
      "plan.public_verification",
      "must match the selected receipt-bound asset and release identity",
    );
  }
  exactKeys(plan.profile, "plan.profile", [
    "package_operation",
    "install_kind",
    "trust_basis",
    "required_limitations",
  ]);
  if (
    plan.profile.package_operation !== contract.packageOperation ||
    plan.profile.install_kind !== contract.installKind ||
    plan.profile.trust_basis !== contract.trustBasis ||
    JSON.stringify(plan.profile.required_limitations) !==
      JSON.stringify(contract.requiredLimitations)
  ) {
    fail("plan.profile", "must match the closed package profile");
  }
  exactKeys(plan.constraints, "plan.constraints", [
    "adapter_owns_local_paths",
    "bounded_output_required",
    "minimal_environment_required",
    "requires_isolated_roots",
    "root_escape_allowed",
    "shell_execution_allowed",
    "symlinks_allowed",
    "termination_handshake_required",
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
    plan.constraints.termination_handshake_required !== true ||
    plan.constraints.tokenized_argv_required !== true ||
    plan.constraints.windows_service_behavior_assumed !== false
  ) {
    fail("plan.constraints", "must preserve the closed fixture executor contract");
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
    if (step.id !== PLAN_STEP_ORDER[index])
      fail(`plan.steps[${index}].id`, `must equal ordered gate ${PLAN_STEP_ORDER[index]}`);
    if (step.action !== expectedActions[index])
      fail(`plan.steps[${index}].action`, `must equal ${expectedActions[index]}`);
    const expectedSource = index < 2 ? "public_verifier" : "adapter";
    if (step.source !== expectedSource)
      fail(`plan.steps[${index}].source`, `must equal ${expectedSource}`);
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
  RELEASE_SIGNER_WORKFLOW,
  RELEASE_SOURCE_REF,
  RESULT_STATUSES,
  deepFreeze,
  exactKeys,
  fail,
  object,
  platformContract,
  slug,
  sortedObject,
  string,
  validateInstallSmokeIdentity,
  validateInput,
});
