use std::{
    fs::{self, File},
    io::Read,
    path::{Path, PathBuf},
    sync::Arc,
};

use serde::{Deserialize, Serialize};

use crate::atomic_json;

pub(crate) const ETW_LEASE_SCHEMA_VERSION: u16 = 1;
pub(crate) const ETW_LEASE_FILE_NAME: &str = "etw-lease.v1.json";
pub(crate) const ETW_OWNER_LOCK_FILE_NAME: &str = "etw-owner.v1.lock";
const ETW_LEASE_MAX_BYTES: usize = 16 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum EtwLeasePhase {
    Intent,
    Active,
    Stopping,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct EtwControllerIdentityV1 {
    pub process_id: u32,
    pub process_started_at: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct EtwSessionIdentityV1 {
    pub name: String,
    pub provider_id: [u8; 16],
    pub session_flags: u64,
    pub configuration_digest: [u8; 32],
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct EtwLeaseV1 {
    pub schema_version: u16,
    pub phase: EtwLeasePhase,
    pub install_id: [u8; 16],
    pub service_generation: [u8; 16],
    pub service_instance_id: [u8; 16],
    pub boot_identity: [u8; 16],
    pub controller: EtwControllerIdentityV1,
    pub session: EtwSessionIdentityV1,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum EtwLeasePersistenceError {
    InvalidProtectedRoot,
    AuthorityMismatch,
    SnapshotMismatch,
    ObservationChanged,
    MutationNotPermitted,
    PriorLeaseMismatch,
    ReadFailed,
    SerializeFailed,
    InvalidLease,
    PayloadTooLarge,
    AtomicReplaceFailed,
    RemoveFailed,
}

/// Capability for a service-owned storage directory whose platform verifier has
/// already proven all of the invariants required below.
///
/// The native service-storage lane is the only code allowed to call
/// `from_platform_verified`. Keeping the constructor unsafe prevents an
/// arbitrary path from silently becoming trusted before that verifier is wired.
#[derive(Debug, Clone)]
pub(crate) struct ProtectedEtwLeaseRoot {
    path: PathBuf,
    brand: Arc<()>,
}

impl ProtectedEtwLeaseRoot {
    /// # Safety
    ///
    /// The caller must have opened and verified every mutable path component,
    /// rejected links and Windows reparse points, and proven a service-owned
    /// directory whose inherited access control excludes unprivileged writers.
    /// If the lease or owner-lock leaf already exists, the caller must also
    /// prove that it is a service-owned regular file whose access control
    /// excludes unprivileged writers. The directory must already exist. Its
    /// security must remain held by the installer/service boundary for this
    /// capability's lifetime.
    pub(crate) unsafe fn from_platform_verified(
        path: PathBuf,
    ) -> Result<Self, EtwLeasePersistenceError> {
        if !path.is_absolute() || path.file_name().is_none() || !path.is_dir() {
            return Err(EtwLeasePersistenceError::InvalidProtectedRoot);
        }
        Ok(Self {
            path,
            brand: Arc::new(()),
        })
    }

    fn lease_path(&self) -> PathBuf {
        self.path.join(ETW_LEASE_FILE_NAME)
    }
}

#[derive(Debug, Clone)]
pub(crate) struct EtwLeaseStore {
    path: PathBuf,
    root_brand: Arc<()>,
}

#[derive(Debug)]
pub(crate) struct EtwOwnershipAuthority {
    root_brand: Arc<()>,
}

#[derive(Debug, Clone)]
pub(crate) struct EtwLeaseSnapshot {
    observation: EtwLeaseObservation,
    state: EtwLeaseSnapshotState,
    root_brand: Arc<()>,
}

impl EtwLeaseSnapshot {
    pub(crate) fn observation(&self) -> &EtwLeaseObservation {
        &self.observation
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum EtwLeaseSnapshotState {
    Absent,
    TrustedBytes(Vec<u8>),
    CorruptBytes(Vec<u8>),
    Untrusted,
}

impl EtwLeaseStore {
    pub(crate) fn new(root: &ProtectedEtwLeaseRoot) -> Self {
        Self {
            path: root.lease_path(),
            root_brand: Arc::clone(&root.brand),
        }
    }

    pub(crate) fn observe(
        &self,
        authority: &EtwOwnershipAuthority,
    ) -> Result<EtwLeaseSnapshot, EtwLeasePersistenceError> {
        self.authorize(authority)?;
        self.capture_snapshot()
    }

    fn capture_snapshot(&self) -> Result<EtwLeaseSnapshot, EtwLeasePersistenceError> {
        let (observation, state) = match inspect_lease_leaf(&self.path)? {
            EtwLeaseLeaf::Absent => (EtwLeaseObservation::Absent, EtwLeaseSnapshotState::Absent),
            EtwLeaseLeaf::Untrusted => (
                EtwLeaseObservation::Untrusted,
                EtwLeaseSnapshotState::Untrusted,
            ),
            EtwLeaseLeaf::Trusted => {
                let bytes = read_bounded(&self.path)?;
                if bytes.len() > ETW_LEASE_MAX_BYTES {
                    (
                        EtwLeaseObservation::Corrupt,
                        EtwLeaseSnapshotState::CorruptBytes(bytes),
                    )
                } else {
                    match serde_json::from_slice::<EtwLeaseV1>(&bytes) {
                        Ok(lease) => (
                            EtwLeaseObservation::Trusted(lease),
                            EtwLeaseSnapshotState::TrustedBytes(bytes),
                        ),
                        Err(_) => (
                            EtwLeaseObservation::Corrupt,
                            EtwLeaseSnapshotState::CorruptBytes(bytes),
                        ),
                    }
                }
            }
        };
        Ok(EtwLeaseSnapshot {
            observation,
            state,
            root_brand: Arc::clone(&self.root_brand),
        })
    }

    fn authorize(&self, authority: &EtwOwnershipAuthority) -> Result<(), EtwLeasePersistenceError> {
        if Arc::ptr_eq(&self.root_brand, &authority.root_brand) {
            Ok(())
        } else {
            Err(EtwLeasePersistenceError::AuthorityMismatch)
        }
    }

    fn verify_snapshot(&self, snapshot: &EtwLeaseSnapshot) -> Result<(), EtwLeasePersistenceError> {
        if !Arc::ptr_eq(&self.root_brand, &snapshot.root_brand) {
            return Err(EtwLeasePersistenceError::SnapshotMismatch);
        }
        let current = self.capture_snapshot()?;
        if current.state != snapshot.state {
            return Err(EtwLeasePersistenceError::ObservationChanged);
        }
        Ok(())
    }

    fn verify_replacement_prior(
        prior: &EtwLeaseObservation,
        next: &EtwLeaseV1,
    ) -> Result<(), EtwLeasePersistenceError> {
        let EtwLeaseObservation::Trusted(prior) = prior else {
            return match prior {
                EtwLeaseObservation::Absent => Ok(()),
                EtwLeaseObservation::Corrupt | EtwLeaseObservation::Untrusted => {
                    Err(EtwLeasePersistenceError::MutationNotPermitted)
                }
                EtwLeaseObservation::Trusted(_) => unreachable!(),
            };
        };
        if prior.schema_version != ETW_LEASE_SCHEMA_VERSION || !lease_is_well_formed(prior) {
            return Err(EtwLeasePersistenceError::MutationNotPermitted);
        }
        if prior.install_id != next.install_id
            || prior.service_generation != next.service_generation
            || prior.boot_identity != next.boot_identity
            || prior.session != next.session
        {
            return Err(EtwLeasePersistenceError::PriorLeaseMismatch);
        }
        Ok(())
    }

    pub(crate) fn replace(
        &self,
        authority: &EtwOwnershipAuthority,
        prior: &EtwLeaseSnapshot,
        lease: &EtwLeaseV1,
    ) -> Result<(), EtwLeasePersistenceError> {
        self.authorize(authority)?;
        if !Arc::ptr_eq(&self.root_brand, &prior.root_brand) {
            return Err(EtwLeasePersistenceError::SnapshotMismatch);
        }
        if lease.schema_version != ETW_LEASE_SCHEMA_VERSION || !lease_is_well_formed(lease) {
            return Err(EtwLeasePersistenceError::InvalidLease);
        }
        Self::verify_replacement_prior(&prior.observation, lease)?;

        let bytes =
            serde_json::to_vec(lease).map_err(|_| EtwLeasePersistenceError::SerializeFailed)?;
        if bytes.len() > ETW_LEASE_MAX_BYTES {
            return Err(EtwLeasePersistenceError::PayloadTooLarge);
        }
        self.verify_snapshot(prior)?;
        atomic_json::write_bytes_atomic(&self.path, &bytes)
            .map_err(|_| EtwLeasePersistenceError::AtomicReplaceFailed)
    }

    pub(crate) fn remove_after_proven_absence(
        &self,
        authority: &EtwOwnershipAuthority,
        prior: &EtwLeaseSnapshot,
    ) -> Result<bool, EtwLeasePersistenceError> {
        self.authorize(authority)?;
        if !Arc::ptr_eq(&self.root_brand, &prior.root_brand) {
            return Err(EtwLeasePersistenceError::SnapshotMismatch);
        }
        match prior.observation {
            EtwLeaseObservation::Corrupt | EtwLeaseObservation::Untrusted => {
                Err(EtwLeasePersistenceError::MutationNotPermitted)
            }
            EtwLeaseObservation::Absent => {
                self.verify_snapshot(prior)?;
                Ok(false)
            }
            EtwLeaseObservation::Trusted(_) => {
                self.verify_snapshot(prior)?;
                fs::remove_file(&self.path).map_err(|_| EtwLeasePersistenceError::RemoveFailed)?;
                sync_parent(&self.path).map_err(|_| EtwLeasePersistenceError::RemoveFailed)?;
                Ok(true)
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EtwLeaseLeaf {
    Absent,
    Trusted,
    Untrusted,
}

fn inspect_lease_leaf(path: &Path) -> Result<EtwLeaseLeaf, EtwLeasePersistenceError> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(EtwLeaseLeaf::Absent)
        }
        Err(_) => return Err(EtwLeasePersistenceError::ReadFailed),
    };
    if !metadata.is_file() || is_link_or_reparse(&metadata) {
        return Ok(EtwLeaseLeaf::Untrusted);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;

        if metadata.mode() & 0o077 != 0 {
            return Ok(EtwLeaseLeaf::Untrusted);
        }
    }
    Ok(EtwLeaseLeaf::Trusted)
}

fn read_bounded(path: &Path) -> Result<Vec<u8>, EtwLeasePersistenceError> {
    let file = File::open(path).map_err(|_| EtwLeasePersistenceError::ReadFailed)?;
    let mut bytes = Vec::with_capacity(ETW_LEASE_MAX_BYTES.min(4096));
    file.take((ETW_LEASE_MAX_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|_| EtwLeasePersistenceError::ReadFailed)?;
    Ok(bytes)
}

#[cfg(windows)]
fn is_link_or_reparse(metadata: &fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    use windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT;

    metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
}

#[cfg(not(windows))]
fn is_link_or_reparse(metadata: &fs::Metadata) -> bool {
    metadata.file_type().is_symlink()
}

#[cfg(unix)]
fn sync_parent(path: &Path) -> std::io::Result<()> {
    File::open(path.parent().expect("lease path has protected parent"))?.sync_all()
}

#[cfg(windows)]
fn sync_parent(_path: &Path) -> std::io::Result<()> {
    Ok(())
}

#[cfg(windows)]
pub(crate) enum WindowsEtwOwnerAcquire {
    Acquired(WindowsEtwOwnerGuard),
    Contended,
}

#[cfg(windows)]
pub(crate) struct WindowsEtwOwnerGuard {
    _file: File,
    authority: EtwOwnershipAuthority,
}

#[cfg(windows)]
impl WindowsEtwOwnerGuard {
    pub(crate) fn try_acquire(
        root: &ProtectedEtwLeaseRoot,
    ) -> Result<WindowsEtwOwnerAcquire, String> {
        use std::{fs::OpenOptions, os::windows::fs::OpenOptionsExt};
        use windows_sys::Win32::Foundation::{ERROR_LOCK_VIOLATION, ERROR_SHARING_VIOLATION};

        let path = root.path.join(ETW_OWNER_LOCK_FILE_NAME);
        match inspect_lease_leaf(&path)
            .map_err(|error| format!("etw_owner_lock_inspect_failed:{error:?}"))?
        {
            EtwLeaseLeaf::Untrusted => return Err("etw_owner_lock_untrusted".to_string()),
            EtwLeaseLeaf::Absent | EtwLeaseLeaf::Trusted => {}
        }

        let file = match OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .share_mode(0)
            .open(path)
        {
            Ok(file) => file,
            Err(error)
                if matches!(
                    error.raw_os_error(),
                    Some(code)
                        if code == ERROR_SHARING_VIOLATION as i32
                            || code == ERROR_LOCK_VIOLATION as i32
                ) =>
            {
                return Ok(WindowsEtwOwnerAcquire::Contended);
            }
            Err(error) => return Err(format!("etw_owner_lock_open_failed:{error}")),
        };

        Ok(WindowsEtwOwnerAcquire::Acquired(Self {
            _file: file,
            authority: EtwOwnershipAuthority {
                root_brand: Arc::clone(&root.brand),
            },
        }))
    }

    pub(crate) fn authority(&self) -> &EtwOwnershipAuthority {
        &self.authority
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct EtwExpectedOwnerV1 {
    pub install_id: [u8; 16],
    pub service_generation: [u8; 16],
    pub boot_identity: [u8; 16],
    pub session: EtwSessionIdentityV1,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum EtwLeaseObservation {
    Absent,
    Corrupt,
    Untrusted,
    Trusted(EtwLeaseV1),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum EtwSessionObservation {
    Absent,
    Present(EtwSessionIdentityV1),
    QueryUnavailable,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum EtwControllerObservation {
    Absent,
    Present(EtwControllerIdentityV1),
    QueryUnavailable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum EtwReclaimAttempt {
    NotAttempted,
    StopFailed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum EtwLeaseConflict {
    CorruptLease,
    UntrustedLease,
    SchemaVersionMismatch,
    MalformedLease,
    InstallIdMismatch,
    ServiceGenerationMismatch,
    BootIdentityMismatch,
    LeaseSessionMismatch,
    SessionWithoutTrustedLease,
    ObservedSessionMismatch,
    ControllerStillActive,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum EtwRecoveryHold {
    SessionQueryUnavailable,
    ControllerQueryUnavailable,
    ControllerObservationMismatch,
    StopFailed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum EtwRecoveryDecision {
    StartFresh { discard_stale_lease: bool },
    ReclaimExact { phase: EtwLeasePhase },
    Conflict(EtwLeaseConflict),
    Retain(EtwRecoveryHold),
}

pub(crate) fn decide_etw_recovery(
    expected: &EtwExpectedOwnerV1,
    lease_observation: &EtwLeaseObservation,
    session_observation: &EtwSessionObservation,
    controller_observation: &EtwControllerObservation,
    reclaim_attempt: EtwReclaimAttempt,
) -> EtwRecoveryDecision {
    let lease = match lease_observation {
        EtwLeaseObservation::Absent => {
            return match session_observation {
                EtwSessionObservation::Absent => EtwRecoveryDecision::StartFresh {
                    discard_stale_lease: false,
                },
                EtwSessionObservation::Present(_) => {
                    EtwRecoveryDecision::Conflict(EtwLeaseConflict::SessionWithoutTrustedLease)
                }
                EtwSessionObservation::QueryUnavailable => {
                    EtwRecoveryDecision::Retain(EtwRecoveryHold::SessionQueryUnavailable)
                }
            };
        }
        EtwLeaseObservation::Corrupt => {
            return EtwRecoveryDecision::Conflict(EtwLeaseConflict::CorruptLease);
        }
        EtwLeaseObservation::Untrusted => {
            return EtwRecoveryDecision::Conflict(EtwLeaseConflict::UntrustedLease);
        }
        EtwLeaseObservation::Trusted(lease) => lease,
    };

    if lease.schema_version != ETW_LEASE_SCHEMA_VERSION {
        return EtwRecoveryDecision::Conflict(EtwLeaseConflict::SchemaVersionMismatch);
    }
    if !lease_is_well_formed(lease) {
        return EtwRecoveryDecision::Conflict(EtwLeaseConflict::MalformedLease);
    }
    if lease.install_id != expected.install_id {
        return EtwRecoveryDecision::Conflict(EtwLeaseConflict::InstallIdMismatch);
    }
    if lease.service_generation != expected.service_generation {
        return EtwRecoveryDecision::Conflict(EtwLeaseConflict::ServiceGenerationMismatch);
    }
    if lease.session != expected.session {
        return EtwRecoveryDecision::Conflict(EtwLeaseConflict::LeaseSessionMismatch);
    }
    if lease.boot_identity != expected.boot_identity {
        return match session_observation {
            EtwSessionObservation::Absent => EtwRecoveryDecision::StartFresh {
                discard_stale_lease: true,
            },
            EtwSessionObservation::Present(_) => {
                EtwRecoveryDecision::Conflict(EtwLeaseConflict::BootIdentityMismatch)
            }
            EtwSessionObservation::QueryUnavailable => {
                EtwRecoveryDecision::Retain(EtwRecoveryHold::SessionQueryUnavailable)
            }
        };
    }
    if matches!(session_observation, EtwSessionObservation::QueryUnavailable) {
        return EtwRecoveryDecision::Retain(EtwRecoveryHold::SessionQueryUnavailable);
    }
    if matches!(
        session_observation,
        EtwSessionObservation::Present(session) if session != &lease.session
    ) {
        return EtwRecoveryDecision::Conflict(EtwLeaseConflict::ObservedSessionMismatch);
    }

    match controller_observation {
        EtwControllerObservation::QueryUnavailable => {
            return EtwRecoveryDecision::Retain(EtwRecoveryHold::ControllerQueryUnavailable);
        }
        EtwControllerObservation::Present(controller)
            if controller.process_id != lease.controller.process_id
                || controller.process_started_at == 0 =>
        {
            return EtwRecoveryDecision::Retain(EtwRecoveryHold::ControllerObservationMismatch);
        }
        EtwControllerObservation::Present(controller) if controller == &lease.controller => {
            return EtwRecoveryDecision::Conflict(EtwLeaseConflict::ControllerStillActive);
        }
        EtwControllerObservation::Absent | EtwControllerObservation::Present(_) => {}
    }

    if reclaim_attempt == EtwReclaimAttempt::StopFailed {
        return EtwRecoveryDecision::Retain(EtwRecoveryHold::StopFailed);
    }

    match session_observation {
        EtwSessionObservation::Absent => EtwRecoveryDecision::StartFresh {
            discard_stale_lease: true,
        },
        EtwSessionObservation::Present(session) if session == &lease.session => {
            EtwRecoveryDecision::ReclaimExact { phase: lease.phase }
        }
        EtwSessionObservation::Present(_) => unreachable!("mismatch handled above"),
        EtwSessionObservation::QueryUnavailable => unreachable!("handled above"),
    }
}

fn lease_is_well_formed(lease: &EtwLeaseV1) -> bool {
    lease.install_id != [0; 16]
        && lease.service_generation != [0; 16]
        && lease.service_instance_id != [0; 16]
        && lease.boot_identity != [0; 16]
        && lease.controller.process_id != 0
        && lease.controller.process_started_at != 0
        && !lease.session.name.is_empty()
        && lease.session.name.len() <= 128
        && !lease.session.name.contains('\0')
        && lease.session.provider_id != [0; 16]
        && lease.session.session_flags != 0
        && lease.session.configuration_digest != [0; 32]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn protected_store_round_trips_atomic_phase_replacements_and_exact_removal() {
        let (root_path, root, store) = test_store("round-trip");
        let authority = test_authority(&root);
        let absent = store.observe(&authority).expect("absence observes");
        assert_eq!(absent.observation(), &EtwLeaseObservation::Absent);
        assert_eq!(
            store.path.file_name().and_then(|name| name.to_str()),
            Some(ETW_LEASE_FILE_NAME)
        );

        let intent = lease(EtwLeasePhase::Intent);
        store
            .replace(&authority, &absent, &intent)
            .expect("intent persists");
        let intent_snapshot = store.observe(&authority).expect("intent observes");
        assert_eq!(
            intent_snapshot.observation(),
            &EtwLeaseObservation::Trusted(intent.clone())
        );

        let mut active = intent;
        active.phase = EtwLeasePhase::Active;
        store
            .replace(&authority, &intent_snapshot, &active)
            .expect("active atomically replaces intent");
        let active_snapshot = store.observe(&authority).expect("active observes");
        assert_eq!(
            active_snapshot.observation(),
            &EtwLeaseObservation::Trusted(active)
        );
        assert!(store
            .remove_after_proven_absence(&authority, &active_snapshot)
            .expect("exact lease removes"));
        let missing = store.observe(&authority).expect("absence re-observes");
        assert!(!store
            .remove_after_proven_absence(&authority, &missing)
            .expect("missing lease is already removed"));
        cleanup_store(&root_path);
    }

    #[test]
    fn invalid_replacement_never_erases_the_last_valid_lease() {
        let (root_path, root, store) = test_store("invalid-replace");
        let authority = test_authority(&root);
        let absent = store.observe(&authority).expect("absence observes");
        let valid = lease(EtwLeasePhase::Active);
        store
            .replace(&authority, &absent, &valid)
            .expect("valid lease persists");
        let valid_snapshot = store.observe(&authority).expect("valid lease observes");

        let mut invalid = valid.clone();
        invalid.controller.process_started_at = 0;
        assert_eq!(
            store.replace(&authority, &valid_snapshot, &invalid),
            Err(EtwLeasePersistenceError::InvalidLease)
        );
        assert_eq!(
            observed(&store, &authority),
            EtwLeaseObservation::Trusted(valid)
        );
        cleanup_store(&root_path);
    }

    #[test]
    fn corrupt_existing_bytes_cannot_be_replaced_or_removed() {
        let (root_path, root, store) = test_store("corrupt-mutation");
        let authority = test_authority(&root);
        write_private_fixture(&store.path, b"not-json");
        let corrupt = store.observe(&authority).expect("corruption observes");

        assert_eq!(corrupt.observation(), &EtwLeaseObservation::Corrupt);
        assert_eq!(
            store.replace(&authority, &corrupt, &lease(EtwLeasePhase::Intent)),
            Err(EtwLeasePersistenceError::MutationNotPermitted)
        );
        assert_eq!(
            store.remove_after_proven_absence(&authority, &corrupt),
            Err(EtwLeasePersistenceError::MutationNotPermitted)
        );
        assert_eq!(
            fs::read(&store.path).expect("corrupt bytes remain"),
            b"not-json"
        );
        cleanup_store(&root_path);
    }

    #[test]
    fn replacement_rejects_conflicting_install_generation_boot_and_session_identity() {
        type LeaseMutation = fn(&mut EtwLeaseV1);
        let cases: [LeaseMutation; 4] = [
            |lease| lease.install_id[0] ^= 1,
            |lease| lease.service_generation[0] ^= 1,
            |lease| lease.boot_identity[0] ^= 1,
            |lease| lease.session.configuration_digest[0] ^= 1,
        ];

        for (index, mutate) in cases.into_iter().enumerate() {
            let (root_path, root, store) = test_store(&format!("identity-mismatch-{index}"));
            let authority = test_authority(&root);
            let absent = store.observe(&authority).expect("absence observes");
            let prior = lease(EtwLeasePhase::Intent);
            store
                .replace(&authority, &absent, &prior)
                .expect("prior lease persists");
            let prior_snapshot = store.observe(&authority).expect("prior lease observes");
            let mut conflicting = prior.clone();
            mutate(&mut conflicting);

            assert_eq!(
                store.replace(&authority, &prior_snapshot, &conflicting),
                Err(EtwLeasePersistenceError::PriorLeaseMismatch)
            );
            assert_eq!(
                observed(&store, &authority),
                EtwLeaseObservation::Trusted(prior)
            );
            cleanup_store(&root_path);
        }
    }

    #[test]
    fn stale_absence_snapshot_cannot_replace_a_newer_lease() {
        let (root_path, root, store) = test_store("stale-absence");
        let authority = test_authority(&root);
        let stale_absence = store.observe(&authority).expect("absence observes");
        let external = lease(EtwLeasePhase::Active);
        write_private_fixture(
            &store.path,
            &serde_json::to_vec(&external).expect("external lease serializes"),
        );

        assert_eq!(
            store.replace(&authority, &stale_absence, &lease(EtwLeasePhase::Intent)),
            Err(EtwLeasePersistenceError::ObservationChanged)
        );
        assert_eq!(
            observed(&store, &authority),
            EtwLeaseObservation::Trusted(external)
        );
        cleanup_store(&root_path);
    }

    #[test]
    fn ownership_authority_and_snapshots_are_bound_to_one_exact_root() {
        let (path_a, root_a, store_a) = test_store("root-a");
        let (path_b, root_b, store_b) = test_store("root-b");
        let authority_a = test_authority(&root_a);
        let authority_b = test_authority(&root_b);
        let snapshot_a = store_a.observe(&authority_a).expect("root A observes");

        assert_eq!(
            store_b.observe(&authority_a).unwrap_err(),
            EtwLeasePersistenceError::AuthorityMismatch
        );
        assert_eq!(
            store_b.replace(&authority_a, &snapshot_a, &lease(EtwLeasePhase::Intent)),
            Err(EtwLeasePersistenceError::AuthorityMismatch)
        );
        assert_eq!(
            store_b.replace(&authority_b, &snapshot_a, &lease(EtwLeasePhase::Intent)),
            Err(EtwLeasePersistenceError::SnapshotMismatch)
        );
        assert_eq!(
            store_a.remove_after_proven_absence(&authority_b, &snapshot_a),
            Err(EtwLeasePersistenceError::AuthorityMismatch)
        );
        assert_eq!(
            observed(&store_a, &authority_a),
            EtwLeaseObservation::Absent
        );
        assert_eq!(
            observed(&store_b, &authority_b),
            EtwLeaseObservation::Absent
        );
        cleanup_store(&path_a);
        cleanup_store(&path_b);
    }

    #[test]
    fn malformed_and_oversized_trusted_bytes_are_corrupt_not_absent() {
        let (root_path, root, store) = test_store("corrupt");
        let authority = test_authority(&root);
        write_private_fixture(&store.path, b"not-json");
        assert_eq!(observed(&store, &authority), EtwLeaseObservation::Corrupt);

        write_private_fixture(&store.path, &vec![b'x'; ETW_LEASE_MAX_BYTES + 1]);
        assert_eq!(observed(&store, &authority), EtwLeaseObservation::Corrupt);
        cleanup_store(&root_path);
    }

    #[test]
    fn unexpected_leaf_types_are_untrusted_and_never_replaced_or_removed() {
        let (root_path, root, store) = test_store("untrusted-leaf");
        let authority = test_authority(&root);
        fs::create_dir(&store.path).expect("untrusted directory leaf fixture");
        let untrusted = store.observe(&authority).expect("untrusted leaf observes");

        assert_eq!(untrusted.observation(), &EtwLeaseObservation::Untrusted);
        assert_eq!(
            store.replace(&authority, &untrusted, &lease(EtwLeasePhase::Intent)),
            Err(EtwLeasePersistenceError::MutationNotPermitted)
        );
        assert_eq!(
            store.remove_after_proven_absence(&authority, &untrusted),
            Err(EtwLeasePersistenceError::MutationNotPermitted)
        );
        assert!(store.path.is_dir());
        cleanup_store(&root_path);
    }

    #[cfg(unix)]
    #[test]
    fn group_or_other_readable_lease_bytes_are_untrusted() {
        use std::os::unix::fs::PermissionsExt;

        let (root_path, root, store) = test_store("public-mode");
        let authority = test_authority(&root);
        fs::write(
            &store.path,
            serde_json::to_vec(&lease(EtwLeasePhase::Intent)).expect("serialize fixture"),
        )
        .expect("write public fixture");
        fs::set_permissions(&store.path, fs::Permissions::from_mode(0o644))
            .expect("set public fixture mode");

        assert_eq!(observed(&store, &authority), EtwLeaseObservation::Untrusted);
        cleanup_store(&root_path);
    }

    #[test]
    fn protected_root_capability_rejects_relative_missing_and_file_paths() {
        assert!(matches!(
            unsafe { ProtectedEtwLeaseRoot::from_platform_verified(PathBuf::from("relative")) },
            Err(EtwLeasePersistenceError::InvalidProtectedRoot)
        ));

        let missing = std::env::temp_dir().join(format!(
            "batcave-etw-lease-missing-{}-{}",
            std::process::id(),
            unique_suffix()
        ));
        assert!(matches!(
            unsafe { ProtectedEtwLeaseRoot::from_platform_verified(missing) },
            Err(EtwLeasePersistenceError::InvalidProtectedRoot)
        ));

        let file = std::env::temp_dir().join(format!(
            "batcave-etw-lease-file-{}-{}",
            std::process::id(),
            unique_suffix()
        ));
        fs::write(&file, b"not a directory").expect("file fixture");
        assert!(matches!(
            unsafe { ProtectedEtwLeaseRoot::from_platform_verified(file.clone()) },
            Err(EtwLeasePersistenceError::InvalidProtectedRoot)
        ));
        fs::remove_file(file).expect("file fixture cleanup");
    }

    #[test]
    fn ownership_names_are_versioned_and_machine_wide() {
        assert_eq!(ETW_LEASE_FILE_NAME, "etw-lease.v1.json");
        assert_eq!(ETW_OWNER_LOCK_FILE_NAME, "etw-owner.v1.lock");
    }

    #[cfg(windows)]
    #[test]
    fn protected_lock_file_allows_exactly_one_machine_wide_owner() {
        let (root_path, root) = test_root("owner-lock");
        let first = match WindowsEtwOwnerGuard::try_acquire(&root).expect("first owner acquires") {
            WindowsEtwOwnerAcquire::Acquired(guard) => guard,
            WindowsEtwOwnerAcquire::Contended => panic!("first owner unexpectedly contended"),
        };
        let store = EtwLeaseStore::new(&root);
        let absent = store
            .observe(first.authority())
            .expect("owner authority observes absence");
        store
            .replace(first.authority(), &absent, &lease(EtwLeasePhase::Intent))
            .expect("owner authority permits lease write");

        assert!(matches!(
            WindowsEtwOwnerGuard::try_acquire(&root).expect("contention is explicit"),
            WindowsEtwOwnerAcquire::Contended
        ));

        drop(first);
        let recovered =
            match WindowsEtwOwnerGuard::try_acquire(&root).expect("closed owner releases lock") {
                WindowsEtwOwnerAcquire::Acquired(guard) => guard,
                WindowsEtwOwnerAcquire::Contended => panic!("released owner remained contended"),
            };
        assert!(matches!(
            store
                .observe(recovered.authority())
                .map(|snapshot| snapshot.observation),
            Ok(EtwLeaseObservation::Trusted(_))
        ));
        drop(recovered);
        cleanup_store(&root_path);
    }

    #[test]
    fn fresh_start_requires_both_lease_and_session_to_be_absent() {
        assert_eq!(
            decide(
                EtwLeaseObservation::Absent,
                EtwSessionObservation::Absent,
                EtwControllerObservation::QueryUnavailable,
                EtwReclaimAttempt::NotAttempted,
            ),
            EtwRecoveryDecision::StartFresh {
                discard_stale_lease: false,
            }
        );
        assert_eq!(
            decide(
                EtwLeaseObservation::Absent,
                EtwSessionObservation::Present(session()),
                EtwControllerObservation::Absent,
                EtwReclaimAttempt::NotAttempted,
            ),
            EtwRecoveryDecision::Conflict(EtwLeaseConflict::SessionWithoutTrustedLease)
        );
    }

    #[test]
    fn every_crash_phase_reclaims_only_the_exact_session_for_a_dead_controller() {
        for phase in [
            EtwLeasePhase::Intent,
            EtwLeasePhase::Active,
            EtwLeasePhase::Stopping,
        ] {
            let lease = lease(phase);
            assert_eq!(
                decide(
                    EtwLeaseObservation::Trusted(lease.clone()),
                    EtwSessionObservation::Present(lease.session),
                    EtwControllerObservation::Absent,
                    EtwReclaimAttempt::NotAttempted,
                ),
                EtwRecoveryDecision::ReclaimExact { phase }
            );
        }
    }

    #[test]
    fn every_crash_phase_discards_a_stale_lease_only_after_session_absence_is_proven() {
        for phase in [
            EtwLeasePhase::Intent,
            EtwLeasePhase::Active,
            EtwLeasePhase::Stopping,
        ] {
            assert_eq!(
                decide(
                    EtwLeaseObservation::Trusted(lease(phase)),
                    EtwSessionObservation::Absent,
                    EtwControllerObservation::Absent,
                    EtwReclaimAttempt::NotAttempted,
                ),
                EtwRecoveryDecision::StartFresh {
                    discard_stale_lease: true,
                }
            );
        }
    }

    #[test]
    fn live_exact_controller_blocks_reclaim_and_replacement() {
        let lease = lease(EtwLeasePhase::Active);
        assert_eq!(
            decide(
                EtwLeaseObservation::Trusted(lease.clone()),
                EtwSessionObservation::Present(lease.session.clone()),
                EtwControllerObservation::Present(lease.controller),
                EtwReclaimAttempt::NotAttempted,
            ),
            EtwRecoveryDecision::Conflict(EtwLeaseConflict::ControllerStillActive)
        );
    }

    #[test]
    fn reused_pid_with_a_different_creation_time_proves_the_old_controller_is_dead() {
        let lease = lease(EtwLeasePhase::Active);
        let reused = EtwControllerIdentityV1 {
            process_id: lease.controller.process_id,
            process_started_at: lease.controller.process_started_at + 1,
        };
        assert_eq!(
            decide(
                EtwLeaseObservation::Trusted(lease.clone()),
                EtwSessionObservation::Present(lease.session),
                EtwControllerObservation::Present(reused),
                EtwReclaimAttempt::NotAttempted,
            ),
            EtwRecoveryDecision::ReclaimExact {
                phase: EtwLeasePhase::Active,
            }
        );
    }

    #[test]
    fn an_observation_for_another_pid_does_not_prove_the_recorded_controller_dead() {
        let lease = lease(EtwLeasePhase::Active);
        let unrelated = EtwControllerIdentityV1 {
            process_id: lease.controller.process_id + 1,
            process_started_at: lease.controller.process_started_at,
        };
        assert_controller_observation_retained(&lease, unrelated);
    }

    #[test]
    fn zero_creation_time_does_not_prove_pid_reuse() {
        let lease = lease(EtwLeasePhase::Active);
        let unknown = EtwControllerIdentityV1 {
            process_id: lease.controller.process_id,
            process_started_at: 0,
        };
        assert_controller_observation_retained(&lease, unknown);
    }

    #[test]
    fn corrupt_and_untrusted_leases_are_never_overwritten() {
        for (observation, conflict) in [
            (EtwLeaseObservation::Corrupt, EtwLeaseConflict::CorruptLease),
            (
                EtwLeaseObservation::Untrusted,
                EtwLeaseConflict::UntrustedLease,
            ),
        ] {
            for session in [
                EtwSessionObservation::Absent,
                EtwSessionObservation::QueryUnavailable,
            ] {
                assert_eq!(
                    decide(
                        observation.clone(),
                        session,
                        EtwControllerObservation::Absent,
                        EtwReclaimAttempt::NotAttempted,
                    ),
                    EtwRecoveryDecision::Conflict(conflict)
                );
            }
        }
    }

    #[test]
    fn malformed_and_schema_mismatched_leases_fail_closed() {
        let mut wrong_schema = lease(EtwLeasePhase::Intent);
        wrong_schema.schema_version += 1;
        assert_conflict(wrong_schema, EtwLeaseConflict::SchemaVersionMismatch);

        for mutate in [
            |lease: &mut EtwLeaseV1| lease.service_instance_id = [0; 16],
            |lease: &mut EtwLeaseV1| lease.controller.process_id = 0,
            |lease: &mut EtwLeaseV1| lease.session.configuration_digest = [0; 32],
        ] {
            let mut malformed = lease(EtwLeasePhase::Intent);
            mutate(&mut malformed);
            assert_conflict(malformed, EtwLeaseConflict::MalformedLease);
        }
    }

    #[test]
    fn install_generation_and_session_mismatches_never_reclaim() {
        type LeaseMutation = fn(&mut EtwLeaseV1);
        let cases: [(LeaseMutation, EtwLeaseConflict); 3] = [
            (
                |lease| lease.install_id[0] ^= 1,
                EtwLeaseConflict::InstallIdMismatch,
            ),
            (
                |lease| lease.service_generation[0] ^= 1,
                EtwLeaseConflict::ServiceGenerationMismatch,
            ),
            (
                |lease| lease.session.configuration_digest[0] ^= 1,
                EtwLeaseConflict::LeaseSessionMismatch,
            ),
        ];

        for (mutate, expected_conflict) in cases {
            let mut candidate = lease(EtwLeasePhase::Active);
            mutate(&mut candidate);
            assert_conflict(candidate, expected_conflict);
        }
    }

    #[test]
    fn prior_boot_lease_is_discarded_only_after_current_boot_session_absence_is_proven() {
        let mut prior_boot = lease(EtwLeasePhase::Active);
        prior_boot.boot_identity[0] ^= 1;
        assert_eq!(
            decide(
                EtwLeaseObservation::Trusted(prior_boot.clone()),
                EtwSessionObservation::Absent,
                EtwControllerObservation::QueryUnavailable,
                EtwReclaimAttempt::NotAttempted,
            ),
            EtwRecoveryDecision::StartFresh {
                discard_stale_lease: true,
            }
        );
        assert_eq!(
            decide(
                EtwLeaseObservation::Trusted(prior_boot.clone()),
                EtwSessionObservation::Present(prior_boot.session.clone()),
                EtwControllerObservation::Absent,
                EtwReclaimAttempt::NotAttempted,
            ),
            EtwRecoveryDecision::Conflict(EtwLeaseConflict::BootIdentityMismatch)
        );
        assert_eq!(
            decide(
                EtwLeaseObservation::Trusted(prior_boot),
                EtwSessionObservation::QueryUnavailable,
                EtwControllerObservation::Absent,
                EtwReclaimAttempt::NotAttempted,
            ),
            EtwRecoveryDecision::Retain(EtwRecoveryHold::SessionQueryUnavailable)
        );

        let mut mismatched = lease(EtwLeasePhase::Active);
        mismatched.boot_identity[0] ^= 1;
        mismatched.session.session_flags ^= 1;
        assert_eq!(
            decide(
                EtwLeaseObservation::Trusted(mismatched),
                EtwSessionObservation::Absent,
                EtwControllerObservation::Absent,
                EtwReclaimAttempt::NotAttempted,
            ),
            EtwRecoveryDecision::Conflict(EtwLeaseConflict::LeaseSessionMismatch)
        );
    }

    #[test]
    fn exact_name_prefix_or_provider_alone_never_authorizes_reclaim() {
        let lease = lease(EtwLeasePhase::Active);
        let mut prefix_only = lease.session.clone();
        prefix_only.name.push_str("-foreign");
        assert_eq!(
            decide(
                EtwLeaseObservation::Trusted(lease.clone()),
                EtwSessionObservation::Present(prefix_only),
                EtwControllerObservation::Absent,
                EtwReclaimAttempt::NotAttempted,
            ),
            EtwRecoveryDecision::Conflict(EtwLeaseConflict::ObservedSessionMismatch)
        );

        let mut provider_only = lease.session.clone();
        provider_only.configuration_digest[0] ^= 1;
        assert_eq!(
            decide(
                EtwLeaseObservation::Trusted(lease),
                EtwSessionObservation::Present(provider_only),
                EtwControllerObservation::Absent,
                EtwReclaimAttempt::NotAttempted,
            ),
            EtwRecoveryDecision::Conflict(EtwLeaseConflict::ObservedSessionMismatch)
        );
    }

    #[test]
    fn known_session_mismatch_outranks_incomplete_controller_or_stop_state() {
        let lease = lease(EtwLeasePhase::Stopping);
        let mut mismatched = lease.session.clone();
        mismatched.session_flags ^= 1;

        for (controller, reclaim_attempt) in [
            (
                EtwControllerObservation::QueryUnavailable,
                EtwReclaimAttempt::NotAttempted,
            ),
            (
                EtwControllerObservation::Absent,
                EtwReclaimAttempt::StopFailed,
            ),
        ] {
            assert_eq!(
                decide(
                    EtwLeaseObservation::Trusted(lease.clone()),
                    EtwSessionObservation::Present(mismatched.clone()),
                    controller,
                    reclaim_attempt,
                ),
                EtwRecoveryDecision::Conflict(EtwLeaseConflict::ObservedSessionMismatch)
            );
        }
    }

    #[test]
    fn unavailable_session_or_controller_truth_retains_ownership_state() {
        let lease = lease(EtwLeasePhase::Stopping);
        assert_eq!(
            decide(
                EtwLeaseObservation::Trusted(lease.clone()),
                EtwSessionObservation::QueryUnavailable,
                EtwControllerObservation::Absent,
                EtwReclaimAttempt::NotAttempted,
            ),
            EtwRecoveryDecision::Retain(EtwRecoveryHold::SessionQueryUnavailable)
        );
        assert_eq!(
            decide(
                EtwLeaseObservation::Trusted(lease.clone()),
                EtwSessionObservation::Present(lease.session),
                EtwControllerObservation::QueryUnavailable,
                EtwReclaimAttempt::NotAttempted,
            ),
            EtwRecoveryDecision::Retain(EtwRecoveryHold::ControllerQueryUnavailable)
        );
    }

    #[test]
    fn stop_failure_preserves_the_lease_and_blocks_replacement() {
        let lease = lease(EtwLeasePhase::Stopping);
        for session_observation in [
            EtwSessionObservation::Absent,
            EtwSessionObservation::Present(lease.session.clone()),
        ] {
            assert_eq!(
                decide(
                    EtwLeaseObservation::Trusted(lease.clone()),
                    session_observation,
                    EtwControllerObservation::Absent,
                    EtwReclaimAttempt::StopFailed,
                ),
                EtwRecoveryDecision::Retain(EtwRecoveryHold::StopFailed)
            );
        }
    }

    #[test]
    fn lease_json_is_versioned_closed_and_uses_named_phases() {
        let lease = lease(EtwLeasePhase::Stopping);
        let json = serde_json::to_string(&lease).expect("serialize lease");
        assert!(json.contains("\"schema_version\":1"));
        assert!(json.contains("\"phase\":\"stopping\""));

        let mut value = serde_json::to_value(&lease).expect("lease value");
        value
            .as_object_mut()
            .expect("object")
            .insert("unexpected".to_string(), serde_json::Value::Bool(true));
        assert!(serde_json::from_value::<EtwLeaseV1>(value).is_err());
    }

    fn decide(
        lease: EtwLeaseObservation,
        session: EtwSessionObservation,
        controller: EtwControllerObservation,
        reclaim_attempt: EtwReclaimAttempt,
    ) -> EtwRecoveryDecision {
        decide_etw_recovery(&expected(), &lease, &session, &controller, reclaim_attempt)
    }

    fn assert_conflict(lease: EtwLeaseV1, expected_conflict: EtwLeaseConflict) {
        let session = lease.session.clone();
        assert_eq!(
            decide(
                EtwLeaseObservation::Trusted(lease),
                EtwSessionObservation::Present(session),
                EtwControllerObservation::Absent,
                EtwReclaimAttempt::NotAttempted,
            ),
            EtwRecoveryDecision::Conflict(expected_conflict)
        );
    }

    fn assert_controller_observation_retained(
        lease: &EtwLeaseV1,
        controller: EtwControllerIdentityV1,
    ) {
        for session in [
            EtwSessionObservation::Present(lease.session.clone()),
            EtwSessionObservation::Absent,
        ] {
            assert_eq!(
                decide(
                    EtwLeaseObservation::Trusted(lease.clone()),
                    session,
                    EtwControllerObservation::Present(controller.clone()),
                    EtwReclaimAttempt::NotAttempted,
                ),
                EtwRecoveryDecision::Retain(EtwRecoveryHold::ControllerObservationMismatch)
            );
        }
    }

    fn expected() -> EtwExpectedOwnerV1 {
        EtwExpectedOwnerV1 {
            install_id: [1; 16],
            service_generation: [2; 16],
            boot_identity: [4; 16],
            session: session(),
        }
    }

    fn lease(phase: EtwLeasePhase) -> EtwLeaseV1 {
        let expected = expected();
        EtwLeaseV1 {
            schema_version: ETW_LEASE_SCHEMA_VERSION,
            phase,
            install_id: expected.install_id,
            service_generation: expected.service_generation,
            service_instance_id: [3; 16],
            boot_identity: expected.boot_identity,
            controller: EtwControllerIdentityV1 {
                process_id: 41,
                process_started_at: 99,
            },
            session: expected.session,
        }
    }

    fn session() -> EtwSessionIdentityV1 {
        EtwSessionIdentityV1 {
            name: "BatCave Process Network v1".to_string(),
            provider_id: [5; 16],
            session_flags: 0x1000_0000,
            configuration_digest: [6; 32],
        }
    }

    fn test_store(name: &str) -> (PathBuf, ProtectedEtwLeaseRoot, EtwLeaseStore) {
        let (path, root) = test_root(name);
        let store = EtwLeaseStore::new(&root);
        (path, root, store)
    }

    fn test_root(name: &str) -> (PathBuf, ProtectedEtwLeaseRoot) {
        let path = std::env::temp_dir().join(format!(
            "batcave-etw-lease-{name}-{}-{}",
            std::process::id(),
            unique_suffix()
        ));
        fs::create_dir(&path).expect("protected root fixture");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;

            fs::set_permissions(&path, fs::Permissions::from_mode(0o700))
                .expect("protect root fixture");
        }
        // SAFETY: this process created the unique test directory, rejects public
        // access on Unix, and retains exclusive ownership until cleanup.
        let root = unsafe { ProtectedEtwLeaseRoot::from_platform_verified(path.clone()) }
            .expect("test root accepted");
        (path, root)
    }

    fn test_authority(root: &ProtectedEtwLeaseRoot) -> EtwOwnershipAuthority {
        EtwOwnershipAuthority {
            root_brand: Arc::clone(&root.brand),
        }
    }

    fn observed(store: &EtwLeaseStore, authority: &EtwOwnershipAuthority) -> EtwLeaseObservation {
        store
            .observe(authority)
            .expect("lease observation succeeds")
            .observation
    }

    fn write_private_fixture(path: &Path, bytes: &[u8]) {
        fs::write(path, bytes).expect("fixture writes");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;

            fs::set_permissions(path, fs::Permissions::from_mode(0o600))
                .expect("fixture private mode");
        }
    }

    fn cleanup_store(path: &Path) {
        fs::remove_dir_all(path).expect("store fixture cleanup");
    }

    fn unique_suffix() -> u128 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system clock after epoch")
            .as_nanos()
    }
}
