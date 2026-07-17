#[cfg(test)]
use super::native::NamedPipeSnapshot;
use super::native::{
    DirectorySnapshot, ElevatedMachineSnapshot, FileSnapshot, ParentCurrentUserAuthority,
    ParentCurrentUserCapturePoint, ParentCurrentUserObjects, ParentCurrentUserResidueTimeline,
    ParentHelperFileSnapshot, ProcessSnapshot, RegistrySnapshot, RegistryView, ServiceSnapshot,
    VerifiedEvidenceFile,
};
use super::private_evidence::{
    parse_private_success_packet, PrivateSuccessPacket, PrivateSuccessPayload,
};
use crate::collector_service::etw_lease::{
    EtwLeasePhase, EtwLeaseV1, EtwSessionIdentityV1, ETW_LEASE_SCHEMA_VERSION,
};
use crate::collector_service::windows_provisioner::{
    AcePolicyForProof, InstallResidueForProof, InstalledBoundariesForProof,
    MachineRegistrationForProof, ProductRegistrationKeyForProof, ResidueFileForProof,
    ResidueKindForProof, RuntimeLockObservation, SecurityPrincipalForProof,
    ServiceDataResidueForProof, ServiceInstallResidueForProof, ServiceRegistryKeyForProof,
    ShortcutForProof, TerminatedServiceForProof, SERVICE_LIFECYCLE_LOCK_FILE_NAME,
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
const PRIVATE_EVIDENCE_PROJECTION_READY: bool = true;
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
    declared_current_user_objects_preserved: bool,
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
    FinalUninstalledPreservingDeclaredCurrentUserObjects,
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

fn absolute_path_eq(left: &str, right: &str) -> bool {
    match (
        normalize_absolute_windows_path(left),
        normalize_absolute_windows_path(right),
    ) {
        (Ok(left), Ok(right)) => left.eq_ignore_ascii_case(&right),
        _ => false,
    }
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
        || !packet.declared_current_user_objects_preserved
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

#[derive(Clone, Copy)]
pub(super) struct ParentCurrentUserProjection<'a> {
    pub(super) authority: &'a ParentCurrentUserAuthority,
    pub(super) before_uninstall: &'a ParentCurrentUserObjects,
    pub(super) after_uninstall: &'a ParentCurrentUserObjects,
    pub(super) residue_timeline: &'a ParentCurrentUserResidueTimeline,
}

pub(super) struct PreparedSanitizedExport {
    bytes: Vec<u8>,
    receipt: EvidenceReceipt,
}

impl PreparedSanitizedExport {
    pub(super) fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    pub(super) fn receipt(&self) -> &EvidenceReceipt {
        &self.receipt
    }

    #[cfg(test)]
    pub(super) fn from_bytes_for_test(bytes: Vec<u8>) -> Self {
        Self {
            receipt: EvidenceReceipt {
                name: "windows-lifecycle-proof.sanitized.json".to_string(),
                size: bytes.len() as u64,
                sha256: sha256_hex(&bytes),
            },
            bytes,
        }
    }
}

pub(super) fn derive_sanitized_export(
    private_files: &[VerifiedEvidenceFile],
    plan: &ProofPlan,
    controller_source_commit_sha: &str,
    controller_sha256: &str,
    parent_desktop_results: &[DesktopPhaseResult],
    parent_current_user: ParentCurrentUserProjection<'_>,
) -> Result<PreparedSanitizedExport, String> {
    validate_verified_projection_manifest(private_files)?;
    let mut private_packets = Vec::with_capacity(SUCCESS_PRIVATE_EVIDENCE_LEAVES.len());
    for file in private_files {
        let bytes = file.read_all_exact("private_projection")?;
        let packet = parse_private_success_packet(&file.receipt().name, &bytes)?;
        private_packets.push((file.receipt().clone(), packet));
    }

    let prepared = prepare_sanitized_export(
        &private_packets,
        plan,
        controller_source_commit_sha,
        controller_sha256,
        parent_desktop_results,
        parent_current_user,
    )?;
    for file in private_files {
        file.revalidate()?;
    }
    Ok(prepared)
}

fn prepare_sanitized_export(
    private_packets: &[(EvidenceReceipt, PrivateSuccessPacket)],
    plan: &ProofPlan,
    controller_source_commit_sha: &str,
    controller_sha256: &str,
    parent_desktop_results: &[DesktopPhaseResult],
    parent_current_user: ParentCurrentUserProjection<'_>,
) -> Result<PreparedSanitizedExport, String> {
    let packet = derive_sanitized_packet(
        private_packets,
        plan,
        controller_source_commit_sha,
        controller_sha256,
        parent_desktop_results,
        parent_current_user,
    )?;
    let comparison_packets = private_packets
        .iter()
        .map(|(receipt, packet)| (receipt, packet.clone()))
        .collect::<Vec<_>>();
    compare_verified_projection(
        &packet,
        &comparison_packets,
        plan,
        parent_desktop_results,
        parent_current_user,
    )?;
    let sanitized_bytes = serde_json::to_vec_pretty(&packet)
        .map_err(|_| "lifecycle_sanitized_export_serialize_failed".to_string())?;
    reject_private_path_leakage(&sanitized_bytes)?;
    let expected_receipts = private_packets
        .iter()
        .map(|(receipt, _)| receipt.clone())
        .collect::<Vec<_>>();
    validate_sanitized_export_bytes_with_parent_results(
        &sanitized_bytes,
        plan,
        controller_source_commit_sha,
        controller_sha256,
        &expected_receipts,
        parent_desktop_results,
    )?;
    let size = u64::try_from(sanitized_bytes.len())
        .map_err(|_| "lifecycle_sanitized_export_size_invalid".to_string())?;
    let receipt = EvidenceReceipt {
        name: "windows-lifecycle-proof.sanitized.json".to_string(),
        size,
        sha256: sha256_hex(&sanitized_bytes),
    };
    Ok(PreparedSanitizedExport {
        bytes: sanitized_bytes,
        receipt,
    })
}

fn validate_verified_projection_manifest(
    private_files: &[VerifiedEvidenceFile],
) -> Result<(), String> {
    for file in private_files {
        file.revalidate()?;
    }
    let private = private_files
        .iter()
        .map(|file| (file.receipt(), file.identity()))
        .collect::<Vec<_>>();
    validate_projection_manifest_parts(&private)
}

fn validate_projection_manifest_parts(
    private: &[(&EvidenceReceipt, super::native::FileIdentity)],
) -> Result<(), String> {
    if private.len() != SUCCESS_PRIVATE_EVIDENCE_LEAVES.len()
        || private
            .iter()
            .map(|(receipt, _)| receipt.name.as_str())
            .ne(SUCCESS_PRIVATE_EVIDENCE_LEAVES)
    {
        return Err("lifecycle_private_projection_manifest_invalid".to_string());
    }
    let mut names = BTreeSet::new();
    let mut identities = Vec::with_capacity(private.len());
    for (receipt, identity) in private {
        if receipt.size == 0
            || receipt.size > MAX_EVIDENCE_SIZE
            || validate_sha256(&receipt.sha256, "private_projection").is_err()
            || !names.insert(receipt.name.as_str())
            || identities.contains(identity)
        {
            return Err("lifecycle_private_projection_manifest_invalid".to_string());
        }
        identities.push(*identity);
    }
    if names
        != SUCCESS_PRIVATE_EVIDENCE_LEAVES
            .into_iter()
            .collect::<BTreeSet<_>>()
    {
        return Err("lifecycle_private_projection_manifest_invalid".to_string());
    }
    Ok(())
}

fn reject_private_path_leakage(bytes: &[u8]) -> Result<(), String> {
    let lower = bytes.iter().map(u8::to_ascii_lowercase).collect::<Vec<_>>();
    let backslash_drive_absolute = lower
        .windows(4)
        .any(|window| window[0].is_ascii_alphabetic() && window[1..] == *b":\\\\");
    let slash_drive_absolute = lower
        .windows(3)
        .any(|window| window[0].is_ascii_alphabetic() && window[1..] == *b":/");
    if backslash_drive_absolute
        || slash_drive_absolute
        || lower.windows(4).any(|window| window == b"\\\\\\\\")
        || lower
            .windows(b"batcavelifecycleproof-v1-".len())
            .any(|window| window == b"batcavelifecycleproof-v1-")
    {
        return Err("lifecycle_private_projection_path_leak".to_string());
    }
    Ok(())
}

fn derive_sanitized_packet(
    private_packets: &[(EvidenceReceipt, PrivateSuccessPacket)],
    plan: &ProofPlan,
    controller_source_commit_sha: &str,
    controller_sha256: &str,
    parent_desktop_results: &[DesktopPhaseResult],
    parent_current_user: ParentCurrentUserProjection<'_>,
) -> Result<SanitizedExportPacket, String> {
    if private_packets.len() != SUCCESS_PRIVATE_EVIDENCE_LEAVES.len()
        || private_packets
            .iter()
            .map(|(receipt, _)| receipt.name.as_str())
            .ne(SUCCESS_PRIVATE_EVIDENCE_LEAVES)
    {
        return Err("lifecycle_private_projection_manifest_invalid".to_string());
    }

    let private_evidence = private_packets
        .iter()
        .map(|(receipt, packet)| {
            derive_private_evidence(
                receipt,
                packet,
                plan,
                parent_desktop_results,
                parent_current_user,
            )
        })
        .collect::<Result<Vec<_>, _>>()?;
    let receipt_by_name = private_packets
        .iter()
        .map(|(receipt, _)| (receipt.name.as_str(), receipt))
        .collect::<BTreeMap<_, _>>();
    let current_user_retention = project_parent_current_user_retention(
        receipt_by_name
            .get("final-incompatible-service-restored-state.private.json")
            .copied()
            .ok_or_else(|| "lifecycle_private_projection_manifest_invalid".to_string())?,
        receipt_by_name
            .get("final-uninstall-state.private.json")
            .copied()
            .ok_or_else(|| "lifecycle_private_projection_manifest_invalid".to_string())?,
        parent_current_user.before_uninstall,
        parent_current_user.after_uninstall,
    )?;

    Ok(SanitizedExportPacket {
        schema_version: SANITIZED_SCHEMA.to_string(),
        profile: plan.profile.clone(),
        plan_sha256: plan_sha256(),
        controller_source_commit_sha: controller_source_commit_sha.to_string(),
        controller_sha256: controller_sha256.to_string(),
        completed_stage: LifecycleStage::FinalUninstall,
        process_tree_settled: true,
        private_evidence,
        final_product_absent: true,
        declared_current_user_objects_preserved: true,
        current_user_retention,
    })
}

fn derive_private_evidence(
    receipt: &EvidenceReceipt,
    packet: &PrivateSuccessPacket,
    plan: &ProofPlan,
    parent_desktop_results: &[DesktopPhaseResult],
    parent_current_user: ParentCurrentUserProjection<'_>,
) -> Result<SanitizedPrivateEvidence, String> {
    let name = receipt.name.as_str();
    let expected_desktop_phase = desktop_phase_for_leaf(name);
    let expected_event = match name {
        "baseline-crashed-state.private.json" | "final-crashed-state.private.json" => 1,
        "baseline-rollback-recovery-state.private.json" => 2,
        _ => 0,
    };
    let (raw_machine, desktop_phase, event) = match packet.payload() {
        PrivateSuccessPayload::Machine(machine)
            if expected_desktop_phase.is_none() && expected_event == 0 =>
        {
            (machine, None, None)
        }
        PrivateSuccessPayload::Desktop(desktop) => {
            let Some(expected_phase) = expected_desktop_phase else {
                return Err("lifecycle_private_projection_payload_mismatch".to_string());
            };
            let parent = parent_desktop_results
                .iter()
                .find(|result| result.phase == expected_phase)
                .ok_or_else(|| "lifecycle_private_projection_parent_desktop_missing".to_string())?;
            if desktop.result != *parent {
                return Err("lifecycle_private_projection_parent_desktop_drift".to_string());
            }
            validate_desktop_phase_result(parent, plan)?;
            (
                &desktop.machine,
                Some(sanitize_parent_desktop_result(parent)?),
                None,
            )
        }
        PrivateSuccessPayload::ServiceCrash(crash) if expected_event == 1 => (
            &crash.machine,
            None,
            Some(SanitizedStageEvent::ServiceCrash(project_service_crash(
                &crash.termination,
            )?)),
        ),
        PrivateSuccessPayload::UpgradeRollback(rollback) if expected_event == 2 => (
            &rollback.machine,
            None,
            Some(SanitizedStageEvent::UpgradeRollback(
                SanitizedUpgradeRollbackEvent {
                    candidate_sha256: rollback.rollback.candidate_sha256.clone(),
                    candidate_failure_code: rollback.rollback.candidate_failure_code.clone(),
                    candidate_failure_detail: rollback.rollback.candidate_failure_detail.clone(),
                    execution_marker_sha256: rollback.rollback.execution_marker_sha256.clone(),
                    restored_sha256: rollback.rollback.restored_sha256.clone(),
                    restored_process_id: rollback.rollback.restored_process_id,
                },
            )),
        ),
        _ => return Err("lifecycle_private_projection_payload_mismatch".to_string()),
    };
    let machine = project_machine_snapshot(
        raw_machine,
        parent_current_user.authority,
        name,
        parent_current_user.residue_timeline,
    )?;
    Ok(SanitizedPrivateEvidence {
        receipt: receipt.clone(),
        assertion: assertion_for_leaf(name),
        machine,
        desktop_phase,
        event,
    })
}

fn project_parent_current_user_retention(
    before_uninstall_source: &EvidenceReceipt,
    after_uninstall_source: &EvidenceReceipt,
    before: &ParentCurrentUserObjects,
    after: &ParentCurrentUserObjects,
) -> Result<SanitizedCurrentUserRetention, String> {
    let retained = |relative_leaf: &str,
                    before: &Observation<FileSnapshot>,
                    after: &Observation<FileSnapshot>|
     -> Result<SanitizedRetainedUserObject, String> {
        Ok(SanitizedRetainedUserObject {
            path: LogicalPath {
                root: LogicalRoot::CurrentUserData,
                relative_leaf: relative_leaf.to_string(),
            },
            before_uninstall: project_parent_user_digest(before)?,
            after_uninstall: project_parent_user_digest(after)?,
        })
    };
    Ok(SanitizedCurrentUserRetention {
        before_uninstall_source: before_uninstall_source.clone(),
        after_uninstall_source: after_uninstall_source.clone(),
        settings: retained(RETAINED_SETTINGS_LEAF, &before.settings, &after.settings)?,
        cache: retained(RETAINED_CACHE_LEAF, &before.cache, &after.cache)?,
        diagnostics: retained(
            RETAINED_DIAGNOSTICS_LEAF,
            &before.diagnostics,
            &after.diagnostics,
        )?,
    })
}

fn compare_verified_projection(
    export: &SanitizedExportPacket,
    private_packets: &[(&EvidenceReceipt, PrivateSuccessPacket)],
    plan: &ProofPlan,
    parent_desktop_results: &[DesktopPhaseResult],
    parent_current_user: ParentCurrentUserProjection<'_>,
) -> Result<(), String> {
    let by_name = export
        .private_evidence
        .iter()
        .map(|entry| (entry.receipt.name.as_str(), entry))
        .collect::<BTreeMap<_, _>>();
    for (receipt, packet) in private_packets {
        let entry = by_name
            .get(receipt.name.as_str())
            .copied()
            .ok_or_else(|| "lifecycle_private_projection_manifest_invalid".to_string())?;
        if entry.receipt != **receipt {
            return Err("lifecycle_private_projection_receipt_mismatch".to_string());
        }
        let machine = match packet.payload() {
            PrivateSuccessPayload::Machine(machine) => {
                if entry.desktop_phase.is_some() || entry.event.is_some() {
                    return Err("lifecycle_private_projection_payload_mismatch".to_string());
                }
                machine
            }
            PrivateSuccessPayload::Desktop(desktop) => {
                let parent = parent_desktop_results
                    .iter()
                    .find(|result| result.phase == desktop.result.phase)
                    .ok_or_else(|| {
                        "lifecycle_private_projection_parent_desktop_missing".to_string()
                    })?;
                if desktop.result != *parent {
                    return Err("lifecycle_private_projection_parent_desktop_drift".to_string());
                }
                let projected = sanitize_parent_desktop_result(&desktop.result)?;
                if entry.desktop_phase.as_ref() != Some(&projected) || entry.event.is_some() {
                    return Err("lifecycle_private_projection_desktop_drift".to_string());
                }
                &desktop.machine
            }
            PrivateSuccessPayload::ServiceCrash(crash) => {
                let projected =
                    SanitizedStageEvent::ServiceCrash(project_service_crash(&crash.termination)?);
                if entry.event.as_ref() != Some(&projected) || entry.desktop_phase.is_some() {
                    return Err("lifecycle_private_projection_event_drift".to_string());
                }
                &crash.machine
            }
            PrivateSuccessPayload::UpgradeRollback(rollback) => {
                let projected =
                    SanitizedStageEvent::UpgradeRollback(SanitizedUpgradeRollbackEvent {
                        candidate_sha256: rollback.rollback.candidate_sha256.clone(),
                        candidate_failure_code: rollback.rollback.candidate_failure_code.clone(),
                        candidate_failure_detail: rollback
                            .rollback
                            .candidate_failure_detail
                            .clone(),
                        execution_marker_sha256: rollback.rollback.execution_marker_sha256.clone(),
                        restored_sha256: rollback.rollback.restored_sha256.clone(),
                        restored_process_id: rollback.rollback.restored_process_id,
                    });
                if entry.event.as_ref() != Some(&projected) || entry.desktop_phase.is_some() {
                    return Err("lifecycle_private_projection_event_drift".to_string());
                }
                &rollback.machine
            }
        };
        compare_machine_projection(machine, &entry.machine, plan, parent_current_user.authority)?;
        compare_parent_current_user_residue_projection(
            &receipt.name,
            &entry.machine,
            parent_current_user.residue_timeline,
            parent_current_user.authority,
        )?;
    }
    compare_parent_current_user_retention(
        &export.current_user_retention,
        parent_current_user.before_uninstall,
        parent_current_user.after_uninstall,
    )?;
    Ok(())
}

fn compare_machine_projection(
    raw: &ElevatedMachineSnapshot,
    sanitized: &SanitizedMachineSnapshot,
    _plan: &ProofPlan,
    parent_current_user: &ParentCurrentUserAuthority,
) -> Result<(), String> {
    let mut projected = project_machine_snapshot_without_parent_residue(raw, parent_current_user)?;
    projected.hkcu_autostart = sanitized.hkcu_autostart.clone();
    projected.known_retired_helper_artifacts = sanitized.known_retired_helper_artifacts.clone();
    projected.unknown_helper_sentinel = sanitized.unknown_helper_sentinel.clone();
    let mut supplied = sanitized.clone();
    canonicalize_residue_file_order(&mut projected);
    canonicalize_residue_file_order(&mut supplied);
    if projected == supplied {
        Ok(())
    } else {
        Err("lifecycle_private_projection_machine_drift".to_string())
    }
}

fn canonicalize_residue_file_order(machine: &mut SanitizedMachineSnapshot) {
    let sort = |files: &mut Vec<SanitizedPathFileSnapshot>| {
        files.sort_by(|left, right| {
            (&left.path.root, &left.path.relative_leaf)
                .cmp(&(&right.path.root, &right.path.relative_leaf))
        });
    };
    sort(&mut machine.staged_service_images);
    sort(&mut machine.rollback_service_images);
    sort(&mut machine.atomic_temporary_files);
}

fn project_machine_snapshot(
    raw: &ElevatedMachineSnapshot,
    parent_current_user: &ParentCurrentUserAuthority,
    receipt_name: &str,
    residue_timeline: &ParentCurrentUserResidueTimeline,
) -> Result<SanitizedMachineSnapshot, String> {
    let mut machine = project_machine_snapshot_without_parent_residue(raw, parent_current_user)?;
    let residue =
        project_parent_current_user_residue(receipt_name, residue_timeline, parent_current_user)?;
    machine.hkcu_autostart = residue.hkcu_autostart;
    machine.known_retired_helper_artifacts = residue.known_retired_helper_artifacts;
    machine.unknown_helper_sentinel = residue.unknown_helper_sentinel;
    Ok(machine)
}

fn project_machine_snapshot_without_parent_residue(
    raw: &ElevatedMachineSnapshot,
    parent_current_user: &ParentCurrentUserAuthority,
) -> Result<SanitizedMachineSnapshot, String> {
    let roots = projection_roots()?;
    let service = project_service_projection(
        &raw.machine.service,
        &raw.machine.service_binary,
        &raw.installed_boundaries,
    )?;
    let runtime = project_runtime_projection(raw)?;
    let residue = project_service_install_residue_projection(raw, &roots)?;
    let registration = project_machine_registration_projection(raw, &roots)?;
    Ok(SanitizedMachineSnapshot {
        service,
        install_root: project_directory_observation(
            &raw.machine.install_root,
            &roots,
            LogicalRoot::Install,
        )?,
        monitor: project_file_observation(&raw.machine.monitor)?,
        service_binary: project_file_observation(&raw.machine.service_binary)?,
        uninstaller: project_file_observation(&raw.machine.uninstaller)?,
        legacy_cli: project_file_observation(&raw.machine.legacy_cli)?,
        uninstall_registry: project_registry_observation(&raw.machine.uninstall_registry, &roots)?,
        product_processes: project_process_observation(&raw.machine.product_processes, &roots)?,
        product_data_root: project_directory_observation(
            &raw.product_data_root,
            &roots,
            LogicalRoot::ProductData,
        )?,
        service_data_root: project_directory_observation(
            &raw.service_data_root,
            &roots,
            LogicalRoot::ServiceData,
        )?,
        current_user_data_root: project_parent_current_user_directory(parent_current_user)?,
        installed_boundaries: project_boundaries_observation(&raw.installed_boundaries)?,
        service_registry_key: residue.service_registry_key,
        named_pipe: runtime.named_pipe,
        etw_session: runtime.etw_session,
        etw_lease_file: runtime.etw_lease_file,
        etw_owner_lock: runtime.etw_owner_lock,
        service_lifecycle_lock: runtime.service_lifecycle_lock,
        upgrade_transaction_journal: residue.upgrade_transaction_journal,
        staged_service_images: residue.staged_service_images,
        rollback_service_images: residue.rollback_service_images,
        atomic_temporary_files: residue.atomic_temporary_files,
        failure_marker: residue.failure_marker,
        machine_product_key: registration.machine_product_key,
        hkcu_autostart: None,
        public_desktop_shortcut: registration.public_desktop_shortcut,
        common_start_menu_shortcut: registration.common_start_menu_shortcut,
        known_retired_helper_artifacts: Vec::new(),
        unknown_helper_sentinel: None,
    })
}

fn parent_residue_capture_point(
    receipt_name: &str,
) -> Result<ParentCurrentUserCapturePoint, String> {
    let point = match receipt_name {
        "initial-state.private.json" => {
            ParentCurrentUserCapturePoint::Checkpoint(LifecycleStage::InitialState)
        }
        "final-repair-state.private.json" => {
            ParentCurrentUserCapturePoint::Checkpoint(LifecycleStage::FinalRepair)
        }
        "final-primary-desktop.private.json" => {
            ParentCurrentUserCapturePoint::DesktopComplete(DesktopPhase::FinalPrimary)
        }
        "initial-uninstall-state.private.json" => {
            ParentCurrentUserCapturePoint::Checkpoint(LifecycleStage::InitialUninstall)
        }
        "baseline-install-state.private.json" => {
            ParentCurrentUserCapturePoint::Checkpoint(LifecycleStage::BaselineInstall)
        }
        "baseline-primary-desktop.private.json" => {
            ParentCurrentUserCapturePoint::DesktopComplete(DesktopPhase::BaselinePrimary)
        }
        "baseline-second-instance-desktop.private.json" => {
            ParentCurrentUserCapturePoint::DesktopComplete(DesktopPhase::BaselineSecondInstance)
        }
        "baseline-restart-stopped-state.private.json" | "baseline-restart-state.private.json" => {
            ParentCurrentUserCapturePoint::Checkpoint(LifecycleStage::BaselineRestart)
        }
        "baseline-crashed-state.private.json" | "baseline-crash-recovery-state.private.json" => {
            ParentCurrentUserCapturePoint::Checkpoint(LifecycleStage::BaselineCrashRecovery)
        }
        "baseline-rollback-recovery-state.private.json" => {
            ParentCurrentUserCapturePoint::Checkpoint(LifecycleStage::BaselineRollbackRecovery)
        }
        "legacy-residue-seeded-state.private.json" => {
            ParentCurrentUserCapturePoint::Checkpoint(LifecycleStage::LegacyResidueSeeded)
        }
        "final-upgrade-state.private.json" => {
            ParentCurrentUserCapturePoint::Checkpoint(LifecycleStage::FinalUpgrade)
        }
        "final-restart-stopped-state.private.json" | "final-restart-state.private.json" => {
            ParentCurrentUserCapturePoint::Checkpoint(LifecycleStage::FinalRestart)
        }
        "final-crashed-state.private.json" | "final-crash-recovery-state.private.json" => {
            ParentCurrentUserCapturePoint::Checkpoint(LifecycleStage::FinalCrashRecovery)
        }
        "final-missing-service-state.private.json" => {
            ParentCurrentUserCapturePoint::FinalMissingServiceBeforeDesktop
        }
        "final-missing-service-desktop.private.json"
        | "final-missing-service-restored-state.private.json" => {
            ParentCurrentUserCapturePoint::DesktopComplete(DesktopPhase::FinalMissingService)
        }
        "final-stopped-service-state.private.json"
        | "final-stopped-service-desktop.private.json"
        | "final-stopped-service-restored-state.private.json" => {
            ParentCurrentUserCapturePoint::DesktopComplete(DesktopPhase::FinalStoppedService)
        }
        "final-incompatible-service-state.private.json"
        | "final-incompatible-service-desktop.private.json" => {
            ParentCurrentUserCapturePoint::DesktopComplete(DesktopPhase::FinalIncompatibleService)
        }
        "final-incompatible-service-restored-state.private.json" => {
            ParentCurrentUserCapturePoint::Checkpoint(LifecycleStage::FinalFallbackStates)
        }
        "final-uninstall-state.private.json" => {
            ParentCurrentUserCapturePoint::Checkpoint(LifecycleStage::FinalUninstall)
        }
        _ => return Err("lifecycle_parent_user_residue_receipt_invalid".to_string()),
    };
    Ok(point)
}

fn compare_parent_current_user_residue_projection(
    receipt_name: &str,
    sanitized: &SanitizedMachineSnapshot,
    timeline: &ParentCurrentUserResidueTimeline,
    authority: &ParentCurrentUserAuthority,
) -> Result<(), String> {
    let projected = project_parent_current_user_residue(receipt_name, timeline, authority)?;
    if sanitized.hkcu_autostart != projected.hkcu_autostart {
        return Err("lifecycle_parent_user_run_projection_drift".to_string());
    }
    if sanitized.known_retired_helper_artifacts != projected.known_retired_helper_artifacts
        || sanitized.unknown_helper_sentinel != projected.unknown_helper_sentinel
    {
        return Err("lifecycle_parent_user_helper_projection_drift".to_string());
    }
    Ok(())
}

struct ProjectedParentCurrentUserResidue {
    hkcu_autostart: Option<SanitizedRegistryValueSnapshot>,
    known_retired_helper_artifacts: Vec<SanitizedPathFileSnapshot>,
    unknown_helper_sentinel: Option<SanitizedPathFileSnapshot>,
}

fn project_parent_current_user_residue(
    receipt_name: &str,
    timeline: &ParentCurrentUserResidueTimeline,
    authority: &ParentCurrentUserAuthority,
) -> Result<ProjectedParentCurrentUserResidue, String> {
    let raw = timeline.get(parent_residue_capture_point(receipt_name)?)?;
    if !super::native::valid_parent_run_key_owner(&raw.hkcu_run.owner_sid, &authority.user_sid) {
        return Err("lifecycle_parent_user_run_projection_owner_invalid".to_string());
    }
    let expected_autostart = match &raw.hkcu_run.batcave_monitor {
        Observation::Absent => None,
        Observation::Present(value)
            if value.value_type == 1 && value.value == super::native::exact_parent_run_value() =>
        {
            Some(SanitizedRegistryValueSnapshot {
                key: LogicalPath {
                    root: LogicalRoot::Hkcu,
                    relative_leaf: "software/microsoft/windows/currentversion/run".to_string(),
                },
                value_name: "BatCave Monitor".to_string(),
                target: LogicalPath {
                    root: LogicalRoot::Install,
                    relative_leaf: "batcave-monitor.exe".to_string(),
                },
            })
        }
        Observation::Present(_) => {
            return Err("lifecycle_parent_user_run_projection_invalid".to_string());
        }
        Observation::Unknown(_) => {
            return Err("lifecycle_parent_user_run_projection_unknown".to_string());
        }
    };
    if raw.hkcu_run.dacl_sha256.len() != 64 || raw.hkcu_run.manifest_sha256.len() != 64 {
        return Err("lifecycle_parent_user_run_projection_drift".to_string());
    }

    let (known, sentinel) = match &raw.helper {
        Observation::Absent => (BTreeMap::new(), None),
        Observation::Unknown(_) => {
            return Err("lifecycle_parent_user_helper_projection_unknown".to_string());
        }
        Observation::Present(helper) => {
            if helper.unexpected_entry_count != 0
                || helper.root_owner_sid != authority.user_sid
                || helper.root_dacl_sha256.len() != 64
                || helper.manifest_sha256.len() != 64
            {
                return Err("lifecycle_parent_user_helper_projection_invalid".to_string());
            }
            let mut known = BTreeMap::new();
            for file in &helper.known_files {
                let projected = project_parent_helper_file(file, &authority.user_sid)?;
                if known
                    .insert(projected.path.relative_leaf.clone(), projected)
                    .is_some()
                {
                    return Err("lifecycle_parent_user_helper_projection_invalid".to_string());
                }
            }
            let sentinel = match &helper.sentinel {
                Observation::Absent => None,
                Observation::Present(file) => {
                    Some(project_parent_helper_file(file, &authority.user_sid)?)
                }
                Observation::Unknown(_) => {
                    return Err("lifecycle_parent_user_helper_projection_unknown".to_string());
                }
            };
            (known, sentinel)
        }
    };
    let mut known = known;
    let known_retired_helper_artifacts = KNOWN_RETIRED_HELPER_LEAVES
        .iter()
        .filter_map(|leaf| known.remove(*leaf))
        .collect::<Vec<_>>();
    if !known.is_empty() {
        return Err("lifecycle_parent_user_helper_projection_invalid".to_string());
    }
    Ok(ProjectedParentCurrentUserResidue {
        hkcu_autostart: expected_autostart,
        known_retired_helper_artifacts,
        unknown_helper_sentinel: sentinel,
    })
}

fn project_parent_helper_file(
    raw: &ParentHelperFileSnapshot,
    parent_sid: &str,
) -> Result<SanitizedPathFileSnapshot, String> {
    if raw.relative_leaf.is_empty()
        || raw.owner_sid != parent_sid
        || raw.dacl_sha256.len() != 64
        || raw.file.size == 0
        || raw.file.identity.volume_serial == 0
        || raw.file.identity.file_index == 0
        || validate_sha256(&raw.file.sha256, "parent_helper_projection").is_err()
    {
        return Err("lifecycle_parent_user_helper_projection_invalid".to_string());
    }
    let path = LogicalPath {
        root: LogicalRoot::CurrentUserData,
        relative_leaf: raw.relative_leaf.clone(),
    };
    validate_logical_path(&path)?;
    Ok(SanitizedPathFileSnapshot {
        path,
        file: SanitizedFileSnapshot {
            size: raw.file.size,
            sha256: raw.file.sha256.clone(),
            volume_serial: raw.file.identity.volume_serial,
            file_index: raw.file.identity.file_index,
        },
    })
}

struct ProjectedMachineRegistration {
    machine_product_key: Option<LogicalPath>,
    public_desktop_shortcut: Option<SanitizedShortcutSnapshot>,
    common_start_menu_shortcut: Option<SanitizedShortcutSnapshot>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum RestorationAuthorityExpectation {
    AllowlistedStopped,
    BaselineRunning,
    FinalRunning,
    ProductAbsent,
}

fn project_machine_registration_projection(
    raw: &ElevatedMachineSnapshot,
    roots: &[SanitizationRoot],
) -> Result<ProjectedMachineRegistration, String> {
    validate_registration_identity_set(raw)?;
    let machine_product_key = project_product_registration(&raw.machine_registration)?;
    let public_shortcut = project_shortcut(
        &raw.machine_registration.public_desktop_shortcut,
        LogicalRoot::PublicDesktop,
        roots,
    )?;
    let common_shortcut = project_shortcut(
        &raw.machine_registration.common_start_menu_shortcut,
        LogicalRoot::CommonStartMenu,
        roots,
    )?;
    Ok(ProjectedMachineRegistration {
        machine_product_key,
        public_desktop_shortcut: public_shortcut,
        common_start_menu_shortcut: common_shortcut,
    })
}

fn project_product_registration(
    raw: &MachineRegistrationForProof,
) -> Result<Option<LogicalPath>, String> {
    match (&raw.product_key_64, &raw.product_key_32) {
        (Observation::Absent, Observation::Absent) => Ok(None),
        (Observation::Present(key), Observation::Absent) => {
            validate_product_registration_key(key)?;
            Ok(Some(LogicalPath {
                root: LogicalRoot::Hklm,
                relative_leaf: "software/batcave/batcave monitor".to_string(),
            }))
        }
        (Observation::Unknown(_), _) | (_, Observation::Unknown(_)) => {
            Err("lifecycle_private_projection_observation_unknown".to_string())
        }
        _ => Err("lifecycle_private_projection_product_registration_invalid".to_string()),
    }
}

fn validate_product_registration_key(key: &ProductRegistrationKeyForProof) -> Result<(), String> {
    if !key
        .final_key_path
        .eq_ignore_ascii_case(r"\REGISTRY\MACHINE\SOFTWARE\batcave\BatCave Monitor")
        || !absolute_path_eq(&key.install_root, r"C:\Program Files\BatCave Monitor")
        || key.value_names != [String::new()]
        || !key.subkey_names.is_empty()
        || key.default_value_type != 1
        || key.last_write_time_100ns == 0
        || !matches!(
            key.owner,
            SecurityPrincipalForProof::LocalSystem
                | SecurityPrincipalForProof::Administrators
                | SecurityPrincipalForProof::TrustedInstaller
        )
    {
        return Err("lifecycle_private_projection_product_registration_invalid".to_string());
    }
    validate_sha256(
        &key.dacl_sha256,
        "private_projection_product_registration_dacl",
    )
}

fn project_shortcut(
    raw: &Observation<ShortcutForProof>,
    expected_root: LogicalRoot,
    roots: &[SanitizationRoot],
) -> Result<Option<SanitizedShortcutSnapshot>, String> {
    match raw {
        Observation::Absent => Ok(None),
        Observation::Unknown(_) => {
            Err("lifecycle_private_projection_observation_unknown".to_string())
        }
        Observation::Present(shortcut) => {
            validate_sha256(&shortcut.sha256, "private_projection_shortcut")?;
            validate_sha256(&shortcut.dacl_sha256, "private_projection_shortcut_dacl")?;
            let path = sanitize_path(&shortcut.path, roots)?;
            let target = sanitize_path(&shortcut.target, roots)?;
            if shortcut.size == 0
                || shortcut.size > 1024 * 1024
                || shortcut.volume_serial == 0
                || shortcut.file_index == 0
                || path.root != expected_root
                || path.relative_leaf != "BatCave Monitor.lnk"
                || target.root != LogicalRoot::Install
                || !target
                    .relative_leaf
                    .eq_ignore_ascii_case("batcave-monitor.exe")
                || !shortcut.arguments.is_empty()
                || !shortcut.icon_path.is_empty()
                || shortcut.icon_index != 0
                || !absolute_path_eq(
                    &shortcut.working_directory,
                    r"C:\Program Files\BatCave Monitor",
                )
                || shortcut.show_command != 1
                || shortcut.hotkey != 0
                || !shortcut.description.is_empty()
                || shortcut.app_user_model_id != "dev.batcave.monitor"
                || (expected_root == LogicalRoot::CommonStartMenu
                    && !matches!(
                        shortcut.owner,
                        SecurityPrincipalForProof::LocalSystem
                            | SecurityPrincipalForProof::Administrators
                            | SecurityPrincipalForProof::TrustedInstaller
                    ))
            {
                return Err("lifecycle_private_projection_shortcut_invalid".to_string());
            }
            Ok(Some(SanitizedShortcutSnapshot {
                path,
                target,
                sha256: shortcut.sha256.clone(),
            }))
        }
    }
}

fn validate_registration_identity_set(raw: &ElevatedMachineSnapshot) -> Result<(), String> {
    let mut identities = BTreeSet::new();
    for shortcut in [
        &raw.machine_registration.public_desktop_shortcut,
        &raw.machine_registration.common_start_menu_shortcut,
    ] {
        if let Observation::Present(shortcut) = shortcut {
            if !identities.insert((shortcut.volume_serial, shortcut.file_index)) {
                return Err(
                    "lifecycle_private_projection_registration_identity_invalid".to_string()
                );
            }
        }
    }
    for file in [
        &raw.machine.monitor,
        &raw.machine.service_binary,
        &raw.machine.uninstaller,
        &raw.machine.legacy_cli,
    ] {
        if let Observation::Present(file) = file {
            if identities.contains(&(file.identity.volume_serial, file.identity.file_index)) {
                return Err(
                    "lifecycle_private_projection_registration_identity_invalid".to_string()
                );
            }
        }
    }
    Ok(())
}

struct ProjectedRuntime {
    named_pipe: Option<SanitizedNamedPipeSnapshot>,
    etw_session: Option<SanitizedEtwObservation>,
    etw_lease_file: Option<LogicalPath>,
    etw_owner_lock: Option<LogicalPath>,
    service_lifecycle_lock: Option<LogicalPath>,
}

fn project_runtime_projection(raw: &ElevatedMachineSnapshot) -> Result<ProjectedRuntime, String> {
    let named_pipe = project_named_pipe(&raw.named_pipe, &raw.machine.service)?;
    let etw_owner_lock = project_runtime_lock(
        &raw.etw_owner_lock,
        crate::collector_service::etw_lease::ETW_OWNER_LOCK_FILE_NAME,
    )?;
    let service_lifecycle_lock = project_runtime_lock(
        &raw.service_lifecycle_lock,
        SERVICE_LIFECYCLE_LOCK_FILE_NAME,
    )?;
    let etw_lease_file = match &raw.etw_lease {
        Observation::Present(_) => Some(service_data_path(
            crate::collector_service::etw_lease::ETW_LEASE_FILE_NAME,
        )),
        Observation::Absent => None,
        Observation::Unknown(_) => {
            return Err("lifecycle_private_projection_observation_unknown".to_string());
        }
    };
    let etw_session = project_etw_session(
        &raw.etw_lease,
        &raw.etw_session,
        &raw.machine.service,
        &raw.named_pipe,
        matches!(raw.etw_owner_lock, RuntimeLockObservation::Held {}),
        matches!(raw.service_lifecycle_lock, RuntimeLockObservation::Held {}),
    )?;
    Ok(ProjectedRuntime {
        named_pipe,
        etw_session,
        etw_lease_file,
        etw_owner_lock,
        service_lifecycle_lock,
    })
}

struct ProjectedServiceInstallResidue {
    service_registry_key: Option<LogicalPath>,
    upgrade_transaction_journal: Option<SanitizedPathFileSnapshot>,
    staged_service_images: Vec<SanitizedPathFileSnapshot>,
    rollback_service_images: Vec<SanitizedPathFileSnapshot>,
    atomic_temporary_files: Vec<SanitizedPathFileSnapshot>,
    failure_marker: Option<SanitizedPathFileSnapshot>,
}

fn project_service_install_residue_projection(
    raw: &ElevatedMachineSnapshot,
    roots: &[SanitizationRoot],
) -> Result<ProjectedServiceInstallResidue, String> {
    validate_raw_residue_identity_set(&raw.service_install_residue)?;
    let service_registry_key =
        project_service_registry_key(&raw.service_install_residue.service_registry_key)?;
    let (upgrade_transaction_journal, service_atomic_temporary_files) =
        project_service_data_residue(
            &raw.service_install_residue.service_data,
            &raw.service_data_root,
            roots,
        )?;
    let install_residue = project_install_residue(
        &raw.service_install_residue.install,
        &raw.machine.install_root,
        roots,
    )?;
    let mut atomic_temporary_files = service_atomic_temporary_files;
    atomic_temporary_files.extend(install_residue.atomic_temporary_files);
    let sort_files = |files: &mut Vec<SanitizedPathFileSnapshot>| {
        files.sort_by(|left, right| {
            (&left.path.root, &left.path.relative_leaf)
                .cmp(&(&right.path.root, &right.path.relative_leaf))
        });
    };
    let mut staged_service_images = install_residue.staged_service_images;
    let mut rollback_service_images = install_residue.rollback_service_images;
    sort_files(&mut staged_service_images);
    sort_files(&mut rollback_service_images);
    sort_files(&mut atomic_temporary_files);
    Ok(ProjectedServiceInstallResidue {
        service_registry_key,
        upgrade_transaction_journal,
        staged_service_images,
        rollback_service_images,
        atomic_temporary_files,
        failure_marker: install_residue.rollback_execution_marker,
    })
}

pub(super) fn validate_restoration_machine_authority(
    raw: &ElevatedMachineSnapshot,
    expectation: RestorationAuthorityExpectation,
) -> Result<(), String> {
    let service = project_service_projection(
        &raw.machine.service,
        &raw.machine.service_binary,
        &raw.installed_boundaries,
    )?;
    let runtime = project_runtime_projection(raw)?;
    let roots = projection_roots()?;
    let residue = project_service_install_residue_projection(raw, &roots)?;
    let registration = project_machine_registration_projection(raw, &roots)?;
    let boundaries = project_boundaries_observation(&raw.installed_boundaries)?;

    validate_observation(&service, validate_service_snapshot)?;
    validate_observation(&boundaries, validate_installed_boundaries)?;
    if let Some(etw) = &runtime.etw_session {
        validate_etw_observation(etw)?;
        let service = service
            .as_present()
            .ok_or_else(|| "lifecycle_stage_restoration_etw_service_missing".to_string())?;
        if etw.lease.service_generation != sha256_digest16(&service.image_sha256)? {
            return Err("lifecycle_stage_restoration_etw_generation_invalid".to_string());
        }
    }

    let runtime_present = runtime.named_pipe.is_some()
        && runtime.etw_session.is_some()
        && runtime.etw_lease_file.is_some()
        && runtime.etw_owner_lock.is_some()
        && runtime.service_lifecycle_lock.is_some();
    let runtime_absent = runtime.named_pipe.is_none()
        && runtime.etw_session.is_none()
        && runtime.etw_lease_file.is_none()
        && runtime.etw_owner_lock.is_none()
        && runtime.service_lifecycle_lock.is_none();
    let transaction_residue_absent = residue.upgrade_transaction_journal.is_none()
        && residue.staged_service_images.is_empty()
        && residue.rollback_service_images.is_empty()
        && residue.atomic_temporary_files.is_empty()
        && residue.failure_marker.is_none();
    let (installed, expect_runtime, expect_shortcuts) = match expectation {
        RestorationAuthorityExpectation::AllowlistedStopped => (true, false, true),
        RestorationAuthorityExpectation::BaselineRunning => (true, true, true),
        RestorationAuthorityExpectation::FinalRunning => (true, true, false),
        RestorationAuthorityExpectation::ProductAbsent => (false, false, false),
    };
    let registration_matches = registration.machine_product_key.is_some() == installed
        && registration.public_desktop_shortcut.is_some() == expect_shortcuts
        && registration.common_start_menu_shortcut.is_some() == expect_shortcuts;
    let residue_matches =
        residue.service_registry_key.is_some() == installed && transaction_residue_absent;
    let runtime_matches = if expect_runtime {
        runtime_present
    } else {
        runtime_absent
    };
    if registration_matches && residue_matches && runtime_matches {
        Ok(())
    } else {
        Err("lifecycle_stage_restoration_authority_invalid".to_string())
    }
}

fn validate_raw_residue_identity_set(raw: &ServiceInstallResidueForProof) -> Result<(), String> {
    let mut identities = BTreeSet::new();
    let mut validate = |file: &ResidueFileForProof, root_volume: u32| {
        if file.volume_serial != root_volume
            || file.file_index == 0
            || !identities.insert((file.volume_serial, file.file_index))
        {
            Err("lifecycle_private_projection_residue_identity_invalid".to_string())
        } else {
            Ok(())
        }
    };
    if let Observation::Present(service_data) = &raw.service_data {
        if let Observation::Present(journal) = &service_data.upgrade_transaction_journal {
            validate(journal, service_data.volume_serial)?;
        }
        for file in &service_data.atomic_temporary_files {
            validate(file, service_data.volume_serial)?;
        }
    }
    if let Observation::Present(install) = &raw.install {
        for file in install
            .staged_service_images
            .iter()
            .chain(&install.rollback_service_images)
            .chain(&install.atomic_temporary_files)
        {
            validate(file, install.volume_serial)?;
        }
        if let Observation::Present(marker) = &install.rollback_execution_marker {
            validate(marker, install.volume_serial)?;
        }
    }
    Ok(())
}

struct ProjectedInstallResidue {
    staged_service_images: Vec<SanitizedPathFileSnapshot>,
    rollback_service_images: Vec<SanitizedPathFileSnapshot>,
    atomic_temporary_files: Vec<SanitizedPathFileSnapshot>,
    rollback_execution_marker: Option<SanitizedPathFileSnapshot>,
}

fn project_service_registry_key(
    raw: &Observation<ServiceRegistryKeyForProof>,
) -> Result<Option<LogicalPath>, String> {
    match raw {
        Observation::Absent => Ok(None),
        Observation::Unknown(_) => {
            Err("lifecycle_private_projection_observation_unknown".to_string())
        }
        Observation::Present(registry) => {
            match &registry.last_failure {
                Observation::Absent => {}
                Observation::Present(value)
                    if !value.is_empty()
                        && value.encode_utf16().count() <= 32 * 1024
                        && !value.chars().any(|character| character == '\0') => {}
                Observation::Present(_) => {
                    return Err("lifecycle_private_projection_service_failure_invalid".to_string())
                }
                Observation::Unknown(_) => {
                    return Err("lifecycle_private_projection_observation_unknown".to_string())
                }
            }
            Ok(Some(LogicalPath {
                root: LogicalRoot::Hklm,
                relative_leaf: "system/currentcontrolset/services/batcavecollector".to_string(),
            }))
        }
    }
}

fn project_service_data_residue(
    raw: &Observation<ServiceDataResidueForProof>,
    raw_root: &Observation<DirectorySnapshot>,
    roots: &[SanitizationRoot],
) -> Result<
    (
        Option<SanitizedPathFileSnapshot>,
        Vec<SanitizedPathFileSnapshot>,
    ),
    String,
> {
    match raw {
        Observation::Absent => {
            if !matches!(raw_root, Observation::Absent) {
                return Err("lifecycle_private_projection_residue_root_mismatch".to_string());
            }
            Ok((None, Vec::new()))
        }
        Observation::Unknown(_) => {
            Err("lifecycle_private_projection_observation_unknown".to_string())
        }
        Observation::Present(residue) => {
            require_residue_root_identity(raw_root, residue.volume_serial, residue.file_index)?;
            let journal_raw = residue.upgrade_transaction_journal.as_present();
            let total_bytes = residue
                .atomic_temporary_files
                .iter()
                .chain(journal_raw)
                .try_fold(0_u64, |total, file| total.checked_add(file.size))
                .ok_or_else(|| {
                    "lifecycle_private_projection_residue_total_size_invalid".to_string()
                })?;
            if !crate::collector_service::windows_provisioner::residue_set_bounds_valid_for_proof(
                residue.atomic_temporary_files.len() + usize::from(journal_raw.is_some()),
                total_bytes,
            ) {
                return Err("lifecycle_private_projection_residue_total_size_invalid".to_string());
            }
            let journal = match &residue.upgrade_transaction_journal {
                Observation::Absent => None,
                Observation::Unknown(_) => {
                    return Err("lifecycle_private_projection_observation_unknown".to_string())
                }
                Observation::Present(file) => {
                    if !crate::collector_service::windows_provisioner::residue_size_valid_for_proof(
                        ResidueKindForProof::Journal,
                        file.size,
                    ) {
                        return Err(
                            "lifecycle_private_projection_residue_file_size_invalid".to_string()
                        );
                    }
                    let file = project_residue_file(file, roots)?;
                    if file.path.root != LogicalRoot::ServiceData
                        || file.path.relative_leaf != "installer-upgrade.v1.json"
                    {
                        return Err(
                            "lifecycle_private_projection_upgrade_journal_invalid".to_string()
                        );
                    }
                    Some(file)
                }
            };
            let atomic = project_residue_file_set(
                &residue.atomic_temporary_files,
                roots,
                LogicalRoot::ServiceData,
                crate::collector_service::windows_provisioner::service_data_atomic_temp_name_for_proof,
                ResidueKindForProof::ServiceDataAtomic,
            )?;
            Ok((journal, atomic))
        }
    }
}

fn project_install_residue(
    raw: &Observation<InstallResidueForProof>,
    raw_root: &Observation<DirectorySnapshot>,
    roots: &[SanitizationRoot],
) -> Result<ProjectedInstallResidue, String> {
    match raw {
        Observation::Absent => {
            if !matches!(raw_root, Observation::Absent) {
                return Err("lifecycle_private_projection_residue_root_mismatch".to_string());
            }
            Ok(ProjectedInstallResidue {
                staged_service_images: Vec::new(),
                rollback_service_images: Vec::new(),
                atomic_temporary_files: Vec::new(),
                rollback_execution_marker: None,
            })
        }
        Observation::Unknown(_) => {
            Err("lifecycle_private_projection_observation_unknown".to_string())
        }
        Observation::Present(residue) => {
            require_residue_root_identity(raw_root, residue.volume_serial, residue.file_index)?;
            let marker_raw = residue.rollback_execution_marker.as_present();
            let total_bytes = residue
                .staged_service_images
                .iter()
                .chain(&residue.rollback_service_images)
                .chain(&residue.atomic_temporary_files)
                .chain(marker_raw)
                .try_fold(0_u64, |total, file| total.checked_add(file.size))
                .ok_or_else(|| {
                    "lifecycle_private_projection_residue_total_size_invalid".to_string()
                })?;
            let count = residue.staged_service_images.len()
                + residue.rollback_service_images.len()
                + residue.atomic_temporary_files.len()
                + usize::from(marker_raw.is_some());
            if !crate::collector_service::windows_provisioner::residue_set_bounds_valid_for_proof(
                count,
                total_bytes,
            ) {
                return Err("lifecycle_private_projection_residue_total_size_invalid".to_string());
            }
            let staged = project_residue_file_set(
                &residue.staged_service_images,
                roots,
                LogicalRoot::Install,
                crate::collector_service::windows_provisioner::install_staged_name_for_proof,
                ResidueKindForProof::Staged,
            )?;
            let rollback = project_residue_file_set(
                &residue.rollback_service_images,
                roots,
                LogicalRoot::Install,
                crate::collector_service::windows_provisioner::install_rollback_name_for_proof,
                ResidueKindForProof::Rollback,
            )?;
            let atomic = project_residue_file_set(
                &residue.atomic_temporary_files,
                roots,
                LogicalRoot::Install,
                crate::collector_service::windows_provisioner::install_atomic_temp_name_for_proof,
                ResidueKindForProof::InstallAtomic,
            )?;
            let marker = match &residue.rollback_execution_marker {
                Observation::Absent => None,
                Observation::Unknown(_) => {
                    return Err("lifecycle_private_projection_observation_unknown".to_string())
                }
                Observation::Present(file) => {
                    if !crate::collector_service::windows_provisioner::residue_size_valid_for_proof(
                        ResidueKindForProof::RollbackExecutionMarker,
                        file.size,
                    ) {
                        return Err(
                            "lifecycle_private_projection_residue_file_size_invalid".to_string()
                        );
                    }
                    let file = project_residue_file(file, roots)?;
                    if file.path.root != LogicalRoot::Install
                        || file.path.relative_leaf
                            != crate::collector_service::windows_provisioner::rollback_execution_marker_name_for_proof()
                    {
                        return Err(
                            "lifecycle_private_projection_rollback_marker_invalid".to_string(),
                        );
                    }
                    Some(file)
                }
            };
            Ok(ProjectedInstallResidue {
                staged_service_images: staged,
                rollback_service_images: rollback,
                atomic_temporary_files: atomic,
                rollback_execution_marker: marker,
            })
        }
    }
}

fn require_residue_root_identity(
    root: &Observation<DirectorySnapshot>,
    volume_serial: u32,
    file_index: u64,
) -> Result<(), String> {
    match root {
        Observation::Present(root)
            if root.identity.volume_serial == volume_serial
                && root.identity.file_index == file_index =>
        {
            Ok(())
        }
        Observation::Unknown(_) => {
            Err("lifecycle_private_projection_observation_unknown".to_string())
        }
        _ => Err("lifecycle_private_projection_residue_root_mismatch".to_string()),
    }
}

fn project_residue_file_set(
    raw: &[ResidueFileForProof],
    roots: &[SanitizationRoot],
    expected_root: LogicalRoot,
    valid_name: impl Fn(&str) -> bool,
    kind: ResidueKindForProof,
) -> Result<Vec<SanitizedPathFileSnapshot>, String> {
    if raw.len() > 64 {
        return Err("lifecycle_private_projection_residue_set_invalid".to_string());
    }
    let mut projected = Vec::with_capacity(raw.len());
    let mut paths = BTreeSet::new();
    for file in raw {
        if !crate::collector_service::windows_provisioner::residue_size_valid_for_proof(
            kind, file.size,
        ) {
            return Err("lifecycle_private_projection_residue_file_size_invalid".to_string());
        }
        let file = project_residue_file(file, roots)?;
        if file.path.root != expected_root
            || file.path.relative_leaf.contains('/')
            || file.path.relative_leaf.contains('\\')
            || !valid_name(&file.path.relative_leaf)
            || !paths.insert(file.path.relative_leaf.to_ascii_lowercase())
        {
            return Err("lifecycle_private_projection_residue_set_invalid".to_string());
        }
        projected.push(file);
    }
    projected.sort_by(|left, right| left.path.relative_leaf.cmp(&right.path.relative_leaf));
    Ok(projected)
}

fn project_residue_file(
    raw: &ResidueFileForProof,
    roots: &[SanitizationRoot],
) -> Result<SanitizedPathFileSnapshot, String> {
    validate_sha256(&raw.sha256, "private_projection_residue")?;
    if raw.size == 0 || raw.volume_serial == 0 || raw.file_index == 0 {
        return Err("lifecycle_private_projection_residue_file_invalid".to_string());
    }
    Ok(SanitizedPathFileSnapshot {
        path: sanitize_path(&raw.path, roots)?,
        file: SanitizedFileSnapshot {
            size: raw.size,
            sha256: raw.sha256.clone(),
            volume_serial: raw.volume_serial,
            file_index: raw.file_index,
        },
    })
}

fn project_named_pipe(
    raw: &Observation<super::native::NamedPipeSnapshot>,
    service: &Observation<ServiceSnapshot>,
) -> Result<Option<SanitizedNamedPipeSnapshot>, String> {
    match raw {
        Observation::Absent => Ok(None),
        Observation::Unknown(_) => {
            Err("lifecycle_private_projection_observation_unknown".to_string())
        }
        Observation::Present(pipe) => {
            let Observation::Present(service) = service else {
                return Err("lifecycle_private_projection_pipe_service_missing".to_string());
            };
            if service.state != 4
                || service.process_id != pipe.server_process_id
                || service.process_started_at_100ns != Some(pipe.server_process_started_at_100ns)
            {
                return Err("lifecycle_private_projection_pipe_service_mismatch".to_string());
            }
            Ok(Some(SanitizedNamedPipeSnapshot {
                server_process_id: pipe.server_process_id,
            }))
        }
    }
}

fn project_etw_session(
    lease: &Observation<EtwLeaseV1>,
    session: &Observation<crate::windows_network::EtwSessionProofSnapshot>,
    service: &Observation<ServiceSnapshot>,
    pipe: &Observation<super::native::NamedPipeSnapshot>,
    owner_lock_held: bool,
    process_lock_held: bool,
) -> Result<Option<SanitizedEtwObservation>, String> {
    match (lease, session) {
        (Observation::Absent, Observation::Absent) => Ok(None),
        (Observation::Unknown(_), _) | (_, Observation::Unknown(_)) => {
            Err("lifecycle_private_projection_observation_unknown".to_string())
        }
        (Observation::Present(lease), Observation::Present(session)) => {
            let Observation::Present(service) = service else {
                return Err("lifecycle_private_projection_etw_service_missing".to_string());
            };
            let Observation::Present(pipe) = pipe else {
                return Err("lifecycle_private_projection_etw_pipe_missing".to_string());
            };
            if lease.phase != EtwLeasePhase::Active
                || lease.session != session.identity
                || service.state != 4
                || service.process_id != lease.controller.process_id
                || service.process_started_at_100ns != Some(lease.controller.process_started_at)
                || pipe.server_process_id != lease.controller.process_id
                || pipe.server_process_started_at_100ns != lease.controller.process_started_at
            {
                return Err("lifecycle_private_projection_etw_identity_mismatch".to_string());
            }
            let buffers_lost = session
                .log_buffers_lost
                .checked_add(session.realtime_buffers_lost)
                .ok_or_else(|| "lifecycle_private_projection_etw_loss_overflow".to_string())?;
            Ok(Some(SanitizedEtwObservation {
                lease: lease.clone(),
                observed_session: session.identity.clone(),
                owner_lock_held,
                process_lock_held,
                events_lost: session.events_lost,
                buffers_lost,
            }))
        }
        _ => Err("lifecycle_private_projection_etw_presence_mismatch".to_string()),
    }
}

fn project_runtime_lock(
    observation: &RuntimeLockObservation,
    leaf: &str,
) -> Result<Option<LogicalPath>, String> {
    match observation {
        RuntimeLockObservation::Held {} | RuntimeLockObservation::Released {} => {
            Ok(Some(service_data_path(leaf)))
        }
        RuntimeLockObservation::Absent {} => Ok(None),
        RuntimeLockObservation::Unknown { .. } => {
            Err("lifecycle_private_projection_observation_unknown".to_string())
        }
    }
}

fn service_data_path(leaf: &str) -> LogicalPath {
    LogicalPath {
        root: LogicalRoot::ServiceData,
        relative_leaf: leaf.to_string(),
    }
}

fn project_parent_current_user_directory(
    authority: &ParentCurrentUserAuthority,
) -> Result<Observation<SanitizedDirectorySnapshot>, String> {
    project_observation(&authority.data_root, |directory| {
        if !directory
            .final_path
            .eq_ignore_ascii_case(&authority.resolved_data_root)
        {
            return Err("lifecycle_parent_user_data_root_drift".to_string());
        }
        Ok(SanitizedDirectorySnapshot {
            volume_serial: directory.identity.volume_serial,
            file_index: directory.identity.file_index,
            final_path: LogicalPath {
                root: LogicalRoot::CurrentUserData,
                relative_leaf: String::new(),
            },
        })
    })
}

fn compare_parent_current_user_retention(
    sanitized: &SanitizedCurrentUserRetention,
    before: &ParentCurrentUserObjects,
    after: &ParentCurrentUserObjects,
) -> Result<(), String> {
    for (object, leaf, before, after) in [
        (
            &sanitized.settings,
            RETAINED_SETTINGS_LEAF,
            &before.settings,
            &after.settings,
        ),
        (
            &sanitized.cache,
            RETAINED_CACHE_LEAF,
            &before.cache,
            &after.cache,
        ),
        (
            &sanitized.diagnostics,
            RETAINED_DIAGNOSTICS_LEAF,
            &before.diagnostics,
            &after.diagnostics,
        ),
    ] {
        if object.path.root != LogicalRoot::CurrentUserData
            || object.path.relative_leaf != leaf
            || object.before_uninstall != project_parent_user_digest(before)?
            || object.after_uninstall != project_parent_user_digest(after)?
        {
            return Err("lifecycle_parent_user_retention_drift".to_string());
        }
    }
    Ok(())
}

fn project_parent_user_digest(
    observation: &Observation<FileSnapshot>,
) -> Result<Observation<SanitizedDigestSnapshot>, String> {
    project_observation(observation, |file| {
        validate_sha256(&file.sha256, "parent_user_object")?;
        Ok(SanitizedDigestSnapshot {
            size: file.size,
            sha256: file.sha256.clone(),
        })
    })
}

fn projection_roots() -> Result<Vec<SanitizationRoot>, String> {
    Ok(vec![
        SanitizationRoot::new(LogicalRoot::Install, r"C:\Program Files\BatCave Monitor")?,
        SanitizationRoot::new(LogicalRoot::ProductData, r"C:\ProgramData\BatCaveMonitor")?,
        SanitizationRoot::new(
            LogicalRoot::ServiceData,
            r"C:\ProgramData\BatCaveMonitor\Service",
        )?,
        SanitizationRoot::new(
            LogicalRoot::WebViewRuntime,
            r"C:\Program Files (x86)\Microsoft\EdgeWebView\Application",
        )?,
        SanitizationRoot::new(LogicalRoot::PublicDesktop, r"C:\Users\Public\Desktop")?,
        SanitizationRoot::new(
            LogicalRoot::CommonStartMenu,
            r"C:\ProgramData\Microsoft\Windows\Start Menu\Programs",
        )?,
    ])
}

fn project_service_projection(
    raw: &Observation<ServiceSnapshot>,
    raw_service_file: &Observation<FileSnapshot>,
    raw_boundaries: &Observation<InstalledBoundariesForProof>,
) -> Result<Observation<SanitizedServiceSnapshot>, String> {
    match raw {
        Observation::Absent => Ok(Observation::Absent),
        Observation::Present(raw) => {
            let Observation::Present(file) = raw_service_file else {
                return Err("lifecycle_private_projection_service_file_missing".to_string());
            };
            let Observation::Present(boundaries) = raw_boundaries else {
                return Err("lifecycle_private_projection_service_boundary_missing".to_string());
            };
            Ok(Observation::Present(SanitizedServiceSnapshot {
                state: raw.state,
                process_id: raw.process_id,
                process_started_at_100ns: raw.process_started_at_100ns,
                win32_exit_code: raw.win32_exit_code,
                service_specific_exit_code: raw.service_specific_exit_code,
                image_path: LogicalPath {
                    root: LogicalRoot::Install,
                    relative_leaf: "batcave-collector-service.exe".to_string(),
                },
                image_sha256: file.sha256.clone(),
                local_system: true,
                own_process: true,
                automatic_start: true,
                recovery_restart_action_count: 3,
                owner_marker: "dev.batcave.monitor/service-v1".to_string(),
                service_dacl_sha256: boundaries.service_dacl_sha256.clone(),
            }))
        }
        Observation::Unknown(_) => {
            Err("lifecycle_private_projection_observation_unknown".to_string())
        }
    }
}

fn project_file_observation(
    raw: &Observation<FileSnapshot>,
) -> Result<Observation<SanitizedFileSnapshot>, String> {
    project_observation(raw, |file| {
        Ok(SanitizedFileSnapshot {
            size: file.size,
            sha256: file.sha256.clone(),
            volume_serial: file.identity.volume_serial,
            file_index: file.identity.file_index,
        })
    })
}

fn project_directory_observation(
    raw: &Observation<DirectorySnapshot>,
    roots: &[SanitizationRoot],
    expected_root: LogicalRoot,
) -> Result<Observation<SanitizedDirectorySnapshot>, String> {
    project_observation(raw, |directory| {
        let final_path = sanitize_path(&directory.final_path, roots)?;
        if final_path.root != expected_root {
            return Err("lifecycle_private_projection_directory_root_invalid".to_string());
        }
        Ok(SanitizedDirectorySnapshot {
            volume_serial: directory.identity.volume_serial,
            file_index: directory.identity.file_index,
            final_path,
        })
    })
}

fn project_registry_observation(
    raw: &Observation<RegistrySnapshot>,
    roots: &[SanitizationRoot],
) -> Result<Observation<SanitizedRegistrySnapshot>, String> {
    project_observation(raw, |registry| {
        let install_location = sanitize_path(&registry.install_location, roots)?;
        if install_location.root != LogicalRoot::Install
            || !install_location.relative_leaf.is_empty()
        {
            return Err("lifecycle_private_projection_registry_root_invalid".to_string());
        }
        Ok(SanitizedRegistrySnapshot {
            view: match registry.view {
                RegistryView::Registry32 => "32",
                RegistryView::Registry64 => "64",
            }
            .to_string(),
            key: LogicalPath {
                root: LogicalRoot::Hklm,
                relative_leaf:
                    "software/microsoft/windows/currentversion/uninstall/batcave-monitor"
                        .to_string(),
            },
            install_location,
        })
    })
}

fn project_process_observation(
    raw: &Observation<Vec<ProcessSnapshot>>,
    roots: &[SanitizationRoot],
) -> Result<Observation<Vec<SanitizedProcessSnapshot>>, String> {
    project_observation(raw, |processes| {
        processes
            .iter()
            .map(|process| {
                Ok(SanitizedProcessSnapshot {
                    process_id: process.process_id,
                    executable_name: process.executable_name.clone(),
                    executable_path: process
                        .executable_path
                        .as_deref()
                        .map(|path| sanitize_path(path, roots))
                        .transpose()?,
                })
            })
            .collect()
    })
}

fn project_boundaries_observation(
    raw: &Observation<InstalledBoundariesForProof>,
) -> Result<Observation<SanitizedBoundarySnapshot>, String> {
    project_observation(raw, |boundaries| {
        Ok(SanitizedBoundarySnapshot {
            service_dacl_sha256: boundaries.service_dacl_sha256.clone(),
            service_aces: boundaries
                .service_aces
                .iter()
                .map(project_ace)
                .collect::<Result<_, _>>()?,
            service_data_root_owner: project_principal(boundaries.service_data_root.owner)?,
            service_data_root_dacl_protected: boundaries.service_data_root.dacl_protected,
            service_data_root_reparse: boundaries.service_data_root.reparse,
            service_data_root_dacl_sha256: boundaries.service_data_root_dacl_sha256.clone(),
            service_data_root_aces: boundaries
                .service_data_root
                .aces
                .iter()
                .map(project_ace)
                .collect::<Result<_, _>>()?,
        })
    })
}

fn project_ace(ace: &AcePolicyForProof) -> Result<SanitizedAceSnapshot, String> {
    Ok(SanitizedAceSnapshot {
        principal: project_principal(ace.principal)?,
        allow: ace.allow,
        inherit_only: ace.inherit_only,
        object_inherit: ace.object_inherit,
        container_inherit: ace.container_inherit,
        mask: ace.mask,
    })
}

fn project_principal(principal: SecurityPrincipalForProof) -> Result<SanitizedPrincipal, String> {
    match principal {
        SecurityPrincipalForProof::LocalSystem => Ok(SanitizedPrincipal::LocalSystem),
        SecurityPrincipalForProof::Administrators => Ok(SanitizedPrincipal::Administrators),
        SecurityPrincipalForProof::InteractiveUsers => Ok(SanitizedPrincipal::InteractiveUsers),
        SecurityPrincipalForProof::CollectorService => Ok(SanitizedPrincipal::CollectorService),
        SecurityPrincipalForProof::TrustedInstaller | SecurityPrincipalForProof::Other => {
            Err("lifecycle_private_projection_principal_invalid".to_string())
        }
    }
}

fn project_service_crash(
    termination: &TerminatedServiceForProof,
) -> Result<SanitizedServiceCrashEvent, String> {
    Ok(SanitizedServiceCrashEvent {
        process_id: termination.target.process_id,
        process_started_at_100ns: termination.target.process_started_at_100ns,
        image_path: sanitize_path(&termination.target.image_path, &projection_roots()?)?,
        image_sha256: termination.target.image_sha256.clone(),
        process_exit_code: termination.process_exit_code,
        win32_exit_code: termination.win32_exit_code,
        service_specific_exit_code: termination.service_specific_exit_code,
    })
}

fn project_observation<T, U>(
    raw: &Observation<T>,
    project: impl FnOnce(&T) -> Result<U, String>,
) -> Result<Observation<U>, String> {
    match raw {
        Observation::Present(value) => Ok(Observation::Present(project(value)?)),
        Observation::Absent => Ok(Observation::Absent),
        Observation::Unknown(_) => {
            Err("lifecycle_private_projection_observation_unknown".to_string())
        }
    }
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
        let expects_known_retired_helpers = (12..=18).contains(&index);
        let has_known_retired_helpers = !entry.machine.known_retired_helper_artifacts.is_empty();
        if expects_known_retired_helpers != has_known_retired_helpers {
            return Err("lifecycle_sanitized_legacy_helper_lifetime_invalid".to_string());
        }
        let expects_hkcu_autostart = (12..=26).contains(&index);
        if expects_hkcu_autostart != entry.machine.hkcu_autostart.is_some() {
            return Err("lifecycle_sanitized_hkcu_autostart_lifetime_invalid".to_string());
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

#[derive(Clone, Debug, Eq, PartialEq)]
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
    let mut service_instance_generations = BTreeMap::new();
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
        if entry
            .machine
            .service
            .as_present()
            .is_some_and(|service| service.state == 4)
        {
            let generation = running_generation(&entry.machine)?;
            if service_instance_generations
                .insert(generation.service_instance_id, generation.clone())
                .is_some_and(|prior| prior != generation)
            {
                return Err("lifecycle_sanitized_generation_continuity_invalid".to_string());
            }
        }
        if receipt_name == "initial-uninstall-state.private.json" {
            post_uninstall_epoch = true;
        }
    }
    match (pre_uninstall_install_id, post_reinstall_install_id) {
        (Some(before), Some(after)) if before != after => {}
        _ => return Err("lifecycle_sanitized_etw_install_epoch_invalid".to_string()),
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
        validate_path_file(file, Some(LogicalRoot::ServiceData))?;
    }
    for files in [
        &machine.staged_service_images,
        &machine.rollback_service_images,
    ] {
        validate_path_file_set(files, Some(LogicalRoot::Install))?;
    }
    validate_path_file_set(&machine.atomic_temporary_files, None)?;
    if machine.atomic_temporary_files.iter().any(|file| {
        !matches!(
            file.path.root,
            LogicalRoot::ServiceData | LogicalRoot::Install
        )
    }) {
        return Err("lifecycle_sanitized_atomic_temp_root_invalid".to_string());
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
            SanitizedEvidenceAssertion::FinalUninstalledPreservingDeclaredCurrentUserObjects
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
            if !matches!(machine.legacy_cli, Observation::Absent) {
                return Err("lifecycle_sanitized_upgrade_cleanup_invalid".to_string());
            }
            validate_known_helper_seed(machine)?;
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
        SanitizedEvidenceAssertion::FinalUninstalledPreservingDeclaredCurrentUserObjects => {
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
    shared_shortcuts_present: bool,
}

fn allowlisted_artifacts(plan: &ProofPlan) -> InstalledArtifactExpectation<'_> {
    InstalledArtifactExpectation {
        monitor_sha256: &plan.allowlisted_start.monitor_sha256,
        service_sha256: &plan.allowlisted_start.service_sha256,
        service_size: None,
        uninstaller_sha256: &plan.allowlisted_start.uninstaller_sha256,
        uninstaller_size: None,
        shared_shortcuts_present: true,
    }
}

fn baseline_artifacts(plan: &ProofPlan) -> InstalledArtifactExpectation<'_> {
    InstalledArtifactExpectation {
        monitor_sha256: &plan.baseline.monitor_sha256,
        service_sha256: &plan.baseline.service_sha256,
        service_size: None,
        uninstaller_sha256: &plan.baseline.uninstaller_sha256,
        uninstaller_size: Some(plan.baseline.uninstaller_size),
        shared_shortcuts_present: true,
    }
}

fn final_artifacts(plan: &ProofPlan) -> InstalledArtifactExpectation<'_> {
    InstalledArtifactExpectation {
        monitor_sha256: &plan.final_candidate.monitor_sha256,
        service_sha256: &plan.final_candidate.service_sha256,
        service_size: None,
        uninstaller_sha256: &plan.final_candidate.uninstaller_sha256,
        uninstaller_size: Some(plan.final_candidate.uninstaller_size),
        shared_shortcuts_present: false,
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
    if !shared_shortcut_timeline_valid(machine, artifacts.shared_shortcuts_present) {
        return Err("lifecycle_sanitized_shared_shortcut_timeline_invalid".to_string());
    }
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
    if !shared_shortcut_timeline_valid(machine, artifacts.shared_shortcuts_present) {
        return Err("lifecycle_sanitized_shared_shortcut_timeline_invalid".to_string());
    }
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
    let product_key_valid = machine.machine_product_key.as_ref().is_some_and(|path| {
        logical_leaf_eq(path, LogicalRoot::Hklm, "software/batcave/batcave monitor")
    });
    uninstall_registry_valid && (!require_service_key || service_key_valid) && product_key_valid
}

fn shared_shortcut_timeline_valid(
    machine: &SanitizedMachineSnapshot,
    should_be_present: bool,
) -> bool {
    if !should_be_present {
        return machine.public_desktop_shortcut.is_none()
            && machine.common_start_menu_shortcut.is_none();
    }
    machine
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
        })
        && machine
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
            })
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
    use super::super::private_evidence::{
        packet_for_test, PrivateDesktopPayload, PrivateServiceCrashPayload,
        PrivateUpgradeRollbackPayload,
    };
    use super::*;
    use crate::collector_service::windows_provisioner::{
        FailedUpgradeRollbackForProof, SecurityPolicyForProof, ServiceTerminationTargetForProof,
    };
    use crate::windows_lifecycle_proof_contract::{
        parse_plan, DesktopCollectorState, DesktopPrivilegedSource,
    };

    #[test]
    fn private_projection_gate_is_enabled_after_integrated_review() {
        assert_eq!(require_private_evidence_projection_ready(), Ok(()));
    }

    #[test]
    fn private_worker_packet_excludes_parent_current_user_raw_authority() {
        let plan = parse_plan().expect("plan");
        let receipts = success_receipts();
        let export = valid_export(&plan, &receipts);
        let entry = export
            .private_evidence
            .iter()
            .find(|entry| entry.receipt.name == "legacy-residue-seeded-state.private.json")
            .expect("seeded entry");
        let packet = packet_for_test(PrivateSuccessPayload::Machine(raw_machine(&entry.machine)));
        let json = serde_json::to_string(&packet).expect("private packet json");
        for forbidden in [
            "S-1-5-21-1",
            "user_sid",
            "hkcu_run",
            "local_app_data",
            "unknown-sentinel.bin",
            r"C:\Users\proof",
        ] {
            assert!(
                !json.contains(forbidden),
                "private packet leaked {forbidden}"
            );
        }
    }

    #[test]
    fn verified_projection_derives_representable_machine_semantics() {
        let plan = parse_plan().expect("plan");
        let receipts = success_receipts();
        let export = valid_export(&plan, &receipts);
        let entry = export
            .private_evidence
            .iter()
            .find(|entry| entry.receipt.name == "initial-state.private.json")
            .expect("initial entry");
        let packet = packet_for_test(PrivateSuccessPayload::Machine(raw_machine(&entry.machine)));

        assert_eq!(
            compare_verified_projection(
                &export,
                &[(&entry.receipt, packet)],
                &plan,
                &parent_desktop_results(&plan),
                parent_current_user_projection(&export),
            ),
            Ok(())
        );
    }

    #[test]
    fn parent_prepares_exact_sanitized_v2_bytes_from_private_and_parent_authority() {
        let plan = parse_plan().expect("plan");
        let receipts = success_receipts();
        let expected = valid_export(&plan, &receipts);
        let parent_results = parent_desktop_results(&plan);
        let private_packets = private_packets_for_export(&expected, &parent_results);
        let prepared = prepare_sanitized_export(
            &private_packets,
            &plan,
            &"c".repeat(40),
            &"d".repeat(64),
            &parent_results,
            parent_current_user_projection(&expected),
        )
        .expect("parent-prepared export");

        assert_eq!(
            serde_json::from_slice::<SanitizedExportPacket>(prepared.bytes())
                .expect("prepared packet"),
            expected
        );
        assert_eq!(
            prepared.receipt(),
            &EvidenceReceipt {
                name: "windows-lifecycle-proof.sanitized.json".to_string(),
                size: prepared.bytes().len() as u64,
                sha256: sha256_hex(prepared.bytes()),
            }
        );
        assert_eq!(reject_private_path_leakage(prepared.bytes()), Ok(()));
        assert!(prepared.bytes().contains(&b'\n'));
    }

    #[test]
    fn initial_residue_projection_is_independent_and_enforces_run_owner_matrix() {
        assert_eq!(
            parent_residue_capture_point("initial-state.private.json"),
            Ok(ParentCurrentUserCapturePoint::Checkpoint(
                LifecycleStage::InitialState
            ))
        );
        assert_eq!(
            parent_residue_capture_point("final-repair-state.private.json"),
            Ok(ParentCurrentUserCapturePoint::Checkpoint(
                LifecycleStage::FinalRepair
            ))
        );

        let plan = parse_plan().expect("plan");
        let receipts = success_receipts();
        let export = valid_export(&plan, &receipts);
        let initial = export
            .private_evidence
            .iter()
            .find(|entry| entry.receipt.name == "initial-state.private.json")
            .expect("initial entry");
        let authority = parent_current_user();
        for owner in [&authority.user_sid, "S-1-5-18", "S-1-5-32-544"] {
            let mut snapshot = parent_residue_snapshot(&initial.machine);
            snapshot.hkcu_run.owner_sid = owner.to_string();
            let mut timeline = ParentCurrentUserResidueTimeline::default();
            timeline
                .insert(
                    ParentCurrentUserCapturePoint::Checkpoint(LifecycleStage::InitialState),
                    snapshot,
                )
                .expect("initial parent capture");
            assert_eq!(
                compare_parent_current_user_residue_projection(
                    "initial-state.private.json",
                    &initial.machine,
                    &timeline,
                    &authority,
                ),
                Ok(())
            );
        }
        let mut snapshot = parent_residue_snapshot(&initial.machine);
        snapshot.hkcu_run.owner_sid = "S-1-5-11".to_string();
        let mut timeline = ParentCurrentUserResidueTimeline::default();
        timeline
            .insert(
                ParentCurrentUserCapturePoint::Checkpoint(LifecycleStage::InitialState),
                snapshot,
            )
            .expect("initial parent capture");
        assert!(compare_parent_current_user_residue_projection(
            "initial-state.private.json",
            &initial.machine,
            &timeline,
            &authority,
        )
        .is_err());
    }

    #[test]
    fn raw_runtime_authority_roundtrips_and_requires_every_nested_field() {
        let plan = parse_plan().expect("plan");
        let receipts = success_receipts();
        let export = valid_export(&plan, &receipts);
        let running = export
            .private_evidence
            .iter()
            .find(|entry| entry.receipt.name == "final-repair-state.private.json")
            .expect("running entry");
        let raw = raw_machine(&running.machine);
        let value = serde_json::to_value(&raw).expect("raw machine");
        assert_eq!(
            serde_json::from_value::<ElevatedMachineSnapshot>(value.clone())
                .expect("raw roundtrip"),
            raw
        );
        for field in [
            "named_pipe",
            "etw_lease",
            "etw_session",
            "etw_owner_lock",
            "service_lifecycle_lock",
            "service_install_residue",
            "machine_registration",
        ] {
            let mut missing = value.clone();
            missing
                .as_object_mut()
                .expect("machine object")
                .remove(field);
            assert!(
                serde_json::from_value::<ElevatedMachineSnapshot>(missing).is_err(),
                "{field}"
            );
        }
        let mut unknown_lock = value.clone();
        unknown_lock["etw_owner_lock"]
            .as_object_mut()
            .expect("lock object")
            .insert("unexpected".to_string(), serde_json::json!(true));
        assert!(serde_json::from_value::<ElevatedMachineSnapshot>(unknown_lock).is_err());
        let mut unknown_nested = value;
        unknown_nested["named_pipe"]["value"]
            .as_object_mut()
            .expect("named pipe value")
            .insert("unexpected".to_string(), serde_json::json!(true));
        assert!(serde_json::from_value::<ElevatedMachineSnapshot>(unknown_nested).is_err());
    }

    #[test]
    fn raw_service_install_residue_projects_exact_v2_semantics_and_rejects_hostile_drift() {
        let plan = parse_plan().expect("plan");
        let receipts = success_receipts();
        let export = valid_export(&plan, &receipts);
        let mut machine = export
            .private_evidence
            .iter()
            .find(|entry| entry.receipt.name == "final-repair-state.private.json")
            .expect("running entry")
            .machine
            .clone();
        machine.upgrade_transaction_journal = Some(path_file(
            LogicalRoot::ServiceData,
            "installer-upgrade.v1.json",
            "1",
        ));
        machine.staged_service_images.push(path_file(
            LogicalRoot::Install,
            "batcave-collector-service.0.2.0.staged.exe",
            "2",
        ));
        machine.rollback_service_images.push(path_file(
            LogicalRoot::Install,
            &format!("batcave-collector-service.{}.rollback.exe", "3".repeat(64)),
            "3",
        ));
        machine.atomic_temporary_files = vec![
            path_file(
                LogicalRoot::ServiceData,
                "installer-upgrade.v1.json.41.1.tmp",
                "4",
            ),
            path_file(
                LogicalRoot::Install,
                "batcave-collector-service.rollback.tmp",
                "5",
            ),
        ];
        machine.failure_marker = Some(path_file(
            LogicalRoot::Install,
            "batcave-rollback-fixture-ran.v1",
            "6",
        ));
        machine
            .upgrade_transaction_journal
            .as_mut()
            .expect("journal")
            .file
            .file_index = 20;
        machine.staged_service_images[0].file.file_index = 21;
        machine.rollback_service_images[0].file.file_index = 22;
        machine.atomic_temporary_files[0].file.file_index = 23;
        machine.atomic_temporary_files[1].file.file_index = 24;
        machine
            .failure_marker
            .as_mut()
            .expect("marker")
            .file
            .file_index = 25;
        let raw = raw_machine(&machine);
        assert_eq!(
            compare_machine_projection(&raw, &machine, &plan, &parent_current_user()),
            Ok(())
        );

        let sanitized = serde_json::to_value(&machine).expect("sanitized machine");
        assert!(sanitized.get("failure_marker").is_some());
        assert!(sanitized.get("rollback_execution_marker").is_none());
        assert_eq!(SANITIZED_SCHEMA, "batcave_windows_lifecycle_sanitized_v2");

        let assert_rejected = |raw: ElevatedMachineSnapshot| {
            assert!(
                compare_machine_projection(&raw, &machine, &plan, &parent_current_user()).is_err()
            );
        };
        for field in 0..3 {
            let mut hostile = raw.clone();
            match field {
                0 => {
                    hostile.service_install_residue.service_registry_key =
                        Observation::Unknown("registry_unknown".to_string())
                }
                1 => {
                    hostile.service_install_residue.service_data =
                        Observation::Unknown("service_data_unknown".to_string())
                }
                2 => {
                    hostile.service_install_residue.install =
                        Observation::Unknown("install_unknown".to_string())
                }
                _ => unreachable!(),
            }
            assert_rejected(hostile);
        }

        let mut unknown_last_failure = raw.clone();
        let Observation::Present(registry) = &mut unknown_last_failure
            .service_install_residue
            .service_registry_key
        else {
            panic!("service registry");
        };
        registry.last_failure = Observation::Unknown("failure_unknown".to_string());
        assert_rejected(unknown_last_failure);

        let mut root_identity = raw.clone();
        let Observation::Present(service_data) =
            &mut root_identity.service_install_residue.service_data
        else {
            panic!("service data");
        };
        service_data.file_index += 1;
        assert_rejected(root_identity);

        let mut wrong_journal_root = raw.clone();
        let Observation::Present(service_data) =
            &mut wrong_journal_root.service_install_residue.service_data
        else {
            panic!("service data");
        };
        let Observation::Present(journal) = &mut service_data.upgrade_transaction_journal else {
            panic!("journal");
        };
        journal.path = raw_path(&logical_path(
            LogicalRoot::Install,
            "installer-upgrade.v1.json",
        ));
        assert_rejected(wrong_journal_root);

        let mut mixed_case_journal = raw.clone();
        let Observation::Present(service_data) =
            &mut mixed_case_journal.service_install_residue.service_data
        else {
            panic!("service data");
        };
        let Observation::Present(journal) = &mut service_data.upgrade_transaction_journal else {
            panic!("journal");
        };
        journal.path = raw_path(&logical_path(
            LogicalRoot::ServiceData,
            "INSTALLER-UPGRADE.V1.JSON",
        ));
        assert_rejected(mixed_case_journal);

        let mut classifier_spoof = raw.clone();
        let Observation::Present(install) = &mut classifier_spoof.service_install_residue.install
        else {
            panic!("install residue");
        };
        install.staged_service_images[0].path = raw_path(&logical_path(
            LogicalRoot::Install,
            "batcave-collector-service..staged.exe",
        ));
        assert_rejected(classifier_spoof);

        let mut marker_unknown = raw.clone();
        let Observation::Present(install) = &mut marker_unknown.service_install_residue.install
        else {
            panic!("install residue");
        };
        install.rollback_execution_marker = Observation::Unknown("marker_unknown".to_string());
        assert_rejected(marker_unknown);

        let mut mixed_case_marker = raw.clone();
        let Observation::Present(install) = &mut mixed_case_marker.service_install_residue.install
        else {
            panic!("install residue");
        };
        let Observation::Present(marker) = &mut install.rollback_execution_marker else {
            panic!("marker");
        };
        marker.path = raw_path(&logical_path(
            LogicalRoot::Install,
            "BATCAVE-ROLLBACK-FIXTURE-RAN.V1",
        ));
        assert_rejected(mixed_case_marker);

        let mut wrong_volume = raw.clone();
        let Observation::Present(install) = &mut wrong_volume.service_install_residue.install
        else {
            panic!("install residue");
        };
        install.staged_service_images[0].volume_serial += 1;
        assert_eq!(
            compare_machine_projection(&wrong_volume, &machine, &plan, &parent_current_user()),
            Err("lifecycle_private_projection_residue_identity_invalid".to_string())
        );

        let mut cross_root_identity_reuse = raw.clone();
        let journal_identity = {
            let Observation::Present(service_data) = &cross_root_identity_reuse
                .service_install_residue
                .service_data
            else {
                panic!("service data");
            };
            let Observation::Present(journal) = &service_data.upgrade_transaction_journal else {
                panic!("journal");
            };
            (journal.volume_serial, journal.file_index)
        };
        let Observation::Present(install) =
            &mut cross_root_identity_reuse.service_install_residue.install
        else {
            panic!("install residue");
        };
        install.staged_service_images[0].volume_serial = journal_identity.0;
        install.staged_service_images[0].file_index = journal_identity.1;
        assert_eq!(
            compare_machine_projection(
                &cross_root_identity_reuse,
                &machine,
                &plan,
                &parent_current_user(),
            ),
            Err("lifecycle_private_projection_residue_identity_invalid".to_string())
        );

        let mut oversize_journal = raw.clone();
        let Observation::Present(service_data) =
            &mut oversize_journal.service_install_residue.service_data
        else {
            panic!("service data");
        };
        let Observation::Present(journal) = &mut service_data.upgrade_transaction_journal else {
            panic!("journal");
        };
        journal.size = 16 * 1024 + 1;
        assert_eq!(
            compare_machine_projection(&oversize_journal, &machine, &plan, &parent_current_user(),),
            Err("lifecycle_private_projection_residue_file_size_invalid".to_string())
        );

        let mut oversize_marker = raw.clone();
        let Observation::Present(install) = &mut oversize_marker.service_install_residue.install
        else {
            panic!("install residue");
        };
        let Observation::Present(marker) = &mut install.rollback_execution_marker else {
            panic!("marker");
        };
        marker.size = 1025;
        assert_eq!(
            compare_machine_projection(&oversize_marker, &machine, &plan, &parent_current_user()),
            Err("lifecycle_private_projection_residue_file_size_invalid".to_string())
        );

        let staged = |index: usize, size: u64| ResidueFileForProof {
            path: format!(
                r"C:\Program Files\BatCave Monitor\batcave-collector-service.0.2.{index}.staged.exe"
            ),
            size,
            sha256: format!("{:064x}", index + 1),
            volume_serial: 1,
            file_index: 100 + index as u64,
        };
        let mut too_many = raw.clone();
        let Observation::Present(install) = &mut too_many.service_install_residue.install else {
            panic!("install residue");
        };
        install.staged_service_images = (0..65).map(|index| staged(index, 1)).collect();
        install.rollback_service_images.clear();
        install.atomic_temporary_files.clear();
        install.rollback_execution_marker = Observation::Absent;
        assert_eq!(
            compare_machine_projection(&too_many, &machine, &plan, &parent_current_user()),
            Err("lifecycle_private_projection_residue_total_size_invalid".to_string())
        );

        let mut aggregate_overflow = raw.clone();
        let Observation::Present(install) = &mut aggregate_overflow.service_install_residue.install
        else {
            panic!("install residue");
        };
        install.staged_service_images = (0..5)
            .map(|index| staged(index, 64 * 1024 * 1024))
            .collect();
        install.rollback_service_images.clear();
        install.atomic_temporary_files.clear();
        install.rollback_execution_marker = Observation::Absent;
        assert_eq!(
            compare_machine_projection(
                &aggregate_overflow,
                &machine,
                &plan,
                &parent_current_user(),
            ),
            Err("lifecycle_private_projection_residue_total_size_invalid".to_string())
        );

        let mut unknown_nested = serde_json::to_value(&raw).expect("raw machine");
        unknown_nested["service_install_residue"]
            .as_object_mut()
            .expect("residue object")
            .insert("unexpected".to_string(), serde_json::json!(true));
        assert!(serde_json::from_value::<ElevatedMachineSnapshot>(unknown_nested).is_err());
    }

    #[test]
    fn raw_machine_registration_projects_exact_v2_semantics_and_rejects_hostile_drift() {
        let plan = parse_plan().expect("plan");
        let receipts = success_receipts();
        let export = valid_export(&plan, &receipts);
        let machine = export
            .private_evidence
            .iter()
            .find(|entry| entry.receipt.name == "baseline-install-state.private.json")
            .expect("baseline entry")
            .machine
            .clone();
        let raw = raw_machine(&machine);
        let assert_rejected = |raw: ElevatedMachineSnapshot| {
            assert!(
                compare_machine_projection(&raw, &machine, &plan, &parent_current_user()).is_err()
            );
        };
        assert_eq!(
            compare_machine_projection(&raw, &machine, &plan, &parent_current_user()),
            Ok(())
        );
        assert_eq!(SANITIZED_SCHEMA, "batcave_windows_lifecycle_sanitized_v2");

        let final_machine = export
            .private_evidence
            .iter()
            .find(|entry| entry.receipt.name == "final-repair-state.private.json")
            .expect("final repair entry")
            .machine
            .clone();
        let final_raw = raw_machine(&final_machine);
        assert!(matches!(
            final_raw.machine_registration.public_desktop_shortcut,
            Observation::Absent
        ));
        assert!(matches!(
            final_raw.machine_registration.common_start_menu_shortcut,
            Observation::Absent
        ));
        assert_eq!(
            compare_machine_projection(&final_raw, &final_machine, &plan, &parent_current_user(),),
            Ok(())
        );

        let mut unknown_product = raw.clone();
        unknown_product.machine_registration.product_key_64 =
            Observation::Unknown("product_unknown".to_string());
        assert_rejected(unknown_product);

        let mut unexpected_32_view = raw.clone();
        unexpected_32_view.machine_registration.product_key_32 = unexpected_32_view
            .machine_registration
            .product_key_64
            .clone();
        assert_rejected(unexpected_32_view);

        for mutation in 0..7 {
            let mut hostile = raw.clone();
            let Observation::Present(product) = &mut hostile.machine_registration.product_key_64
            else {
                panic!("product key");
            };
            match mutation {
                0 => product.final_key_path.push_str("\\spoof"),
                1 => product.install_root.push_str("\\spoof"),
                2 => product.value_names.push("named".to_string()),
                3 => product.subkey_names.push("child".to_string()),
                4 => product.last_write_time_100ns = 0,
                5 => product.dacl_sha256 = "not-a-digest".to_string(),
                6 => product.owner = SecurityPrincipalForProof::InteractiveUsers,
                _ => unreachable!(),
            }
            assert_rejected(hostile);
        }

        let mut unknown_public = raw.clone();
        unknown_public.machine_registration.public_desktop_shortcut =
            Observation::Unknown("public_unknown".to_string());
        assert_rejected(unknown_public);

        let mut unknown_common = raw.clone();
        unknown_common
            .machine_registration
            .common_start_menu_shortcut = Observation::Unknown("common_unknown".to_string());
        assert_rejected(unknown_common);

        for mutation in 0..11 {
            let mut hostile = raw.clone();
            let Observation::Present(shortcut) =
                &mut hostile.machine_registration.public_desktop_shortcut
            else {
                panic!("public shortcut");
            };
            match mutation {
                0 => shortcut.path.push_str(".spoof"),
                1 => shortcut.target.push_str(".spoof"),
                2 => shortcut.arguments = "--spoof".to_string(),
                3 => shortcut.icon_path = r"C:\spoof.ico".to_string(),
                4 => shortcut.icon_index = 1,
                5 => shortcut.working_directory.push_str("\\spoof"),
                6 => shortcut.show_command = 0,
                7 => shortcut.hotkey = 1,
                8 => shortcut.description = "spoof".to_string(),
                9 => shortcut.app_user_model_id = "spoof".to_string(),
                10 => shortcut.dacl_sha256 = "not-a-digest".to_string(),
                _ => unreachable!(),
            }
            assert_rejected(hostile);
        }

        let mut common_untrusted_owner = raw.clone();
        let Observation::Present(shortcut) = &mut common_untrusted_owner
            .machine_registration
            .common_start_menu_shortcut
        else {
            panic!("common shortcut");
        };
        shortcut.owner = SecurityPrincipalForProof::InteractiveUsers;
        assert_rejected(common_untrusted_owner);

        let mut shortcut_collision = raw.clone();
        let public_identity = {
            let Observation::Present(shortcut) = &shortcut_collision
                .machine_registration
                .public_desktop_shortcut
            else {
                panic!("public shortcut");
            };
            (shortcut.volume_serial, shortcut.file_index)
        };
        let Observation::Present(shortcut) = &mut shortcut_collision
            .machine_registration
            .common_start_menu_shortcut
        else {
            panic!("common shortcut");
        };
        shortcut.volume_serial = public_identity.0;
        shortcut.file_index = public_identity.1;
        assert_eq!(
            compare_machine_projection(
                &shortcut_collision,
                &machine,
                &plan,
                &parent_current_user(),
            ),
            Err("lifecycle_private_projection_registration_identity_invalid".to_string())
        );

        let mut binary_collision = raw.clone();
        let monitor_identity = binary_collision
            .machine
            .monitor
            .as_present()
            .expect("monitor")
            .identity;
        let Observation::Present(shortcut) = &mut binary_collision
            .machine_registration
            .public_desktop_shortcut
        else {
            panic!("public shortcut");
        };
        shortcut.volume_serial = monitor_identity.volume_serial;
        shortcut.file_index = monitor_identity.file_index;
        assert_eq!(
            compare_machine_projection(&binary_collision, &machine, &plan, &parent_current_user()),
            Err("lifecycle_private_projection_registration_identity_invalid".to_string())
        );

        let mut unknown_nested = serde_json::to_value(&raw).expect("raw machine");
        unknown_nested["machine_registration"]
            .as_object_mut()
            .expect("registration object")
            .insert("unexpected".to_string(), serde_json::json!(true));
        assert!(serde_json::from_value::<ElevatedMachineSnapshot>(unknown_nested).is_err());
    }

    #[test]
    fn service_failure_projection_uses_the_observer_utf16_bound() {
        let valid = ServiceRegistryKeyForProof {
            last_failure: Observation::Present("é".repeat(32_768)),
        };
        assert!(project_service_registry_key(&Observation::Present(valid)).is_ok());
        let invalid = ServiceRegistryKeyForProof {
            last_failure: Observation::Present("é".repeat(32_769)),
        };
        assert_eq!(
            project_service_registry_key(&Observation::Present(invalid)),
            Err("lifecycle_private_projection_service_failure_invalid".to_string())
        );
    }

    #[test]
    fn sanitized_named_pipe_keeps_v2_shape_without_process_start_time() {
        assert_eq!(SANITIZED_SCHEMA, "batcave_windows_lifecycle_sanitized_v2");
        let pipe = SanitizedNamedPipeSnapshot {
            server_process_id: 41,
        };
        assert_eq!(
            serde_json::to_value(&pipe).expect("pipe serializes"),
            serde_json::json!({ "server_process_id": 41 })
        );
        assert!(
            serde_json::from_value::<SanitizedNamedPipeSnapshot>(serde_json::json!({
                "server_process_id": 41,
                "server_process_started_at_100ns": 4100
            }))
            .is_err()
        );
    }

    #[test]
    fn raw_runtime_authority_projects_running_and_stopped_snapshots() {
        let plan = parse_plan().expect("plan");
        let receipts = success_receipts();
        let export = valid_export(&plan, &receipts);
        for name in [
            "final-repair-state.private.json",
            "final-stopped-service-state.private.json",
        ] {
            let entry = export
                .private_evidence
                .iter()
                .find(|entry| entry.receipt.name == name)
                .expect("machine entry");
            assert_eq!(
                compare_machine_projection(
                    &raw_machine(&entry.machine),
                    &entry.machine,
                    &plan,
                    &parent_current_user(),
                ),
                Ok(()),
                "{name}"
            );
        }
    }

    #[test]
    fn restoration_authority_requires_exact_runtime_residue_and_registration_profiles() {
        let plan = parse_plan().expect("plan");
        let baseline = raw_machine(&installed_machine(
            baseline_artifacts(&plan),
            ServiceExpectation::Running,
            false,
        ));
        assert!(validate_restoration_machine_authority(
            &baseline,
            RestorationAuthorityExpectation::BaselineRunning,
        )
        .is_ok());
        assert!(validate_restoration_machine_authority(
            &baseline,
            RestorationAuthorityExpectation::FinalRunning,
        )
        .is_err());

        let final_running = raw_machine(&installed_machine(
            final_artifacts(&plan),
            ServiceExpectation::Running,
            true,
        ));
        assert!(validate_restoration_machine_authority(
            &final_running,
            RestorationAuthorityExpectation::FinalRunning,
        )
        .is_ok());

        let allowlisted_stopped = raw_machine(&installed_machine(
            allowlisted_artifacts(&plan),
            ServiceExpectation::Stopped {
                win32_exit_code: plan.allowlisted_start.win32_exit_code,
                service_specific_exit_code: plan.allowlisted_start.service_specific_exit_code,
            },
            false,
        ));
        assert!(validate_restoration_machine_authority(
            &allowlisted_stopped,
            RestorationAuthorityExpectation::AllowlistedStopped,
        )
        .is_ok());

        let absent = raw_machine(&absent_machine(true));
        assert!(validate_restoration_machine_authority(
            &absent,
            RestorationAuthorityExpectation::ProductAbsent,
        )
        .is_ok());

        let mut unknown_runtime = final_running.clone();
        unknown_runtime.named_pipe = Observation::Unknown("pipe_unknown".to_string());
        assert!(validate_restoration_machine_authority(
            &unknown_runtime,
            RestorationAuthorityExpectation::FinalRunning,
        )
        .is_err());

        let mut released_lock = baseline.clone();
        released_lock.etw_owner_lock = RuntimeLockObservation::Released {};
        assert!(validate_restoration_machine_authority(
            &released_lock,
            RestorationAuthorityExpectation::BaselineRunning,
        )
        .is_err());

        let mut etw_loss = baseline.clone();
        let Observation::Present(session) = &mut etw_loss.etw_session else {
            panic!("ETW session");
        };
        session.events_lost = 1;
        assert!(validate_restoration_machine_authority(
            &etw_loss,
            RestorationAuthorityExpectation::BaselineRunning,
        )
        .is_err());

        let mut generation_drift = baseline.clone();
        let Observation::Present(lease) = &mut generation_drift.etw_lease else {
            panic!("ETW lease");
        };
        lease.service_generation[0] ^= 1;
        assert!(validate_restoration_machine_authority(
            &generation_drift,
            RestorationAuthorityExpectation::BaselineRunning,
        )
        .is_err());

        let mut invalid_boundaries = baseline.clone();
        let Observation::Present(boundaries) = &mut invalid_boundaries.installed_boundaries else {
            panic!("installed boundaries");
        };
        boundaries.service_aces.clear();
        assert!(validate_restoration_machine_authority(
            &invalid_boundaries,
            RestorationAuthorityExpectation::BaselineRunning,
        )
        .is_err());

        let mut stale_registration = final_running;
        stale_registration
            .machine_registration
            .public_desktop_shortcut = baseline.machine_registration.public_desktop_shortcut;
        assert!(validate_restoration_machine_authority(
            &stale_registration,
            RestorationAuthorityExpectation::FinalRunning,
        )
        .is_err());
    }

    #[test]
    fn restoration_packet_is_typed_bound_and_fail_closed() {
        let plan = parse_plan().expect("plan");
        let mut machine = raw_machine(&installed_machine(
            final_artifacts(&plan),
            ServiceExpectation::Running,
            true,
        ));
        machine.machine.product_processes = Observation::Present(vec![ProcessSnapshot {
            process_id: 55,
            parent_process_id: 0,
            executable_name: "batcave-collector-service.exe".to_string(),
            executable_path: Some(
                r"C:\Program Files\BatCave Monitor\batcave-collector-service.exe".to_string(),
            ),
        }]);
        let restoration = crate::windows_lifecycle_proof_contract::RestorationOutcome::Restored {
            evidence: EvidenceReceipt {
                name: "final-repair-restoration.private.json".to_string(),
                size: 1,
                sha256: "a".repeat(64),
            },
        };
        assert_eq!(
            super::super::lifecycle::validate_restoration_target(
                LifecycleStage::FinalRepair,
                &machine,
                &plan,
            ),
            Ok(())
        );
        let packet = serde_json::json!({
            "schema_version": "batcave.windows-lifecycle.restoration.v1",
            "stage": LifecycleStage::FinalRepair,
            "target_stage": LifecycleStage::FinalRepair,
            "method": "observed_target_state",
            "restored": true,
            "reason": null,
            "machine_after_attempt": machine,
        });
        let validate = |packet: &serde_json::Value| {
            super::super::lifecycle::validate_stage_restoration_packet(
                &serde_json::to_vec(packet).expect("packet"),
                LifecycleStage::FinalRepair,
                LifecycleStage::FinalRepair,
                false,
                &restoration,
                &plan,
            )
        };
        assert_eq!(validate(&packet), Ok(()));

        let mut wrong_method = packet.clone();
        wrong_method["method"] = serde_json::json!("provisioner_transaction");
        assert_eq!(
            validate(&wrong_method),
            Err("lifecycle_stage_restoration_packet_binding_invalid".to_string())
        );

        let mut false_claim = packet.clone();
        false_claim["restored"] = serde_json::json!(false);
        false_claim["reason"] = serde_json::json!("hostile_claim");
        assert_eq!(
            validate(&false_claim),
            Err("lifecycle_stage_restoration_packet_claim_invalid".to_string())
        );

        let mut untrusted_snapshot = packet.clone();
        untrusted_snapshot["machine_after_attempt"]["named_pipe"] =
            serde_json::json!({ "state": "absent" });
        assert_eq!(
            validate(&untrusted_snapshot),
            Err("lifecycle_stage_restoration_packet_claim_invalid".to_string())
        );

        let mut unknown_field = packet;
        unknown_field["unexpected"] = serde_json::json!(true);
        assert_eq!(
            validate(&unknown_field),
            Err("lifecycle_stage_restoration_packet_invalid".to_string())
        );
    }

    #[test]
    fn raw_runtime_authority_rejects_identity_presence_loss_and_unknown_drift() {
        let plan = parse_plan().expect("plan");
        let receipts = success_receipts();
        let export = valid_export(&plan, &receipts);
        let machine = &export
            .private_evidence
            .iter()
            .find(|entry| entry.receipt.name == "final-repair-state.private.json")
            .expect("running entry")
            .machine;
        let assert_rejected = |raw: ElevatedMachineSnapshot| {
            assert!(
                compare_machine_projection(&raw, machine, &plan, &parent_current_user(),).is_err()
            );
        };

        let mut pipe_pid = raw_machine(machine);
        let Observation::Present(pipe) = &mut pipe_pid.named_pipe else {
            panic!("pipe");
        };
        pipe.server_process_id += 1;
        assert_rejected(pipe_pid);

        let mut pipe_start = raw_machine(machine);
        let Observation::Present(pipe) = &mut pipe_start.named_pipe else {
            panic!("pipe");
        };
        pipe.server_process_started_at_100ns += 1;
        assert_rejected(pipe_start);

        let mut lease_controller = raw_machine(machine);
        let Observation::Present(lease) = &mut lease_controller.etw_lease else {
            panic!("lease");
        };
        lease.controller.process_started_at += 1;
        assert_rejected(lease_controller);

        let mut session_identity = raw_machine(machine);
        let Observation::Present(session) = &mut session_identity.etw_session else {
            panic!("session");
        };
        session.identity.configuration_digest[0] ^= 1;
        assert_rejected(session_identity);

        let mut missing_session = raw_machine(machine);
        missing_session.etw_session = Observation::Absent;
        assert_rejected(missing_session);

        let mut loss = raw_machine(machine);
        let Observation::Present(session) = &mut loss.etw_session else {
            panic!("session");
        };
        session.events_lost = 1;
        assert_rejected(loss);

        let mut finite_log_loss = raw_machine(machine);
        let Observation::Present(session) = &mut finite_log_loss.etw_session else {
            panic!("session");
        };
        session.log_buffers_lost = 1;
        assert_rejected(finite_log_loss);

        let mut finite_realtime_loss = raw_machine(machine);
        let Observation::Present(session) = &mut finite_realtime_loss.etw_session else {
            panic!("session");
        };
        session.realtime_buffers_lost = 1;
        assert_rejected(finite_realtime_loss);

        let mut overflow = raw_machine(machine);
        let Observation::Present(session) = &mut overflow.etw_session else {
            panic!("session");
        };
        session.log_buffers_lost = u64::MAX;
        session.realtime_buffers_lost = 1;
        assert_eq!(
            compare_machine_projection(&overflow, machine, &plan, &parent_current_user()),
            Err("lifecycle_private_projection_etw_loss_overflow".to_string())
        );

        for unknown in [0, 1, 2, 3, 4] {
            let mut raw = raw_machine(machine);
            match unknown {
                0 => raw.named_pipe = Observation::Unknown("pipe_unknown".to_string()),
                1 => raw.etw_lease = Observation::Unknown("lease_unknown".to_string()),
                2 => raw.etw_session = Observation::Unknown("session_unknown".to_string()),
                3 => {
                    raw.etw_owner_lock = RuntimeLockObservation::Unknown {
                        reason: "lock_unknown".to_string(),
                    }
                }
                4 => {
                    raw.service_lifecycle_lock = RuntimeLockObservation::Unknown {
                        reason: "lifecycle_lock_unknown".to_string(),
                    }
                }
                _ => unreachable!(),
            }
            assert_rejected(raw);
        }

        let mut sanitized_loss = machine.clone();
        sanitized_loss
            .etw_session
            .as_mut()
            .expect("sanitized ETW")
            .events_lost = 1;
        assert_eq!(
            validate_machine_snapshot(&sanitized_loss),
            Err("lifecycle_sanitized_etw_invalid".to_string())
        );
    }

    #[test]
    fn runtime_lock_projection_distinguishes_held_released_absent_and_unknown() {
        let path = service_data_path(crate::collector_service::etw_lease::ETW_OWNER_LOCK_FILE_NAME);
        for observation in [
            RuntimeLockObservation::Held {},
            RuntimeLockObservation::Released {},
        ] {
            assert_eq!(
                project_runtime_lock(
                    &observation,
                    crate::collector_service::etw_lease::ETW_OWNER_LOCK_FILE_NAME,
                ),
                Ok(Some(path.clone()))
            );
        }
        assert_eq!(
            project_runtime_lock(
                &RuntimeLockObservation::Absent {},
                crate::collector_service::etw_lease::ETW_OWNER_LOCK_FILE_NAME,
            ),
            Ok(None)
        );
        assert!(project_runtime_lock(
            &RuntimeLockObservation::Unknown {
                reason: "lock_unknown".to_string(),
            },
            crate::collector_service::etw_lease::ETW_OWNER_LOCK_FILE_NAME,
        )
        .is_err());
    }

    #[test]
    fn verified_projection_rejects_worker_sanitized_drift_and_swapped_machine_payloads() {
        let plan = parse_plan().expect("plan");
        let receipts = success_receipts();
        let original = valid_export(&plan, &receipts);
        let initial = original
            .private_evidence
            .iter()
            .find(|entry| entry.receipt.name == "initial-state.private.json")
            .expect("initial entry");
        let raw = raw_machine(&initial.machine);

        let mut drifted = original.clone();
        let drifted_initial = drifted
            .private_evidence
            .iter_mut()
            .find(|entry| entry.receipt.name == "initial-state.private.json")
            .expect("drifted initial");
        let Observation::Present(monitor) = &mut drifted_initial.machine.monitor else {
            panic!("initial monitor");
        };
        monitor.sha256 = "f".repeat(64);
        assert_eq!(
            compare_verified_projection(
                &drifted,
                &[(
                    &initial.receipt,
                    packet_for_test(PrivateSuccessPayload::Machine(raw.clone())),
                )],
                &plan,
                &parent_desktop_results(&plan),
                parent_current_user_projection(&original),
            ),
            Err("lifecycle_private_projection_machine_drift".to_string())
        );

        let final_repair = original
            .private_evidence
            .iter()
            .find(|entry| entry.receipt.name == "final-repair-state.private.json")
            .expect("final repair");
        assert!(compare_verified_projection(
            &original,
            &[(
                &final_repair.receipt,
                packet_for_test(PrivateSuccessPayload::Machine(raw)),
            )],
            &plan,
            &parent_desktop_results(&plan),
            parent_current_user_projection(&original),
        )
        .is_err());
    }

    #[test]
    fn present_service_requires_exact_raw_binary_and_boundary_authority() {
        let plan = parse_plan().expect("plan");
        let receipts = success_receipts();
        let export = valid_export(&plan, &receipts);
        let entry = export
            .private_evidence
            .iter()
            .find(|entry| entry.receipt.name == "final-repair-state.private.json")
            .expect("running service entry");
        let raw = raw_machine(&entry.machine);

        for missing_binary in [
            Observation::Absent,
            Observation::Unknown("lifecycle_service_binary_access_denied".to_string()),
        ] {
            let mut hostile = raw.clone();
            hostile.machine.service_binary = missing_binary;
            assert_eq!(
                compare_machine_projection(&hostile, &entry.machine, &plan, &parent_current_user(),),
                Err("lifecycle_private_projection_service_file_missing".to_string())
            );
        }

        for missing_boundaries in [
            Observation::Absent,
            Observation::Unknown("lifecycle_service_boundary_access_denied".to_string()),
        ] {
            let mut hostile = raw.clone();
            hostile.installed_boundaries = missing_boundaries;
            assert_eq!(
                compare_machine_projection(&hostile, &entry.machine, &plan, &parent_current_user(),),
                Err("lifecycle_private_projection_service_boundary_missing".to_string())
            );
        }
    }

    #[test]
    fn verified_projection_rejects_parent_desktop_and_crash_event_drift() {
        let plan = parse_plan().expect("plan");
        let receipts = success_receipts();
        let export = valid_export(&plan, &receipts);
        let mut parent = parent_desktop_results(&plan);
        let final_primary = export
            .private_evidence
            .iter()
            .find(|entry| entry.receipt.name == "final-primary-desktop.private.json")
            .expect("final primary");
        let result = parent
            .iter()
            .find(|result| result.phase == DesktopPhase::FinalPrimary)
            .expect("parent final primary")
            .clone();
        parent[0]
            .observation
            .as_mut()
            .expect("parent observation")
            .desktop
            .process_id += 1;
        assert_eq!(
            compare_verified_projection(
                &export,
                &[(
                    &final_primary.receipt,
                    packet_for_test(PrivateSuccessPayload::Desktop(Box::new(
                        PrivateDesktopPayload {
                            machine: raw_machine(&final_primary.machine),
                            result,
                        },
                    ))),
                )],
                &plan,
                &parent,
                parent_current_user_projection(&export),
            ),
            Err("lifecycle_private_projection_parent_desktop_drift".to_string())
        );

        let crashed = export
            .private_evidence
            .iter()
            .find(|entry| entry.receipt.name == "baseline-crashed-state.private.json")
            .expect("baseline crash");
        let Some(SanitizedStageEvent::ServiceCrash(event)) = crashed.event.as_ref() else {
            panic!("crash event");
        };
        let mut termination = raw_termination(event);
        termination.target.process_id += 1;
        assert_eq!(
            compare_verified_projection(
                &export,
                &[(
                    &crashed.receipt,
                    packet_for_test(PrivateSuccessPayload::ServiceCrash(
                        PrivateServiceCrashPayload {
                            machine: raw_machine(&crashed.machine),
                            termination,
                        },
                    )),
                )],
                &plan,
                &parent_desktop_results(&plan),
                parent_current_user_projection(&export),
            ),
            Err("lifecycle_private_projection_event_drift".to_string())
        );

        let rollback = export
            .private_evidence
            .iter()
            .find(|entry| entry.receipt.name == "baseline-rollback-recovery-state.private.json")
            .expect("rollback");
        let Some(SanitizedStageEvent::UpgradeRollback(event)) = rollback.event.as_ref() else {
            panic!("rollback event");
        };
        let mut raw_rollback = FailedUpgradeRollbackForProof {
            candidate_sha256: event.candidate_sha256.clone(),
            candidate_failure_code: event.candidate_failure_code.clone(),
            candidate_failure_detail: event.candidate_failure_detail.clone(),
            execution_marker_sha256: event.execution_marker_sha256.clone(),
            restored_sha256: event.restored_sha256.clone(),
            restored_process_id: event.restored_process_id,
        };
        raw_rollback.restored_process_id += 1;
        assert_eq!(
            compare_verified_projection(
                &export,
                &[(
                    &rollback.receipt,
                    packet_for_test(PrivateSuccessPayload::UpgradeRollback(
                        PrivateUpgradeRollbackPayload {
                            machine: raw_machine(&rollback.machine),
                            rollback: raw_rollback,
                        },
                    )),
                )],
                &plan,
                &parent_desktop_results(&plan),
                parent_current_user_projection(&export),
            ),
            Err("lifecycle_private_projection_event_drift".to_string())
        );
    }

    #[test]
    fn worker_pipe_and_etw_claims_must_match_raw_runtime_authority() {
        let plan = parse_plan().expect("plan");
        let receipts = success_receipts();
        let mut export = valid_export(&plan, &receipts);
        let entry = export
            .private_evidence
            .iter_mut()
            .find(|entry| entry.receipt.name == "initial-state.private.json")
            .expect("initial entry");
        let raw = raw_machine(&entry.machine);
        entry.machine.named_pipe = Some(SanitizedNamedPipeSnapshot {
            server_process_id: 999,
        });
        entry.machine.etw_session = Some(healthy_etw(
            &plan.final_candidate.service_sha256,
            &test_service_instance_id(999, 999),
        ));
        export.current_user_retention.settings.after_uninstall =
            Observation::Present(SanitizedDigestSnapshot {
                size: 99,
                sha256: "f".repeat(64),
            });

        assert_eq!(
            compare_machine_projection(&raw, &entry.machine, &plan, &parent_current_user()),
            Err("lifecycle_private_projection_machine_drift".to_string())
        );
        assert_eq!(
            compare_parent_current_user_retention(
                &export.current_user_retention,
                &parent_user_objects(),
                &parent_user_objects(),
            ),
            Err("lifecycle_parent_user_retention_drift".to_string())
        );
    }

    #[test]
    fn projection_rejects_hostile_private_paths_and_export_leakage() {
        let plan = parse_plan().expect("plan");
        let receipts = success_receipts();
        let export = valid_export(&plan, &receipts);
        let initial = export
            .private_evidence
            .iter()
            .find(|entry| entry.receipt.name == "initial-state.private.json")
            .expect("initial entry");
        for hostile in [
            r"C:\Users\albert\private.txt",
            r"\\server\share\BatCave",
            r"C:\Program Files\BatCave Monitor\..\secret",
            r"C:\Program Files\BatCave Monitor\file.exe:stream",
            r"C:\Program Files\BatCave Monitor\NUL",
            r"C:\ProgramData\BatCaveLifecycleProof-v1-secret",
        ] {
            let mut raw = raw_machine(&initial.machine);
            raw.machine.install_root = Observation::Present(DirectorySnapshot {
                identity: super::super::native::FileIdentity {
                    volume_serial: 1,
                    file_index: 2,
                },
                final_path: hostile.to_string(),
            });
            assert!(
                compare_machine_projection(&raw, &initial.machine, &plan, &parent_current_user(),)
                    .is_err(),
                "{hostile}"
            );
        }

        for leaked in [
            br#"{"path":"C:\\Users\\albert\\private.txt"}"#.as_slice(),
            br#"{"path":"C:/Users/albert/private.txt"}"#.as_slice(),
            br#"{"path":"\\\\server\\share\\private.txt"}"#.as_slice(),
            br#"{"root":"BatCaveLifecycleProof-v1-secret"}"#.as_slice(),
        ] {
            assert_eq!(
                reject_private_path_leakage(leaked),
                Err("lifecycle_private_projection_path_leak".to_string())
            );
        }
    }

    #[test]
    fn projection_manifest_rejects_missing_extra_duplicate_tampered_and_reused_files() {
        let receipts = success_receipts();
        let identities = (0..receipts.len())
            .map(|index| super::super::native::FileIdentity {
                volume_serial: 1,
                file_index: index as u64 + 1,
            })
            .collect::<Vec<_>>();
        let entries = receipts
            .iter()
            .zip(&identities)
            .map(|(receipt, identity)| (receipt, *identity))
            .collect::<Vec<_>>();
        assert_eq!(validate_projection_manifest_parts(&entries), Ok(()));
        assert!(validate_projection_manifest_parts(&entries[..entries.len() - 1]).is_err());

        let mut duplicate_receipts = receipts.clone();
        duplicate_receipts[1] = duplicate_receipts[0].clone();
        let duplicate_entries = duplicate_receipts
            .iter()
            .zip(&identities)
            .map(|(receipt, identity)| (receipt, *identity))
            .collect::<Vec<_>>();
        assert!(validate_projection_manifest_parts(&duplicate_entries).is_err());

        let mut reused = entries.clone();
        reused[1].1 = reused[0].1;
        assert!(validate_projection_manifest_parts(&reused).is_err());

        let mut reordered = entries.clone();
        reordered.swap(0, 1);
        assert!(validate_projection_manifest_parts(&reordered).is_err());

        let mut tampered_receipts = receipts.clone();
        tampered_receipts[0].sha256 = "not-a-digest".to_string();
        let tampered_entries = tampered_receipts
            .iter()
            .zip(&identities)
            .map(|(receipt, identity)| (receipt, *identity))
            .collect::<Vec<_>>();
        assert!(validate_projection_manifest_parts(&tampered_entries).is_err());

        let mut extra = entries.clone();
        extra.push((&receipts[0], identities[0]));
        assert!(validate_projection_manifest_parts(&extra).is_err());
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
    fn sanitized_export_enforces_shared_shortcut_timeline() {
        let plan = parse_plan().expect("plan");
        let receipts = success_receipts();
        let packet = valid_export(&plan, &receipts);
        let validate = |packet: &SanitizedExportPacket| {
            validate_sanitized_export_bytes(
                &serde_json::to_vec(packet).expect("packet"),
                &plan,
                &"c".repeat(40),
                &"d".repeat(64),
                &receipts,
            )
        };

        assert_eq!(validate(&packet), Ok(()));
        for (index, entry) in packet.private_evidence.iter().enumerate() {
            let historical_shortcuts_required = entry.receipt.name == "initial-state.private.json"
                || entry.receipt.name.starts_with("baseline-")
                || entry.receipt.name == "legacy-residue-seeded-state.private.json";
            assert_eq!(
                entry.machine.public_desktop_shortcut.is_some()
                    && entry.machine.common_start_menu_shortcut.is_some(),
                historical_shortcuts_required,
                "{} shortcut timeline",
                entry.receipt.name
            );

            for (location, replacement) in [
                (
                    "public desktop",
                    if historical_shortcuts_required {
                        None
                    } else {
                        Some(shortcut(LogicalRoot::PublicDesktop))
                    },
                ),
                (
                    "common start menu",
                    if historical_shortcuts_required {
                        None
                    } else {
                        Some(shortcut(LogicalRoot::CommonStartMenu))
                    },
                ),
            ] {
                let mut wrong = packet.clone();
                if location == "public desktop" {
                    wrong.private_evidence[index]
                        .machine
                        .public_desktop_shortcut = replacement;
                } else {
                    wrong.private_evidence[index]
                        .machine
                        .common_start_menu_shortcut = replacement;
                }
                assert!(
                    validate(&wrong).is_err(),
                    "{} {location}",
                    entry.receipt.name
                );
            }
        }
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

        for (name, owner_lock) in [
            ("final-stopped-service-state.private.json", true),
            ("baseline-crashed-state.private.json", false),
        ] {
            let mut runtime_lock_residue = valid_export(&plan, &receipts);
            let machine = &mut runtime_lock_residue
                .private_evidence
                .iter_mut()
                .find(|entry| entry.receipt.name == name)
                .expect("stopped or crashed state")
                .machine;
            if owner_lock {
                machine.etw_owner_lock = Some(service_data_path(
                    crate::collector_service::etw_lease::ETW_OWNER_LOCK_FILE_NAME,
                ));
            } else {
                machine.service_lifecycle_lock =
                    Some(service_data_path(SERVICE_LIFECYCLE_LOCK_FILE_NAME));
            }
            assert!(validate_sanitized_export_bytes(
                &serde_json::to_vec(&runtime_lock_residue).expect("runtime lock residue"),
                &plan,
                &"c".repeat(40),
                &"d".repeat(64),
                &receipts,
            )
            .is_err());
        }

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
            .find(|entry| entry.receipt.name == "final-crash-recovery-state.private.json")
            .expect("non-adjacent final recovery")
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

        let mut reused_install_epoch = valid_export(&plan, &receipts);
        let pre_uninstall_install_id = reused_install_epoch
            .private_evidence
            .iter()
            .find(|entry| entry.receipt.name == "final-repair-state.private.json")
            .expect("pre-uninstall state")
            .machine
            .etw_session
            .as_ref()
            .expect("pre-uninstall ETW")
            .lease
            .install_id;
        let mut post_uninstall_epoch = false;
        for entry in &mut reused_install_epoch.private_evidence {
            if entry.receipt.name == "initial-uninstall-state.private.json" {
                post_uninstall_epoch = true;
                continue;
            }
            if post_uninstall_epoch {
                if let Some(etw) = &mut entry.machine.etw_session {
                    etw.lease.install_id = pre_uninstall_install_id;
                }
            }
        }
        assert_eq!(
            validate_sanitized_export_bytes(
                &serde_json::to_vec(&reused_install_epoch).expect("reused install epoch"),
                &plan,
                &"c".repeat(40),
                &"d".repeat(64),
                &receipts,
            ),
            Err("lifecycle_sanitized_etw_install_epoch_invalid".to_string())
        );

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

    fn raw_machine(machine: &SanitizedMachineSnapshot) -> ElevatedMachineSnapshot {
        ElevatedMachineSnapshot {
            machine: super::super::native::PreflightSnapshot {
                service: raw_observation(&machine.service, |service| ServiceSnapshot {
                    state: service.state,
                    process_id: service.process_id,
                    process_started_at_100ns: service.process_started_at_100ns,
                    win32_exit_code: service.win32_exit_code,
                    service_specific_exit_code: service.service_specific_exit_code,
                }),
                install_root: raw_observation(&machine.install_root, raw_directory),
                monitor: raw_observation(&machine.monitor, raw_file),
                service_binary: raw_observation(&machine.service_binary, raw_file),
                uninstaller: raw_observation(&machine.uninstaller, raw_file),
                legacy_cli: raw_observation(&machine.legacy_cli, raw_file),
                uninstall_registry: raw_observation(&machine.uninstall_registry, |registry| {
                    RegistrySnapshot {
                        view: if registry.view == "32" {
                            RegistryView::Registry32
                        } else {
                            RegistryView::Registry64
                        },
                        install_location: raw_path(&registry.install_location),
                    }
                }),
                product_processes: raw_observation(&machine.product_processes, |processes| {
                    processes
                        .iter()
                        .map(|process| ProcessSnapshot {
                            process_id: process.process_id,
                            parent_process_id: 0,
                            executable_name: process.executable_name.clone(),
                            executable_path: process.executable_path.as_ref().map(raw_path),
                        })
                        .collect()
                }),
            },
            product_data_root: raw_observation(&machine.product_data_root, raw_directory),
            service_data_root: raw_observation(&machine.service_data_root, raw_directory),
            current_user_data_root: Observation::Unknown(
                "lifecycle_current_user_root_parent_authority_missing".to_string(),
            ),
            installed_boundaries: raw_observation(&machine.installed_boundaries, raw_boundaries),
            named_pipe: match &machine.named_pipe {
                Some(pipe) => Observation::Present(NamedPipeSnapshot {
                    server_process_id: pipe.server_process_id,
                    server_process_started_at_100ns: machine
                        .service
                        .as_present()
                        .and_then(|service| service.process_started_at_100ns)
                        .expect("running service start time"),
                }),
                None => Observation::Absent,
            },
            etw_lease: match &machine.etw_session {
                Some(etw) => Observation::Present(etw.lease.clone()),
                None => Observation::Absent,
            },
            etw_session: match &machine.etw_session {
                Some(etw) => {
                    Observation::Present(crate::windows_network::EtwSessionProofSnapshot {
                        identity: etw.observed_session.clone(),
                        events_lost: etw.events_lost,
                        log_buffers_lost: etw.buffers_lost,
                        realtime_buffers_lost: 0,
                    })
                }
                None => Observation::Absent,
            },
            etw_owner_lock: if machine.etw_owner_lock.is_some() {
                RuntimeLockObservation::Held {}
            } else {
                RuntimeLockObservation::Absent {}
            },
            service_lifecycle_lock: if machine.service_lifecycle_lock.is_some() {
                RuntimeLockObservation::Held {}
            } else {
                RuntimeLockObservation::Absent {}
            },
            service_install_residue: raw_service_install_residue(machine),
            machine_registration: raw_machine_registration(machine),
        }
    }

    fn raw_machine_registration(machine: &SanitizedMachineSnapshot) -> MachineRegistrationForProof {
        MachineRegistrationForProof {
            product_key_64: if machine.machine_product_key.is_some() {
                Observation::Present(ProductRegistrationKeyForProof {
                    final_key_path: r"\REGISTRY\MACHINE\SOFTWARE\batcave\BatCave Monitor"
                        .to_string(),
                    install_root: r"C:\Program Files\BatCave Monitor".to_string(),
                    value_names: vec![String::new()],
                    subkey_names: Vec::new(),
                    default_value_type: 1,
                    last_write_time_100ns: 1,
                    owner: SecurityPrincipalForProof::LocalSystem,
                    dacl_sha256: "7".repeat(64),
                })
            } else {
                Observation::Absent
            },
            product_key_32: Observation::Absent,
            public_desktop_shortcut: machine
                .public_desktop_shortcut
                .as_ref()
                .map(|shortcut| raw_shortcut(shortcut, 900))
                .map_or(Observation::Absent, Observation::Present),
            common_start_menu_shortcut: machine
                .common_start_menu_shortcut
                .as_ref()
                .map(|shortcut| raw_shortcut(shortcut, 901))
                .map_or(Observation::Absent, Observation::Present),
        }
    }

    fn raw_shortcut(shortcut: &SanitizedShortcutSnapshot, file_index: u64) -> ShortcutForProof {
        ShortcutForProof {
            path: raw_path(&shortcut.path),
            target: raw_path(&shortcut.target),
            arguments: String::new(),
            icon_path: String::new(),
            icon_index: 0,
            working_directory: r"C:\Program Files\BatCave Monitor".to_string(),
            show_command: 1,
            hotkey: 0,
            description: String::new(),
            app_user_model_id: "dev.batcave.monitor".to_string(),
            owner: SecurityPrincipalForProof::LocalSystem,
            dacl_sha256: "8".repeat(64),
            size: 128,
            sha256: shortcut.sha256.clone(),
            volume_serial: 1,
            file_index,
        }
    }

    fn raw_service_install_residue(
        machine: &SanitizedMachineSnapshot,
    ) -> ServiceInstallResidueForProof {
        ServiceInstallResidueForProof {
            service_registry_key: if machine.service_registry_key.is_some() {
                Observation::Present(ServiceRegistryKeyForProof {
                    last_failure: Observation::Absent,
                })
            } else {
                Observation::Absent
            },
            service_data: match &machine.service_data_root {
                Observation::Present(root) => Observation::Present(ServiceDataResidueForProof {
                    volume_serial: root.volume_serial,
                    file_index: root.file_index,
                    upgrade_transaction_journal: machine
                        .upgrade_transaction_journal
                        .as_ref()
                        .map(raw_residue_file)
                        .map_or(Observation::Absent, Observation::Present),
                    atomic_temporary_files: machine
                        .atomic_temporary_files
                        .iter()
                        .filter(|file| file.path.root == LogicalRoot::ServiceData)
                        .map(raw_residue_file)
                        .collect(),
                }),
                Observation::Absent => Observation::Absent,
                Observation::Unknown(reason) => Observation::Unknown(reason.clone()),
            },
            install: match &machine.install_root {
                Observation::Present(root) => Observation::Present(InstallResidueForProof {
                    volume_serial: root.volume_serial,
                    file_index: root.file_index,
                    staged_service_images: machine
                        .staged_service_images
                        .iter()
                        .map(raw_residue_file)
                        .collect(),
                    rollback_service_images: machine
                        .rollback_service_images
                        .iter()
                        .map(raw_residue_file)
                        .collect(),
                    atomic_temporary_files: machine
                        .atomic_temporary_files
                        .iter()
                        .filter(|file| file.path.root == LogicalRoot::Install)
                        .map(raw_residue_file)
                        .collect(),
                    rollback_execution_marker: machine
                        .failure_marker
                        .as_ref()
                        .map(raw_residue_file)
                        .map_or(Observation::Absent, Observation::Present),
                }),
                Observation::Absent => Observation::Absent,
                Observation::Unknown(reason) => Observation::Unknown(reason.clone()),
            },
        }
    }

    fn raw_residue_file(file: &SanitizedPathFileSnapshot) -> ResidueFileForProof {
        ResidueFileForProof {
            path: raw_path(&file.path),
            size: file.file.size,
            sha256: file.file.sha256.clone(),
            volume_serial: file.file.volume_serial,
            file_index: file.file.file_index,
        }
    }

    fn parent_current_user() -> ParentCurrentUserAuthority {
        ParentCurrentUserAuthority {
            user_sid: "S-1-5-21-1".to_string(),
            session_id: 1,
            logon_luid: super::super::native::LogonLuid {
                low_part: 2,
                high_part: 0,
            },
            profile: DirectorySnapshot {
                identity: super::super::native::FileIdentity {
                    volume_serial: 1,
                    file_index: 10,
                },
                final_path: r"C:\Users\proof".to_string(),
            },
            local_app_data: DirectorySnapshot {
                identity: super::super::native::FileIdentity {
                    volume_serial: 1,
                    file_index: 11,
                },
                final_path: r"C:\Users\proof\AppData\Local".to_string(),
            },
            resolved_data_root: r"C:\Users\proof\AppData\Local\BatCaveMonitor".to_string(),
            data_root: Observation::Present(DirectorySnapshot {
                identity: super::super::native::FileIdentity {
                    volume_serial: 1,
                    file_index: 1,
                },
                final_path: r"C:\Users\proof\AppData\Local\BatCaveMonitor".to_string(),
            }),
        }
    }

    fn parent_user_objects() -> ParentCurrentUserObjects {
        ParentCurrentUserObjects {
            settings: parent_user_file("a", 20),
            cache: parent_user_file("b", 21),
            diagnostics: parent_user_file("c", 22),
        }
    }

    fn parent_current_user_projection(
        export: &SanitizedExportPacket,
    ) -> ParentCurrentUserProjection<'static> {
        ParentCurrentUserProjection {
            authority: Box::leak(Box::new(parent_current_user())),
            before_uninstall: Box::leak(Box::new(parent_user_objects())),
            after_uninstall: Box::leak(Box::new(parent_user_objects())),
            residue_timeline: Box::leak(Box::new(parent_residue_timeline(export))),
        }
    }

    fn private_packets_for_export(
        export: &SanitizedExportPacket,
        parent_results: &[DesktopPhaseResult],
    ) -> Vec<(EvidenceReceipt, PrivateSuccessPacket)> {
        export
            .private_evidence
            .iter()
            .map(|entry| {
                let payload = match (&entry.desktop_phase, &entry.event) {
                    (Some(desktop), None) => {
                        let result = parent_results
                            .iter()
                            .find(|result| result.phase == desktop.phase)
                            .expect("parent desktop result")
                            .clone();
                        PrivateSuccessPayload::Desktop(Box::new(PrivateDesktopPayload {
                            machine: raw_machine(&entry.machine),
                            result,
                        }))
                    }
                    (None, Some(SanitizedStageEvent::ServiceCrash(event))) => {
                        PrivateSuccessPayload::ServiceCrash(PrivateServiceCrashPayload {
                            machine: raw_machine(&entry.machine),
                            termination: raw_termination(event),
                        })
                    }
                    (None, Some(SanitizedStageEvent::UpgradeRollback(event))) => {
                        PrivateSuccessPayload::UpgradeRollback(PrivateUpgradeRollbackPayload {
                            machine: raw_machine(&entry.machine),
                            rollback: FailedUpgradeRollbackForProof {
                                candidate_sha256: event.candidate_sha256.clone(),
                                candidate_failure_code: event.candidate_failure_code.clone(),
                                candidate_failure_detail: event.candidate_failure_detail.clone(),
                                execution_marker_sha256: event.execution_marker_sha256.clone(),
                                restored_sha256: event.restored_sha256.clone(),
                                restored_process_id: event.restored_process_id,
                            },
                        })
                    }
                    (None, None) => PrivateSuccessPayload::Machine(raw_machine(&entry.machine)),
                    (Some(_), Some(_)) => panic!("private packet has desktop and event"),
                };
                (entry.receipt.clone(), packet_for_test(payload))
            })
            .collect()
    }

    fn parent_residue_timeline(export: &SanitizedExportPacket) -> ParentCurrentUserResidueTimeline {
        let mut timeline = ParentCurrentUserResidueTimeline::default();
        let mut captured = BTreeSet::new();
        for entry in &export.private_evidence {
            let point = parent_residue_capture_point(&entry.receipt.name).expect("capture point");
            if captured.insert(point) {
                timeline
                    .insert(point, parent_residue_snapshot(&entry.machine))
                    .expect("unique capture point");
            }
        }
        let rollback = export
            .private_evidence
            .iter()
            .find(|entry| entry.receipt.name == "legacy-residue-seeded-state.private.json")
            .expect("seeded entry");
        timeline
            .insert(
                ParentCurrentUserCapturePoint::BaselineRollbackRecoverySeeded,
                parent_residue_snapshot(&rollback.machine),
            )
            .expect("seeded capture");
        timeline
    }

    fn parent_residue_snapshot(
        machine: &SanitizedMachineSnapshot,
    ) -> super::super::native::ParentCurrentUserResidueSnapshot {
        let hkcu_value = machine
            .hkcu_autostart
            .as_ref()
            .map(|_| super::super::native::ParentRunValueSnapshot {
                value_type: 1,
                value: super::super::native::exact_parent_run_value().to_string(),
            })
            .map_or(Observation::Absent, Observation::Present);
        let helper = if machine.known_retired_helper_artifacts.is_empty()
            && machine.unknown_helper_sentinel.is_none()
        {
            Observation::Absent
        } else {
            Observation::Present(super::super::native::ParentHelperManifestSnapshot {
                root: DirectorySnapshot {
                    identity: super::super::native::FileIdentity {
                        volume_serial: 1,
                        file_index: 400,
                    },
                    final_path: r"C:\Users\proof\AppData\Local\BatCaveMonitor\elevated-helper"
                        .to_string(),
                },
                root_owner_sid: "S-1-5-21-1".to_string(),
                root_dacl_sha256: "d".repeat(64),
                known_files: machine
                    .known_retired_helper_artifacts
                    .iter()
                    .map(parent_helper_snapshot)
                    .collect(),
                sentinel: machine
                    .unknown_helper_sentinel
                    .as_ref()
                    .map(parent_helper_snapshot)
                    .map_or(Observation::Absent, Observation::Present),
                unexpected_entry_count: 0,
                manifest_sha256: "e".repeat(64),
            })
        };
        super::super::native::ParentCurrentUserResidueSnapshot {
            hkcu_run: super::super::native::ParentRunKeySnapshot {
                final_key_path:
                    r"\REGISTRY\USER\S-1-5-21-1\Software\Microsoft\Windows\CurrentVersion\Run"
                        .to_string(),
                owner_sid: "S-1-5-21-1".to_string(),
                dacl_sha256: "c".repeat(64),
                last_write_time_100ns: 1,
                value_count: u32::from(machine.hkcu_autostart.is_some()),
                manifest_sha256: "b".repeat(64),
                batcave_monitor: hkcu_value,
            },
            helper,
        }
    }

    fn parent_helper_snapshot(file: &SanitizedPathFileSnapshot) -> ParentHelperFileSnapshot {
        ParentHelperFileSnapshot {
            relative_leaf: file.path.relative_leaf.clone(),
            file: FileSnapshot {
                size: file.file.size,
                sha256: file.file.sha256.clone(),
                identity: super::super::native::FileIdentity {
                    volume_serial: file.file.volume_serial,
                    file_index: file.file.file_index,
                },
            },
            owner_sid: "S-1-5-21-1".to_string(),
            dacl_sha256: "a".repeat(64),
        }
    }

    fn parent_user_file(hash_digit: &str, file_index: u64) -> Observation<FileSnapshot> {
        Observation::Present(FileSnapshot {
            size: 1,
            sha256: hash_digit.repeat(64),
            identity: super::super::native::FileIdentity {
                volume_serial: 1,
                file_index,
            },
        })
    }

    fn raw_observation<T, U>(
        observation: &Observation<T>,
        map: impl FnOnce(&T) -> U,
    ) -> Observation<U> {
        match observation {
            Observation::Present(value) => Observation::Present(map(value)),
            Observation::Absent => Observation::Absent,
            Observation::Unknown(reason) => Observation::Unknown(reason.clone()),
        }
    }

    fn raw_file(file: &SanitizedFileSnapshot) -> FileSnapshot {
        FileSnapshot {
            size: file.size,
            sha256: file.sha256.clone(),
            identity: super::super::native::FileIdentity {
                volume_serial: file.volume_serial,
                file_index: file.file_index,
            },
        }
    }

    fn raw_directory(directory: &SanitizedDirectorySnapshot) -> DirectorySnapshot {
        DirectorySnapshot {
            identity: super::super::native::FileIdentity {
                volume_serial: directory.volume_serial,
                file_index: directory.file_index,
            },
            final_path: raw_path(&directory.final_path),
        }
    }

    fn raw_boundaries(boundaries: &SanitizedBoundarySnapshot) -> InstalledBoundariesForProof {
        InstalledBoundariesForProof {
            service_dacl_sha256: boundaries.service_dacl_sha256.clone(),
            service_aces: boundaries.service_aces.iter().map(raw_ace).collect(),
            service_data_root_dacl_sha256: boundaries.service_data_root_dacl_sha256.clone(),
            service_data_root: SecurityPolicyForProof {
                owner: raw_principal(boundaries.service_data_root_owner),
                dacl_protected: boundaries.service_data_root_dacl_protected,
                reparse: boundaries.service_data_root_reparse,
                aces: boundaries
                    .service_data_root_aces
                    .iter()
                    .map(raw_ace)
                    .collect(),
            },
        }
    }

    fn raw_ace(ace: &SanitizedAceSnapshot) -> AcePolicyForProof {
        AcePolicyForProof {
            principal: raw_principal(ace.principal),
            allow: ace.allow,
            inherit_only: ace.inherit_only,
            object_inherit: ace.object_inherit,
            container_inherit: ace.container_inherit,
            mask: ace.mask,
        }
    }

    fn raw_principal(principal: SanitizedPrincipal) -> SecurityPrincipalForProof {
        match principal {
            SanitizedPrincipal::LocalSystem => SecurityPrincipalForProof::LocalSystem,
            SanitizedPrincipal::Administrators => SecurityPrincipalForProof::Administrators,
            SanitizedPrincipal::InteractiveUsers => SecurityPrincipalForProof::InteractiveUsers,
            SanitizedPrincipal::CollectorService => SecurityPrincipalForProof::CollectorService,
        }
    }

    fn raw_termination(event: &SanitizedServiceCrashEvent) -> TerminatedServiceForProof {
        TerminatedServiceForProof {
            target: ServiceTerminationTargetForProof {
                process_id: event.process_id,
                process_started_at_100ns: event.process_started_at_100ns,
                image_path: raw_path(&event.image_path),
                image_sha256: event.image_sha256.clone(),
            },
            process_exit_code: event.process_exit_code,
            win32_exit_code: event.win32_exit_code,
            service_specific_exit_code: event.service_specific_exit_code,
        }
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
            declared_current_user_objects_preserved: true,
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
            LogicalRoot::ProductData => r"C:\ProgramData\BatCaveMonitor",
            LogicalRoot::ServiceData => r"C:\ProgramData\BatCaveMonitor\Service",
            LogicalRoot::CurrentUserData => r"C:\Users\proof\AppData\Local\BatCaveMonitor",
            LogicalRoot::PublicDesktop => r"C:\Users\Public\Desktop",
            LogicalRoot::CommonStartMenu => r"C:\ProgramData\Microsoft\Windows\Start Menu\Programs",
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
                SanitizedEvidenceAssertion::FinalUninstalledPreservingDeclaredCurrentUserObjects => {
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
        apply_parent_current_user_residue_fixture(receipt_name, &mut machine);
        machine
    }

    fn apply_parent_current_user_residue_fixture(
        receipt_name: &str,
        machine: &mut SanitizedMachineSnapshot,
    ) {
        let seeded_known = matches!(
            receipt_name,
            "legacy-residue-seeded-state.private.json"
                | "final-upgrade-state.private.json"
                | "final-restart-stopped-state.private.json"
                | "final-restart-state.private.json"
                | "final-crashed-state.private.json"
                | "final-crash-recovery-state.private.json"
                | "final-missing-service-state.private.json"
        );
        let seeded_cleaned = matches!(
            receipt_name,
            "final-missing-service-desktop.private.json"
                | "final-missing-service-restored-state.private.json"
                | "final-stopped-service-state.private.json"
                | "final-stopped-service-desktop.private.json"
                | "final-stopped-service-restored-state.private.json"
                | "final-incompatible-service-state.private.json"
                | "final-incompatible-service-desktop.private.json"
                | "final-incompatible-service-restored-state.private.json"
        );
        let final_uninstall = receipt_name == "final-uninstall-state.private.json";
        machine.hkcu_autostart =
            (seeded_known || seeded_cleaned).then(|| SanitizedRegistryValueSnapshot {
                key: logical_path(
                    LogicalRoot::Hkcu,
                    "software/microsoft/windows/currentversion/run",
                ),
                value_name: "BatCave Monitor".to_string(),
                target: logical_path(LogicalRoot::Install, "batcave-monitor.exe"),
            });
        machine.known_retired_helper_artifacts = if seeded_known {
            KNOWN_RETIRED_HELPER_LEAVES
                .iter()
                .map(|leaf| known_helper_path_file(leaf))
                .collect()
        } else {
            Vec::new()
        };
        machine.unknown_helper_sentinel =
            (seeded_known || seeded_cleaned || final_uninstall).then(unknown_sentinel);
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
            machine_product_key: Some(logical_path(
                LogicalRoot::Hklm,
                "software/batcave/batcave monitor",
            )),
            hkcu_autostart: Some(SanitizedRegistryValueSnapshot {
                key: logical_path(
                    LogicalRoot::Hkcu,
                    "software/microsoft/windows/currentversion/run",
                ),
                value_name: "BatCave Monitor".to_string(),
                target: logical_path(LogicalRoot::Install, "batcave-monitor.exe"),
            }),
            public_desktop_shortcut: artifacts
                .shared_shortcuts_present
                .then(|| shortcut(LogicalRoot::PublicDesktop)),
            common_start_menu_shortcut: artifacts
                .shared_shortcuts_present
                .then(|| shortcut(LogicalRoot::CommonStartMenu)),
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
