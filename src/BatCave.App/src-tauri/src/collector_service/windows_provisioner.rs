use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};
use windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT;

use super::{
    etw_lease::ProtectedEtwLeaseRoot,
    protocol::COLLECTOR_SERVICE_NAME,
    windows_upgrade::{
        decide_upgrade_resume, is_staged_upgrade_name, staged_transaction_matches,
        upgrade_backup_name, UpgradeJournalV1, UpgradePhase, UpgradeResumeAction,
        UPGRADE_JOURNAL_FILE_NAME,
    },
};

const PROVISION_SWITCH: &str = "--provision";
const PRODUCT_DIRECTORY_NAME: &str = "BatCave Monitor";
const SERVICE_EXECUTABLE_NAME: &str = "batcave-collector-service.exe";
const MONITOR_EXECUTABLE_NAME: &str = "batcave-monitor.exe";
const LEGACY_WINDOWS_CLI_NAME: &str = "batcave-monitor-cli.exe";
const SERVICE_ACCOUNT: &str = "LocalSystem";
const SERVICE_OWNER_MARKER: &str = "dev.batcave.monitor/service-v1";
const SERVICE_FAILURE_VALUE: &str = "BatCaveLastFailure";
pub(crate) const SERVICE_LIFECYCLE_LOCK_FILE_NAME: &str = "process-owner.v1.lock";
const SERVICE_TYPE_OWN_PROCESS: u32 = 0x10;
const ERROR_FILE_NOT_FOUND_CODE: u32 = 2;
const ERROR_PATH_NOT_FOUND_CODE: u32 = 3;
#[cfg(feature = "private-windows-lifecycle-proof")]
const PRIVATE_ROLLBACK_FIXTURE_MAX_BYTES: usize = 64 * 1024 * 1024;
#[cfg(feature = "private-windows-lifecycle-proof")]
const PRIVATE_ROLLBACK_MARKER_NAME: &str = "batcave-rollback-fixture-ran.v1";
#[cfg(feature = "private-windows-lifecycle-proof")]
const PRIVATE_ROLLBACK_MARKER_BYTES: &[u8] = b"batcave_windows_lifecycle_rollback_fixture_v1\n";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct LegacyCliImage {
    size: u64,
    sha256: [u8; 32],
}

// Exact bytes from the former per-machine Windows CLI payload. The current
// Windows CLI is a standalone release asset and is not owned by NSIS.
const LEGACY_WINDOWS_CLI_IMAGES: [LegacyCliImage; 1] = [LegacyCliImage {
    size: 1_425_920,
    sha256: [
        0x80, 0xf3, 0x09, 0x39, 0x2d, 0x52, 0xca, 0xd1, 0xde, 0x5b, 0x18, 0x4c, 0x28, 0xa5, 0xe8,
        0xcf, 0xf6, 0x51, 0xd6, 0xa2, 0x57, 0x07, 0x9b, 0xd3, 0x34, 0x4c, 0xbb, 0x67, 0xcf, 0x21,
        0x5b, 0x4a,
    ],
}];

fn legacy_cli_image_matches(images: &[LegacyCliImage], size: u64, sha256: &[u8; 32]) -> bool {
    images
        .iter()
        .any(|image| image.size == size && image.sha256 == *sha256)
}

fn is_missing_path_error(error: u32) -> bool {
    matches!(error, ERROR_FILE_NOT_FOUND_CODE | ERROR_PATH_NOT_FOUND_CODE)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum MissingServiceCleanup {
    None,
    ProductOnly,
    ServiceTree,
}

fn missing_service_cleanup(
    product_root_exists: bool,
    service_root_exists: bool,
) -> MissingServiceCleanup {
    if service_root_exists {
        MissingServiceCleanup::ServiceTree
    } else if product_root_exists {
        MissingServiceCleanup::ProductOnly
    } else {
        MissingServiceCleanup::None
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ProvisionVerb {
    PrepareUpgrade,
    PrepareUpgradeStaged,
    CommitUpgradeStaged,
    Install,
    RetireInstallerShortcuts,
    Uninstall,
    UninstallStaged,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum InstallerControllerKind {
    Stable,
    Staged,
}

fn installer_controller_kind(name: &str) -> Result<InstallerControllerKind, String> {
    if name == SERVICE_EXECUTABLE_NAME {
        Ok(InstallerControllerKind::Stable)
    } else if is_staged_upgrade_name(name) {
        Ok(InstallerControllerKind::Staged)
    } else {
        Err("installer_shortcut_controller_name_invalid".to_string())
    }
}

pub(crate) fn run_cli(args: &[String]) -> Option<i32> {
    let verb = match args {
        [switch, verb] if switch == PROVISION_SWITCH => match verb.as_str() {
            "prepare-upgrade" => ProvisionVerb::PrepareUpgrade,
            "prepare-upgrade-staged" => ProvisionVerb::PrepareUpgradeStaged,
            "commit-upgrade-staged" => ProvisionVerb::CommitUpgradeStaged,
            "install" => ProvisionVerb::Install,
            "retire-installer-shortcuts" => ProvisionVerb::RetireInstallerShortcuts,
            "uninstall" => ProvisionVerb::Uninstall,
            "uninstall-staged" => ProvisionVerb::UninstallStaged,
            _ => {
                eprintln!("collector_service_provisioner_verb_invalid");
                return Some(2);
            }
        },
        [switch, ..] if switch == PROVISION_SWITCH => {
            eprintln!("collector_service_provisioner_arguments_invalid");
            return Some(2);
        }
        [] => return None,
        _ => {
            eprintln!("collector_service_provisioner_arguments_invalid");
            return Some(2);
        }
    };

    Some(match run_verb(verb) {
        Ok(()) => 0,
        Err(error) => {
            eprintln!("{error}");
            1
        }
    })
}

fn run_verb(verb: ProvisionVerb) -> Result<(), String> {
    match verb {
        ProvisionVerb::PrepareUpgrade => native::prepare_upgrade(),
        ProvisionVerb::PrepareUpgradeStaged => native::prepare_upgrade_staged(),
        ProvisionVerb::CommitUpgradeStaged => native::commit_upgrade_staged(),
        ProvisionVerb::Install => native::install(),
        ProvisionVerb::RetireInstallerShortcuts => native::retire_installer_shortcuts(),
        ProvisionVerb::Uninstall => native::uninstall(),
        ProvisionVerb::UninstallStaged => native::uninstall_staged(),
    }
}

#[cfg(feature = "private-windows-lifecycle-proof")]
fn validate_failed_upgrade_fixture_inputs(
    candidate_bytes: &[u8],
    expected_candidate_sha256: [u8; 32],
    expected_original_sha256: [u8; 32],
) -> Result<(), String> {
    if candidate_bytes.is_empty()
        || candidate_bytes.len() > PRIVATE_ROLLBACK_FIXTURE_MAX_BYTES
        || expected_candidate_sha256 == [0; 32]
        || expected_original_sha256 == [0; 32]
        || expected_candidate_sha256 == expected_original_sha256
        || <[u8; 32]>::from(Sha256::digest(candidate_bytes)) != expected_candidate_sha256
    {
        return Err("collector_service_proof_upgrade_fixture_invalid".to_string());
    }
    Ok(())
}

pub(crate) fn open_protected_etw_lease_root() -> Result<ProtectedEtwLeaseRoot, String> {
    native::open_protected_etw_lease_root()
}

pub(crate) fn record_service_failure(category: &str) -> Result<(), String> {
    native::record_service_failure(category)
}

pub(crate) fn clear_service_failure() -> Result<(), String> {
    native::clear_service_failure()
}

pub(crate) fn acquire_service_lifecycle_marker() -> Result<impl std::fmt::Debug, String> {
    native::acquire_service_lifecycle_marker()
}

#[cfg(feature = "private-windows-lifecycle-proof")]
pub(crate) fn observe_installed_boundaries_for_proof(
    expected_image: &Path,
) -> Result<InstalledBoundariesForProof, String> {
    native::observe_installed_boundaries_for_proof(expected_image)
}

#[cfg(feature = "private-windows-lifecycle-proof")]
pub(crate) fn data_roots_for_proof() -> Result<(PathBuf, PathBuf), String> {
    native::data_roots_for_proof()
}

#[cfg(feature = "private-windows-lifecycle-proof")]
#[derive(Clone, Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields, rename_all = "snake_case", tag = "kind")]
pub(crate) enum RuntimeLockObservation {
    Absent {},
    Released {},
    Held {},
    Unknown { reason: String },
}

#[cfg(feature = "private-windows-lifecycle-proof")]
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ProtectedRuntimeFilesForProof {
    pub(crate) etw_lease: super::etw_lease::ReadOnlyEtwLeaseObservation,
    pub(crate) etw_owner_lock: RuntimeLockObservation,
    pub(crate) service_lifecycle_lock: RuntimeLockObservation,
}

#[cfg(feature = "private-windows-lifecycle-proof")]
pub(crate) fn observe_protected_runtime_files_for_proof(
    expected_volume_serial: u32,
    expected_file_index: u64,
) -> Result<ProtectedRuntimeFilesForProof, String> {
    native::observe_protected_runtime_files_for_proof(expected_volume_serial, expected_file_index)
}

#[cfg(feature = "private-windows-lifecycle-proof")]
#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum SecurityPrincipalForProof {
    LocalSystem,
    Administrators,
    TrustedInstaller,
    InteractiveUsers,
    CollectorService,
    Other,
}

#[cfg(feature = "private-windows-lifecycle-proof")]
#[derive(Clone, Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct AcePolicyForProof {
    pub(crate) principal: SecurityPrincipalForProof,
    pub(crate) allow: bool,
    pub(crate) inherit_only: bool,
    pub(crate) object_inherit: bool,
    pub(crate) container_inherit: bool,
    pub(crate) mask: u32,
}

#[cfg(feature = "private-windows-lifecycle-proof")]
#[derive(Clone, Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SecurityPolicyForProof {
    pub(crate) owner: SecurityPrincipalForProof,
    pub(crate) dacl_protected: bool,
    pub(crate) reparse: bool,
    pub(crate) aces: Vec<AcePolicyForProof>,
}

#[cfg(feature = "private-windows-lifecycle-proof")]
#[derive(Clone, Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct InstalledBoundariesForProof {
    pub(crate) service_dacl_sha256: String,
    pub(crate) service_aces: Vec<AcePolicyForProof>,
    pub(crate) service_data_root_dacl_sha256: String,
    pub(crate) service_data_root: SecurityPolicyForProof,
}

#[cfg(feature = "private-windows-lifecycle-proof")]
#[derive(Clone, Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ResidueFileForProof {
    pub(crate) path: String,
    pub(crate) size: u64,
    pub(crate) sha256: String,
    pub(crate) volume_serial: u32,
    pub(crate) file_index: u64,
}

#[cfg(feature = "private-windows-lifecycle-proof")]
#[derive(Clone, Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ServiceRegistryKeyForProof {
    pub(crate) last_failure: crate::windows_lifecycle_proof_contract::Observation<String>,
}

#[cfg(feature = "private-windows-lifecycle-proof")]
#[derive(Clone, Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ServiceDataResidueForProof {
    pub(crate) volume_serial: u32,
    pub(crate) file_index: u64,
    pub(crate) upgrade_transaction_journal:
        crate::windows_lifecycle_proof_contract::Observation<ResidueFileForProof>,
    pub(crate) atomic_temporary_files: Vec<ResidueFileForProof>,
}

#[cfg(feature = "private-windows-lifecycle-proof")]
#[derive(Clone, Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct InstallResidueForProof {
    pub(crate) volume_serial: u32,
    pub(crate) file_index: u64,
    pub(crate) staged_service_images: Vec<ResidueFileForProof>,
    pub(crate) rollback_service_images: Vec<ResidueFileForProof>,
    pub(crate) atomic_temporary_files: Vec<ResidueFileForProof>,
    pub(crate) rollback_execution_marker:
        crate::windows_lifecycle_proof_contract::Observation<ResidueFileForProof>,
}

#[cfg(feature = "private-windows-lifecycle-proof")]
#[derive(Clone, Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ServiceInstallResidueForProof {
    pub(crate) service_registry_key:
        crate::windows_lifecycle_proof_contract::Observation<ServiceRegistryKeyForProof>,
    pub(crate) service_data:
        crate::windows_lifecycle_proof_contract::Observation<ServiceDataResidueForProof>,
    pub(crate) install:
        crate::windows_lifecycle_proof_contract::Observation<InstallResidueForProof>,
}

#[cfg(feature = "private-windows-lifecycle-proof")]
#[derive(Clone, Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ProductRegistrationKeyForProof {
    pub(crate) final_key_path: String,
    pub(crate) install_root: String,
    pub(crate) value_names: Vec<String>,
    pub(crate) subkey_names: Vec<String>,
    pub(crate) default_value_type: u32,
    pub(crate) last_write_time_100ns: u64,
    pub(crate) owner: SecurityPrincipalForProof,
    pub(crate) dacl_sha256: String,
}

#[cfg(feature = "private-windows-lifecycle-proof")]
#[derive(Clone, Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ShortcutForProof {
    pub(crate) path: String,
    pub(crate) target: String,
    pub(crate) arguments: String,
    pub(crate) icon_path: String,
    pub(crate) icon_index: i32,
    pub(crate) working_directory: String,
    pub(crate) show_command: i32,
    pub(crate) hotkey: u16,
    pub(crate) description: String,
    pub(crate) app_user_model_id: String,
    pub(crate) owner: SecurityPrincipalForProof,
    pub(crate) dacl_sha256: String,
    pub(crate) size: u64,
    pub(crate) sha256: String,
    pub(crate) volume_serial: u32,
    pub(crate) file_index: u64,
}

#[cfg(feature = "private-windows-lifecycle-proof")]
#[derive(Clone, Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct MachineRegistrationForProof {
    pub(crate) product_key_64:
        crate::windows_lifecycle_proof_contract::Observation<ProductRegistrationKeyForProof>,
    pub(crate) product_key_32:
        crate::windows_lifecycle_proof_contract::Observation<ProductRegistrationKeyForProof>,
    pub(crate) public_desktop_shortcut:
        crate::windows_lifecycle_proof_contract::Observation<ShortcutForProof>,
    pub(crate) common_start_menu_shortcut:
        crate::windows_lifecycle_proof_contract::Observation<ShortcutForProof>,
}

#[cfg(feature = "private-windows-lifecycle-proof")]
pub(crate) fn observe_machine_registration_for_proof() -> MachineRegistrationForProof {
    native::observe_machine_registration_for_proof()
}

#[cfg(feature = "private-windows-lifecycle-proof")]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ResidueKindForProof {
    Journal,
    ServiceDataAtomic,
    Staged,
    Rollback,
    InstallAtomic,
    RollbackExecutionMarker,
}

#[cfg(feature = "private-windows-lifecycle-proof")]
pub(crate) fn observe_service_install_residue_for_proof() -> ServiceInstallResidueForProof {
    native::observe_service_install_residue_for_proof()
}

#[cfg(feature = "private-windows-lifecycle-proof")]
pub(crate) fn service_data_atomic_temp_name_for_proof(name: &str) -> bool {
    native::is_owned_atomic_temp_name(name)
}

#[cfg(feature = "private-windows-lifecycle-proof")]
pub(crate) fn install_staged_name_for_proof(name: &str) -> bool {
    is_staged_upgrade_name(name)
}

#[cfg(feature = "private-windows-lifecycle-proof")]
pub(crate) fn install_rollback_name_for_proof(name: &str) -> bool {
    native::rollback_digest_from_name(name).is_some()
}

#[cfg(feature = "private-windows-lifecycle-proof")]
pub(crate) fn install_atomic_temp_name_for_proof(name: &str) -> bool {
    native::install_atomic_temp_name_for_proof(name)
}

#[cfg(feature = "private-windows-lifecycle-proof")]
pub(crate) fn rollback_execution_marker_name_for_proof() -> &'static str {
    PRIVATE_ROLLBACK_MARKER_NAME
}

#[cfg(feature = "private-windows-lifecycle-proof")]
pub(crate) fn residue_size_valid_for_proof(kind: ResidueKindForProof, size: u64) -> bool {
    let kind = match kind {
        ResidueKindForProof::Journal => native::ProofResidueKind::Journal,
        ResidueKindForProof::ServiceDataAtomic => native::ProofResidueKind::ServiceDataAtomic,
        ResidueKindForProof::Staged => native::ProofResidueKind::Staged,
        ResidueKindForProof::Rollback => native::ProofResidueKind::Rollback,
        ResidueKindForProof::InstallAtomic => native::ProofResidueKind::InstallAtomic,
        ResidueKindForProof::RollbackExecutionMarker => {
            native::ProofResidueKind::RollbackExecutionMarker
        }
    };
    native::proof_residue_size_valid(kind, size)
}

#[cfg(feature = "private-windows-lifecycle-proof")]
pub(crate) fn residue_set_bounds_valid_for_proof(count: usize, total_bytes: u64) -> bool {
    count <= native::PROOF_RESIDUE_MAX_MATCHED_CHILDREN
        && total_bytes <= native::PROOF_RESIDUE_MAX_TOTAL_BYTES
}

#[cfg(feature = "private-windows-lifecycle-proof")]
#[derive(Clone, Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ServiceTerminationTargetForProof {
    pub(crate) process_id: u32,
    pub(crate) process_started_at_100ns: u64,
    pub(crate) image_path: String,
    pub(crate) image_sha256: String,
}

#[cfg(feature = "private-windows-lifecycle-proof")]
#[derive(Clone, Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ServiceTerminationObservationForProof {
    pub(crate) service_state: Option<u32>,
    pub(crate) service_process_id: Option<u32>,
    pub(crate) win32_exit_code: Option<u32>,
    pub(crate) service_specific_exit_code: Option<u32>,
    pub(crate) lifecycle_marker_settled: bool,
    pub(crate) process_exited: bool,
    pub(crate) process_exit_code: Option<u32>,
}

#[cfg(feature = "private-windows-lifecycle-proof")]
#[derive(Clone, Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct TerminatedServiceForProof {
    pub(crate) target: ServiceTerminationTargetForProof,
    pub(crate) process_exit_code: u32,
    pub(crate) win32_exit_code: u32,
    pub(crate) service_specific_exit_code: u32,
}

#[cfg(feature = "private-windows-lifecycle-proof")]
#[derive(Clone, Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct FailedUpgradeRollbackForProof {
    pub(crate) candidate_sha256: String,
    pub(crate) candidate_failure_code: String,
    pub(crate) candidate_failure_detail: String,
    pub(crate) execution_marker_sha256: String,
    pub(crate) restored_sha256: String,
    pub(crate) restored_process_id: u32,
}

#[cfg(feature = "private-windows-lifecycle-proof")]
#[derive(Clone, Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct FailedUpgradeRollbackFailure {
    pub(crate) reason: String,
    pub(crate) service_settled: bool,
}

#[cfg(feature = "private-windows-lifecycle-proof")]
#[derive(Clone, Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ServiceTerminationFailure {
    pub(crate) reason: String,
    pub(crate) service_settled: bool,
    pub(crate) target: Option<ServiceTerminationTargetForProof>,
    pub(crate) terminal_observation: Option<ServiceTerminationObservationForProof>,
}

#[cfg(feature = "private-windows-lifecycle-proof")]
#[derive(Clone, Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ServiceStateTransitionFailure {
    pub(crate) reason: String,
    pub(crate) service_settled: bool,
}

#[cfg(feature = "private-windows-lifecycle-proof")]
pub(crate) struct ServiceStateTransactionOutcome<T, E> {
    pub(crate) body: Result<T, E>,
    pub(crate) restoration: ServiceStateRestorationOutcome,
}

#[cfg(feature = "private-windows-lifecycle-proof")]
pub(crate) enum ServiceStateRestorationOutcome {
    Restored,
    Failed(Box<ServiceStateTransitionFailure>),
    BlockedUnsettled,
}

#[cfg(feature = "private-windows-lifecycle-proof")]
fn restore_after_settled_body<T, E>(
    body: &Result<T, E>,
    body_settled: impl FnOnce(&E) -> bool,
    restore: impl FnOnce() -> Result<(), Box<ServiceStateTransitionFailure>>,
) -> ServiceStateRestorationOutcome {
    if body.as_ref().is_err_and(|failure| !body_settled(failure)) {
        return ServiceStateRestorationOutcome::BlockedUnsettled;
    }
    match restore() {
        Ok(()) => ServiceStateRestorationOutcome::Restored,
        Err(failure) => ServiceStateRestorationOutcome::Failed(failure),
    }
}

#[cfg(feature = "private-windows-lifecycle-proof")]
pub(crate) fn terminate_running_service_for_proof(
    expected_sha256: [u8; 32],
) -> Result<TerminatedServiceForProof, Box<ServiceTerminationFailure>> {
    native::terminate_running_service_for_proof(expected_sha256)
}

#[cfg(feature = "private-windows-lifecycle-proof")]
pub(crate) fn exercise_failed_upgrade_rollback_for_proof(
    candidate_bytes: &[u8],
    expected_candidate_sha256: [u8; 32],
    expected_original_sha256: [u8; 32],
) -> Result<FailedUpgradeRollbackForProof, Box<FailedUpgradeRollbackFailure>> {
    native::exercise_failed_upgrade_rollback_for_proof(
        candidate_bytes,
        expected_candidate_sha256,
        expected_original_sha256,
    )
}

#[cfg(feature = "private-windows-lifecycle-proof")]
pub(crate) fn with_missing_service_for_proof<T, E>(
    final_service_bytes: &[u8],
    expected_final_sha256: [u8; 32],
    body: impl FnOnce() -> Result<T, E>,
    body_settled: impl FnOnce(&E) -> bool,
) -> Result<ServiceStateTransactionOutcome<T, E>, Box<ServiceStateTransitionFailure>> {
    native::remove_service_boundary_for_proof(final_service_bytes, expected_final_sha256)?;
    let body = body();
    let restoration = restore_after_settled_body(&body, body_settled, || {
        native::restore_service_boundary_for_proof(final_service_bytes, expected_final_sha256)
    });
    Ok(ServiceStateTransactionOutcome { body, restoration })
}

#[cfg(feature = "private-windows-lifecycle-proof")]
pub(crate) fn with_stopped_service_for_proof<T, E>(
    expected_sha256: [u8; 32],
    body: impl FnOnce() -> Result<T, E>,
    body_settled: impl FnOnce(&E) -> bool,
) -> Result<ServiceStateTransactionOutcome<T, E>, Box<ServiceStateTransitionFailure>> {
    native::stop_running_service_for_proof(expected_sha256)?;
    let body = body();
    let restoration = restore_after_settled_body(&body, body_settled, || {
        native::start_stopped_service_for_proof(expected_sha256)
    });
    Ok(ServiceStateTransactionOutcome { body, restoration })
}

#[cfg(feature = "private-windows-lifecycle-proof")]
pub(crate) fn with_incompatible_service_for_proof<T, E>(
    final_service_bytes: &[u8],
    expected_final_sha256: [u8; 32],
    incompatible_service_bytes: &[u8],
    expected_incompatible_sha256: [u8; 32],
    body: impl FnOnce() -> Result<T, E>,
    body_settled: impl FnOnce(&E) -> bool,
) -> Result<ServiceStateTransactionOutcome<T, E>, Box<ServiceStateTransitionFailure>> {
    native::replace_running_service_for_proof(
        incompatible_service_bytes,
        expected_incompatible_sha256,
        expected_final_sha256,
    )?;
    let body = body();
    let restoration = restore_after_settled_body(&body, body_settled, || {
        native::replace_running_service_for_proof(
            final_service_bytes,
            expected_final_sha256,
            expected_incompatible_sha256,
        )
    });
    Ok(ServiceStateTransactionOutcome { body, restoration })
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PrincipalClass {
    LocalSystem,
    Administrators,
    TrustedInstaller,
    InteractiveUsers,
    CollectorService,
    Other,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct AcePolicy {
    principal: PrincipalClass,
    allow: bool,
    inherit_only: bool,
    object_inherit: bool,
    container_inherit: bool,
    mask: u32,
}

#[derive(Clone, Debug)]
struct SecurityPolicy {
    owner: PrincipalClass,
    dacl_protected: bool,
    reparse: bool,
    aces: Vec<AcePolicy>,
}

const FILE_GENERIC_READ_EXECUTE: u32 = 0x0012_00a9;
const FILE_MODIFY: u32 = 0x0013_01bf;
const FILE_ALL_ACCESS: u32 = 0x001f_01ff;

fn validate_product_root_policy(policy: &SecurityPolicy, service_leaf: bool) -> Result<(), String> {
    if policy.reparse {
        return Err("collector_service_root_reparse_rejected".to_string());
    }
    if policy.owner != PrincipalClass::LocalSystem {
        return Err("collector_service_root_owner_invalid".to_string());
    }
    if !policy.dacl_protected {
        return Err("collector_service_root_dacl_unprotected".to_string());
    }

    let service_mask = if service_leaf {
        FILE_MODIFY
    } else {
        FILE_GENERIC_READ_EXECUTE
    };
    let expected = [
        (PrincipalClass::LocalSystem, FILE_ALL_ACCESS),
        (PrincipalClass::Administrators, FILE_ALL_ACCESS),
        (PrincipalClass::CollectorService, service_mask),
    ];
    if policy.aces.len() != expected.len() {
        return Err("collector_service_root_dacl_invalid".to_string());
    }
    for (principal, mask) in expected {
        let matches = policy
            .aces
            .iter()
            .filter(|ace| {
                ace.allow
                    && !ace.inherit_only
                    && ace.object_inherit
                    && ace.container_inherit
                    && ace.principal == principal
                    && ace.mask == mask
            })
            .count();
        if matches != 1 {
            return Err("collector_service_root_dacl_invalid".to_string());
        }
    }
    Ok(())
}

#[derive(Debug)]
struct ExistingServicePolicy<'a> {
    owner_marker: Option<&'a str>,
    image_path: &'a Path,
    account: &'a str,
    service_type: u32,
}

fn validate_existing_service_policy(
    policy: &ExistingServicePolicy<'_>,
    expected_image_path: &Path,
) -> Result<(), String> {
    if policy.owner_marker != Some(SERVICE_OWNER_MARKER) {
        return Err("collector_service_foreign_service_rejected".to_string());
    }
    if !fixed_path_eq(policy.image_path, expected_image_path)
        || policy.account != SERVICE_ACCOUNT
        || policy.service_type != SERVICE_TYPE_OWN_PROCESS
    {
        return Err("collector_service_owned_service_identity_invalid".to_string());
    }
    Ok(())
}

fn expected_service_path(program_files: &Path) -> PathBuf {
    program_files
        .join(PRODUCT_DIRECTORY_NAME)
        .join(SERVICE_EXECUTABLE_NAME)
}

fn staged_service_executable_name() -> String {
    "batcave-collector-service.recovery.exe".to_string()
}

fn expected_staged_service_path(program_files: &Path) -> PathBuf {
    program_files
        .join(PRODUCT_DIRECTORY_NAME)
        .join(staged_service_executable_name())
}

fn validate_current_service_path(current: &Path, expected: &Path) -> Result<(), String> {
    if fixed_path_eq(current, expected) {
        Ok(())
    } else {
        Err("collector_service_executable_location_invalid".to_string())
    }
}

fn fixed_path_eq(left: &Path, right: &Path) -> bool {
    use std::os::windows::ffi::OsStrExt;

    let left = left.as_os_str().encode_wide().collect::<Vec<_>>();
    let right = right.as_os_str().encode_wide().collect::<Vec<_>>();
    left.len() == right.len()
        && left.iter().zip(right.iter()).all(|(left, right)| {
            if *left <= 0x7f && *right <= 0x7f {
                (*left as u8).eq_ignore_ascii_case(&(*right as u8))
            } else {
                left == right
            }
        })
}

pub(crate) fn strip_verbatim_disk_prefix(path: PathBuf) -> PathBuf {
    use std::{
        ffi::OsString,
        os::windows::ffi::{OsStrExt, OsStringExt},
    };

    const VERBATIM_PREFIX: &[u16] = &[b'\\' as u16, b'\\' as u16, b'?' as u16, b'\\' as u16];

    let wide = path.as_os_str().encode_wide().collect::<Vec<_>>();
    let Some(disk_path) = wide.strip_prefix(VERBATIM_PREFIX) else {
        return path;
    };
    if disk_path.len() < 3
        || !matches!(disk_path[0], 0x41..=0x5a | 0x61..=0x7a)
        || disk_path[1] != b':' as u16
        || disk_path[2] != b'\\' as u16
    {
        return path;
    }

    PathBuf::from(OsString::from_wide(disk_path))
}

fn attributes_are_reparse(attributes: u32) -> bool {
    attributes & FILE_ATTRIBUTE_REPARSE_POINT != 0
}

mod native {
    use super::*;

    use std::{
        ffi::{c_void, OsString},
        fs,
        mem::size_of,
        os::windows::ffi::{OsStrExt, OsStringExt},
        ptr, thread,
        time::{Duration, Instant},
    };

    #[cfg(feature = "private-windows-lifecycle-proof")]
    use windows_sys::core::GUID;
    #[cfg(feature = "private-windows-lifecycle-proof")]
    use windows_sys::Wdk::System::Registry::{KeyNameInformation, NtQueryKey};
    use windows_sys::Win32::{
        Foundation::{
            CloseHandle, GetLastError, LocalFree, SetLastError, ERROR_ALREADY_EXISTS,
            ERROR_FILE_NOT_FOUND, ERROR_INSUFFICIENT_BUFFER, ERROR_NOT_ALL_ASSIGNED,
            ERROR_SERVICE_ALREADY_RUNNING, ERROR_SERVICE_DOES_NOT_EXIST,
            ERROR_SERVICE_MARKED_FOR_DELETE, ERROR_SERVICE_NOT_ACTIVE, ERROR_SHARING_VIOLATION,
            ERROR_SUCCESS, HANDLE, LUID, WAIT_OBJECT_0,
        },
        Security::{
            AclSizeInformation, AdjustTokenPrivileges,
            Authorization::{
                ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW,
                ConvertStringSidToSidW, GetSecurityInfo, SE_FILE_OBJECT,
            },
            CreateWellKnownSid, EqualSid, GetAce, GetAclInformation, GetLengthSid,
            GetSecurityDescriptorControl, GetSecurityDescriptorDacl, GetTokenInformation,
            LookupAccountNameW, LookupPrivilegeValueW, TokenElevation, WinBuiltinAdministratorsSid,
            WinInteractiveSid, WinLocalSystemSid, ACCESS_ALLOWED_ACE, ACL_SIZE_INFORMATION,
            CONTAINER_INHERIT_ACE, DACL_SECURITY_INFORMATION, INHERIT_ONLY_ACE,
            LUID_AND_ATTRIBUTES, OBJECT_INHERIT_ACE, OWNER_SECURITY_INFORMATION,
            PSECURITY_DESCRIPTOR, PSID, SECURITY_ATTRIBUTES, SECURITY_MAX_SID_SIZE,
            SE_DACL_PROTECTED, SE_PRIVILEGE_ENABLED, SID_NAME_USE, TOKEN_ADJUST_PRIVILEGES,
            TOKEN_ELEVATION, TOKEN_PRIVILEGES, TOKEN_QUERY,
        },
        Storage::FileSystem::{
            CreateDirectoryW, CreateFileW, DeleteFileW, FileDispositionInfo,
            GetFileInformationByHandle, GetFinalPathNameByHandleW, MoveFileExW, ReadFile,
            RemoveDirectoryW, SetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION, DELETE,
            FILE_ADD_SUBDIRECTORY, FILE_APPEND_DATA, FILE_ATTRIBUTE_DIRECTORY,
            FILE_ATTRIBUTE_REPARSE_POINT, FILE_DELETE_CHILD, FILE_DISPOSITION_INFO,
            FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT, FILE_READ_ATTRIBUTES,
            FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE, FILE_WRITE_DATA,
            MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, OPEN_ALWAYS, OPEN_EXISTING,
            READ_CONTROL, WRITE_DAC, WRITE_OWNER,
        },
        System::{
            Registry::{
                RegCloseKey, RegDeleteValueW, RegOpenKeyExW, RegQueryValueExW, RegSetValueExW,
                HKEY_LOCAL_MACHINE, KEY_QUERY_VALUE, KEY_SET_VALUE, REG_SZ,
            },
            Services::{
                ChangeServiceConfig2W, CloseServiceHandle, ControlService, CreateServiceW,
                DeleteService, OpenSCManagerW, OpenServiceW, QueryServiceConfig2W,
                QueryServiceConfigW, QueryServiceObjectSecurity, QueryServiceStatusEx,
                SetServiceObjectSecurity, StartServiceW, QUERY_SERVICE_CONFIGW, SC_HANDLE,
                SC_MANAGER_CONNECT, SC_MANAGER_CREATE_SERVICE, SC_STATUS_PROCESS_INFO,
                SERVICE_ALL_ACCESS, SERVICE_AUTO_START, SERVICE_CONFIG_DELAYED_AUTO_START_INFO,
                SERVICE_CONFIG_DESCRIPTION, SERVICE_CONFIG_REQUIRED_PRIVILEGES_INFO,
                SERVICE_CONFIG_SERVICE_SID_INFO, SERVICE_CONTROL_STOP,
                SERVICE_DELAYED_AUTO_START_INFO, SERVICE_DESCRIPTIONW, SERVICE_ERROR_NORMAL,
                SERVICE_QUERY_STATUS, SERVICE_REQUIRED_PRIVILEGES_INFOW, SERVICE_RUNNING,
                SERVICE_SID_INFO, SERVICE_SID_TYPE_UNRESTRICTED, SERVICE_START_PENDING,
                SERVICE_STATUS, SERVICE_STATUS_PROCESS, SERVICE_STOPPED, SERVICE_STOP_PENDING,
                SERVICE_WIN32_OWN_PROCESS,
            },
            Threading::{
                GetCurrentProcess, OpenProcess, OpenProcessToken, QueryFullProcessImageNameW,
                WaitForSingleObject, PROCESS_QUERY_LIMITED_INFORMATION,
            },
        },
        UI::Shell::{
            SHGetFolderPathW, CSIDL_COMMON_APPDATA, CSIDL_PROGRAM_FILES, SHGFP_TYPE_CURRENT,
        },
    };
    #[cfg(feature = "private-windows-lifecycle-proof")]
    use windows_sys::Win32::{
        Foundation::{
            ERROR_LOCK_VIOLATION, ERROR_NO_MORE_ITEMS, FILETIME, PROPERTYKEY, WAIT_TIMEOUT,
        },
        Security::GetSecurityDescriptorOwner,
        Storage::FileSystem::{SetFilePointerEx, FILE_BEGIN},
        System::{
            Com::{
                CoCreateInstance, CoInitializeEx, CoTaskMemFree, CoUninitialize,
                StructuredStorage::{PropVariantClear, PropVariantToStringAlloc, PROPVARIANT},
                CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED,
            },
            Registry::{
                RegEnumKeyExW, RegEnumValueW, RegGetKeySecurity, RegQueryInfoKeyW,
                KEY_ENUMERATE_SUB_KEYS, KEY_WOW64_32KEY, KEY_WOW64_64KEY,
            },
            Threading::{GetExitCodeProcess, TerminateProcess, PROCESS_TERMINATE},
            Variant::VT_LPWSTR,
        },
        UI::Shell::{FOLDERID_CommonPrograms, FOLDERID_PublicDesktop, SHGetKnownFolderPath},
    };

    #[cfg(feature = "private-windows-lifecycle-proof")]
    use crate::collector_service::etw_lease::observe_lease_read_only_for_proof;
    use crate::collector_service::etw_lease::{ETW_LEASE_FILE_NAME, ETW_OWNER_LOCK_FILE_NAME};

    const SECURITY_DESCRIPTOR_REVISION_1: u32 = 1;
    const ACCESS_ALLOWED_ACE_TYPE: u8 = 0;
    const ACCESS_DENIED_ACE_TYPE: u8 = 1;
    const PRODUCT_ROOT_NAME: &str = "BatCaveMonitor";
    const SERVICE_ROOT_NAME: &str = "Service";
    const SERVICE_DISPLAY_NAME: &str = "BatCave Collector Service";
    const SERVICE_DESCRIPTION: &str =
        "Collects local system telemetry for BatCave Monitor without network access.";
    const SERVICE_OWNER_VALUE: &str = "BatCaveInstallerOwner";
    const SERVICE_REGISTRY_PATH: &str = r"SYSTEM\CurrentControlSet\Services\BatCaveCollector";
    const PRODUCT_UNINSTALL_REGISTRY_PATH: &str =
        r"Software\Microsoft\Windows\CurrentVersion\Uninstall\BatCave Monitor";
    #[cfg(feature = "private-windows-lifecycle-proof")]
    const PRODUCT_REGISTRATION_PATH: &str = r"Software\batcave\BatCave Monitor";
    #[cfg(feature = "private-windows-lifecycle-proof")]
    const PRODUCT_REGISTRATION_NT_PATH: &str =
        r"\REGISTRY\MACHINE\SOFTWARE\batcave\BatCave Monitor";
    #[cfg(feature = "private-windows-lifecycle-proof")]
    const PROOF_INSTALL_ROOT: &str = r"C:\Program Files\BatCave Monitor";
    #[cfg(feature = "private-windows-lifecycle-proof")]
    const PROOF_MONITOR_PATH: &str = r"C:\Program Files\BatCave Monitor\batcave-monitor.exe";
    #[cfg(feature = "private-windows-lifecycle-proof")]
    const PUBLIC_DESKTOP_PATH: &str = r"C:\Users\Public\Desktop";
    #[cfg(feature = "private-windows-lifecycle-proof")]
    const COMMON_PROGRAMS_PATH: &str = r"C:\ProgramData\Microsoft\Windows\Start Menu\Programs";
    #[cfg(feature = "private-windows-lifecycle-proof")]
    const PRODUCT_SHORTCUT_NAME: &str = "BatCave Monitor.lnk";
    #[cfg(feature = "private-windows-lifecycle-proof")]
    const PRODUCT_APP_USER_MODEL_ID: &str = "dev.batcave.monitor";
    #[cfg(feature = "private-windows-lifecycle-proof")]
    const PROOF_SHORTCUT_MAX_BYTES: u64 = 1024 * 1024;
    #[cfg(feature = "private-windows-lifecycle-proof")]
    const PROOF_COM_TEXT_CAPACITY: usize = 32 * 1024;
    #[cfg(feature = "private-windows-lifecycle-proof")]
    const PROOF_REGISTRY_MAX_TEXT_BYTES: u32 = 64 * 1024;
    #[cfg(feature = "private-windows-lifecycle-proof")]
    const PROOF_REGISTRY_MAX_NAME_CHARS: u32 = 1024;
    #[cfg(feature = "private-windows-lifecycle-proof")]
    const PRODUCT_REGISTRY_WRITE_MASK: u32 = 0x0000_0002
        | 0x0000_0004
        | 0x0000_0020
        | DELETE
        | WRITE_DAC
        | WRITE_OWNER
        | GENERIC_WRITE
        | GENERIC_ALL;
    #[cfg(feature = "private-windows-lifecycle-proof")]
    const CLSID_SHELL_LINK: GUID = GUID::from_u128(0x00021401_0000_0000_c000_000000000046);
    #[cfg(feature = "private-windows-lifecycle-proof")]
    const IID_SHELL_LINK_W: GUID = GUID::from_u128(0x000214f9_0000_0000_c000_000000000046);
    #[cfg(feature = "private-windows-lifecycle-proof")]
    const IID_PERSIST_FILE: GUID = GUID::from_u128(0x0000010b_0000_0000_c000_000000000046);
    #[cfg(feature = "private-windows-lifecycle-proof")]
    const IID_PROPERTY_STORE: GUID = GUID::from_u128(0x886d8eeb_8cf2_4446_8d02_cdba1dbdcf99);
    #[cfg(feature = "private-windows-lifecycle-proof")]
    const PKEY_APP_USER_MODEL_ID: PROPERTYKEY = PROPERTYKEY {
        fmtid: GUID::from_u128(0x9f4c2855_9f79_4b39_a8d0_e1d42de1d5f3),
        pid: 5,
    };
    // Deterministic NT SERVICE SID for "BatCaveCollector"; unlike account
    // lookup, it remains available after SCM deletion for cleanup retries.
    const SERVICE_SID: &str = "S-1-5-80-729049718-3519104438-3277487564-1168609684-1739013119";
    const SERVICE_REQUIRED_PRIVILEGES: [&str; 2] =
        ["SeChangeNotifyPrivilege", "SeSystemProfilePrivilege"];
    const SERVICE_OPERATION_TIMEOUT: Duration = Duration::from_secs(30);
    const UPGRADE_JOURNAL_MAX_BYTES: usize = 16 * 1024;
    const SERVICE_POLL_INTERVAL: Duration = Duration::from_millis(100);
    const SERVICE_QUERY_STATUS_MASK: u32 = 0x0000_0004;
    const SERVICE_LIFECYCLE_PROBE_ACCESS: u32 =
        FILE_READ_ATTRIBUTES | FILE_WRITE_DATA | READ_CONTROL;
    const SYNCHRONIZE_ACCESS: u32 = 0x0010_0000;
    const GENERIC_READ: u32 = 0x8000_0000;
    const GENERIC_WRITE: u32 = 0x4000_0000;
    const GENERIC_ALL: u32 = 0x1000_0000;
    const UNTRUSTED_WRITE_MASK: u32 = FILE_WRITE_DATA
        | FILE_APPEND_DATA
        | FILE_ADD_SUBDIRECTORY
        | FILE_DELETE_CHILD
        | DELETE
        | WRITE_DAC
        | WRITE_OWNER
        | GENERIC_WRITE
        | GENERIC_ALL;

    #[derive(Debug)]
    struct OwnedHandle(HANDLE);

    // Windows kernel handles may be closed from any thread. This wrapper only
    // retains the handles to keep verified filesystem objects non-replaceable.
    unsafe impl Send for OwnedHandle {}
    unsafe impl Sync for OwnedHandle {}

    impl OwnedHandle {
        fn new(handle: HANDLE, context: &str) -> Result<Self, String> {
            if handle.is_null() || handle == (-1_isize as HANDLE) {
                Err(last_error(context))
            } else {
                Ok(Self(handle))
            }
        }

        fn raw(&self) -> HANDLE {
            self.0
        }
    }

    impl Drop for OwnedHandle {
        fn drop(&mut self) {
            unsafe {
                CloseHandle(self.0);
            }
        }
    }

    struct EnabledPrivilege {
        token: OwnedHandle,
        previous: TOKEN_PRIVILEGES,
    }

    impl EnabledPrivilege {
        fn new(name: &str) -> Result<Self, String> {
            let mut token = ptr::null_mut();
            if unsafe {
                OpenProcessToken(
                    GetCurrentProcess(),
                    TOKEN_ADJUST_PRIVILEGES | TOKEN_QUERY,
                    &mut token,
                )
            } == 0
            {
                return Err(last_error("collector_service_privilege_token_open_failed"));
            }
            let token = OwnedHandle::new(token, "collector_service_privilege_token_invalid")?;

            let name = wide(name);
            let mut luid = LUID::default();
            if unsafe { LookupPrivilegeValueW(ptr::null(), name.as_ptr(), &mut luid) } == 0 {
                return Err(last_error("collector_service_privilege_lookup_failed"));
            }

            let requested = TOKEN_PRIVILEGES {
                PrivilegeCount: 1,
                Privileges: [LUID_AND_ATTRIBUTES {
                    Luid: luid,
                    Attributes: SE_PRIVILEGE_ENABLED,
                }],
            };
            let mut previous = TOKEN_PRIVILEGES::default();
            let mut returned = 0_u32;
            unsafe { SetLastError(ERROR_SUCCESS) };
            if unsafe {
                AdjustTokenPrivileges(
                    token.raw(),
                    0,
                    &requested,
                    size_of::<TOKEN_PRIVILEGES>() as u32,
                    &mut previous,
                    &mut returned,
                )
            } == 0
            {
                return Err(last_error("collector_service_privilege_enable_failed"));
            }
            let status = unsafe { GetLastError() };
            if status == ERROR_NOT_ALL_ASSIGNED {
                return Err("collector_service_privilege_not_assigned".to_string());
            }
            if status != ERROR_SUCCESS {
                return Err(format!(
                    "collector_service_privilege_enable_failed:{status}"
                ));
            }

            Ok(Self { token, previous })
        }
    }

    impl Drop for EnabledPrivilege {
        fn drop(&mut self) {
            unsafe {
                AdjustTokenPrivileges(
                    self.token.raw(),
                    0,
                    &self.previous,
                    0,
                    ptr::null_mut(),
                    ptr::null_mut(),
                );
            }
        }
    }

    #[derive(Debug)]
    struct OwnedScHandle(SC_HANDLE);

    impl OwnedScHandle {
        fn new(handle: SC_HANDLE, context: &str) -> Result<Self, String> {
            if handle.is_null() {
                Err(last_error(context))
            } else {
                Ok(Self(handle))
            }
        }

        fn raw(&self) -> SC_HANDLE {
            self.0
        }
    }

    impl Drop for OwnedScHandle {
        fn drop(&mut self) {
            unsafe {
                CloseServiceHandle(self.0);
            }
        }
    }

    struct OwnedRegistryKey(windows_sys::Win32::System::Registry::HKEY);

    impl OwnedRegistryKey {
        #[cfg(feature = "private-windows-lifecycle-proof")]
        fn raw(&self) -> windows_sys::Win32::System::Registry::HKEY {
            self.0
        }
    }

    impl Drop for OwnedRegistryKey {
        fn drop(&mut self) {
            unsafe {
                RegCloseKey(self.0);
            }
        }
    }

    #[cfg(feature = "private-windows-lifecycle-proof")]
    #[repr(C)]
    struct UnknownVtable {
        query_interface:
            unsafe extern "system" fn(*mut c_void, *const GUID, *mut *mut c_void) -> i32,
        add_ref: unsafe extern "system" fn(*mut c_void) -> u32,
        release: unsafe extern "system" fn(*mut c_void) -> u32,
    }

    #[cfg(feature = "private-windows-lifecycle-proof")]
    #[repr(C)]
    struct ShellLinkWVtable {
        unknown: UnknownVtable,
        get_path: unsafe extern "system" fn(*mut c_void, *mut u16, i32, *mut c_void, u32) -> i32,
        get_id_list: unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> i32,
        set_id_list: unsafe extern "system" fn(*mut c_void, *const c_void) -> i32,
        get_description: unsafe extern "system" fn(*mut c_void, *mut u16, i32) -> i32,
        set_description: unsafe extern "system" fn(*mut c_void, *const u16) -> i32,
        get_working_directory: unsafe extern "system" fn(*mut c_void, *mut u16, i32) -> i32,
        set_working_directory: unsafe extern "system" fn(*mut c_void, *const u16) -> i32,
        get_arguments: unsafe extern "system" fn(*mut c_void, *mut u16, i32) -> i32,
        set_arguments: unsafe extern "system" fn(*mut c_void, *const u16) -> i32,
        get_hotkey: unsafe extern "system" fn(*mut c_void, *mut u16) -> i32,
        set_hotkey: unsafe extern "system" fn(*mut c_void, u16) -> i32,
        get_show_command: unsafe extern "system" fn(*mut c_void, *mut i32) -> i32,
        set_show_command: unsafe extern "system" fn(*mut c_void, i32) -> i32,
        get_icon_location: unsafe extern "system" fn(*mut c_void, *mut u16, i32, *mut i32) -> i32,
        set_icon_location: unsafe extern "system" fn(*mut c_void, *const u16, i32) -> i32,
        set_relative_path: unsafe extern "system" fn(*mut c_void, *const u16, u32) -> i32,
        resolve: unsafe extern "system" fn(*mut c_void, *mut c_void, u32) -> i32,
        set_path: unsafe extern "system" fn(*mut c_void, *const u16) -> i32,
    }

    #[cfg(feature = "private-windows-lifecycle-proof")]
    #[repr(C)]
    struct PersistFileVtable {
        unknown: UnknownVtable,
        get_class_id: unsafe extern "system" fn(*mut c_void, *mut GUID) -> i32,
        is_dirty: unsafe extern "system" fn(*mut c_void) -> i32,
        load: unsafe extern "system" fn(*mut c_void, *const u16, u32) -> i32,
        save: unsafe extern "system" fn(*mut c_void, *const u16, i32) -> i32,
        save_completed: unsafe extern "system" fn(*mut c_void, *const u16) -> i32,
        get_current_file: unsafe extern "system" fn(*mut c_void, *mut *mut u16) -> i32,
    }

    #[cfg(feature = "private-windows-lifecycle-proof")]
    #[repr(C)]
    struct PropertyStoreVtable {
        unknown: UnknownVtable,
        get_count: unsafe extern "system" fn(*mut c_void, *mut u32) -> i32,
        get_at: unsafe extern "system" fn(*mut c_void, u32, *mut PROPERTYKEY) -> i32,
        get_value:
            unsafe extern "system" fn(*mut c_void, *const PROPERTYKEY, *mut PROPVARIANT) -> i32,
        set_value:
            unsafe extern "system" fn(*mut c_void, *const PROPERTYKEY, *const PROPVARIANT) -> i32,
        commit: unsafe extern "system" fn(*mut c_void) -> i32,
    }

    #[cfg(feature = "private-windows-lifecycle-proof")]
    struct ComPtr(*mut c_void);

    #[cfg(feature = "private-windows-lifecycle-proof")]
    impl ComPtr {
        fn query(&self, iid: &GUID, context: &str) -> Result<Self, String> {
            let mut value = ptr::null_mut();
            let vtable = unsafe { &**(self.0.cast::<*const UnknownVtable>()) };
            let result = unsafe { (vtable.query_interface)(self.0, iid, &mut value) };
            if result != 0 || value.is_null() {
                Err(format!("{context}:{result:#010x}"))
            } else {
                Ok(Self(value))
            }
        }

        fn shell_link(&self) -> &ShellLinkWVtable {
            unsafe { &**(self.0.cast::<*const ShellLinkWVtable>()) }
        }

        fn persist_file(&self) -> &PersistFileVtable {
            unsafe { &**(self.0.cast::<*const PersistFileVtable>()) }
        }

        fn property_store(&self) -> &PropertyStoreVtable {
            unsafe { &**(self.0.cast::<*const PropertyStoreVtable>()) }
        }
    }

    #[cfg(feature = "private-windows-lifecycle-proof")]
    impl Drop for ComPtr {
        fn drop(&mut self) {
            if !self.0.is_null() {
                let vtable = unsafe { &**(self.0.cast::<*const UnknownVtable>()) };
                unsafe { (vtable.release)(self.0) };
            }
        }
    }

    #[cfg(feature = "private-windows-lifecycle-proof")]
    struct ComApartment;

    #[cfg(feature = "private-windows-lifecycle-proof")]
    impl ComApartment {
        fn initialize() -> Result<Self, String> {
            let result = unsafe { CoInitializeEx(ptr::null(), COINIT_MULTITHREADED as u32) };
            if result < 0 {
                Err(format!(
                    "collector_service_proof_shortcut_com_initialize_failed:{result:#010x}"
                ))
            } else {
                Ok(Self)
            }
        }
    }

    #[cfg(feature = "private-windows-lifecycle-proof")]
    impl Drop for ComApartment {
        fn drop(&mut self) {
            unsafe { CoUninitialize() };
        }
    }

    #[derive(Debug)]
    pub(super) struct ServiceLifecycleMarker {
        _root: ProtectedEtwLeaseRoot,
        _file: OwnedHandle,
    }

    #[derive(Debug)]
    struct OwnedSecurityDescriptor(PSECURITY_DESCRIPTOR);

    impl OwnedSecurityDescriptor {
        fn from_sddl(value: &str) -> Result<Self, String> {
            let value = wide(value);
            let mut descriptor = ptr::null_mut();
            if unsafe {
                ConvertStringSecurityDescriptorToSecurityDescriptorW(
                    value.as_ptr(),
                    SECURITY_DESCRIPTOR_REVISION_1,
                    &mut descriptor,
                    ptr::null_mut(),
                )
            } == 0
            {
                return Err(last_error("collector_service_root_sddl_invalid"));
            }
            Ok(Self(descriptor))
        }

        fn attributes(&mut self) -> SECURITY_ATTRIBUTES {
            SECURITY_ATTRIBUTES {
                nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
                lpSecurityDescriptor: self.0.cast(),
                bInheritHandle: 0,
            }
        }
    }

    impl Drop for OwnedSecurityDescriptor {
        fn drop(&mut self) {
            if !self.0.is_null() {
                unsafe {
                    LocalFree(self.0.cast());
                }
            }
        }
    }

    struct OwnedSecurityInfo {
        descriptor: PSECURITY_DESCRIPTOR,
        owner: PSID,
        dacl: *mut windows_sys::Win32::Security::ACL,
    }

    impl OwnedSecurityInfo {
        fn read(handle: HANDLE, context: &str) -> Result<Self, String> {
            let mut owner = ptr::null_mut();
            let mut dacl = ptr::null_mut();
            let mut descriptor = ptr::null_mut();
            let status = unsafe {
                GetSecurityInfo(
                    handle,
                    SE_FILE_OBJECT,
                    OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION,
                    &mut owner,
                    ptr::null_mut(),
                    &mut dacl,
                    ptr::null_mut(),
                    &mut descriptor,
                )
            };
            if status != 0 || descriptor.is_null() || owner.is_null() || dacl.is_null() {
                if !descriptor.is_null() {
                    unsafe { LocalFree(descriptor.cast()) };
                }
                return Err(format!("{context}:{status}"));
            }
            Ok(Self {
                descriptor,
                owner,
                dacl,
            })
        }
    }

    impl Drop for OwnedSecurityInfo {
        fn drop(&mut self) {
            unsafe {
                LocalFree(self.descriptor.cast());
            }
        }
    }

    #[derive(Clone)]
    struct OwnedSid(Vec<u8>);

    impl OwnedSid {
        fn as_psid(&self) -> PSID {
            self.0.as_ptr().cast_mut().cast()
        }
    }

    struct SecurityPrincipals {
        system: OwnedSid,
        administrators: OwnedSid,
        trusted_installer: OwnedSid,
        interactive: OwnedSid,
        service: Option<OwnedSid>,
    }

    impl SecurityPrincipals {
        fn load_base() -> Result<Self, String> {
            Ok(Self {
                system: well_known_sid(WinLocalSystemSid)?,
                administrators: well_known_sid(WinBuiltinAdministratorsSid)?,
                trusted_installer: account_sid(r"NT SERVICE\TrustedInstaller")?,
                interactive: well_known_sid(WinInteractiveSid)?,
                service: None,
            })
        }

        fn load_with_service() -> Result<Self, String> {
            let mut principals = Self::load_base()?;
            principals.service = Some(sid_from_string(SERVICE_SID)?);
            Ok(principals)
        }

        fn service(&self) -> Result<&OwnedSid, String> {
            self.service
                .as_ref()
                .ok_or_else(|| "collector_service_sid_unavailable".to_string())
        }

        fn classify(&self, sid: PSID) -> PrincipalClass {
            if unsafe { EqualSid(sid, self.system.as_psid()) } != 0 {
                PrincipalClass::LocalSystem
            } else if unsafe { EqualSid(sid, self.administrators.as_psid()) } != 0 {
                PrincipalClass::Administrators
            } else if unsafe { EqualSid(sid, self.trusted_installer.as_psid()) } != 0 {
                PrincipalClass::TrustedInstaller
            } else if unsafe { EqualSid(sid, self.interactive.as_psid()) } != 0 {
                PrincipalClass::InteractiveUsers
            } else if self
                .service
                .as_ref()
                .is_some_and(|service| unsafe { EqualSid(sid, service.as_psid()) } != 0)
            {
                PrincipalClass::CollectorService
            } else {
                PrincipalClass::Other
            }
        }
    }

    pub(super) fn acquire_service_lifecycle_marker() -> Result<ServiceLifecycleMarker, String> {
        let root = open_protected_etw_lease_root()?;
        let roots = fixed_roots()?;
        let path = wide_path(&roots.service.join(SERVICE_LIFECYCLE_LOCK_FILE_NAME));
        let file = OwnedHandle::new(
            unsafe {
                CreateFileW(
                    path.as_ptr(),
                    FILE_READ_ATTRIBUTES | FILE_WRITE_DATA | READ_CONTROL,
                    0,
                    ptr::null(),
                    OPEN_ALWAYS,
                    FILE_FLAG_OPEN_REPARSE_POINT,
                    ptr::null_mut(),
                )
            },
            "collector_service_lifecycle_file_acquire_failed",
        )?;
        let info = file_information(file.raw(), "collector_service_lifecycle_file_info_failed")?;
        if info.dwFileAttributes & (FILE_ATTRIBUTE_DIRECTORY | FILE_ATTRIBUTE_REPARSE_POINT) != 0 {
            return Err("collector_service_lifecycle_file_untrusted".to_string());
        }
        validate_no_untrusted_writer(
            file.raw(),
            &SecurityPrincipals::load_with_service()?,
            true,
            true,
        )?;
        Ok(ServiceLifecycleMarker {
            _root: root,
            _file: file,
        })
    }

    fn require_service_lifecycle_active() -> Result<(), String> {
        match try_open_service_lifecycle_file()? {
            LifecycleFileProbe::Locked => Ok(()),
            LifecycleFileProbe::Missing => {
                Err("collector_service_lifecycle_file_missing".to_string())
            }
            LifecycleFileProbe::Opened(_) => {
                Err("collector_service_lifecycle_file_not_owned".to_string())
            }
        }
    }

    fn prove_service_lifecycle_settled(required: bool) -> Result<(), String> {
        let deadline = Instant::now() + SERVICE_OPERATION_TIMEOUT;
        loop {
            match try_open_service_lifecycle_file()? {
                LifecycleFileProbe::Opened(file) => {
                    let principals = SecurityPrincipals::load_with_service()?;
                    let info = file_information(
                        file.raw(),
                        "collector_service_lifecycle_file_info_failed",
                    )?;
                    if info.dwFileAttributes
                        & (FILE_ATTRIBUTE_DIRECTORY | FILE_ATTRIBUTE_REPARSE_POINT)
                        != 0
                    {
                        return Err("collector_service_lifecycle_file_untrusted".to_string());
                    }
                    validate_no_untrusted_writer(file.raw(), &principals, true, true)?;
                    return Ok(());
                }
                LifecycleFileProbe::Missing if !required => return Ok(()),
                LifecycleFileProbe::Missing => {
                    return Err("collector_service_lifecycle_file_missing".to_string());
                }
                LifecycleFileProbe::Locked if Instant::now() < deadline => {
                    thread::sleep(SERVICE_POLL_INTERVAL);
                }
                LifecycleFileProbe::Locked => {
                    return Err("collector_service_lifecycle_exit_unproven".to_string());
                }
            }
        }
    }

    enum LifecycleFileProbe {
        Missing,
        Locked,
        Opened(OwnedHandle),
    }

    fn try_open_service_lifecycle_file() -> Result<LifecycleFileProbe, String> {
        let roots = fixed_roots()?;
        let path = wide_path(&roots.service.join(SERVICE_LIFECYCLE_LOCK_FILE_NAME));
        let file = unsafe {
            CreateFileW(
                path.as_ptr(),
                SERVICE_LIFECYCLE_PROBE_ACCESS,
                0,
                ptr::null(),
                OPEN_EXISTING,
                FILE_FLAG_OPEN_REPARSE_POINT,
                ptr::null_mut(),
            )
        };
        if !file.is_null() && file != (-1_isize as HANDLE) {
            return Ok(LifecycleFileProbe::Opened(OwnedHandle(file)));
        }
        let error = unsafe { GetLastError() };
        match error {
            error if is_missing_path_error(error) => Ok(LifecycleFileProbe::Missing),
            ERROR_SHARING_VIOLATION => Ok(LifecycleFileProbe::Locked),
            _ => Err(format!(
                "collector_service_lifecycle_file_probe_failed:{error}"
            )),
        }
    }

    pub(super) fn path_exists_no_follow(path: &Path) -> Result<bool, String> {
        let path = wide_path(path);
        let handle = unsafe {
            CreateFileW(
                path.as_ptr(),
                FILE_READ_ATTRIBUTES,
                FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
                ptr::null(),
                OPEN_EXISTING,
                FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT,
                ptr::null_mut(),
            )
        };
        if !handle.is_null() && handle != (-1_isize as HANDLE) {
            drop(OwnedHandle(handle));
            return Ok(true);
        }
        let error = unsafe { GetLastError() };
        if is_missing_path_error(error) {
            Ok(false)
        } else {
            Err(format!("collector_service_residue_probe_failed:{error}"))
        }
    }

    fn retire_legacy_cli(image: &VerifiedServiceImage) -> Result<(), String> {
        let install_directory = image
            .path()
            .parent()
            .ok_or_else(|| "collector_service_install_directory_missing".to_string())?;
        let principals = SecurityPrincipals::load_base()?;
        retire_legacy_cli_path(
            &install_directory.join(LEGACY_WINDOWS_CLI_NAME),
            &LEGACY_WINDOWS_CLI_IMAGES,
            Some(&principals),
        )
    }

    fn retire_staged_upgrade_image(image: &VerifiedServiceImage) -> Result<(), String> {
        let install_directory = image
            .path()
            .parent()
            .ok_or_else(|| "collector_service_install_directory_missing".to_string())?;
        let stable_digest = trusted_file_digest(image.path(), "collector_service_stable_image")?;
        for entry in fs::read_dir(install_directory).map_err(|error| {
            format!("collector_service_upgrade_install_directory_read_failed:{error}")
        })? {
            let entry = entry.map_err(|error| {
                format!("collector_service_upgrade_install_directory_entry_failed:{error}")
            })?;
            let name = entry.file_name();
            if name.to_str().is_some_and(is_staged_upgrade_name) {
                delete_trusted_leaf(
                    &entry.path(),
                    Some(stable_digest),
                    "collector_service_staged_image",
                )?;
            }
        }
        cleanup_upgrade_install_residue(install_directory, None)
    }

    fn open_file_for_hash(
        path: &Path,
        principals: &SecurityPrincipals,
        context: &str,
    ) -> Result<OwnedHandle, String> {
        let path_wide = wide_path(path);
        let handle = OwnedHandle::new(
            unsafe {
                CreateFileW(
                    path_wide.as_ptr(),
                    GENERIC_READ | READ_CONTROL,
                    FILE_SHARE_READ,
                    ptr::null(),
                    OPEN_EXISTING,
                    FILE_FLAG_OPEN_REPARSE_POINT,
                    ptr::null_mut(),
                )
            },
            context,
        )?;
        let info = file_information(handle.raw(), context)?;
        if info.dwFileAttributes & (FILE_ATTRIBUTE_DIRECTORY | FILE_ATTRIBUTE_REPARSE_POINT) != 0
            || !fixed_path_eq(&final_path(&handle, context)?, path)
        {
            return Err(format!("{context}_untrusted"));
        }
        validate_no_untrusted_writer(handle.raw(), principals, false, false)
            .map_err(|_| format!("{context}_untrusted"))?;
        Ok(handle)
    }

    fn file_size(info: &BY_HANDLE_FILE_INFORMATION) -> u64 {
        (u64::from(info.nFileSizeHigh) << 32) | u64::from(info.nFileSizeLow)
    }

    fn trusted_file_digest(path: &Path, context: &str) -> Result<[u8; 32], String> {
        let principals = SecurityPrincipals::load_base()?;
        let file = open_file_for_hash(path, &principals, context)?;
        let info = file_information(file.raw(), context)?;
        hash_open_file(file.raw(), file_size(&info))
    }

    fn delete_trusted_leaf(
        path: &Path,
        expected_digest: Option<[u8; 32]>,
        context: &str,
    ) -> Result<(), String> {
        let path_wide = wide_path(path);
        let raw = unsafe {
            CreateFileW(
                path_wide.as_ptr(),
                GENERIC_READ | DELETE | READ_CONTROL,
                FILE_SHARE_READ,
                ptr::null(),
                OPEN_EXISTING,
                FILE_FLAG_OPEN_REPARSE_POINT,
                ptr::null_mut(),
            )
        };
        if raw.is_null() || raw == (-1_isize as HANDLE) {
            let error = unsafe { GetLastError() };
            return if is_missing_path_error(error) {
                Ok(())
            } else {
                Err(format!("{context}_open_failed:{error}"))
            };
        }
        let principals = SecurityPrincipals::load_base()?;
        let file = OwnedHandle(raw);
        let info = file_information(file.raw(), context)?;
        if info.dwFileAttributes & (FILE_ATTRIBUTE_DIRECTORY | FILE_ATTRIBUTE_REPARSE_POINT) != 0
            || !fixed_path_eq(&final_path(&file, context)?, path)
        {
            return Err(format!("{context}_identity_invalid"));
        }
        validate_no_untrusted_writer(file.raw(), &principals, false, false)
            .map_err(|_| format!("{context}_identity_invalid"))?;
        if let Some(expected_digest) = expected_digest {
            if hash_open_file(file.raw(), file_size(&info))? != expected_digest {
                return Err(format!("{context}_identity_invalid"));
            }
        }
        let disposition = FILE_DISPOSITION_INFO { DeleteFile: true };
        if unsafe {
            SetFileInformationByHandle(
                file.raw(),
                FileDispositionInfo,
                (&disposition as *const FILE_DISPOSITION_INFO).cast(),
                size_of::<FILE_DISPOSITION_INFO>() as u32,
            )
        } == 0
        {
            return Err(last_error(&format!("{context}_remove_failed")));
        }
        drop(file);
        if path_exists_no_follow(path)? {
            Err(format!("{context}_present"))
        } else {
            Ok(())
        }
    }

    fn upgrade_journal_path() -> Result<PathBuf, String> {
        Ok(fixed_roots()?.service.join(UPGRADE_JOURNAL_FILE_NAME))
    }

    fn read_upgrade_journal() -> Result<Option<UpgradeJournalV1>, String> {
        let path = upgrade_journal_path()?;
        let principals = SecurityPrincipals::load_with_service()?;
        if verify_optional_leaf(&path, &principals)?.is_none() {
            return Ok(None);
        }
        let bytes = fs::read(&path)
            .map_err(|error| format!("collector_service_upgrade_journal_read_failed:{error}"))?;
        if bytes.len() > UPGRADE_JOURNAL_MAX_BYTES {
            return Err("collector_service_upgrade_journal_too_large".to_string());
        }
        let journal = serde_json::from_slice::<UpgradeJournalV1>(&bytes)
            .map_err(|error| format!("collector_service_upgrade_journal_parse_failed:{error}"))?;
        journal.validate().map_err(str::to_string)?;
        Ok(Some(journal))
    }

    fn write_upgrade_journal(journal: &UpgradeJournalV1) -> Result<(), String> {
        journal.validate().map_err(str::to_string)?;
        let path = upgrade_journal_path()?;
        let bytes = serde_json::to_vec(journal).map_err(|error| {
            format!("collector_service_upgrade_journal_serialize_failed:{error}")
        })?;
        crate::atomic_json::write_bytes_atomic(&path, &bytes).map_err(|error| {
            format!(
                "collector_service_upgrade_journal_write_failed:{:?}:{}",
                error.operation, error.error
            )
        })?;
        let principals = SecurityPrincipals::load_with_service()?;
        verify_optional_leaf(&path, &principals)?
            .ok_or_else(|| "collector_service_upgrade_journal_missing".to_string())?;
        Ok(())
    }

    fn delete_upgrade_journal() -> Result<(), String> {
        let path = upgrade_journal_path()?;
        let principals = SecurityPrincipals::load_with_service()?;
        drop(verify_optional_leaf(&path, &principals)?);
        let path_wide = wide_path(&path);
        if unsafe { DeleteFileW(path_wide.as_ptr()) } == 0 {
            let error = unsafe { GetLastError() };
            if !is_missing_path_error(error) {
                return Err(format!(
                    "collector_service_upgrade_journal_remove_failed:{error}"
                ));
            }
        }
        Ok(())
    }

    fn ensure_upgrade_backup(
        stable: &Path,
        install_directory: &Path,
        old_digest: [u8; 32],
    ) -> Result<String, String> {
        let backup_name = upgrade_backup_name(&old_digest);
        let backup = install_directory.join(&backup_name);
        cleanup_upgrade_backup_temps(install_directory, &backup_name)?;
        if path_exists_no_follow(&backup)?
            && trusted_file_digest(&backup, "collector_service_upgrade_backup")? == old_digest
        {
            return Ok(backup_name);
        }
        let payload = fs::read(stable)
            .map_err(|error| format!("collector_service_upgrade_backup_read_failed:{error}"))?;
        let payload_digest: [u8; 32] = Sha256::digest(&payload).into();
        if payload_digest != old_digest {
            return Err("collector_service_upgrade_stable_image_changed".to_string());
        }
        crate::atomic_json::write_bytes_atomic(&backup, &payload).map_err(|error| {
            format!(
                "collector_service_upgrade_backup_write_failed:{:?}:{}",
                error.operation, error.error
            )
        })?;
        if trusted_file_digest(&backup, "collector_service_upgrade_backup")? != old_digest {
            return Err("collector_service_upgrade_backup_identity_invalid".to_string());
        }
        Ok(backup_name)
    }

    fn cleanup_upgrade_backup_temps(
        install_directory: &Path,
        backup_name: &str,
    ) -> Result<(), String> {
        for entry in fs::read_dir(install_directory).map_err(|error| {
            format!("collector_service_upgrade_install_directory_read_failed:{error}")
        })? {
            let entry = entry.map_err(|error| {
                format!("collector_service_upgrade_install_directory_entry_failed:{error}")
            })?;
            let name = entry.file_name();
            let Some(name) = name.to_str() else {
                continue;
            };
            if atomic_temp_suffix(name, backup_name).is_some() {
                delete_trusted_leaf(&entry.path(), None, "collector_service_upgrade_backup_temp")?;
            }
        }
        Ok(())
    }

    fn cleanup_upgrade_install_residue(
        install_directory: &Path,
        keep_staged_name: Option<&str>,
    ) -> Result<(), String> {
        for entry in fs::read_dir(install_directory).map_err(|error| {
            format!("collector_service_upgrade_install_directory_read_failed:{error}")
        })? {
            let entry = entry.map_err(|error| {
                format!("collector_service_upgrade_install_directory_entry_failed:{error}")
            })?;
            let name = entry.file_name();
            let Some(name) = name.to_str() else {
                continue;
            };
            if keep_staged_name == Some(name) {
                continue;
            }
            if is_staged_upgrade_name(name) || name == "batcave-collector-service.rollback.tmp" {
                delete_trusted_leaf(&entry.path(), None, "collector_service_upgrade_residue")?;
                continue;
            }
            if let Some(expected_digest) = rollback_digest_from_name(name) {
                delete_trusted_leaf(
                    &entry.path(),
                    Some(expected_digest),
                    "collector_service_upgrade_residue",
                )?;
                continue;
            }
            let Some(base) = atomic_temp_base(name) else {
                continue;
            };
            if rollback_digest_from_name(base).is_some() {
                delete_trusted_leaf(&entry.path(), None, "collector_service_upgrade_residue")?;
            }
        }
        Ok(())
    }

    pub(super) fn rollback_digest_from_name(name: &str) -> Option<[u8; 32]> {
        let digest = name
            .strip_prefix("batcave-collector-service.")?
            .strip_suffix(".rollback.exe")?;
        if digest.len() != 64 || !digest.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return None;
        }
        let mut bytes = [0_u8; 32];
        for (index, byte) in bytes.iter_mut().enumerate() {
            *byte = u8::from_str_radix(&digest[index * 2..index * 2 + 2], 16).ok()?;
        }
        Some(bytes)
    }

    pub(super) fn atomic_temp_base(name: &str) -> Option<&str> {
        let without_suffix = name.strip_suffix(".tmp")?;
        let (without_sequence, sequence) = without_suffix.rsplit_once('.')?;
        let (base, process_id) = without_sequence.rsplit_once('.')?;
        (!base.is_empty()
            && !process_id.is_empty()
            && process_id.bytes().all(|byte| byte.is_ascii_digit())
            && !sequence.is_empty()
            && sequence.bytes().all(|byte| byte.is_ascii_digit()))
        .then_some(base)
    }

    fn restore_upgrade_backup(
        stable: &Path,
        backup: &Path,
        expected_digest: [u8; 32],
    ) -> Result<(), String> {
        if trusted_file_digest(backup, "collector_service_upgrade_backup")? != expected_digest {
            return Err("collector_service_upgrade_backup_identity_invalid".to_string());
        }
        let temp = stable.with_file_name("batcave-collector-service.rollback.tmp");
        fs::copy(backup, &temp)
            .map_err(|error| format!("collector_service_upgrade_restore_copy_failed:{error}"))?;
        if trusted_file_digest(&temp, "collector_service_upgrade_restore_temp")? != expected_digest
        {
            return Err("collector_service_upgrade_restore_identity_invalid".to_string());
        }
        let temp_wide = wide_path(&temp);
        let stable_wide = wide_path(stable);
        if unsafe {
            MoveFileExW(
                temp_wide.as_ptr(),
                stable_wide.as_ptr(),
                MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
            )
        } == 0
        {
            return Err(last_error(
                "collector_service_upgrade_restore_replace_failed",
            ));
        }
        if trusted_file_digest(stable, "collector_service_stable_image")? != expected_digest {
            return Err("collector_service_upgrade_restore_identity_invalid".to_string());
        }
        Ok(())
    }

    fn delete_upgrade_backup(
        install_directory: &Path,
        journal: &UpgradeJournalV1,
    ) -> Result<(), String> {
        let path = install_directory.join(&journal.backup_name);
        delete_trusted_leaf(
            &path,
            Some(journal.old_digest),
            "collector_service_upgrade_backup",
        )
    }

    pub(super) fn retire_upgrade_artifacts(
        delete_backup: impl FnOnce() -> Result<(), String>,
        delete_staged: impl FnOnce() -> Result<(), String>,
        cleanup_residue: impl FnOnce() -> Result<(), String>,
        delete_journal: impl FnOnce() -> Result<(), String>,
    ) -> Result<(), String> {
        delete_backup()?;
        delete_staged()?;
        cleanup_residue()?;
        delete_journal()
    }

    fn retire_legacy_cli_path(
        path: &Path,
        known_images: &[LegacyCliImage],
        principals: Option<&SecurityPrincipals>,
    ) -> Result<(), String> {
        let path_wide = wide_path(path);
        let raw = unsafe {
            CreateFileW(
                path_wide.as_ptr(),
                GENERIC_READ | DELETE | READ_CONTROL,
                FILE_SHARE_READ,
                ptr::null(),
                OPEN_EXISTING,
                FILE_FLAG_OPEN_REPARSE_POINT,
                ptr::null_mut(),
            )
        };
        if raw.is_null() || raw == (-1_isize as HANDLE) {
            let error = unsafe { GetLastError() };
            return if is_missing_path_error(error) {
                Ok(())
            } else {
                Err(format!("collector_service_legacy_cli_open_failed:{error}"))
            };
        }
        let file = OwnedHandle(raw);
        let info = file_information(file.raw(), "collector_service_legacy_cli_info_failed")?;
        if info.dwFileAttributes & (FILE_ATTRIBUTE_DIRECTORY | FILE_ATTRIBUTE_REPARSE_POINT) != 0
            || !fixed_path_eq(
                &final_path(&file, "collector_service_legacy_cli_final_path_failed")?,
                path,
            )
        {
            return Err("collector_service_legacy_cli_residue_untrusted".to_string());
        }
        if let Some(principals) = principals {
            validate_no_untrusted_writer(file.raw(), principals, false, false)
                .map_err(|_| "collector_service_legacy_cli_residue_untrusted".to_string())?;
        }

        let size = file_size(&info);
        if !known_images.iter().any(|image| image.size == size) {
            return Err("collector_service_legacy_cli_residue_untrusted".to_string());
        }
        let digest = hash_open_file(file.raw(), size)?;
        if !legacy_cli_image_matches(known_images, size, &digest) {
            return Err("collector_service_legacy_cli_residue_untrusted".to_string());
        }

        let disposition = FILE_DISPOSITION_INFO { DeleteFile: true };
        if unsafe {
            SetFileInformationByHandle(
                file.raw(),
                FileDispositionInfo,
                (&disposition as *const FILE_DISPOSITION_INFO).cast(),
                size_of::<FILE_DISPOSITION_INFO>() as u32,
            )
        } == 0
        {
            return Err(last_error("collector_service_legacy_cli_remove_failed"));
        }
        drop(file);
        if path_exists_no_follow(path)? {
            Err("collector_service_legacy_cli_residue_present".to_string())
        } else {
            Ok(())
        }
    }

    fn hash_open_file(handle: HANDLE, size: u64) -> Result<[u8; 32], String> {
        let mut remaining = size;
        let mut digest = Sha256::new();
        let mut buffer = [0_u8; 64 * 1024];
        while remaining > 0 {
            let requested = remaining.min(buffer.len() as u64) as u32;
            let mut read = 0_u32;
            if unsafe {
                ReadFile(
                    handle,
                    buffer.as_mut_ptr().cast(),
                    requested,
                    &mut read,
                    ptr::null_mut(),
                )
            } == 0
            {
                return Err(last_error("collector_service_legacy_cli_read_failed"));
            }
            if read == 0 {
                return Err("collector_service_legacy_cli_read_incomplete".to_string());
            }
            digest.update(&buffer[..read as usize]);
            remaining = remaining.saturating_sub(u64::from(read));
        }
        Ok(digest.finalize().into())
    }

    #[cfg(test)]
    pub(super) fn retire_legacy_cli_fixture(path: &Path, expected: &[u8]) -> Result<(), String> {
        let image = LegacyCliImage {
            size: expected.len() as u64,
            sha256: Sha256::digest(expected).into(),
        };
        retire_legacy_cli_path(path, &[image], None)
    }

    #[cfg(test)]
    pub(super) fn lifecycle_probe_requests_write_access() -> bool {
        SERVICE_LIFECYCLE_PROBE_ACCESS & FILE_WRITE_DATA != 0
    }

    #[cfg(test)]
    pub(super) fn configured_service_sid() -> Result<String, String> {
        sid_string(&sid_from_string(SERVICE_SID)?)
    }

    pub(super) fn retire_installer_shortcuts() -> Result<(), String> {
        require_elevated()?;
        let controller = verify_current_installer_controller()?;
        let result = retire_shortcuts_with_controller(&controller);
        let install_directory = controller.install_directory()?.to_path_buf();
        drop(controller);
        match result {
            Ok(()) => Ok(()),
            Err(error) => fail_shortcut_retirement_with_upgrade_recovery(error, &install_directory),
        }
    }

    fn retire_shortcuts_with_controller(controller: &VerifiedServiceImage) -> Result<(), String> {
        let monitor_path = controller
            .install_directory()?
            .join(MONITOR_EXECUTABLE_NAME);
        super::super::windows_shortcut_retirement::retire_shared_legacy_shortcuts(&monitor_path)
    }

    fn fail_shortcut_retirement_with_upgrade_recovery(
        error: String,
        install_directory: &Path,
    ) -> Result<(), String> {
        let mut journal = match read_upgrade_journal() {
            Ok(Some(journal)) => journal,
            Ok(None) => return Err(error),
            Err(recovery) => {
                return Err(format!(
                    "{error};collector_service_upgrade_rollback_failed:{recovery}"
                ))
            }
        };
        let stable = install_directory.join(SERVICE_EXECUTABLE_NAME);
        let recovery = (|| {
            let manager = open_manager(SC_MANAGER_CONNECT)?;
            let service = open_service(&manager, SERVICE_ALL_ACCESS)?
                .ok_or_else(|| "collector_service_upgrade_service_missing".to_string())?;
            validate_service_contract(&service, &stable)?;
            let _protected_root = open_protected_etw_lease_root()?;
            rollback_upgrade(&mut journal, &service, &stable)
        })();
        match recovery {
            Ok(()) => Err(error),
            Err(recovery) => Err(format!(
                "{error};collector_service_upgrade_rollback_failed:{recovery}"
            )),
        }
    }

    pub(super) fn prepare_upgrade() -> Result<(), String> {
        require_elevated()?;
        let image = verify_current_binary_path()?;
        let manager = open_manager(SC_MANAGER_CONNECT)?;
        let service = open_service(&manager, SERVICE_ALL_ACCESS)?
            .ok_or_else(|| "collector_service_upgrade_service_missing".to_string())?;
        validate_service_contract(&service, image.path())?;
        let _protected_root = open_protected_etw_lease_root()?;
        stop_service_and_wait(&service, true)
    }

    pub(super) fn prepare_upgrade_staged() -> Result<(), String> {
        require_elevated()?;
        let staged = verify_current_staged_binary_path()?;
        let stable = staged
            .path()
            .parent()
            .ok_or_else(|| "collector_service_install_directory_missing".to_string())?
            .join(SERVICE_EXECUTABLE_NAME);
        let manager = open_manager(SC_MANAGER_CONNECT)?;
        let service = open_service(&manager, SERVICE_ALL_ACCESS)?
            .ok_or_else(|| "collector_service_upgrade_service_missing".to_string())?;
        prepare_upgrade_staged_image(&staged, &stable, &service)
    }

    fn prepare_upgrade_staged_image(
        staged: &VerifiedServiceImage,
        stable: &Path,
        service: &OwnedScHandle,
    ) -> Result<(), String> {
        validate_service_contract(service, stable)?;
        let _protected_root = open_protected_etw_lease_root()?;
        let prepared = resume_upgrade_transaction(staged, stable, service)?;
        let prior_digest = trusted_file_digest(stable, "collector_service_stable_image")?;
        ensure_service_generation_ready(service, stable, prior_digest)?;
        retire_shortcuts_with_controller(staged)?;
        ensure_uninstaller_compatibility_alias(staged)?;
        let result = (|| {
            settle_service_for_replacement(service)?;
            if prepared {
                Ok(())
            } else {
                prepare_upgrade_transaction(staged, stable)
            }
        })();
        if let Err(error) = result {
            return match ensure_service_generation_ready(service, stable, prior_digest) {
                Ok(()) => Err(error),
                Err(restart) => Err(format!(
                    "{error};collector_service_upgrade_restart_failed:{restart}"
                )),
            };
        }
        Ok(())
    }

    pub(super) fn commit_upgrade_staged() -> Result<(), String> {
        require_elevated()?;
        let staged = verify_current_staged_binary_path()?;
        let stable = staged
            .path()
            .parent()
            .ok_or_else(|| "collector_service_install_directory_missing".to_string())?
            .join(SERVICE_EXECUTABLE_NAME);
        let manager = open_manager(SC_MANAGER_CONNECT)?;
        let service = open_service(&manager, SERVICE_ALL_ACCESS)?
            .ok_or_else(|| "collector_service_upgrade_service_missing".to_string())?;
        validate_service_contract(&service, &stable)?;
        let _protected_root = open_protected_etw_lease_root()?;
        let mut journal = read_upgrade_journal()?
            .ok_or_else(|| "collector_service_upgrade_journal_missing".to_string())?;
        let action = upgrade_resume_action(&journal, &stable, staged.path())?;
        match action {
            UpgradeResumeAction::CommitCandidate | UpgradeResumeAction::ReusePreparedSameImage => {
                commit_upgrade_candidate(&mut journal, &service, &stable, || {
                    retire_shortcuts_with_controller(&staged)
                })
            }
            UpgradeResumeAction::FinalizeVerified => Ok(()),
            UpgradeResumeAction::ReusePrepared | UpgradeResumeAction::RetryRollback => {
                Err("collector_service_upgrade_candidate_not_installed".to_string())
            }
        }
    }

    pub(super) fn install() -> Result<(), String> {
        require_elevated()?;
        let image = verify_current_binary_path()?;
        let manager = open_manager(SC_MANAGER_CONNECT | SC_MANAGER_CREATE_SERVICE)?;
        if let Some(service) = open_service(&manager, SERVICE_ALL_ACCESS)? {
            validate_service_contract(&service, image.path())?;
            let _protected_root = open_protected_etw_lease_root()?;
            start_service_and_wait(&service)?;
            validate_service_contract(&service, image.path())?;
            if let Err(error) = retire_shortcuts_with_controller(&image) {
                let install_directory = image.install_directory()?.to_path_buf();
                drop(image);
                return fail_shortcut_retirement_with_upgrade_recovery(error, &install_directory);
            }
            finalize_upgrade_transaction(&image)?;
            retire_legacy_cli(&image)?;
            return retire_staged_upgrade_image(&image);
        }

        retire_upgrade_transaction_for_uninstall(image.path(), None)?;
        let service = create_service(&manager, image.path())?;
        let mut roots_created = RootCreationJournal::default();
        let install_result = (|| {
            configure_new_service(&service)?;
            set_owner_marker()?;
            provision_roots(&mut roots_created)?;
            validate_service_contract(&service, image.path())?;
            start_service_and_wait(&service)?;
            validate_service_contract(&service, image.path())?;
            retire_shortcuts_with_controller(&image)
        })();
        if let Err(error) = install_result {
            if let Err(rollback) = rollback_new_install(
                service,
                &manager,
                roots_created.product,
                roots_created.service,
            ) {
                return Err(format!(
                    "{error};collector_service_rollback_failed:{rollback}"
                ));
            }
            return Err(error);
        }
        finalize_upgrade_transaction(&image)?;
        retire_legacy_cli(&image)?;
        retire_staged_upgrade_image(&image)
    }

    fn prepare_upgrade_transaction(
        staged: &VerifiedServiceImage,
        stable: &Path,
    ) -> Result<(), String> {
        if read_upgrade_journal()?.is_some() {
            return Err("collector_service_upgrade_journal_already_exists".to_string());
        }
        let install_directory = stable
            .parent()
            .ok_or_else(|| "collector_service_install_directory_missing".to_string())?;
        let old_digest = trusted_file_digest(stable, "collector_service_stable_image")?;
        let new_digest = trusted_file_digest(staged.path(), "collector_service_staged_image")?;
        let backup_name = ensure_upgrade_backup(stable, install_directory, old_digest)?;
        let journal = UpgradeJournalV1::new(
            old_digest,
            new_digest,
            backup_name,
            staged_service_executable_name(),
        );
        write_upgrade_journal(&journal)
    }

    fn ensure_uninstaller_compatibility_alias(staged: &VerifiedServiceImage) -> Result<(), String> {
        let Some(installed_version) = read_installed_product_version()? else {
            return Ok(());
        };
        let alias_name = format!("batcave-collector-service.{installed_version}.staged.exe");
        if !is_staged_upgrade_name(&alias_name) {
            return Err("collector_service_installed_version_invalid".to_string());
        }
        let install_directory = staged
            .path()
            .parent()
            .ok_or_else(|| "collector_service_install_directory_missing".to_string())?;
        let alias = install_directory.join(&alias_name);
        if fixed_path_eq(&alias, staged.path()) {
            return Ok(());
        }
        let payload = fs::read(staged.path()).map_err(|error| {
            format!("collector_service_recovery_alias_source_read_failed:{error}")
        })?;
        let expected_digest: [u8; 32] = Sha256::digest(&payload).into();
        crate::atomic_json::write_bytes_atomic(&alias, &payload).map_err(|error| {
            format!(
                "collector_service_recovery_alias_write_failed:{:?}:{}",
                error.operation, error.error
            )
        })?;
        if trusted_file_digest(&alias, "collector_service_recovery_alias")? != expected_digest {
            return Err("collector_service_recovery_alias_identity_invalid".to_string());
        }
        Ok(())
    }

    fn resume_upgrade_transaction(
        staged: &VerifiedServiceImage,
        stable: &Path,
        service: &OwnedScHandle,
    ) -> Result<bool, String> {
        let Some(mut journal) = read_upgrade_journal()? else {
            cleanup_upgrade_install_residue(
                stable
                    .parent()
                    .ok_or_else(|| "collector_service_install_directory_missing".to_string())?,
                staged.path().file_name().and_then(|name| name.to_str()),
            )?;
            return Ok(false);
        };
        let current_staged_name = staged.path().file_name().and_then(|name| name.to_str());
        let current_staged_digest =
            trusted_file_digest(staged.path(), "collector_service_staged_image")?;
        if !staged_transaction_matches(&journal, current_staged_name, current_staged_digest) {
            resolve_superseded_upgrade_transaction(
                &mut journal,
                service,
                stable,
                current_staged_name,
            )?;
            cleanup_upgrade_install_residue(
                stable
                    .parent()
                    .ok_or_else(|| "collector_service_install_directory_missing".to_string())?,
                current_staged_name,
            )?;
            return Ok(false);
        }
        match upgrade_resume_action(&journal, stable, staged.path())? {
            UpgradeResumeAction::ReusePrepared | UpgradeResumeAction::ReusePreparedSameImage => {
                ensure_service_generation_ready(service, stable, journal.old_digest)?;
                Ok(true)
            }
            UpgradeResumeAction::CommitCandidate => {
                commit_upgrade_candidate(&mut journal, service, stable, || {
                    retire_shortcuts_with_controller(staged)
                })?;
                finalize_verified_upgrade(stable, staged.path(), &journal)?;
                Ok(false)
            }
            UpgradeResumeAction::RetryRollback => {
                rollback_upgrade(&mut journal, service, stable)?;
                Ok(true)
            }
            UpgradeResumeAction::FinalizeVerified => {
                if let Err(error) = ensure_verified_candidate_ready(&mut journal, service, stable) {
                    if journal.phase == UpgradePhase::Prepared {
                        return Ok(true);
                    }
                    return Err(error);
                }
                if let Err(error) = retire_shortcuts_with_controller(staged) {
                    return match rollback_upgrade(&mut journal, service, stable) {
                        Ok(()) => Err(error),
                        Err(rollback) => Err(format!(
                            "{error};collector_service_upgrade_rollback_failed:{rollback}"
                        )),
                    };
                }
                finalize_verified_upgrade(stable, staged.path(), &journal)?;
                Ok(false)
            }
        }
    }

    fn upgrade_resume_action(
        journal: &UpgradeJournalV1,
        stable: &Path,
        staged: &Path,
    ) -> Result<UpgradeResumeAction, String> {
        if staged.file_name().and_then(|name| name.to_str()) != Some(journal.staged_name.as_str()) {
            return Err("collector_service_upgrade_journal_identity_invalid".to_string());
        }
        let install_directory = stable
            .parent()
            .ok_or_else(|| "collector_service_install_directory_missing".to_string())?;
        let backup = install_directory.join(&journal.backup_name);
        let backup_digest = if path_exists_no_follow(&backup)? {
            trusted_file_digest(&backup, "collector_service_upgrade_backup")?
        } else {
            [0; 32]
        };
        let stable_digest = if path_exists_no_follow(stable)? {
            trusted_file_digest(stable, "collector_service_stable_image")?
        } else {
            [0; 32]
        };
        decide_upgrade_resume(
            journal,
            stable_digest,
            trusted_file_digest(staged, "collector_service_staged_image")?,
            backup_digest,
        )
        .map_err(str::to_string)
    }

    fn resolve_superseded_upgrade_transaction(
        journal: &mut UpgradeJournalV1,
        service: &OwnedScHandle,
        stable: &Path,
        keep_staged_name: Option<&str>,
    ) -> Result<(), String> {
        let install_directory = stable
            .parent()
            .ok_or_else(|| "collector_service_install_directory_missing".to_string())?;
        let prior_staged = install_directory.join(&journal.staged_name);
        let stable_digest = if path_exists_no_follow(stable)? {
            trusted_file_digest(stable, "collector_service_stable_image")?
        } else {
            [0; 32]
        };
        match journal.phase {
            UpgradePhase::Prepared if stable_digest == journal.old_digest => {}
            UpgradePhase::Prepared | UpgradePhase::CandidateInstalled => {
                rollback_upgrade(journal, service, stable)?;
            }
            UpgradePhase::Verified if stable_digest == journal.new_digest => {
                if let Err(error) = ensure_verified_candidate_ready(journal, service, stable) {
                    if journal.phase != UpgradePhase::Prepared {
                        return Err(error);
                    }
                } else {
                    delete_upgrade_journal()?;
                    delete_upgrade_backup(install_directory, journal)?;
                    if keep_staged_name != Some(journal.staged_name.as_str()) {
                        delete_trusted_leaf(
                            &prior_staged,
                            Some(journal.new_digest),
                            "collector_service_superseded_staged_image",
                        )?;
                    }
                    return cleanup_upgrade_install_residue(install_directory, keep_staged_name);
                }
            }
            UpgradePhase::Verified => {
                rollback_upgrade(journal, service, stable)?;
            }
        }
        if journal.phase != UpgradePhase::Prepared
            || !path_exists_no_follow(stable)?
            || trusted_file_digest(stable, "collector_service_stable_image")? != journal.old_digest
        {
            return Err("collector_service_upgrade_supersede_state_invalid".to_string());
        }
        retire_upgrade_artifacts(
            || delete_upgrade_backup(install_directory, journal),
            || {
                if keep_staged_name == Some(journal.staged_name.as_str()) {
                    Ok(())
                } else {
                    delete_trusted_leaf(
                        &prior_staged,
                        Some(journal.new_digest),
                        "collector_service_superseded_staged_image",
                    )
                }
            },
            || cleanup_upgrade_install_residue(install_directory, keep_staged_name),
            delete_upgrade_journal,
        )
    }

    fn commit_upgrade_candidate(
        journal: &mut UpgradeJournalV1,
        service: &OwnedScHandle,
        stable: &Path,
        final_gate: impl FnOnce() -> Result<(), String>,
    ) -> Result<(), String> {
        settle_service_for_replacement(service)?;
        journal.phase = UpgradePhase::CandidateInstalled;
        write_upgrade_journal(journal)?;
        let result = (|| {
            let process_id = start_upgrade_service_generation(service, false)?;
            validate_service_contract(service, stable)?;
            validate_running_service_image(service, stable, journal.new_digest, process_id)?;
            journal.phase = UpgradePhase::Verified;
            write_upgrade_journal(journal)?;
            final_gate()
        })();
        if let Err(error) = result {
            return match rollback_upgrade(journal, service, stable) {
                Ok(()) => Err(error),
                Err(rollback) => Err(format!(
                    "{error};collector_service_upgrade_rollback_failed:{rollback}"
                )),
            };
        }
        Ok(())
    }

    fn rollback_upgrade(
        journal: &mut UpgradeJournalV1,
        service: &OwnedScHandle,
        stable: &Path,
    ) -> Result<(), String> {
        settle_service_for_replacement(service)?;
        let install_directory = stable
            .parent()
            .ok_or_else(|| "collector_service_install_directory_missing".to_string())?;
        restore_upgrade_backup(
            stable,
            &install_directory.join(&journal.backup_name),
            journal.old_digest,
        )?;
        validate_service_contract(service, stable)?;
        let process_id = start_upgrade_service_generation(service, false)?;
        validate_running_service_image(service, stable, journal.old_digest, process_id)?;
        journal.phase = UpgradePhase::Prepared;
        write_upgrade_journal(journal)
    }

    fn ensure_verified_candidate_ready(
        journal: &mut UpgradeJournalV1,
        service: &OwnedScHandle,
        stable: &Path,
    ) -> Result<(), String> {
        let result = ensure_service_generation_ready(service, stable, journal.new_digest);
        if let Err(error) = result {
            return match rollback_upgrade(journal, service, stable) {
                Ok(()) => Err(error),
                Err(rollback) => Err(format!(
                    "{error};collector_service_upgrade_rollback_failed:{rollback}"
                )),
            };
        }
        Ok(())
    }

    fn ensure_service_generation_ready(
        service: &OwnedScHandle,
        stable: &Path,
        expected_digest: [u8; 32],
    ) -> Result<(), String> {
        let status = query_service_status(service)?;
        let process_id = if status.dwCurrentState == SERVICE_RUNNING {
            status.dwProcessId
        } else {
            start_upgrade_service_generation(service, false)?
        };
        validate_service_contract(service, stable)?;
        validate_running_service_image(service, stable, expected_digest, process_id)
    }

    fn finalize_upgrade_transaction(image: &VerifiedServiceImage) -> Result<(), String> {
        let Some(journal) = read_upgrade_journal()? else {
            return Ok(());
        };
        if journal.phase != UpgradePhase::Verified {
            return Err("collector_service_upgrade_journal_not_verified".to_string());
        }
        let install_directory = image
            .path()
            .parent()
            .ok_or_else(|| "collector_service_install_directory_missing".to_string())?;
        let staged = install_directory.join(&journal.staged_name);
        finalize_verified_upgrade(image.path(), &staged, &journal)
    }

    fn finalize_verified_upgrade(
        stable: &Path,
        staged: &Path,
        journal: &UpgradeJournalV1,
    ) -> Result<(), String> {
        if journal.phase != UpgradePhase::Verified
            || trusted_file_digest(stable, "collector_service_stable_image")? != journal.new_digest
            || trusted_file_digest(staged, "collector_service_staged_image")? != journal.new_digest
        {
            return Err("collector_service_upgrade_journal_state_invalid".to_string());
        }
        let install_directory = stable
            .parent()
            .ok_or_else(|| "collector_service_install_directory_missing".to_string())?;
        delete_upgrade_journal()?;
        delete_upgrade_backup(install_directory, journal)
    }

    pub(super) fn uninstall() -> Result<(), String> {
        require_elevated()?;
        let image = verify_current_binary_path()?;
        let stable = image.path().to_path_buf();
        uninstall_with_controller(&image, &stable, None)
    }

    pub(super) fn uninstall_staged() -> Result<(), String> {
        require_elevated()?;
        let image = verify_current_staged_binary_path()?;
        let stable = image
            .path()
            .parent()
            .ok_or_else(|| "collector_service_install_directory_missing".to_string())?
            .join(SERVICE_EXECUTABLE_NAME);
        let current_staged_name = image.path().file_name().and_then(|name| name.to_str());
        uninstall_with_controller(&image, &stable, current_staged_name)
    }

    fn uninstall_with_controller(
        controller: &VerifiedServiceImage,
        stable: &Path,
        keep_staged_name: Option<&str>,
    ) -> Result<(), String> {
        let manager = open_manager(SC_MANAGER_CONNECT)?;
        let service = match open_service(&manager, SERVICE_ALL_ACCESS) {
            Ok(Some(service)) => service,
            Ok(None) => {
                return finish_uninstall_after_service_absent(controller, stable, keep_staged_name)
            }
            Err(error) if service_open_is_delete_pending(&error) => {
                wait_service_deleted(&manager)?;
                return finish_uninstall_after_service_absent(controller, stable, keep_staged_name);
            }
            Err(error) => return Err(error),
        };
        validate_service_contract(&service, stable)?;
        let was_active = query_service_status(&service)?.dwCurrentState != SERVICE_STOPPED;
        let principals = SecurityPrincipals::load_with_service()?;
        let _protected_root = open_protected_etw_lease_root()?;
        if let Err(error) = settle_service_for_replacement(&service) {
            return failed_service_mutation(was_active, error, || start_service_and_wait(&service));
        }
        if unsafe { DeleteService(service.raw()) } == 0 {
            let delete_error = last_error("collector_service_delete_failed");
            return failed_service_mutation(was_active, delete_error, || {
                start_service_and_wait(&service)
            });
        }
        drop(service);
        wait_service_deleted(&manager)?;
        retire_upgrade_transaction_for_uninstall(stable, keep_staged_name)?;
        drop(_protected_root);
        cleanup_roots_if_owned(true, &principals)?;
        retire_legacy_cli(controller)?;
        if keep_staged_name.is_some() {
            Ok(())
        } else {
            retire_staged_upgrade_image(controller)
        }
    }

    fn finish_uninstall_after_service_absent(
        controller: &VerifiedServiceImage,
        stable: &Path,
        keep_staged_name: Option<&str>,
    ) -> Result<(), String> {
        retire_upgrade_transaction_for_uninstall(stable, keep_staged_name)?;
        let roots = fixed_roots()?;
        match missing_service_cleanup(
            path_exists_no_follow(&roots.product)?,
            path_exists_no_follow(&roots.service)?,
        ) {
            MissingServiceCleanup::None => {}
            MissingServiceCleanup::ProductOnly => {
                cleanup_product_root_if_owned(&SecurityPrincipals::load_with_service()?)?;
            }
            MissingServiceCleanup::ServiceTree => {
                cleanup_roots_if_owned(true, &SecurityPrincipals::load_with_service()?)?;
            }
        }
        retire_legacy_cli(controller)?;
        if keep_staged_name.is_some() {
            Ok(())
        } else {
            retire_staged_upgrade_image(controller)
        }
    }

    fn retire_upgrade_transaction_for_uninstall(
        stable: &Path,
        keep_staged_name: Option<&str>,
    ) -> Result<(), String> {
        let install_directory = stable
            .parent()
            .ok_or_else(|| "collector_service_install_directory_missing".to_string())?;
        if let Some(journal) = read_upgrade_journal()? {
            let staged = install_directory.join(&journal.staged_name);
            retire_upgrade_artifacts(
                || delete_upgrade_backup(install_directory, &journal),
                || {
                    if keep_staged_name == Some(journal.staged_name.as_str()) {
                        Ok(())
                    } else {
                        delete_trusted_leaf(
                            &staged,
                            Some(journal.new_digest),
                            "collector_service_upgrade_staged_image",
                        )
                    }
                },
                || cleanup_upgrade_install_residue(install_directory, keep_staged_name),
                delete_upgrade_journal,
            )?;
            return Ok(());
        }
        cleanup_upgrade_install_residue(install_directory, keep_staged_name)
    }

    pub(super) fn failed_service_mutation(
        was_active: bool,
        error: String,
        restart: impl FnOnce() -> Result<(), String>,
    ) -> Result<(), String> {
        if !was_active {
            return Err(error);
        }
        match restart() {
            Ok(()) => Err(error),
            Err(restart) => Err(format!(
                "{error};collector_service_restart_failed:{restart}"
            )),
        }
    }

    fn open_manager(access: u32) -> Result<OwnedScHandle, String> {
        OwnedScHandle::new(
            unsafe { OpenSCManagerW(ptr::null(), ptr::null(), access) },
            "collector_service_scm_open_failed",
        )
    }

    fn open_service(manager: &OwnedScHandle, access: u32) -> Result<Option<OwnedScHandle>, String> {
        let name = wide(COLLECTOR_SERVICE_NAME);
        let handle = unsafe { OpenServiceW(manager.raw(), name.as_ptr(), access) };
        if !handle.is_null() {
            return Ok(Some(OwnedScHandle(handle)));
        }
        let error = unsafe { GetLastError() };
        if error == ERROR_SERVICE_DOES_NOT_EXIST {
            Ok(None)
        } else {
            Err(format!("collector_service_open_failed:{error}"))
        }
    }

    pub(super) fn service_open_is_delete_pending(error: &str) -> bool {
        error == format!("collector_service_open_failed:{ERROR_SERVICE_MARKED_FOR_DELETE}")
    }

    fn create_service(manager: &OwnedScHandle, image: &Path) -> Result<OwnedScHandle, String> {
        let name = wide(COLLECTOR_SERVICE_NAME);
        let display_name = wide(SERVICE_DISPLAY_NAME);
        let account = wide(SERVICE_ACCOUNT);
        let binary_path = quoted_service_path(image);
        OwnedScHandle::new(
            unsafe {
                CreateServiceW(
                    manager.raw(),
                    name.as_ptr(),
                    display_name.as_ptr(),
                    SERVICE_ALL_ACCESS,
                    SERVICE_WIN32_OWN_PROCESS,
                    SERVICE_AUTO_START,
                    SERVICE_ERROR_NORMAL,
                    binary_path.as_ptr(),
                    ptr::null(),
                    ptr::null_mut(),
                    ptr::null(),
                    account.as_ptr(),
                    ptr::null(),
                )
            },
            "collector_service_create_failed",
        )
    }

    fn configure_new_service(service: &OwnedScHandle) -> Result<(), String> {
        let mut delayed = SERVICE_DELAYED_AUTO_START_INFO {
            fDelayedAutostart: 1,
        };
        change_service_config2(
            service,
            SERVICE_CONFIG_DELAYED_AUTO_START_INFO,
            (&mut delayed as *mut SERVICE_DELAYED_AUTO_START_INFO).cast(),
            "collector_service_delayed_start_config_failed",
        )?;

        let mut sid = SERVICE_SID_INFO {
            dwServiceSidType: SERVICE_SID_TYPE_UNRESTRICTED,
        };
        change_service_config2(
            service,
            SERVICE_CONFIG_SERVICE_SID_INFO,
            (&mut sid as *mut SERVICE_SID_INFO).cast(),
            "collector_service_sid_config_failed",
        )?;

        let mut privileges = multi_wide(&SERVICE_REQUIRED_PRIVILEGES);
        let mut required = SERVICE_REQUIRED_PRIVILEGES_INFOW {
            pmszRequiredPrivileges: privileges.as_mut_ptr(),
        };
        change_service_config2(
            service,
            SERVICE_CONFIG_REQUIRED_PRIVILEGES_INFO,
            (&mut required as *mut SERVICE_REQUIRED_PRIVILEGES_INFOW).cast(),
            "collector_service_privileges_config_failed",
        )?;

        let mut description_text = wide(SERVICE_DESCRIPTION);
        let mut description = SERVICE_DESCRIPTIONW {
            lpDescription: description_text.as_mut_ptr(),
        };
        change_service_config2(
            service,
            SERVICE_CONFIG_DESCRIPTION,
            (&mut description as *mut SERVICE_DESCRIPTIONW).cast(),
            "collector_service_description_config_failed",
        )?;

        let descriptor = OwnedSecurityDescriptor::from_sddl(
            "D:P(A;;0x000f01ff;;;SY)(A;;0x000f01ff;;;BA)(A;;0x00000004;;;IU)",
        )?;
        if unsafe {
            SetServiceObjectSecurity(service.raw(), DACL_SECURITY_INFORMATION, descriptor.0)
        } == 0
        {
            return Err(last_error("collector_service_dacl_config_failed"));
        }
        Ok(())
    }

    fn change_service_config2(
        service: &OwnedScHandle,
        level: u32,
        value: *const c_void,
        context: &str,
    ) -> Result<(), String> {
        if unsafe { ChangeServiceConfig2W(service.raw(), level, value) } == 0 {
            Err(last_error(context))
        } else {
            Ok(())
        }
    }

    fn rollback_new_install(
        service: OwnedScHandle,
        manager: &OwnedScHandle,
        product_root_created: bool,
        service_root_created: bool,
    ) -> Result<(), String> {
        stop_service_and_wait(&service, false)?;
        let principals = if product_root_created || service_root_created {
            Some(SecurityPrincipals::load_with_service()?)
        } else {
            None
        };
        if unsafe { DeleteService(service.raw()) } == 0 {
            return Err(last_error("collector_service_rollback_delete_failed"));
        }
        drop(service);
        wait_service_deleted(manager)?;
        if let Some(principals) = principals.as_ref() {
            cleanup_created_roots(product_root_created, service_root_created, principals)?;
        }
        Ok(())
    }

    fn validate_service_contract(
        service: &OwnedScHandle,
        expected_image: &Path,
    ) -> Result<(), String> {
        let config = query_service_config(service)?;
        validate_existing_service_policy(
            &ExistingServicePolicy {
                owner_marker: read_owner_marker()?.as_deref(),
                image_path: &config.image_path,
                account: &config.account,
                service_type: config.service_type,
            },
            expected_image,
        )?;
        if config.start_type != SERVICE_AUTO_START || config.error_control != SERVICE_ERROR_NORMAL {
            return Err("collector_service_start_contract_invalid".to_string());
        }
        let delayed: SERVICE_DELAYED_AUTO_START_INFO = query_config2_fixed(
            service,
            SERVICE_CONFIG_DELAYED_AUTO_START_INFO,
            "collector_service_delayed_start_query_failed",
        )?;
        if delayed.fDelayedAutostart == 0 {
            return Err("collector_service_delayed_start_contract_invalid".to_string());
        }
        let sid: SERVICE_SID_INFO = query_config2_fixed(
            service,
            SERVICE_CONFIG_SERVICE_SID_INFO,
            "collector_service_sid_query_failed",
        )?;
        if sid.dwServiceSidType != SERVICE_SID_TYPE_UNRESTRICTED {
            return Err("collector_service_sid_contract_invalid".to_string());
        }
        let mut privileges = query_required_privileges(service)?;
        privileges.sort();
        let mut expected = SERVICE_REQUIRED_PRIVILEGES
            .iter()
            .map(|value| (*value).to_string())
            .collect::<Vec<_>>();
        expected.sort();
        if privileges != expected {
            return Err("collector_service_privileges_contract_invalid".to_string());
        }
        validate_service_dacl(service)
    }

    #[cfg(feature = "private-windows-lifecycle-proof")]
    pub(super) fn observe_installed_boundaries_for_proof(
        expected_image: &Path,
    ) -> Result<InstalledBoundariesForProof, String> {
        let manager = open_manager(SC_MANAGER_CONNECT)?;
        let service = open_service(
            &manager,
            SERVICE_QUERY_STATUS
                | windows_sys::Win32::System::Services::SERVICE_QUERY_CONFIG
                | READ_CONTROL,
        )?
        .ok_or_else(|| "collector_service_proof_service_missing".to_string())?;
        validate_service_contract(&service, expected_image)?;
        let (service_policy, service_dacl_sha256) = service_dacl_policy(&service)?;
        validate_service_dacl_policy(&service_policy)?;
        let service_aces = service_policy
            .into_iter()
            .map(ace_policy_for_proof)
            .collect();

        let roots = fixed_roots()?;
        let principals = SecurityPrincipals::load_with_service()?;
        let _program_data = open_directory(
            &roots.program_data,
            "collector_service_programdata_open_failed",
        )?;
        let _product = open_and_verify_root(&roots.product, false, &principals)?;
        let service_root = open_and_verify_root(&roots.service, true, &principals)?;
        let (service_data_policy, service_data_root_dacl_sha256) =
            security_policy_with_dacl_sha256(service_root.raw(), &principals)?;
        validate_product_root_policy(&service_data_policy, true)?;
        let service_data_root = security_policy_for_proof(service_data_policy);
        for leaf in [
            ETW_LEASE_FILE_NAME,
            ETW_OWNER_LOCK_FILE_NAME,
            UPGRADE_JOURNAL_FILE_NAME,
        ] {
            let _ = verify_optional_leaf(&roots.service.join(leaf), &principals)?;
        }
        Ok(InstalledBoundariesForProof {
            service_dacl_sha256,
            service_aces,
            service_data_root_dacl_sha256,
            service_data_root,
        })
    }

    #[cfg(feature = "private-windows-lifecycle-proof")]
    pub(super) fn data_roots_for_proof() -> Result<(PathBuf, PathBuf), String> {
        let roots = fixed_roots()?;
        Ok((roots.product, roots.service))
    }

    #[cfg(feature = "private-windows-lifecycle-proof")]
    const PROOF_RESIDUE_MAX_DIRECT_CHILDREN: usize = 256;
    #[cfg(feature = "private-windows-lifecycle-proof")]
    pub(super) const PROOF_RESIDUE_MAX_MATCHED_CHILDREN: usize = 64;
    #[cfg(feature = "private-windows-lifecycle-proof")]
    pub(super) const PROOF_RESIDUE_MAX_TOTAL_BYTES: u64 = 256 * 1024 * 1024;

    #[cfg(feature = "private-windows-lifecycle-proof")]
    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    pub(super) enum ProofResidueKind {
        Journal,
        ServiceDataAtomic,
        Staged,
        Rollback,
        InstallAtomic,
        RollbackExecutionMarker,
    }

    #[cfg(feature = "private-windows-lifecycle-proof")]
    struct PinnedProofResidueFile {
        snapshot: ResidueFileForProof,
        path: PathBuf,
        handle: OwnedHandle,
        allow_service_writer: bool,
        dacl_sha256: String,
    }

    #[cfg(feature = "private-windows-lifecycle-proof")]
    impl PinnedProofResidueFile {
        fn revalidate(&self, principals: &SecurityPrincipals) -> Result<(), String> {
            let info = file_information(
                self.handle.raw(),
                "collector_service_proof_residue_revalidate_info_failed",
            )?;
            if !proof_residue_file_information_valid(&info)
                || info.dwVolumeSerialNumber != self.snapshot.volume_serial
                || ((u64::from(info.nFileIndexHigh) << 32) | u64::from(info.nFileIndexLow))
                    != self.snapshot.file_index
                || file_size(&info) != self.snapshot.size
                || !fixed_path_eq(
                    &final_path(
                        &self.handle,
                        "collector_service_proof_residue_revalidate_path_failed",
                    )?,
                    &self.path,
                )
            {
                return Err("collector_service_proof_residue_identity_changed".to_string());
            }
            validate_no_untrusted_writer(
                self.handle.raw(),
                principals,
                self.allow_service_writer,
                self.allow_service_writer,
            )
            .map_err(|_| "collector_service_proof_residue_acl_changed".to_string())?;
            let (_, dacl_sha256) = security_policy_with_dacl_sha256(self.handle.raw(), principals)
                .map_err(|_| "collector_service_proof_residue_acl_changed".to_string())?;
            if dacl_sha256 != self.dacl_sha256 {
                return Err("collector_service_proof_residue_acl_changed".to_string());
            }
            Ok(())
        }
    }

    #[cfg(feature = "private-windows-lifecycle-proof")]
    pub(super) fn install_atomic_temp_name_for_proof(name: &str) -> bool {
        if name == "batcave-collector-service.rollback.tmp" {
            return true;
        }
        atomic_temp_base(name).is_some_and(|base| {
            is_staged_upgrade_name(base) || rollback_digest_from_name(base).is_some()
        })
    }

    #[cfg(feature = "private-windows-lifecycle-proof")]
    pub(super) fn classify_service_data_residue_name_for_proof(
        name: &str,
    ) -> Result<Option<ProofResidueKind>, String> {
        if name == UPGRADE_JOURNAL_FILE_NAME {
            return Ok(Some(ProofResidueKind::Journal));
        }
        if is_owned_atomic_temp_name(name) {
            return Ok(Some(ProofResidueKind::ServiceDataAtomic));
        }
        let lowercase = name.to_ascii_lowercase();
        if lowercase == UPGRADE_JOURNAL_FILE_NAME || is_owned_atomic_temp_name(&lowercase) {
            return Err("collector_service_proof_residue_name_noncanonical".to_string());
        }
        Ok(None)
    }

    #[cfg(feature = "private-windows-lifecycle-proof")]
    pub(super) fn classify_install_residue_name_for_proof(
        name: &str,
    ) -> Result<Option<ProofResidueKind>, String> {
        let classified = if is_staged_upgrade_name(name) {
            Some(ProofResidueKind::Staged)
        } else if rollback_digest_from_name(name).is_some() {
            Some(ProofResidueKind::Rollback)
        } else if install_atomic_temp_name_for_proof(name) {
            Some(ProofResidueKind::InstallAtomic)
        } else if name == PRIVATE_ROLLBACK_MARKER_NAME {
            Some(ProofResidueKind::RollbackExecutionMarker)
        } else {
            None
        };
        if classified.is_some() {
            return Ok(classified);
        }
        let lowercase = name.to_ascii_lowercase();
        if is_staged_upgrade_name(&lowercase)
            || rollback_digest_from_name(&lowercase).is_some()
            || install_atomic_temp_name_for_proof(&lowercase)
            || lowercase == PRIVATE_ROLLBACK_MARKER_NAME
        {
            return Err("collector_service_proof_residue_name_noncanonical".to_string());
        }
        Ok(None)
    }

    #[cfg(feature = "private-windows-lifecycle-proof")]
    fn proof_residue_file_information_valid(info: &BY_HANDLE_FILE_INFORMATION) -> bool {
        info.dwFileAttributes & (FILE_ATTRIBUTE_DIRECTORY | FILE_ATTRIBUTE_REPARSE_POINT) == 0
            && info.nNumberOfLinks == 1
            && ((u64::from(info.nFileIndexHigh) << 32) | u64::from(info.nFileIndexLow)) != 0
    }

    #[cfg(feature = "private-windows-lifecycle-proof")]
    pub(super) fn proof_residue_size_valid(kind: ProofResidueKind, size: u64) -> bool {
        let maximum = match kind {
            ProofResidueKind::Journal | ProofResidueKind::ServiceDataAtomic => {
                UPGRADE_JOURNAL_MAX_BYTES as u64
            }
            ProofResidueKind::Staged
            | ProofResidueKind::Rollback
            | ProofResidueKind::InstallAtomic => PRIVATE_ROLLBACK_FIXTURE_MAX_BYTES as u64,
            ProofResidueKind::RollbackExecutionMarker => 1024,
        };
        (1..=maximum).contains(&size)
    }

    #[cfg(feature = "private-windows-lifecycle-proof")]
    pub(super) fn reserve_proof_residue_bytes(remaining: &mut u64, size: u64) -> bool {
        let Some(next) = remaining.checked_sub(size) else {
            return false;
        };
        *remaining = next;
        true
    }

    #[cfg(feature = "private-windows-lifecycle-proof")]
    pub(super) fn proof_residue_match_count_valid(current: usize) -> bool {
        current < PROOF_RESIDUE_MAX_MATCHED_CHILDREN
    }

    #[cfg(all(test, feature = "private-windows-lifecycle-proof"))]
    pub(super) fn proof_residue_file_has_single_link_for_test(path: &Path) -> Result<bool, String> {
        let file = open_file(path, "collector_service_proof_residue_test_open_failed")?;
        Ok(proof_residue_file_information_valid(&file_information(
            file.raw(),
            "collector_service_proof_residue_test_info_failed",
        )?))
    }

    #[cfg(feature = "private-windows-lifecycle-proof")]
    pub(super) fn observe_service_install_residue_for_proof() -> ServiceInstallResidueForProof {
        ServiceInstallResidueForProof {
            service_registry_key: observe_service_registry_key_for_proof(),
            service_data: match observe_service_data_residue_for_proof() {
                Ok(value) => value,
                Err(reason) => {
                    crate::windows_lifecycle_proof_contract::Observation::Unknown(reason)
                }
            },
            install: match observe_install_residue_for_proof() {
                Ok(value) => value,
                Err(reason) => {
                    crate::windows_lifecycle_proof_contract::Observation::Unknown(reason)
                }
            },
        }
    }

    #[cfg(feature = "private-windows-lifecycle-proof")]
    #[derive(Clone, Debug, Eq, PartialEq)]
    struct ProductRegistryCapture {
        final_key_path: String,
        install_root: String,
        value_names: Vec<String>,
        subkey_names: Vec<String>,
        default_value_type: u32,
        last_write_time_100ns: u64,
        owner: PrincipalClass,
        dacl_sha256: String,
    }

    #[cfg(feature = "private-windows-lifecycle-proof")]
    impl From<ProductRegistryCapture> for ProductRegistrationKeyForProof {
        fn from(value: ProductRegistryCapture) -> Self {
            Self {
                final_key_path: value.final_key_path,
                install_root: value.install_root,
                value_names: value.value_names,
                subkey_names: value.subkey_names,
                default_value_type: value.default_value_type,
                last_write_time_100ns: value.last_write_time_100ns,
                owner: principal_for_proof(value.owner),
                dacl_sha256: value.dacl_sha256,
            }
        }
    }

    #[cfg(feature = "private-windows-lifecycle-proof")]
    pub(super) fn observe_machine_registration_for_proof() -> MachineRegistrationForProof {
        use crate::windows_lifecycle_proof_contract::Observation;

        let product_key_64 = observe_product_registry_view_for_proof(KEY_WOW64_64KEY);
        let product_key_32 = match observe_product_registry_view_for_proof(KEY_WOW64_32KEY) {
            Observation::Absent => Observation::Absent,
            Observation::Present(_) => Observation::Unknown(
                "collector_service_proof_product_registry_32_view_present".to_string(),
            ),
            Observation::Unknown(reason) => Observation::Unknown(reason),
        };
        MachineRegistrationForProof {
            product_key_64,
            product_key_32,
            public_desktop_shortcut: observe_shortcut_for_proof(
                &FOLDERID_PublicDesktop,
                Path::new(PUBLIC_DESKTOP_PATH),
                false,
            ),
            common_start_menu_shortcut: observe_shortcut_for_proof(
                &FOLDERID_CommonPrograms,
                Path::new(COMMON_PROGRAMS_PATH),
                true,
            ),
        }
    }

    #[cfg(feature = "private-windows-lifecycle-proof")]
    fn observe_product_registry_view_for_proof(
        view: u32,
    ) -> crate::windows_lifecycle_proof_contract::Observation<ProductRegistrationKeyForProof> {
        use crate::windows_lifecycle_proof_contract::Observation;

        let Some(key) = (match open_product_registry_key_for_proof(view) {
            Ok(value) => value,
            Err(reason) => return Observation::Unknown(reason),
        }) else {
            return match open_product_registry_key_for_proof(view) {
                Ok(None) => Observation::Absent,
                Ok(Some(_)) => Observation::Unknown(
                    "collector_service_proof_product_registry_appeared".to_string(),
                ),
                Err(reason) => Observation::Unknown(reason),
            };
        };
        let principals = match SecurityPrincipals::load_base() {
            Ok(value) => value,
            Err(reason) => return Observation::Unknown(reason),
        };
        let first = match capture_product_registry_key_for_proof(&key, &principals) {
            Ok(value) => value,
            Err(reason) => return Observation::Unknown(reason),
        };
        let second = match capture_product_registry_key_for_proof(&key, &principals) {
            Ok(value) => value,
            Err(reason) => return Observation::Unknown(reason),
        };
        let reopened = match open_product_registry_key_for_proof(view) {
            Ok(Some(value)) => value,
            Ok(None) => {
                return Observation::Unknown(
                    "collector_service_proof_product_registry_disappeared".to_string(),
                )
            }
            Err(reason) => return Observation::Unknown(reason),
        };
        let third = match capture_product_registry_key_for_proof(&reopened, &principals) {
            Ok(value) => value,
            Err(reason) => return Observation::Unknown(reason),
        };
        if first != second || second != third {
            return Observation::Unknown(
                "collector_service_proof_product_registry_changed".to_string(),
            );
        }
        Observation::Present(first.into())
    }

    #[cfg(feature = "private-windows-lifecycle-proof")]
    fn open_product_registry_key_for_proof(view: u32) -> Result<Option<OwnedRegistryKey>, String> {
        let path = wide(PRODUCT_REGISTRATION_PATH);
        let mut key = ptr::null_mut();
        let status = unsafe {
            RegOpenKeyExW(
                HKEY_LOCAL_MACHINE,
                path.as_ptr(),
                0,
                KEY_QUERY_VALUE | KEY_ENUMERATE_SUB_KEYS | READ_CONTROL | view,
                &mut key,
            )
        };
        if status == ERROR_SUCCESS {
            Ok(Some(OwnedRegistryKey(key)))
        } else if is_missing_path_error(status) {
            Ok(None)
        } else {
            Err(format!(
                "collector_service_proof_product_registry_open_failed:{status}"
            ))
        }
    }

    #[cfg(feature = "private-windows-lifecycle-proof")]
    fn capture_product_registry_key_for_proof(
        key: &OwnedRegistryKey,
        principals: &SecurityPrincipals,
    ) -> Result<ProductRegistryCapture, String> {
        let final_key_path = query_registry_key_path_for_proof(key.raw())?;
        if !final_key_path.eq_ignore_ascii_case(PRODUCT_REGISTRATION_NT_PATH) {
            return Err("collector_service_proof_product_registry_path_invalid".to_string());
        }
        let mut subkey_count = 0_u32;
        let mut maximum_subkey_name = 0_u32;
        let mut value_count = 0_u32;
        let mut maximum_value_name = 0_u32;
        let mut maximum_value_bytes = 0_u32;
        let mut security_descriptor_bytes = 0_u32;
        let mut last_write = FILETIME::default();
        let status = unsafe {
            RegQueryInfoKeyW(
                key.raw(),
                ptr::null_mut(),
                ptr::null_mut(),
                ptr::null(),
                &mut subkey_count,
                &mut maximum_subkey_name,
                ptr::null_mut(),
                &mut value_count,
                &mut maximum_value_name,
                &mut maximum_value_bytes,
                &mut security_descriptor_bytes,
                &mut last_write,
            )
        };
        if status != ERROR_SUCCESS {
            return Err(format!(
                "collector_service_proof_product_registry_info_failed:{status}"
            ));
        }
        if subkey_count != 0
            || value_count != 1
            || maximum_subkey_name > PROOF_REGISTRY_MAX_NAME_CHARS
            || maximum_value_name > PROOF_REGISTRY_MAX_NAME_CHARS
            || !(2..=PROOF_REGISTRY_MAX_TEXT_BYTES).contains(&maximum_value_bytes)
            || security_descriptor_bytes == 0
            || security_descriptor_bytes > PROOF_REGISTRY_MAX_TEXT_BYTES
        {
            return Err("collector_service_proof_product_registry_shape_invalid".to_string());
        }
        let (value_names, default_value_type, install_root) =
            enumerate_product_registry_values_for_proof(
                key.raw(),
                maximum_value_name,
                maximum_value_bytes,
            )?;
        let subkey_names = enumerate_product_registry_subkeys_for_proof(key.raw())?;
        if value_names != [String::new()]
            || !subkey_names.is_empty()
            || default_value_type != REG_SZ
            || !fixed_path_eq(Path::new(&install_root), Path::new(PROOF_INSTALL_ROOT))
        {
            return Err("collector_service_proof_product_registry_contract_invalid".to_string());
        }
        let (owner, dacl_sha256) = registry_dacl_for_proof(key.raw(), principals)?;
        Ok(ProductRegistryCapture {
            final_key_path,
            install_root,
            value_names,
            subkey_names,
            default_value_type,
            last_write_time_100ns: (u64::from(last_write.dwHighDateTime) << 32)
                | u64::from(last_write.dwLowDateTime),
            owner,
            dacl_sha256,
        })
    }

    #[cfg(feature = "private-windows-lifecycle-proof")]
    fn enumerate_product_registry_values_for_proof(
        key: windows_sys::Win32::System::Registry::HKEY,
        maximum_name: u32,
        maximum_bytes: u32,
    ) -> Result<(Vec<String>, u32, String), String> {
        let mut name = vec![0_u16; maximum_name as usize + 1];
        let mut name_chars = name.len() as u32;
        let mut value_type = 0_u32;
        let mut data = vec![0_u8; maximum_bytes as usize];
        let mut data_bytes = data.len() as u32;
        let status = unsafe {
            RegEnumValueW(
                key,
                0,
                name.as_mut_ptr(),
                &mut name_chars,
                ptr::null(),
                &mut value_type,
                data.as_mut_ptr(),
                &mut data_bytes,
            )
        };
        if status != ERROR_SUCCESS || name_chars as usize > name.len() || data_bytes > maximum_bytes
        {
            return Err(format!(
                "collector_service_proof_product_registry_value_enumeration_failed:{status}"
            ));
        }
        let value_name = String::from_utf16(&name[..name_chars as usize]).map_err(|_| {
            "collector_service_proof_product_registry_value_name_invalid".to_string()
        })?;
        let install_root = parse_registry_sz_for_proof(&data[..data_bytes as usize], value_type)?;
        name_chars = name.len() as u32;
        data_bytes = data.len() as u32;
        let terminal = unsafe {
            RegEnumValueW(
                key,
                1,
                name.as_mut_ptr(),
                &mut name_chars,
                ptr::null(),
                &mut value_type,
                data.as_mut_ptr(),
                &mut data_bytes,
            )
        };
        if terminal != ERROR_NO_MORE_ITEMS {
            return Err("collector_service_proof_product_registry_value_count_changed".to_string());
        }
        Ok((vec![value_name], REG_SZ, install_root))
    }

    #[cfg(feature = "private-windows-lifecycle-proof")]
    fn enumerate_product_registry_subkeys_for_proof(
        key: windows_sys::Win32::System::Registry::HKEY,
    ) -> Result<Vec<String>, String> {
        let mut name = vec![0_u16; PROOF_REGISTRY_MAX_NAME_CHARS as usize + 1];
        let mut name_chars = name.len() as u32;
        let status = unsafe {
            RegEnumKeyExW(
                key,
                0,
                name.as_mut_ptr(),
                &mut name_chars,
                ptr::null(),
                ptr::null_mut(),
                ptr::null_mut(),
                ptr::null_mut(),
            )
        };
        if status == ERROR_NO_MORE_ITEMS {
            Ok(Vec::new())
        } else {
            Err(format!(
                "collector_service_proof_product_registry_subkey_enumeration_failed:{status}"
            ))
        }
    }

    #[cfg(feature = "private-windows-lifecycle-proof")]
    fn parse_registry_sz_for_proof(bytes: &[u8], value_type: u32) -> Result<String, String> {
        if value_type != REG_SZ || bytes.len() < 2 || bytes.len() & 1 != 0 {
            return Err("collector_service_proof_product_registry_default_invalid".to_string());
        }
        let wide = bytes
            .chunks_exact(2)
            .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
            .collect::<Vec<_>>();
        let Some((&0, text)) = wide.split_last() else {
            return Err("collector_service_proof_product_registry_default_invalid".to_string());
        };
        if text.is_empty() || text.contains(&0) {
            return Err("collector_service_proof_product_registry_default_invalid".to_string());
        }
        String::from_utf16(text)
            .map_err(|_| "collector_service_proof_product_registry_default_invalid".to_string())
    }

    #[cfg(feature = "private-windows-lifecycle-proof")]
    fn query_registry_key_path_for_proof(
        key: windows_sys::Win32::System::Registry::HKEY,
    ) -> Result<String, String> {
        let mut required = 0_u32;
        unsafe {
            NtQueryKey(
                key.cast(),
                KeyNameInformation,
                ptr::null_mut(),
                0,
                &mut required,
            );
        }
        if !(6..=8 * 1024).contains(&required) {
            return Err("collector_service_proof_product_registry_path_size_invalid".to_string());
        }
        let mut buffer = vec![0_u32; (required as usize).div_ceil(size_of::<u32>())];
        let status = unsafe {
            NtQueryKey(
                key.cast(),
                KeyNameInformation,
                buffer.as_mut_ptr().cast(),
                required,
                &mut required,
            )
        };
        if status != 0 {
            return Err(format!(
                "collector_service_proof_product_registry_path_query_failed:{status:#010x}"
            ));
        }
        let bytes =
            unsafe { std::slice::from_raw_parts(buffer.as_ptr().cast::<u8>(), buffer.len() * 4) };
        let name_bytes = u32::from_ne_bytes(bytes[..4].try_into().expect("four bytes")) as usize;
        if name_bytes == 0 || name_bytes & 1 != 0 || name_bytes + 4 > required as usize {
            return Err("collector_service_proof_product_registry_path_invalid".to_string());
        }
        let wide = bytes[4..4 + name_bytes]
            .chunks_exact(2)
            .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
            .collect::<Vec<_>>();
        String::from_utf16(&wide)
            .map_err(|_| "collector_service_proof_product_registry_path_invalid".to_string())
    }

    #[cfg(feature = "private-windows-lifecycle-proof")]
    fn registry_dacl_for_proof(
        key: windows_sys::Win32::System::Registry::HKEY,
        principals: &SecurityPrincipals,
    ) -> Result<(PrincipalClass, String), String> {
        let mut required = 0_u32;
        let information = OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION;
        let status = unsafe { RegGetKeySecurity(key, information, ptr::null_mut(), &mut required) };
        if status != ERROR_INSUFFICIENT_BUFFER
            || required == 0
            || required > PROOF_REGISTRY_MAX_TEXT_BYTES
        {
            return Err(format!(
                "collector_service_proof_product_registry_security_size_failed:{status}"
            ));
        }
        let mut buffer = aligned_buffer(required as usize);
        let descriptor = buffer.as_mut_ptr().cast();
        let status = unsafe { RegGetKeySecurity(key, information, descriptor, &mut required) };
        if status != ERROR_SUCCESS {
            return Err(format!(
                "collector_service_proof_product_registry_security_failed:{status}"
            ));
        }
        let mut owner = ptr::null_mut();
        let mut owner_defaulted = 0_i32;
        if unsafe { GetSecurityDescriptorOwner(descriptor, &mut owner, &mut owner_defaulted) } == 0
            || owner.is_null()
        {
            return Err("collector_service_proof_product_registry_owner_invalid".to_string());
        }
        let owner = principals.classify(owner);
        if !matches!(
            owner,
            PrincipalClass::LocalSystem
                | PrincipalClass::Administrators
                | PrincipalClass::TrustedInstaller
        ) {
            return Err("collector_service_proof_product_registry_owner_invalid".to_string());
        }
        let mut present = 0_i32;
        let mut defaulted = 0_i32;
        let mut dacl = ptr::null_mut();
        if unsafe { GetSecurityDescriptorDacl(descriptor, &mut present, &mut dacl, &mut defaulted) }
            == 0
            || present == 0
            || dacl.is_null()
        {
            return Err("collector_service_proof_product_registry_dacl_invalid".to_string());
        }
        for ace in read_aces(dacl, principals)? {
            if ace.allow
                && !ace.inherit_only
                && !matches!(
                    ace.principal,
                    PrincipalClass::LocalSystem
                        | PrincipalClass::Administrators
                        | PrincipalClass::TrustedInstaller
                )
                && ace.mask & PRODUCT_REGISTRY_WRITE_MASK != 0
            {
                return Err(
                    "collector_service_proof_product_registry_unprivileged_writer".to_string(),
                );
            }
        }
        Ok((
            owner,
            dacl_sha256(dacl, "collector_service_proof_product_registry_dacl")?,
        ))
    }

    #[cfg(feature = "private-windows-lifecycle-proof")]
    struct PinnedProofDirectory {
        path: PathBuf,
        handle: OwnedHandle,
        volume_serial: u32,
        file_index: u64,
        file_attributes: u32,
        creation_time_100ns: u64,
        last_write_time_100ns: u64,
        owner: PrincipalClass,
        dacl_sha256: String,
        strict_writer_policy: bool,
    }

    #[cfg(feature = "private-windows-lifecycle-proof")]
    impl PinnedProofDirectory {
        fn open(
            path: &Path,
            principals: &SecurityPrincipals,
            strict_writer_policy: bool,
        ) -> Result<Self, String> {
            let handle = open_directory(
                path,
                "collector_service_proof_shortcut_ancestry_open_failed",
            )?;
            let info = file_information(
                handle.raw(),
                "collector_service_proof_shortcut_ancestry_info_failed",
            )?;
            if !fixed_path_eq(
                &final_path(
                    &handle,
                    "collector_service_proof_shortcut_ancestry_path_failed",
                )?,
                path,
            ) {
                return Err(
                    "collector_service_proof_shortcut_ancestry_identity_invalid".to_string()
                );
            }
            if strict_writer_policy {
                validate_no_untrusted_writer(handle.raw(), principals, false, false).map_err(
                    |_| "collector_service_proof_shortcut_common_acl_invalid".to_string(),
                )?;
            }
            let (policy, dacl_sha256) = security_policy_with_dacl_sha256(handle.raw(), principals)?;
            Ok(Self {
                path: path.to_path_buf(),
                handle,
                volume_serial: info.dwVolumeSerialNumber,
                file_index: (u64::from(info.nFileIndexHigh) << 32) | u64::from(info.nFileIndexLow),
                file_attributes: info.dwFileAttributes,
                creation_time_100ns: proof_filetime(info.ftCreationTime),
                last_write_time_100ns: proof_filetime(info.ftLastWriteTime),
                owner: policy.owner,
                dacl_sha256,
                strict_writer_policy,
            })
        }

        fn revalidate(&self, principals: &SecurityPrincipals) -> Result<(), String> {
            let info = file_information(
                self.handle.raw(),
                "collector_service_proof_shortcut_ancestry_revalidate_info_failed",
            )?;
            if info.dwFileAttributes & FILE_ATTRIBUTE_DIRECTORY == 0
                || info.dwFileAttributes & FILE_ATTRIBUTE_REPARSE_POINT != 0
                || info.dwVolumeSerialNumber != self.volume_serial
                || ((u64::from(info.nFileIndexHigh) << 32) | u64::from(info.nFileIndexLow))
                    != self.file_index
                || info.dwFileAttributes != self.file_attributes
                || proof_filetime(info.ftCreationTime) != self.creation_time_100ns
                || proof_filetime(info.ftLastWriteTime) != self.last_write_time_100ns
                || !fixed_path_eq(
                    &final_path(
                        &self.handle,
                        "collector_service_proof_shortcut_ancestry_revalidate_path_failed",
                    )?,
                    &self.path,
                )
            {
                return Err("collector_service_proof_shortcut_ancestry_changed".to_string());
            }
            if self.strict_writer_policy {
                validate_no_untrusted_writer(self.handle.raw(), principals, false, false).map_err(
                    |_| "collector_service_proof_shortcut_common_acl_changed".to_string(),
                )?;
            }
            let (policy, dacl_sha256) =
                security_policy_with_dacl_sha256(self.handle.raw(), principals)?;
            if policy.owner != self.owner || dacl_sha256 != self.dacl_sha256 {
                return Err("collector_service_proof_shortcut_ancestry_acl_changed".to_string());
            }
            Ok(())
        }
    }

    #[cfg(feature = "private-windows-lifecycle-proof")]
    struct PinnedProofShortcut {
        snapshot: ShortcutForProof,
        path: PathBuf,
        handle: OwnedHandle,
        file_attributes: u32,
        creation_time_100ns: u64,
        last_write_time_100ns: u64,
        owner: PrincipalClass,
        dacl_sha256: String,
        strict_writer_policy: bool,
    }

    #[cfg(feature = "private-windows-lifecycle-proof")]
    impl PinnedProofShortcut {
        fn open(
            path: &Path,
            principals: &SecurityPrincipals,
            strict_writer_policy: bool,
        ) -> Result<Option<Self>, String> {
            let path_wide = wide_path(path);
            let raw = unsafe {
                CreateFileW(
                    path_wide.as_ptr(),
                    GENERIC_READ | READ_CONTROL | FILE_READ_ATTRIBUTES,
                    FILE_SHARE_READ,
                    ptr::null(),
                    OPEN_EXISTING,
                    FILE_FLAG_OPEN_REPARSE_POINT,
                    ptr::null_mut(),
                )
            };
            if raw.is_null() || raw == (-1_isize as HANDLE) {
                let error = unsafe { GetLastError() };
                if is_missing_path_error(error) {
                    return Ok(None);
                }
                return Err(format!(
                    "collector_service_proof_shortcut_open_failed:{error}"
                ));
            }
            let handle = OwnedHandle(raw);
            let info =
                file_information(handle.raw(), "collector_service_proof_shortcut_info_failed")?;
            let size = file_size(&info);
            if info.dwFileAttributes & (FILE_ATTRIBUTE_DIRECTORY | FILE_ATTRIBUTE_REPARSE_POINT)
                != 0
                || info.nNumberOfLinks != 1
                || ((u64::from(info.nFileIndexHigh) << 32) | u64::from(info.nFileIndexLow)) == 0
                || !(1..=PROOF_SHORTCUT_MAX_BYTES).contains(&size)
                || !fixed_path_eq(
                    &final_path(&handle, "collector_service_proof_shortcut_path_failed")?,
                    path,
                )
            {
                return Err("collector_service_proof_shortcut_identity_invalid".to_string());
            }
            if strict_writer_policy {
                validate_no_untrusted_writer(handle.raw(), principals, false, false).map_err(
                    |_| "collector_service_proof_shortcut_common_acl_invalid".to_string(),
                )?;
            }
            let (policy, dacl_sha256) = security_policy_with_dacl_sha256(handle.raw(), principals)?;
            let sha256 = digest_hex(&hash_open_file_from_start_for_proof(handle.raw(), size)?);
            Ok(Some(Self {
                snapshot: ShortcutForProof {
                    path: path
                        .to_str()
                        .ok_or_else(|| {
                            "collector_service_proof_shortcut_path_utf16_invalid".to_string()
                        })?
                        .to_string(),
                    target: String::new(),
                    arguments: String::new(),
                    icon_path: String::new(),
                    icon_index: 0,
                    working_directory: String::new(),
                    show_command: 0,
                    hotkey: 0,
                    description: String::new(),
                    app_user_model_id: String::new(),
                    owner: principal_for_proof(policy.owner),
                    dacl_sha256: dacl_sha256.clone(),
                    size,
                    sha256,
                    volume_serial: info.dwVolumeSerialNumber,
                    file_index: (u64::from(info.nFileIndexHigh) << 32)
                        | u64::from(info.nFileIndexLow),
                },
                path: path.to_path_buf(),
                handle,
                file_attributes: info.dwFileAttributes,
                creation_time_100ns: proof_filetime(info.ftCreationTime),
                last_write_time_100ns: proof_filetime(info.ftLastWriteTime),
                owner: policy.owner,
                dacl_sha256,
                strict_writer_policy,
            }))
        }

        fn read_contract(&mut self) -> Result<(), String> {
            let contract = read_shortcut_contract_for_proof(&self.path)?;
            validate_shortcut_contract_for_proof(&contract)?;
            self.snapshot.target = contract.target;
            self.snapshot.arguments = contract.arguments;
            self.snapshot.icon_path = contract.icon_path;
            self.snapshot.icon_index = contract.icon_index;
            self.snapshot.working_directory = contract.working_directory;
            self.snapshot.show_command = contract.show_command;
            self.snapshot.hotkey = contract.hotkey;
            self.snapshot.description = contract.description;
            self.snapshot.app_user_model_id = contract.app_user_model_id;
            Ok(())
        }

        fn revalidate(&self, principals: &SecurityPrincipals) -> Result<(), String> {
            let info = file_information(
                self.handle.raw(),
                "collector_service_proof_shortcut_revalidate_info_failed",
            )?;
            if info.dwFileAttributes & (FILE_ATTRIBUTE_DIRECTORY | FILE_ATTRIBUTE_REPARSE_POINT)
                != 0
                || info.nNumberOfLinks != 1
                || info.dwVolumeSerialNumber != self.snapshot.volume_serial
                || ((u64::from(info.nFileIndexHigh) << 32) | u64::from(info.nFileIndexLow))
                    != self.snapshot.file_index
                || file_size(&info) != self.snapshot.size
                || info.dwFileAttributes != self.file_attributes
                || proof_filetime(info.ftCreationTime) != self.creation_time_100ns
                || proof_filetime(info.ftLastWriteTime) != self.last_write_time_100ns
                || !fixed_path_eq(
                    &final_path(
                        &self.handle,
                        "collector_service_proof_shortcut_revalidate_path_failed",
                    )?,
                    &self.path,
                )
                || digest_hex(&hash_open_file_from_start_for_proof(
                    self.handle.raw(),
                    self.snapshot.size,
                )?) != self.snapshot.sha256
            {
                return Err("collector_service_proof_shortcut_changed".to_string());
            }
            if self.strict_writer_policy {
                validate_no_untrusted_writer(self.handle.raw(), principals, false, false).map_err(
                    |_| "collector_service_proof_shortcut_common_acl_changed".to_string(),
                )?;
            }
            let (policy, dacl_sha256) =
                security_policy_with_dacl_sha256(self.handle.raw(), principals)?;
            if policy.owner != self.owner || dacl_sha256 != self.dacl_sha256 {
                return Err("collector_service_proof_shortcut_acl_changed".to_string());
            }
            Ok(())
        }
    }

    #[cfg(feature = "private-windows-lifecycle-proof")]
    #[derive(Debug, Eq, PartialEq)]
    struct ShortcutContract {
        target: String,
        arguments: String,
        icon_path: String,
        icon_index: i32,
        working_directory: String,
        show_command: i32,
        hotkey: u16,
        description: String,
        app_user_model_id: String,
    }

    #[cfg(feature = "private-windows-lifecycle-proof")]
    fn observe_shortcut_for_proof(
        folder_id: &GUID,
        expected_root: &Path,
        strict_writer_policy: bool,
    ) -> crate::windows_lifecycle_proof_contract::Observation<ShortcutForProof> {
        use crate::windows_lifecycle_proof_contract::Observation;

        let root = match known_folder_path_for_proof(folder_id) {
            Ok(value) if fixed_path_eq(&value, expected_root) => value,
            Ok(_) => {
                return Observation::Unknown(
                    "collector_service_proof_shortcut_known_folder_invalid".to_string(),
                )
            }
            Err(reason) => return Observation::Unknown(reason),
        };
        let principals = match SecurityPrincipals::load_base() {
            Ok(value) => value,
            Err(reason) => return Observation::Unknown(reason),
        };
        let ancestry =
            match pin_known_folder_ancestry_for_proof(&root, &principals, strict_writer_policy) {
                Ok(value) => value,
                Err(reason) => return Observation::Unknown(reason),
            };
        let path = root.join(PRODUCT_SHORTCUT_NAME);
        let mut shortcut = match PinnedProofShortcut::open(&path, &principals, strict_writer_policy)
        {
            Ok(Some(value)) => value,
            Ok(None) => {
                return match PinnedProofShortcut::open(&path, &principals, strict_writer_policy) {
                    Ok(None) => {
                        for directory in &ancestry {
                            if let Err(reason) = directory.revalidate(&principals) {
                                return Observation::Unknown(reason);
                            }
                        }
                        match known_folder_path_for_proof(folder_id) {
                            Ok(after) if fixed_path_eq(&after, &root) => Observation::Absent,
                            Ok(_) => Observation::Unknown(
                                "collector_service_proof_shortcut_known_folder_changed".to_string(),
                            ),
                            Err(reason) => Observation::Unknown(reason),
                        }
                    }
                    Ok(Some(_)) => Observation::Unknown(
                        "collector_service_proof_shortcut_appeared".to_string(),
                    ),
                    Err(reason) => Observation::Unknown(reason),
                }
            }
            Err(reason) => return Observation::Unknown(reason),
        };
        if let Err(reason) = shortcut.read_contract() {
            return Observation::Unknown(reason);
        }
        if let Err(reason) = shortcut.revalidate(&principals) {
            return Observation::Unknown(reason);
        }
        for directory in &ancestry {
            if let Err(reason) = directory.revalidate(&principals) {
                return Observation::Unknown(reason);
            }
        }
        match known_folder_path_for_proof(folder_id) {
            Ok(after) if fixed_path_eq(&after, &root) => Observation::Present(shortcut.snapshot),
            Ok(_) => Observation::Unknown(
                "collector_service_proof_shortcut_known_folder_changed".to_string(),
            ),
            Err(reason) => Observation::Unknown(reason),
        }
    }

    #[cfg(feature = "private-windows-lifecycle-proof")]
    fn pin_known_folder_ancestry_for_proof(
        root: &Path,
        principals: &SecurityPrincipals,
        strict_root_writer_policy: bool,
    ) -> Result<Vec<PinnedProofDirectory>, String> {
        let mut paths = root
            .ancestors()
            .filter(|path| !path.as_os_str().is_empty())
            .collect::<Vec<_>>();
        paths.reverse();
        if paths.len() > 16 || paths.last().copied() != Some(root) {
            return Err("collector_service_proof_shortcut_ancestry_invalid".to_string());
        }
        paths
            .into_iter()
            .map(|path| {
                PinnedProofDirectory::open(
                    path,
                    principals,
                    strict_root_writer_policy && path == root,
                )
            })
            .collect()
    }

    #[cfg(feature = "private-windows-lifecycle-proof")]
    fn known_folder_path_for_proof(folder_id: &GUID) -> Result<PathBuf, String> {
        let mut raw = ptr::null_mut();
        let result = unsafe { SHGetKnownFolderPath(folder_id, 0, ptr::null_mut(), &mut raw) };
        if result < 0 || raw.is_null() {
            return Err(format!(
                "collector_service_proof_shortcut_known_folder_failed:{result:#010x}"
            ));
        }
        let value =
            read_nul_terminated_wide_for_proof(raw, PROOF_COM_TEXT_CAPACITY).map(PathBuf::from);
        unsafe { CoTaskMemFree(raw.cast()) };
        value
    }

    #[cfg(feature = "private-windows-lifecycle-proof")]
    fn read_shortcut_contract_for_proof(path: &Path) -> Result<ShortcutContract, String> {
        let _apartment = ComApartment::initialize()?;
        let mut raw = ptr::null_mut();
        let result = unsafe {
            CoCreateInstance(
                &CLSID_SHELL_LINK,
                ptr::null_mut(),
                CLSCTX_INPROC_SERVER,
                &IID_SHELL_LINK_W,
                &mut raw,
            )
        };
        if result != 0 || raw.is_null() {
            return Err(format!(
                "collector_service_proof_shortcut_create_failed:{result:#010x}"
            ));
        }
        let shell = ComPtr(raw);
        let persist = shell.query(
            &IID_PERSIST_FILE,
            "collector_service_proof_shortcut_persist_query_failed",
        )?;
        let path_wide = wide_path(path);
        let result = unsafe { (persist.persist_file().load)(persist.0, path_wide.as_ptr(), 0) };
        if result != 0 {
            return Err(format!(
                "collector_service_proof_shortcut_load_failed:{result:#010x}"
            ));
        }
        let target = shell_link_text_for_proof(|buffer, capacity| unsafe {
            (shell.shell_link().get_path)(shell.0, buffer, capacity, ptr::null_mut(), 4)
        })?;
        let description = shell_link_text_for_proof(|buffer, capacity| unsafe {
            (shell.shell_link().get_description)(shell.0, buffer, capacity)
        })?;
        let working_directory = shell_link_text_for_proof(|buffer, capacity| unsafe {
            (shell.shell_link().get_working_directory)(shell.0, buffer, capacity)
        })?;
        let arguments = shell_link_text_for_proof(|buffer, capacity| unsafe {
            (shell.shell_link().get_arguments)(shell.0, buffer, capacity)
        })?;
        let mut hotkey = 0_u16;
        let result = unsafe { (shell.shell_link().get_hotkey)(shell.0, &mut hotkey) };
        if result != 0 {
            return Err(format!(
                "collector_service_proof_shortcut_hotkey_failed:{result:#010x}"
            ));
        }
        let mut show_command = 0_i32;
        let result = unsafe { (shell.shell_link().get_show_command)(shell.0, &mut show_command) };
        if result != 0 {
            return Err(format!(
                "collector_service_proof_shortcut_show_command_failed:{result:#010x}"
            ));
        }
        let mut icon_index = i32::MIN;
        let icon_path = shell_link_text_for_proof(|buffer, capacity| unsafe {
            (shell.shell_link().get_icon_location)(shell.0, buffer, capacity, &mut icon_index)
        })?;
        let property_store = shell.query(
            &IID_PROPERTY_STORE,
            "collector_service_proof_shortcut_property_store_query_failed",
        )?;
        let app_user_model_id = property_string_for_proof(
            &property_store,
            &PKEY_APP_USER_MODEL_ID,
            "collector_service_proof_shortcut_app_id",
        )?;
        Ok(ShortcutContract {
            target,
            arguments,
            icon_path,
            icon_index,
            working_directory,
            show_command,
            hotkey,
            description,
            app_user_model_id,
        })
    }

    #[cfg(feature = "private-windows-lifecycle-proof")]
    fn shell_link_text_for_proof(
        read: impl FnOnce(*mut u16, i32) -> i32,
    ) -> Result<String, String> {
        let mut buffer = vec![u16::MAX; PROOF_COM_TEXT_CAPACITY];
        let result = read(buffer.as_mut_ptr(), buffer.len() as i32);
        if result != 0 {
            return Err(format!(
                "collector_service_proof_shortcut_property_failed:{result:#010x}"
            ));
        }
        let Some(end) = buffer.iter().position(|value| *value == 0) else {
            return Err("collector_service_proof_shortcut_property_unbounded".to_string());
        };
        String::from_utf16(&buffer[..end])
            .map_err(|_| "collector_service_proof_shortcut_property_utf16_invalid".to_string())
    }

    #[cfg(feature = "private-windows-lifecycle-proof")]
    fn property_string_for_proof(
        store: &ComPtr,
        key: &PROPERTYKEY,
        context: &str,
    ) -> Result<String, String> {
        let mut value = PROPVARIANT::default();
        let result = unsafe { (store.property_store().get_value)(store.0, key, &mut value) };
        if result != 0 {
            unsafe { PropVariantClear(&mut value) };
            return Err(format!("{context}_read_failed:{result:#010x}"));
        }
        let value_type = unsafe { value.Anonymous.Anonymous.vt };
        if value_type != VT_LPWSTR {
            unsafe { PropVariantClear(&mut value) };
            return Err(format!("{context}_type_invalid"));
        }
        let mut raw = ptr::null_mut();
        let result = unsafe { PropVariantToStringAlloc(&value, &mut raw) };
        let text = if result == 0 && !raw.is_null() {
            read_nul_terminated_wide_for_proof(raw, PROOF_COM_TEXT_CAPACITY)
        } else {
            Err(format!("{context}_conversion_failed:{result:#010x}"))
        };
        if !raw.is_null() {
            unsafe { CoTaskMemFree(raw.cast()) };
        }
        unsafe { PropVariantClear(&mut value) };
        text
    }

    #[cfg(feature = "private-windows-lifecycle-proof")]
    fn read_nul_terminated_wide_for_proof(
        value: *const u16,
        capacity: usize,
    ) -> Result<String, String> {
        let mut length = 0_usize;
        while length < capacity && unsafe { *value.add(length) } != 0 {
            length += 1;
        }
        if length == capacity {
            return Err("collector_service_proof_wide_text_unbounded".to_string());
        }
        String::from_utf16(unsafe { std::slice::from_raw_parts(value, length) })
            .map_err(|_| "collector_service_proof_wide_text_utf16_invalid".to_string())
    }

    #[cfg(feature = "private-windows-lifecycle-proof")]
    fn validate_shortcut_contract_for_proof(contract: &ShortcutContract) -> Result<(), String> {
        if !fixed_path_eq(Path::new(&contract.target), Path::new(PROOF_MONITOR_PATH))
            || !contract.arguments.is_empty()
            || !contract.icon_path.is_empty()
            || contract.icon_index != 0
            || !fixed_path_eq(
                Path::new(&contract.working_directory),
                Path::new(PROOF_INSTALL_ROOT),
            )
            || contract.show_command != 1
            || contract.hotkey != 0
            || !contract.description.is_empty()
            || contract.app_user_model_id != PRODUCT_APP_USER_MODEL_ID
        {
            return Err("collector_service_proof_shortcut_contract_invalid".to_string());
        }
        Ok(())
    }

    #[cfg(feature = "private-windows-lifecycle-proof")]
    fn hash_open_file_from_start_for_proof(handle: HANDLE, size: u64) -> Result<[u8; 32], String> {
        if unsafe { SetFilePointerEx(handle, 0, ptr::null_mut(), FILE_BEGIN) } == 0 {
            return Err(last_error("collector_service_proof_shortcut_seek_failed"));
        }
        hash_open_file(handle, size)
    }

    #[cfg(feature = "private-windows-lifecycle-proof")]
    fn proof_filetime(value: FILETIME) -> u64 {
        (u64::from(value.dwHighDateTime) << 32) | u64::from(value.dwLowDateTime)
    }

    #[cfg(feature = "private-windows-lifecycle-proof")]
    fn observe_service_registry_key_for_proof(
    ) -> crate::windows_lifecycle_proof_contract::Observation<ServiceRegistryKeyForProof> {
        use crate::windows_lifecycle_proof_contract::Observation;

        let open = || -> Result<Option<OwnedRegistryKey>, String> {
            let path = wide(SERVICE_REGISTRY_PATH);
            let mut key = ptr::null_mut();
            let status = unsafe {
                RegOpenKeyExW(
                    HKEY_LOCAL_MACHINE,
                    path.as_ptr(),
                    0,
                    KEY_QUERY_VALUE | windows_sys::Win32::System::Registry::KEY_WOW64_64KEY,
                    &mut key,
                )
            };
            if status == 0 {
                Ok(Some(OwnedRegistryKey(key)))
            } else if is_missing_path_error(status) {
                Ok(None)
            } else {
                Err(format!(
                    "collector_service_proof_service_registry_open_failed:{status}"
                ))
            }
        };

        let Some(key) = (match open() {
            Ok(key) => key,
            Err(reason) => return Observation::Unknown(reason),
        }) else {
            return match open() {
                Ok(None) => Observation::Absent,
                Ok(Some(_)) => Observation::Unknown(
                    "collector_service_proof_service_registry_appeared".to_string(),
                ),
                Err(reason) => Observation::Unknown(reason),
            };
        };
        let first = match read_optional_registry_string_for_proof(key.0, SERVICE_FAILURE_VALUE) {
            Ok(value) => value,
            Err(reason) => return Observation::Unknown(reason),
        };
        let second = match read_optional_registry_string_for_proof(key.0, SERVICE_FAILURE_VALUE) {
            Ok(value) => value,
            Err(reason) => return Observation::Unknown(reason),
        };
        let reopened = match open() {
            Ok(Some(key)) => key,
            Ok(None) => {
                return Observation::Unknown(
                    "collector_service_proof_service_registry_disappeared".to_string(),
                )
            }
            Err(reason) => return Observation::Unknown(reason),
        };
        let third = match read_optional_registry_string_for_proof(reopened.0, SERVICE_FAILURE_VALUE)
        {
            Ok(value) => value,
            Err(reason) => return Observation::Unknown(reason),
        };
        if first != second || second != third {
            return Observation::Unknown(
                "collector_service_proof_service_registry_changed".to_string(),
            );
        }
        Observation::Present(ServiceRegistryKeyForProof {
            last_failure: first.map_or(Observation::Absent, Observation::Present),
        })
    }

    #[cfg(feature = "private-windows-lifecycle-proof")]
    fn read_optional_registry_string_for_proof(
        key: windows_sys::Win32::System::Registry::HKEY,
        value_name: &str,
    ) -> Result<Option<String>, String> {
        let name = wide(value_name);
        let mut value_type = 0_u32;
        let mut bytes = 0_u32;
        let status = unsafe {
            RegQueryValueExW(
                key,
                name.as_ptr(),
                ptr::null_mut(),
                &mut value_type,
                ptr::null_mut(),
                &mut bytes,
            )
        };
        if status == ERROR_FILE_NOT_FOUND {
            return Ok(None);
        }
        if status != 0
            || value_type != REG_SZ
            || !(2..=64 * 1024).contains(&bytes)
            || !bytes.is_multiple_of(2)
        {
            return Err(format!(
                "collector_service_proof_service_registry_value_query_failed:{status}"
            ));
        }
        let mut value = vec![0_u16; bytes as usize / size_of::<u16>()];
        let status = unsafe {
            RegQueryValueExW(
                key,
                name.as_ptr(),
                ptr::null_mut(),
                &mut value_type,
                value.as_mut_ptr().cast(),
                &mut bytes,
            )
        };
        if status != 0 || value_type != REG_SZ || !bytes.is_multiple_of(2) {
            return Err(format!(
                "collector_service_proof_service_registry_value_read_failed:{status}"
            ));
        }
        value.truncate(bytes as usize / size_of::<u16>());
        while value.last() == Some(&0) {
            value.pop();
        }
        let value = String::from_utf16(&value).map_err(|_| {
            "collector_service_proof_service_registry_value_utf16_invalid".to_string()
        })?;
        if value.is_empty() {
            return Err("collector_service_proof_service_registry_value_empty".to_string());
        }
        Ok(Some(value))
    }

    #[cfg(feature = "private-windows-lifecycle-proof")]
    fn observe_service_data_residue_for_proof() -> Result<
        crate::windows_lifecycle_proof_contract::Observation<ServiceDataResidueForProof>,
        String,
    > {
        use crate::windows_lifecycle_proof_contract::Observation;

        let roots = fixed_roots()?;
        let principals = SecurityPrincipals::load_with_service()?;
        let _program_data = open_directory(
            &roots.program_data,
            "collector_service_proof_programdata_open_failed",
        )?;
        let Some(_product) =
            open_optional_product_root_for_proof(&roots.product, false, &principals)?
        else {
            return if open_optional_product_root_for_proof(&roots.product, false, &principals)?
                .is_none()
            {
                Ok(Observation::Absent)
            } else {
                Err("collector_service_proof_product_root_appeared".to_string())
            };
        };
        let Some(service) =
            open_optional_product_root_for_proof(&roots.service, true, &principals)?
        else {
            return if open_optional_product_root_for_proof(&roots.service, true, &principals)?
                .is_none()
            {
                Ok(Observation::Absent)
            } else {
                Err("collector_service_proof_service_root_appeared".to_string())
            };
        };
        let info = file_information(
            service.raw(),
            "collector_service_proof_service_residue_root_info_failed",
        )?;
        let files = capture_residue_files_for_proof(
            &roots.service,
            &service,
            &principals,
            true,
            classify_service_data_residue_name_for_proof,
        )?;
        let mut journal = Observation::Absent;
        let mut atomic_temporary_files = Vec::new();
        for (kind, file) in files {
            match kind {
                ProofResidueKind::Journal => journal = Observation::Present(file),
                ProofResidueKind::ServiceDataAtomic => atomic_temporary_files.push(file),
                _ => unreachable!("service-data classifier returned install residue"),
            }
        }
        Ok(Observation::Present(ServiceDataResidueForProof {
            volume_serial: info.dwVolumeSerialNumber,
            file_index: (u64::from(info.nFileIndexHigh) << 32) | u64::from(info.nFileIndexLow),
            upgrade_transaction_journal: journal,
            atomic_temporary_files,
        }))
    }

    #[cfg(feature = "private-windows-lifecycle-proof")]
    fn observe_install_residue_for_proof(
    ) -> Result<crate::windows_lifecycle_proof_contract::Observation<InstallResidueForProof>, String>
    {
        use crate::windows_lifecycle_proof_contract::Observation;

        let program_files = known_folder(CSIDL_PROGRAM_FILES)?;
        let _program_files = open_directory(
            &program_files,
            "collector_service_proof_program_files_open_failed",
        )?;
        let install_path = program_files.join(PRODUCT_DIRECTORY_NAME);
        let principals = SecurityPrincipals::load_base()?;
        let Some(install) = open_optional_install_root_for_proof(&install_path, &principals)?
        else {
            return if open_optional_install_root_for_proof(&install_path, &principals)?.is_none() {
                Ok(Observation::Absent)
            } else {
                Err("collector_service_proof_install_root_appeared".to_string())
            };
        };
        let info = file_information(
            install.raw(),
            "collector_service_proof_install_residue_root_info_failed",
        )?;
        let files = capture_residue_files_for_proof(
            &install_path,
            &install,
            &principals,
            false,
            classify_install_residue_name_for_proof,
        )?;
        let mut staged_service_images = Vec::new();
        let mut rollback_service_images = Vec::new();
        let mut atomic_temporary_files = Vec::new();
        let mut rollback_execution_marker = Observation::Absent;
        for (kind, file) in files {
            match kind {
                ProofResidueKind::Staged => staged_service_images.push(file),
                ProofResidueKind::Rollback => rollback_service_images.push(file),
                ProofResidueKind::InstallAtomic => atomic_temporary_files.push(file),
                ProofResidueKind::RollbackExecutionMarker => {
                    rollback_execution_marker = Observation::Present(file)
                }
                _ => unreachable!("install classifier returned service-data residue"),
            }
        }
        Ok(Observation::Present(InstallResidueForProof {
            volume_serial: info.dwVolumeSerialNumber,
            file_index: (u64::from(info.nFileIndexHigh) << 32) | u64::from(info.nFileIndexLow),
            staged_service_images,
            rollback_service_images,
            atomic_temporary_files,
            rollback_execution_marker,
        }))
    }

    #[cfg(feature = "private-windows-lifecycle-proof")]
    fn open_optional_product_root_for_proof(
        path: &Path,
        service_leaf: bool,
        principals: &SecurityPrincipals,
    ) -> Result<Option<OwnedHandle>, String> {
        let Some(handle) = open_optional_directory_for_proof(path)? else {
            return Ok(None);
        };
        let policy = security_policy(handle.raw(), principals)?;
        validate_product_root_policy(&policy, service_leaf)?;
        Ok(Some(handle))
    }

    #[cfg(feature = "private-windows-lifecycle-proof")]
    fn open_optional_install_root_for_proof(
        path: &Path,
        principals: &SecurityPrincipals,
    ) -> Result<Option<OwnedHandle>, String> {
        let Some(handle) = open_optional_directory_for_proof(path)? else {
            return Ok(None);
        };
        validate_no_untrusted_writer(handle.raw(), principals, false, false)?;
        Ok(Some(handle))
    }

    #[cfg(feature = "private-windows-lifecycle-proof")]
    fn open_optional_directory_for_proof(path: &Path) -> Result<Option<OwnedHandle>, String> {
        let path_wide = wide_path(path);
        let raw = unsafe {
            CreateFileW(
                path_wide.as_ptr(),
                FILE_READ_ATTRIBUTES | READ_CONTROL,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                ptr::null(),
                OPEN_EXISTING,
                FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT,
                ptr::null_mut(),
            )
        };
        if raw.is_null() || raw == (-1_isize as HANDLE) {
            let error = unsafe { GetLastError() };
            return if is_missing_path_error(error) {
                Ok(None)
            } else {
                Err(format!(
                    "collector_service_proof_residue_root_open_failed:{error}"
                ))
            };
        }
        let handle = OwnedHandle(raw);
        let info = file_information(
            handle.raw(),
            "collector_service_proof_residue_root_info_failed",
        )?;
        if info.dwFileAttributes & FILE_ATTRIBUTE_DIRECTORY == 0
            || info.dwFileAttributes & FILE_ATTRIBUTE_REPARSE_POINT != 0
            || !fixed_path_eq(
                &final_path(&handle, "collector_service_proof_residue_root_path_failed")?,
                path,
            )
        {
            return Err("collector_service_proof_residue_root_identity_invalid".to_string());
        }
        Ok(Some(handle))
    }

    #[cfg(feature = "private-windows-lifecycle-proof")]
    fn capture_residue_files_for_proof(
        root_path: &Path,
        root: &OwnedHandle,
        principals: &SecurityPrincipals,
        allow_service_writer: bool,
        classify: impl Fn(&str) -> Result<Option<ProofResidueKind>, String>,
    ) -> Result<Vec<(ProofResidueKind, ResidueFileForProof)>, String> {
        let root_info = file_information(
            root.raw(),
            "collector_service_proof_residue_root_capture_info_failed",
        )?;
        let (_, root_dacl_sha256) = security_policy_with_dacl_sha256(root.raw(), principals)?;
        revalidate_residue_root_path_for_proof(
            root_path,
            &root_info,
            &root_dacl_sha256,
            principals,
            allow_service_writer,
        )?;
        let before = enumerate_direct_children_for_proof(root_path)?;
        let mut pinned = Vec::new();
        let mut remaining_bytes = PROOF_RESIDUE_MAX_TOTAL_BYTES;
        for name in &before {
            let Some(name) = name.to_str() else {
                continue;
            };
            let Some(kind) = classify(name)? else {
                continue;
            };
            if !proof_residue_match_count_valid(pinned.len()) {
                return Err("collector_service_proof_residue_match_set_too_large".to_string());
            }
            pinned.push((
                kind,
                open_residue_file_for_proof(
                    &root_path.join(name),
                    principals,
                    allow_service_writer,
                    kind,
                    &mut remaining_bytes,
                )?,
            ));
        }
        revalidate_residue_root_path_for_proof(
            root_path,
            &root_info,
            &root_dacl_sha256,
            principals,
            allow_service_writer,
        )?;
        if enumerate_direct_children_for_proof(root_path)? != before {
            return Err("collector_service_proof_residue_enumeration_changed".to_string());
        }
        revalidate_residue_root_path_for_proof(
            root_path,
            &root_info,
            &root_dacl_sha256,
            principals,
            allow_service_writer,
        )?;
        validate_no_untrusted_writer(
            root.raw(),
            principals,
            allow_service_writer,
            allow_service_writer,
        )
        .map_err(|_| "collector_service_proof_residue_root_acl_changed".to_string())?;
        let after_root_info = file_information(
            root.raw(),
            "collector_service_proof_residue_root_revalidate_info_failed",
        )?;
        let (_, after_root_dacl_sha256) = security_policy_with_dacl_sha256(root.raw(), principals)
            .map_err(|_| "collector_service_proof_residue_root_acl_changed".to_string())?;
        if after_root_info.dwFileAttributes
            & (FILE_ATTRIBUTE_DIRECTORY | FILE_ATTRIBUTE_REPARSE_POINT)
            != FILE_ATTRIBUTE_DIRECTORY
            || after_root_info.dwVolumeSerialNumber != root_info.dwVolumeSerialNumber
            || after_root_info.nFileIndexHigh != root_info.nFileIndexHigh
            || after_root_info.nFileIndexLow != root_info.nFileIndexLow
            || after_root_dacl_sha256 != root_dacl_sha256
            || !fixed_path_eq(
                &final_path(
                    root,
                    "collector_service_proof_residue_root_revalidate_path_failed",
                )?,
                root_path,
            )
        {
            return Err("collector_service_proof_residue_root_identity_changed".to_string());
        }
        for (_, file) in &pinned {
            file.revalidate(principals)?;
        }
        Ok(pinned
            .into_iter()
            .map(|(kind, file)| (kind, file.snapshot))
            .collect())
    }

    #[cfg(feature = "private-windows-lifecycle-proof")]
    fn revalidate_residue_root_path_for_proof(
        root_path: &Path,
        expected_info: &BY_HANDLE_FILE_INFORMATION,
        expected_dacl_sha256: &str,
        principals: &SecurityPrincipals,
        allow_service_writer: bool,
    ) -> Result<(), String> {
        let root = open_optional_directory_for_proof(root_path)?
            .ok_or_else(|| "collector_service_proof_residue_root_disappeared".to_string())?;
        validate_no_untrusted_writer(
            root.raw(),
            principals,
            allow_service_writer,
            allow_service_writer,
        )?;
        let info = file_information(
            root.raw(),
            "collector_service_proof_residue_root_reopen_info_failed",
        )?;
        let (_, dacl_sha256) = security_policy_with_dacl_sha256(root.raw(), principals)?;
        if info.dwFileAttributes & (FILE_ATTRIBUTE_DIRECTORY | FILE_ATTRIBUTE_REPARSE_POINT)
            != FILE_ATTRIBUTE_DIRECTORY
            || info.dwVolumeSerialNumber != expected_info.dwVolumeSerialNumber
            || info.nFileIndexHigh != expected_info.nFileIndexHigh
            || info.nFileIndexLow != expected_info.nFileIndexLow
            || dacl_sha256 != expected_dacl_sha256
            || !fixed_path_eq(
                &final_path(
                    &root,
                    "collector_service_proof_residue_root_reopen_path_failed",
                )?,
                root_path,
            )
        {
            return Err("collector_service_proof_residue_root_path_changed".to_string());
        }
        Ok(())
    }

    #[cfg(feature = "private-windows-lifecycle-proof")]
    fn enumerate_direct_children_for_proof(root: &Path) -> Result<Vec<OsString>, String> {
        let mut names = Vec::new();
        for entry in fs::read_dir(root)
            .map_err(|error| format!("collector_service_proof_residue_enumerate_failed:{error}"))?
        {
            let entry = entry
                .map_err(|error| format!("collector_service_proof_residue_entry_failed:{error}"))?;
            if names.len() == PROOF_RESIDUE_MAX_DIRECT_CHILDREN {
                return Err("collector_service_proof_residue_enumeration_too_large".to_string());
            }
            names.push(entry.file_name());
        }
        names.sort();
        Ok(names)
    }

    #[cfg(feature = "private-windows-lifecycle-proof")]
    fn open_residue_file_for_proof(
        path: &Path,
        principals: &SecurityPrincipals,
        allow_service_writer: bool,
        kind: ProofResidueKind,
        remaining_bytes: &mut u64,
    ) -> Result<PinnedProofResidueFile, String> {
        let path_wide = wide_path(path);
        let handle = OwnedHandle::new(
            unsafe {
                CreateFileW(
                    path_wide.as_ptr(),
                    GENERIC_READ | READ_CONTROL | FILE_READ_ATTRIBUTES,
                    FILE_SHARE_READ,
                    ptr::null(),
                    OPEN_EXISTING,
                    FILE_FLAG_OPEN_REPARSE_POINT,
                    ptr::null_mut(),
                )
            },
            "collector_service_proof_residue_file_open_failed",
        )?;
        let info = file_information(
            handle.raw(),
            "collector_service_proof_residue_file_info_failed",
        )?;
        if !proof_residue_file_information_valid(&info)
            || !fixed_path_eq(
                &final_path(&handle, "collector_service_proof_residue_file_path_failed")?,
                path,
            )
        {
            return Err("collector_service_proof_residue_file_identity_invalid".to_string());
        }
        validate_no_untrusted_writer(
            handle.raw(),
            principals,
            allow_service_writer,
            allow_service_writer,
        )?;
        let (_, dacl_sha256) = security_policy_with_dacl_sha256(handle.raw(), principals)?;
        let size = file_size(&info);
        if !proof_residue_size_valid(kind, size) {
            return Err("collector_service_proof_residue_file_size_invalid".to_string());
        }
        if !reserve_proof_residue_bytes(remaining_bytes, size) {
            return Err("collector_service_proof_residue_total_size_invalid".to_string());
        }
        let sha256 = digest_hex(&hash_open_file(handle.raw(), size)?);
        let path_string = path
            .to_str()
            .ok_or_else(|| "collector_service_proof_residue_file_path_utf16_invalid".to_string())?
            .to_string();
        Ok(PinnedProofResidueFile {
            snapshot: ResidueFileForProof {
                path: path_string,
                size,
                sha256,
                volume_serial: info.dwVolumeSerialNumber,
                file_index: (u64::from(info.nFileIndexHigh) << 32) | u64::from(info.nFileIndexLow),
            },
            path: path.to_path_buf(),
            handle,
            allow_service_writer,
            dacl_sha256,
        })
    }

    #[cfg(feature = "private-windows-lifecycle-proof")]
    pub(super) fn exercise_failed_upgrade_rollback_for_proof(
        candidate_bytes: &[u8],
        expected_candidate_sha256: [u8; 32],
        expected_original_sha256: [u8; 32],
    ) -> Result<FailedUpgradeRollbackForProof, Box<FailedUpgradeRollbackFailure>> {
        let before_mutation = |reason| {
            Box::new(FailedUpgradeRollbackFailure {
                reason,
                service_settled: true,
            })
        };
        require_elevated().map_err(before_mutation)?;
        validate_failed_upgrade_fixture_inputs(
            candidate_bytes,
            expected_candidate_sha256,
            expected_original_sha256,
        )
        .map_err(before_mutation)?;

        let program_files = known_folder(CSIDL_PROGRAM_FILES).map_err(before_mutation)?;
        let stable = expected_service_path(&program_files);
        let staged = expected_staged_service_path(&program_files);
        let install_directory = stable
            .parent()
            .ok_or_else(|| {
                before_mutation("collector_service_install_directory_missing".to_string())
            })?
            .to_path_buf();
        let manager = open_manager(SC_MANAGER_CONNECT).map_err(before_mutation)?;
        let service = open_service(&manager, SERVICE_ALL_ACCESS)
            .map_err(before_mutation)?
            .ok_or_else(|| {
                before_mutation("collector_service_proof_service_missing".to_string())
            })?;
        validate_service_contract(&service, &stable).map_err(before_mutation)?;
        let _protected_root = open_protected_etw_lease_root().map_err(before_mutation)?;
        if trusted_file_digest(&stable, "collector_service_stable_image")
            .map_err(before_mutation)?
            != expected_original_sha256
        {
            return Err(before_mutation(
                "collector_service_proof_original_digest_invalid".to_string(),
            ));
        }
        let original_status = query_service_status(&service).map_err(before_mutation)?;
        validate_running_service_image(
            &service,
            &stable,
            expected_original_sha256,
            original_status.dwProcessId,
        )
        .map_err(before_mutation)?;
        ensure_upgrade_proof_residue_absent(&install_directory).map_err(before_mutation)?;

        let operation = (|| -> Result<FailedUpgradeRollbackForProof, String> {
            crate::atomic_json::write_bytes_atomic(&staged, candidate_bytes).map_err(|error| {
                format!(
                    "collector_service_proof_staged_write_failed:{:?}:{}",
                    error.operation, error.error
                )
            })?;
            let staged_image = verify_service_image_at(&staged, &program_files)?;
            if trusted_file_digest(staged_image.path(), "collector_service_staged_image")?
                != expected_candidate_sha256
            {
                return Err("collector_service_proof_staged_digest_invalid".to_string());
            }

            prepare_upgrade_staged_image(&staged_image, &stable, &service)?;
            let prepared = read_upgrade_journal()?
                .ok_or_else(|| "collector_service_proof_upgrade_journal_missing".to_string())?;
            if prepared.phase != UpgradePhase::Prepared
                || prepared.old_digest != expected_original_sha256
                || prepared.new_digest != expected_candidate_sha256
            {
                return Err("collector_service_proof_upgrade_journal_invalid".to_string());
            }

            restore_upgrade_backup(&stable, staged_image.path(), expected_candidate_sha256)?;
            let mut journal = read_upgrade_journal()?
                .ok_or_else(|| "collector_service_proof_upgrade_journal_missing".to_string())?;
            if upgrade_resume_action(&journal, &stable, staged_image.path())?
                != UpgradeResumeAction::CommitCandidate
            {
                return Err("collector_service_proof_candidate_action_invalid".to_string());
            }
            let candidate_failure =
                match commit_upgrade_candidate(&mut journal, &service, &stable, || Ok(())) {
                    Ok(()) => {
                        rollback_upgrade(&mut journal, &service, &stable)?;
                        return Err(
                            "collector_service_proof_candidate_unexpectedly_started".to_string()
                        );
                    }
                    Err(failure) => failure,
                };
            let execution_marker_sha256 =
                verify_and_retire_rollback_execution_marker(&install_directory, true)?;

            let restored_journal = read_upgrade_journal()?
                .ok_or_else(|| "collector_service_proof_upgrade_journal_missing".to_string())?;
            if restored_journal.phase != UpgradePhase::Prepared
                || restored_journal.old_digest != expected_original_sha256
                || restored_journal.new_digest != expected_candidate_sha256
            {
                return Err("collector_service_proof_rollback_journal_invalid".to_string());
            }
            let restored_process_id =
                validate_proof_running_generation(&service, &stable, expected_original_sha256)?;
            drop(staged_image);
            let mut restored_journal = restored_journal;
            resolve_superseded_upgrade_transaction(&mut restored_journal, &service, &stable, None)?;
            ensure_upgrade_proof_residue_absent(&install_directory)?;
            let final_process_id =
                validate_proof_running_generation(&service, &stable, expected_original_sha256)?;
            if final_process_id != restored_process_id {
                return Err("collector_service_proof_restored_generation_changed".to_string());
            }

            Ok(FailedUpgradeRollbackForProof {
                candidate_sha256: digest_hex(&expected_candidate_sha256),
                candidate_failure_code: "collector_service_proof_candidate_start_failed"
                    .to_string(),
                candidate_failure_detail: candidate_failure,
                execution_marker_sha256,
                restored_sha256: digest_hex(&expected_original_sha256),
                restored_process_id,
            })
        })();

        match operation {
            Ok(proof) => Ok(proof),
            Err(primary) => {
                let recovery = recover_failed_upgrade_proof(
                    &service,
                    &stable,
                    &staged,
                    &install_directory,
                    expected_candidate_sha256,
                    expected_original_sha256,
                );
                Err(Box::new(FailedUpgradeRollbackFailure {
                    reason: match &recovery {
                        Ok(()) => primary,
                        Err(recovery) => {
                            format!("{primary};collector_service_proof_recovery_failed:{recovery}")
                        }
                    },
                    service_settled: recovery.is_ok(),
                }))
            }
        }
    }

    #[cfg(feature = "private-windows-lifecycle-proof")]
    pub(super) fn remove_service_boundary_for_proof(
        final_service_bytes: &[u8],
        expected_final_sha256: [u8; 32],
    ) -> Result<(), Box<ServiceStateTransitionFailure>> {
        let before_mutation = |reason| service_state_failure(reason, true);
        require_elevated().map_err(before_mutation)?;
        validate_service_image_bytes(final_service_bytes, expected_final_sha256)
            .map_err(before_mutation)?;
        let program_files = known_folder(CSIDL_PROGRAM_FILES).map_err(before_mutation)?;
        let stable = expected_service_path(&program_files);
        if trusted_file_digest(&stable, "collector_service_stable_image")
            .map_err(before_mutation)?
            != expected_final_sha256
        {
            return Err(before_mutation(
                "collector_service_proof_original_digest_invalid".to_string(),
            ));
        }
        let manager = open_manager(SC_MANAGER_CONNECT).map_err(before_mutation)?;
        let service = open_service(&manager, SERVICE_ALL_ACCESS)
            .map_err(before_mutation)?
            .ok_or_else(|| {
                before_mutation("collector_service_proof_service_missing".to_string())
            })?;
        validate_service_contract(&service, &stable).map_err(before_mutation)?;
        let status = query_service_status(&service).map_err(before_mutation)?;
        validate_running_service_image(
            &service,
            &stable,
            expected_final_sha256,
            status.dwProcessId,
        )
        .map_err(before_mutation)?;
        let principals = SecurityPrincipals::load_with_service().map_err(before_mutation)?;
        let protected_root = open_protected_etw_lease_root().map_err(before_mutation)?;

        let operation = (|| -> Result<(), String> {
            settle_service_for_replacement(&service)?;
            if unsafe { DeleteService(service.raw()) } == 0 {
                return Err(last_error("collector_service_delete_failed"));
            }
            drop(service);
            wait_service_deleted(&manager)?;
            drop(protected_root);
            cleanup_roots_if_owned(false, &principals)?;
            delete_trusted_leaf(
                &stable,
                Some(expected_final_sha256),
                "collector_service_proof_missing_service_image",
            )?;
            if open_service(&manager, SERVICE_QUERY_STATUS)?.is_some()
                || path_exists_no_follow(&stable)?
                || path_exists_no_follow(&fixed_roots()?.service)?
            {
                return Err("collector_service_proof_missing_state_invalid".to_string());
            }
            Ok(())
        })();
        match operation {
            Ok(()) => Ok(()),
            Err(primary) => {
                let recovery =
                    restore_service_boundary_inner(final_service_bytes, expected_final_sha256);
                Err(service_state_failure(
                    match &recovery {
                        Ok(()) => primary,
                        Err(recovery) => {
                            format!("{primary};collector_service_proof_recovery_failed:{recovery}")
                        }
                    },
                    recovery.is_ok(),
                ))
            }
        }
    }

    #[cfg(feature = "private-windows-lifecycle-proof")]
    pub(super) fn restore_service_boundary_for_proof(
        final_service_bytes: &[u8],
        expected_final_sha256: [u8; 32],
    ) -> Result<(), Box<ServiceStateTransitionFailure>> {
        restore_service_boundary_inner(final_service_bytes, expected_final_sha256)
            .map_err(|reason| service_state_failure(reason, false))
    }

    #[cfg(feature = "private-windows-lifecycle-proof")]
    fn restore_service_boundary_inner(
        final_service_bytes: &[u8],
        expected_final_sha256: [u8; 32],
    ) -> Result<(), String> {
        require_elevated()?;
        validate_service_image_bytes(final_service_bytes, expected_final_sha256)?;
        let program_files = known_folder(CSIDL_PROGRAM_FILES)?;
        let stable = expected_service_path(&program_files);
        if path_exists_no_follow(&stable)? {
            if trusted_file_digest(&stable, "collector_service_stable_image")?
                != expected_final_sha256
            {
                return Err("collector_service_proof_restore_image_untrusted".to_string());
            }
        } else {
            crate::atomic_json::write_bytes_atomic(&stable, final_service_bytes).map_err(
                |error| {
                    format!(
                        "collector_service_proof_restore_image_write_failed:{:?}:{}",
                        error.operation, error.error
                    )
                },
            )?;
        }
        let image = verify_service_image_at(&stable, &program_files)?;
        if trusted_file_digest(image.path(), "collector_service_stable_image")?
            != expected_final_sha256
        {
            return Err("collector_service_proof_restore_digest_invalid".to_string());
        }
        let manager = open_manager(SC_MANAGER_CONNECT | SC_MANAGER_CREATE_SERVICE)?;
        if let Some(service) = open_service(&manager, SERVICE_ALL_ACCESS)? {
            validate_service_contract(&service, &stable)?;
            let mut roots_created = RootCreationJournal::default();
            provision_roots(&mut roots_created)?;
            start_service_and_wait(&service)?;
            validate_proof_running_generation(&service, &stable, expected_final_sha256)?;
        } else {
            let service = create_service(&manager, &stable)?;
            let mut roots_created = RootCreationJournal::default();
            let install = (|| {
                configure_new_service(&service)?;
                set_owner_marker()?;
                provision_roots(&mut roots_created)?;
                validate_service_contract(&service, &stable)?;
                start_service_and_wait(&service)?;
                validate_proof_running_generation(&service, &stable, expected_final_sha256)?;
                Ok(())
            })();
            if let Err(primary) = install {
                return match rollback_new_install(
                    service,
                    &manager,
                    roots_created.product,
                    roots_created.service,
                ) {
                    Ok(()) => Err(primary),
                    Err(rollback) => Err(format!(
                        "{primary};collector_service_proof_restore_rollback_failed:{rollback}"
                    )),
                };
            }
        }
        retire_shortcuts_with_controller(&image)?;
        retire_legacy_cli(&image)?;
        retire_staged_upgrade_image(&image)
    }

    #[cfg(feature = "private-windows-lifecycle-proof")]
    pub(super) fn stop_running_service_for_proof(
        expected_sha256: [u8; 32],
    ) -> Result<(), Box<ServiceStateTransitionFailure>> {
        let before_mutation = |reason| service_state_failure(reason, true);
        require_elevated().map_err(before_mutation)?;
        let stable =
            expected_service_path(&known_folder(CSIDL_PROGRAM_FILES).map_err(before_mutation)?);
        if trusted_file_digest(&stable, "collector_service_stable_image")
            .map_err(before_mutation)?
            != expected_sha256
        {
            return Err(before_mutation(
                "collector_service_proof_original_digest_invalid".to_string(),
            ));
        }
        let manager = open_manager(SC_MANAGER_CONNECT).map_err(before_mutation)?;
        let service = open_service(&manager, SERVICE_ALL_ACCESS)
            .map_err(before_mutation)?
            .ok_or_else(|| {
                before_mutation("collector_service_proof_service_missing".to_string())
            })?;
        validate_service_contract(&service, &stable).map_err(before_mutation)?;
        let status = query_service_status(&service).map_err(before_mutation)?;
        validate_running_service_image(&service, &stable, expected_sha256, status.dwProcessId)
            .map_err(before_mutation)?;
        let _protected_root = open_protected_etw_lease_root().map_err(before_mutation)?;
        match stop_service_and_wait(&service, true)
            .and_then(|_| validate_clean_stopped_status(&query_service_status(&service)?))
        {
            Ok(()) => Ok(()),
            Err(primary) => {
                let recovery = start_service_and_wait(&service).and_then(|_| {
                    let status = query_service_status(&service)?;
                    validate_running_service_image(
                        &service,
                        &stable,
                        expected_sha256,
                        status.dwProcessId,
                    )
                });
                Err(service_state_failure(
                    match &recovery {
                        Ok(()) => primary,
                        Err(recovery) => {
                            format!("{primary};collector_service_proof_recovery_failed:{recovery}")
                        }
                    },
                    recovery.is_ok(),
                ))
            }
        }
    }

    #[cfg(feature = "private-windows-lifecycle-proof")]
    pub(super) fn start_stopped_service_for_proof(
        expected_sha256: [u8; 32],
    ) -> Result<(), Box<ServiceStateTransitionFailure>> {
        let before_mutation = |reason| service_state_failure(reason, true);
        require_elevated().map_err(before_mutation)?;
        let stable =
            expected_service_path(&known_folder(CSIDL_PROGRAM_FILES).map_err(before_mutation)?);
        if trusted_file_digest(&stable, "collector_service_stable_image")
            .map_err(before_mutation)?
            != expected_sha256
        {
            return Err(before_mutation(
                "collector_service_proof_original_digest_invalid".to_string(),
            ));
        }
        let manager = open_manager(SC_MANAGER_CONNECT).map_err(before_mutation)?;
        let service = open_service(&manager, SERVICE_ALL_ACCESS)
            .map_err(before_mutation)?
            .ok_or_else(|| {
                before_mutation("collector_service_proof_service_missing".to_string())
            })?;
        validate_service_contract(&service, &stable).map_err(before_mutation)?;
        start_service_and_wait(&service)
            .and_then(|_| {
                let status = query_service_status(&service)?;
                validate_running_service_image(
                    &service,
                    &stable,
                    expected_sha256,
                    status.dwProcessId,
                )
            })
            .map_err(|reason| service_state_failure(reason, false))
    }

    #[cfg(feature = "private-windows-lifecycle-proof")]
    pub(super) fn replace_running_service_for_proof(
        candidate_bytes: &[u8],
        expected_candidate_sha256: [u8; 32],
        expected_original_sha256: [u8; 32],
    ) -> Result<(), Box<ServiceStateTransitionFailure>> {
        let before_mutation = |reason| service_state_failure(reason, true);
        require_elevated().map_err(before_mutation)?;
        validate_failed_upgrade_fixture_inputs(
            candidate_bytes,
            expected_candidate_sha256,
            expected_original_sha256,
        )
        .map_err(before_mutation)?;
        let program_files = known_folder(CSIDL_PROGRAM_FILES).map_err(before_mutation)?;
        let stable = expected_service_path(&program_files);
        let staged = expected_staged_service_path(&program_files);
        let install_directory = stable
            .parent()
            .ok_or_else(|| {
                before_mutation("collector_service_install_directory_missing".to_string())
            })?
            .to_path_buf();
        let manager = open_manager(SC_MANAGER_CONNECT).map_err(before_mutation)?;
        let service = open_service(&manager, SERVICE_ALL_ACCESS)
            .map_err(before_mutation)?
            .ok_or_else(|| {
                before_mutation("collector_service_proof_service_missing".to_string())
            })?;
        validate_service_contract(&service, &stable).map_err(before_mutation)?;
        let protected_root = open_protected_etw_lease_root().map_err(before_mutation)?;
        if trusted_file_digest(&stable, "collector_service_stable_image")
            .map_err(before_mutation)?
            != expected_original_sha256
        {
            return Err(before_mutation(
                "collector_service_proof_original_digest_invalid".to_string(),
            ));
        }
        let status = query_service_status(&service).map_err(before_mutation)?;
        validate_running_service_image(
            &service,
            &stable,
            expected_original_sha256,
            status.dwProcessId,
        )
        .map_err(before_mutation)?;
        ensure_upgrade_proof_residue_absent(&install_directory).map_err(before_mutation)?;

        let operation = (|| -> Result<(), String> {
            crate::atomic_json::write_bytes_atomic(&staged, candidate_bytes).map_err(|error| {
                format!(
                    "collector_service_proof_staged_write_failed:{:?}:{}",
                    error.operation, error.error
                )
            })?;
            let staged_image = verify_service_image_at(&staged, &program_files)?;
            if trusted_file_digest(staged_image.path(), "collector_service_staged_image")?
                != expected_candidate_sha256
            {
                return Err("collector_service_proof_staged_digest_invalid".to_string());
            }
            prepare_upgrade_staged_image(&staged_image, &stable, &service)?;
            let mut journal = read_upgrade_journal()?
                .ok_or_else(|| "collector_service_proof_upgrade_journal_missing".to_string())?;
            if journal.phase != UpgradePhase::Prepared
                || journal.old_digest != expected_original_sha256
                || journal.new_digest != expected_candidate_sha256
            {
                return Err("collector_service_proof_upgrade_journal_invalid".to_string());
            }
            commit_upgrade_candidate(&mut journal, &service, &stable, || Ok(()))?;
            if journal.phase != UpgradePhase::Verified {
                return Err("collector_service_proof_candidate_not_verified".to_string());
            }
            validate_proof_running_generation(&service, &stable, expected_candidate_sha256)?;
            drop(staged_image);
            finalize_verified_upgrade(&stable, &staged, &journal)?;
            delete_trusted_leaf(
                &staged,
                Some(expected_candidate_sha256),
                "collector_service_proof_staged_image",
            )?;
            cleanup_upgrade_install_residue(&install_directory, None)?;
            validate_proof_running_generation(&service, &stable, expected_candidate_sha256)?;
            Ok(())
        })();
        drop(protected_root);
        match operation {
            Ok(()) => Ok(()),
            Err(primary) => {
                let recovery = recover_failed_upgrade_proof(
                    &service,
                    &stable,
                    &staged,
                    &install_directory,
                    expected_candidate_sha256,
                    expected_original_sha256,
                );
                Err(service_state_failure(
                    match &recovery {
                        Ok(()) => primary,
                        Err(recovery) => {
                            format!("{primary};collector_service_proof_recovery_failed:{recovery}")
                        }
                    },
                    recovery.is_ok(),
                ))
            }
        }
    }

    #[cfg(feature = "private-windows-lifecycle-proof")]
    fn validate_service_image_bytes(bytes: &[u8], expected_sha256: [u8; 32]) -> Result<(), String> {
        if bytes.is_empty()
            || bytes.len() > PRIVATE_ROLLBACK_FIXTURE_MAX_BYTES
            || expected_sha256 == [0; 32]
            || <[u8; 32]>::from(Sha256::digest(bytes)) != expected_sha256
        {
            Err("collector_service_proof_service_bytes_invalid".to_string())
        } else {
            Ok(())
        }
    }

    #[cfg(feature = "private-windows-lifecycle-proof")]
    fn service_state_failure(
        reason: String,
        service_settled: bool,
    ) -> Box<ServiceStateTransitionFailure> {
        Box::new(ServiceStateTransitionFailure {
            reason,
            service_settled,
        })
    }

    #[cfg(feature = "private-windows-lifecycle-proof")]
    fn validate_proof_running_generation(
        service: &OwnedScHandle,
        stable: &Path,
        expected_digest: [u8; 32],
    ) -> Result<u32, String> {
        validate_service_contract(service, stable)?;
        let status = query_service_status(service)?;
        validate_running_service_image(service, stable, expected_digest, status.dwProcessId)?;
        Ok(status.dwProcessId)
    }

    #[cfg(feature = "private-windows-lifecycle-proof")]
    fn recover_failed_upgrade_proof(
        service: &OwnedScHandle,
        stable: &Path,
        staged: &Path,
        install_directory: &Path,
        expected_candidate_sha256: [u8; 32],
        expected_original_sha256: [u8; 32],
    ) -> Result<(), String> {
        if let Some(mut journal) = read_upgrade_journal()? {
            if journal.old_digest != expected_original_sha256
                || journal.new_digest != expected_candidate_sha256
            {
                return Err("collector_service_proof_recovery_journal_invalid".to_string());
            }
            resolve_superseded_upgrade_transaction(&mut journal, service, stable, None)?;
        } else {
            if trusted_file_digest(stable, "collector_service_stable_image")?
                != expected_original_sha256
            {
                return Err("collector_service_proof_recovery_original_missing".to_string());
            }
            let status = query_service_status(service)?;
            if status.dwCurrentState == SERVICE_STOPPED && status.dwProcessId == 0 {
                start_upgrade_service_generation(service, false)?;
            }
            validate_proof_running_generation(service, stable, expected_original_sha256)?;
            delete_trusted_leaf(
                staged,
                Some(expected_candidate_sha256),
                "collector_service_proof_staged_cleanup",
            )?;
            let backup = install_directory.join(upgrade_backup_name(&expected_original_sha256));
            delete_trusted_leaf(
                &backup,
                Some(expected_original_sha256),
                "collector_service_proof_backup_cleanup",
            )?;
            cleanup_upgrade_install_residue(install_directory, None)?;
        }
        verify_and_retire_rollback_execution_marker(install_directory, false)?;
        ensure_upgrade_proof_residue_absent(install_directory)?;
        validate_proof_running_generation(service, stable, expected_original_sha256).map(|_| ())
    }

    #[cfg(feature = "private-windows-lifecycle-proof")]
    fn ensure_upgrade_proof_residue_absent(install_directory: &Path) -> Result<(), String> {
        if read_upgrade_journal()?.is_some() {
            return Err("collector_service_proof_upgrade_residue_present".to_string());
        }
        let service_root = fixed_roots()?.service;
        for entry in fs::read_dir(&service_root).map_err(|error| {
            format!("collector_service_upgrade_service_root_read_failed:{error}")
        })? {
            let entry = entry.map_err(|error| {
                format!("collector_service_upgrade_service_root_entry_failed:{error}")
            })?;
            let name = entry.file_name();
            let Some(name) = name.to_str() else {
                continue;
            };
            if name == UPGRADE_JOURNAL_FILE_NAME
                || atomic_temp_base(name) == Some(UPGRADE_JOURNAL_FILE_NAME)
            {
                return Err("collector_service_proof_upgrade_residue_present".to_string());
            }
        }
        for entry in fs::read_dir(install_directory).map_err(|error| {
            format!("collector_service_upgrade_install_directory_read_failed:{error}")
        })? {
            let entry = entry.map_err(|error| {
                format!("collector_service_upgrade_install_directory_entry_failed:{error}")
            })?;
            let name = entry.file_name();
            let Some(name) = name.to_str() else {
                continue;
            };
            let atomic_base = atomic_temp_base(name);
            if name == PRIVATE_ROLLBACK_MARKER_NAME
                || name == "batcave-collector-service.rollback.tmp"
                || is_staged_upgrade_name(name)
                || rollback_digest_from_name(name).is_some()
                || atomic_base.and_then(rollback_digest_from_name).is_some()
            {
                return Err("collector_service_proof_upgrade_residue_present".to_string());
            }
        }
        Ok(())
    }

    #[cfg(feature = "private-windows-lifecycle-proof")]
    fn verify_and_retire_rollback_execution_marker(
        install_directory: &Path,
        required: bool,
    ) -> Result<String, String> {
        let marker = install_directory.join(PRIVATE_ROLLBACK_MARKER_NAME);
        if !path_exists_no_follow(&marker)? {
            return if required {
                Err("collector_service_proof_execution_marker_missing".to_string())
            } else {
                Ok(String::new())
            };
        }
        let expected_digest: [u8; 32] = Sha256::digest(PRIVATE_ROLLBACK_MARKER_BYTES).into();
        if trusted_file_digest(&marker, "collector_service_proof_execution_marker")?
            != expected_digest
        {
            return Err("collector_service_proof_execution_marker_invalid".to_string());
        }
        delete_trusted_leaf(
            &marker,
            Some(expected_digest),
            "collector_service_proof_execution_marker",
        )?;
        Ok(digest_hex(&expected_digest))
    }

    #[cfg(feature = "private-windows-lifecycle-proof")]
    fn digest_hex(digest: &[u8; 32]) -> String {
        digest.iter().map(|byte| format!("{byte:02x}")).collect()
    }

    #[cfg(feature = "private-windows-lifecycle-proof")]
    pub(super) fn terminate_running_service_for_proof(
        expected_sha256: [u8; 32],
    ) -> Result<TerminatedServiceForProof, Box<ServiceTerminationFailure>> {
        let before_mutation = |reason| {
            Box::new(ServiceTerminationFailure {
                reason,
                service_settled: true,
                target: None,
                terminal_observation: None,
            })
        };
        require_elevated().map_err(before_mutation)?;
        if expected_sha256 == [0; 32] {
            return Err(before_mutation(
                "collector_service_proof_digest_invalid".to_string(),
            ));
        }
        let service_path =
            expected_service_path(&known_folder(CSIDL_PROGRAM_FILES).map_err(before_mutation)?);
        let manager = open_manager(SC_MANAGER_CONNECT).map_err(before_mutation)?;
        let service = open_service(
            &manager,
            SERVICE_QUERY_STATUS
                | windows_sys::Win32::System::Services::SERVICE_QUERY_CONFIG
                | READ_CONTROL,
        )
        .map_err(before_mutation)?
        .ok_or_else(|| before_mutation("collector_service_proof_service_missing".to_string()))?;
        validate_service_contract(&service, &service_path).map_err(before_mutation)?;
        let status = query_service_status(&service).map_err(before_mutation)?;
        if status.dwCurrentState != SERVICE_RUNNING || status.dwProcessId == 0 {
            return Err(before_mutation(
                "collector_service_proof_service_not_running".to_string(),
            ));
        }
        validate_running_service_image(
            &service,
            &service_path,
            expected_sha256,
            status.dwProcessId,
        )
        .map_err(before_mutation)?;
        let process = OwnedHandle::new(
            unsafe {
                OpenProcess(
                    PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_TERMINATE | SYNCHRONIZE_ACCESS,
                    0,
                    status.dwProcessId,
                )
            },
            "collector_service_proof_process_open_failed",
        )
        .map_err(before_mutation)?;
        let process_started_at_100ns =
            crate::collector_service::windows_transport::process_started_at(process.raw())
                .map_err(|error| {
                    before_mutation(format!(
                        "collector_service_proof_process_time_failed:{error}"
                    ))
                })?;
        let current = query_service_status(&service).map_err(before_mutation)?;
        if current.dwCurrentState != SERVICE_RUNNING
            || current.dwProcessId != status.dwProcessId
            || unsafe { WaitForSingleObject(process.raw(), 0) } != WAIT_TIMEOUT
        {
            return Err(before_mutation(
                "collector_service_proof_process_generation_changed".to_string(),
            ));
        }
        let mut process_path = vec![0_u16; 32_768];
        let mut process_path_length = process_path.len() as u32;
        if unsafe {
            QueryFullProcessImageNameW(
                process.raw(),
                0,
                process_path.as_mut_ptr(),
                &mut process_path_length,
            )
        } == 0
        {
            return Err(before_mutation(last_error(
                "collector_service_proof_process_path_failed",
            )));
        }
        process_path.truncate(process_path_length as usize);
        let process_path =
            strip_verbatim_disk_prefix(PathBuf::from(OsString::from_wide(&process_path)));
        if !fixed_path_eq(&process_path, &service_path) {
            return Err(before_mutation(
                "collector_service_proof_process_path_invalid".to_string(),
            ));
        }
        let target = ServiceTerminationTargetForProof {
            process_id: status.dwProcessId,
            process_started_at_100ns,
            image_path: process_path.to_string_lossy().into_owned(),
            image_sha256: expected_sha256
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect(),
        };
        if unsafe { TerminateProcess(process.raw(), 1) } == 0 {
            return Err(termination_failure(
                last_error("collector_service_proof_terminate_failed"),
                &target,
                &service,
                &process,
                false,
            ));
        }
        wait_service_state(&service, SERVICE_STOPPED, SERVICE_OPERATION_TIMEOUT)
            .map_err(|reason| termination_failure(reason, &target, &service, &process, false))?;
        prove_service_lifecycle_settled(true)
            .map_err(|reason| termination_failure(reason, &target, &service, &process, false))?;
        wait_service_process_exit(&process)
            .map_err(|reason| termination_failure(reason, &target, &service, &process, true))?;
        let mut process_exit_code = 0_u32;
        if unsafe { GetExitCodeProcess(process.raw(), &mut process_exit_code) } == 0 {
            return Err(termination_failure(
                last_error("collector_service_proof_process_exit_query_failed"),
                &target,
                &service,
                &process,
                true,
            ));
        }
        if process_exit_code != 1 {
            return Err(termination_failure(
                "collector_service_proof_process_exit_code_invalid".to_string(),
                &target,
                &service,
                &process,
                true,
            ));
        }
        let stopped = query_service_status(&service)
            .map_err(|reason| termination_failure(reason, &target, &service, &process, true))?;
        if stopped.dwCurrentState != SERVICE_STOPPED || stopped.dwProcessId != 0 {
            return Err(termination_failure(
                "collector_service_proof_crash_settlement_unproven".to_string(),
                &target,
                &service,
                &process,
                true,
            ));
        }
        Ok(TerminatedServiceForProof {
            target,
            process_exit_code,
            win32_exit_code: stopped.dwWin32ExitCode,
            service_specific_exit_code: stopped.dwServiceSpecificExitCode,
        })
    }

    #[cfg(feature = "private-windows-lifecycle-proof")]
    fn termination_failure(
        reason: String,
        target: &ServiceTerminationTargetForProof,
        service: &OwnedScHandle,
        process: &OwnedHandle,
        lifecycle_marker_settled: bool,
    ) -> Box<ServiceTerminationFailure> {
        let service_status = query_service_status(service).ok();
        let process_exited = unsafe { WaitForSingleObject(process.raw(), 0) } == WAIT_OBJECT_0;
        let process_exit_code = process_exited.then(|| {
            let mut exit_code = 0_u32;
            (unsafe { GetExitCodeProcess(process.raw(), &mut exit_code) } != 0).then_some(exit_code)
        });
        let terminal_observation = ServiceTerminationObservationForProof {
            service_state: service_status.as_ref().map(|status| status.dwCurrentState),
            service_process_id: service_status.as_ref().map(|status| status.dwProcessId),
            win32_exit_code: service_status.as_ref().map(|status| status.dwWin32ExitCode),
            service_specific_exit_code: service_status
                .as_ref()
                .map(|status| status.dwServiceSpecificExitCode),
            lifecycle_marker_settled,
            process_exited,
            process_exit_code: process_exit_code.flatten(),
        };
        let service_stopped = matches!(
            service_status,
            Some(status)
                if status.dwCurrentState == SERVICE_STOPPED && status.dwProcessId == 0
        );
        Box::new(ServiceTerminationFailure {
            reason,
            service_settled: lifecycle_marker_settled && process_exited && service_stopped,
            target: Some(target.clone()),
            terminal_observation: Some(terminal_observation),
        })
    }

    struct QueriedServiceConfig {
        service_type: u32,
        start_type: u32,
        error_control: u32,
        image_path: PathBuf,
        account: String,
    }

    fn query_service_config(service: &OwnedScHandle) -> Result<QueriedServiceConfig, String> {
        let mut needed = 0_u32;
        unsafe {
            QueryServiceConfigW(service.raw(), ptr::null_mut(), 0, &mut needed);
        }
        if unsafe { GetLastError() } != ERROR_INSUFFICIENT_BUFFER || needed == 0 {
            return Err(last_error("collector_service_config_size_failed"));
        }
        let mut buffer = aligned_buffer(needed as usize);
        let config = buffer.as_mut_ptr().cast::<QUERY_SERVICE_CONFIGW>();
        if unsafe { QueryServiceConfigW(service.raw(), config, needed, &mut needed) } == 0 {
            return Err(last_error("collector_service_config_query_failed"));
        }
        let config = unsafe { &*config };
        let binary = wide_ptr_string(config.lpBinaryPathName)?;
        let image_path = unquote_service_path(&binary)?;
        Ok(QueriedServiceConfig {
            service_type: config.dwServiceType,
            start_type: config.dwStartType,
            error_control: config.dwErrorControl,
            image_path,
            account: wide_ptr_string(config.lpServiceStartName)?,
        })
    }

    fn query_config2_fixed<T: Copy + Default>(
        service: &OwnedScHandle,
        level: u32,
        context: &str,
    ) -> Result<T, String> {
        let mut value = T::default();
        let mut needed = 0_u32;
        if unsafe {
            QueryServiceConfig2W(
                service.raw(),
                level,
                (&mut value as *mut T).cast(),
                size_of::<T>() as u32,
                &mut needed,
            )
        } == 0
        {
            return Err(last_error(context));
        }
        Ok(value)
    }

    fn query_required_privileges(service: &OwnedScHandle) -> Result<Vec<String>, String> {
        let mut needed = 0_u32;
        unsafe {
            QueryServiceConfig2W(
                service.raw(),
                SERVICE_CONFIG_REQUIRED_PRIVILEGES_INFO,
                ptr::null_mut(),
                0,
                &mut needed,
            );
        }
        if unsafe { GetLastError() } != ERROR_INSUFFICIENT_BUFFER || needed == 0 {
            return Err(last_error("collector_service_privileges_size_failed"));
        }
        let mut buffer = aligned_buffer(needed as usize);
        if unsafe {
            QueryServiceConfig2W(
                service.raw(),
                SERVICE_CONFIG_REQUIRED_PRIVILEGES_INFO,
                buffer.as_mut_ptr().cast(),
                needed,
                &mut needed,
            )
        } == 0
        {
            return Err(last_error("collector_service_privileges_query_failed"));
        }
        let info = unsafe { &*buffer.as_ptr().cast::<SERVICE_REQUIRED_PRIVILEGES_INFOW>() };
        read_multi_wide(info.pmszRequiredPrivileges)
    }

    fn service_dacl_policy(service: &OwnedScHandle) -> Result<(Vec<AcePolicy>, String), String> {
        let mut needed = 0_u32;
        unsafe {
            QueryServiceObjectSecurity(
                service.raw(),
                DACL_SECURITY_INFORMATION,
                ptr::null_mut(),
                0,
                &mut needed,
            );
        }
        if unsafe { GetLastError() } != ERROR_INSUFFICIENT_BUFFER || needed == 0 {
            return Err(last_error("collector_service_dacl_size_failed"));
        }
        let mut buffer = aligned_buffer(needed as usize);
        let descriptor = buffer.as_mut_ptr().cast();
        if unsafe {
            QueryServiceObjectSecurity(
                service.raw(),
                DACL_SECURITY_INFORMATION,
                descriptor,
                needed,
                &mut needed,
            )
        } == 0
        {
            return Err(last_error("collector_service_dacl_query_failed"));
        }
        let mut present = 0_i32;
        let mut defaulted = 0_i32;
        let mut dacl = ptr::null_mut();
        if unsafe { GetSecurityDescriptorDacl(descriptor, &mut present, &mut dacl, &mut defaulted) }
            == 0
            || present == 0
            || dacl.is_null()
        {
            return Err(last_error("collector_service_dacl_invalid"));
        }
        let principals = SecurityPrincipals::load_base()?;
        let dacl_sha256 = dacl_sha256(dacl, "collector_service_dacl")?;
        Ok((read_aces(dacl, &principals)?, dacl_sha256))
    }

    fn validate_service_dacl(service: &OwnedScHandle) -> Result<(), String> {
        let (aces, _) = service_dacl_policy(service)?;
        validate_service_dacl_policy(&aces)
    }

    fn validate_service_dacl_policy(aces: &[AcePolicy]) -> Result<(), String> {
        let expected = [
            (PrincipalClass::LocalSystem, SERVICE_ALL_ACCESS),
            (PrincipalClass::Administrators, SERVICE_ALL_ACCESS),
            (PrincipalClass::InteractiveUsers, SERVICE_QUERY_STATUS_MASK),
        ];
        if aces.len() != expected.len() {
            return Err("collector_service_dacl_contract_invalid".to_string());
        }
        for (principal, mask) in expected {
            if aces
                .iter()
                .filter(|ace| {
                    ace.principal == principal
                        && ace.allow
                        && !ace.inherit_only
                        && !ace.object_inherit
                        && !ace.container_inherit
                        && ace.mask == mask
                })
                .count()
                != 1
            {
                return Err("collector_service_dacl_contract_invalid".to_string());
            }
        }
        Ok(())
    }

    fn set_owner_marker() -> Result<(), String> {
        let key = open_service_registry_key(KEY_SET_VALUE | KEY_QUERY_VALUE)?;
        let name = wide(SERVICE_OWNER_VALUE);
        let value = wide(SERVICE_OWNER_MARKER);
        let status = unsafe {
            RegSetValueExW(
                key.0,
                name.as_ptr(),
                0,
                REG_SZ,
                value.as_ptr().cast(),
                (value.len() * size_of::<u16>()) as u32,
            )
        };
        if status != 0 {
            return Err(format!(
                "collector_service_owner_marker_write_failed:{status}"
            ));
        }
        Ok(())
    }

    pub(super) fn record_service_failure(category: &str) -> Result<(), String> {
        let key = open_service_registry_key(KEY_SET_VALUE)?;
        let name = wide(SERVICE_FAILURE_VALUE);
        let value = wide(category);
        let status = unsafe {
            RegSetValueExW(
                key.0,
                name.as_ptr(),
                0,
                REG_SZ,
                value.as_ptr().cast(),
                (value.len() * size_of::<u16>()) as u32,
            )
        };
        if status == 0 {
            Ok(())
        } else {
            Err(format!("collector_service_failure_record_failed:{status}"))
        }
    }

    pub(super) fn clear_service_failure() -> Result<(), String> {
        let key = open_service_registry_key(KEY_SET_VALUE)?;
        let name = wide(SERVICE_FAILURE_VALUE);
        let status = unsafe { RegDeleteValueW(key.0, name.as_ptr()) };
        if status == 0 || status == ERROR_FILE_NOT_FOUND {
            Ok(())
        } else {
            Err(format!("collector_service_failure_clear_failed:{status}"))
        }
    }

    fn read_owner_marker() -> Result<Option<String>, String> {
        let key = match open_service_registry_key(KEY_QUERY_VALUE) {
            Ok(key) => key,
            Err(error)
                if error
                    == format!("collector_service_registry_open_failed:{ERROR_FILE_NOT_FOUND}") =>
            {
                return Ok(None);
            }
            Err(error) => return Err(error),
        };
        let name = wide(SERVICE_OWNER_VALUE);
        let mut value_type = 0_u32;
        let mut bytes = 0_u32;
        let status = unsafe {
            RegQueryValueExW(
                key.0,
                name.as_ptr(),
                ptr::null_mut(),
                &mut value_type,
                ptr::null_mut(),
                &mut bytes,
            )
        };
        if status == ERROR_FILE_NOT_FOUND {
            return Ok(None);
        }
        if status != 0 || value_type != REG_SZ || bytes < 2 {
            return Err(format!(
                "collector_service_owner_marker_query_failed:{status}"
            ));
        }
        let mut value = vec![0_u16; (bytes as usize).div_ceil(size_of::<u16>())];
        let status = unsafe {
            RegQueryValueExW(
                key.0,
                name.as_ptr(),
                ptr::null_mut(),
                &mut value_type,
                value.as_mut_ptr().cast(),
                &mut bytes,
            )
        };
        if status != 0 {
            return Err(format!(
                "collector_service_owner_marker_query_failed:{status}"
            ));
        }
        let length = value
            .iter()
            .position(|item| *item == 0)
            .unwrap_or(value.len());
        Ok(Some(String::from_utf16_lossy(&value[..length])))
    }

    fn read_installed_product_version() -> Result<Option<String>, String> {
        let path = wide(PRODUCT_UNINSTALL_REGISTRY_PATH);
        let mut raw = ptr::null_mut();
        let status = unsafe {
            RegOpenKeyExW(
                HKEY_LOCAL_MACHINE,
                path.as_ptr(),
                0,
                KEY_QUERY_VALUE,
                &mut raw,
            )
        };
        if status == ERROR_FILE_NOT_FOUND {
            return Ok(None);
        }
        if status != 0 {
            return Err(format!(
                "collector_service_product_registry_open_failed:{status}"
            ));
        }
        let key = OwnedRegistryKey(raw);
        let name = wide("DisplayVersion");
        let mut value_type = 0_u32;
        let mut bytes = 0_u32;
        let status = unsafe {
            RegQueryValueExW(
                key.0,
                name.as_ptr(),
                ptr::null_mut(),
                &mut value_type,
                ptr::null_mut(),
                &mut bytes,
            )
        };
        if status == ERROR_FILE_NOT_FOUND {
            return Ok(None);
        }
        if status != 0 || value_type != REG_SZ || !(2..=1024).contains(&bytes) {
            return Err(format!(
                "collector_service_product_version_query_failed:{status}"
            ));
        }
        let mut value = vec![0_u16; (bytes as usize).div_ceil(size_of::<u16>())];
        let status = unsafe {
            RegQueryValueExW(
                key.0,
                name.as_ptr(),
                ptr::null_mut(),
                &mut value_type,
                value.as_mut_ptr().cast(),
                &mut bytes,
            )
        };
        if status != 0 || value_type != REG_SZ {
            return Err(format!(
                "collector_service_product_version_query_failed:{status}"
            ));
        }
        let length = value
            .iter()
            .position(|item| *item == 0)
            .unwrap_or(value.len());
        let version = String::from_utf16(&value[..length])
            .map_err(|_| "collector_service_product_version_invalid".to_string())?;
        if version.is_empty() {
            Ok(None)
        } else {
            Ok(Some(version))
        }
    }

    fn open_service_registry_key(access: u32) -> Result<OwnedRegistryKey, String> {
        let path = wide(SERVICE_REGISTRY_PATH);
        let mut key = ptr::null_mut();
        let status =
            unsafe { RegOpenKeyExW(HKEY_LOCAL_MACHINE, path.as_ptr(), 0, access, &mut key) };
        if status == 0 {
            Ok(OwnedRegistryKey(key))
        } else {
            Err(format!("collector_service_registry_open_failed:{status}"))
        }
    }

    fn query_service_status(service: &OwnedScHandle) -> Result<SERVICE_STATUS_PROCESS, String> {
        let mut status = SERVICE_STATUS_PROCESS::default();
        let mut needed = 0_u32;
        if unsafe {
            QueryServiceStatusEx(
                service.raw(),
                SC_STATUS_PROCESS_INFO,
                (&mut status as *mut SERVICE_STATUS_PROCESS).cast(),
                size_of::<SERVICE_STATUS_PROCESS>() as u32,
                &mut needed,
            )
        } == 0
        {
            return Err(last_error("collector_service_status_query_failed"));
        }
        Ok(status)
    }

    fn start_service_and_wait(service: &OwnedScHandle) -> Result<(), String> {
        let mut status = query_service_status(service)?;
        if status.dwCurrentState == SERVICE_RUNNING {
            return Ok(());
        }
        if status.dwCurrentState == SERVICE_START_PENDING {
            return wait_service_state(service, SERVICE_RUNNING, SERVICE_OPERATION_TIMEOUT);
        }
        if status.dwCurrentState == SERVICE_STOP_PENDING {
            wait_service_state(service, SERVICE_STOPPED, SERVICE_OPERATION_TIMEOUT)?;
            status = query_service_status(service)?;
        }
        if status.dwCurrentState != SERVICE_STOPPED {
            return Err(format!(
                "collector_service_start_state_invalid:{}",
                status.dwCurrentState
            ));
        }
        if unsafe { StartServiceW(service.raw(), 0, ptr::null()) } == 0 {
            let error = unsafe { GetLastError() };
            if error != ERROR_SERVICE_ALREADY_RUNNING {
                return Err(format!("collector_service_start_failed:{error}"));
            }
        }
        wait_service_state(service, SERVICE_RUNNING, SERVICE_OPERATION_TIMEOUT)
    }

    fn open_service_process(status: &SERVICE_STATUS_PROCESS) -> Result<OwnedHandle, String> {
        if status.dwProcessId == 0 {
            return Err("collector_service_process_pid_invalid".to_string());
        }
        let handle = unsafe {
            OpenProcess(
                PROCESS_QUERY_LIMITED_INFORMATION | SYNCHRONIZE_ACCESS,
                0,
                status.dwProcessId,
            )
        };
        if handle.is_null() {
            return Err(last_error("collector_service_process_open_failed"));
        }
        Ok(OwnedHandle(handle))
    }

    fn validate_running_service_image(
        service: &OwnedScHandle,
        expected_path: &Path,
        expected_digest: [u8; 32],
        expected_process_id: u32,
    ) -> Result<(), String> {
        let status = query_service_status(service)?;
        if status.dwCurrentState != SERVICE_RUNNING
            || expected_process_id == 0
            || status.dwProcessId != expected_process_id
        {
            return Err("collector_service_upgrade_running_state_unproven".to_string());
        }
        let process = open_service_process(&status)?;
        let mut path = vec![0_u16; 32_768];
        let mut length = path.len() as u32;
        if unsafe { QueryFullProcessImageNameW(process.raw(), 0, path.as_mut_ptr(), &mut length) }
            == 0
        {
            return Err(last_error(
                "collector_service_upgrade_process_image_query_failed",
            ));
        }
        path.truncate(length as usize);
        let process_path = strip_verbatim_disk_prefix(PathBuf::from(OsString::from_wide(&path)));
        if !fixed_path_eq(&process_path, expected_path)
            || trusted_file_digest(expected_path, "collector_service_stable_image")?
                != expected_digest
        {
            return Err("collector_service_upgrade_running_image_invalid".to_string());
        }
        let current = query_service_status(service)?;
        if current.dwCurrentState != SERVICE_RUNNING
            || current.dwProcessId != expected_process_id
            || unsafe { WaitForSingleObject(process.raw(), 0) } == WAIT_OBJECT_0
        {
            return Err("collector_service_upgrade_running_generation_changed".to_string());
        }
        Ok(())
    }

    fn start_upgrade_service_generation(
        service: &OwnedScHandle,
        require_clean_stop: bool,
    ) -> Result<u32, String> {
        let status = query_service_status(service)?;
        if status.dwCurrentState != SERVICE_STOPPED || status.dwProcessId != 0 {
            return Err("collector_service_upgrade_start_state_unproven".to_string());
        }
        if require_clean_stop {
            validate_clean_stopped_status(&status)?;
        } else {
            prove_service_lifecycle_settled(false)?;
        }
        if unsafe { StartServiceW(service.raw(), 0, ptr::null()) } == 0 {
            return Err(last_error("collector_service_upgrade_start_failed"));
        }
        wait_service_state(service, SERVICE_RUNNING, SERVICE_OPERATION_TIMEOUT)?;
        let running = query_service_status(service)?;
        if running.dwCurrentState != SERVICE_RUNNING || running.dwProcessId == 0 {
            return Err("collector_service_upgrade_running_state_unproven".to_string());
        }
        Ok(running.dwProcessId)
    }

    fn wait_service_process_exit(process: &OwnedHandle) -> Result<(), String> {
        let wait = unsafe {
            WaitForSingleObject(process.raw(), SERVICE_OPERATION_TIMEOUT.as_millis() as u32)
        };
        if wait == WAIT_OBJECT_0 {
            Ok(())
        } else {
            Err(format!("collector_service_process_exit_unproven:{wait}"))
        }
    }

    fn stop_service_and_wait(
        service: &OwnedScHandle,
        lifecycle_required_if_stopped: bool,
    ) -> Result<(), String> {
        let mut status = query_service_status(service)?;
        if status.dwCurrentState == SERVICE_START_PENDING {
            wait_service_state(service, SERVICE_RUNNING, SERVICE_OPERATION_TIMEOUT)?;
            status = query_service_status(service)?;
        }
        if status.dwCurrentState == SERVICE_STOPPED {
            prove_service_lifecycle_settled(lifecycle_required_if_stopped)?;
            return validate_clean_stopped_status(&status);
        }
        if status.dwCurrentState == SERVICE_STOP_PENDING {
            require_service_lifecycle_active()?;
            let process = open_service_process(&status)?;
            wait_service_state(service, SERVICE_STOPPED, SERVICE_OPERATION_TIMEOUT)?;
            prove_service_lifecycle_settled(true)?;
            wait_service_process_exit(&process)?;
            status = query_service_status(service)?;
            return validate_clean_stopped_status(&status);
        }
        if status.dwCurrentState != SERVICE_RUNNING {
            return Err(format!(
                "collector_service_stop_state_invalid:{}",
                status.dwCurrentState
            ));
        }
        require_service_lifecycle_active()?;
        let process = open_service_process(&status)?;
        let mut basic = SERVICE_STATUS::default();
        if unsafe { ControlService(service.raw(), SERVICE_CONTROL_STOP, &mut basic) } == 0 {
            let error = unsafe { GetLastError() };
            if error != ERROR_SERVICE_NOT_ACTIVE {
                return Err(format!("collector_service_stop_failed:{error}"));
            }
        }
        wait_service_state(service, SERVICE_STOPPED, SERVICE_OPERATION_TIMEOUT)?;
        prove_service_lifecycle_settled(true)?;
        wait_service_process_exit(&process)?;
        status = query_service_status(service)?;
        validate_clean_stopped_status(&status)
    }

    fn settle_service_for_replacement(service: &OwnedScHandle) -> Result<bool, String> {
        let status = query_service_status(service)?;
        let was_active = status.dwCurrentState != SERVICE_STOPPED;
        if status.dwCurrentState == SERVICE_STOPPED {
            if !stopped_service_can_be_replaced(&status) {
                return Err("collector_service_stopped_process_present".to_string());
            }
            prove_service_lifecycle_settled(false)?;
            return Ok(false);
        }
        stop_service_and_wait(service, true)?;
        Ok(was_active)
    }

    pub(super) fn stopped_service_can_be_replaced(status: &SERVICE_STATUS_PROCESS) -> bool {
        status.dwCurrentState == SERVICE_STOPPED && status.dwProcessId == 0
    }

    pub(super) fn stopped_status_requires_repair(status: &SERVICE_STATUS_PROCESS) -> bool {
        status.dwCurrentState == SERVICE_STOPPED && status.dwWin32ExitCode != ERROR_SUCCESS
    }

    pub(super) fn validate_clean_stopped_status(
        status: &SERVICE_STATUS_PROCESS,
    ) -> Result<(), String> {
        if status.dwCurrentState != SERVICE_STOPPED {
            return Err("collector_service_stop_settlement_unproven".to_string());
        }
        if status.dwWin32ExitCode != ERROR_SUCCESS {
            return Err(format!(
                "collector_service_stop_reported_failure:{}:{}",
                status.dwWin32ExitCode, status.dwServiceSpecificExitCode
            ));
        }
        Ok(())
    }

    fn wait_service_state(
        service: &OwnedScHandle,
        expected: u32,
        timeout: Duration,
    ) -> Result<(), String> {
        let deadline = Instant::now() + timeout;
        loop {
            let status = query_service_status(service)?;
            if status.dwCurrentState == expected {
                return Ok(());
            }
            if Instant::now() >= deadline {
                return Err(format!(
                    "collector_service_state_timeout:{}:{expected}",
                    status.dwCurrentState
                ));
            }
            thread::sleep(SERVICE_POLL_INTERVAL);
        }
    }

    fn wait_service_deleted(manager: &OwnedScHandle) -> Result<(), String> {
        let deadline = Instant::now() + SERVICE_OPERATION_TIMEOUT;
        loop {
            match open_service(manager, SERVICE_QUERY_STATUS) {
                Ok(None) => return Ok(()),
                Ok(Some(service)) => drop(service),
                Err(error) if error.ends_with(&format!(":{ERROR_SERVICE_MARKED_FOR_DELETE}")) => {}
                Err(error) => return Err(error),
            }
            if Instant::now() >= deadline {
                return Err("collector_service_delete_pending_reboot_required".to_string());
            }
            thread::sleep(SERVICE_POLL_INTERVAL);
        }
    }

    fn cleanup_created_roots(
        product_root_created: bool,
        service_root_created: bool,
        principals: &SecurityPrincipals,
    ) -> Result<(), String> {
        if service_root_created {
            cleanup_roots_if_owned(product_root_created, principals)
        } else if product_root_created {
            cleanup_product_root_if_owned(principals)
        } else {
            Ok(())
        }
    }

    #[derive(Default)]
    struct RootCreationJournal {
        product: bool,
        service: bool,
    }

    fn cleanup_product_root_if_owned(principals: &SecurityPrincipals) -> Result<(), String> {
        let roots = fixed_roots()?;
        let _program_data = open_directory(
            &roots.program_data,
            "collector_service_programdata_open_failed",
        )?;
        let product = open_and_verify_root(&roots.product, false, principals)?;
        drop(product);
        remove_directory(&roots.product)
    }

    fn cleanup_roots_if_owned(
        remove_product: bool,
        principals: &SecurityPrincipals,
    ) -> Result<(), String> {
        let roots = fixed_roots()?;
        let _program_data = open_directory(
            &roots.program_data,
            "collector_service_programdata_open_failed",
        )?;
        let product = open_and_verify_root(&roots.product, false, principals)?;
        let service = open_and_verify_root(&roots.service, true, principals)?;
        for leaf in [
            ETW_LEASE_FILE_NAME,
            ETW_OWNER_LOCK_FILE_NAME,
            SERVICE_LIFECYCLE_LOCK_FILE_NAME,
            UPGRADE_JOURNAL_FILE_NAME,
        ] {
            let path = roots.service.join(leaf);
            drop(verify_optional_leaf(&path, principals)?);
            let path_wide = wide_path(&path);
            if unsafe { DeleteFileW(path_wide.as_ptr()) } == 0 {
                let error = unsafe { GetLastError() };
                if error != ERROR_FILE_NOT_FOUND {
                    return Err(format!("collector_service_root_leaf_remove_failed:{error}"));
                }
            }
        }
        cleanup_stale_atomic_leaves(&roots.service, principals)?;
        drop(service);
        remove_directory(&roots.service)?;
        if remove_product {
            drop(product);
            remove_directory(&roots.product)
        } else {
            Ok(())
        }
    }

    fn remove_directory(path: &Path) -> Result<(), String> {
        let path = wide_path(path);
        if unsafe { RemoveDirectoryW(path.as_ptr()) } == 0 {
            Err(last_error("collector_service_root_remove_failed"))
        } else {
            Ok(())
        }
    }

    fn cleanup_stale_atomic_leaves(
        service_root: &Path,
        principals: &SecurityPrincipals,
    ) -> Result<(), String> {
        let entries = fs::read_dir(service_root)
            .map_err(|error| format!("collector_service_root_enumerate_failed:{error}"))?;
        for entry in entries {
            let entry = entry
                .map_err(|error| format!("collector_service_root_enumerate_failed:{error}"))?;
            let name = entry.file_name();
            let Some(name) = name.to_str() else {
                continue;
            };
            if !is_owned_atomic_temp_name(name) {
                continue;
            }
            let path = entry.path();
            drop(verify_optional_leaf(&path, principals)?);
            let path_wide = wide_path(&path);
            if unsafe { DeleteFileW(path_wide.as_ptr()) } == 0 {
                let error = unsafe { GetLastError() };
                if error != ERROR_FILE_NOT_FOUND {
                    return Err(format!(
                        "collector_service_root_temp_leaf_remove_failed:{error}"
                    ));
                }
            }
        }
        Ok(())
    }

    pub(super) fn is_owned_atomic_temp_name(name: &str) -> bool {
        [ETW_LEASE_FILE_NAME, UPGRADE_JOURNAL_FILE_NAME]
            .iter()
            .any(|base| atomic_temp_suffix(name, base).is_some())
    }

    fn atomic_temp_suffix<'a>(name: &'a str, base: &str) -> Option<&'a str> {
        let suffix = name
            .strip_prefix(base)?
            .strip_prefix('.')?
            .strip_suffix(".tmp")?;
        let mut parts = suffix.split('.');
        matches!(
            (parts.next(), parts.next(), parts.next()),
            (Some(process), Some(sequence), None)
                if !process.is_empty()
                    && !sequence.is_empty()
                    && process.bytes().all(|byte| byte.is_ascii_digit())
                    && sequence.bytes().all(|byte| byte.is_ascii_digit())
        )
        .then_some(suffix)
    }

    fn aligned_buffer(bytes: usize) -> Vec<usize> {
        vec![0; bytes.div_ceil(size_of::<usize>())]
    }

    fn quoted_service_path(path: &Path) -> Vec<u16> {
        std::iter::once(u16::from(b'"'))
            .chain(path.as_os_str().encode_wide())
            .chain([u16::from(b'"'), 0])
            .collect()
    }

    fn unquote_service_path(value: &str) -> Result<PathBuf, String> {
        let Some(value) = value
            .strip_prefix('"')
            .and_then(|value| value.strip_suffix('"'))
        else {
            return Err("collector_service_image_path_unquoted".to_string());
        };
        if value.contains('"') {
            return Err("collector_service_image_path_invalid".to_string());
        }
        Ok(PathBuf::from(value))
    }

    fn multi_wide(values: &[&str]) -> Vec<u16> {
        let mut result = Vec::new();
        for value in values {
            result.extend(value.encode_utf16());
            result.push(0);
        }
        result.push(0);
        result
    }

    fn read_multi_wide(value: *const u16) -> Result<Vec<String>, String> {
        if value.is_null() {
            return Err("collector_service_privileges_missing".to_string());
        }
        let mut result = Vec::new();
        let mut offset = 0_usize;
        loop {
            let mut length = 0_usize;
            while unsafe { *value.add(offset + length) } != 0 {
                length += 1;
                if length > 32_768 {
                    return Err("collector_service_privileges_invalid".to_string());
                }
            }
            if length == 0 {
                return Ok(result);
            }
            result.push(String::from_utf16_lossy(unsafe {
                std::slice::from_raw_parts(value.add(offset), length)
            }));
            offset += length + 1;
        }
    }

    fn wide_ptr_string(value: *const u16) -> Result<String, String> {
        if value.is_null() {
            return Err("collector_service_config_string_missing".to_string());
        }
        let mut length = 0_usize;
        while unsafe { *value.add(length) } != 0 {
            length += 1;
            if length > 32_768 {
                return Err("collector_service_config_string_invalid".to_string());
            }
        }
        Ok(String::from_utf16_lossy(unsafe {
            std::slice::from_raw_parts(value, length)
        }))
    }

    pub(super) fn open_protected_etw_lease_root() -> Result<ProtectedEtwLeaseRoot, String> {
        let roots = fixed_roots()?;
        let principals = SecurityPrincipals::load_with_service()?;
        let program_data = open_directory(
            &roots.program_data,
            "collector_service_programdata_open_failed",
        )?;
        let product = open_and_verify_root(&roots.product, false, &principals)?;
        let service = open_and_verify_root(&roots.service, true, &principals)?;
        let install_id = protected_root_install_id(service.raw())?;
        let mut leaves = Vec::new();
        for leaf in [
            ETW_LEASE_FILE_NAME,
            ETW_OWNER_LOCK_FILE_NAME,
            UPGRADE_JOURNAL_FILE_NAME,
        ] {
            if let Some(handle) = verify_optional_leaf(&roots.service.join(leaf), &principals)? {
                if retain_verified_leaf(leaf) {
                    leaves.push(handle);
                }
            }
        }
        let guard = ProtectedRootGuard {
            _program_data: program_data,
            _product: product,
            _service: service,
            _leaves: leaves,
        };
        unsafe { ProtectedEtwLeaseRoot::from_platform_verified(roots.service, install_id, guard) }
            .map_err(|error| format!("collector_service_protected_root_invalid:{error:?}"))
    }

    #[cfg(feature = "private-windows-lifecycle-proof")]
    pub(super) fn observe_protected_runtime_files_for_proof(
        expected_volume_serial: u32,
        expected_file_index: u64,
    ) -> Result<ProtectedRuntimeFilesForProof, String> {
        let roots = fixed_roots()?;
        let principals = SecurityPrincipals::load_with_service()?;
        let program_data = open_directory(
            &roots.program_data,
            "collector_service_proof_programdata_open_failed",
        )?;
        let product = open_and_verify_root(&roots.product, false, &principals)?;
        let service = open_and_verify_root(&roots.service, true, &principals)?;
        let service_identity = file_information(
            service.raw(),
            "collector_service_proof_service_root_identity_failed",
        )?;
        if service_identity.dwVolumeSerialNumber != expected_volume_serial
            || ((u64::from(service_identity.nFileIndexHigh) << 32)
                | u64::from(service_identity.nFileIndexLow))
                != expected_file_index
        {
            return Err("collector_service_proof_service_root_identity_mismatch".to_string());
        }
        let lease_path = roots.service.join(ETW_LEASE_FILE_NAME);
        let lease_security = verify_optional_leaf(&lease_path, &principals)?;
        let lease_identity = optional_leaf_identity_for_proof(
            lease_security.as_ref(),
            "collector_service_proof_lease_identity_failed",
        )?;
        let owner_lock_path = roots.service.join(ETW_OWNER_LOCK_FILE_NAME);
        let owner_lock_security = verify_optional_leaf(&owner_lock_path, &principals)?;
        let lifecycle_lock_path = roots.service.join(SERVICE_LIFECYCLE_LOCK_FILE_NAME);
        let lifecycle_lock_security = verify_optional_leaf(&lifecycle_lock_path, &principals)?;

        let mut etw_lease = observe_lease_read_only_for_proof(&lease_path, lease_identity);
        match verify_optional_leaf(&lease_path, &principals).and_then(|handle| {
            optional_leaf_identity_for_proof(
                handle.as_ref(),
                "collector_service_proof_lease_revalidate_identity_failed",
            )
        }) {
            Ok(revalidated_identity) if revalidated_identity == lease_identity => {}
            Ok(_) => {
                etw_lease = super::super::etw_lease::ReadOnlyEtwLeaseObservation::Unknown(
                    "etw_lease_proof_identity_changed_after_observation".to_string(),
                );
            }
            Err(reason) => {
                etw_lease = super::super::etw_lease::ReadOnlyEtwLeaseObservation::Unknown(format!(
                    "etw_lease_proof_revalidate_failed:{reason}"
                ));
            }
        }

        let observation = ProtectedRuntimeFilesForProof {
            etw_lease,
            etw_owner_lock: observe_runtime_lock_for_proof(
                &owner_lock_path,
                owner_lock_security.as_ref(),
                "etw_owner",
            ),
            service_lifecycle_lock: observe_runtime_lock_for_proof(
                &lifecycle_lock_path,
                lifecycle_lock_security.as_ref(),
                "service_lifecycle",
            ),
        };

        let revalidated = open_and_verify_root(&roots.service, true, &principals)?;
        let revalidated_identity = file_information(
            revalidated.raw(),
            "collector_service_proof_service_root_revalidate_failed",
        )?;
        if !same_file_identity(&service_identity, &revalidated_identity) {
            return Err("collector_service_proof_service_root_changed".to_string());
        }
        drop((
            lifecycle_lock_security,
            owner_lock_security,
            lease_security,
            revalidated,
            service,
            product,
            program_data,
        ));
        Ok(observation)
    }

    #[cfg(feature = "private-windows-lifecycle-proof")]
    fn observe_runtime_lock_for_proof(
        path: &Path,
        validated_leaf: Option<&OwnedHandle>,
        label: &str,
    ) -> RuntimeLockObservation {
        let initial_identity = match optional_leaf_identity_for_proof(
            validated_leaf,
            "collector_service_proof_runtime_lock_pin_identity_failed",
        ) {
            Ok(identity) => identity,
            Err(reason) => {
                return RuntimeLockObservation::Unknown {
                    reason: format!("collector_service_proof_{label}_lock_untrusted:{reason}"),
                };
            }
        };
        let path = wide_path(path);
        let file = unsafe {
            CreateFileW(
                path.as_ptr(),
                FILE_WRITE_DATA,
                FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
                ptr::null(),
                OPEN_EXISTING,
                FILE_FLAG_OPEN_REPARSE_POINT,
                ptr::null_mut(),
            )
        };
        let probe = if !file.is_null() && file != (-1_isize as HANDLE) {
            let file = OwnedHandle(file);
            match file_information(
                file.raw(),
                "collector_service_proof_runtime_lock_info_failed",
            ) {
                Ok(info) => RuntimeLockProbeForProof::Opened(file_identity_tuple(&info)),
                Err(reason) => RuntimeLockProbeForProof::Failed(reason),
            }
        } else {
            match unsafe { GetLastError() } {
                error if is_missing_path_error(error) => RuntimeLockProbeForProof::Missing,
                ERROR_SHARING_VIOLATION | ERROR_LOCK_VIOLATION => {
                    RuntimeLockProbeForProof::Contended
                }
                error => RuntimeLockProbeForProof::Failed(error.to_string()),
            }
        };
        classify_runtime_lock_observation(initial_identity, probe, label)
    }

    #[cfg(feature = "private-windows-lifecycle-proof")]
    enum RuntimeLockProbeForProof {
        Missing,
        Contended,
        Opened((u32, u64)),
        Failed(String),
    }

    #[cfg(feature = "private-windows-lifecycle-proof")]
    fn classify_runtime_lock_observation(
        initial_identity: Option<(u32, u64)>,
        probe: RuntimeLockProbeForProof,
        label: &str,
    ) -> RuntimeLockObservation {
        match (initial_identity, probe) {
            (None, RuntimeLockProbeForProof::Missing) => RuntimeLockObservation::Absent {},
            (Some(_), RuntimeLockProbeForProof::Contended) => RuntimeLockObservation::Held {},
            (Some(initial), RuntimeLockProbeForProof::Opened(probe)) if initial == probe => {
                RuntimeLockObservation::Released {}
            }
            (None, RuntimeLockProbeForProof::Opened(_)) => RuntimeLockObservation::Unknown {
                reason: format!("collector_service_proof_{label}_lock_appeared"),
            },
            (None, RuntimeLockProbeForProof::Contended) => RuntimeLockObservation::Unknown {
                reason: format!("collector_service_proof_{label}_lock_contended_after_absence"),
            },
            (Some(_), RuntimeLockProbeForProof::Missing) => RuntimeLockObservation::Unknown {
                reason: format!("collector_service_proof_{label}_lock_disappeared"),
            },
            (Some(_), RuntimeLockProbeForProof::Opened(_)) => RuntimeLockObservation::Unknown {
                reason: format!("collector_service_proof_{label}_lock_identity_changed"),
            },
            (_, RuntimeLockProbeForProof::Failed(reason)) => RuntimeLockObservation::Unknown {
                reason: format!("collector_service_proof_{label}_lock_probe_failed:{reason}"),
            },
        }
    }

    #[cfg(feature = "private-windows-lifecycle-proof")]
    fn optional_leaf_identity_for_proof(
        handle: Option<&OwnedHandle>,
        context: &str,
    ) -> Result<Option<(u32, u64)>, String> {
        handle
            .map(|handle| {
                file_information(handle.raw(), context).map(|info| file_identity_tuple(&info))
            })
            .transpose()
    }

    #[cfg(feature = "private-windows-lifecycle-proof")]
    fn file_identity_tuple(info: &BY_HANDLE_FILE_INFORMATION) -> (u32, u64) {
        (
            info.dwVolumeSerialNumber,
            (u64::from(info.nFileIndexHigh) << 32) | u64::from(info.nFileIndexLow),
        )
    }

    #[cfg(all(test, feature = "private-windows-lifecycle-proof"))]
    mod proof_runtime_observation_tests {
        use super::*;

        #[test]
        fn lock_transition_classification_is_fail_closed() {
            let identity = (7, 11);
            assert_eq!(
                classify_runtime_lock_observation(None, RuntimeLockProbeForProof::Missing, "test",),
                RuntimeLockObservation::Absent {}
            );
            assert_eq!(
                classify_runtime_lock_observation(
                    Some(identity),
                    RuntimeLockProbeForProof::Contended,
                    "test",
                ),
                RuntimeLockObservation::Held {}
            );
            assert_eq!(
                classify_runtime_lock_observation(
                    Some(identity),
                    RuntimeLockProbeForProof::Opened(identity),
                    "test",
                ),
                RuntimeLockObservation::Released {}
            );
            for observation in [
                classify_runtime_lock_observation(
                    None,
                    RuntimeLockProbeForProof::Opened(identity),
                    "test",
                ),
                classify_runtime_lock_observation(
                    None,
                    RuntimeLockProbeForProof::Contended,
                    "test",
                ),
                classify_runtime_lock_observation(
                    Some(identity),
                    RuntimeLockProbeForProof::Missing,
                    "test",
                ),
                classify_runtime_lock_observation(
                    Some(identity),
                    RuntimeLockProbeForProof::Opened((7, 12)),
                    "test",
                ),
            ] {
                assert!(matches!(
                    observation,
                    RuntimeLockObservation::Unknown { .. }
                ));
            }
        }

        #[test]
        fn metadata_pin_does_not_take_ownership_and_write_probe_detects_owner() {
            let root = std::env::temp_dir().join(format!(
                "batcave-proof-lock-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .expect("system time")
                    .as_nanos()
            ));
            std::fs::create_dir(&root).expect("lock fixture root");
            let path = root.join("runtime.lock");
            std::fs::write(&path, b"lock").expect("lock fixture");
            let path_wide = wide_path(&path);
            let pin = OwnedHandle::new(
                unsafe {
                    CreateFileW(
                        path_wide.as_ptr(),
                        FILE_READ_ATTRIBUTES | READ_CONTROL,
                        FILE_SHARE_READ | FILE_SHARE_WRITE,
                        ptr::null(),
                        OPEN_EXISTING,
                        FILE_FLAG_OPEN_REPARSE_POINT,
                        ptr::null_mut(),
                    )
                },
                "proof lock pin",
            )
            .expect("metadata pin opens");
            let identity = file_identity_tuple(
                &file_information(pin.raw(), "proof lock pin identity").expect("pin identity"),
            );
            let owner = OwnedHandle::new(
                unsafe {
                    CreateFileW(
                        path_wide.as_ptr(),
                        FILE_WRITE_DATA,
                        0,
                        ptr::null(),
                        OPEN_EXISTING,
                        FILE_FLAG_OPEN_REPARSE_POINT,
                        ptr::null_mut(),
                    )
                },
                "proof lock owner",
            )
            .expect("metadata pin must not block a real owner");
            let pin_against_owner = OwnedHandle::new(
                unsafe {
                    CreateFileW(
                        path_wide.as_ptr(),
                        FILE_READ_ATTRIBUTES | READ_CONTROL,
                        FILE_SHARE_READ | FILE_SHARE_WRITE,
                        ptr::null(),
                        OPEN_EXISTING,
                        FILE_FLAG_OPEN_REPARSE_POINT,
                        ptr::null_mut(),
                    )
                },
                "proof lock pin against owner",
            )
            .expect("metadata pin must open against a real owner");
            assert_eq!(
                file_identity_tuple(
                    &file_information(
                        pin_against_owner.raw(),
                        "proof lock pin against owner identity",
                    )
                    .expect("pin against owner identity"),
                ),
                identity
            );
            let contended = unsafe {
                CreateFileW(
                    path_wide.as_ptr(),
                    FILE_WRITE_DATA,
                    FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
                    ptr::null(),
                    OPEN_EXISTING,
                    FILE_FLAG_OPEN_REPARSE_POINT,
                    ptr::null_mut(),
                )
            };
            assert!(contended.is_null() || contended == (-1_isize as HANDLE));
            assert!(matches!(
                unsafe { GetLastError() },
                ERROR_SHARING_VIOLATION | ERROR_LOCK_VIOLATION
            ));
            drop(pin_against_owner);
            drop(owner);
            let released = OwnedHandle::new(
                unsafe {
                    CreateFileW(
                        path_wide.as_ptr(),
                        FILE_WRITE_DATA,
                        FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
                        ptr::null(),
                        OPEN_EXISTING,
                        FILE_FLAG_OPEN_REPARSE_POINT,
                        ptr::null_mut(),
                    )
                },
                "proof lock released probe",
            )
            .expect("released probe opens");
            assert_eq!(
                file_identity_tuple(
                    &file_information(released.raw(), "proof lock released identity")
                        .expect("released identity"),
                ),
                identity
            );
            drop((released, pin));
            std::fs::remove_file(path).expect("lock fixture cleanup");
            std::fs::remove_dir(root).expect("lock fixture root cleanup");
        }
    }

    #[cfg(feature = "private-windows-lifecycle-proof")]
    fn same_file_identity(
        left: &BY_HANDLE_FILE_INFORMATION,
        right: &BY_HANDLE_FILE_INFORMATION,
    ) -> bool {
        left.dwVolumeSerialNumber == right.dwVolumeSerialNumber
            && left.nFileIndexHigh == right.nFileIndexHigh
            && left.nFileIndexLow == right.nFileIndexLow
    }

    fn retain_verified_leaf(name: &str) -> bool {
        name == ETW_OWNER_LOCK_FILE_NAME
    }

    #[cfg(test)]
    pub(super) fn mutable_lease_handle_is_released_after_verification() -> bool {
        !retain_verified_leaf(ETW_LEASE_FILE_NAME) && retain_verified_leaf(ETW_OWNER_LOCK_FILE_NAME)
    }

    fn protected_root_install_id(handle: HANDLE) -> Result<[u8; 16], String> {
        let info = file_information(handle, "collector_service_root_identity_failed")?;
        let mut identity = [0_u8; 16];
        identity[..4].copy_from_slice(&info.dwVolumeSerialNumber.to_le_bytes());
        identity[4..8].copy_from_slice(&info.nFileIndexHigh.to_le_bytes());
        identity[8..12].copy_from_slice(&info.nFileIndexLow.to_le_bytes());
        identity[12..].copy_from_slice(b"BCE1");
        Ok(identity)
    }

    #[derive(Debug)]
    struct ProtectedRootGuard {
        _program_data: OwnedHandle,
        _product: OwnedHandle,
        _service: OwnedHandle,
        _leaves: Vec<OwnedHandle>,
    }

    struct FixedRoots {
        program_data: PathBuf,
        product: PathBuf,
        service: PathBuf,
    }

    fn fixed_roots() -> Result<FixedRoots, String> {
        let program_data = known_folder(CSIDL_COMMON_APPDATA)?;
        let product = program_data.join(PRODUCT_ROOT_NAME);
        let service = product.join(SERVICE_ROOT_NAME);
        Ok(FixedRoots {
            program_data,
            product,
            service,
        })
    }

    fn provision_roots(journal: &mut RootCreationJournal) -> Result<(), String> {
        let roots = fixed_roots()?;
        let principals = SecurityPrincipals::load_with_service()?;
        // An elevated administrator owns this provisioning process, so Windows
        // requires SeRestorePrivilege while assigning LocalSystem as owner.
        let _restore_privilege = EnabledPrivilege::new("SeRestorePrivilege")?;
        let _program_data = open_directory(
            &roots.program_data,
            "collector_service_programdata_open_failed",
        )?;
        let service_sid = sid_string(principals.service()?)?;
        let product_sddl = format!(
            "O:SYG:SYD:P(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)(A;OICI;0x{FILE_GENERIC_READ_EXECUTE:08x};;;{service_sid})"
        );
        let service_sddl = format!(
            "O:SYG:SYD:P(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)(A;OICI;0x{FILE_MODIFY:08x};;;{service_sid})"
        );
        create_or_verify_root(
            &roots.product,
            &product_sddl,
            false,
            &principals,
            &mut journal.product,
        )?;
        create_or_verify_root(
            &roots.service,
            &service_sddl,
            true,
            &principals,
            &mut journal.service,
        )
    }

    fn create_or_verify_root(
        path: &Path,
        sddl: &str,
        service_leaf: bool,
        principals: &SecurityPrincipals,
        created: &mut bool,
    ) -> Result<(), String> {
        let mut descriptor = OwnedSecurityDescriptor::from_sddl(sddl)?;
        let attributes = descriptor.attributes();
        let path_wide = wide_path(path);
        *created = if unsafe { CreateDirectoryW(path_wide.as_ptr(), &attributes) } != 0 {
            true
        } else {
            let error = unsafe { GetLastError() };
            if error != ERROR_ALREADY_EXISTS {
                return Err(format!("collector_service_root_create_failed:{error}"));
            }
            false
        };
        if let Err(error) = open_and_verify_root(path, service_leaf, principals) {
            if *created {
                if unsafe { RemoveDirectoryW(path_wide.as_ptr()) } != 0 {
                    *created = false;
                } else {
                    return Err(format!(
                        "{error};collector_service_root_create_rollback_failed:{}",
                        unsafe { GetLastError() }
                    ));
                }
            }
            return Err(error);
        }
        Ok(())
    }

    fn open_and_verify_root(
        path: &Path,
        service_leaf: bool,
        principals: &SecurityPrincipals,
    ) -> Result<OwnedHandle, String> {
        let handle = open_directory(path, "collector_service_root_open_failed")?;
        let policy = security_policy(handle.raw(), principals)?;
        validate_product_root_policy(&policy, service_leaf)?;
        Ok(handle)
    }

    fn verify_optional_leaf(
        path: &Path,
        principals: &SecurityPrincipals,
    ) -> Result<Option<OwnedHandle>, String> {
        let path_wide = wide_path(path);
        let handle = unsafe {
            CreateFileW(
                path_wide.as_ptr(),
                FILE_READ_ATTRIBUTES | READ_CONTROL,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                ptr::null(),
                OPEN_EXISTING,
                FILE_FLAG_OPEN_REPARSE_POINT,
                ptr::null_mut(),
            )
        };
        if handle.is_null() || handle == (-1_isize as HANDLE) {
            let error = unsafe { GetLastError() };
            if is_missing_path_error(error) {
                return Ok(None);
            }
            return Err(format!("collector_service_root_leaf_open_failed:{error}"));
        }
        let handle = OwnedHandle(handle);
        let info = file_information(handle.raw(), "collector_service_root_leaf_info_failed")?;
        if info.dwFileAttributes & (FILE_ATTRIBUTE_DIRECTORY | FILE_ATTRIBUTE_REPARSE_POINT) != 0 {
            return Err("collector_service_root_leaf_untrusted".to_string());
        }
        validate_no_untrusted_writer(handle.raw(), principals, true, true)?;
        Ok(Some(handle))
    }

    fn open_directory(path: &Path, context: &str) -> Result<OwnedHandle, String> {
        let path = wide_path(path);
        let handle = OwnedHandle::new(
            unsafe {
                CreateFileW(
                    path.as_ptr(),
                    FILE_READ_ATTRIBUTES | READ_CONTROL,
                    FILE_SHARE_READ | FILE_SHARE_WRITE,
                    ptr::null(),
                    OPEN_EXISTING,
                    FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT,
                    ptr::null_mut(),
                )
            },
            context,
        )?;
        let info = file_information(handle.raw(), context)?;
        if info.dwFileAttributes & FILE_ATTRIBUTE_DIRECTORY == 0
            || info.dwFileAttributes & FILE_ATTRIBUTE_REPARSE_POINT != 0
        {
            return Err("collector_service_directory_untrusted".to_string());
        }
        Ok(handle)
    }

    fn security_policy(
        handle: HANDLE,
        principals: &SecurityPrincipals,
    ) -> Result<SecurityPolicy, String> {
        security_policy_with_dacl_sha256(handle, principals).map(|(policy, _)| policy)
    }

    fn security_policy_with_dacl_sha256(
        handle: HANDLE,
        principals: &SecurityPrincipals,
    ) -> Result<(SecurityPolicy, String), String> {
        let security = OwnedSecurityInfo::read(handle, "collector_service_root_security_failed")?;
        let owner = principals.classify(security.owner);
        let mut control = 0_u16;
        let mut revision = 0_u32;
        if unsafe { GetSecurityDescriptorControl(security.descriptor, &mut control, &mut revision) }
            == 0
        {
            return Err(last_error("collector_service_root_control_failed"));
        }
        let aces = read_aces(security.dacl, principals)?;
        let dacl_sha256 = dacl_sha256(security.dacl, "collector_service_root_dacl")?;
        drop(security);
        Ok((
            SecurityPolicy {
                owner,
                dacl_protected: control & SE_DACL_PROTECTED != 0,
                reparse: false,
                aces,
            },
            dacl_sha256,
        ))
    }

    #[cfg(feature = "private-windows-lifecycle-proof")]
    fn security_policy_for_proof(policy: SecurityPolicy) -> SecurityPolicyForProof {
        SecurityPolicyForProof {
            owner: principal_for_proof(policy.owner),
            dacl_protected: policy.dacl_protected,
            reparse: policy.reparse,
            aces: policy.aces.into_iter().map(ace_policy_for_proof).collect(),
        }
    }

    #[cfg(feature = "private-windows-lifecycle-proof")]
    pub(super) fn ace_policy_for_proof(policy: AcePolicy) -> AcePolicyForProof {
        AcePolicyForProof {
            principal: principal_for_proof(policy.principal),
            allow: policy.allow,
            inherit_only: policy.inherit_only,
            object_inherit: policy.object_inherit,
            container_inherit: policy.container_inherit,
            mask: policy.mask,
        }
    }

    #[cfg(feature = "private-windows-lifecycle-proof")]
    fn principal_for_proof(principal: PrincipalClass) -> SecurityPrincipalForProof {
        match principal {
            PrincipalClass::LocalSystem => SecurityPrincipalForProof::LocalSystem,
            PrincipalClass::Administrators => SecurityPrincipalForProof::Administrators,
            PrincipalClass::TrustedInstaller => SecurityPrincipalForProof::TrustedInstaller,
            PrincipalClass::InteractiveUsers => SecurityPrincipalForProof::InteractiveUsers,
            PrincipalClass::CollectorService => SecurityPrincipalForProof::CollectorService,
            PrincipalClass::Other => SecurityPrincipalForProof::Other,
        }
    }

    fn read_aces(
        dacl: *mut windows_sys::Win32::Security::ACL,
        principals: &SecurityPrincipals,
    ) -> Result<Vec<AcePolicy>, String> {
        let mut info = ACL_SIZE_INFORMATION::default();
        if unsafe {
            GetAclInformation(
                dacl,
                (&mut info as *mut ACL_SIZE_INFORMATION).cast(),
                size_of::<ACL_SIZE_INFORMATION>() as u32,
                AclSizeInformation,
            )
        } == 0
        {
            return Err(last_error("collector_service_root_acl_info_failed"));
        }
        let mut result = Vec::with_capacity(info.AceCount as usize);
        for index in 0..info.AceCount {
            let mut raw: *mut c_void = ptr::null_mut();
            if unsafe { GetAce(dacl, index, &mut raw) } == 0 || raw.is_null() {
                return Err(last_error("collector_service_root_ace_read_failed"));
            }
            let ace = unsafe { &*(raw.cast::<ACCESS_ALLOWED_ACE>()) };
            if ace.Header.AceType != ACCESS_ALLOWED_ACE_TYPE {
                return Err("collector_service_root_ace_type_invalid".to_string());
            }
            let flags = u32::from(ace.Header.AceFlags);
            let sid = (&ace.SidStart as *const u32).cast_mut().cast();
            result.push(AcePolicy {
                principal: principals.classify(sid),
                allow: true,
                inherit_only: flags & INHERIT_ONLY_ACE != 0,
                object_inherit: flags & OBJECT_INHERIT_ACE != 0,
                container_inherit: flags & CONTAINER_INHERIT_ACE != 0,
                mask: ace.Mask,
            });
        }
        Ok(result)
    }

    pub(super) fn dacl_sha256(
        dacl: *mut windows_sys::Win32::Security::ACL,
        context: &str,
    ) -> Result<String, String> {
        if dacl.is_null() {
            return Err(format!("{context}_missing"));
        }
        let size = usize::from(unsafe { (*dacl).AclSize });
        if size < size_of::<windows_sys::Win32::Security::ACL>() {
            return Err(format!("{context}_size_invalid"));
        }
        let bytes = unsafe { std::slice::from_raw_parts(dacl.cast::<u8>(), size) };
        Ok(Sha256::digest(bytes)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect())
    }

    fn validate_no_untrusted_writer(
        handle: HANDLE,
        principals: &SecurityPrincipals,
        require_system_owner: bool,
        allow_service_writer: bool,
    ) -> Result<(), String> {
        let security = OwnedSecurityInfo::read(handle, "collector_service_path_security_failed")?;
        if require_system_owner
            && principals.classify(security.owner) != PrincipalClass::LocalSystem
        {
            return Err("collector_service_path_owner_invalid".to_string());
        }
        if !require_system_owner
            && !allow_service_writer
            && !matches!(
                principals.classify(security.owner),
                PrincipalClass::LocalSystem
                    | PrincipalClass::Administrators
                    | PrincipalClass::TrustedInstaller
            )
        {
            return Err("collector_service_path_owner_invalid".to_string());
        }
        let mut info = ACL_SIZE_INFORMATION::default();
        if unsafe {
            GetAclInformation(
                security.dacl,
                (&mut info as *mut ACL_SIZE_INFORMATION).cast(),
                size_of::<ACL_SIZE_INFORMATION>() as u32,
                AclSizeInformation,
            )
        } == 0
        {
            return Err(last_error("collector_service_path_acl_info_failed"));
        }
        for index in 0..info.AceCount {
            let mut raw: *mut c_void = ptr::null_mut();
            if unsafe { GetAce(security.dacl, index, &mut raw) } == 0 || raw.is_null() {
                return Err(last_error("collector_service_path_ace_read_failed"));
            }
            let ace = unsafe { &*(raw.cast::<ACCESS_ALLOWED_ACE>()) };
            if ace.Header.AceType == ACCESS_DENIED_ACE_TYPE {
                continue;
            }
            if ace.Header.AceType != ACCESS_ALLOWED_ACE_TYPE {
                return Err("collector_service_path_ace_type_invalid".to_string());
            }
            if u32::from(ace.Header.AceFlags) & INHERIT_ONLY_ACE != 0 {
                continue;
            }
            let sid = (&ace.SidStart as *const u32).cast_mut().cast();
            let principal = principals.classify(sid);
            let trusted_writer = matches!(
                principal,
                PrincipalClass::LocalSystem
                    | PrincipalClass::Administrators
                    | PrincipalClass::TrustedInstaller
            ) || (allow_service_writer
                && principal == PrincipalClass::CollectorService);
            if !trusted_writer && ace.Mask & UNTRUSTED_WRITE_MASK != 0 {
                return Err("collector_service_path_unprivileged_writer".to_string());
            }
        }
        Ok(())
    }

    struct VerifiedServiceImage {
        path: PathBuf,
        _program_files: OwnedHandle,
        _install_directory: OwnedHandle,
        _image: OwnedHandle,
    }

    impl VerifiedServiceImage {
        fn path(&self) -> &Path {
            &self.path
        }

        fn install_directory(&self) -> Result<&Path, String> {
            self.path
                .parent()
                .ok_or_else(|| "collector_service_install_directory_missing".to_string())
        }
    }

    fn verify_current_installer_controller() -> Result<VerifiedServiceImage, String> {
        let current = std::env::current_exe()
            .map_err(|error| format!("collector_service_executable_path_failed:{error}"))?;
        let name = current
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| "installer_shortcut_controller_name_invalid".to_string())?;
        match installer_controller_kind(name)? {
            InstallerControllerKind::Stable => verify_current_binary_path(),
            InstallerControllerKind::Staged => verify_current_staged_binary_path(),
        }
    }

    fn verify_current_binary_path() -> Result<VerifiedServiceImage, String> {
        let program_files = known_folder(CSIDL_PROGRAM_FILES)?;
        verify_current_binary_at(&expected_service_path(&program_files), &program_files)
    }

    fn verify_current_staged_binary_path() -> Result<VerifiedServiceImage, String> {
        let program_files = known_folder(CSIDL_PROGRAM_FILES)?;
        let current = std::env::current_exe()
            .map_err(|error| format!("collector_service_executable_path_failed:{error}"))?;
        let name = current
            .file_name()
            .and_then(|name| name.to_str())
            .filter(|name| is_staged_upgrade_name(name))
            .ok_or_else(|| "collector_service_staged_image_name_invalid".to_string())?;
        let expected = program_files.join(PRODUCT_DIRECTORY_NAME).join(name);
        verify_current_binary_at(&expected, &program_files)
    }

    fn verify_current_binary_at(
        expected: &Path,
        program_files: &Path,
    ) -> Result<VerifiedServiceImage, String> {
        let current = std::env::current_exe()
            .map_err(|error| format!("collector_service_executable_path_failed:{error}"))?;
        validate_current_service_path(&current, expected)?;
        verify_service_image_at(&current, program_files)
    }

    fn verify_service_image_at(
        image_path: &Path,
        program_files: &Path,
    ) -> Result<VerifiedServiceImage, String> {
        let install_dir = image_path
            .parent()
            .ok_or_else(|| "collector_service_install_directory_missing".to_string())?;
        let principals = SecurityPrincipals::load_base()?;
        let program_files_handle =
            open_directory(program_files, "collector_service_program_files_open_failed")?;
        let install = open_directory(
            install_dir,
            "collector_service_install_directory_open_failed",
        )?;
        if !fixed_path_eq(
            &final_path(
                &install,
                "collector_service_install_directory_final_path_failed",
            )?,
            install_dir,
        ) {
            return Err("collector_service_install_directory_identity_invalid".to_string());
        }
        validate_no_untrusted_writer(install.raw(), &principals, false, false)?;
        let image = open_file(image_path, "collector_service_executable_open_failed")?;
        if !fixed_path_eq(
            &final_path(&image, "collector_service_executable_final_path_failed")?,
            image_path,
        ) {
            return Err("collector_service_executable_identity_invalid".to_string());
        }
        validate_no_untrusted_writer(image.raw(), &principals, false, false)?;
        Ok(VerifiedServiceImage {
            path: image_path.to_path_buf(),
            _program_files: program_files_handle,
            _install_directory: install,
            _image: image,
        })
    }

    fn open_file(path: &Path, context: &str) -> Result<OwnedHandle, String> {
        let path = wide_path(path);
        let handle = OwnedHandle::new(
            unsafe {
                CreateFileW(
                    path.as_ptr(),
                    FILE_READ_ATTRIBUTES | READ_CONTROL,
                    FILE_SHARE_READ | FILE_SHARE_WRITE,
                    ptr::null(),
                    OPEN_EXISTING,
                    FILE_FLAG_OPEN_REPARSE_POINT,
                    ptr::null_mut(),
                )
            },
            context,
        )?;
        let info = file_information(handle.raw(), context)?;
        if info.dwFileAttributes & (FILE_ATTRIBUTE_DIRECTORY | FILE_ATTRIBUTE_REPARSE_POINT) != 0 {
            return Err("collector_service_executable_untrusted".to_string());
        }
        Ok(handle)
    }

    fn require_elevated() -> Result<(), String> {
        let mut token = ptr::null_mut();
        if unsafe {
            OpenProcessToken(
                GetCurrentProcess(),
                windows_sys::Win32::Security::TOKEN_QUERY,
                &mut token,
            )
        } == 0
        {
            return Err(last_error(
                "collector_service_provisioner_token_open_failed",
            ));
        }
        let token = OwnedHandle::new(token, "collector_service_provisioner_token_invalid")?;
        let mut elevation = TOKEN_ELEVATION::default();
        let mut returned = 0_u32;
        if unsafe {
            GetTokenInformation(
                token.raw(),
                TokenElevation,
                (&mut elevation as *mut TOKEN_ELEVATION).cast(),
                size_of::<TOKEN_ELEVATION>() as u32,
                &mut returned,
            )
        } == 0
        {
            return Err(last_error(
                "collector_service_provisioner_token_query_failed",
            ));
        }
        if elevation.TokenIsElevated == 0 {
            return Err("collector_service_provisioner_elevation_required".to_string());
        }
        Ok(())
    }

    fn known_folder(csidl: u32) -> Result<PathBuf, String> {
        let mut buffer = vec![0_u16; 32_768];
        let result = unsafe {
            SHGetFolderPathW(
                ptr::null_mut(),
                csidl as i32,
                ptr::null_mut(),
                SHGFP_TYPE_CURRENT as u32,
                buffer.as_mut_ptr(),
            )
        };
        if result < 0 {
            return Err(format!("collector_service_known_folder_failed:{result}"));
        }
        let length = buffer
            .iter()
            .position(|value| *value == 0)
            .unwrap_or(buffer.len());
        Ok(PathBuf::from(String::from_utf16_lossy(&buffer[..length])))
    }

    fn well_known_sid(kind: i32) -> Result<OwnedSid, String> {
        let mut bytes = vec![0_u8; SECURITY_MAX_SID_SIZE as usize];
        let mut size = bytes.len() as u32;
        if unsafe {
            CreateWellKnownSid(kind, ptr::null_mut(), bytes.as_mut_ptr().cast(), &mut size)
        } == 0
        {
            return Err(last_error("collector_service_well_known_sid_failed"));
        }
        bytes.truncate(size as usize);
        Ok(OwnedSid(bytes))
    }

    fn account_sid(account: &str) -> Result<OwnedSid, String> {
        let account = wide(account);
        let mut sid_bytes = 0_u32;
        let mut domain_chars = 0_u32;
        let mut use_kind: SID_NAME_USE = 0;
        unsafe {
            LookupAccountNameW(
                ptr::null(),
                account.as_ptr(),
                ptr::null_mut(),
                &mut sid_bytes,
                ptr::null_mut(),
                &mut domain_chars,
                &mut use_kind,
            )
        };
        if unsafe { GetLastError() } != ERROR_INSUFFICIENT_BUFFER {
            return Err(last_error("collector_service_account_sid_size_failed"));
        }
        let mut sid = vec![0_u8; sid_bytes as usize];
        let mut domain = vec![0_u16; domain_chars as usize];
        if unsafe {
            LookupAccountNameW(
                ptr::null(),
                account.as_ptr(),
                sid.as_mut_ptr().cast(),
                &mut sid_bytes,
                domain.as_mut_ptr(),
                &mut domain_chars,
                &mut use_kind,
            )
        } == 0
        {
            return Err(last_error("collector_service_account_sid_failed"));
        }
        sid.truncate(sid_bytes as usize);
        Ok(OwnedSid(sid))
    }

    fn sid_from_string(value: &str) -> Result<OwnedSid, String> {
        let value = wide(value);
        let mut sid = ptr::null_mut();
        if unsafe { ConvertStringSidToSidW(value.as_ptr(), &mut sid) } == 0 || sid.is_null() {
            return Err(last_error("collector_service_sid_parse_failed"));
        }
        let size = unsafe { GetLengthSid(sid) };
        if size == 0 {
            unsafe { LocalFree(sid.cast()) };
            return Err(last_error("collector_service_sid_size_failed"));
        }
        let bytes = unsafe { std::slice::from_raw_parts(sid.cast::<u8>(), size as usize) }.to_vec();
        unsafe { LocalFree(sid.cast()) };
        Ok(OwnedSid(bytes))
    }

    fn sid_string(sid: &OwnedSid) -> Result<String, String> {
        let mut value = ptr::null_mut();
        if unsafe { ConvertSidToStringSidW(sid.as_psid(), &mut value) } == 0 {
            return Err(last_error("collector_service_sid_string_failed"));
        }
        let mut length = 0_usize;
        while unsafe { *value.add(length) } != 0 {
            length += 1;
        }
        let result = String::from_utf16_lossy(unsafe { std::slice::from_raw_parts(value, length) });
        unsafe { LocalFree(value.cast()) };
        Ok(result)
    }

    fn file_information(
        handle: HANDLE,
        context: &str,
    ) -> Result<BY_HANDLE_FILE_INFORMATION, String> {
        let mut info = BY_HANDLE_FILE_INFORMATION::default();
        if unsafe { GetFileInformationByHandle(handle, &mut info) } == 0 {
            return Err(last_error(context));
        }
        Ok(info)
    }

    fn wide(value: &str) -> Vec<u16> {
        value.encode_utf16().chain(std::iter::once(0)).collect()
    }

    fn wide_path(value: &Path) -> Vec<u16> {
        value
            .as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect()
    }

    fn final_path(handle: &OwnedHandle, context: &str) -> Result<PathBuf, String> {
        let required = unsafe { GetFinalPathNameByHandleW(handle.raw(), ptr::null_mut(), 0, 0) };
        if required == 0 {
            return Err(last_error(context));
        }
        let mut buffer = vec![0_u16; required as usize + 1];
        let written = unsafe {
            GetFinalPathNameByHandleW(handle.raw(), buffer.as_mut_ptr(), buffer.len() as u32, 0)
        };
        if written == 0 || written as usize >= buffer.len() {
            return Err(last_error(context));
        }
        let path = PathBuf::from(OsString::from_wide(&buffer[..written as usize]));
        Ok(strip_verbatim_disk_prefix(path))
    }

    fn last_error(context: &str) -> String {
        format!("{context}:{}", unsafe { GetLastError() })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(feature = "private-windows-lifecycle-proof")]
    #[test]
    fn fallback_restoration_runs_only_after_a_settled_body() {
        use std::cell::Cell;

        let called = Cell::new(false);
        let unsettled_body: Result<(), ()> = Err(());
        assert!(matches!(
            restore_after_settled_body(
                &unsettled_body,
                |_| false,
                || {
                    called.set(true);
                    Ok(())
                },
            ),
            ServiceStateRestorationOutcome::BlockedUnsettled
        ));
        assert!(!called.get());

        let settled_body: Result<(), ()> = Err(());
        assert!(matches!(
            restore_after_settled_body(
                &settled_body,
                |_| true,
                || {
                    called.set(true);
                    Ok(())
                }
            ),
            ServiceStateRestorationOutcome::Restored
        ));
        assert!(called.get());

        for service_settled in [true, false] {
            let successful_body: Result<(), ()> = Ok(());
            let outcome = restore_after_settled_body(
                &successful_body,
                |_| unreachable!(),
                || {
                    Err(Box::new(ServiceStateTransitionFailure {
                        reason: "restoration_failed".to_string(),
                        service_settled,
                    }))
                },
            );
            assert!(matches!(
                outcome,
                ServiceStateRestorationOutcome::Failed(failure)
                    if failure.service_settled == service_settled
            ));
        }
    }

    fn exact_root_aces(service_mask: u32) -> [AcePolicy; 3] {
        [
            AcePolicy {
                principal: PrincipalClass::LocalSystem,
                allow: true,
                inherit_only: false,
                object_inherit: true,
                container_inherit: true,
                mask: FILE_ALL_ACCESS,
            },
            AcePolicy {
                principal: PrincipalClass::Administrators,
                allow: true,
                inherit_only: false,
                object_inherit: true,
                container_inherit: true,
                mask: FILE_ALL_ACCESS,
            },
            AcePolicy {
                principal: PrincipalClass::CollectorService,
                allow: true,
                inherit_only: false,
                object_inherit: true,
                container_inherit: true,
                mask: service_mask,
            },
        ]
    }

    #[cfg(feature = "private-windows-lifecycle-proof")]
    #[test]
    fn proof_policy_preserves_every_observed_principal_and_ace_flag() {
        let policy = AcePolicy {
            principal: PrincipalClass::Other,
            allow: false,
            inherit_only: true,
            object_inherit: false,
            container_inherit: true,
            mask: 0x1234,
        };
        assert_eq!(
            native::ace_policy_for_proof(policy),
            AcePolicyForProof {
                principal: SecurityPrincipalForProof::Other,
                allow: false,
                inherit_only: true,
                object_inherit: false,
                container_inherit: true,
                mask: 0x1234,
            }
        );

        let boundaries = InstalledBoundariesForProof {
            service_dacl_sha256: "a".repeat(64),
            service_aces: vec![native::ace_policy_for_proof(policy)],
            service_data_root_dacl_sha256: "b".repeat(64),
            service_data_root: SecurityPolicyForProof {
                owner: SecurityPrincipalForProof::LocalSystem,
                dacl_protected: true,
                reparse: false,
                aces: vec![native::ace_policy_for_proof(policy)],
            },
        };
        let encoded = serde_json::to_vec(&boundaries).expect("serialize proof boundaries");
        assert_eq!(
            serde_json::from_slice::<InstalledBoundariesForProof>(&encoded)
                .expect("deserialize proof boundaries"),
            boundaries
        );
    }

    #[test]
    fn dacl_digest_binds_the_exact_acl_byte_sequence() {
        let mut acl = windows_sys::Win32::Security::ACL {
            AclRevision: 2,
            Sbz1: 0,
            AclSize: size_of::<windows_sys::Win32::Security::ACL>() as u16,
            AceCount: 0,
            Sbz2: 0,
        };
        let bytes = unsafe {
            std::slice::from_raw_parts(
                (&raw const acl).cast::<u8>(),
                size_of::<windows_sys::Win32::Security::ACL>(),
            )
        };
        let expected = Sha256::digest(bytes)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        let original = native::dacl_sha256(&raw mut acl, "fixture").expect("hash fixture ACL");
        assert_eq!(original, expected);

        acl.AceCount = 1;
        assert_ne!(
            native::dacl_sha256(&raw mut acl, "fixture").expect("hash changed fixture ACL"),
            original
        );
    }

    #[test]
    fn cli_dispatch_accepts_only_fixed_provisioner_verbs() {
        assert_eq!(run_cli(&[]), None);
        assert_eq!(run_cli(&["--other".to_string()]), Some(2));
        assert_eq!(run_cli(&[PROVISION_SWITCH.to_string()]), Some(2));
        assert_eq!(
            run_cli(&[PROVISION_SWITCH.to_string(), "adopt".to_string()]),
            Some(2)
        );
    }

    #[test]
    fn service_executable_must_be_at_the_fixed_program_files_path() {
        let program_files = Path::new(r"C:\Program Files");
        let expected = expected_service_path(program_files);
        assert_eq!(validate_current_service_path(&expected, &expected), Ok(()));
        assert_eq!(
            expected_staged_service_path(program_files),
            PathBuf::from(
                r"C:\Program Files\BatCave Monitor\batcave-collector-service.recovery.exe"
            )
        );
        assert_eq!(
            validate_current_service_path(
                Path::new(r"C:\Users\standard\BatCave Monitor\batcave-collector-service.exe"),
                &expected,
            ),
            Err("collector_service_executable_location_invalid".to_string())
        );
    }

    #[test]
    fn final_disk_path_normalization_preserves_non_disk_namespaces() {
        assert_eq!(
            strip_verbatim_disk_prefix(PathBuf::from(r"\\?\C:\Program Files\BatCave Monitor")),
            PathBuf::from(r"C:\Program Files\BatCave Monitor")
        );
        assert_eq!(
            strip_verbatim_disk_prefix(PathBuf::from(r"C:\Program Files\BatCave Monitor")),
            PathBuf::from(r"C:\Program Files\BatCave Monitor")
        );
        assert_eq!(
            strip_verbatim_disk_prefix(PathBuf::from(r"\\?\UNC\server\share")),
            PathBuf::from(r"\\?\UNC\server\share")
        );
    }

    #[test]
    fn reparse_and_unprotected_or_untrusted_root_policies_fail_closed() {
        let aces = exact_root_aces(FILE_MODIFY);
        let valid = SecurityPolicy {
            owner: PrincipalClass::LocalSystem,
            dacl_protected: true,
            reparse: false,
            aces: aces.to_vec(),
        };
        assert_eq!(validate_product_root_policy(&valid, true), Ok(()));

        for invalid in [
            SecurityPolicy {
                reparse: true,
                ..valid.clone()
            },
            SecurityPolicy {
                dacl_protected: false,
                ..valid.clone()
            },
            SecurityPolicy {
                owner: PrincipalClass::Other,
                ..valid
            },
        ] {
            assert!(validate_product_root_policy(&invalid, true).is_err());
        }
        assert!(attributes_are_reparse(FILE_ATTRIBUTE_REPARSE_POINT));
    }

    #[test]
    fn explicit_unprivileged_writer_is_not_hidden_by_expected_aces() {
        let expected = exact_root_aces(FILE_MODIFY);
        let mut hostile = expected.to_vec();
        hostile.push(AcePolicy {
            principal: PrincipalClass::Other,
            allow: true,
            inherit_only: false,
            object_inherit: true,
            container_inherit: true,
            mask: FILE_MODIFY,
        });
        let policy = SecurityPolicy {
            owner: PrincipalClass::LocalSystem,
            dacl_protected: true,
            reparse: false,
            aces: hostile,
        };
        assert_eq!(
            validate_product_root_policy(&policy, true),
            Err("collector_service_root_dacl_invalid".to_string())
        );
    }

    #[test]
    fn public_marker_does_not_adopt_a_foreign_or_retargeted_service() {
        let expected_image =
            Path::new(r"C:\Program Files\BatCave Monitor\batcave-collector-service.exe");
        let valid = ExistingServicePolicy {
            owner_marker: Some(SERVICE_OWNER_MARKER),
            image_path: expected_image,
            account: SERVICE_ACCOUNT,
            service_type: SERVICE_TYPE_OWN_PROCESS,
        };
        assert_eq!(
            validate_existing_service_policy(&valid, expected_image),
            Ok(())
        );
        assert_eq!(
            validate_existing_service_policy(
                &ExistingServicePolicy {
                    owner_marker: None,
                    ..valid
                },
                expected_image,
            ),
            Err("collector_service_foreign_service_rejected".to_string())
        );
        assert!(validate_existing_service_policy(
            &ExistingServicePolicy {
                image_path: Path::new(r"C:\Temp\batcave-collector-service.exe"),
                ..valid
            },
            expected_image,
        )
        .is_err());
    }

    #[test]
    fn service_identity_constant_matches_the_runtime_contract() {
        assert_eq!(COLLECTOR_SERVICE_NAME, "BatCaveCollector");
        assert_eq!(
            native::configured_service_sid().expect("configured service SID"),
            "S-1-5-80-729049718-3519104438-3277487564-1168609684-1739013119"
        );
    }

    #[test]
    fn shortcut_controller_authority_accepts_only_stable_recovery_or_semver_staged_images() {
        assert_eq!(
            installer_controller_kind(SERVICE_EXECUTABLE_NAME),
            Ok(InstallerControllerKind::Stable)
        );
        for name in [
            "batcave-collector-service.recovery.exe",
            "batcave-collector-service.0.2.0.staged.exe",
            "batcave-collector-service.0.2.0-rc.2+build.7.staged.exe",
        ] {
            assert_eq!(
                installer_controller_kind(name),
                Ok(InstallerControllerKind::Staged),
                "{name}"
            );
        }
        for name in [
            "batcave-monitor.exe",
            "BATCAVE-COLLECTOR-SERVICE.EXE",
            "batcave-collector-service.staged.exe",
            "batcave-collector-service.01.2.3.staged.exe",
            r"..\batcave-collector-service.recovery.exe",
        ] {
            assert_eq!(
                installer_controller_kind(name),
                Err("installer_shortcut_controller_name_invalid".to_string()),
                "{name}"
            );
        }
    }

    #[test]
    fn absent_leaf_and_absent_parent_are_both_missing_paths() {
        assert!(is_missing_path_error(ERROR_FILE_NOT_FOUND_CODE));
        assert!(is_missing_path_error(ERROR_PATH_NOT_FOUND_CODE));
        assert!(!is_missing_path_error(5));
    }

    #[test]
    fn missing_service_cleanup_resumes_from_partial_root_removal() {
        assert_eq!(
            missing_service_cleanup(false, false),
            MissingServiceCleanup::None
        );
        assert_eq!(
            missing_service_cleanup(true, false),
            MissingServiceCleanup::ProductOnly
        );
        assert_eq!(
            missing_service_cleanup(false, true),
            MissingServiceCleanup::ServiceTree
        );
        assert_eq!(
            missing_service_cleanup(true, true),
            MissingServiceCleanup::ServiceTree
        );
    }

    #[test]
    fn uninstall_recognizes_the_scm_delete_pending_state() {
        assert!(native::service_open_is_delete_pending(
            "collector_service_open_failed:1072"
        ));
        assert!(!native::service_open_is_delete_pending(
            "collector_service_open_failed:5"
        ));
    }

    #[test]
    fn lifecycle_probe_requests_access_that_conflicts_with_the_owner() {
        assert!(native::lifecycle_probe_requests_write_access());
    }

    #[test]
    fn mutable_lease_verification_does_not_block_atomic_replacement() {
        assert!(native::mutable_lease_handle_is_released_after_verification());
    }

    #[test]
    fn stopped_service_status_requires_a_clean_scm_exit() {
        use windows_sys::Win32::System::Services::{
            SERVICE_RUNNING, SERVICE_STATUS_PROCESS, SERVICE_STOPPED,
        };

        let clean = SERVICE_STATUS_PROCESS {
            dwCurrentState: SERVICE_STOPPED,
            ..Default::default()
        };
        assert_eq!(native::validate_clean_stopped_status(&clean), Ok(()));

        let failed = SERVICE_STATUS_PROCESS {
            dwWin32ExitCode: 1_066,
            dwServiceSpecificExitCode: 1,
            ..clean
        };
        assert!(native::stopped_status_requires_repair(&failed));
        assert_eq!(
            native::validate_clean_stopped_status(&failed),
            Err("collector_service_stop_reported_failure:1066:1".to_string())
        );
        assert!(native::stopped_service_can_be_replaced(&failed));

        let stale_specific = SERVICE_STATUS_PROCESS {
            dwServiceSpecificExitCode: 9,
            ..clean
        };
        assert!(!native::stopped_status_requires_repair(&stale_specific));
        assert_eq!(
            native::validate_clean_stopped_status(&stale_specific),
            Ok(())
        );

        let running = SERVICE_STATUS_PROCESS {
            dwCurrentState: SERVICE_RUNNING,
            ..clean
        };
        assert!(!native::stopped_service_can_be_replaced(&running));
        assert!(!native::stopped_service_can_be_replaced(
            &SERVICE_STATUS_PROCESS {
                dwProcessId: 7,
                ..clean
            }
        ));
        assert_eq!(
            native::validate_clean_stopped_status(&running),
            Err("collector_service_stop_settlement_unproven".to_string())
        );
    }

    #[test]
    fn legacy_cli_allowlist_accepts_only_the_observed_product_bytes() {
        let known = LEGACY_WINDOWS_CLI_IMAGES[0];
        assert_eq!(known.size, 1_425_920);
        assert!(legacy_cli_image_matches(
            &LEGACY_WINDOWS_CLI_IMAGES,
            known.size,
            &known.sha256,
        ));

        let mut changed = known.sha256;
        changed[0] ^= 1;
        assert!(!legacy_cli_image_matches(
            &LEGACY_WINDOWS_CLI_IMAGES,
            known.size,
            &changed,
        ));
        assert!(!legacy_cli_image_matches(
            &LEGACY_WINDOWS_CLI_IMAGES,
            known.size + 1,
            &known.sha256,
        ));
    }

    #[cfg(feature = "private-windows-lifecycle-proof")]
    #[test]
    fn private_rollback_fixture_inputs_are_digest_bound_and_bounded() {
        let bytes = b"fixed rollback candidate";
        let candidate: [u8; 32] = Sha256::digest(bytes).into();
        let original = [0x5a; 32];
        assert_eq!(
            validate_failed_upgrade_fixture_inputs(bytes, candidate, original),
            Ok(())
        );
        for invalid in [
            validate_failed_upgrade_fixture_inputs(&[], candidate, original),
            validate_failed_upgrade_fixture_inputs(bytes, [0; 32], original),
            validate_failed_upgrade_fixture_inputs(bytes, candidate, [0; 32]),
            validate_failed_upgrade_fixture_inputs(bytes, candidate, candidate),
            validate_failed_upgrade_fixture_inputs(bytes, [0x7b; 32], original),
        ] {
            assert_eq!(
                invalid,
                Err("collector_service_proof_upgrade_fixture_invalid".to_string())
            );
        }
    }

    #[test]
    fn legacy_cli_cleanup_deletes_the_hashed_handle_and_retains_a_replacement() {
        let root = std::env::temp_dir().join(format!(
            "batcave-legacy-cli-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("system time")
                .as_nanos()
        ));
        std::fs::create_dir(&root).expect("legacy CLI fixture root");
        let root = strip_verbatim_disk_prefix(
            std::fs::canonicalize(&root).expect("canonical legacy CLI fixture root"),
        );
        let path = root.join(LEGACY_WINDOWS_CLI_NAME);
        let expected = b"known legacy CLI fixture";
        assert_eq!(native::retire_legacy_cli_fixture(&path, expected), Ok(()));

        std::fs::write(&path, expected).expect("known legacy CLI fixture");
        assert_eq!(native::retire_legacy_cli_fixture(&path, expected), Ok(()));
        assert!(!path.exists());

        let replacement = b"arbitrary replacement...";
        assert_eq!(replacement.len(), expected.len());
        std::fs::write(&path, replacement).expect("replacement fixture");
        assert_eq!(
            native::retire_legacy_cli_fixture(&path, expected),
            Err("collector_service_legacy_cli_residue_untrusted".to_string())
        );
        assert_eq!(
            std::fs::read(&path).expect("replacement remains"),
            replacement
        );

        std::fs::remove_file(path).expect("replacement cleanup");
        std::fs::remove_dir(root).expect("legacy CLI fixture root cleanup");
    }

    #[test]
    fn no_follow_residue_probe_reports_present_and_missing_paths() {
        let root = std::env::temp_dir().join(format!(
            "batcave-residue-probe-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("system time")
                .as_nanos()
        ));
        std::fs::create_dir(&root).expect("probe root");
        let file = root.join("residue");
        assert!(!native::path_exists_no_follow(&file).expect("missing leaf probe"));
        std::fs::write(&file, b"owned residue").expect("probe residue");
        assert!(native::path_exists_no_follow(&file).expect("present residue probe"));
        assert!(
            !native::path_exists_no_follow(&root.join("missing-parent").join("residue"))
                .expect("missing parent probe")
        );
        std::fs::remove_file(file).expect("probe residue cleanup");
        std::fs::remove_dir(root).expect("probe root cleanup");
    }

    #[test]
    fn uninstall_recognizes_only_owned_atomic_temporary_leaves() {
        assert!(native::is_owned_atomic_temp_name(
            "etw-lease.v1.json.123.456.tmp"
        ));
        assert!(native::is_owned_atomic_temp_name(
            "installer-upgrade.v1.json.123.456.tmp"
        ));
        for name in [
            "etw-lease.v1.json",
            "etw-lease.v1.json.tmp",
            "etw-lease.v1.json.123.tmp",
            "etw-lease.v1.json.123.456.tmp.extra",
            "etw-lease.v1.json.pid.456.tmp",
            "other.123.456.tmp",
        ] {
            assert!(!native::is_owned_atomic_temp_name(name), "{name}");
        }
    }

    #[test]
    fn upgrade_residue_names_are_strict_and_digest_bound() {
        assert!(is_staged_upgrade_name(
            "batcave-collector-service.0.2.0-rc.2.staged.exe"
        ));
        for name in [
            "batcave-collector-service..staged.exe",
            "batcave-collector-service.-.staged.exe",
            "batcave-collector-service.1..2.staged.exe",
            "batcave-collector-service.1-.staged.exe",
            "batcave-collector-service.0.2.0/staged.exe",
            "batcave-collector-service.0.2.0.staged.exe.extra",
            "other.0.2.0.staged.exe",
        ] {
            assert!(!is_staged_upgrade_name(name), "{name}");
        }

        let digest = [0xab; 32];
        let rollback = upgrade_backup_name(&digest);
        assert_eq!(native::rollback_digest_from_name(&rollback), Some(digest));
        assert_eq!(
            native::atomic_temp_base(&format!("{rollback}.123.456.tmp")),
            Some(rollback.as_str())
        );
        #[cfg(feature = "private-windows-lifecycle-proof")]
        {
            assert!(native::install_atomic_temp_name_for_proof(
                "batcave-collector-service.rollback.tmp"
            ));
            assert!(native::install_atomic_temp_name_for_proof(&format!(
                "{rollback}.123.456.tmp"
            )));
            assert!(native::install_atomic_temp_name_for_proof(
                "batcave-collector-service.0.2.0.staged.exe.123.456.tmp"
            ));
        }
        for name in [
            "batcave-collector-service.ab.rollback.exe",
            "batcave-collector-service.zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzz.rollback.exe",
            "batcave-collector-service.rollback.tmp",
            "batcave-collector-service.ab.rollback.exe.pid.1.tmp",
        ] {
            assert_eq!(native::rollback_digest_from_name(name), None, "{name}");
        }
        #[cfg(feature = "private-windows-lifecycle-proof")]
        for name in [
            "batcave-collector-service.rollback.tmp.extra",
            "batcave-collector-service.0.2.0.staged.exe.tmp",
            "batcave-collector-service.0.2.0.staged.exe.pid.1.tmp",
            "other.123.456.tmp",
            "batcave-rollback-fixture-ran.v1",
        ] {
            assert!(!native::install_atomic_temp_name_for_proof(name), "{name}");
        }
    }

    #[cfg(feature = "private-windows-lifecycle-proof")]
    #[test]
    fn proof_residue_rejects_hardlinks_and_oversize_matching_leaves() {
        let directory = std::env::temp_dir().join(format!(
            "batcave-proof-residue-link-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("system time")
                .as_nanos()
        ));
        std::fs::create_dir(&directory).expect("temp directory");
        let original = directory.join("installer-upgrade.v1.json");
        let alias = directory.join("installer-upgrade.v1.json.alias");
        std::fs::write(&original, b"{}").expect("fixture");
        std::fs::hard_link(&original, &alias).expect("hard link");
        assert_eq!(
            native::proof_residue_file_has_single_link_for_test(&original),
            Ok(false)
        );
        std::fs::remove_file(&alias).expect("hard-link cleanup");
        std::fs::remove_file(&original).expect("fixture cleanup");
        std::fs::remove_dir(&directory).expect("temp directory cleanup");

        assert!(native::proof_residue_size_valid(
            native::ProofResidueKind::Journal,
            16 * 1024
        ));
        assert!(!native::proof_residue_size_valid(
            native::ProofResidueKind::Journal,
            16 * 1024 + 1
        ));
        assert!(!native::proof_residue_size_valid(
            native::ProofResidueKind::Staged,
            PRIVATE_ROLLBACK_FIXTURE_MAX_BYTES as u64 + 1
        ));
        assert!(!native::proof_residue_size_valid(
            native::ProofResidueKind::RollbackExecutionMarker,
            1025
        ));
        assert!(native::proof_residue_match_count_valid(
            native::PROOF_RESIDUE_MAX_MATCHED_CHILDREN - 1
        ));
        assert!(!native::proof_residue_match_count_valid(
            native::PROOF_RESIDUE_MAX_MATCHED_CHILDREN
        ));
        let mut remaining = native::PROOF_RESIDUE_MAX_TOTAL_BYTES;
        assert!(native::reserve_proof_residue_bytes(
            &mut remaining,
            native::PROOF_RESIDUE_MAX_TOTAL_BYTES
        ));
        assert_eq!(remaining, 0);
        assert!(!native::reserve_proof_residue_bytes(&mut remaining, 1));
    }

    #[cfg(feature = "private-windows-lifecycle-proof")]
    #[test]
    fn proof_residue_case_aliases_fail_closed_instead_of_appearing_absent() {
        for name in [
            "INSTALLER-UPGRADE.V1.JSON",
            "Installer-Upgrade.v1.json.41.1.tmp",
            "ETW-LEASE.V1.JSON.41.1.TMP",
        ] {
            assert!(native::classify_service_data_residue_name_for_proof(name).is_err());
        }
        let digest = "ab".repeat(32);
        for name in [
            "BATCAVE-COLLECTOR-SERVICE.0.2.0.STAGED.EXE".to_string(),
            format!("BATCAVE-COLLECTOR-SERVICE.{digest}.ROLLBACK.EXE"),
            "BATCAVE-COLLECTOR-SERVICE.0.2.0.STAGED.EXE.41.1.TMP".to_string(),
            format!("BATCAVE-COLLECTOR-SERVICE.{digest}.ROLLBACK.EXE.41.1.TMP"),
            "BATCAVE-COLLECTOR-SERVICE.ROLLBACK.TMP".to_string(),
            "BATCAVE-ROLLBACK-FIXTURE-RAN.V1".to_string(),
        ] {
            assert!(native::classify_install_residue_name_for_proof(&name).is_err());
        }
    }

    #[test]
    fn transaction_journal_is_deleted_only_after_owned_residue() {
        use std::cell::RefCell;

        let events = RefCell::new(Vec::new());
        let result = native::retire_upgrade_artifacts(
            || {
                events.borrow_mut().push("backup");
                Ok(())
            },
            || {
                events.borrow_mut().push("staged");
                Err("staged_locked".to_string())
            },
            || {
                events.borrow_mut().push("residue");
                Ok(())
            },
            || {
                events.borrow_mut().push("journal");
                Ok(())
            },
        );
        assert_eq!(result, Err("staged_locked".to_string()));
        assert_eq!(&*events.borrow(), &["backup", "staged"]);

        events.borrow_mut().clear();
        native::retire_upgrade_artifacts(
            || {
                events.borrow_mut().push("backup");
                Ok(())
            },
            || {
                events.borrow_mut().push("staged");
                Ok(())
            },
            || {
                events.borrow_mut().push("residue");
                Ok(())
            },
            || {
                events.borrow_mut().push("journal");
                Ok(())
            },
        )
        .expect("complete retirement");
        assert_eq!(
            &*events.borrow(),
            &["backup", "staged", "residue", "journal"]
        );
    }

    #[test]
    fn failed_predelete_uninstall_restores_a_previously_active_service() {
        let mut restarted = false;
        assert_eq!(
            native::failed_service_mutation(true, "predelete_failed".to_string(), || {
                restarted = true;
                Ok(())
            }),
            Err("predelete_failed".to_string())
        );
        assert!(restarted);

        restarted = false;
        assert_eq!(
            native::failed_service_mutation(false, "dirty_stopped".to_string(), || {
                restarted = true;
                Ok(())
            }),
            Err("dirty_stopped".to_string())
        );
        assert!(!restarted);
    }
}
