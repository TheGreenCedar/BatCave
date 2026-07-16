use crate::collector_service::etw_lease::{
    EtwLeasePhase, EtwLeaseV1, EtwSessionIdentityV1, ETW_LEASE_SCHEMA_VERSION,
};
#[cfg(test)]
use crate::windows_lifecycle_proof_contract::DesktopPhaseObservation;
use crate::windows_lifecycle_proof_contract::{
    plan_sha256, valid_service_instance_id, validate_desktop_phase_result,
    validate_desktop_visible, validate_sha256, DesktopCollectorRuntimeObservation,
    DesktopFileObservation, DesktopPhase, DesktopPhaseDisposition, DesktopPhaseResult,
    DesktopProcessObservation, DesktopSecondInstanceObservation, DesktopServiceProcessObservation,
    DesktopVisibleObservation, EvidenceReceipt, LifecycleStage, Observation, ProofPlan,
    SUCCESS_PRIVATE_EVIDENCE_LEAVES,
};
use crate::windows_network::NetworkAttributionMonitor;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::cmp::Reverse;
use std::collections::{BTreeMap, BTreeSet};

pub(super) const SANITIZED_SCHEMA: &str = "batcave_windows_lifecycle_sanitized_v2";
const MAX_EVIDENCE_SIZE: u64 = 8 * 1024 * 1024;
const PRIVATE_EVIDENCE_PROJECTION_READY: bool = false;
const KNOWN_RETIRED_HELPER_LEAVES: [&str; 8] = [
    "elevated-helper/snapshot.json",
    "elevated-helper/snapshot.json.tmp",
    "elevated-helper/stop.signal",
    "elevated-helper/accepted.signal",
    "elevated-helper/run-dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd/snapshot.json",
    "elevated-helper/run-dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd/snapshot.json.tmp",
    "elevated-helper/run-dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd/stop.signal",
    "elevated-helper/run-dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd/accepted.signal",
];
const UNKNOWN_HELPER_SENTINEL_LEAF: &str = "elevated-helper/unknown-sentinel.bin";
const RETAINED_SETTINGS_LEAF: &str = "settings.json";
const RETAINED_CACHE_LEAF: &str = "warm-cache.json";
const RETAINED_DIAGNOSTICS_LEAF: &str = "diagnostics.jsonl";

pub(super) fn require_private_evidence_projection_ready() -> Result<(), String> {
    if PRIVATE_EVIDENCE_PROJECTION_READY {
        Ok(())
    } else {
        Err("lifecycle_private_evidence_projection_not_reviewed".to_string())
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum LogicalRoot {
    Install,
    ProductData,
    ServiceData,
    CurrentUserData,
    PublicDesktop,
    CommonStartMenu,
    Evidence,
    ProofArtifact,
    WebViewRuntime,
    Windows,
    Hklm,
    Hkcu,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct LogicalPath {
    pub(super) root: LogicalRoot,
    pub(super) relative_leaf: String,
}

#[derive(Clone, Debug)]
pub(super) struct SanitizationRoot {
    logical: LogicalRoot,
    canonical: String,
}

impl SanitizationRoot {
    pub(super) fn new(logical: LogicalRoot, canonical: &str) -> Result<Self, String> {
        let canonical = normalize_absolute_windows_path(canonical)?;
        if canonical.ends_with('\\') {
            return Err("lifecycle_sanitization_root_trailing_separator".to_string());
        }
        Ok(Self { logical, canonical })
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct SanitizedExportPacket {
    schema_version: String,
    profile: String,
    plan_sha256: String,
    controller_source_commit_sha: String,
    controller_sha256: String,
    completed_stage: LifecycleStage,
    process_tree_settled: bool,
    private_evidence: Vec<SanitizedPrivateEvidence>,
    final_product_absent: bool,
    current_user_data_preserved: bool,
    current_user_retention: SanitizedCurrentUserRetention,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct SanitizedPrivateEvidence {
    receipt: EvidenceReceipt,
    assertion: SanitizedEvidenceAssertion,
    machine: SanitizedMachineSnapshot,
    desktop_phase: Option<SanitizedDesktopPhaseObservation>,
    event: Option<SanitizedStageEvent>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
enum SanitizedEvidenceAssertion {
    InitialLegacyStopped,
    FinalInstalledRunning,
    ProductAbsent,
    BaselineInstalledRunning,
    BaselineStopped,
    BaselineCrashed,
    BaselineRecovered,
    BaselineRollbackRecovered,
    LegacyResidueSeeded,
    FinalUpgradedRunning,
    FinalStopped,
    FinalCrashed,
    FinalRecovered,
    FinalMissingService,
    FinalIncompatibleRunning,
    FinalUninstalledPreservingCurrentUserData,
    DesktopPhase,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct SanitizedMachineSnapshot {
    service: Observation<SanitizedServiceSnapshot>,
    install_root: Observation<SanitizedDirectorySnapshot>,
    monitor: Observation<SanitizedFileSnapshot>,
    service_binary: Observation<SanitizedFileSnapshot>,
    uninstaller: Observation<SanitizedFileSnapshot>,
    legacy_cli: Observation<SanitizedFileSnapshot>,
    uninstall_registry: Observation<SanitizedRegistrySnapshot>,
    product_processes: Observation<Vec<SanitizedProcessSnapshot>>,
    product_data_root: Observation<SanitizedDirectorySnapshot>,
    service_data_root: Observation<SanitizedDirectorySnapshot>,
    current_user_data_root: Observation<SanitizedDirectorySnapshot>,
    installed_boundaries: Observation<SanitizedBoundarySnapshot>,
    service_registry_key: Option<LogicalPath>,
    named_pipe: Option<SanitizedNamedPipeSnapshot>,
    etw_session: Option<SanitizedEtwObservation>,
    etw_lease_file: Option<LogicalPath>,
    etw_owner_lock: Option<LogicalPath>,
    service_lifecycle_lock: Option<LogicalPath>,
    upgrade_transaction_journal: Option<SanitizedPathFileSnapshot>,
    staged_service_images: Vec<SanitizedPathFileSnapshot>,
    rollback_service_images: Vec<SanitizedPathFileSnapshot>,
    atomic_temporary_files: Vec<SanitizedPathFileSnapshot>,
    failure_marker: Option<SanitizedPathFileSnapshot>,
    machine_product_key: Option<LogicalPath>,
    hkcu_autostart: Option<SanitizedRegistryValueSnapshot>,
    public_desktop_shortcut: Option<SanitizedShortcutSnapshot>,
    common_start_menu_shortcut: Option<SanitizedShortcutSnapshot>,
    known_retired_helper_artifacts: Vec<SanitizedPathFileSnapshot>,
    unknown_helper_sentinel: Option<SanitizedPathFileSnapshot>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct SanitizedServiceSnapshot {
    state: u32,
    process_id: u32,
    process_started_at_100ns: Option<u64>,
    win32_exit_code: u32,
    service_specific_exit_code: u32,
    image_path: LogicalPath,
    image_sha256: String,
    local_system: bool,
    own_process: bool,
    automatic_start: bool,
    recovery_restart_action_count: u8,
    owner_marker: String,
    service_dacl_sha256: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct SanitizedFileSnapshot {
    size: u64,
    sha256: String,
    volume_serial: u32,
    file_index: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct SanitizedDirectorySnapshot {
    volume_serial: u32,
    file_index: u64,
    final_path: LogicalPath,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct SanitizedRegistrySnapshot {
    view: String,
    key: LogicalPath,
    install_location: LogicalPath,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct SanitizedProcessSnapshot {
    process_id: u32,
    executable_name: String,
    executable_path: Option<LogicalPath>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
enum SanitizedPrincipal {
    LocalSystem,
    Administrators,
    InteractiveUsers,
    CollectorService,
}

#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(deny_unknown_fields)]
struct SanitizedAceSnapshot {
    principal: SanitizedPrincipal,
    allow: bool,
    inherit_only: bool,
    object_inherit: bool,
    container_inherit: bool,
    mask: u32,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct SanitizedBoundarySnapshot {
    service_dacl_sha256: String,
    service_aces: Vec<SanitizedAceSnapshot>,
    service_data_root_owner: SanitizedPrincipal,
    service_data_root_dacl_protected: bool,
    service_data_root_reparse: bool,
    service_data_root_dacl_sha256: String,
    service_data_root_aces: Vec<SanitizedAceSnapshot>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct SanitizedNamedPipeSnapshot {
    server_process_id: u32,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct SanitizedEtwObservation {
    lease: EtwLeaseV1,
    observed_session: EtwSessionIdentityV1,
    owner_lock_held: bool,
    process_lock_held: bool,
    events_lost: u64,
    buffers_lost: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct SanitizedPathFileSnapshot {
    path: LogicalPath,
    file: SanitizedFileSnapshot,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct SanitizedRegistryValueSnapshot {
    key: LogicalPath,
    value_name: String,
    target: LogicalPath,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct SanitizedShortcutSnapshot {
    path: LogicalPath,
    target: LogicalPath,
    sha256: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct SanitizedCurrentUserRetention {
    before_uninstall_source: EvidenceReceipt,
    after_uninstall_source: EvidenceReceipt,
    settings: SanitizedRetainedUserObject,
    cache: SanitizedRetainedUserObject,
    diagnostics: SanitizedRetainedUserObject,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct SanitizedRetainedUserObject {
    path: LogicalPath,
    before_uninstall: Observation<SanitizedDigestSnapshot>,
    after_uninstall: Observation<SanitizedDigestSnapshot>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct SanitizedDigestSnapshot {
    size: u64,
    sha256: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case", tag = "kind", content = "value")]
enum SanitizedStageEvent {
    ServiceCrash(SanitizedServiceCrashEvent),
    UpgradeRollback(SanitizedUpgradeRollbackEvent),
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct SanitizedServiceCrashEvent {
    process_id: u32,
    process_started_at_100ns: u64,
    image_path: LogicalPath,
    image_sha256: String,
    process_exit_code: u32,
    win32_exit_code: u32,
    service_specific_exit_code: u32,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct SanitizedUpgradeRollbackEvent {
    candidate_sha256: String,
    candidate_failure_code: String,
    candidate_failure_detail: String,
    execution_marker_sha256: String,
    restored_sha256: String,
    restored_process_id: u32,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct SanitizedDesktopProcessObservation {
    process_id: u32,
    parent_process_id: Option<u32>,
    started_at_100ns: u64,
    session_id: u32,
    elevated: bool,
    executable_path: LogicalPath,
    executable_size: u64,
    executable_sha256: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct SanitizedDesktopSecondInstanceObservation {
    attempted_process: SanitizedDesktopProcessObservation,
    terminal_exit_code: u32,
    process_tree_settled: bool,
    focused_primary_process_id: u32,
    focused_primary_started_at_100ns: u64,
    service_instance_id_before: String,
    service_instance_id_after: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct SanitizedDesktopFileObservation {
    executable_path: LogicalPath,
    executable_size: u64,
    executable_sha256: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct SanitizedDesktopServiceProcessObservation {
    process_id: u32,
    started_at_100ns: u64,
    local_system: bool,
    executable_path: LogicalPath,
    executable_size: u64,
    executable_sha256: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct SanitizedDesktopCollectorRuntimeObservation {
    installed_service: Option<SanitizedDesktopFileObservation>,
    service_process: Option<SanitizedDesktopServiceProcessObservation>,
    pipe_server_process_id: Option<u32>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct SanitizedDesktopPhaseObservation {
    phase: DesktopPhase,
    process_tree_settled: bool,
    desktop: SanitizedDesktopProcessObservation,
    process_tree: Vec<SanitizedDesktopProcessObservation>,
    webview_process_ids: Vec<u32>,
    second_instance: Option<SanitizedDesktopSecondInstanceObservation>,
    collector_runtime: SanitizedDesktopCollectorRuntimeObservation,
    visible: DesktopVisibleObservation,
}

pub(super) fn sanitize_path(
    value: &str,
    roots: &[SanitizationRoot],
) -> Result<LogicalPath, String> {
    let normalized = normalize_absolute_windows_path(value)?;
    let mut matches = roots
        .iter()
        .filter_map(|root| {
            strip_windows_root(&normalized, &root.canonical)
                .map(|relative_leaf| (root.canonical.len(), root.logical, relative_leaf))
        })
        .collect::<Vec<_>>();
    matches.sort_by_key(|entry| Reverse(entry.0));
    let Some((_, logical, relative_leaf)) = matches.into_iter().next() else {
        return Err("lifecycle_sanitized_path_outside_allowlist".to_string());
    };
    validate_relative_leaf(&relative_leaf)?;
    Ok(LogicalPath {
        root: logical,
        relative_leaf: relative_leaf.replace('\\', "/"),
    })
}

pub(super) fn validate_sanitized_export_bytes(
    bytes: &[u8],
    plan: &ProofPlan,
    controller_source_commit_sha: &str,
    controller_sha256: &str,
    expected_receipts: &[EvidenceReceipt],
) -> Result<(), String> {
    if bytes.is_empty() || bytes.len() as u64 > MAX_EVIDENCE_SIZE {
        return Err("lifecycle_sanitized_export_size_invalid".to_string());
    }
    let packet: SanitizedExportPacket = serde_json::from_slice(bytes)
        .map_err(|_| "lifecycle_sanitized_export_json_invalid".to_string())?;
    if packet.schema_version != SANITIZED_SCHEMA
        || packet.profile != plan.profile
        || packet.plan_sha256 != plan_sha256()
        || packet.controller_source_commit_sha != controller_source_commit_sha
        || packet.controller_sha256 != controller_sha256
        || packet.completed_stage != LifecycleStage::FinalUninstall
        || !packet.process_tree_settled
        || !packet.final_product_absent
        || !packet.current_user_data_preserved
    {
        return Err("lifecycle_sanitized_export_shape_invalid".to_string());
    }
    validate_sha256(&packet.plan_sha256, "sanitized_plan")?;
    validate_sha256(&packet.controller_sha256, "sanitized_controller")?;
    if packet.controller_source_commit_sha.len() != 40
        || !packet
            .controller_source_commit_sha
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit())
    {
        return Err("lifecycle_sanitized_export_source_commit_invalid".to_string());
    }
    validate_sanitized_private_evidence(&packet.private_evidence, expected_receipts, plan)?;
    validate_current_user_retention(&packet.current_user_retention)?;
    validate_current_user_retention_sources(
        &packet.current_user_retention,
        &packet.private_evidence,
    )?;
    if !current_user_retention_preserved(&packet.current_user_retention) {
        return Err("lifecycle_sanitized_user_retention_invalid".to_string());
    }
    let final_packet = packet
        .private_evidence
        .iter()
        .find(|entry| entry.receipt.name == "final-uninstall-state.private.json")
        .ok_or_else(|| "lifecycle_sanitized_export_final_state_missing".to_string())?;
    validate_final_absence(&final_packet.machine)
}

pub(super) fn validate_sanitized_export_bytes_with_parent_results(
    bytes: &[u8],
    plan: &ProofPlan,
    controller_source_commit_sha: &str,
    controller_sha256: &str,
    expected_receipts: &[EvidenceReceipt],
    parent_desktop_results: &[DesktopPhaseResult],
) -> Result<(), String> {
    validate_sanitized_export_bytes(
        bytes,
        plan,
        controller_source_commit_sha,
        controller_sha256,
        expected_receipts,
    )?;
    let packet: SanitizedExportPacket = serde_json::from_slice(bytes)
        .map_err(|_| "lifecycle_sanitized_export_json_invalid".to_string())?;
    validate_parent_desktop_results(&packet.private_evidence, parent_desktop_results, plan)
}

fn validate_sanitized_private_evidence(
    entries: &[SanitizedPrivateEvidence],
    expected_receipts: &[EvidenceReceipt],
    plan: &ProofPlan,
) -> Result<(), String> {
    if entries.len() != SUCCESS_PRIVATE_EVIDENCE_LEAVES.len()
        || expected_receipts.len() != SUCCESS_PRIVATE_EVIDENCE_LEAVES.len()
    {
        return Err("lifecycle_sanitized_export_manifest_incomplete".to_string());
    }
    let expected = expected_receipts
        .iter()
        .map(|receipt| (receipt.name.as_str(), receipt))
        .collect::<BTreeMap<_, _>>();
    if expected.len() != SUCCESS_PRIVATE_EVIDENCE_LEAVES.len()
        || expected.keys().copied().collect::<BTreeSet<_>>()
            != SUCCESS_PRIVATE_EVIDENCE_LEAVES
                .into_iter()
                .collect::<BTreeSet<_>>()
    {
        return Err("lifecycle_sanitized_export_expected_manifest_invalid".to_string());
    }

    let mut names = BTreeSet::new();
    let mut phases = BTreeSet::new();
    for entry in entries {
        let name = entry.receipt.name.as_str();
        if !names.insert(name)
            || expected.get(name).copied() != Some(&entry.receipt)
            || entry.receipt.size == 0
            || entry.receipt.size > MAX_EVIDENCE_SIZE
            || validate_sha256(&entry.receipt.sha256, "sanitized_private_evidence").is_err()
        {
            return Err("lifecycle_sanitized_export_receipt_invalid".to_string());
        }
        validate_machine_snapshot(&entry.machine)?;
        if entry.assertion != assertion_for_leaf(name) {
            return Err("lifecycle_sanitized_stage_assertion_invalid".to_string());
        }
        validate_stage_machine_assertion(entry.assertion, &entry.machine, plan)?;

        if let Some(phase) = desktop_phase_for_leaf(name) {
            let desktop = entry
                .desktop_phase
                .as_ref()
                .ok_or_else(|| "lifecycle_sanitized_desktop_phase_missing".to_string())?;
            if entry.event.is_some() || desktop.phase != phase || !phases.insert(phase) {
                return Err("lifecycle_sanitized_desktop_phase_invalid".to_string());
            }
            validate_sanitized_desktop_phase(desktop, plan)?;
            validate_desktop_machine_state(phase, &entry.machine, plan)?;
            validate_desktop_machine_runtime(phase, desktop, &entry.machine)?;
        } else if entry.desktop_phase.is_some() {
            return Err("lifecycle_sanitized_desktop_phase_unexpected".to_string());
        } else {
            validate_stage_event(name, entry.event.as_ref(), &entry.machine, plan)?;
        }
    }
    let by_name = entries
        .iter()
        .map(|entry| (entry.receipt.name.as_str(), entry))
        .collect::<BTreeMap<_, _>>();
    let seeded_sentinel = by_name
        .get("legacy-residue-seeded-state.private.json")
        .and_then(|entry| entry.machine.unknown_helper_sentinel.as_ref())
        .ok_or_else(|| "lifecycle_sanitized_unknown_helper_sentinel_missing".to_string())?;
    for (index, leaf) in SUCCESS_PRIVATE_EVIDENCE_LEAVES.iter().enumerate() {
        let entry = by_name
            .get(leaf)
            .ok_or_else(|| "lifecycle_sanitized_export_manifest_incomplete".to_string())?;
        let is_legacy_seed = *leaf == "legacy-residue-seeded-state.private.json";
        let has_known_retired_helpers = !entry.machine.known_retired_helper_artifacts.is_empty();
        if is_legacy_seed != has_known_retired_helpers {
            return Err("lifecycle_sanitized_legacy_helper_lifetime_invalid".to_string());
        }
        if index < 12 {
            if entry.machine.unknown_helper_sentinel.is_some() {
                return Err("lifecycle_sanitized_unknown_helper_sentinel_early".to_string());
            }
        } else if entry.machine.unknown_helper_sentinel.as_ref() != Some(seeded_sentinel) {
            return Err("lifecycle_sanitized_unknown_helper_sentinel_drift".to_string());
        }
    }
    validate_lifecycle_continuity(&by_name)?;
    if phases
        != [
            DesktopPhase::FinalPrimary,
            DesktopPhase::BaselinePrimary,
            DesktopPhase::BaselineSecondInstance,
            DesktopPhase::FinalMissingService,
            DesktopPhase::FinalStoppedService,
            DesktopPhase::FinalIncompatibleService,
        ]
        .into_iter()
        .collect()
    {
        return Err("lifecycle_sanitized_desktop_manifest_incomplete".to_string());
    }
    Ok(())
}

fn validate_desktop_machine_runtime(
    phase: DesktopPhase,
    desktop: &SanitizedDesktopPhaseObservation,
    machine: &SanitizedMachineSnapshot,
) -> Result<(), String> {
    let runtime = &desktop.collector_runtime;
    match phase {
        DesktopPhase::FinalMissingService => {
            if runtime.installed_service.is_some()
                || runtime.service_process.is_some()
                || runtime.pipe_server_process_id.is_some()
                || !matches!(machine.service, Observation::Absent)
                || !matches!(machine.service_binary, Observation::Absent)
                || machine.named_pipe.is_some()
                || machine.etw_session.is_some()
            {
                return Err("lifecycle_sanitized_desktop_machine_runtime_mismatch".to_string());
            }
        }
        DesktopPhase::FinalStoppedService => {
            let service = machine.service.as_present().ok_or_else(|| {
                "lifecycle_sanitized_desktop_machine_runtime_mismatch".to_string()
            })?;
            let service_file = machine.service_binary.as_present().ok_or_else(|| {
                "lifecycle_sanitized_desktop_machine_runtime_mismatch".to_string()
            })?;
            let installed = runtime.installed_service.as_ref().ok_or_else(|| {
                "lifecycle_sanitized_desktop_machine_runtime_mismatch".to_string()
            })?;
            if service.state != 1
                || service.process_id != 0
                || installed.executable_path != service.image_path
                || installed.executable_size != service_file.size
                || installed.executable_sha256 != service_file.sha256
                || runtime.service_process.is_some()
                || runtime.pipe_server_process_id.is_some()
                || machine.named_pipe.is_some()
                || machine.etw_session.is_some()
            {
                return Err("lifecycle_sanitized_desktop_machine_runtime_mismatch".to_string());
            }
        }
        DesktopPhase::FinalPrimary
        | DesktopPhase::BaselinePrimary
        | DesktopPhase::BaselineSecondInstance
        | DesktopPhase::FinalIncompatibleService => {
            let service = machine.service.as_present().ok_or_else(|| {
                "lifecycle_sanitized_desktop_machine_runtime_mismatch".to_string()
            })?;
            let service_file = machine.service_binary.as_present().ok_or_else(|| {
                "lifecycle_sanitized_desktop_machine_runtime_mismatch".to_string()
            })?;
            let installed = runtime.installed_service.as_ref().ok_or_else(|| {
                "lifecycle_sanitized_desktop_machine_runtime_mismatch".to_string()
            })?;
            let process = runtime.service_process.as_ref().ok_or_else(|| {
                "lifecycle_sanitized_desktop_machine_runtime_mismatch".to_string()
            })?;
            let etw = machine.etw_session.as_ref().ok_or_else(|| {
                "lifecycle_sanitized_desktop_machine_runtime_mismatch".to_string()
            })?;
            if service.state != 4
                || service.process_id != process.process_id
                || service.process_started_at_100ns != Some(process.started_at_100ns)
                || service.image_path != process.executable_path
                || service.image_sha256 != process.executable_sha256
                || installed.executable_path != service.image_path
                || installed.executable_size != service_file.size
                || installed.executable_sha256 != service_file.sha256
                || process.executable_size != service_file.size
                || machine
                    .named_pipe
                    .as_ref()
                    .map(|pipe| pipe.server_process_id)
                    != runtime.pipe_server_process_id
                || desktop
                    .visible
                    .service_instance_id
                    .as_deref()
                    .is_some_and(|instance| {
                        digest16(instance.as_bytes()) != etw.lease.service_instance_id
                    })
            {
                return Err("lifecycle_sanitized_desktop_machine_runtime_mismatch".to_string());
            }
        }
    }
    Ok(())
}

#[derive(Debug, Eq, PartialEq)]
struct ServiceGeneration {
    process_id: u32,
    started_at_100ns: u64,
    service_instance_id: [u8; 16],
    image_path: LogicalPath,
    image_sha256: String,
}

fn validate_lifecycle_continuity(
    entries: &BTreeMap<&str, &SanitizedPrivateEvidence>,
) -> Result<(), String> {
    let mut installed_boundary = None;
    let mut run_etw_identity = None;
    let mut pre_uninstall_install_id = None;
    let mut post_reinstall_install_id = None;
    let mut post_uninstall_epoch = false;
    for receipt_name in SUCCESS_PRIVATE_EVIDENCE_LEAVES {
        let entry = required_entry(entries, receipt_name)?;
        if let Observation::Present(boundary) = &entry.machine.installed_boundaries {
            if installed_boundary.is_none() {
                installed_boundary = Some(boundary);
            } else if installed_boundary != Some(boundary) {
                return Err("lifecycle_sanitized_security_boundary_drift".to_string());
            }
        }
        if let Some(etw) = &entry.machine.etw_session {
            let identity = (etw.lease.boot_identity, &etw.lease.session);
            if run_etw_identity.is_none() {
                run_etw_identity = Some(identity);
            } else if run_etw_identity != Some(identity) {
                return Err("lifecycle_sanitized_etw_run_identity_drift".to_string());
            }
            let epoch_install_id = if post_uninstall_epoch {
                &mut post_reinstall_install_id
            } else {
                &mut pre_uninstall_install_id
            };
            if epoch_install_id.is_none() {
                *epoch_install_id = Some(etw.lease.install_id);
            } else if *epoch_install_id != Some(etw.lease.install_id) {
                return Err("lifecycle_sanitized_etw_install_epoch_drift".to_string());
            }
        }
        if receipt_name == "initial-uninstall-state.private.json" {
            post_uninstall_epoch = true;
        }
    }
    for (state, desktop) in [
        (
            "final-repair-state.private.json",
            "final-primary-desktop.private.json",
        ),
        (
            "baseline-install-state.private.json",
            "baseline-primary-desktop.private.json",
        ),
        (
            "baseline-primary-desktop.private.json",
            "baseline-second-instance-desktop.private.json",
        ),
        (
            "final-missing-service-state.private.json",
            "final-missing-service-desktop.private.json",
        ),
        (
            "final-stopped-service-state.private.json",
            "final-stopped-service-desktop.private.json",
        ),
        (
            "final-incompatible-service-state.private.json",
            "final-incompatible-service-desktop.private.json",
        ),
    ] {
        require_machine_runtime_equal(
            &required_entry(entries, state)?.machine,
            &required_entry(entries, desktop)?.machine,
        )?;
    }

    let baseline_install = running_generation(
        &required_entry(entries, "baseline-install-state.private.json")?.machine,
    )?;
    let baseline_restart = running_generation(
        &required_entry(entries, "baseline-restart-state.private.json")?.machine,
    )?;
    require_new_generation(&baseline_install, &baseline_restart)?;
    require_crash_matches_generation(
        required_entry(entries, "baseline-crashed-state.private.json")?,
        &baseline_restart,
    )?;
    let baseline_recovered = running_generation(
        &required_entry(entries, "baseline-crash-recovery-state.private.json")?.machine,
    )?;
    require_new_generation(&baseline_restart, &baseline_recovered)?;
    let baseline_rollback = running_generation(
        &required_entry(entries, "baseline-rollback-recovery-state.private.json")?.machine,
    )?;
    require_new_generation(&baseline_recovered, &baseline_rollback)?;
    let seeded = running_generation(
        &required_entry(entries, "legacy-residue-seeded-state.private.json")?.machine,
    )?;
    if seeded != baseline_rollback {
        return Err("lifecycle_sanitized_generation_continuity_invalid".to_string());
    }

    let final_upgrade =
        running_generation(&required_entry(entries, "final-upgrade-state.private.json")?.machine)?;
    require_new_generation(&seeded, &final_upgrade)?;
    let final_restart =
        running_generation(&required_entry(entries, "final-restart-state.private.json")?.machine)?;
    require_new_generation(&final_upgrade, &final_restart)?;
    require_crash_matches_generation(
        required_entry(entries, "final-crashed-state.private.json")?,
        &final_restart,
    )?;
    let final_recovered = running_generation(
        &required_entry(entries, "final-crash-recovery-state.private.json")?.machine,
    )?;
    require_new_generation(&final_restart, &final_recovered)?;
    let missing_restored = running_generation(
        &required_entry(entries, "final-missing-service-restored-state.private.json")?.machine,
    )?;
    require_new_generation(&final_recovered, &missing_restored)?;
    let stopped_restored = running_generation(
        &required_entry(entries, "final-stopped-service-restored-state.private.json")?.machine,
    )?;
    require_new_generation(&missing_restored, &stopped_restored)?;
    let incompatible = running_generation(
        &required_entry(entries, "final-incompatible-service-state.private.json")?.machine,
    )?;
    require_new_generation(&stopped_restored, &incompatible)?;
    let incompatible_restored = running_generation(
        &required_entry(
            entries,
            "final-incompatible-service-restored-state.private.json",
        )?
        .machine,
    )?;
    require_new_generation(&incompatible, &incompatible_restored)
}

fn required_entry<'a>(
    entries: &'a BTreeMap<&str, &'a SanitizedPrivateEvidence>,
    name: &str,
) -> Result<&'a SanitizedPrivateEvidence, String> {
    entries
        .get(name)
        .copied()
        .ok_or_else(|| "lifecycle_sanitized_export_manifest_incomplete".to_string())
}

fn running_generation(machine: &SanitizedMachineSnapshot) -> Result<ServiceGeneration, String> {
    let service = machine
        .service
        .as_present()
        .ok_or_else(|| "lifecycle_sanitized_generation_missing".to_string())?;
    let etw = machine
        .etw_session
        .as_ref()
        .ok_or_else(|| "lifecycle_sanitized_generation_missing".to_string())?;
    if service.state != 4
        || service.process_id == 0
        || service.process_started_at_100ns.is_none()
        || etw.lease.controller.process_id != service.process_id
        || etw.lease.controller.process_started_at
            != service.process_started_at_100ns.unwrap_or_default()
        || etw.lease.service_generation != sha256_digest16(&service.image_sha256)?
    {
        return Err("lifecycle_sanitized_generation_invalid".to_string());
    }
    Ok(ServiceGeneration {
        process_id: service.process_id,
        started_at_100ns: service.process_started_at_100ns.unwrap_or_default(),
        service_instance_id: etw.lease.service_instance_id,
        image_path: service.image_path.clone(),
        image_sha256: service.image_sha256.clone(),
    })
}

fn require_new_generation(
    before: &ServiceGeneration,
    after: &ServiceGeneration,
) -> Result<(), String> {
    require_new_service_process(before, after)?;
    if before.service_instance_id == after.service_instance_id {
        return Err("lifecycle_sanitized_generation_continuity_invalid".to_string());
    }
    Ok(())
}

fn require_new_service_process(
    before: &ServiceGeneration,
    after: &ServiceGeneration,
) -> Result<(), String> {
    if (before.process_id, before.started_at_100ns) == (after.process_id, after.started_at_100ns) {
        return Err("lifecycle_sanitized_generation_continuity_invalid".to_string());
    }
    Ok(())
}

fn require_crash_matches_generation(
    crashed: &SanitizedPrivateEvidence,
    running: &ServiceGeneration,
) -> Result<(), String> {
    let Some(SanitizedStageEvent::ServiceCrash(event)) = crashed.event.as_ref() else {
        return Err("lifecycle_sanitized_service_crash_invalid".to_string());
    };
    if event.process_id != running.process_id
        || event.process_started_at_100ns != running.started_at_100ns
        || event.image_path != running.image_path
        || event.image_sha256 != running.image_sha256
    {
        return Err("lifecycle_sanitized_service_crash_generation_mismatch".to_string());
    }
    Ok(())
}

fn require_machine_runtime_equal(
    left: &SanitizedMachineSnapshot,
    right: &SanitizedMachineSnapshot,
) -> Result<(), String> {
    if left.service != right.service
        || left.service_binary != right.service_binary
        || left.installed_boundaries != right.installed_boundaries
        || left.service_registry_key != right.service_registry_key
        || left.named_pipe != right.named_pipe
        || left.etw_session != right.etw_session
        || left.etw_lease_file != right.etw_lease_file
        || left.etw_owner_lock != right.etw_owner_lock
        || left.service_lifecycle_lock != right.service_lifecycle_lock
        || left.failure_marker != right.failure_marker
    {
        return Err("lifecycle_sanitized_machine_runtime_continuity_invalid".to_string());
    }
    Ok(())
}

fn validate_parent_desktop_results(
    entries: &[SanitizedPrivateEvidence],
    parent_results: &[DesktopPhaseResult],
    plan: &ProofPlan,
) -> Result<(), String> {
    let expected_phases = [
        DesktopPhase::FinalPrimary,
        DesktopPhase::BaselinePrimary,
        DesktopPhase::BaselineSecondInstance,
        DesktopPhase::FinalMissingService,
        DesktopPhase::FinalStoppedService,
        DesktopPhase::FinalIncompatibleService,
    ];
    if parent_results.len() != expected_phases.len() {
        return Err("lifecycle_sanitized_parent_desktop_manifest_incomplete".to_string());
    }
    let by_phase = entries
        .iter()
        .filter_map(|entry| {
            entry
                .desktop_phase
                .as_ref()
                .map(|desktop| (desktop.phase, desktop))
        })
        .collect::<BTreeMap<_, _>>();
    for (expected_phase, result) in expected_phases.into_iter().zip(parent_results) {
        if result.phase != expected_phase || result.disposition != DesktopPhaseDisposition::Passed {
            return Err("lifecycle_sanitized_parent_desktop_result_invalid".to_string());
        }
        validate_desktop_phase_result(result, plan)?;
        let expected = sanitize_parent_desktop_result(result)?;
        if by_phase.get(&expected_phase).copied() != Some(&expected) {
            return Err("lifecycle_sanitized_parent_desktop_result_mismatch".to_string());
        }
    }
    Ok(())
}

fn sanitize_parent_desktop_result(
    result: &DesktopPhaseResult,
) -> Result<SanitizedDesktopPhaseObservation, String> {
    let observation = result
        .observation
        .as_ref()
        .ok_or_else(|| "lifecycle_sanitized_parent_desktop_observation_missing".to_string())?;
    let roots = [
        SanitizationRoot::new(LogicalRoot::Install, r"C:\Program Files\BatCave Monitor")?,
        SanitizationRoot::new(
            LogicalRoot::WebViewRuntime,
            r"C:\Program Files (x86)\Microsoft\EdgeWebView\Application",
        )?,
    ];
    Ok(SanitizedDesktopPhaseObservation {
        phase: result.phase,
        process_tree_settled: result.process_tree_settled,
        desktop: sanitize_parent_desktop_process(&observation.desktop, &roots)?,
        process_tree: observation
            .process_tree
            .iter()
            .map(|process| sanitize_parent_desktop_process(process, &roots))
            .collect::<Result<_, _>>()?,
        webview_process_ids: observation.webview_process_ids.clone(),
        second_instance: observation
            .second_instance
            .as_ref()
            .map(|second| sanitize_parent_second_instance(second, &roots))
            .transpose()?,
        collector_runtime: sanitize_parent_collector_runtime(
            &observation.collector_runtime,
            &roots,
        )?,
        visible: observation.visible.clone(),
    })
}

fn sanitize_parent_desktop_process(
    process: &DesktopProcessObservation,
    roots: &[SanitizationRoot],
) -> Result<SanitizedDesktopProcessObservation, String> {
    Ok(SanitizedDesktopProcessObservation {
        process_id: process.process_id,
        parent_process_id: process.parent_process_id,
        started_at_100ns: process.started_at_100ns,
        session_id: process.session_id,
        elevated: process.elevated,
        executable_path: sanitize_path(&process.executable_path, roots)?,
        executable_size: process.executable_size,
        executable_sha256: process.executable_sha256.clone(),
    })
}

fn sanitize_parent_second_instance(
    second: &DesktopSecondInstanceObservation,
    roots: &[SanitizationRoot],
) -> Result<SanitizedDesktopSecondInstanceObservation, String> {
    Ok(SanitizedDesktopSecondInstanceObservation {
        attempted_process: sanitize_parent_desktop_process(&second.attempted_process, roots)?,
        terminal_exit_code: second.terminal_exit_code,
        process_tree_settled: second.process_tree_settled,
        focused_primary_process_id: second.focused_primary_process_id,
        focused_primary_started_at_100ns: second.focused_primary_started_at_100ns,
        service_instance_id_before: second.service_instance_id_before.clone(),
        service_instance_id_after: second.service_instance_id_after.clone(),
    })
}

fn sanitize_parent_collector_runtime(
    runtime: &DesktopCollectorRuntimeObservation,
    roots: &[SanitizationRoot],
) -> Result<SanitizedDesktopCollectorRuntimeObservation, String> {
    Ok(SanitizedDesktopCollectorRuntimeObservation {
        installed_service: runtime
            .installed_service
            .as_ref()
            .map(|file| sanitize_parent_desktop_file(file, roots))
            .transpose()?,
        service_process: runtime
            .service_process
            .as_ref()
            .map(|process| sanitize_parent_service_process(process, roots))
            .transpose()?,
        pipe_server_process_id: runtime.pipe_server_process_id,
    })
}

fn sanitize_parent_desktop_file(
    file: &DesktopFileObservation,
    roots: &[SanitizationRoot],
) -> Result<SanitizedDesktopFileObservation, String> {
    Ok(SanitizedDesktopFileObservation {
        executable_path: sanitize_path(&file.executable_path, roots)?,
        executable_size: file.executable_size,
        executable_sha256: file.executable_sha256.clone(),
    })
}

fn sanitize_parent_service_process(
    process: &DesktopServiceProcessObservation,
    roots: &[SanitizationRoot],
) -> Result<SanitizedDesktopServiceProcessObservation, String> {
    Ok(SanitizedDesktopServiceProcessObservation {
        process_id: process.process_id,
        started_at_100ns: process.started_at_100ns,
        local_system: process.local_system,
        executable_path: sanitize_path(&process.executable_path, roots)?,
        executable_size: process.executable_size,
        executable_sha256: process.executable_sha256.clone(),
    })
}

fn validate_machine_snapshot(machine: &SanitizedMachineSnapshot) -> Result<(), String> {
    validate_observation(&machine.service, validate_service_snapshot)?;
    for file in [
        &machine.monitor,
        &machine.service_binary,
        &machine.uninstaller,
        &machine.legacy_cli,
    ] {
        validate_observation(file, validate_file_snapshot)?;
    }
    for directory in [
        &machine.install_root,
        &machine.product_data_root,
        &machine.service_data_root,
        &machine.current_user_data_root,
    ] {
        validate_observation(directory, validate_directory_snapshot)?;
    }
    validate_observation(&machine.uninstall_registry, |registry| {
        if !matches!(registry.view.as_str(), "32" | "64")
            || registry.key.root != LogicalRoot::Hklm
            || registry.install_location.root != LogicalRoot::Install
        {
            return Err("lifecycle_sanitized_registry_invalid".to_string());
        }
        validate_logical_path(&registry.key)?;
        validate_logical_path(&registry.install_location)
    })?;
    validate_observation(&machine.product_processes, |processes| {
        if processes.len() > 64 {
            return Err("lifecycle_sanitized_process_set_invalid".to_string());
        }
        let mut process_ids = BTreeSet::new();
        for process in processes {
            if process.process_id == 0
                || !process_ids.insert(process.process_id)
                || !valid_leaf_name(&process.executable_name)
            {
                return Err("lifecycle_sanitized_process_invalid".to_string());
            }
            if let Some(path) = &process.executable_path {
                validate_nonroot_logical_path(path)?;
            }
        }
        Ok(())
    })?;
    validate_observation(&machine.installed_boundaries, validate_installed_boundaries)?;
    if let (Some(service), Some(boundaries)) = (
        machine.service.as_present(),
        machine.installed_boundaries.as_present(),
    ) {
        if service.service_dacl_sha256 != boundaries.service_dacl_sha256 {
            return Err("lifecycle_sanitized_service_dacl_binding_invalid".to_string());
        }
    }
    if let Some(path) = &machine.service_registry_key {
        validate_registry_path(path, LogicalRoot::Hklm)?;
    }
    if machine
        .named_pipe
        .as_ref()
        .is_some_and(|pipe| pipe.server_process_id == 0)
    {
        return Err("lifecycle_sanitized_named_pipe_invalid".to_string());
    }
    if let Some(etw) = &machine.etw_session {
        validate_etw_observation(etw)?;
        let service = machine
            .service
            .as_present()
            .ok_or_else(|| "lifecycle_sanitized_etw_service_missing".to_string())?;
        if service.process_id == 0
            || service.process_started_at_100ns.is_none()
            || etw.lease.controller.process_id != service.process_id
            || etw.lease.controller.process_started_at
                != service.process_started_at_100ns.unwrap_or_default()
            || etw.lease.service_generation != sha256_digest16(&service.image_sha256)?
        {
            return Err("lifecycle_sanitized_etw_service_identity_invalid".to_string());
        }
    }
    for path in [
        machine.etw_lease_file.as_ref(),
        machine.etw_owner_lock.as_ref(),
        machine.service_lifecycle_lock.as_ref(),
    ]
    .into_iter()
    .flatten()
    {
        if path.root != LogicalRoot::ServiceData {
            return Err("lifecycle_sanitized_service_data_path_invalid".to_string());
        }
        validate_nonroot_logical_path(path)?;
    }
    if let Some(file) = &machine.upgrade_transaction_journal {
        validate_path_file(file, Some(LogicalRoot::Install))?;
    }
    for files in [
        &machine.staged_service_images,
        &machine.rollback_service_images,
        &machine.atomic_temporary_files,
    ] {
        validate_path_file_set(files, Some(LogicalRoot::Install))?;
    }
    if let Some(file) = &machine.failure_marker {
        validate_path_file(file, Some(LogicalRoot::Install))?;
    }
    if let Some(path) = &machine.machine_product_key {
        validate_registry_path(path, LogicalRoot::Hklm)?;
    }
    if let Some(autostart) = &machine.hkcu_autostart {
        validate_registry_path(&autostart.key, LogicalRoot::Hkcu)?;
        if !valid_leaf_name(&autostart.value_name)
            || autostart.target.root != LogicalRoot::Install
            || autostart.target.relative_leaf.is_empty()
        {
            return Err("lifecycle_sanitized_autostart_invalid".to_string());
        }
        validate_logical_path(&autostart.target)?;
    }
    if let Some(shortcut) = &machine.public_desktop_shortcut {
        validate_shortcut(shortcut, LogicalRoot::PublicDesktop)?;
    }
    if let Some(shortcut) = &machine.common_start_menu_shortcut {
        validate_shortcut(shortcut, LogicalRoot::CommonStartMenu)?;
    }
    validate_path_file_set(
        &machine.known_retired_helper_artifacts,
        Some(LogicalRoot::CurrentUserData),
    )?;
    if let Some(sentinel) = &machine.unknown_helper_sentinel {
        validate_path_file(sentinel, Some(LogicalRoot::CurrentUserData))?;
    }
    Ok(())
}

fn validate_observation<T>(
    observation: &Observation<T>,
    validate: impl FnOnce(&T) -> Result<(), String>,
) -> Result<(), String> {
    match observation {
        Observation::Present(value) => validate(value),
        Observation::Absent => Ok(()),
        Observation::Unknown(_) => Err("lifecycle_sanitized_observation_unknown".to_string()),
    }
}

trait ObservationExt<T> {
    fn as_present(&self) -> Option<&T>;
}

impl<T> ObservationExt<T> for Observation<T> {
    fn as_present(&self) -> Option<&T> {
        match self {
            Observation::Present(value) => Some(value),
            Observation::Absent | Observation::Unknown(_) => None,
        }
    }
}

fn validate_file_snapshot(file: &SanitizedFileSnapshot) -> Result<(), String> {
    if file.size == 0 || file.file_index == 0 {
        return Err("lifecycle_sanitized_file_invalid".to_string());
    }
    validate_sha256(&file.sha256, "sanitized_file")
}

fn validate_service_snapshot(service: &SanitizedServiceSnapshot) -> Result<(), String> {
    if service.image_path.root != LogicalRoot::Install
        || service.image_path.relative_leaf != "batcave-collector-service.exe"
        || !service.local_system
        || !service.own_process
        || !service.automatic_start
        || service.recovery_restart_action_count == 0
        || service.owner_marker != "dev.batcave.monitor/service-v1"
        || (service.process_id == 0) != service.process_started_at_100ns.is_none()
        || service
            .process_started_at_100ns
            .is_some_and(|started_at| started_at == 0)
    {
        return Err("lifecycle_sanitized_service_contract_invalid".to_string());
    }
    validate_nonroot_logical_path(&service.image_path)?;
    validate_sha256(&service.image_sha256, "sanitized_service_image")?;
    validate_sha256(&service.service_dacl_sha256, "sanitized_service_dacl")
}

fn validate_directory_snapshot(directory: &SanitizedDirectorySnapshot) -> Result<(), String> {
    if directory.file_index == 0 {
        return Err("lifecycle_sanitized_directory_invalid".to_string());
    }
    validate_logical_path(&directory.final_path)
}

fn validate_installed_boundaries(boundaries: &SanitizedBoundarySnapshot) -> Result<(), String> {
    validate_sha256(&boundaries.service_dacl_sha256, "sanitized_service_dacl")?;
    validate_sha256(
        &boundaries.service_data_root_dacl_sha256,
        "sanitized_service_data_root_dacl",
    )?;
    let expected_service = [
        ace(SanitizedPrincipal::LocalSystem, 0x000f_01ff, false),
        ace(SanitizedPrincipal::Administrators, 0x000f_01ff, false),
        ace(SanitizedPrincipal::InteractiveUsers, 0x0000_0004, false),
    ]
    .into_iter()
    .collect::<BTreeSet<_>>();
    let expected_root = [
        ace(SanitizedPrincipal::LocalSystem, 0x001f_01ff, true),
        ace(SanitizedPrincipal::Administrators, 0x001f_01ff, true),
        ace(SanitizedPrincipal::CollectorService, 0x0013_01bf, true),
    ]
    .into_iter()
    .collect::<BTreeSet<_>>();
    let service = boundaries
        .service_aces
        .iter()
        .cloned()
        .collect::<BTreeSet<_>>();
    let root = boundaries
        .service_data_root_aces
        .iter()
        .cloned()
        .collect::<BTreeSet<_>>();
    if boundaries.service_aces.len() != expected_service.len()
        || service != expected_service
        || boundaries.service_data_root_owner != SanitizedPrincipal::LocalSystem
        || !boundaries.service_data_root_dacl_protected
        || boundaries.service_data_root_reparse
        || boundaries.service_data_root_aces.len() != expected_root.len()
        || root != expected_root
    {
        return Err("lifecycle_sanitized_installed_boundaries_invalid".to_string());
    }
    Ok(())
}

fn ace(
    principal: SanitizedPrincipal,
    mask: u32,
    inherited_for_children: bool,
) -> SanitizedAceSnapshot {
    SanitizedAceSnapshot {
        principal,
        allow: true,
        inherit_only: false,
        object_inherit: inherited_for_children,
        container_inherit: inherited_for_children,
        mask,
    }
}

fn validate_registry_path(path: &LogicalPath, root: LogicalRoot) -> Result<(), String> {
    if path.root != root || path.relative_leaf.is_empty() {
        return Err("lifecycle_sanitized_registry_path_invalid".to_string());
    }
    validate_logical_path(path)
}

fn validate_etw_observation(etw: &SanitizedEtwObservation) -> Result<(), String> {
    let lease = &etw.lease;
    let session = &etw.observed_session;
    if lease.schema_version != ETW_LEASE_SCHEMA_VERSION
        || lease.phase != EtwLeasePhase::Active
        || lease.install_id == [0; 16]
        || lease.service_generation == [0; 16]
        || lease.service_instance_id == [0; 16]
        || lease.boot_identity == [0; 16]
        || lease.controller.process_id == 0
        || lease.controller.process_started_at == 0
        || lease.session != *session
        || session != &NetworkAttributionMonitor::session_identity()
        || !etw.owner_lock_held
        || !etw.process_lock_held
        || etw.events_lost != 0
        || etw.buffers_lost != 0
    {
        return Err("lifecycle_sanitized_etw_invalid".to_string());
    }
    Ok(())
}

fn sha256_digest16(value: &str) -> Result<[u8; 16], String> {
    validate_sha256(value, "sanitized_service_generation")?;
    let mut digest = [0_u8; 16];
    for (index, byte) in digest.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&value[index * 2..index * 2 + 2], 16)
            .map_err(|_| "lifecycle_sanitized_service_generation_invalid".to_string())?;
    }
    Ok(digest)
}

fn digest16(bytes: &[u8]) -> [u8; 16] {
    let digest = Sha256::digest(bytes);
    let mut value = [0_u8; 16];
    value.copy_from_slice(&digest[..16]);
    value
}

fn validate_path_file(
    file: &SanitizedPathFileSnapshot,
    expected_root: Option<LogicalRoot>,
) -> Result<(), String> {
    if expected_root.is_some_and(|root| file.path.root != root)
        || file.path.relative_leaf.is_empty()
    {
        return Err("lifecycle_sanitized_path_file_root_invalid".to_string());
    }
    validate_logical_path(&file.path)?;
    validate_file_snapshot(&file.file)
}

fn validate_path_file_set(
    files: &[SanitizedPathFileSnapshot],
    expected_root: Option<LogicalRoot>,
) -> Result<(), String> {
    if files.len() > 64 {
        return Err("lifecycle_sanitized_path_file_set_invalid".to_string());
    }
    let mut paths = BTreeSet::new();
    for file in files {
        validate_path_file(file, expected_root)?;
        if !paths.insert((
            &file.path.root,
            file.path.relative_leaf.to_ascii_lowercase(),
        )) {
            return Err("lifecycle_sanitized_path_file_set_invalid".to_string());
        }
    }
    Ok(())
}

fn validate_shortcut(
    shortcut: &SanitizedShortcutSnapshot,
    expected_root: LogicalRoot,
) -> Result<(), String> {
    if shortcut.path.root != expected_root || shortcut.target.root != LogicalRoot::Install {
        return Err("lifecycle_sanitized_shortcut_invalid".to_string());
    }
    validate_nonroot_logical_path(&shortcut.path)?;
    validate_nonroot_logical_path(&shortcut.target)?;
    validate_sha256(&shortcut.sha256, "sanitized_shortcut")
}

fn validate_current_user_retention(
    retention: &SanitizedCurrentUserRetention,
) -> Result<(), String> {
    for (object, expected_leaf) in [
        (&retention.settings, RETAINED_SETTINGS_LEAF),
        (&retention.cache, RETAINED_CACHE_LEAF),
        (&retention.diagnostics, RETAINED_DIAGNOSTICS_LEAF),
    ] {
        if object.path.root != LogicalRoot::CurrentUserData
            || object.path.relative_leaf != expected_leaf
        {
            return Err("lifecycle_sanitized_user_retention_path_invalid".to_string());
        }
        validate_nonroot_logical_path(&object.path)?;
        validate_digest_observation(&object.before_uninstall)?;
        validate_digest_observation(&object.after_uninstall)?;
    }
    Ok(())
}

fn validate_current_user_retention_sources(
    retention: &SanitizedCurrentUserRetention,
    entries: &[SanitizedPrivateEvidence],
) -> Result<(), String> {
    let by_name = entries
        .iter()
        .map(|entry| (entry.receipt.name.as_str(), &entry.receipt))
        .collect::<BTreeMap<_, _>>();
    if by_name
        .get("final-incompatible-service-restored-state.private.json")
        .copied()
        != Some(&retention.before_uninstall_source)
        || by_name.get("final-uninstall-state.private.json").copied()
            != Some(&retention.after_uninstall_source)
    {
        return Err("lifecycle_sanitized_user_retention_source_invalid".to_string());
    }
    Ok(())
}

fn validate_digest_observation(
    observation: &Observation<SanitizedDigestSnapshot>,
) -> Result<(), String> {
    validate_observation(observation, |digest| {
        if digest.size == 0 {
            return Err("lifecycle_sanitized_retained_digest_invalid".to_string());
        }
        validate_sha256(&digest.sha256, "sanitized_retained_digest")
    })
}

fn validate_logical_path(path: &LogicalPath) -> Result<(), String> {
    validate_relative_leaf(&path.relative_leaf.replace('/', r"\"))
}

fn validate_nonroot_logical_path(path: &LogicalPath) -> Result<(), String> {
    if path.relative_leaf.is_empty() {
        return Err("lifecycle_sanitized_path_leaf_missing".to_string());
    }
    validate_logical_path(path)
}

fn validate_stage_event(
    receipt_name: &str,
    event: Option<&SanitizedStageEvent>,
    machine: &SanitizedMachineSnapshot,
    plan: &ProofPlan,
) -> Result<(), String> {
    match (receipt_name, event) {
        (
            "baseline-crashed-state.private.json" | "final-crashed-state.private.json",
            Some(SanitizedStageEvent::ServiceCrash(event)),
        ) => {
            let expected_service_sha256 = if receipt_name.starts_with("baseline-") {
                &plan.baseline.service_sha256
            } else {
                &plan.final_candidate.service_sha256
            };
            let service = machine
                .service
                .as_present()
                .ok_or_else(|| "lifecycle_sanitized_crashed_service_missing".to_string())?;
            if event.process_id == 0
                || event.process_started_at_100ns == 0
                || event.image_path.root != LogicalRoot::Install
                || event.image_path.relative_leaf != "batcave-collector-service.exe"
                || event.image_sha256.as_str() != expected_service_sha256
                || event.process_exit_code != 1
                || (event.win32_exit_code == 0 && event.service_specific_exit_code == 0)
                || event.win32_exit_code != service.win32_exit_code
                || event.service_specific_exit_code != service.service_specific_exit_code
                || service.state == 4
                || service.process_id != 0
            {
                return Err("lifecycle_sanitized_service_crash_invalid".to_string());
            }
            validate_logical_path(&event.image_path)?;
            validate_sha256(&event.image_sha256, "sanitized_crashed_service")
        }
        (
            "baseline-rollback-recovery-state.private.json",
            Some(SanitizedStageEvent::UpgradeRollback(event)),
        ) => {
            let restored_service = machine
                .service
                .as_present()
                .ok_or_else(|| "lifecycle_sanitized_rollback_service_missing".to_string())?;
            if event.candidate_sha256 != plan.rollback_failing_service_fixture.sha256
                || event.candidate_failure_code != "collector_service_proof_candidate_start_failed"
                || !valid_bounded_reason(&event.candidate_failure_detail)
                || !event
                    .candidate_failure_detail
                    .starts_with("collector_service_")
                || event
                    .candidate_failure_detail
                    .contains("collector_service_upgrade_rollback_failed")
                || event.execution_marker_sha256
                    != sha256_hex(b"batcave_windows_lifecycle_rollback_fixture_v1\n")
                || event.restored_sha256 != plan.baseline.service_sha256
                || event.restored_process_id == 0
                || event.restored_process_id != restored_service.process_id
                || restored_service.state != 4
            {
                return Err("lifecycle_sanitized_upgrade_rollback_invalid".to_string());
            }
            for (hash, label) in [
                (&event.candidate_sha256, "sanitized_rollback_candidate"),
                (&event.execution_marker_sha256, "sanitized_rollback_marker"),
                (&event.restored_sha256, "sanitized_rollback_restored"),
            ] {
                validate_sha256(hash, label)?;
            }
            Ok(())
        }
        (
            "baseline-crashed-state.private.json"
            | "final-crashed-state.private.json"
            | "baseline-rollback-recovery-state.private.json",
            _,
        ) => Err("lifecycle_sanitized_stage_event_missing".to_string()),
        (_, None) => Ok(()),
        (_, Some(_)) => Err("lifecycle_sanitized_stage_event_unexpected".to_string()),
    }
}

fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn assertion_for_leaf(value: &str) -> SanitizedEvidenceAssertion {
    match value {
        "initial-state.private.json" => SanitizedEvidenceAssertion::InitialLegacyStopped,
        "final-repair-state.private.json" => SanitizedEvidenceAssertion::FinalInstalledRunning,
        "final-primary-desktop.private.json"
        | "baseline-primary-desktop.private.json"
        | "baseline-second-instance-desktop.private.json"
        | "final-missing-service-desktop.private.json"
        | "final-stopped-service-desktop.private.json"
        | "final-incompatible-service-desktop.private.json" => {
            SanitizedEvidenceAssertion::DesktopPhase
        }
        "initial-uninstall-state.private.json" => SanitizedEvidenceAssertion::ProductAbsent,
        "baseline-install-state.private.json" | "baseline-restart-state.private.json" => {
            SanitizedEvidenceAssertion::BaselineInstalledRunning
        }
        "baseline-restart-stopped-state.private.json" => {
            SanitizedEvidenceAssertion::BaselineStopped
        }
        "baseline-crashed-state.private.json" => SanitizedEvidenceAssertion::BaselineCrashed,
        "baseline-crash-recovery-state.private.json" => {
            SanitizedEvidenceAssertion::BaselineRecovered
        }
        "baseline-rollback-recovery-state.private.json" => {
            SanitizedEvidenceAssertion::BaselineRollbackRecovered
        }
        "legacy-residue-seeded-state.private.json" => {
            SanitizedEvidenceAssertion::LegacyResidueSeeded
        }
        "final-upgrade-state.private.json" => SanitizedEvidenceAssertion::FinalUpgradedRunning,
        "final-restart-stopped-state.private.json" | "final-stopped-service-state.private.json" => {
            SanitizedEvidenceAssertion::FinalStopped
        }
        "final-restart-state.private.json"
        | "final-crash-recovery-state.private.json"
        | "final-missing-service-restored-state.private.json"
        | "final-stopped-service-restored-state.private.json"
        | "final-incompatible-service-restored-state.private.json" => {
            SanitizedEvidenceAssertion::FinalRecovered
        }
        "final-crashed-state.private.json" => SanitizedEvidenceAssertion::FinalCrashed,
        "final-missing-service-state.private.json" => {
            SanitizedEvidenceAssertion::FinalMissingService
        }
        "final-incompatible-service-state.private.json" => {
            SanitizedEvidenceAssertion::FinalIncompatibleRunning
        }
        "final-uninstall-state.private.json" => {
            SanitizedEvidenceAssertion::FinalUninstalledPreservingCurrentUserData
        }
        _ => unreachable!("fixed success evidence leaf"),
    }
}

fn validate_stage_machine_assertion(
    assertion: SanitizedEvidenceAssertion,
    machine: &SanitizedMachineSnapshot,
    plan: &ProofPlan,
) -> Result<(), String> {
    match assertion {
        SanitizedEvidenceAssertion::InitialLegacyStopped => {
            validate_installed_machine(
                machine,
                allowlisted_artifacts(plan),
                ServiceExpectation::Stopped {
                    win32_exit_code: plan.allowlisted_start.win32_exit_code,
                    service_specific_exit_code: plan.allowlisted_start.service_specific_exit_code,
                },
            )?;
            require_legacy_cli(machine, Some(&plan.allowlisted_start.legacy_cli_sha256))
        }
        SanitizedEvidenceAssertion::FinalInstalledRunning
        | SanitizedEvidenceAssertion::FinalRecovered => {
            validate_installed_machine(
                machine,
                final_artifacts(plan),
                ServiceExpectation::Running,
            )?;
            require_legacy_cli(machine, None)
        }
        SanitizedEvidenceAssertion::ProductAbsent => validate_product_absence(machine, false),
        SanitizedEvidenceAssertion::BaselineInstalledRunning
        | SanitizedEvidenceAssertion::BaselineRecovered
        | SanitizedEvidenceAssertion::BaselineRollbackRecovered => {
            validate_installed_machine(
                machine,
                baseline_artifacts(plan),
                ServiceExpectation::Running,
            )?;
            require_legacy_cli(machine, None)
        }
        SanitizedEvidenceAssertion::BaselineStopped => {
            validate_installed_machine(
                machine,
                baseline_artifacts(plan),
                ServiceExpectation::Stopped {
                    win32_exit_code: 0,
                    service_specific_exit_code: 0,
                },
            )?;
            require_legacy_cli(machine, None)
        }
        SanitizedEvidenceAssertion::BaselineCrashed => {
            validate_installed_machine(
                machine,
                baseline_artifacts(plan),
                ServiceExpectation::Crashed,
            )?;
            require_legacy_cli(machine, None)
        }
        SanitizedEvidenceAssertion::LegacyResidueSeeded => {
            validate_installed_machine(
                machine,
                baseline_artifacts(plan),
                ServiceExpectation::Running,
            )?;
            require_legacy_cli(machine, Some(&plan.allowlisted_start.legacy_cli_sha256))?;
            validate_known_helper_seed(machine)?;
            validate_unknown_helper_sentinel(machine)
        }
        SanitizedEvidenceAssertion::FinalUpgradedRunning => {
            validate_installed_machine(
                machine,
                final_artifacts(plan),
                ServiceExpectation::Running,
            )?;
            if !matches!(machine.legacy_cli, Observation::Absent)
                || !machine.known_retired_helper_artifacts.is_empty()
            {
                return Err("lifecycle_sanitized_upgrade_cleanup_invalid".to_string());
            }
            validate_unknown_helper_sentinel(machine)
        }
        SanitizedEvidenceAssertion::FinalStopped => {
            validate_installed_machine(
                machine,
                final_artifacts(plan),
                ServiceExpectation::Stopped {
                    win32_exit_code: 0,
                    service_specific_exit_code: 0,
                },
            )?;
            require_legacy_cli(machine, None)
        }
        SanitizedEvidenceAssertion::FinalCrashed => {
            validate_installed_machine(
                machine,
                final_artifacts(plan),
                ServiceExpectation::Crashed,
            )?;
            require_legacy_cli(machine, None)
        }
        SanitizedEvidenceAssertion::FinalMissingService => {
            validate_desktop_only_machine(machine, final_artifacts(plan))?;
            require_legacy_cli(machine, None)
        }
        SanitizedEvidenceAssertion::FinalIncompatibleRunning => {
            validate_installed_machine(
                machine,
                incompatible_artifacts(plan),
                ServiceExpectation::Running,
            )?;
            require_legacy_cli(machine, None)
        }
        SanitizedEvidenceAssertion::FinalUninstalledPreservingCurrentUserData => {
            validate_product_absence(machine, true)
        }
        SanitizedEvidenceAssertion::DesktopPhase => Ok(()),
    }
}

#[derive(Clone, Copy)]
enum ServiceExpectation {
    Running,
    Stopped {
        win32_exit_code: u32,
        service_specific_exit_code: u32,
    },
    Crashed,
}

#[derive(Clone, Copy)]
struct InstalledArtifactExpectation<'a> {
    monitor_sha256: &'a str,
    service_sha256: &'a str,
    service_size: Option<u64>,
    uninstaller_sha256: &'a str,
    uninstaller_size: Option<u64>,
}

fn allowlisted_artifacts(plan: &ProofPlan) -> InstalledArtifactExpectation<'_> {
    InstalledArtifactExpectation {
        monitor_sha256: &plan.allowlisted_start.monitor_sha256,
        service_sha256: &plan.allowlisted_start.service_sha256,
        service_size: None,
        uninstaller_sha256: &plan.allowlisted_start.uninstaller_sha256,
        uninstaller_size: None,
    }
}

fn baseline_artifacts(plan: &ProofPlan) -> InstalledArtifactExpectation<'_> {
    InstalledArtifactExpectation {
        monitor_sha256: &plan.baseline.monitor_sha256,
        service_sha256: &plan.baseline.service_sha256,
        service_size: None,
        uninstaller_sha256: &plan.baseline.uninstaller_sha256,
        uninstaller_size: Some(plan.baseline.uninstaller_size),
    }
}

fn final_artifacts(plan: &ProofPlan) -> InstalledArtifactExpectation<'_> {
    InstalledArtifactExpectation {
        monitor_sha256: &plan.final_candidate.monitor_sha256,
        service_sha256: &plan.final_candidate.service_sha256,
        service_size: None,
        uninstaller_sha256: &plan.final_candidate.uninstaller_sha256,
        uninstaller_size: Some(plan.final_candidate.uninstaller_size),
    }
}

fn incompatible_artifacts(plan: &ProofPlan) -> InstalledArtifactExpectation<'_> {
    InstalledArtifactExpectation {
        service_sha256: &plan.incompatible_service_fixture.sha256,
        service_size: Some(plan.incompatible_service_fixture.size),
        ..final_artifacts(plan)
    }
}

fn require_legacy_cli(
    machine: &SanitizedMachineSnapshot,
    expected_sha256: Option<&str>,
) -> Result<(), String> {
    match (expected_sha256, &machine.legacy_cli) {
        (Some(expected), Observation::Present(file)) if file.sha256.as_str() == expected => Ok(()),
        (None, Observation::Absent) => Ok(()),
        _ => Err("lifecycle_sanitized_legacy_cli_identity_invalid".to_string()),
    }
}

fn validate_known_helper_seed(machine: &SanitizedMachineSnapshot) -> Result<(), String> {
    if machine.known_retired_helper_artifacts.len() != KNOWN_RETIRED_HELPER_LEAVES.len() {
        return Err("lifecycle_sanitized_legacy_helper_manifest_invalid".to_string());
    }
    let files = machine
        .known_retired_helper_artifacts
        .iter()
        .map(|file| (file.path.relative_leaf.as_str(), file))
        .collect::<BTreeMap<_, _>>();
    if files.len() != KNOWN_RETIRED_HELPER_LEAVES.len() {
        return Err("lifecycle_sanitized_legacy_helper_manifest_invalid".to_string());
    }
    for leaf in KNOWN_RETIRED_HELPER_LEAVES {
        let file = files
            .get(leaf)
            .ok_or_else(|| "lifecycle_sanitized_legacy_helper_manifest_invalid".to_string())?;
        let bytes = known_helper_fixture_bytes(leaf);
        if file.path.root != LogicalRoot::CurrentUserData
            || file.file.size != bytes.len() as u64
            || file.file.sha256 != sha256_hex(&bytes)
        {
            return Err("lifecycle_sanitized_legacy_helper_identity_invalid".to_string());
        }
    }
    Ok(())
}

fn validate_unknown_helper_sentinel(machine: &SanitizedMachineSnapshot) -> Result<(), String> {
    let sentinel = machine
        .unknown_helper_sentinel
        .as_ref()
        .ok_or_else(|| "lifecycle_sanitized_unknown_helper_sentinel_missing".to_string())?;
    let bytes = b"batcave_windows_lifecycle_unknown_helper_sentinel_v1\n";
    if sentinel.path.root != LogicalRoot::CurrentUserData
        || sentinel.path.relative_leaf != UNKNOWN_HELPER_SENTINEL_LEAF
        || sentinel.file.size != bytes.len() as u64
        || sentinel.file.sha256 != sha256_hex(bytes)
    {
        return Err("lifecycle_sanitized_unknown_helper_sentinel_invalid".to_string());
    }
    Ok(())
}

fn known_helper_fixture_bytes(leaf: &str) -> Vec<u8> {
    format!("batcave_windows_lifecycle_known_helper_fixture_v1:{leaf}\n").into_bytes()
}

fn validate_installed_machine(
    machine: &SanitizedMachineSnapshot,
    artifacts: InstalledArtifactExpectation<'_>,
    service_expectation: ServiceExpectation,
) -> Result<(), String> {
    if !directory_role_valid(&machine.install_root, LogicalRoot::Install, true)
        || !directory_role_valid(&machine.product_data_root, LogicalRoot::ProductData, true)
        || !directory_role_valid(&machine.service_data_root, LogicalRoot::ServiceData, true)
        || !directory_role_valid(
            &machine.current_user_data_root,
            LogicalRoot::CurrentUserData,
            true,
        )
        || machine
            .monitor
            .as_present()
            .is_none_or(|file| file.sha256.as_str() != artifacts.monitor_sha256)
        || machine.service_binary.as_present().is_none_or(|file| {
            file.sha256.as_str() != artifacts.service_sha256
                || artifacts.service_size.is_some_and(|size| file.size != size)
        })
        || machine.uninstaller.as_present().is_none_or(|file| {
            file.sha256.as_str() != artifacts.uninstaller_sha256
                || artifacts
                    .uninstaller_size
                    .is_some_and(|size| file.size != size)
        })
        || !matches!(machine.uninstall_registry, Observation::Present(_))
        || machine.service_registry_key.is_none()
        || machine.machine_product_key.is_none()
        || machine.hkcu_autostart.is_none()
        || machine.public_desktop_shortcut.is_none()
        || machine.common_start_menu_shortcut.is_none()
        || !matches!(machine.installed_boundaries, Observation::Present(_))
        || !no_surviving_product_processes(machine)
        || !installed_registration_valid(machine, true)
        || !transaction_residue_absent(machine)
    {
        return Err("lifecycle_sanitized_installed_machine_invalid".to_string());
    }
    if machine
        .service
        .as_present()
        .is_none_or(|service| service.image_sha256.as_str() != artifacts.service_sha256)
    {
        return Err("lifecycle_sanitized_service_image_mismatch".to_string());
    }
    match (service_expectation, &machine.service) {
        (ServiceExpectation::Running, Observation::Present(service))
            if service.state == 4
                && service.process_id != 0
                && machine
                    .named_pipe
                    .as_ref()
                    .is_some_and(|pipe| pipe.server_process_id == service.process_id)
                && machine.etw_session.as_ref().is_some_and(|etw| {
                    etw.lease.controller.process_id == service.process_id
                        && service.process_started_at_100ns
                            == Some(etw.lease.controller.process_started_at)
                })
                && machine.etw_lease_file.as_ref().is_some_and(|path| {
                    logical_leaf_eq(path, LogicalRoot::ServiceData, "etw-lease.v1.json")
                })
                && machine.etw_owner_lock.as_ref().is_some_and(|path| {
                    logical_leaf_eq(path, LogicalRoot::ServiceData, "etw-owner.v1.lock")
                })
                && machine.service_lifecycle_lock.as_ref().is_some_and(|path| {
                    logical_leaf_eq(path, LogicalRoot::ServiceData, "process-owner.v1.lock")
                }) =>
        {
            Ok(())
        }
        (
            ServiceExpectation::Stopped {
                win32_exit_code,
                service_specific_exit_code,
            },
            Observation::Present(service),
        ) if service.state == 1
            && service.process_id == 0
            && service.win32_exit_code == win32_exit_code
            && service.service_specific_exit_code == service_specific_exit_code
            && runtime_residue_absent(machine) =>
        {
            Ok(())
        }
        (ServiceExpectation::Crashed, Observation::Present(service))
            if service.state != 4 && service.process_id == 0 && runtime_residue_absent(machine) =>
        {
            Ok(())
        }
        _ => Err("lifecycle_sanitized_service_assertion_invalid".to_string()),
    }
}

fn validate_desktop_only_machine(
    machine: &SanitizedMachineSnapshot,
    artifacts: InstalledArtifactExpectation<'_>,
) -> Result<(), String> {
    if !matches!(machine.service, Observation::Absent)
        || machine.service_registry_key.is_some()
        || !runtime_residue_absent(machine)
        || !directory_role_valid(&machine.install_root, LogicalRoot::Install, true)
        || !directory_role_valid(&machine.product_data_root, LogicalRoot::ProductData, true)
        || !directory_role_valid(&machine.service_data_root, LogicalRoot::ServiceData, false)
        || !directory_role_valid(
            &machine.current_user_data_root,
            LogicalRoot::CurrentUserData,
            true,
        )
        || machine
            .monitor
            .as_present()
            .is_none_or(|file| file.sha256.as_str() != artifacts.monitor_sha256)
        || !matches!(machine.service_binary, Observation::Absent)
        || !matches!(machine.legacy_cli, Observation::Absent)
        || machine.uninstaller.as_present().is_none_or(|file| {
            file.sha256.as_str() != artifacts.uninstaller_sha256
                || artifacts
                    .uninstaller_size
                    .is_some_and(|size| file.size != size)
        })
        || !matches!(machine.uninstall_registry, Observation::Present(_))
        || machine.machine_product_key.is_none()
        || machine.hkcu_autostart.is_none()
        || machine.public_desktop_shortcut.is_none()
        || machine.common_start_menu_shortcut.is_none()
        || !matches!(machine.installed_boundaries, Observation::Absent)
        || !no_surviving_product_processes(machine)
        || !installed_registration_valid(machine, false)
        || !transaction_residue_absent(machine)
    {
        return Err("lifecycle_sanitized_desktop_only_machine_invalid".to_string());
    }
    Ok(())
}

fn directory_role_valid(
    observation: &Observation<SanitizedDirectorySnapshot>,
    expected_root: LogicalRoot,
    should_be_present: bool,
) -> bool {
    match (should_be_present, observation) {
        (true, Observation::Present(directory)) => {
            logical_leaf_eq(&directory.final_path, expected_root, "")
        }
        (false, Observation::Absent) => true,
        _ => false,
    }
}

fn no_surviving_product_processes(machine: &SanitizedMachineSnapshot) -> bool {
    match &machine.product_processes {
        Observation::Absent => true,
        Observation::Present(processes) => processes.is_empty(),
        Observation::Unknown(_) => false,
    }
}

fn runtime_residue_absent(machine: &SanitizedMachineSnapshot) -> bool {
    machine.named_pipe.is_none()
        && machine.etw_session.is_none()
        && machine.etw_lease_file.is_none()
        && machine.etw_owner_lock.is_none()
        && machine.service_lifecycle_lock.is_none()
}

fn transaction_residue_absent(machine: &SanitizedMachineSnapshot) -> bool {
    machine.upgrade_transaction_journal.is_none()
        && machine.staged_service_images.is_empty()
        && machine.rollback_service_images.is_empty()
        && machine.atomic_temporary_files.is_empty()
        && machine.failure_marker.is_none()
}

fn installed_registration_valid(
    machine: &SanitizedMachineSnapshot,
    require_service_key: bool,
) -> bool {
    let uninstall_registry_valid =
        machine
            .uninstall_registry
            .as_present()
            .is_some_and(|registry| {
                logical_leaf_eq(
                    &registry.key,
                    LogicalRoot::Hklm,
                    "software/microsoft/windows/currentversion/uninstall/batcave-monitor",
                ) && registry.install_location.root == LogicalRoot::Install
                    && registry.install_location.relative_leaf.is_empty()
            });
    let service_key_valid = machine.service_registry_key.as_ref().is_some_and(|path| {
        logical_leaf_eq(
            path,
            LogicalRoot::Hklm,
            "system/currentcontrolset/services/batcavecollector",
        )
    });
    let product_key_valid = machine
        .machine_product_key
        .as_ref()
        .is_some_and(|path| logical_leaf_eq(path, LogicalRoot::Hklm, "software/batcavemonitor"));
    let autostart_valid = machine.hkcu_autostart.as_ref().is_some_and(|autostart| {
        logical_leaf_eq(
            &autostart.key,
            LogicalRoot::Hkcu,
            "software/microsoft/windows/currentversion/run",
        ) && autostart.value_name == "BatCave Monitor"
            && logical_leaf_eq(
                &autostart.target,
                LogicalRoot::Install,
                "batcave-monitor.exe",
            )
    });
    let public_shortcut_valid = machine
        .public_desktop_shortcut
        .as_ref()
        .is_some_and(|shortcut| {
            logical_leaf_eq(
                &shortcut.path,
                LogicalRoot::PublicDesktop,
                "BatCave Monitor.lnk",
            ) && logical_leaf_eq(
                &shortcut.target,
                LogicalRoot::Install,
                "batcave-monitor.exe",
            )
        });
    let start_menu_shortcut_valid =
        machine
            .common_start_menu_shortcut
            .as_ref()
            .is_some_and(|shortcut| {
                logical_leaf_eq(
                    &shortcut.path,
                    LogicalRoot::CommonStartMenu,
                    "BatCave Monitor.lnk",
                ) && logical_leaf_eq(
                    &shortcut.target,
                    LogicalRoot::Install,
                    "batcave-monitor.exe",
                )
            });
    uninstall_registry_valid
        && (!require_service_key || service_key_valid)
        && product_key_valid
        && autostart_valid
        && public_shortcut_valid
        && start_menu_shortcut_valid
}

fn logical_leaf_eq(path: &LogicalPath, root: LogicalRoot, relative_leaf: &str) -> bool {
    path.root == root && path.relative_leaf.eq_ignore_ascii_case(relative_leaf)
}

fn desktop_phase_for_leaf(value: &str) -> Option<DesktopPhase> {
    match value {
        "final-primary-desktop.private.json" => Some(DesktopPhase::FinalPrimary),
        "baseline-primary-desktop.private.json" => Some(DesktopPhase::BaselinePrimary),
        "baseline-second-instance-desktop.private.json" => {
            Some(DesktopPhase::BaselineSecondInstance)
        }
        "final-missing-service-desktop.private.json" => Some(DesktopPhase::FinalMissingService),
        "final-stopped-service-desktop.private.json" => Some(DesktopPhase::FinalStoppedService),
        "final-incompatible-service-desktop.private.json" => {
            Some(DesktopPhase::FinalIncompatibleService)
        }
        _ => None,
    }
}

fn validate_sanitized_desktop_phase(
    observation: &SanitizedDesktopPhaseObservation,
    plan: &ProofPlan,
) -> Result<(), String> {
    if !observation.process_tree_settled {
        return Err("lifecycle_sanitized_desktop_unsettled".to_string());
    }
    validate_sanitized_desktop_process(&observation.desktop)?;
    if observation.desktop.parent_process_id.is_some()
        || observation.desktop.session_id == 0
        || observation.desktop.executable_path.root != LogicalRoot::Install
        || observation.desktop.executable_path.relative_leaf != "batcave-monitor.exe"
        || observation.desktop.executable_sha256 != expected_monitor_sha256(observation.phase, plan)
    {
        return Err("lifecycle_sanitized_desktop_identity_invalid".to_string());
    }

    if observation.process_tree.is_empty()
        || observation.process_tree.len() > 128
        || observation.webview_process_ids.is_empty()
        || observation.webview_process_ids.len() > 32
    {
        return Err("lifecycle_sanitized_desktop_process_tree_invalid".to_string());
    }
    let mut processes = BTreeMap::new();
    processes.insert(observation.desktop.process_id, &observation.desktop);
    for process in &observation.process_tree {
        validate_sanitized_desktop_process(process)?;
        if process.session_id != observation.desktop.session_id
            || processes.insert(process.process_id, process).is_some()
        {
            return Err("lifecycle_sanitized_desktop_process_tree_invalid".to_string());
        }
    }
    for process in &observation.process_tree {
        let Some(parent_process_id) = process.parent_process_id else {
            return Err("lifecycle_sanitized_desktop_process_tree_invalid".to_string());
        };
        if !processes.contains_key(&parent_process_id) {
            return Err("lifecycle_sanitized_desktop_process_tree_invalid".to_string());
        }
        let mut next = process.parent_process_id;
        let mut seen = BTreeSet::new();
        loop {
            let Some(parent_process_id) = next else {
                return Err("lifecycle_sanitized_desktop_process_tree_invalid".to_string());
            };
            if parent_process_id == observation.desktop.process_id {
                break;
            }
            if !seen.insert(parent_process_id) {
                return Err("lifecycle_sanitized_desktop_process_tree_invalid".to_string());
            }
            next = processes
                .get(&parent_process_id)
                .and_then(|parent| parent.parent_process_id);
        }
    }

    let mut webview_ids = BTreeSet::new();
    let mut webview_hashes = BTreeSet::new();
    let mut webview_sizes = BTreeSet::new();
    let mut webview_paths = BTreeSet::new();
    for process_id in &observation.webview_process_ids {
        let Some(webview) = processes.get(process_id) else {
            return Err("lifecycle_sanitized_webview_identity_invalid".to_string());
        };
        if !webview_ids.insert(*process_id) || !valid_webview_logical_path(&webview.executable_path)
        {
            return Err("lifecycle_sanitized_webview_identity_invalid".to_string());
        }
        webview_hashes.insert(webview.executable_sha256.as_str());
        webview_sizes.insert(webview.executable_size);
        webview_paths.insert(webview.executable_path.relative_leaf.to_ascii_lowercase());
    }
    if webview_hashes.len() != 1 || webview_sizes.len() != 1 || webview_paths.len() != 1 {
        return Err("lifecycle_sanitized_webview_identity_invalid".to_string());
    }
    validate_sanitized_collector_runtime(observation.phase, &observation.collector_runtime, plan)?;
    validate_sanitized_desktop_process_roles(observation)?;
    validate_sanitized_second_instance(observation)?;
    validate_desktop_visible(observation.phase, &observation.visible)
}

fn validate_sanitized_desktop_process(
    process: &SanitizedDesktopProcessObservation,
) -> Result<(), String> {
    if process.process_id == 0
        || process.started_at_100ns == 0
        || process.session_id == 0
        || process.elevated
        || process.executable_size == 0
    {
        return Err("lifecycle_sanitized_desktop_process_invalid".to_string());
    }
    validate_logical_path(&process.executable_path)?;
    validate_sha256(&process.executable_sha256, "sanitized_desktop_process")
}

fn valid_webview_logical_path(path: &LogicalPath) -> bool {
    if path.root != LogicalRoot::WebViewRuntime {
        return false;
    }
    let mut components = path.relative_leaf.split('/');
    let Some(version) = components.next() else {
        return false;
    };
    let Some(leaf) = components.next() else {
        return false;
    };
    components.next().is_none()
        && valid_webview_version(version)
        && leaf.eq_ignore_ascii_case("msedgewebview2.exe")
}

fn valid_webview_version(version: &str) -> bool {
    !version.is_empty()
        && version
            .split('.')
            .all(|segment| !segment.is_empty() && segment.bytes().all(|byte| byte.is_ascii_digit()))
}

fn validate_sanitized_collector_runtime(
    phase: DesktopPhase,
    runtime: &SanitizedDesktopCollectorRuntimeObservation,
    plan: &ProofPlan,
) -> Result<(), String> {
    let expected_service_sha256 = match phase {
        DesktopPhase::BaselinePrimary | DesktopPhase::BaselineSecondInstance => {
            &plan.baseline.service_sha256
        }
        DesktopPhase::FinalPrimary
        | DesktopPhase::FinalMissingService
        | DesktopPhase::FinalStoppedService => &plan.final_candidate.service_sha256,
        DesktopPhase::FinalIncompatibleService => &plan.incompatible_service_fixture.sha256,
    };
    let service_expected = phase != DesktopPhase::FinalMissingService;
    let running_expected = !matches!(
        phase,
        DesktopPhase::FinalMissingService | DesktopPhase::FinalStoppedService
    );

    if service_expected {
        let service = runtime
            .installed_service
            .as_ref()
            .ok_or_else(|| "lifecycle_sanitized_installed_service_missing".to_string())?;
        validate_sanitized_desktop_file(service, "installed_service")?;
        if service.executable_path.root != LogicalRoot::Install
            || service.executable_path.relative_leaf != "batcave-collector-service.exe"
            || service.executable_sha256.as_str() != expected_service_sha256
            || (phase == DesktopPhase::FinalIncompatibleService
                && service.executable_size != plan.incompatible_service_fixture.size)
        {
            return Err("lifecycle_sanitized_installed_service_identity_invalid".to_string());
        }
    } else if runtime.installed_service.is_some() {
        return Err("lifecycle_sanitized_installed_service_unexpected".to_string());
    }

    if running_expected {
        let service = runtime
            .installed_service
            .as_ref()
            .ok_or_else(|| "lifecycle_sanitized_installed_service_missing".to_string())?;
        let process = runtime
            .service_process
            .as_ref()
            .ok_or_else(|| "lifecycle_sanitized_service_process_missing".to_string())?;
        validate_sanitized_service_process(process)?;
        if process.executable_path != service.executable_path
            || process.executable_size != service.executable_size
            || process.executable_sha256 != service.executable_sha256
            || runtime.pipe_server_process_id != Some(process.process_id)
        {
            return Err("lifecycle_sanitized_service_runtime_invalid".to_string());
        }
    } else if runtime.service_process.is_some() || runtime.pipe_server_process_id.is_some() {
        return Err("lifecycle_sanitized_service_runtime_unexpected".to_string());
    }
    Ok(())
}

fn validate_sanitized_desktop_file(
    file: &SanitizedDesktopFileObservation,
    label: &str,
) -> Result<(), String> {
    if file.executable_size == 0 {
        return Err(format!("lifecycle_sanitized_{label}_identity_invalid"));
    }
    validate_logical_path(&file.executable_path)?;
    validate_sha256(
        &file.executable_sha256,
        &format!("sanitized_{label}_executable"),
    )
}

fn validate_sanitized_service_process(
    process: &SanitizedDesktopServiceProcessObservation,
) -> Result<(), String> {
    if process.process_id == 0 || process.started_at_100ns == 0 || !process.local_system {
        return Err("lifecycle_sanitized_service_process_identity_invalid".to_string());
    }
    validate_sanitized_desktop_file(
        &SanitizedDesktopFileObservation {
            executable_path: process.executable_path.clone(),
            executable_size: process.executable_size,
            executable_sha256: process.executable_sha256.clone(),
        },
        "service_process",
    )
}

fn validate_sanitized_desktop_process_roles(
    observation: &SanitizedDesktopPhaseObservation,
) -> Result<(), String> {
    let mut desktop_process_ids = observation
        .process_tree
        .iter()
        .map(|process| process.process_id)
        .collect::<BTreeSet<_>>();
    desktop_process_ids.insert(observation.desktop.process_id);

    let service_process_id = observation
        .collector_runtime
        .service_process
        .as_ref()
        .map(|process| process.process_id);
    if service_process_id.is_some_and(|process_id| desktop_process_ids.contains(&process_id)) {
        return Err("lifecycle_sanitized_desktop_process_role_collision".to_string());
    }
    if let Some(second) = &observation.second_instance {
        let attempted_process_id = second.attempted_process.process_id;
        if desktop_process_ids.contains(&attempted_process_id)
            || service_process_id == Some(attempted_process_id)
        {
            return Err("lifecycle_sanitized_desktop_process_role_collision".to_string());
        }
    }
    Ok(())
}

fn validate_sanitized_second_instance(
    observation: &SanitizedDesktopPhaseObservation,
) -> Result<(), String> {
    if observation.phase == DesktopPhase::BaselineSecondInstance {
        let second = observation
            .second_instance
            .as_ref()
            .ok_or_else(|| "lifecycle_sanitized_second_instance_missing".to_string())?;
        validate_sanitized_desktop_process(&second.attempted_process)?;
        if second.attempted_process.parent_process_id.is_some()
            || second.attempted_process.process_id == observation.desktop.process_id
            || second.attempted_process.session_id != observation.desktop.session_id
            || second.attempted_process.executable_path != observation.desktop.executable_path
            || second.attempted_process.executable_size != observation.desktop.executable_size
            || second.attempted_process.executable_sha256 != observation.desktop.executable_sha256
            || second.terminal_exit_code != 0
            || !second.process_tree_settled
            || second.focused_primary_process_id != observation.desktop.process_id
            || second.focused_primary_started_at_100ns != observation.desktop.started_at_100ns
            || second.service_instance_id_before != second.service_instance_id_after
            || !valid_service_instance_id(&second.service_instance_id_before)
            || observation.visible.service_instance_id.as_deref()
                != Some(second.service_instance_id_before.as_str())
        {
            return Err("lifecycle_sanitized_second_instance_invalid".to_string());
        }
    } else if observation.second_instance.is_some() {
        return Err("lifecycle_sanitized_second_instance_unexpected".to_string());
    }
    Ok(())
}

fn validate_desktop_machine_state(
    phase: DesktopPhase,
    machine: &SanitizedMachineSnapshot,
    plan: &ProofPlan,
) -> Result<(), String> {
    match phase {
        DesktopPhase::FinalPrimary => {
            validate_installed_machine(machine, final_artifacts(plan), ServiceExpectation::Running)
        }
        DesktopPhase::BaselinePrimary | DesktopPhase::BaselineSecondInstance => {
            validate_installed_machine(
                machine,
                baseline_artifacts(plan),
                ServiceExpectation::Running,
            )
        }
        DesktopPhase::FinalMissingService => {
            validate_desktop_only_machine(machine, final_artifacts(plan))
        }
        DesktopPhase::FinalStoppedService => validate_installed_machine(
            machine,
            final_artifacts(plan),
            ServiceExpectation::Stopped {
                win32_exit_code: 0,
                service_specific_exit_code: 0,
            },
        ),
        DesktopPhase::FinalIncompatibleService => validate_installed_machine(
            machine,
            incompatible_artifacts(plan),
            ServiceExpectation::Running,
        ),
    }?;
    require_legacy_cli(machine, None)
}

fn validate_final_absence(machine: &SanitizedMachineSnapshot) -> Result<(), String> {
    validate_product_absence(machine, true)
}

fn validate_product_absence(
    machine: &SanitizedMachineSnapshot,
    require_unknown_sentinel: bool,
) -> Result<(), String> {
    let no_product_processes = match &machine.product_processes {
        Observation::Absent => true,
        Observation::Present(processes) => processes.is_empty(),
        Observation::Unknown(_) => false,
    };
    if !matches!(machine.service, Observation::Absent)
        || !matches!(machine.install_root, Observation::Absent)
        || !matches!(machine.monitor, Observation::Absent)
        || !matches!(machine.service_binary, Observation::Absent)
        || !matches!(machine.uninstaller, Observation::Absent)
        || !matches!(machine.legacy_cli, Observation::Absent)
        || !matches!(machine.uninstall_registry, Observation::Absent)
        || !no_product_processes
        || !matches!(machine.product_data_root, Observation::Absent)
        || !matches!(machine.service_data_root, Observation::Absent)
        || !matches!(machine.installed_boundaries, Observation::Absent)
        || machine.service_registry_key.is_some()
        || !runtime_residue_absent(machine)
        || !transaction_residue_absent(machine)
        || machine.machine_product_key.is_some()
        || machine.hkcu_autostart.is_some()
        || machine.public_desktop_shortcut.is_some()
        || machine.common_start_menu_shortcut.is_some()
        || !machine.known_retired_helper_artifacts.is_empty()
        || (require_unknown_sentinel != machine.unknown_helper_sentinel.is_some())
        || !directory_role_valid(
            &machine.current_user_data_root,
            LogicalRoot::CurrentUserData,
            true,
        )
    {
        return Err("lifecycle_sanitized_final_absence_invalid".to_string());
    }
    Ok(())
}

fn current_user_retention_preserved(retention: &SanitizedCurrentUserRetention) -> bool {
    [
        &retention.settings,
        &retention.cache,
        &retention.diagnostics,
    ]
    .into_iter()
    .all(|object| {
        matches!(
            (&object.before_uninstall, &object.after_uninstall),
            (Observation::Present(before), Observation::Present(after)) if before == after
        )
    })
}

fn expected_monitor_sha256(phase: DesktopPhase, plan: &ProofPlan) -> &str {
    match phase {
        DesktopPhase::BaselinePrimary | DesktopPhase::BaselineSecondInstance => {
            &plan.baseline.monitor_sha256
        }
        DesktopPhase::FinalPrimary
        | DesktopPhase::FinalMissingService
        | DesktopPhase::FinalStoppedService
        | DesktopPhase::FinalIncompatibleService => &plan.final_candidate.monitor_sha256,
    }
}

fn normalize_absolute_windows_path(value: &str) -> Result<String, String> {
    if value.is_empty() || value.len() > 32_768 || value.contains('\0') {
        return Err("lifecycle_sanitized_path_invalid".to_string());
    }
    let value = value.strip_prefix(r"\\?\").unwrap_or(value);
    if value.starts_with(r"\\") || value.starts_with("//") {
        return Err("lifecycle_sanitized_path_invalid".to_string());
    }
    let bytes = value.as_bytes();
    if bytes.len() < 3
        || !bytes[0].is_ascii_alphabetic()
        || bytes[1] != b':'
        || !matches!(bytes[2], b'\\' | b'/')
    {
        return Err("lifecycle_sanitized_path_not_drive_absolute".to_string());
    }
    let mut normalized = value.replace('/', r"\");
    while normalized.ends_with('\\') && normalized.len() > 3 {
        normalized.pop();
    }
    if normalized
        .split('\\')
        .enumerate()
        .any(|(index, component)| {
            component.is_empty()
                || matches!(component, "." | "..")
                || component.ends_with('.')
                || component.ends_with(' ')
                || reserved_device_component(component)
                || (index > 0 && component.contains(':'))
        })
    {
        return Err("lifecycle_sanitized_path_component_invalid".to_string());
    }
    Ok(normalized)
}

fn strip_windows_root(value: &str, root: &str) -> Option<String> {
    if value.eq_ignore_ascii_case(root) {
        return Some(String::new());
    }
    let remainder = value.get(root.len()..)?;
    if value
        .get(..root.len())
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case(root))
        && remainder.starts_with('\\')
    {
        return Some(remainder.trim_start_matches('\\').to_string());
    }
    None
}

fn validate_relative_leaf(value: &str) -> Result<(), String> {
    let value = value.replace('/', r"\");
    if value.is_empty() {
        return Ok(());
    }
    let lower = value.to_ascii_lowercase();
    if value.len() > 4096
        || value.starts_with('\\')
        || value.ends_with('\\')
        || value.contains(':')
        || lower.contains("batcavelifecycleproof-v1-")
        || lower.contains('%')
        || value.split('\\').any(|component| {
            component.is_empty()
                || matches!(component, "." | "..")
                || component.eq_ignore_ascii_case("users")
                || component.ends_with('.')
                || component.ends_with(' ')
                || reserved_device_component(component)
        })
    {
        return Err("lifecycle_sanitized_path_leaf_invalid".to_string());
    }
    Ok(())
}

fn reserved_device_component(value: &str) -> bool {
    let stem = value.split('.').next().unwrap_or(value);
    matches!(
        stem.to_ascii_uppercase().as_str(),
        "CON"
            | "PRN"
            | "AUX"
            | "NUL"
            | "COM1"
            | "COM2"
            | "COM3"
            | "COM4"
            | "COM5"
            | "COM6"
            | "COM7"
            | "COM8"
            | "COM9"
            | "LPT1"
            | "LPT2"
            | "LPT3"
            | "LPT4"
            | "LPT5"
            | "LPT6"
            | "LPT7"
            | "LPT8"
            | "LPT9"
    )
}

fn valid_leaf_name(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 260
        && !value
            .chars()
            .any(|character| matches!(character, '\\' | '/' | ':' | '\0'))
        && !reserved_device_component(value)
}

fn valid_bounded_reason(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 192
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b':' | b'_' | b'-' | b'.'))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::windows_lifecycle_proof_contract::{
        parse_plan, DesktopCollectorState, DesktopPrivilegedSource,
    };

    #[test]
    fn private_projection_gate_remains_fail_closed_until_typed_binding_exists() {
        assert_eq!(
            require_private_evidence_projection_ready(),
            Err("lifecycle_private_evidence_projection_not_reviewed".to_string())
        );
    }

    fn roots() -> Vec<SanitizationRoot> {
        vec![
            SanitizationRoot::new(LogicalRoot::Install, r"C:\Program Files\BatCave Monitor")
                .expect("install root"),
            SanitizationRoot::new(
                LogicalRoot::ServiceData,
                r"C:\ProgramData\BatCaveMonitor\Service",
            )
            .expect("service root"),
            SanitizationRoot::new(
                LogicalRoot::Evidence,
                r"C:\ProgramData\BatCaveLifecycleProof-v1-nonce",
            )
            .expect("evidence root"),
        ]
    }

    #[test]
    fn sanitization_uses_the_most_specific_logical_root() {
        assert_eq!(
            sanitize_path(
                r"C:\ProgramData\BatCaveMonitor\Service\etw-lease.json",
                &roots()
            ),
            Ok(LogicalPath {
                root: LogicalRoot::ServiceData,
                relative_leaf: "etw-lease.json".to_string(),
            })
        );
        assert_eq!(
            sanitize_path(
                r"\\?\C:\Program Files\BatCave Monitor\batcave-monitor.exe",
                &roots()
            ),
            Ok(LogicalPath {
                root: LogicalRoot::Install,
                relative_leaf: "batcave-monitor.exe".to_string(),
            })
        );
        assert_eq!(
            sanitize_path(r"C:\Program Files\BatCave Monitor", &roots()),
            Ok(LogicalPath {
                root: LogicalRoot::Install,
                relative_leaf: String::new(),
            })
        );
    }

    #[test]
    fn sanitization_rejects_paths_that_could_leak_or_escape() {
        for path in [
            r"\\server\share\BatCave\file.exe",
            r"C:\Users\albert\private.txt",
            r"C:\Program Files\BatCave Monitor\..\secret.txt",
            r"C:\Program Files\BatCave Monitor\file.exe:secret",
            r"C:\Program Files\BatCave Monitor\NUL",
            "C:\\Program Files\\BatCave Monitor\\file.exe. ",
            r"C:\Program Files\BatCave MonitorX\file.exe",
        ] {
            assert!(sanitize_path(path, &roots()).is_err(), "{path}");
        }
    }

    #[test]
    fn sanitization_output_contains_no_absolute_or_nonce_qualified_path() {
        let output = serde_json::to_string(
            &sanitize_path(
                r"C:\ProgramData\BatCaveLifecycleProof-v1-nonce\final-state.private.json",
                &roots(),
            )
            .expect("sanitized"),
        )
        .expect("json");
        assert!(!output.contains("C:"));
        assert!(!output.contains("ProgramData"));
        assert!(!output.contains("nonce"));
        assert!(output.contains("\"root\":\"evidence\""));
        assert!(output.contains("final-state.private.json"));
    }

    #[test]
    fn sanitized_export_requires_complete_typed_manifest() {
        let plan = parse_plan().expect("plan");
        let receipts = success_receipts();
        let packet = valid_export(&plan, &receipts);
        let pre_uninstall_install_id = packet
            .private_evidence
            .iter()
            .find(|entry| entry.receipt.name == "final-repair-state.private.json")
            .expect("final repair")
            .machine
            .etw_session
            .as_ref()
            .expect("pre-uninstall ETW")
            .lease
            .install_id;
        let post_reinstall_install_id = packet
            .private_evidence
            .iter()
            .find(|entry| entry.receipt.name == "baseline-install-state.private.json")
            .expect("baseline install")
            .machine
            .etw_session
            .as_ref()
            .expect("post-reinstall ETW")
            .lease
            .install_id;
        assert_ne!(pre_uninstall_install_id, post_reinstall_install_id);
        assert_eq!(
            validate_sanitized_export_bytes(
                &serde_json::to_vec(&packet).expect("packet"),
                &plan,
                &"c".repeat(40),
                &"d".repeat(64),
                &receipts,
            ),
            Ok(())
        );

        let mut incomplete = packet.clone();
        incomplete.private_evidence.pop();
        assert!(validate_sanitized_export_bytes(
            &serde_json::to_vec(&incomplete).expect("incomplete"),
            &plan,
            &"c".repeat(40),
            &"d".repeat(64),
            &receipts,
        )
        .is_err());

        let almost_empty = serde_json::json!({ "schema_version": SANITIZED_SCHEMA });
        assert!(validate_sanitized_export_bytes(
            &serde_json::to_vec(&almost_empty).expect("almost empty"),
            &plan,
            &"c".repeat(40),
            &"d".repeat(64),
            &receipts,
        )
        .is_err());
    }

    #[test]
    fn sanitized_desktop_packets_match_the_exact_parent_observations() {
        let plan = parse_plan().expect("plan");
        let receipts = success_receipts();
        let parent_results = parent_desktop_results(&plan);
        let packet = valid_export(&plan, &receipts);
        assert_eq!(
            validate_sanitized_export_bytes_with_parent_results(
                &serde_json::to_vec(&packet).expect("packet"),
                &plan,
                &"c".repeat(40),
                &"d".repeat(64),
                &receipts,
                &parent_results,
            ),
            Ok(())
        );

        let mut drift = packet;
        let drifted_instance = "00000065-00000000000000000000000000000002".to_string();
        drift
            .private_evidence
            .iter_mut()
            .find(|entry| entry.receipt.name == "final-primary-desktop.private.json")
            .expect("final desktop")
            .desktop_phase
            .as_mut()
            .expect("desktop phase")
            .visible
            .service_instance_id = Some(drifted_instance.clone());
        for name in [
            "final-repair-state.private.json",
            "final-primary-desktop.private.json",
        ] {
            drift
                .private_evidence
                .iter_mut()
                .find(|entry| entry.receipt.name == name)
                .expect("final repair observation")
                .machine
                .etw_session
                .as_mut()
                .expect("ETW")
                .lease
                .service_instance_id = digest16(drifted_instance.as_bytes());
        }
        let bytes = serde_json::to_vec(&drift).expect("drift");
        assert!(validate_sanitized_export_bytes(
            &bytes,
            &plan,
            &"c".repeat(40),
            &"d".repeat(64),
            &receipts,
        )
        .is_ok());
        assert_eq!(
            validate_sanitized_export_bytes_with_parent_results(
                &bytes,
                &plan,
                &"c".repeat(40),
                &"d".repeat(64),
                &receipts,
                &parent_results,
            ),
            Err("lifecycle_sanitized_parent_desktop_result_mismatch".to_string())
        );
    }

    #[test]
    fn sanitized_export_rejects_drive_relative_ads_device_and_unknown_fields() {
        let plan = parse_plan().expect("plan");
        let receipts = success_receipts();
        for relative_leaf in ["C:Users/albert/private.txt", "file.txt:secret", "NUL"] {
            let mut packet = valid_export(&plan, &receipts);
            packet.private_evidence[0].machine.current_user_data_root =
                Observation::Present(directory(LogicalRoot::CurrentUserData, relative_leaf));
            assert!(validate_sanitized_export_bytes(
                &serde_json::to_vec(&packet).expect("hostile"),
                &plan,
                &"c".repeat(40),
                &"d".repeat(64),
                &receipts,
            )
            .is_err());
        }

        let mut value = serde_json::to_value(valid_export(&plan, &receipts)).expect("packet value");
        value
            .as_object_mut()
            .expect("packet object")
            .insert("path".to_string(), serde_json::json!(r"C:\Users\albert"));
        assert!(validate_sanitized_export_bytes(
            &serde_json::to_vec(&value).expect("unknown field"),
            &plan,
            &"c".repeat(40),
            &"d".repeat(64),
            &receipts,
        )
        .is_err());
    }

    #[test]
    fn sanitized_export_binds_stage_state_final_residue_and_user_retention() {
        let plan = parse_plan().expect("plan");
        let receipts = success_receipts();

        let mut wrong_stage = valid_export(&plan, &receipts);
        wrong_stage
            .private_evidence
            .iter_mut()
            .find(|entry| entry.receipt.name == "final-upgrade-state.private.json")
            .expect("final upgrade")
            .machine = absent_machine(false);
        assert!(validate_sanitized_export_bytes(
            &serde_json::to_vec(&wrong_stage).expect("wrong stage"),
            &plan,
            &"c".repeat(40),
            &"d".repeat(64),
            &receipts,
        )
        .is_err());

        let mut residue = valid_export(&plan, &receipts);
        residue
            .private_evidence
            .iter_mut()
            .find(|entry| entry.receipt.name == "final-uninstall-state.private.json")
            .expect("final uninstall")
            .machine
            .staged_service_images
            .push(path_file(
                LogicalRoot::Install,
                "batcave-collector-service.0.2.0.staged.exe",
                "9",
            ));
        assert!(validate_sanitized_export_bytes(
            &serde_json::to_vec(&residue).expect("residue"),
            &plan,
            &"c".repeat(40),
            &"d".repeat(64),
            &receipts,
        )
        .is_err());

        let mut retention_drift = valid_export(&plan, &receipts);
        retention_drift
            .current_user_retention
            .settings
            .after_uninstall = Observation::Present(SanitizedDigestSnapshot {
            size: 1,
            sha256: "f".repeat(64),
        });
        assert!(validate_sanitized_export_bytes(
            &serde_json::to_vec(&retention_drift).expect("retention drift"),
            &plan,
            &"c".repeat(40),
            &"d".repeat(64),
            &receipts,
        )
        .is_err());

        let mut absent_retention = valid_export(&plan, &receipts);
        absent_retention
            .current_user_retention
            .diagnostics
            .before_uninstall = Observation::Absent;
        absent_retention
            .current_user_retention
            .diagnostics
            .after_uninstall = Observation::Absent;
        assert!(validate_sanitized_export_bytes(
            &serde_json::to_vec(&absent_retention).expect("absent retention"),
            &plan,
            &"c".repeat(40),
            &"d".repeat(64),
            &receipts,
        )
        .is_err());

        let mut aliased_retention = valid_export(&plan, &receipts);
        aliased_retention.current_user_retention.cache.path = aliased_retention
            .current_user_retention
            .settings
            .path
            .clone();
        assert!(validate_sanitized_export_bytes(
            &serde_json::to_vec(&aliased_retention).expect("aliased retention"),
            &plan,
            &"c".repeat(40),
            &"d".repeat(64),
            &receipts,
        )
        .is_err());

        let mut wrong_retention_source = valid_export(&plan, &receipts);
        wrong_retention_source
            .current_user_retention
            .before_uninstall_source = receipts
            .iter()
            .find(|receipt| receipt.name == "final-uninstall-state.private.json")
            .expect("final uninstall receipt")
            .clone();
        assert_eq!(
            validate_sanitized_export_bytes(
                &serde_json::to_vec(&wrong_retention_source).expect("wrong retention source"),
                &plan,
                &"c".repeat(40),
                &"d".repeat(64),
                &receipts,
            ),
            Err("lifecycle_sanitized_user_retention_source_invalid".to_string())
        );

        let mut missing_service_cli_residue = valid_export(&plan, &receipts);
        missing_service_cli_residue
            .private_evidence
            .iter_mut()
            .find(|entry| entry.receipt.name == "final-missing-service-state.private.json")
            .expect("final missing service")
            .machine
            .legacy_cli = Observation::Present(file(&plan.allowlisted_start.legacy_cli_sha256));
        assert!(validate_sanitized_export_bytes(
            &serde_json::to_vec(&missing_service_cli_residue).expect("legacy cli residue"),
            &plan,
            &"c".repeat(40),
            &"d".repeat(64),
            &receipts,
        )
        .is_err());

        let mut wrong_directory_role = valid_export(&plan, &receipts);
        wrong_directory_role
            .private_evidence
            .iter_mut()
            .find(|entry| entry.receipt.name == "final-upgrade-state.private.json")
            .expect("final upgrade")
            .machine
            .install_root = Observation::Present(directory(LogicalRoot::CurrentUserData, ""));
        assert!(validate_sanitized_export_bytes(
            &serde_json::to_vec(&wrong_directory_role).expect("wrong directory role"),
            &plan,
            &"c".repeat(40),
            &"d".repeat(64),
            &receipts,
        )
        .is_err());

        let mut unhealthy_etw = valid_export(&plan, &receipts);
        unhealthy_etw
            .private_evidence
            .iter_mut()
            .find(|entry| entry.receipt.name == "final-upgrade-state.private.json")
            .expect("final upgrade")
            .machine
            .etw_session
            .as_mut()
            .expect("etw")
            .events_lost = 1;
        assert!(validate_sanitized_export_bytes(
            &serde_json::to_vec(&unhealthy_etw).expect("unhealthy etw"),
            &plan,
            &"c".repeat(40),
            &"d".repeat(64),
            &receipts,
        )
        .is_err());

        let mut stale_etw_owner = valid_export(&plan, &receipts);
        stale_etw_owner
            .private_evidence
            .iter_mut()
            .find(|entry| entry.receipt.name == "final-upgrade-state.private.json")
            .expect("final upgrade")
            .machine
            .etw_session
            .as_mut()
            .expect("etw")
            .lease
            .controller
            .process_started_at = 5_499;
        assert!(validate_sanitized_export_bytes(
            &serde_json::to_vec(&stale_etw_owner).expect("stale etw owner"),
            &plan,
            &"c".repeat(40),
            &"d".repeat(64),
            &receipts,
        )
        .is_err());

        let mut mismatched_etw_session = valid_export(&plan, &receipts);
        mismatched_etw_session
            .private_evidence
            .iter_mut()
            .find(|entry| entry.receipt.name == "final-upgrade-state.private.json")
            .expect("final upgrade")
            .machine
            .etw_session
            .as_mut()
            .expect("etw")
            .observed_session
            .configuration_digest = [9; 32];
        assert!(validate_sanitized_export_bytes(
            &serde_json::to_vec(&mismatched_etw_session).expect("mismatched etw session"),
            &plan,
            &"c".repeat(40),
            &"d".repeat(64),
            &receipts,
        )
        .is_err());

        let mut forged_service_generation = valid_export(&plan, &receipts);
        forged_service_generation
            .private_evidence
            .iter_mut()
            .find(|entry| entry.receipt.name == "final-upgrade-state.private.json")
            .expect("final upgrade")
            .machine
            .etw_session
            .as_mut()
            .expect("etw")
            .lease
            .service_generation[0] ^= 1;
        assert!(validate_sanitized_export_bytes(
            &serde_json::to_vec(&forged_service_generation).expect("forged service generation"),
            &plan,
            &"c".repeat(40),
            &"d".repeat(64),
            &receipts,
        )
        .is_err());

        let mut visible_instance_mismatch = valid_export(&plan, &receipts);
        visible_instance_mismatch
            .private_evidence
            .iter_mut()
            .find(|entry| entry.receipt.name == "final-primary-desktop.private.json")
            .expect("final desktop")
            .desktop_phase
            .as_mut()
            .expect("desktop phase")
            .visible
            .service_instance_id = Some(test_service_instance_id(55, 99));
        assert!(validate_sanitized_export_bytes(
            &serde_json::to_vec(&visible_instance_mismatch).expect("visible instance mismatch"),
            &plan,
            &"c".repeat(40),
            &"d".repeat(64),
            &receipts,
        )
        .is_err());

        for mutate in [
            |etw: &mut SanitizedEtwObservation| etw.lease.install_id[0] ^= 1,
            |etw: &mut SanitizedEtwObservation| etw.lease.boot_identity[0] ^= 1,
        ] {
            let mut run_identity_drift = valid_export(&plan, &receipts);
            mutate(
                run_identity_drift
                    .private_evidence
                    .iter_mut()
                    .find(|entry| entry.receipt.name == "final-restart-state.private.json")
                    .expect("final restart")
                    .machine
                    .etw_session
                    .as_mut()
                    .expect("etw"),
            );
            assert!(validate_sanitized_export_bytes(
                &serde_json::to_vec(&run_identity_drift).expect("run identity drift"),
                &plan,
                &"c".repeat(40),
                &"d".repeat(64),
                &receipts,
            )
            .is_err());
        }

        let mut aliased_locks = valid_export(&plan, &receipts);
        let machine = &mut aliased_locks
            .private_evidence
            .iter_mut()
            .find(|entry| entry.receipt.name == "final-upgrade-state.private.json")
            .expect("final upgrade")
            .machine;
        machine.etw_owner_lock = machine.etw_lease_file.clone();
        machine.service_lifecycle_lock = machine.etw_lease_file.clone();
        assert!(validate_sanitized_export_bytes(
            &serde_json::to_vec(&aliased_locks).expect("aliased locks"),
            &plan,
            &"c".repeat(40),
            &"d".repeat(64),
            &receipts,
        )
        .is_err());

        let mut surviving_process = valid_export(&plan, &receipts);
        surviving_process
            .private_evidence
            .iter_mut()
            .find(|entry| entry.receipt.name == "final-upgrade-state.private.json")
            .expect("final upgrade")
            .machine
            .product_processes = Observation::Present(vec![SanitizedProcessSnapshot {
            process_id: 77,
            executable_name: "batcave-monitor.exe".to_string(),
            executable_path: Some(logical_path(LogicalRoot::Install, "batcave-monitor.exe")),
        }]);
        assert!(validate_sanitized_export_bytes(
            &serde_json::to_vec(&surviving_process).expect("surviving process"),
            &plan,
            &"c".repeat(40),
            &"d".repeat(64),
            &receipts,
        )
        .is_err());

        let mut malformed_webview_version = valid_export(&plan, &receipts);
        malformed_webview_version
            .private_evidence
            .iter_mut()
            .find(|entry| entry.receipt.name == "final-primary-desktop.private.json")
            .expect("final desktop")
            .desktop_phase
            .as_mut()
            .expect("desktop phase")
            .process_tree[0]
            .executable_path
            .relative_leaf = "./msedgewebview2.exe".to_string();
        assert!(validate_sanitized_export_bytes(
            &serde_json::to_vec(&malformed_webview_version).expect("malformed webview version"),
            &plan,
            &"c".repeat(40),
            &"d".repeat(64),
            &receipts,
        )
        .is_err());

        let mut colliding_second = valid_export(&plan, &receipts);
        colliding_second
            .private_evidence
            .iter_mut()
            .find(|entry| entry.receipt.name == "baseline-second-instance-desktop.private.json")
            .expect("second desktop")
            .desktop_phase
            .as_mut()
            .expect("desktop phase")
            .second_instance
            .as_mut()
            .expect("second instance")
            .attempted_process
            .process_id = 102;
        assert!(validate_sanitized_export_bytes(
            &serde_json::to_vec(&colliding_second).expect("colliding second"),
            &plan,
            &"c".repeat(40),
            &"d".repeat(64),
            &receipts,
        )
        .is_err());

        let mut colliding_service = valid_export(&plan, &receipts);
        let desktop = colliding_service
            .private_evidence
            .iter_mut()
            .find(|entry| entry.receipt.name == "final-primary-desktop.private.json")
            .expect("final desktop")
            .desktop_phase
            .as_mut()
            .expect("desktop phase");
        let service = desktop
            .collector_runtime
            .service_process
            .as_mut()
            .expect("service");
        service.process_id = 102;
        service.started_at_100ns = 10_200;
        desktop.collector_runtime.pipe_server_process_id = Some(102);
        assert!(validate_sanitized_export_bytes(
            &serde_json::to_vec(&colliding_service).expect("colliding service"),
            &plan,
            &"c".repeat(40),
            &"d".repeat(64),
            &receipts,
        )
        .is_err());

        let mut contradictory_desktop_machine = valid_export(&plan, &receipts);
        let desktop = contradictory_desktop_machine
            .private_evidence
            .iter_mut()
            .find(|entry| entry.receipt.name == "final-primary-desktop.private.json")
            .expect("final desktop")
            .desktop_phase
            .as_mut()
            .expect("desktop phase");
        let service = desktop
            .collector_runtime
            .service_process
            .as_mut()
            .expect("service");
        service.process_id = 77;
        service.started_at_100ns = 7_700;
        desktop.collector_runtime.pipe_server_process_id = Some(77);
        assert!(validate_sanitized_export_bytes(
            &serde_json::to_vec(&contradictory_desktop_machine)
                .expect("contradictory desktop machine"),
            &plan,
            &"c".repeat(40),
            &"d".repeat(64),
            &receipts,
        )
        .is_err());

        let mut replayed_generation = valid_export(&plan, &receipts);
        let prior_instance_id = replayed_generation
            .private_evidence
            .iter()
            .find(|entry| entry.receipt.name == "final-upgrade-state.private.json")
            .expect("final upgrade")
            .machine
            .etw_session
            .as_ref()
            .expect("etw")
            .lease
            .service_instance_id;
        replayed_generation
            .private_evidence
            .iter_mut()
            .find(|entry| entry.receipt.name == "final-restart-state.private.json")
            .expect("final restart")
            .machine
            .etw_session
            .as_mut()
            .expect("etw")
            .lease
            .service_instance_id = prior_instance_id;
        assert!(validate_sanitized_export_bytes(
            &serde_json::to_vec(&replayed_generation).expect("replayed generation"),
            &plan,
            &"c".repeat(40),
            &"d".repeat(64),
            &receipts,
        )
        .is_err());

        let mut weakened_boundary = valid_export(&plan, &receipts);
        let Observation::Present(boundary) = &mut weakened_boundary
            .private_evidence
            .iter_mut()
            .find(|entry| entry.receipt.name == "final-restart-state.private.json")
            .expect("final restart")
            .machine
            .installed_boundaries
        else {
            panic!("installed boundary");
        };
        boundary.service_data_root_dacl_sha256 = "f".repeat(64);
        assert!(validate_sanitized_export_bytes(
            &serde_json::to_vec(&weakened_boundary).expect("weakened boundary"),
            &plan,
            &"c".repeat(40),
            &"d".repeat(64),
            &receipts,
        )
        .is_err());

        let mut nested_retained_root = valid_export(&plan, &receipts);
        nested_retained_root
            .private_evidence
            .iter_mut()
            .find(|entry| entry.receipt.name == "final-uninstall-state.private.json")
            .expect("final uninstall")
            .machine
            .current_user_data_root =
            Observation::Present(directory(LogicalRoot::CurrentUserData, "nested"));
        assert!(validate_sanitized_export_bytes(
            &serde_json::to_vec(&nested_retained_root).expect("nested retained root"),
            &plan,
            &"c".repeat(40),
            &"d".repeat(64),
            &receipts,
        )
        .is_err());
    }

    fn success_receipts() -> Vec<EvidenceReceipt> {
        SUCCESS_PRIVATE_EVIDENCE_LEAVES
            .iter()
            .map(|name| EvidenceReceipt {
                name: (*name).to_string(),
                size: 1,
                sha256: "a".repeat(64),
            })
            .collect()
    }

    fn valid_export(plan: &ProofPlan, receipts: &[EvidenceReceipt]) -> SanitizedExportPacket {
        SanitizedExportPacket {
            schema_version: SANITIZED_SCHEMA.to_string(),
            profile: plan.profile.clone(),
            plan_sha256: plan_sha256(),
            controller_source_commit_sha: "c".repeat(40),
            controller_sha256: "d".repeat(64),
            completed_stage: LifecycleStage::FinalUninstall,
            process_tree_settled: true,
            private_evidence: receipts
                .iter()
                .map(|receipt| {
                    let phase = desktop_phase_for_leaf(&receipt.name);
                    let assertion = assertion_for_leaf(&receipt.name);
                    SanitizedPrivateEvidence {
                        receipt: receipt.clone(),
                        assertion,
                        machine: machine_for_evidence(&receipt.name, phase, plan),
                        desktop_phase: phase.map(|phase| desktop_phase(phase, plan)),
                        event: match receipt.name.as_str() {
                            "baseline-crashed-state.private.json"
                            | "final-crashed-state.private.json" => Some(
                                SanitizedStageEvent::ServiceCrash(SanitizedServiceCrashEvent {
                                    process_id: if receipt.name.starts_with("baseline-") {
                                        66
                                    } else {
                                        70
                                    },
                                    process_started_at_100ns: if receipt
                                        .name
                                        .starts_with("baseline-")
                                    {
                                        6_600
                                    } else {
                                        7_000
                                    },
                                    image_path: logical_path(
                                        LogicalRoot::Install,
                                        "batcave-collector-service.exe",
                                    ),
                                    image_sha256: if receipt.name.starts_with("baseline-") {
                                        plan.baseline.service_sha256.clone()
                                    } else {
                                        plan.final_candidate.service_sha256.clone()
                                    },
                                    process_exit_code: 1,
                                    win32_exit_code: 1066,
                                    service_specific_exit_code: 1,
                                }),
                            ),
                            "baseline-rollback-recovery-state.private.json" => {
                                Some(SanitizedStageEvent::UpgradeRollback(
                                    SanitizedUpgradeRollbackEvent {
                                        candidate_sha256: plan
                                            .rollback_failing_service_fixture
                                            .sha256
                                            .clone(),
                                        candidate_failure_code:
                                            "collector_service_proof_candidate_start_failed"
                                                .to_string(),
                                        candidate_failure_detail:
                                            "collector_service_upgrade_start_failed:1067"
                                                .to_string(),
                                        execution_marker_sha256: sha256_hex(
                                            b"batcave_windows_lifecycle_rollback_fixture_v1\n",
                                        ),
                                        restored_sha256: plan.baseline.service_sha256.clone(),
                                        restored_process_id: 68,
                                    },
                                ))
                            }
                            _ => None,
                        },
                    }
                })
                .collect(),
            final_product_absent: true,
            current_user_data_preserved: true,
            current_user_retention: retention(receipts),
        }
    }

    fn desktop_phase(phase: DesktopPhase, plan: &ProofPlan) -> SanitizedDesktopPhaseObservation {
        let state = phase.expected_collector_state();
        let active = state == DesktopCollectorState::Active;
        let incompatible = state == DesktopCollectorState::Incompatible;
        let service_instance_id = active.then(|| {
            let (process_id, _, generation) =
                test_generation_for_phase(phase).expect("active generation");
            test_service_instance_id(process_id, generation)
        });
        SanitizedDesktopPhaseObservation {
            phase,
            process_tree_settled: true,
            desktop: sanitized_process(
                101,
                None,
                LogicalRoot::Install,
                "batcave-monitor.exe",
                expected_monitor_sha256(phase, plan),
            ),
            process_tree: vec![sanitized_process(
                102,
                Some(101),
                LogicalRoot::WebViewRuntime,
                "1/msedgewebview2.exe",
                &"b".repeat(64),
            )],
            webview_process_ids: vec![102],
            second_instance: (phase == DesktopPhase::BaselineSecondInstance).then(|| {
                SanitizedDesktopSecondInstanceObservation {
                    attempted_process: sanitized_process(
                        103,
                        None,
                        LogicalRoot::Install,
                        "batcave-monitor.exe",
                        expected_monitor_sha256(phase, plan),
                    ),
                    terminal_exit_code: 0,
                    process_tree_settled: true,
                    focused_primary_process_id: 101,
                    focused_primary_started_at_100ns: 10_100,
                    service_instance_id_before: service_instance_id
                        .clone()
                        .expect("second instance service identity"),
                    service_instance_id_after: service_instance_id
                        .clone()
                        .expect("second instance service identity"),
                }
            }),
            collector_runtime: sanitized_collector_runtime(phase, plan),
            visible: DesktopVisibleObservation {
                current_process_standard: true,
                collector_state: state,
                privileged_source: if active {
                    DesktopPrivilegedSource::InstalledCollectorService
                } else {
                    DesktopPrivilegedSource::None
                },
                standard_monitoring_current: !active,
                protected_sample_current: active,
                fallback_etw_disabled: !active,
                service_version: active
                    .then(|| env!("CARGO_PKG_VERSION").to_string())
                    .or_else(|| incompatible.then(|| "0.2.0-rc.3".to_string())),
                service_release_version: active
                    .then(|| env!("CARGO_PKG_VERSION").to_string())
                    .or_else(|| incompatible.then(|| "0.2.0-rc.3".to_string())),
                negotiated_protocol_version: active.then_some(1),
                minimum_desktop_version: active.then(|| env!("CARGO_PKG_VERSION").to_string()),
                service_instance_id,
                service_detail: if incompatible {
                    Some("collector_service_desktop_release_incompatible".to_string())
                } else {
                    match phase {
                        DesktopPhase::FinalMissingService => {
                            Some("collector_service_open_failed:1060".to_string())
                        }
                        DesktopPhase::FinalStoppedService => {
                            Some("collector_service_stopped".to_string())
                        }
                        _ => None,
                    }
                },
            },
        }
    }

    fn parent_desktop_results(plan: &ProofPlan) -> Vec<DesktopPhaseResult> {
        [
            DesktopPhase::FinalPrimary,
            DesktopPhase::BaselinePrimary,
            DesktopPhase::BaselineSecondInstance,
            DesktopPhase::FinalMissingService,
            DesktopPhase::FinalStoppedService,
            DesktopPhase::FinalIncompatibleService,
        ]
        .into_iter()
        .map(|phase| parent_desktop_result(phase, plan))
        .collect()
    }

    fn parent_desktop_result(phase: DesktopPhase, plan: &ProofPlan) -> DesktopPhaseResult {
        let sanitized = desktop_phase(phase, plan);
        DesktopPhaseResult {
            phase,
            disposition: DesktopPhaseDisposition::Passed,
            process_tree_settled: sanitized.process_tree_settled,
            observation: Some(DesktopPhaseObservation {
                desktop: raw_desktop_process(&sanitized.desktop),
                process_tree: sanitized
                    .process_tree
                    .iter()
                    .map(raw_desktop_process)
                    .collect(),
                webview_process_ids: sanitized.webview_process_ids,
                second_instance: sanitized.second_instance.as_ref().map(raw_second_instance),
                collector_runtime: raw_collector_runtime(&sanitized.collector_runtime),
                visible: sanitized.visible,
            }),
            failure_reason: None,
        }
    }

    fn raw_desktop_process(
        process: &SanitizedDesktopProcessObservation,
    ) -> DesktopProcessObservation {
        DesktopProcessObservation {
            process_id: process.process_id,
            parent_process_id: process.parent_process_id,
            started_at_100ns: process.started_at_100ns,
            session_id: process.session_id,
            elevated: process.elevated,
            executable_path: raw_path(&process.executable_path),
            executable_size: process.executable_size,
            executable_sha256: process.executable_sha256.clone(),
        }
    }

    fn raw_second_instance(
        second: &SanitizedDesktopSecondInstanceObservation,
    ) -> DesktopSecondInstanceObservation {
        DesktopSecondInstanceObservation {
            attempted_process: raw_desktop_process(&second.attempted_process),
            terminal_exit_code: second.terminal_exit_code,
            process_tree_settled: second.process_tree_settled,
            focused_primary_process_id: second.focused_primary_process_id,
            focused_primary_started_at_100ns: second.focused_primary_started_at_100ns,
            service_instance_id_before: second.service_instance_id_before.clone(),
            service_instance_id_after: second.service_instance_id_after.clone(),
        }
    }

    fn raw_collector_runtime(
        runtime: &SanitizedDesktopCollectorRuntimeObservation,
    ) -> DesktopCollectorRuntimeObservation {
        DesktopCollectorRuntimeObservation {
            installed_service: runtime.installed_service.as_ref().map(|file| {
                DesktopFileObservation {
                    executable_path: raw_path(&file.executable_path),
                    executable_size: file.executable_size,
                    executable_sha256: file.executable_sha256.clone(),
                }
            }),
            service_process: runtime.service_process.as_ref().map(|process| {
                DesktopServiceProcessObservation {
                    process_id: process.process_id,
                    started_at_100ns: process.started_at_100ns,
                    local_system: process.local_system,
                    executable_path: raw_path(&process.executable_path),
                    executable_size: process.executable_size,
                    executable_sha256: process.executable_sha256.clone(),
                }
            }),
            pipe_server_process_id: runtime.pipe_server_process_id,
        }
    }

    fn raw_path(path: &LogicalPath) -> String {
        let root = match path.root {
            LogicalRoot::Install => r"C:\Program Files\BatCave Monitor",
            LogicalRoot::WebViewRuntime => {
                r"C:\Program Files (x86)\Microsoft\EdgeWebView\Application"
            }
            _ => panic!("unexpected desktop logical root"),
        };
        if path.relative_leaf.is_empty() {
            root.to_string()
        } else {
            format!(r"{root}\{}", path.relative_leaf.replace('/', r"\"))
        }
    }

    fn sanitized_collector_runtime(
        phase: DesktopPhase,
        plan: &ProofPlan,
    ) -> SanitizedDesktopCollectorRuntimeObservation {
        if phase == DesktopPhase::FinalMissingService {
            return SanitizedDesktopCollectorRuntimeObservation {
                installed_service: None,
                service_process: None,
                pipe_server_process_id: None,
            };
        }
        let (sha256, size) = if phase == DesktopPhase::FinalIncompatibleService {
            (
                plan.incompatible_service_fixture.sha256.clone(),
                plan.incompatible_service_fixture.size,
            )
        } else if matches!(
            phase,
            DesktopPhase::BaselinePrimary | DesktopPhase::BaselineSecondInstance
        ) {
            (plan.baseline.service_sha256.clone(), 8192)
        } else {
            (plan.final_candidate.service_sha256.clone(), 8192)
        };
        let installed_service = SanitizedDesktopFileObservation {
            executable_path: logical_path(LogicalRoot::Install, "batcave-collector-service.exe"),
            executable_size: size,
            executable_sha256: sha256.clone(),
        };
        if phase == DesktopPhase::FinalStoppedService {
            return SanitizedDesktopCollectorRuntimeObservation {
                installed_service: Some(installed_service),
                service_process: None,
                pipe_server_process_id: None,
            };
        }
        let (process_id, started_at_100ns, _) =
            test_generation_for_phase(phase).expect("running generation");
        SanitizedDesktopCollectorRuntimeObservation {
            installed_service: Some(installed_service.clone()),
            service_process: Some(SanitizedDesktopServiceProcessObservation {
                process_id,
                started_at_100ns,
                local_system: true,
                executable_path: installed_service.executable_path,
                executable_size: installed_service.executable_size,
                executable_sha256: installed_service.executable_sha256,
            }),
            pipe_server_process_id: Some(process_id),
        }
    }

    fn test_generation_for_phase(phase: DesktopPhase) -> Option<(u32, u64, u64)> {
        match phase {
            DesktopPhase::FinalPrimary => Some((55, 5_500, 7)),
            DesktopPhase::BaselinePrimary | DesktopPhase::BaselineSecondInstance => {
                Some((65, 6_500, 8))
            }
            DesktopPhase::FinalIncompatibleService => Some((74, 7_400, 17)),
            DesktopPhase::FinalMissingService | DesktopPhase::FinalStoppedService => None,
        }
    }

    fn sanitized_process(
        process_id: u32,
        parent_process_id: Option<u32>,
        root: LogicalRoot,
        relative_leaf: &str,
        executable_sha256: &str,
    ) -> SanitizedDesktopProcessObservation {
        SanitizedDesktopProcessObservation {
            process_id,
            parent_process_id,
            started_at_100ns: u64::from(process_id) * 100,
            session_id: 1,
            elevated: false,
            executable_path: logical_path(root, relative_leaf),
            executable_size: 4096,
            executable_sha256: executable_sha256.to_string(),
        }
    }

    fn machine_for_evidence(
        receipt_name: &str,
        phase: Option<DesktopPhase>,
        plan: &ProofPlan,
    ) -> SanitizedMachineSnapshot {
        let mut machine = if let Some(phase) = phase {
            match phase {
                DesktopPhase::FinalPrimary => {
                    installed_machine(final_artifacts(plan), ServiceExpectation::Running, false)
                }
                DesktopPhase::BaselinePrimary | DesktopPhase::BaselineSecondInstance => {
                    installed_machine(baseline_artifacts(plan), ServiceExpectation::Running, false)
                }
                DesktopPhase::FinalMissingService => {
                    desktop_only_machine(final_artifacts(plan), true)
                }
                DesktopPhase::FinalStoppedService => installed_machine(
                    final_artifacts(plan),
                    ServiceExpectation::Stopped {
                        win32_exit_code: 0,
                        service_specific_exit_code: 0,
                    },
                    true,
                ),
                DesktopPhase::FinalIncompatibleService => installed_machine(
                    incompatible_artifacts(plan),
                    ServiceExpectation::Running,
                    true,
                ),
            }
        } else {
            match assertion_for_leaf(receipt_name) {
                SanitizedEvidenceAssertion::InitialLegacyStopped => {
                    let mut machine = installed_machine(
                        allowlisted_artifacts(plan),
                        ServiceExpectation::Stopped {
                            win32_exit_code: plan.allowlisted_start.win32_exit_code,
                            service_specific_exit_code: plan
                                .allowlisted_start
                                .service_specific_exit_code,
                        },
                        false,
                    );
                    machine.legacy_cli =
                        Observation::Present(file(&plan.allowlisted_start.legacy_cli_sha256));
                    machine
                }
                SanitizedEvidenceAssertion::FinalInstalledRunning => {
                    installed_machine(final_artifacts(plan), ServiceExpectation::Running, false)
                }
                SanitizedEvidenceAssertion::ProductAbsent => absent_machine(false),
                SanitizedEvidenceAssertion::BaselineInstalledRunning
                | SanitizedEvidenceAssertion::BaselineRecovered
                | SanitizedEvidenceAssertion::BaselineRollbackRecovered => {
                    installed_machine(baseline_artifacts(plan), ServiceExpectation::Running, false)
                }
                SanitizedEvidenceAssertion::BaselineStopped => installed_machine(
                    baseline_artifacts(plan),
                    ServiceExpectation::Stopped {
                        win32_exit_code: 0,
                        service_specific_exit_code: 0,
                    },
                    false,
                ),
                SanitizedEvidenceAssertion::BaselineCrashed => {
                    installed_machine(baseline_artifacts(plan), ServiceExpectation::Crashed, false)
                }
                SanitizedEvidenceAssertion::LegacyResidueSeeded => {
                    let mut machine = installed_machine(
                        baseline_artifacts(plan),
                        ServiceExpectation::Running,
                        true,
                    );
                    machine.legacy_cli =
                        Observation::Present(file(&plan.allowlisted_start.legacy_cli_sha256));
                    machine.known_retired_helper_artifacts = KNOWN_RETIRED_HELPER_LEAVES
                        .iter()
                        .map(|leaf| known_helper_path_file(leaf))
                        .collect();
                    machine
                }
                SanitizedEvidenceAssertion::FinalUpgradedRunning => {
                    installed_machine(final_artifacts(plan), ServiceExpectation::Running, true)
                }
                SanitizedEvidenceAssertion::FinalStopped => installed_machine(
                    final_artifacts(plan),
                    ServiceExpectation::Stopped {
                        win32_exit_code: 0,
                        service_specific_exit_code: 0,
                    },
                    true,
                ),
                SanitizedEvidenceAssertion::FinalCrashed => {
                    installed_machine(final_artifacts(plan), ServiceExpectation::Crashed, true)
                }
                SanitizedEvidenceAssertion::FinalRecovered => {
                    installed_machine(final_artifacts(plan), ServiceExpectation::Running, true)
                }
                SanitizedEvidenceAssertion::FinalMissingService => {
                    desktop_only_machine(final_artifacts(plan), true)
                }
                SanitizedEvidenceAssertion::FinalIncompatibleRunning => installed_machine(
                    incompatible_artifacts(plan),
                    ServiceExpectation::Running,
                    true,
                ),
                SanitizedEvidenceAssertion::FinalUninstalledPreservingCurrentUserData => {
                    absent_machine(true)
                }
                SanitizedEvidenceAssertion::DesktopPhase => unreachable!("handled above"),
            }
        };
        if let Some((process_id, started_at_100ns, generation)) =
            test_generation_for_receipt(receipt_name)
        {
            apply_test_generation(&mut machine, process_id, started_at_100ns, generation);
        }
        if matches!(
            receipt_name,
            "final-repair-state.private.json" | "final-primary-desktop.private.json"
        ) {
            machine
                .etw_session
                .as_mut()
                .expect("pre-uninstall ETW")
                .lease
                .install_id = [2; 16];
        }
        machine
    }

    fn test_generation_for_receipt(receipt_name: &str) -> Option<(u32, u64, u64)> {
        match receipt_name {
            "final-repair-state.private.json" | "final-primary-desktop.private.json" => {
                Some((55, 5_500, 7))
            }
            "baseline-install-state.private.json"
            | "baseline-primary-desktop.private.json"
            | "baseline-second-instance-desktop.private.json" => Some((65, 6_500, 8)),
            "baseline-restart-state.private.json" => Some((66, 6_600, 9)),
            "baseline-crash-recovery-state.private.json" => Some((67, 6_700, 10)),
            "baseline-rollback-recovery-state.private.json"
            | "legacy-residue-seeded-state.private.json" => Some((68, 6_800, 11)),
            "final-upgrade-state.private.json" => Some((69, 6_900, 12)),
            "final-restart-state.private.json" => Some((70, 7_000, 13)),
            "final-crash-recovery-state.private.json" => Some((71, 7_100, 14)),
            "final-missing-service-restored-state.private.json" => Some((72, 7_200, 15)),
            "final-stopped-service-restored-state.private.json" => Some((73, 7_300, 16)),
            "final-incompatible-service-state.private.json"
            | "final-incompatible-service-desktop.private.json" => Some((74, 7_400, 17)),
            "final-incompatible-service-restored-state.private.json" => Some((75, 7_500, 18)),
            _ => None,
        }
    }

    fn apply_test_generation(
        machine: &mut SanitizedMachineSnapshot,
        process_id: u32,
        started_at_100ns: u64,
        generation: u64,
    ) {
        let Observation::Present(service) = &mut machine.service else {
            panic!("running service");
        };
        service.process_id = process_id;
        service.process_started_at_100ns = Some(started_at_100ns);
        machine
            .named_pipe
            .as_mut()
            .expect("named pipe")
            .server_process_id = process_id;
        let etw = machine.etw_session.as_mut().expect("etw");
        etw.lease.controller.process_id = process_id;
        etw.lease.controller.process_started_at = started_at_100ns;
        etw.lease.service_instance_id =
            digest16(test_service_instance_id(process_id, generation).as_bytes());
    }

    fn installed_machine(
        artifacts: InstalledArtifactExpectation<'_>,
        expectation: ServiceExpectation,
        preserve_unknown_sentinel: bool,
    ) -> SanitizedMachineSnapshot {
        let (service_snapshot, named_pipe, etw_session, lease_paths) = match expectation {
            ServiceExpectation::Running => (
                service(4, 55, 0, 0, artifacts.service_sha256),
                Some(SanitizedNamedPipeSnapshot {
                    server_process_id: 55,
                }),
                Some(healthy_etw(
                    artifacts.service_sha256,
                    &test_service_instance_id(55, 7),
                )),
                true,
            ),
            ServiceExpectation::Stopped {
                win32_exit_code,
                service_specific_exit_code,
            } => (
                service(
                    1,
                    0,
                    win32_exit_code,
                    service_specific_exit_code,
                    artifacts.service_sha256,
                ),
                None,
                None,
                false,
            ),
            ServiceExpectation::Crashed => (
                service(1, 0, 1066, 1, artifacts.service_sha256),
                None,
                None,
                false,
            ),
        };
        SanitizedMachineSnapshot {
            service: Observation::Present(service_snapshot),
            install_root: Observation::Present(directory(LogicalRoot::Install, "")),
            monitor: Observation::Present(file(artifacts.monitor_sha256)),
            service_binary: Observation::Present(file_with_size(
                artifacts.service_sha256,
                artifacts.service_size.unwrap_or(8192),
            )),
            uninstaller: Observation::Present(file_with_size(
                artifacts.uninstaller_sha256,
                artifacts.uninstaller_size.unwrap_or(1),
            )),
            legacy_cli: Observation::Absent,
            uninstall_registry: Observation::Present(SanitizedRegistrySnapshot {
                view: "64".to_string(),
                key: logical_path(
                    LogicalRoot::Hklm,
                    "software/microsoft/windows/currentversion/uninstall/batcave-monitor",
                ),
                install_location: logical_path(LogicalRoot::Install, ""),
            }),
            product_processes: Observation::Present(Vec::new()),
            product_data_root: Observation::Present(directory(LogicalRoot::ProductData, "")),
            service_data_root: Observation::Present(directory(LogicalRoot::ServiceData, "")),
            current_user_data_root: Observation::Present(directory(
                LogicalRoot::CurrentUserData,
                "",
            )),
            installed_boundaries: Observation::Present(SanitizedBoundarySnapshot {
                service_dacl_sha256: "d".repeat(64),
                service_aces: vec![
                    ace(SanitizedPrincipal::LocalSystem, 0x000f_01ff, false),
                    ace(SanitizedPrincipal::Administrators, 0x000f_01ff, false),
                    ace(SanitizedPrincipal::InteractiveUsers, 0x0000_0004, false),
                ],
                service_data_root_owner: SanitizedPrincipal::LocalSystem,
                service_data_root_dacl_protected: true,
                service_data_root_reparse: false,
                service_data_root_dacl_sha256: "e".repeat(64),
                service_data_root_aces: vec![
                    ace(SanitizedPrincipal::LocalSystem, 0x001f_01ff, true),
                    ace(SanitizedPrincipal::Administrators, 0x001f_01ff, true),
                    ace(SanitizedPrincipal::CollectorService, 0x0013_01bf, true),
                ],
            }),
            service_registry_key: Some(logical_path(
                LogicalRoot::Hklm,
                "system/currentcontrolset/services/batcavecollector",
            )),
            named_pipe,
            etw_session,
            etw_lease_file: lease_paths
                .then(|| logical_path(LogicalRoot::ServiceData, "etw-lease.v1.json")),
            etw_owner_lock: lease_paths
                .then(|| logical_path(LogicalRoot::ServiceData, "etw-owner.v1.lock")),
            service_lifecycle_lock: lease_paths
                .then(|| logical_path(LogicalRoot::ServiceData, "process-owner.v1.lock")),
            upgrade_transaction_journal: None,
            staged_service_images: Vec::new(),
            rollback_service_images: Vec::new(),
            atomic_temporary_files: Vec::new(),
            failure_marker: None,
            machine_product_key: Some(logical_path(LogicalRoot::Hklm, "software/batcavemonitor")),
            hkcu_autostart: Some(SanitizedRegistryValueSnapshot {
                key: logical_path(
                    LogicalRoot::Hkcu,
                    "software/microsoft/windows/currentversion/run",
                ),
                value_name: "BatCave Monitor".to_string(),
                target: logical_path(LogicalRoot::Install, "batcave-monitor.exe"),
            }),
            public_desktop_shortcut: Some(shortcut(LogicalRoot::PublicDesktop)),
            common_start_menu_shortcut: Some(shortcut(LogicalRoot::CommonStartMenu)),
            known_retired_helper_artifacts: Vec::new(),
            unknown_helper_sentinel: preserve_unknown_sentinel.then(unknown_sentinel),
        }
    }

    fn desktop_only_machine(
        artifacts: InstalledArtifactExpectation<'_>,
        preserve_unknown_sentinel: bool,
    ) -> SanitizedMachineSnapshot {
        let mut machine = installed_machine(
            artifacts,
            ServiceExpectation::Stopped {
                win32_exit_code: 0,
                service_specific_exit_code: 0,
            },
            preserve_unknown_sentinel,
        );
        machine.service = Observation::Absent;
        machine.service_binary = Observation::Absent;
        machine.service_registry_key = None;
        machine.service_data_root = Observation::Absent;
        machine.installed_boundaries = Observation::Absent;
        machine
    }

    fn absent_machine(preserve_unknown_sentinel: bool) -> SanitizedMachineSnapshot {
        SanitizedMachineSnapshot {
            service: Observation::Absent,
            install_root: Observation::Absent,
            monitor: Observation::Absent,
            service_binary: Observation::Absent,
            uninstaller: Observation::Absent,
            legacy_cli: Observation::Absent,
            uninstall_registry: Observation::Absent,
            product_processes: Observation::Absent,
            product_data_root: Observation::Absent,
            service_data_root: Observation::Absent,
            current_user_data_root: Observation::Present(directory(
                LogicalRoot::CurrentUserData,
                "",
            )),
            installed_boundaries: Observation::Absent,
            service_registry_key: None,
            named_pipe: None,
            etw_session: None,
            etw_lease_file: None,
            etw_owner_lock: None,
            service_lifecycle_lock: None,
            upgrade_transaction_journal: None,
            staged_service_images: Vec::new(),
            rollback_service_images: Vec::new(),
            atomic_temporary_files: Vec::new(),
            failure_marker: None,
            machine_product_key: None,
            hkcu_autostart: None,
            public_desktop_shortcut: None,
            common_start_menu_shortcut: None,
            known_retired_helper_artifacts: Vec::new(),
            unknown_helper_sentinel: preserve_unknown_sentinel.then(unknown_sentinel),
        }
    }

    fn service(
        state: u32,
        process_id: u32,
        win32_exit_code: u32,
        service_specific_exit_code: u32,
        image_sha256: &str,
    ) -> SanitizedServiceSnapshot {
        SanitizedServiceSnapshot {
            state,
            process_id,
            process_started_at_100ns: (process_id != 0).then_some(5_500),
            win32_exit_code,
            service_specific_exit_code,
            image_path: logical_path(LogicalRoot::Install, "batcave-collector-service.exe"),
            image_sha256: image_sha256.to_string(),
            local_system: true,
            own_process: true,
            automatic_start: true,
            recovery_restart_action_count: 3,
            owner_marker: "dev.batcave.monitor/service-v1".to_string(),
            service_dacl_sha256: "d".repeat(64),
        }
    }

    fn healthy_etw(service_sha256: &str, service_instance_id: &str) -> SanitizedEtwObservation {
        let session = NetworkAttributionMonitor::session_identity();
        SanitizedEtwObservation {
            lease: EtwLeaseV1 {
                schema_version: ETW_LEASE_SCHEMA_VERSION,
                phase: EtwLeasePhase::Active,
                install_id: [1; 16],
                service_generation: sha256_digest16(service_sha256).expect("service generation"),
                service_instance_id: digest16(service_instance_id.as_bytes()),
                boot_identity: [3; 16],
                controller: crate::collector_service::etw_lease::EtwControllerIdentityV1 {
                    process_id: 55,
                    process_started_at: 5_500,
                },
                session: session.clone(),
            },
            observed_session: session,
            owner_lock_held: true,
            process_lock_held: true,
            events_lost: 0,
            buffers_lost: 0,
        }
    }

    fn test_service_instance_id(process_id: u32, generation: u64) -> String {
        format!("{process_id:08x}-{generation:032x}")
    }

    fn file(sha256: &str) -> SanitizedFileSnapshot {
        file_with_size(sha256, 1)
    }

    fn file_with_size(sha256: &str, size: u64) -> SanitizedFileSnapshot {
        SanitizedFileSnapshot {
            size,
            sha256: sha256.to_string(),
            volume_serial: 1,
            file_index: 1,
        }
    }

    fn path_file(
        root: LogicalRoot,
        relative_leaf: &str,
        hash_digit: &str,
    ) -> SanitizedPathFileSnapshot {
        SanitizedPathFileSnapshot {
            path: logical_path(root, relative_leaf),
            file: file(&hash_digit.repeat(64)),
        }
    }

    fn known_helper_path_file(relative_leaf: &str) -> SanitizedPathFileSnapshot {
        let bytes = known_helper_fixture_bytes(relative_leaf);
        SanitizedPathFileSnapshot {
            path: logical_path(LogicalRoot::CurrentUserData, relative_leaf),
            file: file_with_size(&sha256_hex(&bytes), bytes.len() as u64),
        }
    }

    fn unknown_sentinel() -> SanitizedPathFileSnapshot {
        let bytes = b"batcave_windows_lifecycle_unknown_helper_sentinel_v1\n";
        SanitizedPathFileSnapshot {
            path: logical_path(LogicalRoot::CurrentUserData, UNKNOWN_HELPER_SENTINEL_LEAF),
            file: file_with_size(&sha256_hex(bytes), bytes.len() as u64),
        }
    }

    fn shortcut(root: LogicalRoot) -> SanitizedShortcutSnapshot {
        SanitizedShortcutSnapshot {
            path: logical_path(root, "BatCave Monitor.lnk"),
            target: logical_path(LogicalRoot::Install, "batcave-monitor.exe"),
            sha256: "6".repeat(64),
        }
    }

    fn retention(receipts: &[EvidenceReceipt]) -> SanitizedCurrentUserRetention {
        SanitizedCurrentUserRetention {
            before_uninstall_source: receipts
                .iter()
                .find(|receipt| {
                    receipt.name == "final-incompatible-service-restored-state.private.json"
                })
                .expect("before uninstall source")
                .clone(),
            after_uninstall_source: receipts
                .iter()
                .find(|receipt| receipt.name == "final-uninstall-state.private.json")
                .expect("after uninstall source")
                .clone(),
            settings: retained(RETAINED_SETTINGS_LEAF, "a"),
            cache: retained(RETAINED_CACHE_LEAF, "b"),
            diagnostics: retained(RETAINED_DIAGNOSTICS_LEAF, "c"),
        }
    }

    fn retained(relative_leaf: &str, hash_digit: &str) -> SanitizedRetainedUserObject {
        let digest = Observation::Present(SanitizedDigestSnapshot {
            size: 1,
            sha256: hash_digit.repeat(64),
        });
        SanitizedRetainedUserObject {
            path: logical_path(LogicalRoot::CurrentUserData, relative_leaf),
            before_uninstall: digest.clone(),
            after_uninstall: digest,
        }
    }

    fn directory(root: LogicalRoot, relative_leaf: &str) -> SanitizedDirectorySnapshot {
        SanitizedDirectorySnapshot {
            volume_serial: 1,
            file_index: 1,
            final_path: logical_path(root, relative_leaf),
        }
    }

    fn logical_path(root: LogicalRoot, relative_leaf: &str) -> LogicalPath {
        LogicalPath {
            root,
            relative_leaf: relative_leaf.to_string(),
        }
    }
}
