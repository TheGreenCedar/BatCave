use std::{
    env, fs,
    io::{self, Write},
    path::{Path, PathBuf},
    sync::Mutex,
};

use serde::{de::DeserializeOwned, Serialize};

use crate::atomic_json::{write_bytes_atomic, AtomicWriteOperation};

const APPLICATION_DIRECTORY: &str = "BatCaveMonitor";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum StoragePlatform {
    Windows,
    Macos,
    Linux,
    Unsupported,
}

impl StoragePlatform {
    pub(crate) fn current() -> Self {
        if cfg!(windows) {
            Self::Windows
        } else if cfg!(target_os = "macos") {
            Self::Macos
        } else if cfg!(target_os = "linux") {
            Self::Linux
        } else {
            Self::Unsupported
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct CurrentUserEnvironment {
    pub local_app_data: Option<PathBuf>,
    pub xdg_data_home: Option<PathBuf>,
    pub home: Option<PathBuf>,
}

impl CurrentUserEnvironment {
    pub(crate) fn from_current_process() -> Self {
        Self {
            local_app_data: env::var_os("LOCALAPPDATA").map(PathBuf::from),
            xdg_data_home: env::var_os("XDG_DATA_HOME").map(PathBuf::from),
            home: env::var_os("HOME").map(PathBuf::from),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum StorageOwner {
    CurrentUser,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ResolvedStorageRoot {
    pub owner: StorageOwner,
    pub directory: PathBuf,
}

pub(crate) fn resolve_current_user_root(
    platform: StoragePlatform,
    environment: &CurrentUserEnvironment,
) -> Result<ResolvedStorageRoot, PersistenceFailure> {
    let root = match platform {
        StoragePlatform::Windows => environment.local_app_data.clone(),
        StoragePlatform::Macos => environment
            .home
            .as_ref()
            .filter(|home| home.is_absolute())
            .map(|home| home.join("Library").join("Application Support")),
        StoragePlatform::Linux => environment
            .xdg_data_home
            .as_ref()
            .filter(|root| root.is_absolute())
            .cloned()
            .or_else(|| {
                environment
                    .home
                    .as_ref()
                    .filter(|home| home.is_absolute())
                    .map(|home| home.join(".local").join("share"))
            }),
        StoragePlatform::Unsupported => None,
    };

    root.map(|root| ResolvedStorageRoot {
        owner: StorageOwner::CurrentUser,
        directory: root.join(APPLICATION_DIRECTORY),
    })
    .ok_or_else(|| PersistenceFailure {
        code: PersistenceFailureCode::InvalidPath,
        operation: PersistenceOperation::ResolveRoot,
        path: None,
        retryable: false,
        summary: "current-user data directory is unavailable".to_string(),
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum UserStorageComponent {
    Settings,
    WarmCache,
    Diagnostics,
}

impl UserStorageComponent {
    fn file_name(self) -> &'static str {
        match self {
            Self::Settings => "settings.json",
            Self::WarmCache => "warm-cache.json",
            Self::Diagnostics => "diagnostics.jsonl",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PersistenceOperation {
    ResolveRoot,
    Create,
    Load,
    Parse,
    Migrate,
    Serialize,
    Write,
    Sync,
    Replace,
    Rotate,
    Remove,
    Permissions,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PersistenceFailureCode {
    PermissionDenied,
    ReadOnlyFilesystem,
    StorageFull,
    CorruptData,
    InvalidPath,
    NotFound,
    SerializationFailed,
    MigrationFailed,
    IoFailure,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PersistenceFailure {
    pub code: PersistenceFailureCode,
    pub operation: PersistenceOperation,
    pub path: Option<PathBuf>,
    pub retryable: bool,
    pub summary: String,
}

impl PersistenceFailure {
    fn from_io(operation: PersistenceOperation, path: &Path, error: io::Error) -> Self {
        let code = match error.kind() {
            io::ErrorKind::PermissionDenied => PersistenceFailureCode::PermissionDenied,
            io::ErrorKind::ReadOnlyFilesystem => PersistenceFailureCode::ReadOnlyFilesystem,
            io::ErrorKind::StorageFull | io::ErrorKind::QuotaExceeded => {
                PersistenceFailureCode::StorageFull
            }
            io::ErrorKind::InvalidInput
            | io::ErrorKind::InvalidFilename
            | io::ErrorKind::NotADirectory
            | io::ErrorKind::IsADirectory => PersistenceFailureCode::InvalidPath,
            io::ErrorKind::NotFound => PersistenceFailureCode::NotFound,
            _ => PersistenceFailureCode::IoFailure,
        };
        Self {
            code,
            operation,
            path: Some(path.to_path_buf()),
            retryable: matches!(
                code,
                PersistenceFailureCode::PermissionDenied
                    | PersistenceFailureCode::ReadOnlyFilesystem
                    | PersistenceFailureCode::StorageFull
                    | PersistenceFailureCode::IoFailure
            ),
            summary: error.to_string(),
        }
    }

    fn corrupt(path: &Path, error: impl std::fmt::Display) -> Self {
        Self {
            code: PersistenceFailureCode::CorruptData,
            operation: PersistenceOperation::Parse,
            path: Some(path.to_path_buf()),
            retryable: false,
            summary: error.to_string(),
        }
    }

    fn serialization(path: &Path, error: impl std::fmt::Display) -> Self {
        Self {
            code: PersistenceFailureCode::SerializationFailed,
            operation: PersistenceOperation::Serialize,
            path: Some(path.to_path_buf()),
            retryable: false,
            summary: error.to_string(),
        }
    }

    fn migration(path: &Path, error: impl Into<String>) -> Self {
        Self {
            code: PersistenceFailureCode::MigrationFailed,
            operation: PersistenceOperation::Migrate,
            path: Some(path.to_path_buf()),
            retryable: false,
            summary: error.into(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PermissionVerification {
    VerifiedPrivate,
    PlatformManaged,
    Invalid,
}

#[derive(Debug)]
pub(crate) struct BackendError {
    operation: PersistenceOperation,
    path: PathBuf,
    error: io::Error,
}

impl BackendError {
    fn new(operation: PersistenceOperation, path: &Path, error: io::Error) -> Self {
        Self {
            operation,
            path: path.to_path_buf(),
            error,
        }
    }

    fn into_failure(self) -> PersistenceFailure {
        PersistenceFailure::from_io(self.operation, &self.path, self.error)
    }

    fn into_failure_for(self, operation: PersistenceOperation) -> PersistenceFailure {
        PersistenceFailure::from_io(operation, &self.path, self.error)
    }
}

pub(crate) trait StorageBackend: Send + Sync {
    fn ensure_private_directory(&self, path: &Path) -> Result<(), BackendError>;
    fn verify_private_directory(&self, path: &Path)
        -> Result<PermissionVerification, BackendError>;
    fn read(&self, path: &Path) -> Result<Option<Vec<u8>>, BackendError>;
    fn write_atomic(&self, path: &Path, payload: &[u8]) -> Result<(), BackendError>;
    fn file_len(&self, path: &Path) -> Result<Option<u64>, BackendError>;
    fn append_and_sync(&self, path: &Path, payload: &[u8]) -> Result<(), BackendError>;
    fn rename_if_exists(&self, from: &Path, to: &Path) -> Result<bool, BackendError>;
    fn remove_if_exists(&self, path: &Path) -> Result<bool, BackendError>;
}

#[derive(Debug, Default)]
pub(crate) struct RealStorageBackend;

impl StorageBackend for RealStorageBackend {
    fn ensure_private_directory(&self, path: &Path) -> Result<(), BackendError> {
        fs::create_dir_all(path)
            .map_err(|error| BackendError::new(PersistenceOperation::Create, path, error))?;
        #[cfg(unix)]
        fs::set_permissions(path, std::os::unix::fs::PermissionsExt::from_mode(0o700))
            .map_err(|error| BackendError::new(PersistenceOperation::Permissions, path, error))?;
        Ok(())
    }

    fn verify_private_directory(
        &self,
        path: &Path,
    ) -> Result<PermissionVerification, BackendError> {
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;

            let metadata = fs::metadata(path).map_err(|error| {
                BackendError::new(PersistenceOperation::Permissions, path, error)
            })?;
            let mode = metadata.mode() & 0o777;
            // SAFETY: `geteuid` has no arguments and does not dereference memory.
            let current_uid = unsafe { libc::geteuid() };
            Ok(if metadata.uid() == current_uid && mode & 0o077 == 0 {
                PermissionVerification::VerifiedPrivate
            } else {
                PermissionVerification::Invalid
            })
        }
        #[cfg(not(unix))]
        {
            let _ = fs::metadata(path).map_err(|error| {
                BackendError::new(PersistenceOperation::Permissions, path, error)
            })?;
            Ok(PermissionVerification::PlatformManaged)
        }
    }

    fn read(&self, path: &Path) -> Result<Option<Vec<u8>>, BackendError> {
        match fs::read(path) {
            Ok(payload) => Ok(Some(payload)),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(BackendError::new(PersistenceOperation::Load, path, error)),
        }
    }

    fn write_atomic(&self, path: &Path, payload: &[u8]) -> Result<(), BackendError> {
        write_bytes_atomic(path, payload).map_err(|error| {
            let operation = match error.operation {
                AtomicWriteOperation::CreateDirectory => PersistenceOperation::Create,
                AtomicWriteOperation::SetPermissions => PersistenceOperation::Permissions,
                AtomicWriteOperation::CreateTemporary | AtomicWriteOperation::Write => {
                    PersistenceOperation::Write
                }
                AtomicWriteOperation::SyncFile | AtomicWriteOperation::SyncDirectory => {
                    PersistenceOperation::Sync
                }
                AtomicWriteOperation::Replace | AtomicWriteOperation::Rename => {
                    PersistenceOperation::Replace
                }
            };
            BackendError::new(operation, &error.path, error.error)
        })
    }

    fn file_len(&self, path: &Path) -> Result<Option<u64>, BackendError> {
        match fs::metadata(path) {
            Ok(metadata) => Ok(Some(metadata.len())),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(BackendError::new(PersistenceOperation::Load, path, error)),
        }
    }

    fn append_and_sync(&self, path: &Path, payload: &[u8]) -> Result<(), BackendError> {
        let mut options = fs::OpenOptions::new();
        options.create(true).append(true);
        #[cfg(unix)]
        std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
        let mut file = options
            .open(path)
            .map_err(|error| BackendError::new(PersistenceOperation::Write, path, error))?;
        #[cfg(unix)]
        fs::set_permissions(path, std::os::unix::fs::PermissionsExt::from_mode(0o600))
            .map_err(|error| BackendError::new(PersistenceOperation::Permissions, path, error))?;
        file.write_all(payload)
            .map_err(|error| BackendError::new(PersistenceOperation::Write, path, error))?;
        file.sync_data()
            .map_err(|error| BackendError::new(PersistenceOperation::Sync, path, error))
    }

    fn rename_if_exists(&self, from: &Path, to: &Path) -> Result<bool, BackendError> {
        match fs::rename(from, to) {
            Ok(()) => Ok(true),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
            Err(error) => Err(BackendError::new(PersistenceOperation::Rotate, from, error)),
        }
    }

    fn remove_if_exists(&self, path: &Path) -> Result<bool, BackendError> {
        match fs::remove_file(path) {
            Ok(()) => Ok(true),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
            Err(error) => Err(BackendError::new(PersistenceOperation::Remove, path, error)),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct DiagnosticPolicy {
    pub max_file_bytes: u64,
    pub max_backups: usize,
    pub max_event_bytes: usize,
}

impl Default for DiagnosticPolicy {
    fn default() -> Self {
        Self {
            max_file_bytes: 1024 * 1024,
            max_backups: 1,
            max_event_bytes: 64 * 1024,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum DiagnosticWriteOutcome {
    Written,
    Suppressed,
    Failed(PersistenceFailure),
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct DiagnosticPersistenceStatus {
    pub active_failure: Option<PersistenceFailure>,
    pub suppressed_events: u64,
}

#[derive(Debug, Default)]
struct DiagnosticState {
    active_failure: Option<PersistenceFailure>,
    suppressed_events: u64,
}

pub(crate) enum JsonMigration<T> {
    Current(T),
    Migrated(T),
}

#[derive(Debug)]
pub(crate) struct MigrationLoad<T> {
    pub value: T,
    pub migrated: bool,
}

pub(crate) struct UserStorageCoordinator<B = RealStorageBackend> {
    root: ResolvedStorageRoot,
    backend: B,
    diagnostic_policy: DiagnosticPolicy,
    diagnostic_state: Mutex<DiagnosticState>,
}

impl UserStorageCoordinator<RealStorageBackend> {
    pub(crate) fn from_current_process() -> Result<Self, PersistenceFailure> {
        let root = resolve_current_user_root(
            StoragePlatform::current(),
            &CurrentUserEnvironment::from_current_process(),
        )?;
        Ok(Self::new(
            root,
            RealStorageBackend,
            DiagnosticPolicy::default(),
        ))
    }
}

impl<B: StorageBackend> UserStorageCoordinator<B> {
    fn new(root: ResolvedStorageRoot, backend: B, diagnostic_policy: DiagnosticPolicy) -> Self {
        Self {
            root,
            backend,
            diagnostic_policy,
            diagnostic_state: Mutex::new(DiagnosticState::default()),
        }
    }

    pub(crate) fn root(&self) -> &ResolvedStorageRoot {
        &self.root
    }

    pub(crate) fn ensure_root(&self) -> Result<PermissionVerification, PersistenceFailure> {
        self.backend
            .ensure_private_directory(&self.root.directory)
            .map_err(BackendError::into_failure)?;
        let verification = self
            .backend
            .verify_private_directory(&self.root.directory)
            .map_err(BackendError::into_failure)?;
        if verification == PermissionVerification::Invalid {
            return Err(PersistenceFailure {
                code: PersistenceFailureCode::PermissionDenied,
                operation: PersistenceOperation::Permissions,
                path: Some(self.root.directory.clone()),
                retryable: true,
                summary: "current-user data directory is not private".to_string(),
            });
        }
        Ok(verification)
    }

    pub(crate) fn component_path(&self, component: UserStorageComponent) -> PathBuf {
        self.root.directory.join(component.file_name())
    }

    pub(crate) fn load_json<T: DeserializeOwned>(
        &self,
        component: UserStorageComponent,
    ) -> Result<Option<T>, PersistenceFailure> {
        let path = self.component_path(component);
        let Some(payload) = self
            .backend
            .read(&path)
            .map_err(BackendError::into_failure)?
        else {
            return Ok(None);
        };
        serde_json::from_slice(&payload)
            .map(Some)
            .map_err(|error| PersistenceFailure::corrupt(&path, error))
    }

    pub(crate) fn write_json<T: Serialize>(
        &self,
        component: UserStorageComponent,
        value: &T,
    ) -> Result<(), PersistenceFailure> {
        self.ensure_root()?;
        let path = self.component_path(component);
        let payload = serde_json::to_vec(value)
            .map_err(|error| PersistenceFailure::serialization(&path, error))?;
        self.backend
            .write_atomic(&path, &payload)
            .map_err(BackendError::into_failure)
    }

    pub(crate) fn load_json_migrating<T: Serialize>(
        &self,
        component: UserStorageComponent,
        migrate: impl FnOnce(serde_json::Value) -> Result<JsonMigration<T>, String>,
    ) -> Result<Option<MigrationLoad<T>>, PersistenceFailure> {
        let path = self.component_path(component);
        let Some(payload) = self
            .backend
            .read(&path)
            .map_err(BackendError::into_failure)?
        else {
            return Ok(None);
        };
        let value = serde_json::from_slice(&payload)
            .map_err(|error| PersistenceFailure::corrupt(&path, error))?;
        match migrate(value).map_err(|error| PersistenceFailure::migration(&path, error))? {
            JsonMigration::Current(value) => Ok(Some(MigrationLoad {
                value,
                migrated: false,
            })),
            JsonMigration::Migrated(value) => {
                self.write_json(component, &value)?;
                Ok(Some(MigrationLoad {
                    value,
                    migrated: true,
                }))
            }
        }
    }

    pub(crate) fn record_diagnostic<T: Serialize>(&self, event: &T) -> DiagnosticWriteOutcome {
        let mut state = self
            .diagnostic_state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if state.active_failure.is_some() {
            state.suppressed_events = state.suppressed_events.saturating_add(1);
            return DiagnosticWriteOutcome::Suppressed;
        }

        let path = self.component_path(UserStorageComponent::Diagnostics);
        let result = (|| {
            self.ensure_root()?;
            let mut payload = serde_json::to_vec(event)
                .map_err(|error| PersistenceFailure::serialization(&path, error))?;
            payload.push(b'\n');
            let max_payload_bytes = self
                .diagnostic_policy
                .max_event_bytes
                .min(usize::try_from(self.diagnostic_policy.max_file_bytes).unwrap_or(usize::MAX));
            if payload.len() > max_payload_bytes {
                return Err(PersistenceFailure::serialization(
                    &path,
                    format!("diagnostic event exceeds {max_payload_bytes} bytes"),
                ));
            }
            if self
                .backend
                .file_len(&path)
                .map_err(BackendError::into_failure)?
                .is_some_and(|length| {
                    length.saturating_add(payload.len() as u64)
                        > self.diagnostic_policy.max_file_bytes
                })
            {
                self.rotate_diagnostics(&path)?;
            }
            self.backend
                .append_and_sync(&path, &payload)
                .map_err(BackendError::into_failure)
        })();

        match result {
            Ok(()) => DiagnosticWriteOutcome::Written,
            Err(failure) => {
                state.active_failure = Some(failure.clone());
                DiagnosticWriteOutcome::Failed(failure)
            }
        }
    }

    fn rotate_diagnostics(&self, path: &Path) -> Result<(), PersistenceFailure> {
        if self.diagnostic_policy.max_backups == 0 {
            self.backend
                .remove_if_exists(path)
                .map_err(|error| error.into_failure_for(PersistenceOperation::Rotate))?;
            return Ok(());
        }

        let oldest = diagnostic_backup_path(path, self.diagnostic_policy.max_backups);
        self.backend
            .remove_if_exists(&oldest)
            .map_err(|error| error.into_failure_for(PersistenceOperation::Rotate))?;
        for index in (1..self.diagnostic_policy.max_backups).rev() {
            let from = diagnostic_backup_path(path, index);
            let to = diagnostic_backup_path(path, index + 1);
            self.backend
                .rename_if_exists(&from, &to)
                .map_err(BackendError::into_failure)?;
        }
        let first = diagnostic_backup_path(path, 1);
        self.backend
            .rename_if_exists(path, &first)
            .map_err(BackendError::into_failure)?;
        Ok(())
    }

    pub(crate) fn diagnostic_status(&self) -> DiagnosticPersistenceStatus {
        let state = self
            .diagnostic_state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        DiagnosticPersistenceStatus {
            active_failure: state.active_failure.clone(),
            suppressed_events: state.suppressed_events,
        }
    }

    pub(crate) fn retry_diagnostics(&self) {
        let mut state = self
            .diagnostic_state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        state.active_failure = None;
    }
}

fn diagnostic_backup_path(path: &Path, index: usize) -> PathBuf {
    let file_name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "diagnostics.jsonl".to_string());
    path.with_file_name(format!("{file_name}.{index}"))
}

#[cfg(test)]
mod tests {
    use std::{
        collections::{HashMap, VecDeque},
        sync::{Arc, Mutex},
    };

    use serde::Serializer;

    use super::*;

    #[derive(Debug, Clone)]
    struct InjectedFault {
        operation: PersistenceOperation,
        kind: io::ErrorKind,
    }

    #[derive(Debug, Default)]
    struct FakeState {
        files: HashMap<PathBuf, Vec<u8>>,
        faults: VecDeque<InjectedFault>,
        calls: Vec<PersistenceOperation>,
        permission: Option<PermissionVerification>,
    }

    #[derive(Debug, Clone, Default)]
    struct FakeBackend {
        state: Arc<Mutex<FakeState>>,
    }

    impl FakeBackend {
        fn with_file(path: PathBuf, payload: impl Into<Vec<u8>>) -> Self {
            let backend = Self::default();
            backend
                .state
                .lock()
                .unwrap()
                .files
                .insert(path, payload.into());
            backend
        }

        fn fail_next(&self, operation: PersistenceOperation, kind: io::ErrorKind) {
            self.state
                .lock()
                .unwrap()
                .faults
                .push_back(InjectedFault { operation, kind });
        }

        fn check_fault(
            &self,
            operation: PersistenceOperation,
            path: &Path,
        ) -> Result<(), BackendError> {
            let mut state = self.state.lock().unwrap();
            state.calls.push(operation);
            if state
                .faults
                .front()
                .is_some_and(|fault| fault.operation == operation)
            {
                let fault = state.faults.pop_front().unwrap();
                return Err(BackendError::new(
                    operation,
                    path,
                    io::Error::from(fault.kind),
                ));
            }
            Ok(())
        }

        fn call_count(&self, operation: PersistenceOperation) -> usize {
            self.state
                .lock()
                .unwrap()
                .calls
                .iter()
                .filter(|candidate| **candidate == operation)
                .count()
        }

        fn file(&self, path: &Path) -> Option<Vec<u8>> {
            self.state.lock().unwrap().files.get(path).cloned()
        }
    }

    impl StorageBackend for FakeBackend {
        fn ensure_private_directory(&self, path: &Path) -> Result<(), BackendError> {
            self.check_fault(PersistenceOperation::Create, path)
        }

        fn verify_private_directory(
            &self,
            path: &Path,
        ) -> Result<PermissionVerification, BackendError> {
            self.check_fault(PersistenceOperation::Permissions, path)?;
            Ok(self
                .state
                .lock()
                .unwrap()
                .permission
                .unwrap_or(PermissionVerification::VerifiedPrivate))
        }

        fn read(&self, path: &Path) -> Result<Option<Vec<u8>>, BackendError> {
            self.check_fault(PersistenceOperation::Load, path)?;
            Ok(self.state.lock().unwrap().files.get(path).cloned())
        }

        fn write_atomic(&self, path: &Path, payload: &[u8]) -> Result<(), BackendError> {
            for operation in [
                PersistenceOperation::Write,
                PersistenceOperation::Sync,
                PersistenceOperation::Replace,
            ] {
                self.check_fault(operation, path)?;
            }
            self.state
                .lock()
                .unwrap()
                .files
                .insert(path.to_path_buf(), payload.to_vec());
            Ok(())
        }

        fn file_len(&self, path: &Path) -> Result<Option<u64>, BackendError> {
            self.check_fault(PersistenceOperation::Load, path)?;
            Ok(self
                .state
                .lock()
                .unwrap()
                .files
                .get(path)
                .map(|payload| payload.len() as u64))
        }

        fn append_and_sync(&self, path: &Path, payload: &[u8]) -> Result<(), BackendError> {
            self.check_fault(PersistenceOperation::Write, path)?;
            self.check_fault(PersistenceOperation::Sync, path)?;
            self.state
                .lock()
                .unwrap()
                .files
                .entry(path.to_path_buf())
                .or_default()
                .extend_from_slice(payload);
            Ok(())
        }

        fn rename_if_exists(&self, from: &Path, to: &Path) -> Result<bool, BackendError> {
            self.check_fault(PersistenceOperation::Rotate, from)?;
            let mut state = self.state.lock().unwrap();
            let Some(payload) = state.files.remove(from) else {
                return Ok(false);
            };
            state.files.insert(to.to_path_buf(), payload);
            Ok(true)
        }

        fn remove_if_exists(&self, path: &Path) -> Result<bool, BackendError> {
            self.check_fault(PersistenceOperation::Remove, path)?;
            Ok(self.state.lock().unwrap().files.remove(path).is_some())
        }
    }

    fn root() -> ResolvedStorageRoot {
        ResolvedStorageRoot {
            owner: StorageOwner::CurrentUser,
            directory: PathBuf::from("/tmp/batcave-persistence-tests"),
        }
    }

    fn coordinator(backend: FakeBackend) -> UserStorageCoordinator<FakeBackend> {
        UserStorageCoordinator::new(
            root(),
            backend,
            DiagnosticPolicy {
                max_file_bytes: 32,
                max_backups: 2,
                max_event_bytes: 128,
            },
        )
    }

    #[test]
    fn resolves_platform_current_user_roots_without_service_fallbacks() {
        let windows = resolve_current_user_root(
            StoragePlatform::Windows,
            &CurrentUserEnvironment {
                local_app_data: Some(PathBuf::from(r"C:\Users\albert\AppData\Local")),
                ..CurrentUserEnvironment::default()
            },
        )
        .unwrap();
        assert_eq!(windows.owner, StorageOwner::CurrentUser);
        assert_eq!(
            windows.directory,
            PathBuf::from(r"C:\Users\albert\AppData\Local").join(APPLICATION_DIRECTORY)
        );

        let macos = resolve_current_user_root(
            StoragePlatform::Macos,
            &CurrentUserEnvironment {
                home: Some(PathBuf::from("/Users/albert")),
                ..CurrentUserEnvironment::default()
            },
        )
        .unwrap();
        assert_eq!(
            macos.directory,
            PathBuf::from("/Users/albert/Library/Application Support/BatCaveMonitor")
        );

        let linux = resolve_current_user_root(
            StoragePlatform::Linux,
            &CurrentUserEnvironment {
                xdg_data_home: Some(PathBuf::from("/var/user-data")),
                home: Some(PathBuf::from("/home/albert")),
                ..CurrentUserEnvironment::default()
            },
        )
        .unwrap();
        assert_eq!(
            linux.directory,
            PathBuf::from("/var/user-data/BatCaveMonitor")
        );
    }

    #[test]
    fn relative_xdg_root_falls_back_to_absolute_home_and_missing_root_is_explicit() {
        let root = resolve_current_user_root(
            StoragePlatform::Linux,
            &CurrentUserEnvironment {
                xdg_data_home: Some(PathBuf::from("relative")),
                home: Some(PathBuf::from("/home/albert")),
                ..CurrentUserEnvironment::default()
            },
        )
        .unwrap();
        assert_eq!(
            root.directory,
            PathBuf::from("/home/albert/.local/share/BatCaveMonitor")
        );

        let failure = resolve_current_user_root(
            StoragePlatform::Unsupported,
            &CurrentUserEnvironment::default(),
        )
        .unwrap_err();
        assert_eq!(failure.operation, PersistenceOperation::ResolveRoot);
        assert_eq!(failure.code, PersistenceFailureCode::InvalidPath);
    }

    #[test]
    fn invalid_or_failed_permission_verification_is_observable() {
        let backend = FakeBackend::default();
        backend.state.lock().unwrap().permission = Some(PermissionVerification::Invalid);
        let failure = coordinator(backend).ensure_root().unwrap_err();
        assert_eq!(failure.operation, PersistenceOperation::Permissions);
        assert_eq!(failure.code, PersistenceFailureCode::PermissionDenied);

        let backend = FakeBackend::default();
        backend.fail_next(
            PersistenceOperation::Permissions,
            io::ErrorKind::PermissionDenied,
        );
        let failure = coordinator(backend).ensure_root().unwrap_err();
        assert_eq!(failure.operation, PersistenceOperation::Permissions);
        assert_eq!(failure.code, PersistenceFailureCode::PermissionDenied);
    }

    #[test]
    fn atomic_write_stage_failures_preserve_the_previous_value() {
        for (operation, kind, code) in [
            (
                PersistenceOperation::Write,
                io::ErrorKind::StorageFull,
                PersistenceFailureCode::StorageFull,
            ),
            (
                PersistenceOperation::Sync,
                io::ErrorKind::ReadOnlyFilesystem,
                PersistenceFailureCode::ReadOnlyFilesystem,
            ),
            (
                PersistenceOperation::Replace,
                io::ErrorKind::PermissionDenied,
                PersistenceFailureCode::PermissionDenied,
            ),
        ] {
            let backend = FakeBackend::default();
            let coordinator = coordinator(backend.clone());
            let path = coordinator.component_path(UserStorageComponent::Settings);
            backend
                .state
                .lock()
                .unwrap()
                .files
                .insert(path.clone(), br#"{"old":true}"#.to_vec());
            backend.fail_next(operation, kind);

            let failure = coordinator
                .write_json(
                    UserStorageComponent::Settings,
                    &serde_json::json!({"new": true}),
                )
                .unwrap_err();

            assert_eq!(failure.operation, operation);
            assert_eq!(failure.code, code);
            assert_eq!(backend.file(&path).unwrap(), br#"{"old":true}"#);
        }
    }

    #[test]
    fn corrupt_json_and_load_failures_are_distinct() {
        let path = root().directory.join("settings.json");
        let backend = FakeBackend::with_file(path, b"{not-json".to_vec());
        let failure = coordinator(backend)
            .load_json::<serde_json::Value>(UserStorageComponent::Settings)
            .unwrap_err();
        assert_eq!(failure.operation, PersistenceOperation::Parse);
        assert_eq!(failure.code, PersistenceFailureCode::CorruptData);

        let backend = FakeBackend::default();
        backend.fail_next(PersistenceOperation::Load, io::ErrorKind::PermissionDenied);
        let failure = coordinator(backend)
            .load_json::<serde_json::Value>(UserStorageComponent::Settings)
            .unwrap_err();
        assert_eq!(failure.operation, PersistenceOperation::Load);
        assert_eq!(failure.code, PersistenceFailureCode::PermissionDenied);
    }

    #[test]
    fn serialization_failure_never_reaches_the_backend() {
        struct FailsSerialization;
        impl Serialize for FailsSerialization {
            fn serialize<S>(&self, _serializer: S) -> Result<S::Ok, S::Error>
            where
                S: Serializer,
            {
                Err(serde::ser::Error::custom("forced serialization failure"))
            }
        }

        let backend = FakeBackend::default();
        let failure = coordinator(backend.clone())
            .write_json(UserStorageComponent::Settings, &FailsSerialization)
            .unwrap_err();
        assert_eq!(failure.operation, PersistenceOperation::Serialize);
        assert_eq!(failure.code, PersistenceFailureCode::SerializationFailed);
        assert_eq!(backend.call_count(PersistenceOperation::Write), 0);
    }

    #[test]
    fn migration_rewrites_only_after_successful_decode() {
        #[derive(Debug, Serialize, PartialEq, Eq)]
        struct Settings {
            theme: String,
            history_point_limit: u32,
        }

        let path = root().directory.join("settings.json");
        let backend = FakeBackend::with_file(path.clone(), br#"{"theme":"night"}"#.to_vec());
        let coordinator = coordinator(backend.clone());

        let loaded = coordinator
            .load_json_migrating(UserStorageComponent::Settings, |legacy| {
                Ok(JsonMigration::Migrated(Settings {
                    theme: legacy["theme"].as_str().unwrap().to_string(),
                    history_point_limit: 120,
                }))
            })
            .unwrap()
            .unwrap();

        assert!(loaded.migrated);
        assert_eq!(loaded.value.history_point_limit, 120);
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&backend.file(&path).unwrap()).unwrap(),
            serde_json::json!({"theme":"night","history_point_limit":120})
        );
    }

    #[test]
    fn current_schema_load_does_not_rewrite_the_file() {
        #[derive(Debug, Serialize, PartialEq, Eq)]
        struct Settings {
            version: u32,
        }

        let path = root().directory.join("settings.json");
        let backend = FakeBackend::with_file(path, br#"{"version":2}"#.to_vec());
        let coordinator = coordinator(backend.clone());

        let loaded = coordinator
            .load_json_migrating(UserStorageComponent::Settings, |current| {
                Ok(JsonMigration::Current(Settings {
                    version: current["version"].as_u64().unwrap() as u32,
                }))
            })
            .unwrap()
            .unwrap();

        assert!(!loaded.migrated);
        assert_eq!(loaded.value, Settings { version: 2 });
        assert_eq!(backend.call_count(PersistenceOperation::Write), 0);
    }

    #[test]
    fn failed_migration_or_rewrite_preserves_legacy_bytes() {
        #[derive(Debug, Serialize)]
        struct Settings {
            version: u32,
        }

        let path = root().directory.join("settings.json");
        let original = br#"{"legacy":true}"#.to_vec();
        let backend = FakeBackend::with_file(path.clone(), original.clone());
        let coordinator = coordinator(backend.clone());
        let failure = coordinator
            .load_json_migrating::<Settings>(UserStorageComponent::Settings, |_legacy| {
                Err("unsupported legacy schema".to_string())
            })
            .unwrap_err();
        assert_eq!(failure.operation, PersistenceOperation::Migrate);
        assert_eq!(failure.code, PersistenceFailureCode::MigrationFailed);
        assert_eq!(backend.file(&path).unwrap(), original);

        backend.fail_next(
            PersistenceOperation::Replace,
            io::ErrorKind::PermissionDenied,
        );
        let failure = coordinator
            .load_json_migrating(UserStorageComponent::Settings, |_legacy| {
                Ok(JsonMigration::Migrated(Settings { version: 2 }))
            })
            .unwrap_err();
        assert_eq!(failure.operation, PersistenceOperation::Replace);
        assert_eq!(backend.file(&path).unwrap(), original);
    }

    #[test]
    fn diagnostics_rotate_to_bounded_backups() {
        let backend = FakeBackend::default();
        let coordinator = coordinator(backend.clone());
        let path = coordinator.component_path(UserStorageComponent::Diagnostics);
        let first = diagnostic_backup_path(&path, 1);
        let second = diagnostic_backup_path(&path, 2);
        {
            let mut state = backend.state.lock().unwrap();
            state.files.insert(path.clone(), vec![b'a'; 30]);
            state.files.insert(first.clone(), b"previous".to_vec());
            state.files.insert(second.clone(), b"oldest".to_vec());
        }

        assert_eq!(
            coordinator.record_diagnostic(&serde_json::json!({"event":"new"})),
            DiagnosticWriteOutcome::Written
        );

        assert_eq!(backend.file(&first).unwrap(), vec![b'a'; 30]);
        assert_eq!(backend.file(&second).unwrap(), b"previous");
        assert!(backend.file(&path).unwrap().ends_with(b"\n"));
    }

    #[test]
    fn rotation_failure_is_classified_and_opens_the_suppression_breaker() {
        let backend = FakeBackend::default();
        let coordinator = coordinator(backend.clone());
        let path = coordinator.component_path(UserStorageComponent::Diagnostics);
        backend
            .state
            .lock()
            .unwrap()
            .files
            .insert(path, vec![b'a'; 30]);
        backend.fail_next(
            PersistenceOperation::Remove,
            io::ErrorKind::PermissionDenied,
        );

        let first = coordinator.record_diagnostic(&serde_json::json!({"event":"new"}));
        let DiagnosticWriteOutcome::Failed(failure) = first else {
            panic!("rotation failure should be observable");
        };
        assert_eq!(failure.operation, PersistenceOperation::Rotate);
        assert_eq!(failure.code, PersistenceFailureCode::PermissionDenied);

        assert_eq!(
            coordinator.record_diagnostic(&serde_json::json!({"event":"suppressed"})),
            DiagnosticWriteOutcome::Suppressed
        );
        assert_eq!(coordinator.diagnostic_status().suppressed_events, 1);
    }

    #[test]
    fn diagnostic_write_failure_is_not_logged_recursively() {
        let backend = FakeBackend::default();
        backend.fail_next(PersistenceOperation::Write, io::ErrorKind::StorageFull);
        let coordinator = coordinator(backend.clone());

        let first = coordinator.record_diagnostic(&serde_json::json!({"event":"first"}));
        let DiagnosticWriteOutcome::Failed(failure) = first else {
            panic!("write failure should be observable");
        };
        assert_eq!(failure.code, PersistenceFailureCode::StorageFull);
        assert_eq!(backend.call_count(PersistenceOperation::Write), 1);

        for index in 0..100 {
            assert_eq!(
                coordinator.record_diagnostic(&serde_json::json!({"event":index})),
                DiagnosticWriteOutcome::Suppressed
            );
        }
        assert_eq!(backend.call_count(PersistenceOperation::Write), 1);
        assert_eq!(coordinator.diagnostic_status().suppressed_events, 100);

        coordinator.retry_diagnostics();
        assert_eq!(
            coordinator.record_diagnostic(&serde_json::json!({"event":"retry"})),
            DiagnosticWriteOutcome::Written
        );
        assert_eq!(backend.call_count(PersistenceOperation::Write), 2);
    }

    #[test]
    fn oversized_diagnostic_is_rejected_before_file_io() {
        let backend = FakeBackend::default();
        let coordinator = coordinator(backend.clone());

        let outcome = coordinator.record_diagnostic(&serde_json::json!({
            "event": "this diagnostic payload is intentionally larger than the file budget"
        }));

        let DiagnosticWriteOutcome::Failed(failure) = outcome else {
            panic!("oversized diagnostic should fail");
        };
        assert_eq!(failure.operation, PersistenceOperation::Serialize);
        assert_eq!(failure.code, PersistenceFailureCode::SerializationFailed);
        assert_eq!(backend.call_count(PersistenceOperation::Write), 0);
    }

    #[cfg(unix)]
    #[test]
    fn real_backend_creates_private_root_and_files() {
        use std::os::unix::fs::MetadataExt;

        let directory =
            env::temp_dir().join(format!("batcave-persistence-real-{}", std::process::id()));
        let _ = fs::remove_dir_all(&directory);
        let coordinator = UserStorageCoordinator::new(
            ResolvedStorageRoot {
                owner: StorageOwner::CurrentUser,
                directory: directory.clone(),
            },
            RealStorageBackend,
            DiagnosticPolicy::default(),
        );

        assert_eq!(
            coordinator.ensure_root().unwrap(),
            PermissionVerification::VerifiedPrivate
        );
        coordinator
            .write_json(
                UserStorageComponent::Settings,
                &serde_json::json!({"ok":true}),
            )
            .unwrap();
        assert_eq!(fs::metadata(&directory).unwrap().mode() & 0o777, 0o700);
        assert_eq!(
            fs::metadata(coordinator.component_path(UserStorageComponent::Settings))
                .unwrap()
                .mode()
                & 0o777,
            0o600
        );
        fs::remove_dir_all(directory).unwrap();
    }
}
