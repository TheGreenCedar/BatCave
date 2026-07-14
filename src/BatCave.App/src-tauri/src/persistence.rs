//! Current-user storage primitives for issue #73.
//!
//! The runtime-store integration lands after the lifecycle work in #68. Windows access remains
//! fail-closed until the later service/ACL slice can verify the current-user SID, DACL, and reparse
//! boundary instead of inferring safety from a profile path.

use std::{
    collections::{HashMap, HashSet},
    env, fs,
    io::{self, Read, Write},
    path::{Path, PathBuf},
    sync::Mutex,
};

use serde::{de::DeserializeOwned, Serialize};

use crate::{
    atomic_json::{write_bytes_atomic, AtomicWriteEffect, AtomicWriteOperation},
    contracts::{
        RuntimePersistence, RuntimePersistenceComponent, RuntimePersistenceDurability,
        RuntimePersistenceFailure, RuntimePersistenceKind, RuntimePersistenceOperation,
        RuntimePersistenceOwner, RuntimePersistencePermissionState, RuntimePersistenceRoot,
        RuntimePersistenceState,
    },
};

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
        StoragePlatform::Windows => environment
            .local_app_data
            .as_ref()
            .filter(|root| is_absolute_windows_local_path(root))
            .cloned(),
        StoragePlatform::Macos => environment
            .home
            .as_ref()
            .filter(|home| is_safe_absolute_posix_path(home))
            .map(|home| home.join("Library").join("Application Support")),
        StoragePlatform::Linux => environment
            .xdg_data_home
            .as_ref()
            .filter(|root| is_safe_absolute_posix_path(root))
            .cloned()
            .or_else(|| {
                environment
                    .home
                    .as_ref()
                    .filter(|home| is_safe_absolute_posix_path(home))
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
        write_effect: PersistenceWriteEffect::NotCommitted,
        summary: "current-user data directory is unavailable".to_string(),
    })
}

fn is_safe_absolute_posix_path(path: &Path) -> bool {
    let value = path.as_os_str().to_string_lossy();
    value.starts_with('/')
        && !value
            .split('/')
            .any(|component| matches!(component, "." | ".."))
}

fn is_absolute_windows_local_path(path: &Path) -> bool {
    let value = path.as_os_str().to_string_lossy();
    let bytes = value.as_bytes();
    bytes.len() >= 3
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && matches!(bytes[2], b'\\' | b'/')
        && !value
            .split(['\\', '/'])
            .any(|component| matches!(component, "." | ".."))
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

    fn runtime_kind(self) -> RuntimePersistenceKind {
        match self {
            Self::Settings => RuntimePersistenceKind::Settings,
            Self::WarmCache => RuntimePersistenceKind::WarmCache,
            Self::Diagnostics => RuntimePersistenceKind::Diagnostics,
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
    VerificationUnavailable,
    IoFailure,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PersistenceWriteEffect {
    NotCommitted,
    CommittedDurabilityUncertain,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PersistenceFailure {
    pub code: PersistenceFailureCode,
    pub operation: PersistenceOperation,
    pub path: Option<PathBuf>,
    pub retryable: bool,
    pub write_effect: PersistenceWriteEffect,
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
            write_effect: PersistenceWriteEffect::NotCommitted,
            summary: error.to_string(),
        }
    }

    fn corrupt(path: &Path, error: impl std::fmt::Display) -> Self {
        Self {
            code: PersistenceFailureCode::CorruptData,
            operation: PersistenceOperation::Parse,
            path: Some(path.to_path_buf()),
            retryable: false,
            write_effect: PersistenceWriteEffect::NotCommitted,
            summary: error.to_string(),
        }
    }

    fn serialization(path: &Path, error: impl std::fmt::Display) -> Self {
        Self {
            code: PersistenceFailureCode::SerializationFailed,
            operation: PersistenceOperation::Serialize,
            path: Some(path.to_path_buf()),
            retryable: false,
            write_effect: PersistenceWriteEffect::NotCommitted,
            summary: error.to_string(),
        }
    }

    fn migration(path: &Path, error: impl Into<String>) -> Self {
        Self {
            code: PersistenceFailureCode::MigrationFailed,
            operation: PersistenceOperation::Migrate,
            path: Some(path.to_path_buf()),
            retryable: false,
            write_effect: PersistenceWriteEffect::NotCommitted,
            summary: error.into(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PermissionVerification {
    #[cfg_attr(
        windows,
        allow(
            dead_code,
            reason = "Unix ownership and mode checks can prove a private root"
        )
    )]
    VerifiedPrivate,
    #[cfg_attr(
        not(windows),
        allow(
            dead_code,
            reason = "Windows remains fail-closed until #69 can verify its DACL"
        )
    )]
    Unverified,
    #[cfg_attr(
        windows,
        allow(
            dead_code,
            reason = "Unix ownership and mode checks can reject an unsafe root"
        )
    )]
    Invalid,
}

#[derive(Debug)]
struct BackendError {
    operation: PersistenceOperation,
    write_effect: PersistenceWriteEffect,
    path: PathBuf,
    error: io::Error,
}

impl BackendError {
    fn new(operation: PersistenceOperation, path: &Path, error: io::Error) -> Self {
        Self {
            operation,
            write_effect: PersistenceWriteEffect::NotCommitted,
            path: path.to_path_buf(),
            error,
        }
    }

    fn committed_durability_uncertain(
        operation: PersistenceOperation,
        path: &Path,
        error: io::Error,
    ) -> Self {
        Self {
            operation,
            write_effect: PersistenceWriteEffect::CommittedDurabilityUncertain,
            path: path.to_path_buf(),
            error,
        }
    }

    fn into_failure(self) -> PersistenceFailure {
        let mut failure = PersistenceFailure::from_io(self.operation, &self.path, self.error);
        failure.write_effect = self.write_effect;
        failure
    }
}

trait StorageBackend: Send + Sync {
    fn prepare_private_directory(
        &self,
        path: &Path,
    ) -> Result<PermissionVerification, BackendError>;
    fn verify_component_file(&self, path: &Path) -> Result<(), BackendError>;
    fn read(&self, path: &Path) -> Result<Option<Vec<u8>>, BackendError>;
    fn write_atomic(&self, path: &Path, payload: &[u8]) -> Result<(), BackendError>;
    fn file_len(&self, path: &Path) -> Result<Option<u64>, BackendError>;
    fn append_and_sync(&self, path: &Path, payload: &[u8]) -> Result<(), BackendError>;
    fn rotate_if_exists(&self, from: &Path, to: &Path) -> Result<bool, BackendError>;
    fn remove_if_exists(&self, path: &Path) -> Result<bool, BackendError>;
}

#[derive(Debug, Default)]
struct RealStorageBackend;

impl StorageBackend for RealStorageBackend {
    fn prepare_private_directory(
        &self,
        path: &Path,
    ) -> Result<PermissionVerification, BackendError> {
        #[cfg(windows)]
        {
            let _ = path;
            Ok(PermissionVerification::Unverified)
        }

        #[cfg(unix)]
        {
            match fs::symlink_metadata(path) {
                Ok(metadata) => verify_unix_root_metadata(path, &metadata)?,
                Err(error) if error.kind() == io::ErrorKind::NotFound => {
                    let mut builder = fs::DirBuilder::new();
                    builder.recursive(true);
                    std::os::unix::fs::DirBuilderExt::mode(&mut builder, 0o700);
                    builder.create(path).map_err(|error| {
                        BackendError::new(PersistenceOperation::Create, path, error)
                    })?;
                    let metadata = fs::symlink_metadata(path).map_err(|error| {
                        BackendError::new(PersistenceOperation::Permissions, path, error)
                    })?;
                    verify_unix_root_metadata(path, &metadata)?;
                }
                Err(error) => {
                    return Err(BackendError::new(
                        PersistenceOperation::Permissions,
                        path,
                        error,
                    ));
                }
            }
            fs::set_permissions(path, std::os::unix::fs::PermissionsExt::from_mode(0o700))
                .map_err(|error| {
                    BackendError::new(PersistenceOperation::Permissions, path, error)
                })?;
            verify_unix_private_directory(path)
        }

        #[cfg(not(any(unix, windows)))]
        {
            let _ = path;
            Ok(PermissionVerification::Unverified)
        }
    }

    fn verify_component_file(&self, path: &Path) -> Result<(), BackendError> {
        verify_regular_file_or_missing(path)
    }

    fn read(&self, path: &Path) -> Result<Option<Vec<u8>>, BackendError> {
        self.verify_component_file(path)?;
        let mut options = fs::OpenOptions::new();
        options.read(true);
        #[cfg(unix)]
        std::os::unix::fs::OpenOptionsExt::custom_flags(&mut options, libc::O_NOFOLLOW);
        match options.open(path) {
            Ok(mut file) => {
                let mut payload = Vec::new();
                file.read_to_end(&mut payload)
                    .map_err(|error| BackendError::new(PersistenceOperation::Load, path, error))?;
                Ok(Some(payload))
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(BackendError::new(PersistenceOperation::Load, path, error)),
        }
    }

    fn write_atomic(&self, path: &Path, payload: &[u8]) -> Result<(), BackendError> {
        self.verify_component_file(path)?;
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
            let write_effect = match error.effect {
                AtomicWriteEffect::NotCommitted => PersistenceWriteEffect::NotCommitted,
                AtomicWriteEffect::CommittedDurabilityUncertain => {
                    PersistenceWriteEffect::CommittedDurabilityUncertain
                }
            };
            let mut backend_error = BackendError::new(operation, &error.path, error.error);
            backend_error.write_effect = write_effect;
            backend_error
        })
    }

    fn file_len(&self, path: &Path) -> Result<Option<u64>, BackendError> {
        self.verify_component_file(path)?;
        match fs::symlink_metadata(path) {
            Ok(metadata) => Ok(Some(metadata.len())),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(BackendError::new(PersistenceOperation::Load, path, error)),
        }
    }

    fn append_and_sync(&self, path: &Path, payload: &[u8]) -> Result<(), BackendError> {
        self.verify_component_file(path)?;
        #[cfg(unix)]
        let creates_file =
            fs::symlink_metadata(path).is_err_and(|error| error.kind() == io::ErrorKind::NotFound);
        let mut options = fs::OpenOptions::new();
        options.create(true).append(true);
        #[cfg(unix)]
        {
            std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
            std::os::unix::fs::OpenOptionsExt::custom_flags(&mut options, libc::O_NOFOLLOW);
        }
        let mut file = options
            .open(path)
            .map_err(|error| BackendError::new(PersistenceOperation::Write, path, error))?;
        #[cfg(unix)]
        fs::set_permissions(path, std::os::unix::fs::PermissionsExt::from_mode(0o600))
            .map_err(|error| BackendError::new(PersistenceOperation::Permissions, path, error))?;
        file.write_all(payload).map_err(|error| {
            BackendError::committed_durability_uncertain(PersistenceOperation::Write, path, error)
        })?;
        file.sync_data().map_err(|error| {
            BackendError::committed_durability_uncertain(PersistenceOperation::Sync, path, error)
        })?;
        #[cfg(unix)]
        if creates_file {
            sync_parent_directory(path)?;
        }
        Ok(())
    }

    fn rotate_if_exists(&self, from: &Path, to: &Path) -> Result<bool, BackendError> {
        self.verify_component_file(from)?;
        self.verify_component_file(to)?;
        if !from.exists() {
            return Ok(false);
        }

        replace_for_rotation(from, to)?;
        #[cfg(unix)]
        {
            sync_parent_directory(from)?;
        }
        Ok(true)
    }

    fn remove_if_exists(&self, path: &Path) -> Result<bool, BackendError> {
        self.verify_component_file(path)?;
        match fs::remove_file(path) {
            Ok(()) => {
                #[cfg(unix)]
                sync_parent_directory(path)?;
                Ok(true)
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
            Err(error) => Err(BackendError::new(PersistenceOperation::Remove, path, error)),
        }
    }
}

#[cfg(unix)]
fn sync_parent_directory(path: &Path) -> Result<(), BackendError> {
    let parent = path.parent().ok_or_else(|| {
        BackendError::new(
            PersistenceOperation::Sync,
            path,
            io::Error::new(io::ErrorKind::InvalidInput, "missing parent directory"),
        )
    })?;
    fs::File::open(parent)
        .and_then(|directory| directory.sync_all())
        .map_err(|error| {
            BackendError::committed_durability_uncertain(PersistenceOperation::Sync, path, error)
        })
}

#[cfg(unix)]
fn verify_unix_root_metadata(path: &Path, metadata: &fs::Metadata) -> Result<(), BackendError> {
    use std::os::unix::fs::MetadataExt;

    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(BackendError::new(
            PersistenceOperation::Permissions,
            path,
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "data directory must be a real directory, not a symlink",
            ),
        ));
    }
    // SAFETY: `geteuid` has no arguments and does not dereference memory.
    let current_uid = unsafe { libc::geteuid() };
    if metadata.uid() != current_uid {
        return Err(BackendError::new(
            PersistenceOperation::Permissions,
            path,
            io::Error::new(
                io::ErrorKind::PermissionDenied,
                "data directory is not owned by the current user",
            ),
        ));
    }
    Ok(())
}

#[cfg(unix)]
fn verify_unix_private_directory(path: &Path) -> Result<PermissionVerification, BackendError> {
    use std::os::unix::fs::MetadataExt;

    let metadata = fs::symlink_metadata(path)
        .map_err(|error| BackendError::new(PersistenceOperation::Permissions, path, error))?;
    verify_unix_root_metadata(path, &metadata)?;
    Ok(if metadata.mode() & 0o077 == 0 {
        PermissionVerification::VerifiedPrivate
    } else {
        PermissionVerification::Invalid
    })
}

fn verify_regular_file_or_missing(path: &Path) -> Result<(), BackendError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => {
            #[cfg(windows)]
            {
                use std::os::windows::fs::MetadataExt;
                use windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT;

                if metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
                    return Err(BackendError::new(
                        PersistenceOperation::Permissions,
                        path,
                        io::Error::new(
                            io::ErrorKind::InvalidInput,
                            "reparse-point component rejected",
                        ),
                    ));
                }
            }
            if metadata.file_type().is_symlink() || !metadata.is_file() {
                return Err(BackendError::new(
                    PersistenceOperation::Permissions,
                    path,
                    io::Error::new(
                        io::ErrorKind::InvalidInput,
                        "storage component must be a regular file",
                    ),
                ));
            }
            #[cfg(unix)]
            {
                use std::os::unix::fs::MetadataExt;

                // SAFETY: `geteuid` has no arguments and does not dereference memory.
                let current_uid = unsafe { libc::geteuid() };
                if metadata.uid() != current_uid {
                    return Err(BackendError::new(
                        PersistenceOperation::Permissions,
                        path,
                        io::Error::new(
                            io::ErrorKind::PermissionDenied,
                            "storage component is not owned by the current user",
                        ),
                    ));
                }
                if metadata.mode() & 0o077 != 0 {
                    fs::set_permissions(path, std::os::unix::fs::PermissionsExt::from_mode(0o600))
                        .map_err(|error| {
                            BackendError::new(PersistenceOperation::Permissions, path, error)
                        })?;
                }
            }
            Ok(())
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(BackendError::new(
            PersistenceOperation::Permissions,
            path,
            error,
        )),
    }
}

#[cfg(windows)]
fn replace_for_rotation(from: &Path, to: &Path) -> Result<(), BackendError> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::{
        Foundation::GetLastError,
        Storage::FileSystem::{MoveFileExW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH},
    };

    let mut from_wide = from.as_os_str().encode_wide().collect::<Vec<_>>();
    from_wide.push(0);
    let mut to_wide = to.as_os_str().encode_wide().collect::<Vec<_>>();
    to_wide.push(0);
    let moved = unsafe {
        MoveFileExW(
            from_wide.as_ptr(),
            to_wide.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    };
    if moved == 0 {
        let error = unsafe { GetLastError() };
        return Err(BackendError::new(
            PersistenceOperation::Rotate,
            from,
            io::Error::from_raw_os_error(error as i32),
        ));
    }
    Ok(())
}

#[cfg(not(windows))]
fn replace_for_rotation(from: &Path, to: &Path) -> Result<(), BackendError> {
    fs::rename(from, to)
        .map_err(|error| BackendError::new(PersistenceOperation::Rotate, from, error))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct DiagnosticPolicy {
    pub max_file_bytes: u64,
    pub max_event_bytes: usize,
}

impl Default for DiagnosticPolicy {
    fn default() -> Self {
        Self {
            max_file_bytes: 1024 * 1024,
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

pub(crate) struct UserStorageCoordinator {
    root: ResolvedStorageRoot,
    backend: Box<dyn StorageBackend>,
    diagnostic_policy: DiagnosticPolicy,
    diagnostic_state: Mutex<DiagnosticState>,
}

impl UserStorageCoordinator {
    fn from_current_process() -> Result<Self, PersistenceFailure> {
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

    pub(crate) fn for_current_user_directory(directory: PathBuf) -> Self {
        Self::new(
            ResolvedStorageRoot {
                owner: StorageOwner::CurrentUser,
                directory,
            },
            RealStorageBackend,
            DiagnosticPolicy::default(),
        )
    }

    fn new(
        root: ResolvedStorageRoot,
        backend: impl StorageBackend + 'static,
        diagnostic_policy: DiagnosticPolicy,
    ) -> Self {
        Self {
            root,
            backend: Box::new(backend),
            diagnostic_policy,
            diagnostic_state: Mutex::new(DiagnosticState::default()),
        }
    }

    pub(crate) fn root(&self) -> &ResolvedStorageRoot {
        &self.root
    }

    pub(crate) fn ensure_root(&self) -> Result<PermissionVerification, PersistenceFailure> {
        let verification = self
            .backend
            .prepare_private_directory(&self.root.directory)
            .map_err(BackendError::into_failure)?;
        match verification {
            PermissionVerification::VerifiedPrivate => Ok(verification),
            PermissionVerification::Invalid => Err(PersistenceFailure {
                code: PersistenceFailureCode::PermissionDenied,
                operation: PersistenceOperation::Permissions,
                path: Some(self.root.directory.clone()),
                retryable: true,
                write_effect: PersistenceWriteEffect::NotCommitted,
                summary: "current-user data directory is not private".to_string(),
            }),
            PermissionVerification::Unverified => Err(PersistenceFailure {
                code: PersistenceFailureCode::VerificationUnavailable,
                operation: PersistenceOperation::Permissions,
                path: Some(self.root.directory.clone()),
                retryable: false,
                write_effect: PersistenceWriteEffect::NotCommitted,
                summary: "current-user ownership and permissions are unverified".to_string(),
            }),
        }
    }

    pub(crate) fn component_path(&self, component: UserStorageComponent) -> PathBuf {
        self.root.directory.join(component.file_name())
    }

    pub(crate) fn load_json<T: DeserializeOwned>(
        &self,
        component: UserStorageComponent,
    ) -> Result<Option<T>, PersistenceFailure> {
        self.ensure_root()?;
        let path = self.component_path(component);
        self.backend
            .verify_component_file(&path)
            .map_err(BackendError::into_failure)?;
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
        self.backend
            .verify_component_file(&path)
            .map_err(BackendError::into_failure)?;
        let payload = serde_json::to_vec(value)
            .map_err(|error| PersistenceFailure::serialization(&path, error))?;
        self.backend
            .write_atomic(&path, &payload)
            .map_err(BackendError::into_failure)
    }

    pub(crate) fn remove(
        &self,
        component: UserStorageComponent,
    ) -> Result<bool, PersistenceFailure> {
        self.ensure_root()?;
        let path = self.component_path(component);
        self.backend
            .remove_if_exists(&path)
            .map_err(BackendError::into_failure)
    }

    pub(crate) fn load_json_migrating<T: Serialize>(
        &self,
        component: UserStorageComponent,
        migrate: impl FnOnce(serde_json::Value) -> Result<JsonMigration<T>, String>,
    ) -> Result<Option<MigrationLoad<T>>, PersistenceFailure> {
        self.ensure_root()?;
        let path = self.component_path(component);
        self.backend
            .verify_component_file(&path)
            .map_err(BackendError::into_failure)?;
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
                if diagnostic_failure_opens_breaker(&failure) {
                    state.active_failure = Some(failure.clone());
                }
                DiagnosticWriteOutcome::Failed(failure)
            }
        }
    }

    fn rotate_diagnostics(&self, path: &Path) -> Result<(), PersistenceFailure> {
        let first = diagnostic_backup_path(path, 1);
        self.backend
            .rotate_if_exists(path, &first)
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

pub(crate) struct RuntimePersistenceCoordinator {
    storage: UserStorageCoordinator,
    root: RuntimePersistenceRoot,
    components: HashMap<UserStorageComponent, RuntimePersistenceComponent>,
    root_failure: Option<PersistenceFailure>,
    // Root failures temporarily mask component truth; restore it after a verified recovery.
    components_before_root_failure:
        Option<HashMap<UserStorageComponent, RuntimePersistenceComponent>>,
    components_attempted_during_root_failure: HashSet<UserStorageComponent>,
    root_revalidation_enabled: bool,
    root_failure_reported_to_diagnostics: bool,
    root_failure_suppressed_diagnostics: u64,
}

impl RuntimePersistenceCoordinator {
    pub(crate) fn from_current_process(now_ms: u64) -> Self {
        match UserStorageCoordinator::from_current_process() {
            Ok(storage) => Self::new(storage, now_ms),
            Err(failure) => Self::unavailable(failure, now_ms),
        }
    }

    pub(crate) fn for_current_user_directory(directory: PathBuf, now_ms: u64) -> Self {
        Self::new(
            UserStorageCoordinator::for_current_user_directory(directory),
            now_ms,
        )
    }

    fn new(storage: UserStorageCoordinator, now_ms: u64) -> Self {
        let root = RuntimePersistenceRoot {
            owner: RuntimePersistenceOwner::CurrentUser,
            directory: Some(storage.root().directory.display().to_string()),
            permission_state: RuntimePersistencePermissionState::Verified,
        };
        let components = [
            UserStorageComponent::Settings,
            UserStorageComponent::WarmCache,
            UserStorageComponent::Diagnostics,
        ]
        .into_iter()
        .map(|component| {
            (
                component,
                RuntimePersistenceComponent {
                    owner: RuntimePersistenceOwner::CurrentUser,
                    kind: component.runtime_kind(),
                    state: RuntimePersistenceState::Healthy,
                    durability: RuntimePersistenceDurability::NotApplicable,
                    last_success_at_ms: None,
                    active_failure: None,
                },
            )
        })
        .collect();
        let mut coordinator = Self {
            storage,
            root,
            components,
            root_failure: None,
            components_before_root_failure: None,
            components_attempted_during_root_failure: HashSet::new(),
            root_revalidation_enabled: true,
            root_failure_reported_to_diagnostics: false,
            root_failure_suppressed_diagnostics: 0,
        };
        if let Err(failure) = coordinator.storage.ensure_root() {
            coordinator.record_root_failure(&failure, now_ms);
        }
        coordinator
    }

    fn unavailable(failure: PersistenceFailure, now_ms: u64) -> Self {
        let storage = UserStorageCoordinator::for_current_user_directory(
            env::temp_dir().join("BatCaveMonitor-unavailable"),
        );
        let components = [
            UserStorageComponent::Settings,
            UserStorageComponent::WarmCache,
            UserStorageComponent::Diagnostics,
        ]
        .into_iter()
        .map(|component| {
            (
                component,
                RuntimePersistenceComponent {
                    owner: RuntimePersistenceOwner::CurrentUser,
                    kind: component.runtime_kind(),
                    state: RuntimePersistenceState::Unavailable,
                    durability: RuntimePersistenceDurability::SessionOnly,
                    last_success_at_ms: None,
                    active_failure: Some(runtime_failure(&failure, now_ms)),
                },
            )
        })
        .collect();
        Self {
            storage,
            root: RuntimePersistenceRoot {
                owner: RuntimePersistenceOwner::CurrentUser,
                directory: None,
                permission_state: RuntimePersistencePermissionState::Unavailable,
            },
            components,
            root_failure: Some(failure),
            components_before_root_failure: None,
            components_attempted_during_root_failure: HashSet::new(),
            root_revalidation_enabled: false,
            root_failure_reported_to_diagnostics: false,
            root_failure_suppressed_diagnostics: 0,
        }
    }

    pub(crate) fn runtime_directory(&self) -> &Path {
        &self.storage.root().directory
    }

    pub(crate) fn load_json<T: DeserializeOwned>(
        &mut self,
        component: UserStorageComponent,
        now_ms: u64,
    ) -> Result<Option<T>, PersistenceFailure> {
        self.revalidate_root(component, now_ms)?;
        match self.storage.load_json(component) {
            Ok(value) => {
                let durability = if value.is_some() {
                    RuntimePersistenceDurability::Durable
                } else {
                    RuntimePersistenceDurability::NotApplicable
                };
                self.record_success(component, durability, value.is_some().then_some(now_ms));
                Ok(value)
            }
            Err(failure) => {
                self.record_operation_failure(component, &failure, now_ms);
                Err(failure)
            }
        }
    }

    pub(crate) fn load_json_migrating<T: Serialize>(
        &mut self,
        component: UserStorageComponent,
        now_ms: u64,
        migrate: impl FnOnce(serde_json::Value) -> Result<JsonMigration<T>, String>,
    ) -> Result<Option<MigrationLoad<T>>, PersistenceFailure> {
        self.revalidate_root(component, now_ms)?;
        match self.storage.load_json_migrating(component, migrate) {
            Ok(value) => {
                let durability = if value.is_some() {
                    RuntimePersistenceDurability::Durable
                } else {
                    RuntimePersistenceDurability::NotApplicable
                };
                self.record_success(component, durability, value.is_some().then_some(now_ms));
                Ok(value)
            }
            Err(failure) => {
                self.record_operation_failure(component, &failure, now_ms);
                Err(failure)
            }
        }
    }

    pub(crate) fn write_json<T: Serialize>(
        &mut self,
        component: UserStorageComponent,
        value: &T,
        now_ms: u64,
    ) -> Result<(), PersistenceFailure> {
        self.revalidate_root(component, now_ms)?;
        match self.storage.write_json(component, value) {
            Ok(()) => {
                self.record_success(
                    component,
                    RuntimePersistenceDurability::Durable,
                    Some(now_ms),
                );
                Ok(())
            }
            Err(failure) => {
                self.record_operation_failure(component, &failure, now_ms);
                Err(failure)
            }
        }
    }

    pub(crate) fn remove(
        &mut self,
        component: UserStorageComponent,
        now_ms: u64,
    ) -> Result<(), PersistenceFailure> {
        self.revalidate_root(component, now_ms)?;
        match self.storage.remove(component) {
            Ok(_) => {
                self.record_success(component, RuntimePersistenceDurability::NotApplicable, None);
                Ok(())
            }
            Err(failure) => {
                self.record_operation_failure(component, &failure, now_ms);
                Err(failure)
            }
        }
    }

    pub(crate) fn record_diagnostic<T: Serialize>(
        &mut self,
        event: &T,
        now_ms: u64,
    ) -> DiagnosticWriteOutcome {
        if let Err(failure) = self.revalidate_root(UserStorageComponent::Diagnostics, now_ms) {
            if self.root_failure_reported_to_diagnostics {
                self.root_failure_suppressed_diagnostics =
                    self.root_failure_suppressed_diagnostics.saturating_add(1);
                return DiagnosticWriteOutcome::Suppressed;
            }
            self.root_failure_reported_to_diagnostics = true;
            return DiagnosticWriteOutcome::Failed(failure);
        }
        let outcome = self.storage.record_diagnostic(event);
        match &outcome {
            DiagnosticWriteOutcome::Written => self.record_success(
                UserStorageComponent::Diagnostics,
                RuntimePersistenceDurability::Durable,
                Some(now_ms),
            ),
            DiagnosticWriteOutcome::Suppressed => {}
            DiagnosticWriteOutcome::Failed(failure) => {
                self.record_operation_failure(UserStorageComponent::Diagnostics, failure, now_ms);
            }
        }
        outcome
    }

    pub(crate) fn retry_diagnostics(&mut self) {
        if self.root_failure.is_none() {
            self.storage.retry_diagnostics();
        }
    }

    fn revalidate_root(
        &mut self,
        component: UserStorageComponent,
        now_ms: u64,
    ) -> Result<(), PersistenceFailure> {
        if !self.root_revalidation_enabled {
            self.components_attempted_during_root_failure
                .insert(component);
            return Err(self
                .root_failure
                .clone()
                .expect("unresolved roots retain their failure"));
        }
        match self.storage.ensure_root() {
            Ok(PermissionVerification::VerifiedPrivate) => {
                if let Some(failure) = self.root_failure.take() {
                    self.root.permission_state = RuntimePersistencePermissionState::Verified;
                    self.root.directory = Some(self.storage.root().directory.display().to_string());
                    self.root_failure_reported_to_diagnostics = false;
                    if let Some(components) = self.components_before_root_failure.take() {
                        self.components = components;
                    }
                    for attempted in self.components_attempted_during_root_failure.drain() {
                        let state = self
                            .components
                            .get_mut(&attempted)
                            .expect("all current-user components are initialized");
                        state.state = RuntimePersistenceState::Degraded;
                        state.durability = RuntimePersistenceDurability::SessionOnly;
                        state.active_failure = Some(runtime_failure(&failure, now_ms));
                    }
                }
                Ok(())
            }
            Ok(PermissionVerification::Unverified) => {
                unreachable!("unverified roots fail closed")
            }
            Ok(PermissionVerification::Invalid) => unreachable!("invalid roots fail closed"),
            Err(failure) => {
                self.record_root_failure(&failure, now_ms);
                self.components_attempted_during_root_failure
                    .insert(component);
                Err(failure)
            }
        }
    }

    fn record_operation_failure(
        &mut self,
        component: UserStorageComponent,
        failure: &PersistenceFailure,
        now_ms: u64,
    ) {
        if failure.path.as_deref() == Some(self.storage.root().directory.as_path())
            && matches!(
                failure.operation,
                PersistenceOperation::Create | PersistenceOperation::Permissions
            )
        {
            self.record_root_failure(failure, now_ms);
            self.components_attempted_during_root_failure
                .insert(component);
            return;
        }
        if self.root_revalidation_enabled {
            if let Err(root_failure) = self.storage.ensure_root() {
                self.record_root_failure(&root_failure, now_ms);
                self.components_attempted_during_root_failure
                    .insert(component);
                return;
            }
        }
        self.record_failure(component, failure, now_ms);
    }

    fn record_root_failure(&mut self, failure: &PersistenceFailure, now_ms: u64) {
        if self.root_failure.is_none() {
            self.components_before_root_failure = Some(self.components.clone());
            self.root_failure_reported_to_diagnostics = false;
        }
        let permission_state = if failure.code == PersistenceFailureCode::VerificationUnavailable {
            RuntimePersistencePermissionState::Unavailable
        } else {
            RuntimePersistencePermissionState::Invalid
        };
        self.root.permission_state = permission_state;
        self.root.directory = (permission_state != RuntimePersistencePermissionState::Unavailable)
            .then(|| self.storage.root().directory.display().to_string());
        self.root_failure = Some(failure.clone());
        for component in self.components.values_mut() {
            component.state = if permission_state == RuntimePersistencePermissionState::Unavailable
            {
                RuntimePersistenceState::Unavailable
            } else {
                RuntimePersistenceState::Degraded
            };
            component.durability = RuntimePersistenceDurability::SessionOnly;
            component.active_failure = Some(runtime_failure(failure, now_ms));
        }
    }

    pub(crate) fn health(&self) -> RuntimePersistence {
        let mut components = [
            UserStorageComponent::Settings,
            UserStorageComponent::WarmCache,
            UserStorageComponent::Diagnostics,
        ]
        .into_iter()
        .filter_map(|component| self.components.get(&component).cloned())
        .collect::<Vec<_>>();
        components.sort_by_key(|component| match component.kind {
            RuntimePersistenceKind::Settings => 0,
            RuntimePersistenceKind::WarmCache => 1,
            RuntimePersistenceKind::Diagnostics => 2,
            RuntimePersistenceKind::ServiceState => 3,
        });
        let root_state = match self.root.permission_state {
            RuntimePersistencePermissionState::Verified => RuntimePersistenceState::Healthy,
            RuntimePersistencePermissionState::Invalid => RuntimePersistenceState::Degraded,
            RuntimePersistencePermissionState::Unavailable => RuntimePersistenceState::Unavailable,
        };
        let state = components
            .iter()
            .filter(|component| component.durability != RuntimePersistenceDurability::NotApplicable)
            .fold(root_state, |state, component| {
                worst_state(state, component.state)
            });
        RuntimePersistence {
            state,
            roots: vec![self.root.clone()],
            components,
            suppressed_diagnostic_events: self
                .storage
                .diagnostic_status()
                .suppressed_events
                .saturating_add(self.root_failure_suppressed_diagnostics),
        }
    }

    pub(crate) fn failure_message(failure: &PersistenceFailure) -> String {
        format!(
            "persistence_{} operation={}{} error={}",
            failure_code(failure.code),
            operation_name(failure.operation),
            failure
                .path
                .as_ref()
                .map(|path| format!(" path={}", path.display()))
                .unwrap_or_default(),
            failure.summary
        )
    }

    fn record_success(
        &mut self,
        component: UserStorageComponent,
        durability: RuntimePersistenceDurability,
        last_success_at_ms: Option<u64>,
    ) {
        let state = self
            .components
            .get_mut(&component)
            .expect("all current-user components are initialized");
        state.state = RuntimePersistenceState::Healthy;
        state.durability = durability;
        state.last_success_at_ms = last_success_at_ms.or(state.last_success_at_ms);
        state.active_failure = None;
    }

    fn record_failure(
        &mut self,
        component: UserStorageComponent,
        failure: &PersistenceFailure,
        now_ms: u64,
    ) {
        let state = self
            .components
            .get_mut(&component)
            .expect("all current-user components are initialized");
        state.state =
            if self.root.permission_state == RuntimePersistencePermissionState::Unavailable {
                RuntimePersistenceState::Unavailable
            } else {
                RuntimePersistenceState::Degraded
            };
        state.durability = RuntimePersistenceDurability::SessionOnly;
        state.active_failure = Some(runtime_failure(failure, now_ms));
    }
}

fn runtime_failure(failure: &PersistenceFailure, now_ms: u64) -> RuntimePersistenceFailure {
    RuntimePersistenceFailure {
        code: failure_code(failure.code).to_string(),
        operation: runtime_operation(failure.operation),
        occurred_at_ms: now_ms,
        retryable: failure.retryable,
        summary: failure.summary.clone(),
    }
}

fn failure_code(code: PersistenceFailureCode) -> &'static str {
    match code {
        PersistenceFailureCode::PermissionDenied => "permission_denied",
        PersistenceFailureCode::ReadOnlyFilesystem => "read_only_filesystem",
        PersistenceFailureCode::StorageFull => "storage_full",
        PersistenceFailureCode::CorruptData => "corrupt_data",
        PersistenceFailureCode::InvalidPath => "invalid_path",
        PersistenceFailureCode::NotFound => "not_found",
        PersistenceFailureCode::SerializationFailed => "serialization_failed",
        PersistenceFailureCode::MigrationFailed => "migration_failed",
        PersistenceFailureCode::VerificationUnavailable => "verification_unavailable",
        PersistenceFailureCode::IoFailure => "io_failure",
    }
}

fn operation_name(operation: PersistenceOperation) -> &'static str {
    match operation {
        PersistenceOperation::ResolveRoot => "resolve_root",
        PersistenceOperation::Create => "create",
        PersistenceOperation::Load => "load",
        PersistenceOperation::Parse => "parse",
        PersistenceOperation::Migrate => "migrate",
        PersistenceOperation::Serialize => "serialize",
        PersistenceOperation::Write => "write",
        PersistenceOperation::Sync => "sync",
        PersistenceOperation::Replace => "replace",
        PersistenceOperation::Rotate => "rotate",
        PersistenceOperation::Remove => "remove",
        PersistenceOperation::Permissions => "permissions",
    }
}

fn runtime_operation(operation: PersistenceOperation) -> RuntimePersistenceOperation {
    match operation {
        PersistenceOperation::ResolveRoot => RuntimePersistenceOperation::ResolveRoot,
        PersistenceOperation::Create => RuntimePersistenceOperation::Create,
        PersistenceOperation::Load => RuntimePersistenceOperation::Load,
        PersistenceOperation::Parse => RuntimePersistenceOperation::Parse,
        PersistenceOperation::Migrate => RuntimePersistenceOperation::Migrate,
        PersistenceOperation::Serialize => RuntimePersistenceOperation::Serialize,
        PersistenceOperation::Write => RuntimePersistenceOperation::Write,
        PersistenceOperation::Sync => RuntimePersistenceOperation::Sync,
        PersistenceOperation::Replace => RuntimePersistenceOperation::Replace,
        PersistenceOperation::Rotate => RuntimePersistenceOperation::Rotate,
        PersistenceOperation::Remove => RuntimePersistenceOperation::Remove,
        PersistenceOperation::Permissions => RuntimePersistenceOperation::Permissions,
    }
}

fn worst_state(
    left: RuntimePersistenceState,
    right: RuntimePersistenceState,
) -> RuntimePersistenceState {
    let rank = |state| match state {
        RuntimePersistenceState::Healthy => 0,
        RuntimePersistenceState::Degraded => 1,
        RuntimePersistenceState::Unavailable => 2,
    };
    if rank(right) > rank(left) {
        right
    } else {
        left
    }
}

fn diagnostic_failure_opens_breaker(failure: &PersistenceFailure) -> bool {
    matches!(
        failure.operation,
        PersistenceOperation::Create
            | PersistenceOperation::Load
            | PersistenceOperation::Write
            | PersistenceOperation::Sync
            | PersistenceOperation::Replace
            | PersistenceOperation::Rotate
            | PersistenceOperation::Remove
            | PersistenceOperation::Permissions
    )
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
        collections::{HashMap, HashSet, VecDeque},
        env,
        sync::{Arc, Mutex},
    };

    use serde::Serializer;

    use super::*;

    #[derive(Debug, Clone)]
    struct InjectedFault {
        operation: PersistenceOperation,
        kind: io::ErrorKind,
        write_effect: PersistenceWriteEffect,
    }

    #[derive(Debug, Default)]
    struct FakeState {
        files: HashMap<PathBuf, Vec<u8>>,
        faults: VecDeque<InjectedFault>,
        calls: Vec<PersistenceOperation>,
        permission: Option<PermissionVerification>,
        unsafe_paths: HashSet<PathBuf>,
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
            self.state.lock().unwrap().faults.push_back(InjectedFault {
                operation,
                kind,
                write_effect: PersistenceWriteEffect::NotCommitted,
            });
        }

        fn fail_next_committed(&self, operation: PersistenceOperation, kind: io::ErrorKind) {
            self.state.lock().unwrap().faults.push_back(InjectedFault {
                operation,
                kind,
                write_effect: PersistenceWriteEffect::CommittedDurabilityUncertain,
            });
        }

        fn check_fault(
            &self,
            operation: PersistenceOperation,
            path: &Path,
            write_effect: PersistenceWriteEffect,
        ) -> Result<(), BackendError> {
            let mut state = self.state.lock().unwrap();
            state.calls.push(operation);
            if state.faults.front().is_some_and(|fault| {
                fault.operation == operation && fault.write_effect == write_effect
            }) {
                let fault = state.faults.pop_front().unwrap();
                let error = io::Error::from(fault.kind);
                return Err(match fault.write_effect {
                    PersistenceWriteEffect::NotCommitted => {
                        BackendError::new(operation, path, error)
                    }
                    PersistenceWriteEffect::CommittedDurabilityUncertain => {
                        BackendError::committed_durability_uncertain(operation, path, error)
                    }
                });
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
        fn prepare_private_directory(
            &self,
            path: &Path,
        ) -> Result<PermissionVerification, BackendError> {
            self.check_fault(
                PersistenceOperation::Create,
                path,
                PersistenceWriteEffect::NotCommitted,
            )?;
            self.check_fault(
                PersistenceOperation::Permissions,
                path,
                PersistenceWriteEffect::NotCommitted,
            )?;
            if self.state.lock().unwrap().unsafe_paths.contains(path) {
                return Err(BackendError::new(
                    PersistenceOperation::Permissions,
                    path,
                    io::Error::new(io::ErrorKind::InvalidInput, "symlink or reparse root"),
                ));
            }
            Ok(self
                .state
                .lock()
                .unwrap()
                .permission
                .unwrap_or(PermissionVerification::VerifiedPrivate))
        }

        fn verify_component_file(&self, path: &Path) -> Result<(), BackendError> {
            self.check_fault(
                PersistenceOperation::Permissions,
                path,
                PersistenceWriteEffect::NotCommitted,
            )?;
            if self.state.lock().unwrap().unsafe_paths.contains(path) {
                return Err(BackendError::new(
                    PersistenceOperation::Permissions,
                    path,
                    io::Error::new(io::ErrorKind::InvalidInput, "symlink or reparse component"),
                ));
            }
            Ok(())
        }

        fn read(&self, path: &Path) -> Result<Option<Vec<u8>>, BackendError> {
            self.check_fault(
                PersistenceOperation::Load,
                path,
                PersistenceWriteEffect::NotCommitted,
            )?;
            Ok(self.state.lock().unwrap().files.get(path).cloned())
        }

        fn write_atomic(&self, path: &Path, payload: &[u8]) -> Result<(), BackendError> {
            for operation in [
                PersistenceOperation::Write,
                PersistenceOperation::Sync,
                PersistenceOperation::Replace,
            ] {
                self.check_fault(operation, path, PersistenceWriteEffect::NotCommitted)?;
            }
            self.state
                .lock()
                .unwrap()
                .files
                .insert(path.to_path_buf(), payload.to_vec());
            self.check_fault(
                PersistenceOperation::Sync,
                path,
                PersistenceWriteEffect::CommittedDurabilityUncertain,
            )
        }

        fn file_len(&self, path: &Path) -> Result<Option<u64>, BackendError> {
            self.check_fault(
                PersistenceOperation::Load,
                path,
                PersistenceWriteEffect::NotCommitted,
            )?;
            Ok(self
                .state
                .lock()
                .unwrap()
                .files
                .get(path)
                .map(|payload| payload.len() as u64))
        }

        fn append_and_sync(&self, path: &Path, payload: &[u8]) -> Result<(), BackendError> {
            self.check_fault(
                PersistenceOperation::Write,
                path,
                PersistenceWriteEffect::NotCommitted,
            )?;
            self.state
                .lock()
                .unwrap()
                .files
                .entry(path.to_path_buf())
                .or_default()
                .extend_from_slice(payload);
            self.check_fault(
                PersistenceOperation::Sync,
                path,
                PersistenceWriteEffect::CommittedDurabilityUncertain,
            )
        }

        fn rotate_if_exists(&self, from: &Path, to: &Path) -> Result<bool, BackendError> {
            self.verify_component_file(from)?;
            self.verify_component_file(to)?;
            self.check_fault(
                PersistenceOperation::Rotate,
                from,
                PersistenceWriteEffect::NotCommitted,
            )?;
            let mut state = self.state.lock().unwrap();
            let Some(payload) = state.files.remove(from) else {
                return Ok(false);
            };
            state.files.insert(to.to_path_buf(), payload);
            drop(state);
            self.check_fault(
                PersistenceOperation::Sync,
                from,
                PersistenceWriteEffect::CommittedDurabilityUncertain,
            )?;
            Ok(true)
        }

        fn remove_if_exists(&self, path: &Path) -> Result<bool, BackendError> {
            self.verify_component_file(path)?;
            self.check_fault(
                PersistenceOperation::Remove,
                path,
                PersistenceWriteEffect::NotCommitted,
            )?;
            Ok(self.state.lock().unwrap().files.remove(path).is_some())
        }
    }

    fn root() -> ResolvedStorageRoot {
        ResolvedStorageRoot {
            owner: StorageOwner::CurrentUser,
            directory: PathBuf::from("/tmp/batcave-persistence-tests"),
        }
    }

    fn coordinator(backend: FakeBackend) -> UserStorageCoordinator {
        UserStorageCoordinator::new(
            root(),
            backend,
            DiagnosticPolicy {
                max_file_bytes: 32,
                max_event_bytes: 128,
            },
        )
    }

    fn runtime_coordinator(backend: FakeBackend, now_ms: u64) -> RuntimePersistenceCoordinator {
        RuntimePersistenceCoordinator::new(coordinator(backend), now_ms)
    }

    fn runtime_component(
        health: &RuntimePersistence,
        kind: RuntimePersistenceKind,
    ) -> &RuntimePersistenceComponent {
        health
            .components
            .iter()
            .find(|component| component.kind == kind)
            .expect("runtime persistence component exists")
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
    fn empty_relative_and_parent_traversing_environment_roots_are_rejected() {
        for local_app_data in [
            "",
            "relative",
            r"C:\Users\albert\..\other",
            r"\\server\share",
        ] {
            let failure = resolve_current_user_root(
                StoragePlatform::Windows,
                &CurrentUserEnvironment {
                    local_app_data: Some(PathBuf::from(local_app_data)),
                    ..CurrentUserEnvironment::default()
                },
            )
            .unwrap_err();
            assert_eq!(failure.operation, PersistenceOperation::ResolveRoot);
        }

        for home in [
            "",
            "relative",
            "/Users/albert/../other",
            "/Users/albert/./other",
        ] {
            let failure = resolve_current_user_root(
                StoragePlatform::Macos,
                &CurrentUserEnvironment {
                    home: Some(PathBuf::from(home)),
                    ..CurrentUserEnvironment::default()
                },
            )
            .unwrap_err();
            assert_eq!(failure.operation, PersistenceOperation::ResolveRoot);
        }

        for xdg_data_home in ["", "relative", "/var/../other", "/var/./other"] {
            let failure = resolve_current_user_root(
                StoragePlatform::Linux,
                &CurrentUserEnvironment {
                    xdg_data_home: Some(PathBuf::from(xdg_data_home)),
                    ..CurrentUserEnvironment::default()
                },
            )
            .unwrap_err();
            assert_eq!(failure.operation, PersistenceOperation::ResolveRoot);
        }
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
    fn unverified_root_prevents_reads_and_writes() {
        let backend = FakeBackend::default();
        backend.state.lock().unwrap().permission = Some(PermissionVerification::Unverified);
        let coordinator = coordinator(backend.clone());

        let load_failure = coordinator
            .load_json::<serde_json::Value>(UserStorageComponent::Settings)
            .unwrap_err();
        assert_eq!(
            load_failure.code,
            PersistenceFailureCode::VerificationUnavailable
        );
        assert_eq!(backend.call_count(PersistenceOperation::Load), 0);

        let write_failure = coordinator
            .write_json(
                UserStorageComponent::Settings,
                &serde_json::json!({"safe": true}),
            )
            .unwrap_err();
        assert_eq!(
            write_failure.code,
            PersistenceFailureCode::VerificationUnavailable
        );
        assert_eq!(backend.call_count(PersistenceOperation::Write), 0);
    }

    #[test]
    fn runtime_health_fails_closed_when_permission_verification_is_unavailable() {
        let backend = FakeBackend::default();
        backend.state.lock().unwrap().permission = Some(PermissionVerification::Unverified);

        let runtime = runtime_coordinator(backend, 10);
        let health = runtime.health();

        assert_eq!(health.state, RuntimePersistenceState::Unavailable);
        assert_eq!(health.roots[0].directory, None);
        assert_eq!(
            health.roots[0].permission_state,
            RuntimePersistencePermissionState::Unavailable
        );
        assert!(health.components.iter().all(|component| {
            component.state == RuntimePersistenceState::Unavailable
                && component.durability == RuntimePersistenceDurability::SessionOnly
                && component.active_failure.is_some()
        }));
    }

    #[test]
    fn runtime_root_health_recovers_after_a_transient_verification_failure() {
        let backend = FakeBackend::default();
        backend.fail_next(
            PersistenceOperation::Permissions,
            io::ErrorKind::PermissionDenied,
        );
        let mut runtime = runtime_coordinator(backend.clone(), 10);

        let degraded = runtime.health();
        assert_eq!(degraded.state, RuntimePersistenceState::Degraded);
        assert_eq!(
            degraded.roots[0].permission_state,
            RuntimePersistencePermissionState::Invalid
        );

        runtime
            .write_json(
                UserStorageComponent::Settings,
                &serde_json::json!({"recovered": true}),
                20,
            )
            .expect("the next operation revalidates the recovered root");

        let recovered = runtime.health();
        assert_eq!(recovered.state, RuntimePersistenceState::Healthy);
        assert_eq!(
            recovered.roots[0].permission_state,
            RuntimePersistencePermissionState::Verified
        );
        let settings = recovered
            .components
            .iter()
            .find(|component| component.kind == RuntimePersistenceKind::Settings)
            .expect("settings health is published");
        assert_eq!(settings.state, RuntimePersistenceState::Healthy);
        assert_eq!(settings.durability, RuntimePersistenceDurability::Durable);
        assert!(settings.active_failure.is_none());
    }

    #[test]
    fn runtime_root_health_detects_later_permission_invalidation() {
        let backend = FakeBackend::default();
        let mut runtime = runtime_coordinator(backend.clone(), 10);
        assert_eq!(runtime.health().state, RuntimePersistenceState::Healthy);
        backend.state.lock().unwrap().permission = Some(PermissionVerification::Invalid);

        let failure = runtime
            .write_json(
                UserStorageComponent::Settings,
                &serde_json::json!({"unsafe": false}),
                20,
            )
            .expect_err("an invalidated root fails before component I/O");

        assert_eq!(failure.operation, PersistenceOperation::Permissions);
        assert_eq!(backend.call_count(PersistenceOperation::Write), 0);
        let invalid = runtime.health();
        assert_eq!(invalid.state, RuntimePersistenceState::Degraded);
        assert_eq!(
            invalid.roots[0].permission_state,
            RuntimePersistencePermissionState::Invalid
        );
        assert!(invalid.components.iter().all(|component| {
            component.state == RuntimePersistenceState::Degraded
                && component.durability == RuntimePersistenceDurability::SessionOnly
                && component.active_failure.is_some()
        }));
    }

    #[test]
    fn cross_component_root_recovery_keeps_failed_settings_session_only() {
        let backend = FakeBackend::default();
        let mut runtime = runtime_coordinator(backend.clone(), 10);
        let settings_path = root().directory.join("settings.json");
        runtime
            .write_json(
                UserStorageComponent::Settings,
                &serde_json::json!({"value": "A"}),
                11,
            )
            .expect("initial settings are durable");
        backend.state.lock().unwrap().permission = Some(PermissionVerification::Invalid);
        runtime
            .write_json(
                UserStorageComponent::Settings,
                &serde_json::json!({"value": "B"}),
                20,
            )
            .expect_err("settings B cannot cross an invalid root");
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&backend.file(&settings_path).unwrap())
                .unwrap(),
            serde_json::json!({"value": "A"})
        );

        backend.state.lock().unwrap().permission = Some(PermissionVerification::VerifiedPrivate);
        runtime
            .write_json(
                UserStorageComponent::WarmCache,
                &serde_json::json!({"rows": []}),
                30,
            )
            .expect("warm cache is first to recover the root");

        let partially_recovered = runtime.health();
        assert_eq!(
            partially_recovered.roots[0].permission_state,
            RuntimePersistencePermissionState::Verified
        );
        assert_eq!(partially_recovered.state, RuntimePersistenceState::Degraded);
        let settings = partially_recovered
            .components
            .iter()
            .find(|component| component.kind == RuntimePersistenceKind::Settings)
            .expect("settings health is published");
        assert_eq!(settings.state, RuntimePersistenceState::Degraded);
        assert_eq!(
            settings.durability,
            RuntimePersistenceDurability::SessionOnly
        );
        assert!(settings.active_failure.is_some());

        runtime
            .write_json(
                UserStorageComponent::Settings,
                &serde_json::json!({"value": "B"}),
                40,
            )
            .expect("settings independently re-prove durability");
        assert_eq!(runtime.health().state, RuntimePersistenceState::Healthy);
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&backend.file(&settings_path).unwrap())
                .unwrap(),
            serde_json::json!({"value": "B"})
        );
    }

    #[test]
    fn runtime_health_publishes_write_replace_and_full_disk_failures() {
        for (operation, kind, code) in [
            (
                PersistenceOperation::Write,
                io::ErrorKind::StorageFull,
                "storage_full",
            ),
            (
                PersistenceOperation::Write,
                io::ErrorKind::PermissionDenied,
                "permission_denied",
            ),
            (
                PersistenceOperation::Replace,
                io::ErrorKind::PermissionDenied,
                "permission_denied",
            ),
        ] {
            let backend = FakeBackend::default();
            let mut runtime = runtime_coordinator(backend.clone(), 10);
            backend.fail_next(operation, kind);

            let failure = runtime
                .write_json(
                    UserStorageComponent::Settings,
                    &serde_json::json!({"theme":"cave"}),
                    20,
                )
                .unwrap_err();
            let health = runtime.health();
            let settings = runtime_component(&health, RuntimePersistenceKind::Settings);

            assert_eq!(failure.operation, operation);
            assert_eq!(health.state, RuntimePersistenceState::Degraded);
            assert_eq!(settings.state, RuntimePersistenceState::Degraded);
            assert_eq!(
                settings.durability,
                RuntimePersistenceDurability::SessionOnly
            );
            assert_eq!(settings.active_failure.as_ref().unwrap().code, code);
            assert_eq!(settings.active_failure.as_ref().unwrap().occurred_at_ms, 20);
        }
    }

    #[test]
    fn runtime_health_publishes_serialization_and_corruption_failures() {
        struct FailsSerialization;
        impl Serialize for FailsSerialization {
            fn serialize<S>(&self, _serializer: S) -> Result<S::Ok, S::Error>
            where
                S: Serializer,
            {
                Err(serde::ser::Error::custom(
                    "forced runtime serialization failure",
                ))
            }
        }

        let backend = FakeBackend::default();
        let mut runtime = runtime_coordinator(backend, 10);
        runtime
            .write_json(UserStorageComponent::Settings, &FailsSerialization, 20)
            .unwrap_err();
        let health = runtime.health();
        let settings = runtime_component(&health, RuntimePersistenceKind::Settings);
        assert_eq!(
            settings.active_failure.as_ref().unwrap().code,
            "serialization_failed"
        );
        assert_eq!(
            settings.active_failure.as_ref().unwrap().operation,
            RuntimePersistenceOperation::Serialize
        );

        let path = root().directory.join("warm-cache.json");
        let backend = FakeBackend::with_file(path, b"{not-json".to_vec());
        let mut runtime = runtime_coordinator(backend, 30);
        runtime
            .load_json::<serde_json::Value>(UserStorageComponent::WarmCache, 40)
            .unwrap_err();
        let health = runtime.health();
        let cache = runtime_component(&health, RuntimePersistenceKind::WarmCache);
        assert_eq!(cache.active_failure.as_ref().unwrap().code, "corrupt_data");
        assert_eq!(
            cache.active_failure.as_ref().unwrap().operation,
            RuntimePersistenceOperation::Parse
        );
    }

    #[test]
    fn runtime_health_publishes_rotation_failure_and_bounded_suppression() {
        let backend = FakeBackend::default();
        let mut runtime = runtime_coordinator(backend.clone(), 10);
        let path = root().directory.join("diagnostics.jsonl");
        backend
            .state
            .lock()
            .unwrap()
            .files
            .insert(path, vec![b'a'; 30]);
        backend.fail_next(
            PersistenceOperation::Rotate,
            io::ErrorKind::PermissionDenied,
        );

        assert!(matches!(
            runtime.record_diagnostic(&serde_json::json!({"event":"first"}), 20),
            DiagnosticWriteOutcome::Failed(_)
        ));
        for index in 0..100 {
            assert_eq!(
                runtime.record_diagnostic(&serde_json::json!({"event":index}), 21 + index),
                DiagnosticWriteOutcome::Suppressed
            );
        }

        let health = runtime.health();
        let diagnostics = runtime_component(&health, RuntimePersistenceKind::Diagnostics);
        assert_eq!(health.suppressed_diagnostic_events, 100);
        assert_eq!(diagnostics.state, RuntimePersistenceState::Degraded);
        assert_eq!(
            diagnostics.active_failure.as_ref().unwrap().operation,
            RuntimePersistenceOperation::Rotate
        );
        assert_eq!(backend.call_count(PersistenceOperation::Rotate), 1);
    }

    #[test]
    fn symlink_or_reparse_roots_and_components_are_rejected_before_io() {
        let backend = FakeBackend::default();
        backend
            .state
            .lock()
            .unwrap()
            .unsafe_paths
            .insert(root().directory.clone());
        let coordinator_under_test = coordinator(backend.clone());
        let failure = coordinator_under_test
            .load_json::<serde_json::Value>(UserStorageComponent::Settings)
            .unwrap_err();
        assert_eq!(failure.operation, PersistenceOperation::Permissions);
        assert_eq!(failure.code, PersistenceFailureCode::InvalidPath);
        assert_eq!(backend.call_count(PersistenceOperation::Load), 0);

        let backend = FakeBackend::default();
        let coordinator_under_test = coordinator(backend.clone());
        let path = coordinator_under_test.component_path(UserStorageComponent::Settings);
        backend.state.lock().unwrap().unsafe_paths.insert(path);
        let failure = coordinator_under_test
            .load_json_migrating::<serde_json::Value>(UserStorageComponent::Settings, |value| {
                Ok(JsonMigration::Current(value))
            })
            .unwrap_err();
        assert_eq!(failure.operation, PersistenceOperation::Permissions);
        assert_eq!(failure.code, PersistenceFailureCode::InvalidPath);
        assert_eq!(backend.call_count(PersistenceOperation::Load), 0);
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
            assert_eq!(failure.write_effect, PersistenceWriteEffect::NotCommitted);
            assert_eq!(backend.file(&path).unwrap(), br#"{"old":true}"#);
        }
    }

    #[test]
    fn post_commit_directory_sync_failure_reports_uncertain_durability() {
        let backend = FakeBackend::default();
        let coordinator = coordinator(backend.clone());
        let path = coordinator.component_path(UserStorageComponent::Settings);
        backend
            .state
            .lock()
            .unwrap()
            .files
            .insert(path.clone(), br#"{"old":true}"#.to_vec());
        backend.fail_next_committed(
            PersistenceOperation::Sync,
            io::ErrorKind::ReadOnlyFilesystem,
        );

        let failure = coordinator
            .write_json(
                UserStorageComponent::Settings,
                &serde_json::json!({"new": true}),
            )
            .unwrap_err();

        assert_eq!(failure.operation, PersistenceOperation::Sync);
        assert_eq!(
            failure.write_effect,
            PersistenceWriteEffect::CommittedDurabilityUncertain
        );
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&backend.file(&path).unwrap()).unwrap(),
            serde_json::json!({"new": true})
        );
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
        {
            let mut state = backend.state.lock().unwrap();
            state.files.insert(path.clone(), vec![b'a'; 30]);
            state.files.insert(first.clone(), b"previous".to_vec());
        }

        assert_eq!(
            coordinator.record_diagnostic(&serde_json::json!({"event":"new"})),
            DiagnosticWriteOutcome::Written
        );

        assert_eq!(backend.file(&first).unwrap(), vec![b'a'; 30]);
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
            .insert(path.clone(), vec![b'a'; 30]);
        let backup = diagnostic_backup_path(&path, 1);
        backend
            .state
            .lock()
            .unwrap()
            .files
            .insert(backup.clone(), b"previous".to_vec());
        backend.fail_next(
            PersistenceOperation::Rotate,
            io::ErrorKind::PermissionDenied,
        );

        let first = coordinator.record_diagnostic(&serde_json::json!({"event":"new"}));
        let DiagnosticWriteOutcome::Failed(failure) = first else {
            panic!("rotation failure should be observable");
        };
        assert_eq!(failure.operation, PersistenceOperation::Rotate);
        assert_eq!(failure.code, PersistenceFailureCode::PermissionDenied);
        assert_eq!(failure.write_effect, PersistenceWriteEffect::NotCommitted);
        assert_eq!(backend.file(&path).unwrap(), vec![b'a'; 30]);
        assert_eq!(backend.file(&backup).unwrap(), b"previous");

        assert_eq!(
            coordinator.record_diagnostic(&serde_json::json!({"event":"suppressed"})),
            DiagnosticWriteOutcome::Suppressed
        );
        assert_eq!(coordinator.diagnostic_status().suppressed_events, 1);
    }

    #[test]
    fn rotation_directory_sync_failure_preserves_current_log_in_backup() {
        let backend = FakeBackend::default();
        let coordinator = coordinator(backend.clone());
        let path = coordinator.component_path(UserStorageComponent::Diagnostics);
        let backup = diagnostic_backup_path(&path, 1);
        backend
            .state
            .lock()
            .unwrap()
            .files
            .insert(path.clone(), vec![b'a'; 30]);
        backend.fail_next_committed(
            PersistenceOperation::Sync,
            io::ErrorKind::ReadOnlyFilesystem,
        );

        let outcome = coordinator.record_diagnostic(&serde_json::json!({"event":"new"}));
        let DiagnosticWriteOutcome::Failed(failure) = outcome else {
            panic!("directory sync failure should be observable");
        };
        assert_eq!(failure.operation, PersistenceOperation::Sync);
        assert_eq!(
            failure.write_effect,
            PersistenceWriteEffect::CommittedDurabilityUncertain
        );
        assert!(backend.file(&path).is_none());
        assert_eq!(backend.file(&backup).unwrap(), vec![b'a'; 30]);
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
    fn diagnostic_sync_failure_reports_possible_committed_event() {
        let backend = FakeBackend::default();
        backend.fail_next_committed(
            PersistenceOperation::Sync,
            io::ErrorKind::ReadOnlyFilesystem,
        );
        let coordinator = coordinator(backend.clone());
        let path = coordinator.component_path(UserStorageComponent::Diagnostics);

        let outcome = coordinator.record_diagnostic(&serde_json::json!({"event":"written"}));
        let DiagnosticWriteOutcome::Failed(failure) = outcome else {
            panic!("diagnostic sync failure should be observable");
        };
        assert_eq!(failure.operation, PersistenceOperation::Sync);
        assert_eq!(
            failure.write_effect,
            PersistenceWriteEffect::CommittedDurabilityUncertain
        );
        assert!(backend.file(&path).unwrap().ends_with(b"\n"));
        assert!(coordinator.diagnostic_status().active_failure.is_some());
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
        assert!(coordinator.diagnostic_status().active_failure.is_none());
        assert_eq!(coordinator.diagnostic_status().suppressed_events, 0);

        assert_eq!(
            coordinator.record_diagnostic(&serde_json::json!({"ok":true})),
            DiagnosticWriteOutcome::Written
        );
    }

    #[cfg(unix)]
    #[test]
    fn real_backend_creates_private_root_and_files() {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};

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
        let settings_path = coordinator.component_path(UserStorageComponent::Settings);
        fs::set_permissions(&settings_path, fs::Permissions::from_mode(0o644)).unwrap();
        coordinator
            .load_json::<serde_json::Value>(UserStorageComponent::Settings)
            .expect("legacy current-user file is hardened before loading");
        assert_eq!(fs::metadata(settings_path).unwrap().mode() & 0o777, 0o600);
        fs::remove_dir_all(directory).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn real_backend_rejects_symlinks_and_hardens_owned_component_permissions() {
        use std::os::unix::fs::{symlink, PermissionsExt};

        let fixture = env::temp_dir().join(format!(
            "batcave-persistence-symlink-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&fixture);
        fs::create_dir_all(&fixture).unwrap();

        let target_root = fixture.join("target-root");
        let linked_root = fixture.join("linked-root");
        fs::create_dir(&target_root).unwrap();
        symlink(&target_root, &linked_root).unwrap();
        let linked_coordinator = UserStorageCoordinator::new(
            ResolvedStorageRoot {
                owner: StorageOwner::CurrentUser,
                directory: linked_root,
            },
            RealStorageBackend,
            DiagnosticPolicy::default(),
        );
        let failure = linked_coordinator.ensure_root().unwrap_err();
        assert_eq!(failure.operation, PersistenceOperation::Permissions);
        assert_eq!(failure.code, PersistenceFailureCode::InvalidPath);

        let real_root = fixture.join("real-root");
        let coordinator = UserStorageCoordinator::new(
            ResolvedStorageRoot {
                owner: StorageOwner::CurrentUser,
                directory: real_root,
            },
            RealStorageBackend,
            DiagnosticPolicy::default(),
        );
        coordinator.ensure_root().unwrap();
        let external = fixture.join("external-settings.json");
        fs::write(&external, r#"{"outside":true}"#).unwrap();
        let component = coordinator.component_path(UserStorageComponent::Settings);
        symlink(&external, &component).unwrap();

        let load_failure = coordinator
            .load_json::<serde_json::Value>(UserStorageComponent::Settings)
            .unwrap_err();
        assert_eq!(load_failure.operation, PersistenceOperation::Permissions);
        assert_eq!(load_failure.code, PersistenceFailureCode::InvalidPath);
        let write_failure = coordinator
            .write_json(
                UserStorageComponent::Settings,
                &serde_json::json!({"inside": true}),
            )
            .unwrap_err();
        assert_eq!(write_failure.operation, PersistenceOperation::Permissions);
        assert_eq!(write_failure.code, PersistenceFailureCode::InvalidPath);
        assert_eq!(fs::read_to_string(external).unwrap(), r#"{"outside":true}"#);

        fs::remove_file(&component).unwrap();
        fs::write(&component, r#"{"public":true}"#).unwrap();
        fs::set_permissions(&component, fs::Permissions::from_mode(0o644)).unwrap();
        let migrated = coordinator
            .load_json::<serde_json::Value>(UserStorageComponent::Settings)
            .unwrap()
            .unwrap();
        assert_eq!(migrated, serde_json::json!({"public": true}));
        assert_eq!(
            fs::metadata(&component).unwrap().permissions().mode() & 0o777,
            0o600
        );

        fs::remove_dir_all(fixture).unwrap();
    }
}
