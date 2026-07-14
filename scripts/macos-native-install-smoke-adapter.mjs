import { requireOwnedNativeArtifactVerificationReceipt } from "./native-artifact-capability.mjs";
import {
  installSmokeContractInternals,
  validateInstallSmokePlan,
} from "./install-smoke-contract.mjs";

export const MACOS_NATIVE_ADAPTER_SOURCE_SCHEMA_VERSION = 1;

const { exactKeys, fail, platformContract } = installSmokeContractInternals;
const sourceReceipts = new WeakMap();

function deepFreeze(value) {
  if (value && typeof value === "object" && !Object.isFrozen(value)) {
    Object.freeze(value);
    for (const child of Object.values(value)) deepFreeze(child);
  }
  return value;
}

const SHARED_TOOL_IDS = Object.freeze([
  "codesign",
  "lipo",
  "plistbuddy",
  "spctl",
  "stapler",
]);

export const MACOS_NATIVE_ADAPTER_SOURCE_PROFILES = deepFreeze({
  dmg: {
    package_operation: "install",
    install_kind: "app_bundle",
    prepare_action: "install_dmg_app",
    trust_basis: "developer_id_notarization_and_staple",
    required_limitations: [],
    artifact_flow: "owned_dmg_mount_copy_required",
    source_descriptor_tool_ids: ["hdiutil", "ditto", ...SHARED_TOOL_IDS],
    future_owned_resources: [
      "private_artifact_root",
      "private_mount_point",
      "private_app_copy",
      "owned_app_processes",
      "private_user_state_root",
    ],
  },
  macos_updater: {
    package_operation: "stage",
    install_kind: "app_bundle",
    prepare_action: "stage_updater_archive_app",
    trust_basis: "contained_app_trust_and_tauri_updater",
    required_limitations: ["macos_updater_staging_only"],
    artifact_flow: "rust_owned_updater_archive_stream_required",
    source_descriptor_tool_ids: [
      "rust_owned_stream_extractor",
      ...SHARED_TOOL_IDS,
    ],
    future_owned_resources: [
      "private_artifact_root",
      "private_extraction_root",
      "private_app_stage",
      "owned_app_processes",
      "private_user_state_root",
    ],
  },
});

function selectedProfile(plan) {
  validateInstallSmokePlan(plan);
  if (plan.execution_kind !== "plan") {
    fail(
      "macos_adapter.plan",
      "must be a contract-only plan; fixtures cannot bind native adapters",
    );
  }
  if (plan.platform.os !== "macos") {
    fail("macos_adapter.platform.os", "must equal macos");
  }
  const profile =
    MACOS_NATIVE_ADAPTER_SOURCE_PROFILES[plan.platform.package.kind];
  if (!profile) {
    fail("macos_adapter.platform.package.kind", "must be dmg or macos_updater");
  }
  const contract = platformContract(plan.platform);
  const expected = {
    package_operation: contract.packageOperation,
    install_kind: contract.installKind,
    trust_basis: contract.trustBasis,
    required_limitations: [...contract.requiredLimitations],
  };
  if (JSON.stringify(plan.profile) !== JSON.stringify(expected)) {
    fail(
      "macos_adapter.profile",
      "must match the closed install-smoke platform profile",
    );
  }
  if (
    profile.package_operation !== expected.package_operation ||
    profile.install_kind !== expected.install_kind ||
    profile.trust_basis !== expected.trust_basis ||
    JSON.stringify(profile.required_limitations) !==
      JSON.stringify(expected.required_limitations)
  ) {
    fail(
      "macos_adapter.source_profile",
      "does not match the shared install-smoke contract",
    );
  }
  return profile;
}

export function bindMacosNativeAdapterSource(
  plan,
  artifactVerificationReceipt,
  ...unexpectedArguments
) {
  if (unexpectedArguments.length) {
    fail(
      "macos_adapter.binding",
      "does not accept caller commands, paths, statuses, trust, cleanup, or evidence",
    );
  }
  const profile = selectedProfile(plan);
  requireOwnedNativeArtifactVerificationReceipt(
    artifactVerificationReceipt,
    plan,
  );

  const receipt = deepFreeze({
    schema_version: MACOS_NATIVE_ADAPTER_SOURCE_SCHEMA_VERSION,
    adapter: "scripts/macos-native-install-smoke-adapter.mjs",
    proof_scope: "macos_adapter_source_contract_only",
    plan_id: plan.plan_id,
    asset: structuredClone(artifactVerificationReceipt.asset),
    profile: structuredClone(profile),
    timeouts: {
      step_timeout_ms: plan.isolation.step_timeout_ms,
      termination_timeout_ms: plan.isolation.termination_timeout_ms,
      settlement_required: true,
    },
    claims: {
      verified_asset_identity_bound: true,
      live_capability_held: false,
      descriptor_only: true,
      package_consumed: false,
      process_executed: false,
      trust_verified: false,
      runtime_executed: false,
      cleanup_proven: false,
      native_proven: false,
      release_evidence_emitted: false,
    },
  });
  sourceReceipts.set(receipt, {
    plan,
    planIdentityReceipt: plan.identity_receipt,
    artifactVerificationReceipt,
  });
  return receipt;
}

export function requireMacosNativeAdapterSourceReceipt(
  receipt,
  plan,
  artifactVerificationReceipt,
) {
  const profile = selectedProfile(plan);
  requireOwnedNativeArtifactVerificationReceipt(
    artifactVerificationReceipt,
    plan,
  );
  const state =
    receipt && typeof receipt === "object"
      ? sourceReceipts.get(receipt)
      : undefined;
  if (!state) {
    fail(
      "macos_adapter.source_receipt",
      "must come from the process-local closed macOS source adapter",
    );
  }
  if (
    state.plan !== plan ||
    state.planIdentityReceipt !== plan.identity_receipt ||
    state.artifactVerificationReceipt !== artifactVerificationReceipt
  ) {
    fail(
      "macos_adapter.source_receipt",
      "must retain the exact process-local plan and artifact verification receipt identities",
    );
  }
  exactKeys(receipt, "macos_adapter.source_receipt", [
    "schema_version",
    "adapter",
    "proof_scope",
    "plan_id",
    "asset",
    "profile",
    "timeouts",
    "claims",
  ]);
  if (
    receipt.schema_version !== MACOS_NATIVE_ADAPTER_SOURCE_SCHEMA_VERSION ||
    receipt.adapter !== "scripts/macos-native-install-smoke-adapter.mjs" ||
    receipt.proof_scope !== "macos_adapter_source_contract_only" ||
    receipt.plan_id !== plan.plan_id ||
    JSON.stringify(receipt.asset) !==
      JSON.stringify(artifactVerificationReceipt.asset) ||
    JSON.stringify(receipt.profile) !== JSON.stringify(profile)
  ) {
    fail(
      "macos_adapter.source_receipt",
      "does not bind the exact plan, artifact, and profile",
    );
  }
  return receipt;
}
