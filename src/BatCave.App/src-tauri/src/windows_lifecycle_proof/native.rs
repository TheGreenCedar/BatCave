use crate::windows_lifecycle_proof_contract::{
    Candidate, DesktopCollectorRuntimeObservation, DesktopFileObservation,
    DesktopProcessObservation, DesktopServiceProcessObservation, EvidenceReceipt,
    EvidenceRootIdentity, Observation, ProofPlan,
};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::ffi::{c_void, OsStr, OsString};
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::mem::{size_of, zeroed};
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::os::windows::fs::{MetadataExt, OpenOptionsExt};
use std::os::windows::io::AsRawHandle;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::ptr::{null, null_mut};
use std::time::{Duration, Instant};
use windows_sys::Win32::Foundation::{
    CloseHandle, GetLastError, LocalFree, ERROR_BROKEN_PIPE, ERROR_CANCELLED, ERROR_FILE_NOT_FOUND,
    ERROR_INSUFFICIENT_BUFFER, ERROR_IO_PENDING, ERROR_NO_MORE_FILES, ERROR_NO_TOKEN,
    ERROR_PATH_NOT_FOUND, ERROR_PIPE_BUSY, ERROR_PIPE_CONNECTED, ERROR_SERVICE_DOES_NOT_EXIST,
    ERROR_SUCCESS, GENERIC_READ, GENERIC_WRITE, HANDLE, INVALID_HANDLE_VALUE, TRUE, WAIT_OBJECT_0,
    WAIT_TIMEOUT,
};
use windows_sys::Win32::Security::Authorization::{
    ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
};
use windows_sys::Win32::Security::Cryptography::{
    BCryptGenRandom, BCRYPT_USE_SYSTEM_PREFERRED_RNG,
};
use windows_sys::Win32::Security::{
    GetLengthSid, GetTokenInformation, IsValidSid, TokenElevation, TokenElevationType,
    TokenElevationTypeFull, TokenSessionId, TokenStatistics, TokenUser, SECURITY_ATTRIBUTES,
    TOKEN_ELEVATION, TOKEN_ELEVATION_TYPE, TOKEN_QUERY, TOKEN_STATISTICS, TOKEN_USER,
};
use windows_sys::Win32::Storage::FileSystem::{
    CreateFileW, GetFileInformationByHandle, GetFinalPathNameByHandleW, BY_HANDLE_FILE_INFORMATION,
    FILE_ATTRIBUTE_REPARSE_POINT, FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_FIRST_PIPE_INSTANCE,
    FILE_FLAG_OPEN_REPARSE_POINT, FILE_FLAG_OVERLAPPED, FILE_READ_ATTRIBUTES, FILE_SHARE_DELETE,
    FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING, PIPE_ACCESS_DUPLEX, SYNCHRONIZE,
};
use windows_sys::Win32::System::Com::CoTaskMemFree;
use windows_sys::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS,
};
use windows_sys::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JobObjectBasicAccountingInformation,
    JobObjectBasicProcessIdList, JobObjectExtendedLimitInformation, QueryInformationJobObject,
    SetInformationJobObject, TerminateJobObject, JOBOBJECT_BASIC_ACCOUNTING_INFORMATION,
    JOBOBJECT_BASIC_PROCESS_ID_LIST, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
    JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
};
use windows_sys::Win32::System::Pipes::{
    ConnectNamedPipe, CreateNamedPipeW, GetNamedPipeClientProcessId, GetNamedPipeServerProcessId,
    SetNamedPipeHandleState, WaitNamedPipeW, PIPE_READMODE_BYTE, PIPE_REJECT_REMOTE_CLIENTS,
    PIPE_TYPE_BYTE, PIPE_WAIT,
};
use windows_sys::Win32::System::Registry::{
    RegCloseKey, RegOpenKeyExW, RegQueryValueExW, HKEY, HKEY_LOCAL_MACHINE, KEY_READ,
    KEY_WOW64_32KEY, KEY_WOW64_64KEY, REG_EXPAND_SZ, REG_SZ,
};
use windows_sys::Win32::System::Services::{
    CloseServiceHandle, OpenSCManagerW, OpenServiceW, QueryServiceStatusEx, SC_HANDLE,
    SC_MANAGER_CONNECT, SC_STATUS_PROCESS_INFO, SERVICE_QUERY_STATUS, SERVICE_RUNNING,
    SERVICE_STATUS_PROCESS,
};
use windows_sys::Win32::System::SystemInformation::{
    GetSystemDirectoryW, GetSystemWindowsDirectoryW,
};
use windows_sys::Win32::System::Threading::{
    CreateProcessW, GetCurrentProcess, GetCurrentThread, GetExitCodeProcess, GetProcessId,
    GetProcessTimes, OpenProcess, OpenProcessToken, OpenThreadToken, QueryFullProcessImageNameW,
    ResumeThread, TerminateProcess, WaitForSingleObject, CREATE_SUSPENDED,
    CREATE_UNICODE_ENVIRONMENT, PROCESS_INFORMATION, PROCESS_QUERY_LIMITED_INFORMATION,
    STARTUPINFOW,
};
use windows_sys::Win32::System::IO::{CancelIoEx, GetOverlappedResult, OVERLAPPED};
use windows_sys::Win32::UI::Shell::{
    FOLDERID_LocalAppData, GetUserProfileDirectoryW, SHGetKnownFolderPath, ShellExecuteExW,
    SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    GetForegroundWindow, GetWindowThreadProcessId, SW_HIDE,
};

const PIPE_PREFIX: &str = r"\\.\pipe\BatCaveLifecycleProof.v1.";
const PIPE_BUFFER_SIZE: u32 = 64 * 1024;
const MAX_FRAME_SIZE: usize = 64 * 1024;
const SERVICE_NAME: &str = "BatCaveCollector";
const INSTALL_ROOT: &str = r"C:\Program Files\BatCave Monitor";
const MONITOR_PATH: &str = r"C:\Program Files\BatCave Monitor\batcave-monitor.exe";
const SERVICE_PATH: &str = r"C:\Program Files\BatCave Monitor\batcave-collector-service.exe";
const UNINSTALLER_PATH: &str = r"C:\Program Files\BatCave Monitor\uninstall.exe";
const LEGACY_CLI_PATH: &str = r"C:\Program Files\BatCave Monitor\batcave-monitor-cli.exe";
const UNINSTALL_KEY: &str = r"SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\BatCave Monitor";
const INSTALL_LOCATION_VALUE: &str = "InstallLocation";
const EVIDENCE_ROOT_PREFIX: &str = r"C:\ProgramData\BatCaveLifecycleProof-v1-";
const PROCESS_TREE_SETTLEMENT_TIMEOUT: Duration = Duration::from_secs(30);
const PROCESS_TREE_SETTLEMENT_TIMEOUT_MS: u32 = 30_000;
const PROCESS_TREE_POLL_INTERVAL: Duration = Duration::from_millis(25);
const DESKTOP_PROCESS_TIMEOUT: Duration = Duration::from_secs(30);
const DESKTOP_PROCESS_STABLE_INTERVAL: Duration = Duration::from_millis(50);
const DESKTOP_PROCESS_STABLE_SNAPSHOTS: usize = 3;
const WINDOWS_PATH_BUFFER_SIZE: usize = 32_768;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct FileIdentity {
    pub(crate) volume_serial: u32,
    pub(crate) file_index: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct FileSnapshot {
    pub(crate) size: u64,
    pub(crate) sha256: String,
    pub(crate) identity: FileIdentity,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ServiceSnapshot {
    pub(crate) state: u32,
    pub(crate) process_id: u32,
    pub(crate) process_started_at_100ns: Option<u64>,
    pub(crate) win32_exit_code: u32,
    pub(crate) service_specific_exit_code: u32,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum RegistryView {
    Registry32,
    Registry64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RegistrySnapshot {
    pub(crate) view: RegistryView,
    pub(crate) install_location: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct DirectorySnapshot {
    pub(crate) identity: FileIdentity,
    pub(crate) final_path: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ProcessSnapshot {
    pub(crate) process_id: u32,
    pub(crate) parent_process_id: u32,
    pub(crate) executable_name: String,
    pub(crate) executable_path: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PreflightSnapshot {
    pub(crate) service: Observation<ServiceSnapshot>,
    pub(crate) install_root: Observation<DirectorySnapshot>,
    pub(crate) monitor: Observation<FileSnapshot>,
    pub(crate) service_binary: Observation<FileSnapshot>,
    pub(crate) uninstaller: Observation<FileSnapshot>,
    pub(crate) legacy_cli: Observation<FileSnapshot>,
    pub(crate) uninstall_registry: Observation<RegistrySnapshot>,
    pub(crate) product_processes: Observation<Vec<ProcessSnapshot>>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ElevatedMachineSnapshot {
    pub(crate) machine: PreflightSnapshot,
    pub(crate) product_data_root: Observation<DirectorySnapshot>,
    pub(crate) service_data_root: Observation<DirectorySnapshot>,
    pub(crate) current_user_data_root: Observation<DirectorySnapshot>,
    pub(crate) installed_boundaries:
        Observation<crate::collector_service::windows_provisioner::InstalledBoundariesForProof>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ParentCurrentUserAuthority {
    pub(crate) user_sid: String,
    pub(crate) session_id: u32,
    pub(crate) logon_luid: LogonLuid,
    pub(crate) profile: DirectorySnapshot,
    pub(crate) local_app_data: DirectorySnapshot,
    pub(crate) resolved_data_root: String,
    pub(crate) data_root: Observation<DirectorySnapshot>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct LogonLuid {
    pub(crate) low_part: u32,
    pub(crate) high_part: i32,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ParentCurrentUserObjects {
    pub(crate) settings: Observation<FileSnapshot>,
    pub(crate) cache: Observation<FileSnapshot>,
    pub(crate) diagnostics: Observation<FileSnapshot>,
}

pub(crate) struct ParentCurrentUserAuthorityGuard {
    authority: ParentCurrentUserAuthority,
    token: OwnedHandle,
    profile: OwnedDirectory,
    local_app_data: OwnedDirectory,
    data_root: OwnedDirectory,
}

pub(crate) struct ParentCurrentUserObjectsGuard {
    authority: ParentCurrentUserObjects,
    settings: Option<ParentObservedFile>,
    cache: Option<ParentObservedFile>,
    diagnostics: Option<ParentObservedFile>,
}

struct OwnedDirectory {
    path: PathBuf,
    handle: File,
    identity: FileIdentity,
}

struct ParentObservedFile {
    path: PathBuf,
    parent_path: PathBuf,
    parent_identity: FileIdentity,
    handle: File,
    size: u64,
    sha256: [u8; 32],
    identity: FileIdentity,
}

pub(crate) struct OwnedFile {
    path: PathBuf,
    handle: File,
    size: u64,
    sha256: [u8; 32],
    identity: FileIdentity,
}

pub(crate) struct VerifiedEvidenceFile {
    receipt: EvidenceReceipt,
    file: OwnedFile,
}

impl VerifiedEvidenceFile {
    pub(crate) fn receipt(&self) -> &EvidenceReceipt {
        &self.receipt
    }

    pub(crate) fn identity(&self) -> FileIdentity {
        self.file.identity()
    }

    pub(crate) fn read_all_exact(&self, label: &str) -> Result<Vec<u8>, String> {
        self.file.read_all_exact(label)
    }

    pub(crate) fn revalidate(&self) -> Result<(), String> {
        self.file.revalidate()
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields, rename_all = "snake_case", tag = "kind")]
pub(crate) enum ProcessTerminal {
    Exited { exit_code: u32 },
    TimedOut,
    SupervisionFailed { reason: String },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ProcessTerminalSnapshot {
    pub(crate) process_id: u32,
    pub(crate) terminal: ProcessTerminal,
    pub(crate) active_processes: Observation<u32>,
}

pub(crate) struct SettledProcessOutcome {
    pub(crate) terminal: ProcessTerminalSnapshot,
}

#[derive(Debug)]
pub(crate) enum ExecuteFailure {
    NotStarted(String),
    SettlementUnproven {
        reason: String,
        terminal: ProcessTerminalSnapshot,
    },
}

struct FixedLaunchContext {
    environment: Vec<u16>,
    current_directory: Vec<u16>,
}

impl FixedLaunchContext {
    fn for_evidence_root(evidence: &ProtectedEvidenceRoot) -> Result<Self, String> {
        let system = system_directory()?;
        let windows = windows_directory()?;
        let canonical_evidence =
            canonical_real_directory(evidence.root(), "child_working_directory")?;
        if !canonical_evidence
            .to_string_lossy()
            .eq_ignore_ascii_case(&evidence.root().to_string_lossy())
        {
            return Err("lifecycle_child_working_directory_changed".to_string());
        }
        Self::from_paths(&system, &windows, &canonical_evidence)
    }

    fn from_paths(system: &Path, windows: &Path, evidence: &Path) -> Result<Self, String> {
        let command_processor = system.join("cmd.exe");
        let metadata = fs::symlink_metadata(&command_processor)
            .map_err(|_| "lifecycle_command_processor_metadata_failed".to_string())?;
        if !metadata.is_file() || metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            return Err("lifecycle_command_processor_type_invalid".to_string());
        }
        let environment =
            build_fixed_environment_block(system, windows, evidence, &command_processor)?;
        Ok(Self {
            environment,
            current_directory: wide(evidence.as_os_str()),
        })
    }

    fn for_desktop(parent_token: &TokenEvidence) -> Result<Self, String> {
        let current_directory =
            canonical_real_directory(Path::new(INSTALL_ROOT), "desktop_working_directory")?;
        if !current_directory
            .to_string_lossy()
            .eq_ignore_ascii_case(INSTALL_ROOT)
        {
            return Err("lifecycle_desktop_working_directory_changed".to_string());
        }
        let (profile, local_app_data) = current_user_directories(parent_token)?;
        let system = system_directory()?;
        let windows = windows_directory()?;
        Ok(Self {
            environment: build_desktop_environment_block(
                &profile,
                &local_app_data,
                &system,
                &windows,
            )?,
            current_directory: wide(current_directory.as_os_str()),
        })
    }
}

impl OwnedDirectory {
    fn open(path: &Path, label: &str) -> Result<Self, String> {
        Self::open_with_delete_sharing(path, label, true)
    }

    fn open_without_delete_sharing(path: &Path, label: &str) -> Result<Self, String> {
        Self::open_with_delete_sharing(path, label, false)
    }

    fn open_with_delete_sharing(
        path: &Path,
        label: &str,
        share_delete: bool,
    ) -> Result<Self, String> {
        let (normalized, mut component_handles) = open_no_follow_directory_components(path, label)?;
        let handle = if share_delete {
            open_directory_handle(
                &normalized,
                label,
                FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            )?
        } else {
            component_handles
                .pop()
                .ok_or_else(|| format!("lifecycle_{label}_component_missing"))?
        };
        let metadata = handle
            .metadata()
            .map_err(|_| format!("lifecycle_{label}_metadata_failed"))?;
        if !metadata.is_dir() || metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            return Err(format!("lifecycle_{label}_type_invalid"));
        }
        let information =
            file_information(&handle).map_err(|_| format!("lifecycle_{label}_identity_failed"))?;
        let final_path =
            final_path(&handle).map_err(|_| format!("lifecycle_{label}_path_failed"))?;
        if !paths_equal(&final_path, &normalized) {
            return Err(format!("lifecycle_{label}_path_changed"));
        }
        Ok(Self {
            path: final_path,
            handle,
            identity: information.identity,
        })
    }

    fn snapshot(&self) -> DirectorySnapshot {
        DirectorySnapshot {
            identity: self.identity,
            final_path: self.path.to_string_lossy().into_owned(),
        }
    }

    fn revalidate(&self) -> Result<(), String> {
        let metadata = self
            .handle
            .metadata()
            .map_err(|_| "lifecycle_parent_user_directory_metadata_failed".to_string())?;
        let information = file_information(&self.handle)
            .map_err(|_| "lifecycle_parent_user_directory_identity_failed".to_string())?;
        let path = final_path(&self.handle)
            .map_err(|_| "lifecycle_parent_user_directory_path_failed".to_string())?;
        let reopened = Self::open(&self.path, "parent_user_directory_reopen")?;
        if !metadata.is_dir()
            || metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
            || information.identity != self.identity
            || path != self.path
            || reopened.identity != self.identity
            || reopened.path != self.path
        {
            return Err("lifecycle_parent_user_directory_changed".to_string());
        }
        Ok(())
    }
}

impl ParentObservedFile {
    fn open(path: &Path, expected_parent: &OwnedDirectory, label: &str) -> Result<Self, String> {
        let requested_parent = path
            .parent()
            .ok_or_else(|| format!("lifecycle_{label}_parent_missing"))?;
        if !paths_equal(requested_parent, &expected_parent.path) {
            return Err(format!("lifecycle_{label}_parent_changed"));
        }
        let mut handle = OpenOptions::new()
            .read(true)
            .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
            .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
            .open(path)
            .map_err(|_| format!("lifecycle_{label}_open_failed"))?;
        let metadata = handle
            .metadata()
            .map_err(|_| format!("lifecycle_{label}_metadata_failed"))?;
        if !metadata.is_file() || metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            return Err(format!("lifecycle_{label}_type_invalid"));
        }
        let information =
            file_information(&handle).map_err(|_| format!("lifecycle_{label}_identity_failed"))?;
        if information.number_of_links != 1 {
            return Err(format!("lifecycle_{label}_link_count_invalid"));
        }
        let sha256 =
            digest_handle(&mut handle).map_err(|_| format!("lifecycle_{label}_hash_failed"))?;
        let path = final_path(&handle).map_err(|_| format!("lifecycle_{label}_path_failed"))?;
        if !path
            .parent()
            .is_some_and(|observed| paths_equal(observed, &expected_parent.path))
        {
            return Err(format!("lifecycle_{label}_parent_changed"));
        }
        Ok(Self {
            path,
            parent_path: expected_parent.path.clone(),
            parent_identity: expected_parent.identity,
            handle,
            size: metadata.len(),
            sha256,
            identity: information.identity,
        })
    }

    fn snapshot(&self) -> FileSnapshot {
        FileSnapshot {
            size: self.size,
            sha256: hex_digest(&self.sha256),
            identity: self.identity,
        }
    }

    fn revalidate(&self) -> Result<(), String> {
        let mut duplicate = self
            .handle
            .try_clone()
            .map_err(|_| "lifecycle_parent_user_file_clone_failed".to_string())?;
        let metadata = duplicate
            .metadata()
            .map_err(|_| "lifecycle_parent_user_file_metadata_failed".to_string())?;
        let information = file_information(&duplicate)
            .map_err(|_| "lifecycle_parent_user_file_identity_failed".to_string())?;
        let path = final_path(&duplicate)
            .map_err(|_| "lifecycle_parent_user_file_path_failed".to_string())?;
        let sha256 = digest_handle(&mut duplicate)
            .map_err(|_| "lifecycle_parent_user_file_hash_failed".to_string())?;
        let parent = OwnedDirectory::open_without_delete_sharing(
            &self.parent_path,
            "parent_user_file_parent_reopen",
        )?;
        if parent.identity != self.parent_identity {
            return Err("lifecycle_parent_user_file_parent_changed".to_string());
        }
        let reopened = Self::open(&self.path, &parent, "parent_user_file_reopen")?;
        if metadata.len() != self.size
            || information.identity != self.identity
            || information.number_of_links != 1
            || path != self.path
            || sha256 != self.sha256
            || reopened.identity != self.identity
            || reopened.size != self.size
            || reopened.sha256 != self.sha256
            || reopened.path != self.path
        {
            return Err("lifecycle_parent_user_file_changed".to_string());
        }
        Ok(())
    }
}

impl ParentCurrentUserAuthorityGuard {
    pub(crate) fn authority(&self) -> &ParentCurrentUserAuthority {
        &self.authority
    }

    pub(crate) fn revalidate(&self) -> Result<(), String> {
        validate_parent_current_user_authority(&self.authority)?;
        let token = standard_token_evidence()?;
        let retained_token = token_evidence(&self.token)?;
        if token.sid_string != self.authority.user_sid
            || token.session_id != self.authority.session_id
            || token.logon_luid != self.authority.logon_luid
            || retained_token != token
        {
            return Err("lifecycle_parent_user_token_changed".to_string());
        }
        let profile_path = profile_directory_for_token(&self.token)?;
        if !profile_path
            .to_string_lossy()
            .eq_ignore_ascii_case(&self.authority.profile.final_path)
        {
            return Err("lifecycle_parent_user_profile_changed".to_string());
        }
        self.profile.revalidate()?;
        let local_app_data = local_app_data_for_token(&self.token)?;
        if !local_app_data
            .to_string_lossy()
            .eq_ignore_ascii_case(&self.authority.local_app_data.final_path)
        {
            return Err("lifecycle_parent_local_app_data_changed".to_string());
        }
        self.local_app_data.revalidate()?;
        self.data_root.revalidate()?;
        Ok(())
    }
}

impl ParentCurrentUserObjectsGuard {
    pub(crate) fn authority(&self) -> &ParentCurrentUserObjects {
        &self.authority
    }

    pub(crate) fn revalidate(&self) -> Result<(), String> {
        for file in [&self.settings, &self.cache, &self.diagnostics]
            .into_iter()
            .flatten()
        {
            file.revalidate()?;
        }
        Ok(())
    }
}

impl OwnedFile {
    pub(crate) fn open(
        path: &Path,
        expected_size: u64,
        expected_sha256: &str,
        label: &str,
    ) -> Result<Self, String> {
        let file = Self::open_unchecked(path, label)?;
        if file.size != expected_size || file.sha256_hex() != expected_sha256 {
            return Err(format!("lifecycle_{label}_identity_mismatch"));
        }
        Ok(file)
    }

    pub(crate) fn open_unchecked(path: &Path, label: &str) -> Result<Self, String> {
        let mut handle = OpenOptions::new()
            .read(true)
            .share_mode(FILE_SHARE_READ)
            .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
            .open(path)
            .map_err(|_| format!("lifecycle_{label}_open_failed"))?;
        let metadata = handle
            .metadata()
            .map_err(|_| format!("lifecycle_{label}_metadata_failed"))?;
        if !metadata.is_file()
            || metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
            || metadata.len() == 0
        {
            return Err(format!("lifecycle_{label}_type_invalid"));
        }
        let information =
            file_information(&handle).map_err(|_| format!("lifecycle_{label}_identity_failed"))?;
        if information.number_of_links != 1 {
            return Err(format!("lifecycle_{label}_link_count_invalid"));
        }
        let sha256 =
            digest_handle(&mut handle).map_err(|_| format!("lifecycle_{label}_hash_failed"))?;
        let final_path =
            final_path(&handle).map_err(|_| format!("lifecycle_{label}_final_path_failed"))?;
        Ok(Self {
            path: final_path,
            handle,
            size: metadata.len(),
            sha256,
            identity: information.identity,
        })
    }

    pub(crate) fn open_current_executable() -> Result<Self, String> {
        let path =
            std::env::current_exe().map_err(|_| "lifecycle_controller_path_failed".to_string())?;
        Self::open_unchecked(&path, "controller")
    }

    pub(crate) fn identity(&self) -> FileIdentity {
        self.identity
    }

    fn transport_identity(&self) -> [u8; 32] {
        transport_file_identity(self.identity)
    }

    pub(crate) fn require_under(&self, root: &Path, label: &str) -> Result<(), String> {
        if self.path.starts_with(root) {
            Ok(())
        } else {
            Err(format!("lifecycle_{label}_final_path_outside_repo"))
        }
    }

    pub(crate) fn sha256_hex(&self) -> String {
        hex_digest(&self.sha256)
    }

    pub(crate) fn revalidate(&self) -> Result<(), String> {
        let mut duplicate = self
            .handle
            .try_clone()
            .map_err(|_| "lifecycle_owned_file_clone_failed".to_string())?;
        let metadata = duplicate
            .metadata()
            .map_err(|_| "lifecycle_owned_file_metadata_failed".to_string())?;
        let information = file_information(&duplicate)
            .map_err(|_| "lifecycle_owned_file_identity_failed".to_string())?;
        if metadata.len() != self.size
            || information.identity != self.identity
            || information.number_of_links != 1
            || digest_handle(&mut duplicate)
                .map_err(|_| "lifecycle_owned_file_hash_failed".to_string())?
                != self.sha256
        {
            return Err("lifecycle_owned_file_changed".to_string());
        }
        Ok(())
    }

    pub(crate) fn copy_to(&self, target: &Path, label: &str) -> Result<OwnedFile, String> {
        self.revalidate()?;
        let mut source = self
            .handle
            .try_clone()
            .map_err(|_| format!("lifecycle_{label}_source_clone_failed"))?;
        source
            .seek(SeekFrom::Start(0))
            .map_err(|_| format!("lifecycle_{label}_source_seek_failed"))?;
        let mut target_file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .share_mode(FILE_SHARE_READ)
            .open(target)
            .map_err(|_| format!("lifecycle_{label}_copy_create_failed"))?;
        std::io::copy(&mut source, &mut target_file)
            .and_then(|_| target_file.sync_all())
            .map_err(|_| format!("lifecycle_{label}_copy_write_failed"))?;
        drop(target_file);
        self.revalidate()?;
        OwnedFile::open(target, self.size, &self.sha256_hex(), label)
    }

    pub(crate) fn read_all_exact(&self, label: &str) -> Result<Vec<u8>, String> {
        self.revalidate()?;
        let mut source = self
            .handle
            .try_clone()
            .map_err(|_| format!("lifecycle_{label}_source_clone_failed"))?;
        source
            .seek(SeekFrom::Start(0))
            .map_err(|_| format!("lifecycle_{label}_source_seek_failed"))?;
        let capacity =
            usize::try_from(self.size).map_err(|_| format!("lifecycle_{label}_size_invalid"))?;
        let mut bytes = Vec::with_capacity(capacity);
        source
            .read_to_end(&mut bytes)
            .map_err(|_| format!("lifecycle_{label}_source_read_failed"))?;
        if bytes.len() != capacity {
            return Err(format!("lifecycle_{label}_source_read_incomplete"));
        }
        self.revalidate()?;
        Ok(bytes)
    }

    pub(crate) fn snapshot(&self) -> FileSnapshot {
        FileSnapshot {
            size: self.size,
            sha256: self.sha256_hex(),
            identity: self.identity,
        }
    }

    pub(crate) fn execute(
        &self,
        evidence: &ProtectedEvidenceRoot,
        fixed_arguments: &str,
        timeout: Duration,
        label: &str,
    ) -> Result<SettledProcessOutcome, ExecuteFailure> {
        self.revalidate().map_err(ExecuteFailure::NotStarted)?;
        let launch =
            FixedLaunchContext::for_evidence_root(evidence).map_err(ExecuteFailure::NotStarted)?;
        let job = Job::new(label).map_err(ExecuteFailure::NotStarted)?;
        let child = SuspendedChild::spawn(&self.path, fixed_arguments, &launch, label)
            .map_err(ExecuteFailure::NotStarted)?;
        if unsafe { AssignProcessToJobObject(job.raw(), child.process.raw()) } == 0 {
            let primary = format!("lifecycle_{label}_job_assignment_failed");
            return match child.settle_unassigned(label) {
                Ok(()) => Err(ExecuteFailure::NotStarted(primary)),
                Err(settlement) => Err(ExecuteFailure::SettlementUnproven {
                    reason: combined_failure(&primary, &settlement),
                    terminal: child.unassigned_terminal(primary),
                }),
            };
        }
        if unsafe { ResumeThread(child.thread.raw()) } == u32::MAX {
            let primary = format!("lifecycle_{label}_resume_failed");
            let terminal = child.terminal_snapshot(
                &job,
                ProcessTerminal::SupervisionFailed {
                    reason: primary.clone(),
                },
            );
            return match job.terminate_and_settle(PROCESS_TREE_SETTLEMENT_TIMEOUT, label) {
                Ok(()) => Err(ExecuteFailure::NotStarted(primary)),
                Err(settlement) => Err(ExecuteFailure::SettlementUnproven {
                    reason: combined_failure(&primary, &settlement),
                    terminal,
                }),
            };
        }

        let deadline = Instant::now() + timeout;
        let primary_wait = match wait_handle_until(child.process.raw(), deadline, label) {
            Ok(wait) => wait,
            Err(reason) => {
                return settle_failed_process(
                    &job,
                    child.terminal_snapshot(&job, ProcessTerminal::SupervisionFailed { reason }),
                    label,
                );
            }
        };
        if primary_wait == WAIT_TIMEOUT {
            return settle_failed_process(
                &job,
                child.terminal_snapshot(&job, ProcessTerminal::TimedOut),
                label,
            );
        }
        if primary_wait != WAIT_OBJECT_0 {
            let reason = format!("lifecycle_{label}_primary_wait_failed");
            return settle_failed_process(
                &job,
                child.terminal_snapshot(&job, ProcessTerminal::SupervisionFailed { reason }),
                label,
            );
        }
        let mut exit_code = 0;
        if unsafe { GetExitCodeProcess(child.process.raw(), &mut exit_code) } == 0 {
            let reason = format!("lifecycle_{label}_exit_query_failed");
            return settle_failed_process(
                &job,
                child.terminal_snapshot(&job, ProcessTerminal::SupervisionFailed { reason }),
                label,
            );
        }
        if exit_code != 0 {
            return settle_failed_process(
                &job,
                child.terminal_snapshot(&job, ProcessTerminal::Exited { exit_code }),
                label,
            );
        }
        match job.wait_for_zero(deadline) {
            Ok(true) => Ok(SettledProcessOutcome {
                terminal: ProcessTerminalSnapshot {
                    process_id: child.process_id,
                    terminal: ProcessTerminal::Exited { exit_code },
                    active_processes: Observation::Present(0),
                },
            }),
            Ok(false) => settle_failed_process(
                &job,
                child.terminal_snapshot(&job, ProcessTerminal::TimedOut),
                label,
            ),
            Err(reason) => settle_failed_process(
                &job,
                child.terminal_snapshot(&job, ProcessTerminal::SupervisionFailed { reason }),
                label,
            ),
        }
    }
}

fn settle_failed_process(
    job: &Job,
    terminal: ProcessTerminalSnapshot,
    label: &str,
) -> Result<SettledProcessOutcome, ExecuteFailure> {
    match job.terminate_and_settle(PROCESS_TREE_SETTLEMENT_TIMEOUT, label) {
        Ok(()) => Ok(SettledProcessOutcome { terminal }),
        Err(settlement) => {
            let primary = terminal_failure(&terminal.terminal, label);
            Err(ExecuteFailure::SettlementUnproven {
                reason: combined_failure(&primary, &settlement),
                terminal,
            })
        }
    }
}

fn terminal_failure(terminal: &ProcessTerminal, label: &str) -> String {
    match terminal {
        ProcessTerminal::Exited { exit_code } => {
            format!("lifecycle_{label}_exit_code_{exit_code}")
        }
        ProcessTerminal::TimedOut => format!("lifecycle_{label}_timeout"),
        ProcessTerminal::SupervisionFailed { reason } => reason.clone(),
    }
}

fn combined_failure(primary: &str, settlement: &str) -> String {
    format!("{primary}|{settlement}")
}

struct Job(OwnedHandle);

impl Job {
    fn new(label: &str) -> Result<Self, String> {
        let handle = unsafe { CreateJobObjectW(null(), null()) };
        if handle.is_null() {
            return Err(format!("lifecycle_{label}_job_create_failed"));
        }
        let job = Self(OwnedHandle(handle));
        let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { zeroed() };
        limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        if unsafe {
            SetInformationJobObject(
                job.raw(),
                JobObjectExtendedLimitInformation,
                (&limits as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
                size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            )
        } == 0
        {
            return Err(format!("lifecycle_{label}_job_configure_failed"));
        }
        Ok(job)
    }

    fn raw(&self) -> HANDLE {
        self.0.raw()
    }

    fn active_processes(&self) -> Result<u32, String> {
        let mut accounting: JOBOBJECT_BASIC_ACCOUNTING_INFORMATION = unsafe { zeroed() };
        if unsafe {
            QueryInformationJobObject(
                self.raw(),
                JobObjectBasicAccountingInformation,
                (&mut accounting as *mut JOBOBJECT_BASIC_ACCOUNTING_INFORMATION).cast(),
                size_of::<JOBOBJECT_BASIC_ACCOUNTING_INFORMATION>() as u32,
                null_mut(),
            )
        } == 0
        {
            return Err("lifecycle_job_accounting_failed".to_string());
        }
        Ok(accounting.ActiveProcesses)
    }

    fn observe_active_processes(&self) -> Observation<u32> {
        match self.active_processes() {
            Ok(active) => Observation::Present(active),
            Err(reason) => Observation::Unknown(reason),
        }
    }

    fn process_ids(&self) -> Result<Vec<u32>, String> {
        let mut capacity = self.active_processes()?.max(1) as usize;
        loop {
            if capacity > 129 {
                return Err("lifecycle_desktop_process_tree_too_large".to_string());
            }
            let bytes = size_of::<JOBOBJECT_BASIC_PROCESS_ID_LIST>()
                + capacity.saturating_sub(1) * size_of::<usize>();
            let words = bytes.div_ceil(size_of::<usize>());
            let mut buffer = vec![0_usize; words];
            let list = buffer
                .as_mut_ptr()
                .cast::<JOBOBJECT_BASIC_PROCESS_ID_LIST>();
            let queried = unsafe {
                QueryInformationJobObject(
                    self.raw(),
                    JobObjectBasicProcessIdList,
                    list.cast(),
                    u32::try_from(bytes)
                        .map_err(|_| "lifecycle_desktop_job_process_size_invalid".to_string())?,
                    null_mut(),
                )
            };
            let assigned = unsafe { (*list).NumberOfAssignedProcesses as usize };
            let listed = unsafe { (*list).NumberOfProcessIdsInList as usize };
            if assigned > listed {
                capacity = assigned.max(capacity.saturating_mul(2)).max(1);
                continue;
            }
            if queried == 0 {
                return Err("lifecycle_desktop_job_process_query_failed".to_string());
            }
            let raw_ids =
                unsafe { std::slice::from_raw_parts((*list).ProcessIdList.as_ptr(), listed) };
            let mut ids = raw_ids
                .iter()
                .map(|process_id| {
                    u32::try_from(*process_id)
                        .map_err(|_| "lifecycle_desktop_job_process_id_invalid".to_string())
                })
                .collect::<Result<Vec<_>, _>>()?;
            ids.sort_unstable();
            ids.dedup();
            if ids.len() != listed || ids.is_empty() {
                return Err("lifecycle_desktop_job_process_list_invalid".to_string());
            }
            return Ok(ids);
        }
    }

    fn wait_for_zero(&self, deadline: Instant) -> Result<bool, String> {
        loop {
            if self.active_processes()? == 0 {
                return Ok(true);
            }
            let Some(remaining) = deadline.checked_duration_since(Instant::now()) else {
                return Ok(false);
            };
            std::thread::sleep(remaining.min(PROCESS_TREE_POLL_INTERVAL));
        }
    }

    fn terminate_and_settle(&self, timeout: Duration, label: &str) -> Result<(), String> {
        if self.active_processes()? == 0 {
            return Ok(());
        }
        if unsafe { TerminateJobObject(self.raw(), 124) } == 0 && self.active_processes()? != 0 {
            return Err(format!("lifecycle_{label}_job_terminate_failed"));
        }
        if !self.wait_for_zero(Instant::now() + timeout)? {
            return Err(format!("lifecycle_{label}_job_settlement_unproven"));
        }
        Ok(())
    }
}

struct SuspendedChild {
    process: OwnedHandle,
    thread: OwnedHandle,
    process_id: u32,
}

pub(super) struct DesktopProcess {
    job: Job,
    child: SuspendedChild,
    executable: OwnedFile,
    root: DesktopProcessObservation,
    parent_token: TokenEvidence,
    settled: bool,
}

#[derive(Debug)]
pub(super) struct DesktopLaunchFailure {
    pub(super) reason: String,
    pub(super) process_tree_settled: bool,
}

#[derive(Debug)]
pub(super) struct DesktopSettlementFailure {
    pub(super) reason: String,
    pub(super) process_tree_settled: bool,
}

struct RetainedDesktopProcess {
    observation: DesktopProcessObservation,
    process: OwnedHandle,
    image: OwnedFile,
}

pub(super) struct DesktopProcessTree {
    retained: Vec<RetainedDesktopProcess>,
    observations: Vec<DesktopProcessObservation>,
    webview_process_ids: Vec<u32>,
}

impl DesktopProcessTree {
    pub(super) fn observations(&self) -> Vec<DesktopProcessObservation> {
        self.observations.clone()
    }

    pub(super) fn webview_process_ids(&self) -> Vec<u32> {
        self.webview_process_ids.clone()
    }

    fn revalidate_after_settlement(&self) -> Result<(), String> {
        for retained in &self.retained {
            if unsafe { WaitForSingleObject(retained.process.raw(), 0) } != WAIT_OBJECT_0
                || process_started_at(retained.process.raw())?
                    != retained.observation.started_at_100ns
            {
                return Err("lifecycle_desktop_retained_process_unsettled".to_string());
            }
            retained.image.revalidate()?;
        }
        Ok(())
    }
}

impl DesktopProcess {
    pub(super) fn launch(expected_sha256: &str, label: &str) -> Result<Self, DesktopLaunchFailure> {
        let parent_token = standard_token_evidence().map_err(settled_launch_failure)?;
        let executable = OwnedFile::open_unchecked(Path::new(MONITOR_PATH), label)
            .map_err(settled_launch_failure)?;
        if executable.sha256_hex() != expected_sha256 {
            return Err(settled_launch_failure(format!(
                "lifecycle_{label}_identity_mismatch"
            )));
        }
        let launch =
            FixedLaunchContext::for_desktop(&parent_token).map_err(settled_launch_failure)?;
        let job = Job::new(label).map_err(settled_launch_failure)?;
        let child = SuspendedChild::spawn(&executable.path, "", &launch, label)
            .map_err(settled_launch_failure)?;
        if unsafe { AssignProcessToJobObject(job.raw(), child.process.raw()) } == 0 {
            let primary = format!("lifecycle_{label}_job_assignment_failed");
            return Err(launch_failure_after_settlement(
                primary,
                child.settle_unassigned(label),
            ));
        }
        let root = match observe_desktop_process(child.process_id, None, label, Some(&parent_token))
        {
            Ok(root) => root,
            Err(primary) => {
                return Err(launch_failure_after_settlement(
                    primary,
                    job.terminate_and_settle(PROCESS_TREE_SETTLEMENT_TIMEOUT, label),
                ));
            }
        };
        if unsafe { ResumeThread(child.thread.raw()) } == u32::MAX {
            let primary = format!("lifecycle_{label}_resume_failed");
            return Err(launch_failure_after_settlement(
                primary,
                job.terminate_and_settle(PROCESS_TREE_SETTLEMENT_TIMEOUT, label),
            ));
        }
        if !root.executable_path.eq_ignore_ascii_case(MONITOR_PATH)
            || root.executable_sha256 != expected_sha256
        {
            let primary = format!("lifecycle_{label}_launched_identity_invalid");
            return Err(launch_failure_after_settlement(
                primary,
                job.terminate_and_settle(PROCESS_TREE_SETTLEMENT_TIMEOUT, label),
            ));
        }
        Ok(Self {
            job,
            child,
            executable,
            root,
            parent_token,
            settled: false,
        })
    }

    pub(super) fn process_id(&self) -> u32 {
        self.child.process_id
    }

    pub(super) fn observation(&self) -> DesktopProcessObservation {
        self.root.clone()
    }

    pub(super) fn current_job_process_ids(&self) -> Result<Vec<u32>, String> {
        self.job.process_ids()
    }

    pub(super) fn process_tree(&self) -> Result<DesktopProcessTree, String> {
        observe_stable_desktop_process_tree(&self.job, self.process_id(), &self.parent_token)
    }

    pub(super) fn executable_path(&self) -> &Path {
        &self.executable.path
    }

    pub(super) fn executable_transport_identity(&self) -> [u8; 32] {
        self.executable.transport_identity()
    }

    pub(super) fn wait_for_clean_exit(
        &mut self,
        retained_tree: Option<&DesktopProcessTree>,
    ) -> Result<u32, String> {
        let deadline = Instant::now() + DESKTOP_PROCESS_TIMEOUT;
        if wait_handle_until(self.child.process.raw(), deadline, "desktop")? != WAIT_OBJECT_0 {
            return Err("lifecycle_desktop_exit_timeout".to_string());
        }
        let mut exit_code = 0;
        if unsafe { GetExitCodeProcess(self.child.process.raw(), &mut exit_code) } == 0 {
            return Err("lifecycle_desktop_exit_query_failed".to_string());
        }
        if !self.job.wait_for_zero(deadline)? {
            return Err("lifecycle_desktop_descendants_unsettled".to_string());
        }
        self.executable.revalidate()?;
        if let Some(retained_tree) = retained_tree {
            retained_tree.revalidate_after_settlement()?;
        }
        self.settled = true;
        Ok(exit_code)
    }

    pub(super) fn terminate_and_settle(
        &mut self,
        label: &str,
    ) -> Result<(), DesktopSettlementFailure> {
        self.job
            .terminate_and_settle(PROCESS_TREE_SETTLEMENT_TIMEOUT, label)
            .map_err(|reason| DesktopSettlementFailure {
                reason,
                process_tree_settled: false,
            })?;
        self.executable
            .revalidate()
            .map_err(|reason| DesktopSettlementFailure {
                reason,
                process_tree_settled: true,
            })?;
        self.settled = true;
        Ok(())
    }
}

fn settled_launch_failure(reason: String) -> DesktopLaunchFailure {
    DesktopLaunchFailure {
        reason,
        process_tree_settled: true,
    }
}

fn launch_failure_after_settlement(
    primary: String,
    settlement: Result<(), String>,
) -> DesktopLaunchFailure {
    match settlement {
        Ok(()) => settled_launch_failure(primary),
        Err(settlement) => DesktopLaunchFailure {
            reason: combined_failure(&primary, &settlement),
            process_tree_settled: false,
        },
    }
}

impl Drop for DesktopProcess {
    fn drop(&mut self) {
        if !self.settled {
            let _ = self
                .job
                .terminate_and_settle(PROCESS_TREE_SETTLEMENT_TIMEOUT, "desktop_drop");
        }
    }
}

impl SuspendedChild {
    fn spawn(
        path: &Path,
        fixed_arguments: &str,
        launch: &FixedLaunchContext,
        label: &str,
    ) -> Result<Self, String> {
        if fixed_arguments.contains('\0')
            || fixed_arguments.contains('\r')
            || fixed_arguments.contains('\n')
        {
            return Err(format!("lifecycle_{label}_arguments_invalid"));
        }
        let application = wide(path.as_os_str());
        let mut command_line = wide(OsStr::new(&format!(
            "\"{}\"{}{}",
            path.display(),
            if fixed_arguments.is_empty() { "" } else { " " },
            fixed_arguments
        )));
        let mut startup: STARTUPINFOW = unsafe { zeroed() };
        startup.cb = size_of::<STARTUPINFOW>() as u32;
        let mut information: PROCESS_INFORMATION = unsafe { zeroed() };
        if unsafe {
            CreateProcessW(
                application.as_ptr(),
                command_line.as_mut_ptr(),
                null(),
                null(),
                0,
                CREATE_SUSPENDED | CREATE_UNICODE_ENVIRONMENT,
                launch.environment.as_ptr().cast(),
                launch.current_directory.as_ptr(),
                &startup,
                &mut information,
            )
        } == 0
        {
            return Err(format!("lifecycle_{label}_process_create_failed"));
        }
        Ok(Self {
            process: OwnedHandle(information.hProcess),
            thread: OwnedHandle(information.hThread),
            process_id: information.dwProcessId,
        })
    }

    fn terminal_snapshot(&self, job: &Job, terminal: ProcessTerminal) -> ProcessTerminalSnapshot {
        ProcessTerminalSnapshot {
            process_id: self.process_id,
            terminal,
            active_processes: job.observe_active_processes(),
        }
    }

    fn unassigned_terminal(&self, reason: String) -> ProcessTerminalSnapshot {
        ProcessTerminalSnapshot {
            process_id: self.process_id,
            terminal: ProcessTerminal::SupervisionFailed { reason },
            active_processes: Observation::Unknown(
                "lifecycle_unassigned_process_job_state_unavailable".to_string(),
            ),
        }
    }

    fn settle_unassigned(&self, label: &str) -> Result<(), String> {
        if unsafe { TerminateProcess(self.process.raw(), 124) } == 0 {
            return Err(format!("lifecycle_{label}_unassigned_terminate_failed"));
        }
        if unsafe {
            WaitForSingleObject(
                self.process.raw(),
                duration_ms(
                    PROCESS_TREE_SETTLEMENT_TIMEOUT,
                    "lifecycle_unassigned_wait_invalid",
                )?,
            )
        } != WAIT_OBJECT_0
        {
            return Err(format!("lifecycle_{label}_unassigned_settlement_unproven"));
        }
        Ok(())
    }
}

pub(crate) struct ElevatedProcess {
    job: Option<Job>,
    handle: OwnedHandle,
    process_id: u32,
    started_at_100ns: u64,
    settled: bool,
}

impl ElevatedProcess {
    pub(crate) fn process_id(&self) -> u32 {
        self.process_id
    }

    pub(crate) fn started_at_100ns(&self) -> u64 {
        self.started_at_100ns
    }

    pub(crate) fn bind_to_parent_job(&mut self) -> Result<(), String> {
        if self.job.is_some() {
            return Err("lifecycle_worker_job_already_bound".to_string());
        }
        let job = Job::new("worker")?;
        if unsafe { AssignProcessToJobObject(job.raw(), self.handle.raw()) } == 0 {
            return Err("lifecycle_worker_job_assignment_failed".to_string());
        }
        self.job = Some(job);
        Ok(())
    }

    pub(crate) fn wait(&mut self, timeout: Duration) -> Result<u32, String> {
        if let Some(exit_code) = self.wait_without_termination(timeout)? {
            return Ok(exit_code);
        }
        self.terminate_and_settle()?;
        Err("lifecycle_worker_timeout".to_string())
    }

    pub(crate) fn wait_without_termination(
        &mut self,
        timeout: Duration,
    ) -> Result<Option<u32>, String> {
        let wait = unsafe {
            WaitForSingleObject(
                self.handle.raw(),
                duration_ms(timeout, "lifecycle_worker_wait_invalid")?,
            )
        };
        if wait == WAIT_TIMEOUT {
            return Ok(None);
        }
        if wait != WAIT_OBJECT_0 {
            return Err("lifecycle_worker_wait_failed".to_string());
        }
        let mut exit_code = 0;
        if unsafe { GetExitCodeProcess(self.handle.raw(), &mut exit_code) } == 0 {
            return Err("lifecycle_worker_exit_query_failed".to_string());
        }
        if let Some(job) = &self.job {
            if !job.wait_for_zero(Instant::now() + PROCESS_TREE_SETTLEMENT_TIMEOUT)? {
                return Err("lifecycle_worker_descendants_unsettled".to_string());
            }
        }
        self.settled = true;
        Ok(Some(exit_code))
    }

    pub(crate) fn terminate_and_settle(&mut self) -> Result<(), String> {
        if let Some(job) = &self.job {
            job.terminate_and_settle(PROCESS_TREE_SETTLEMENT_TIMEOUT, "worker")?;
        } else if unsafe { TerminateProcess(self.handle.raw(), 124) } == 0
            && unsafe { WaitForSingleObject(self.handle.raw(), 0) } != WAIT_OBJECT_0
        {
            return Err("lifecycle_worker_terminate_failed".to_string());
        }
        if unsafe {
            WaitForSingleObject(
                self.handle.raw(),
                duration_ms(
                    PROCESS_TREE_SETTLEMENT_TIMEOUT,
                    "lifecycle_worker_settlement_wait_invalid",
                )?,
            )
        } != WAIT_OBJECT_0
        {
            return Err("lifecycle_worker_timeout_unsettled".to_string());
        }
        self.settled = true;
        Ok(())
    }
}

impl Drop for ElevatedProcess {
    fn drop(&mut self) {
        if !self.settled {
            let _ = self.terminate_and_settle();
        }
    }
}

pub(crate) struct PipeConnection {
    handle: OwnedHandle,
    server: bool,
    connected: bool,
}

impl PipeConnection {
    pub(crate) fn connect(&mut self, timeout: Duration) -> Result<(), String> {
        if !self.server || self.connected {
            return Err("lifecycle_pipe_connect_state_invalid".to_string());
        }
        let timeout_ms = duration_ms(timeout, "lifecycle_pipe_connect_timeout_invalid")?;
        let mut pending = PendingOverlapped::new((), "lifecycle_pipe_connect_event_failed")?;
        let connected = unsafe { ConnectNamedPipe(self.handle.raw(), pending.as_mut_ptr()) };
        if connected == 0 {
            match unsafe { GetLastError() } {
                ERROR_PIPE_CONNECTED => {}
                ERROR_IO_PENDING => {
                    wait_overlapped(
                        self.handle.raw(),
                        pending,
                        timeout_ms,
                        "lifecycle_pipe_connect_timeout",
                    )?;
                }
                _ => return Err("lifecycle_pipe_connect_failed".to_string()),
            }
        }
        self.connected = true;
        Ok(())
    }

    pub(crate) fn write_json<T: Serialize>(&mut self, value: &T) -> Result<(), String> {
        let payload =
            serde_json::to_vec(value).map_err(|_| "lifecycle_pipe_serialize_failed".to_string())?;
        if payload.is_empty() || payload.len() > MAX_FRAME_SIZE {
            return Err("lifecycle_pipe_frame_size_invalid".to_string());
        }
        let length = u32::try_from(payload.len())
            .map_err(|_| "lifecycle_pipe_frame_size_invalid".to_string())?;
        self.write_all(&length.to_le_bytes(), Duration::from_secs(30))?;
        self.write_all(&payload, Duration::from_secs(30))
    }

    pub(crate) fn read_json<T: DeserializeOwned>(
        &mut self,
        timeout: Duration,
    ) -> Result<T, String> {
        let mut length = [0_u8; 4];
        self.read_exact(&mut length, timeout)?;
        let length = u32::from_le_bytes(length) as usize;
        if length == 0 || length > MAX_FRAME_SIZE {
            return Err("lifecycle_pipe_frame_size_invalid".to_string());
        }
        let mut payload = vec![0_u8; length];
        self.read_exact(&mut payload, timeout)?;
        if payload.starts_with(&[0xef, 0xbb, 0xbf]) {
            return Err("lifecycle_pipe_frame_bom_rejected".to_string());
        }
        serde_json::from_slice(&payload).map_err(|_| "lifecycle_pipe_json_invalid".to_string())
    }

    pub(crate) fn server_process_id(&self) -> Result<u32, String> {
        let mut process_id = 0;
        if unsafe { GetNamedPipeServerProcessId(self.handle.raw(), &mut process_id) } == 0
            || process_id == 0
        {
            return Err("lifecycle_pipe_server_pid_failed".to_string());
        }
        Ok(process_id)
    }

    pub(crate) fn client_process_id(&self) -> Result<u32, String> {
        let mut process_id = 0;
        if unsafe { GetNamedPipeClientProcessId(self.handle.raw(), &mut process_id) } == 0
            || process_id == 0
        {
            return Err("lifecycle_pipe_client_pid_failed".to_string());
        }
        Ok(process_id)
    }

    fn write_all(&mut self, mut bytes: &[u8], timeout: Duration) -> Result<(), String> {
        let deadline = Instant::now() + timeout;
        while !bytes.is_empty() {
            let remaining = deadline
                .checked_duration_since(Instant::now())
                .ok_or_else(|| "lifecycle_pipe_write_timeout".to_string())?;
            let written = overlapped_write(self.handle.raw(), bytes, remaining)?;
            if written == 0 {
                return Err("lifecycle_pipe_write_zero".to_string());
            }
            bytes = &bytes[written..];
        }
        Ok(())
    }

    fn read_exact(&mut self, mut bytes: &mut [u8], timeout: Duration) -> Result<(), String> {
        let deadline = Instant::now() + timeout;
        while !bytes.is_empty() {
            let remaining = deadline
                .checked_duration_since(Instant::now())
                .ok_or_else(|| "lifecycle_pipe_read_timeout".to_string())?;
            let read = overlapped_read(self.handle.raw(), bytes, remaining)?;
            if read == 0 {
                return Err("lifecycle_pipe_closed".to_string());
            }
            bytes = &mut bytes[read..];
        }
        Ok(())
    }
}

pub(crate) struct ProtectedEvidenceRoot {
    root: PathBuf,
    identity: EvidenceRootIdentity,
    _handle: OwnedHandle,
}

impl ProtectedEvidenceRoot {
    pub(crate) fn root(&self) -> &Path {
        &self.root
    }

    pub(crate) fn identity(&self) -> EvidenceRootIdentity {
        self.identity
    }

    pub(crate) fn write_json_new<T: Serialize>(
        &self,
        name: &str,
        value: &T,
    ) -> Result<EvidenceReceipt, String> {
        let payload = serde_json::to_vec_pretty(value)
            .map_err(|_| "lifecycle_evidence_serialize_failed".to_string())?;
        self.write_bytes_new(name, &payload)
    }

    pub(crate) fn write_bytes_new(
        &self,
        name: &str,
        payload: &[u8],
    ) -> Result<EvidenceReceipt, String> {
        if !valid_evidence_name(name) {
            return Err("lifecycle_evidence_name_invalid".to_string());
        }
        let path = self.root.join(name);
        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .share_mode(FILE_SHARE_READ)
            .open(&path)
            .map_err(|_| "lifecycle_evidence_create_failed".to_string())?;
        if file
            .write_all(payload)
            .and_then(|_| file.sync_all())
            .is_err()
        {
            drop(file);
            return match fs::remove_file(path) {
                Ok(()) => Err("lifecycle_evidence_write_failed".to_string()),
                Err(_) => Err("lifecycle_evidence_write_cleanup_failed".to_string()),
            };
        }
        Ok(evidence_receipt(name, payload))
    }
}

pub(crate) fn require_standard_token() -> Result<(), String> {
    standard_token_evidence().map(|_| ())
}

pub(crate) fn require_elevated_token() -> Result<(), String> {
    if current_process_elevated()? {
        Ok(())
    } else {
        Err("lifecycle_worker_requires_elevation".to_string())
    }
}

pub(crate) fn current_process_elevated() -> Result<bool, String> {
    Ok(current_token()?.elevated)
}

fn standard_token_evidence() -> Result<TokenEvidence, String> {
    standard_primary_token().map(|(_, token)| token)
}

fn standard_primary_token() -> Result<(OwnedHandle, TokenEvidence), String> {
    require_no_thread_token()?;
    let (handle, token) = current_primary_token()?;
    if token.elevated
        || token.elevation_type == TokenElevationTypeFull
        || token.session_id == 0
        || token.sid.is_empty()
    {
        Err("lifecycle_parent_must_use_standard_token".to_string())
    } else {
        Ok((handle, token))
    }
}

fn require_no_thread_token() -> Result<(), String> {
    let mut token = null_mut();
    if unsafe { OpenThreadToken(GetCurrentThread(), TOKEN_QUERY, TRUE, &mut token) } != 0 {
        drop(OwnedHandle(token));
        return Err("lifecycle_parent_thread_token_present".to_string());
    }
    if unsafe { GetLastError() } == ERROR_NO_TOKEN {
        Ok(())
    } else {
        Err("lifecycle_parent_thread_token_probe_failed".to_string())
    }
}

pub(crate) fn current_process_started_at() -> Result<u64, String> {
    process_started_at(unsafe { GetCurrentProcess() })
}

pub(crate) fn random_hex(length: usize) -> Result<String, String> {
    if length == 0 || !length.is_multiple_of(2) {
        return Err("lifecycle_random_length_invalid".to_string());
    }
    let mut bytes = vec![0_u8; length / 2];
    let status = unsafe {
        BCryptGenRandom(
            null_mut(),
            bytes.as_mut_ptr(),
            bytes.len() as u32,
            BCRYPT_USE_SYSTEM_PREFERRED_RNG,
        )
    };
    if status != 0 {
        return Err("lifecycle_random_generation_failed".to_string());
    }
    Ok(hex_digest(&bytes))
}

pub(crate) fn canonical_real_directory(path: &Path, label: &str) -> Result<PathBuf, String> {
    let canonical =
        fs::canonicalize(path).map_err(|_| format!("lifecycle_{label}_canonicalize_failed"))?;
    for ancestor in canonical.ancestors() {
        let metadata = fs::symlink_metadata(ancestor)
            .map_err(|_| format!("lifecycle_{label}_metadata_failed"))?;
        if !metadata.is_dir() || metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            return Err(format!("lifecycle_{label}_reparse_rejected"));
        }
    }
    let normalized =
        crate::collector_service::windows_provisioner::strip_verbatim_disk_prefix(canonical);
    if normalized.to_string_lossy().starts_with(r"\\") {
        return Err(format!("lifecycle_{label}_non_disk_path_rejected"));
    }
    Ok(normalized)
}

fn paths_equal(left: &Path, right: &Path) -> bool {
    left.to_string_lossy()
        .eq_ignore_ascii_case(&right.to_string_lossy())
}

fn open_directory_handle(path: &Path, label: &str, share_mode: u32) -> Result<File, String> {
    let handle = OpenOptions::new()
        .read(true)
        .share_mode(share_mode)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)
        .map_err(|_| format!("lifecycle_{label}_component_open_failed"))?;
    let metadata = handle
        .metadata()
        .map_err(|_| format!("lifecycle_{label}_component_metadata_failed"))?;
    if !metadata.is_dir() || metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
        return Err(format!("lifecycle_{label}_component_reparse_rejected"));
    }
    Ok(handle)
}

fn open_no_follow_directory_components(
    path: &Path,
    label: &str,
) -> Result<(PathBuf, Vec<File>), String> {
    let normalized =
        crate::collector_service::windows_provisioner::strip_verbatim_disk_prefix(path.into());
    let value = normalized.to_string_lossy();
    let bytes = value.as_bytes();
    if bytes.len() < 3
        || !bytes[0].is_ascii_alphabetic()
        || bytes[1] != b':'
        || !matches!(bytes[2], b'\\' | b'/')
        || value.starts_with(r"\\")
        || normalized.components().any(|component| {
            matches!(
                component,
                std::path::Component::CurDir | std::path::Component::ParentDir
            )
        })
    {
        return Err(format!("lifecycle_{label}_path_invalid"));
    }
    let mut ancestors = normalized.ancestors().collect::<Vec<_>>();
    ancestors.reverse();
    let mut handles = Vec::with_capacity(ancestors.len());
    for component in ancestors.into_iter().filter(|ancestor| ancestor.has_root()) {
        let handle = open_directory_handle(component, label, FILE_SHARE_READ | FILE_SHARE_WRITE)?;
        let observed =
            final_path(&handle).map_err(|_| format!("lifecycle_{label}_component_path_failed"))?;
        if !paths_equal(&observed, component) {
            return Err(format!("lifecycle_{label}_component_path_changed"));
        }
        handles.push(handle);
    }
    if handles.is_empty() {
        return Err(format!("lifecycle_{label}_component_missing"));
    }
    Ok((normalized, handles))
}

pub(crate) fn require_clean_exact_head(repo_root: &Path, expected: &str) -> Result<(), String> {
    let head = git_output(repo_root, &["rev-parse", "HEAD"], "head")?;
    if head != expected {
        return Err("lifecycle_controller_head_mismatch".to_string());
    }
    let status = git_output(
        repo_root,
        &["status", "--porcelain=v1", "--untracked-files=normal"],
        "status",
    )?;
    if !status.is_empty() {
        return Err("lifecycle_controller_worktree_dirty".to_string());
    }
    Ok(())
}

pub(crate) fn capture_parent_preflight(
    plan: &ProofPlan,
    controller_bindings: &[PeerBinding],
) -> Result<PreflightSnapshot, String> {
    let snapshot = capture_machine_snapshot(controller_bindings);
    require_allowlisted_parent_preflight(&snapshot, plan)?;
    Ok(snapshot)
}

pub(crate) fn capture_parent_current_user_authority(
) -> Result<ParentCurrentUserAuthorityGuard, String> {
    let (token_handle, token) = standard_primary_token()?;
    let profile_path = profile_directory_for_token(&token_handle)?;
    let profile = OwnedDirectory::open(&profile_path, "parent_user_profile")?;
    if !profile
        .path
        .to_string_lossy()
        .eq_ignore_ascii_case(&profile_path.to_string_lossy())
    {
        return Err("lifecycle_parent_user_profile_path_changed".to_string());
    }
    let local_app_data_path = local_app_data_for_token(&token_handle)?;
    let local_app_data = OwnedDirectory::open(&local_app_data_path, "parent_user_local_app_data")?;
    let runtime_local_app_data_path = std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .ok_or_else(|| "lifecycle_parent_runtime_local_app_data_missing".to_string())?;
    let runtime_local_app_data = OwnedDirectory::open(
        &runtime_local_app_data_path,
        "parent_user_runtime_local_app_data",
    )?;
    if runtime_local_app_data.identity != local_app_data.identity
        || !runtime_local_app_data
            .path
            .to_string_lossy()
            .eq_ignore_ascii_case(&local_app_data.path.to_string_lossy())
    {
        return Err("lifecycle_parent_runtime_local_app_data_mismatch".to_string());
    }
    let resolved = crate::persistence::resolve_current_user_root(
        crate::persistence::StoragePlatform::Windows,
        &crate::persistence::CurrentUserEnvironment {
            local_app_data: Some(local_app_data.path.clone()),
            xdg_data_home: None,
            home: None,
        },
    )
    .map_err(|_| "lifecycle_parent_user_data_root_resolve_failed".to_string())?;
    let resolved_data_root = resolved.directory.to_string_lossy().into_owned();
    let data_root_guard = OwnedDirectory::open(&resolved.directory, "parent_user_data_root")?;
    if !paths_equal(&data_root_guard.path, &resolved.directory) {
        return Err("lifecycle_parent_user_data_root_path_changed".to_string());
    }
    let data_root = Observation::Present(data_root_guard.snapshot());
    let authority = ParentCurrentUserAuthority {
        user_sid: token.sid_string.clone(),
        session_id: token.session_id,
        logon_luid: token.logon_luid,
        profile: profile.snapshot(),
        local_app_data: local_app_data.snapshot(),
        resolved_data_root,
        data_root,
    };
    validate_parent_current_user_authority(&authority)?;
    Ok(ParentCurrentUserAuthorityGuard {
        authority,
        token: token_handle,
        profile,
        local_app_data,
        data_root: data_root_guard,
    })
}

fn validate_parent_current_user_authority(
    authority: &ParentCurrentUserAuthority,
) -> Result<(), String> {
    if authority.user_sid.is_empty()
        || authority.session_id == 0
        || (authority.logon_luid.low_part == 0 && authority.logon_luid.high_part == 0)
        || authority.profile.identity.volume_serial == 0
        || authority.profile.identity.file_index == 0
        || authority.local_app_data.identity.volume_serial == 0
        || authority.local_app_data.identity.file_index == 0
    {
        return Err("lifecycle_parent_user_authority_identity_invalid".to_string());
    }
    let expected_root = Path::new(&authority.local_app_data.final_path).join("BatCaveMonitor");
    if !authority
        .local_app_data
        .final_path
        .to_ascii_lowercase()
        .starts_with(&(authority.profile.final_path.to_ascii_lowercase() + r"\"))
        || !authority
            .resolved_data_root
            .eq_ignore_ascii_case(&expected_root.to_string_lossy())
        || !matches!(authority.data_root, Observation::Present(_))
        || matches!(
            &authority.data_root,
            Observation::Present(root)
                if !root.final_path.eq_ignore_ascii_case(&authority.resolved_data_root)
                    || root.identity.volume_serial == 0
                    || root.identity.file_index == 0
        )
    {
        return Err("lifecycle_parent_user_authority_path_invalid".to_string());
    }
    Ok(())
}

pub(crate) fn capture_parent_current_user_objects(
    root: &ParentCurrentUserAuthorityGuard,
) -> Result<ParentCurrentUserObjectsGuard, String> {
    root.revalidate()?;
    let data_root = Path::new(&root.authority.resolved_data_root);
    let checkpoint_root =
        OwnedDirectory::open_without_delete_sharing(data_root, "parent_user_checkpoint_data_root")?;
    let Observation::Present(expected_root) = &root.authority.data_root else {
        return Err("lifecycle_parent_user_data_root_missing".to_string());
    };
    if checkpoint_root.snapshot() != *expected_root
        || checkpoint_root.identity != root.data_root.identity
    {
        return Err("lifecycle_parent_user_data_root_changed".to_string());
    }
    let (settings, settings_guard) = retain_parent_user_file(
        &data_root.join("settings.json"),
        &checkpoint_root,
        "parent_user_settings",
    )?;
    let (cache, cache_guard) = retain_parent_user_file(
        &data_root.join("warm-cache.json"),
        &checkpoint_root,
        "parent_user_cache",
    )?;
    let (diagnostics, diagnostics_guard) = retain_parent_user_file(
        &data_root.join("diagnostics.jsonl"),
        &checkpoint_root,
        "parent_user_diagnostics",
    )?;
    checkpoint_root.revalidate()?;
    root.data_root.revalidate()?;
    let authority = ParentCurrentUserObjects {
        settings,
        cache,
        diagnostics,
    };
    require_parent_current_user_objects_present(&authority)?;
    Ok(ParentCurrentUserObjectsGuard {
        authority,
        settings: settings_guard,
        cache: cache_guard,
        diagnostics: diagnostics_guard,
    })
}

pub(crate) fn validate_parent_current_user_objects_preserved(
    before: &ParentCurrentUserObjects,
    after: &ParentCurrentUserObjects,
) -> Result<(), String> {
    require_parent_current_user_objects_present(before)?;
    require_parent_current_user_objects_present(after)?;
    if before.settings != after.settings
        || before.cache != after.cache
        || before.diagnostics != after.diagnostics
    {
        return Err("lifecycle_parent_user_objects_not_preserved".to_string());
    }
    Ok(())
}

fn require_parent_current_user_objects_present(
    objects: &ParentCurrentUserObjects,
) -> Result<(), String> {
    if [&objects.settings, &objects.cache, &objects.diagnostics]
        .into_iter()
        .all(|observation| matches!(observation, Observation::Present(_)))
    {
        Ok(())
    } else {
        Err("lifecycle_parent_user_objects_missing".to_string())
    }
}

fn retain_parent_user_file(
    path: &Path,
    parent: &OwnedDirectory,
    label: &str,
) -> Result<(Observation<FileSnapshot>, Option<ParentObservedFile>), String> {
    match fs::symlink_metadata(path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            Ok((Observation::Absent, None))
        }
        Err(error) => Err(format!(
            "lifecycle_{label}_metadata_failed:{}",
            error.raw_os_error().unwrap_or_default()
        )),
        Ok(_) => {
            let file = ParentObservedFile::open(path, parent, label)?;
            let snapshot = file.snapshot();
            Ok((Observation::Present(snapshot), Some(file)))
        }
    }
}

pub(crate) fn capture_machine_snapshot(controller_bindings: &[PeerBinding]) -> PreflightSnapshot {
    PreflightSnapshot {
        service: observe_service(),
        install_root: observe_directory(Path::new(INSTALL_ROOT), "install_root"),
        monitor: observe_file(Path::new(MONITOR_PATH), "monitor"),
        service_binary: observe_file(Path::new(SERVICE_PATH), "service_binary"),
        uninstaller: observe_file(Path::new(UNINSTALLER_PATH), "uninstaller"),
        legacy_cli: observe_file(Path::new(LEGACY_CLI_PATH), "legacy_cli"),
        uninstall_registry: observe_uninstall_registry(),
        product_processes: observe_product_processes(controller_bindings),
    }
}

pub(crate) fn capture_elevated_machine_snapshot(
    controller_bindings: &[PeerBinding],
) -> ElevatedMachineSnapshot {
    let machine = capture_machine_snapshot(controller_bindings);
    let (product_data_root, service_data_root) =
        match crate::collector_service::windows_provisioner::data_roots_for_proof() {
            Ok((product, service)) => (
                observe_directory(&product, "product_data_root"),
                observe_directory(&service, "service_data_root"),
            ),
            Err(reason) => (
                Observation::Unknown(reason.clone()),
                Observation::Unknown(reason),
            ),
        };
    // The elevated worker must not infer current-user retention authority from
    // its own environment. A later lane will bind the authenticated standard
    // parent's profile root and retain that authority across elevation.
    let current_user_data_root =
        Observation::Unknown("lifecycle_parent_user_data_root_not_bound".to_string());
    let installed_boundaries = match &machine.service {
        Observation::Present(_) => {
            match crate::collector_service::windows_provisioner::observe_installed_boundaries_for_proof(
                Path::new(SERVICE_PATH),
            ) {
                Ok(boundaries) => Observation::Present(boundaries),
                Err(reason) => Observation::Unknown(reason),
            }
        }
        Observation::Absent => Observation::Absent,
        Observation::Unknown(reason) => Observation::Unknown(reason.clone()),
    };
    ElevatedMachineSnapshot {
        machine,
        product_data_root,
        service_data_root,
        current_user_data_root,
        installed_boundaries,
    }
}

pub(crate) fn open_installed_uninstaller(candidate: &Candidate) -> Result<OwnedFile, String> {
    OwnedFile::open(
        Path::new(UNINSTALLER_PATH),
        candidate.uninstaller_size,
        &candidate.uninstaller_sha256,
        "installed_uninstaller",
    )
}

pub(crate) fn open_installed_service(candidate: &Candidate) -> Result<OwnedFile, String> {
    let service = OwnedFile::open_unchecked(Path::new(SERVICE_PATH), "installed_service")?;
    if service.sha256_hex() != candidate.service_sha256 {
        return Err("lifecycle_installed_service_identity_mismatch".to_string());
    }
    Ok(service)
}

pub(crate) fn open_allowlisted_legacy_cli(plan: &ProofPlan) -> Result<OwnedFile, String> {
    let legacy = OwnedFile::open_unchecked(Path::new(LEGACY_CLI_PATH), "allowlisted_legacy_cli")?;
    if legacy.sha256_hex() != plan.allowlisted_start.legacy_cli_sha256 {
        return Err("lifecycle_allowlisted_legacy_cli_identity_mismatch".to_string());
    }
    Ok(legacy)
}

pub(crate) fn parse_sha256(value: &str, label: &str) -> Result<[u8; 32], String> {
    if value.len() != 64 {
        return Err(format!("lifecycle_{label}_sha256_invalid"));
    }
    let mut digest = [0_u8; 32];
    for (index, pair) in value.as_bytes().chunks_exact(2).enumerate() {
        let high =
            hex_nibble(pair[0]).ok_or_else(|| format!("lifecycle_{label}_sha256_invalid"))?;
        let low = hex_nibble(pair[1]).ok_or_else(|| format!("lifecycle_{label}_sha256_invalid"))?;
        digest[index] = (high << 4) | low;
    }
    Ok(digest)
}

pub(crate) fn require_allowlisted_parent_preflight(
    snapshot: &PreflightSnapshot,
    plan: &ProofPlan,
) -> Result<(), String> {
    let service = require_present(&snapshot.service, "service")?;
    if service.state != windows_sys::Win32::System::Services::SERVICE_STOPPED
        || service.process_id != 0
        || service.win32_exit_code != plan.allowlisted_start.win32_exit_code
        || service.service_specific_exit_code != plan.allowlisted_start.service_specific_exit_code
    {
        return Err("lifecycle_start_service_state_not_allowlisted".to_string());
    }
    require_fixed_install_root(&snapshot.install_root, "install_root")?;
    require_file_hash(
        &snapshot.monitor,
        &plan.allowlisted_start.monitor_sha256,
        "start_monitor",
    )?;
    require_file_hash(
        &snapshot.service_binary,
        &plan.allowlisted_start.service_sha256,
        "start_service",
    )?;
    require_file_hash(
        &snapshot.uninstaller,
        &plan.allowlisted_start.uninstaller_sha256,
        "start_uninstaller",
    )?;
    require_file_hash(
        &snapshot.legacy_cli,
        &plan.allowlisted_start.legacy_cli_sha256,
        "start_legacy_cli",
    )?;
    let registry = require_present(&snapshot.uninstall_registry, "uninstall_registry")?;
    if !is_fixed_install_location(&registry.install_location) {
        return Err("lifecycle_start_install_location_invalid".to_string());
    }
    let processes = require_present(&snapshot.product_processes, "product_processes")?;
    if !processes.is_empty() {
        return Err("lifecycle_product_process_running".to_string());
    }
    Ok(())
}

pub(crate) fn require_allowlisted_elevated_preflight(
    snapshot: &ElevatedMachineSnapshot,
    plan: &ProofPlan,
) -> Result<(), String> {
    require_allowlisted_parent_preflight(&snapshot.machine, plan)?;
    require_present(&snapshot.installed_boundaries, "start_installed_boundaries")?;
    Ok(())
}

pub(crate) fn require_installed_candidate(
    snapshot: &PreflightSnapshot,
    candidate: &Candidate,
    expect_legacy_cli_absent: bool,
    label: &str,
) -> Result<(), String> {
    let service = require_present(&snapshot.service, label)?;
    if service.state != windows_sys::Win32::System::Services::SERVICE_RUNNING
        || service.process_id == 0
        || service.win32_exit_code != 0
        || service.service_specific_exit_code != 0
    {
        return Err(format!("lifecycle_{label}_service_not_running"));
    }
    require_fixed_install_root(&snapshot.install_root, label)?;
    require_file_hash(
        &snapshot.monitor,
        &candidate.monitor_sha256,
        &format!("{label}_monitor"),
    )?;
    require_file_hash(
        &snapshot.service_binary,
        &candidate.service_sha256,
        &format!("{label}_service"),
    )?;
    require_file_size_and_hash(
        &snapshot.uninstaller,
        candidate.uninstaller_size,
        &candidate.uninstaller_sha256,
        &format!("{label}_uninstaller"),
    )?;
    if expect_legacy_cli_absent {
        require_absent(&snapshot.legacy_cli, &format!("{label}_legacy_cli"))?;
    }
    let registry = require_present(
        &snapshot.uninstall_registry,
        &format!("{label}_uninstall_registry"),
    )?;
    if !is_fixed_install_location(&registry.install_location) {
        return Err(format!("lifecycle_{label}_install_location_invalid"));
    }
    let processes = require_present(
        &snapshot.product_processes,
        &format!("{label}_product_processes"),
    )?;
    if processes.len() != 1
        || processes[0].process_id != service.process_id
        || !processes[0]
            .executable_name
            .eq_ignore_ascii_case("batcave-collector-service.exe")
        || !processes[0]
            .executable_path
            .as_deref()
            .is_some_and(|path| path.eq_ignore_ascii_case(SERVICE_PATH))
    {
        return Err(format!("lifecycle_{label}_process_set_invalid"));
    }
    Ok(())
}

pub(crate) fn require_elevated_installed_candidate(
    snapshot: &ElevatedMachineSnapshot,
    candidate: &Candidate,
    expect_legacy_cli_absent: bool,
    label: &str,
) -> Result<(), String> {
    require_installed_candidate(
        &snapshot.machine,
        candidate,
        expect_legacy_cli_absent,
        label,
    )?;
    require_present(
        &snapshot.installed_boundaries,
        &format!("{label}_installed_boundaries"),
    )?;
    require_present(
        &snapshot.product_data_root,
        &format!("{label}_product_data_root"),
    )?;
    require_present(
        &snapshot.service_data_root,
        &format!("{label}_service_data_root"),
    )?;
    Ok(())
}

pub(crate) fn require_elevated_stopped_candidate(
    snapshot: &ElevatedMachineSnapshot,
    candidate: &Candidate,
    expect_legacy_cli_absent: bool,
    label: &str,
) -> Result<(), String> {
    require_elevated_stopped_candidate_inner(
        snapshot,
        candidate,
        expect_legacy_cli_absent,
        true,
        label,
    )
}

pub(crate) fn require_elevated_crashed_candidate(
    snapshot: &ElevatedMachineSnapshot,
    candidate: &Candidate,
    expect_legacy_cli_absent: bool,
    label: &str,
) -> Result<(), String> {
    require_elevated_stopped_candidate_inner(
        snapshot,
        candidate,
        expect_legacy_cli_absent,
        false,
        label,
    )
}

fn require_elevated_stopped_candidate_inner(
    snapshot: &ElevatedMachineSnapshot,
    candidate: &Candidate,
    expect_legacy_cli_absent: bool,
    require_clean_exit: bool,
    label: &str,
) -> Result<(), String> {
    let service = require_present(&snapshot.machine.service, label)?;
    if service.state != windows_sys::Win32::System::Services::SERVICE_STOPPED
        || service.process_id != 0
        || (require_clean_exit
            && (service.win32_exit_code != 0 || service.service_specific_exit_code != 0))
        || (!require_clean_exit
            && service.win32_exit_code == 0
            && service.service_specific_exit_code == 0)
    {
        return Err(format!(
            "lifecycle_{label}_service_not_{}stopped",
            if require_clean_exit { "cleanly_" } else { "" }
        ));
    }
    require_fixed_install_root(&snapshot.machine.install_root, label)?;
    require_file_hash(
        &snapshot.machine.monitor,
        &candidate.monitor_sha256,
        &format!("{label}_monitor"),
    )?;
    require_file_hash(
        &snapshot.machine.service_binary,
        &candidate.service_sha256,
        &format!("{label}_service"),
    )?;
    require_file_size_and_hash(
        &snapshot.machine.uninstaller,
        candidate.uninstaller_size,
        &candidate.uninstaller_sha256,
        &format!("{label}_uninstaller"),
    )?;
    if expect_legacy_cli_absent {
        require_absent(&snapshot.machine.legacy_cli, &format!("{label}_legacy_cli"))?;
    }
    let registry = require_present(
        &snapshot.machine.uninstall_registry,
        &format!("{label}_uninstall_registry"),
    )?;
    if !is_fixed_install_location(&registry.install_location) {
        return Err(format!("lifecycle_{label}_install_location_invalid"));
    }
    let processes = require_present(
        &snapshot.machine.product_processes,
        &format!("{label}_product_processes"),
    )?;
    if !processes.is_empty() {
        return Err(format!("lifecycle_{label}_process_residue"));
    }
    require_present(
        &snapshot.installed_boundaries,
        &format!("{label}_installed_boundaries"),
    )?;
    require_present(
        &snapshot.product_data_root,
        &format!("{label}_product_data_root"),
    )?;
    require_present(
        &snapshot.service_data_root,
        &format!("{label}_service_data_root"),
    )?;
    Ok(())
}

pub(crate) fn require_total_product_absence(
    snapshot: &PreflightSnapshot,
    label: &str,
) -> Result<(), String> {
    require_absent(&snapshot.service, &format!("{label}_service"))?;
    require_absent(&snapshot.install_root, &format!("{label}_install_root"))?;
    require_absent(&snapshot.monitor, &format!("{label}_monitor"))?;
    require_absent(&snapshot.service_binary, &format!("{label}_service_binary"))?;
    require_absent(&snapshot.uninstaller, &format!("{label}_uninstaller"))?;
    require_absent(&snapshot.legacy_cli, &format!("{label}_legacy_cli"))?;
    require_absent(
        &snapshot.uninstall_registry,
        &format!("{label}_uninstall_registry"),
    )?;
    let processes = require_present(
        &snapshot.product_processes,
        &format!("{label}_product_processes"),
    )?;
    if !processes.is_empty() {
        return Err(format!("lifecycle_{label}_process_residue"));
    }
    Ok(())
}

pub(crate) fn require_elevated_total_product_absence(
    snapshot: &ElevatedMachineSnapshot,
    label: &str,
) -> Result<(), String> {
    require_total_product_absence(&snapshot.machine, label)?;
    require_absent(
        &snapshot.installed_boundaries,
        &format!("{label}_installed_boundaries"),
    )?;
    require_absent(
        &snapshot.product_data_root,
        &format!("{label}_product_data_root"),
    )?;
    require_absent(
        &snapshot.service_data_root,
        &format!("{label}_service_data_root"),
    )
}

pub(crate) fn create_parent_pipe(locator: &str) -> Result<PipeConnection, String> {
    let token = current_token()?;
    let sddl = format!("D:P(A;;GA;;;SY)(A;;GA;;;BA)(A;;GA;;;{})", token.sid_string);
    let descriptor = SecurityDescriptor::from_sddl(&sddl)?;
    let attributes = SECURITY_ATTRIBUTES {
        nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: descriptor.raw().cast(),
        bInheritHandle: 0,
    };
    let name = wide(format!("{PIPE_PREFIX}{locator}"));
    let handle = unsafe {
        CreateNamedPipeW(
            name.as_ptr(),
            PIPE_ACCESS_DUPLEX | FILE_FLAG_FIRST_PIPE_INSTANCE | FILE_FLAG_OVERLAPPED,
            PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
            1,
            PIPE_BUFFER_SIZE,
            PIPE_BUFFER_SIZE,
            0,
            &attributes,
        )
    };
    if handle == INVALID_HANDLE_VALUE {
        return Err("lifecycle_pipe_create_failed".to_string());
    }
    Ok(PipeConnection {
        handle: OwnedHandle(handle),
        server: true,
        connected: false,
    })
}

pub(crate) fn connect_worker_pipe(
    locator: &str,
    timeout: Duration,
) -> Result<PipeConnection, String> {
    let name = wide(format!("{PIPE_PREFIX}{locator}"));
    let deadline = Instant::now() + timeout;
    loop {
        let handle = unsafe {
            CreateFileW(
                name.as_ptr(),
                GENERIC_READ | GENERIC_WRITE,
                0,
                null(),
                OPEN_EXISTING,
                FILE_FLAG_OVERLAPPED,
                null_mut(),
            )
        };
        if handle != INVALID_HANDLE_VALUE {
            let mode = PIPE_READMODE_BYTE;
            if unsafe { SetNamedPipeHandleState(handle, &mode, null_mut(), null_mut()) } == 0 {
                unsafe { CloseHandle(handle) };
                return Err("lifecycle_pipe_mode_failed".to_string());
            }
            return Ok(PipeConnection {
                handle: OwnedHandle(handle),
                server: false,
                connected: true,
            });
        }
        if unsafe { GetLastError() } != ERROR_PIPE_BUSY {
            return Err("lifecycle_pipe_open_failed".to_string());
        }
        let remaining = deadline
            .checked_duration_since(Instant::now())
            .ok_or_else(|| "lifecycle_pipe_open_timeout".to_string())?;
        if unsafe {
            WaitNamedPipeW(
                name.as_ptr(),
                duration_ms(
                    remaining.min(Duration::from_secs(1)),
                    "lifecycle_pipe_wait_invalid",
                )?,
            )
        } == 0
            && unsafe { GetLastError() } != ERROR_PIPE_BUSY
        {
            return Err("lifecycle_pipe_wait_failed".to_string());
        }
    }
}

pub(crate) fn launch_elevated_worker(locator: &str) -> Result<ElevatedProcess, String> {
    let executable =
        std::env::current_exe().map_err(|_| "lifecycle_controller_path_failed".to_string())?;
    let protected_directory = system_directory()?;
    let verb = wide("runas");
    let file = wide(executable.as_os_str());
    let parameters = wide(format!("--worker {locator}"));
    let directory = wide(protected_directory.as_os_str());
    let mut info: SHELLEXECUTEINFOW = unsafe { zeroed() };
    info.cbSize = size_of::<SHELLEXECUTEINFOW>() as u32;
    info.fMask = SEE_MASK_NOCLOSEPROCESS;
    info.lpVerb = verb.as_ptr();
    info.lpFile = file.as_ptr();
    info.lpParameters = parameters.as_ptr();
    info.lpDirectory = directory.as_ptr();
    info.nShow = SW_HIDE;
    if unsafe { ShellExecuteExW(&mut info) } == 0 {
        return Err(if unsafe { GetLastError() } == ERROR_CANCELLED {
            "lifecycle_elevation_denied".to_string()
        } else {
            "lifecycle_elevation_launch_failed".to_string()
        });
    }
    if info.hProcess.is_null() {
        return Err("lifecycle_elevation_process_missing".to_string());
    }
    let process_id = unsafe { GetProcessId(info.hProcess) };
    if process_id == 0 {
        unsafe { CloseHandle(info.hProcess) };
        return Err("lifecycle_elevation_pid_failed".to_string());
    }
    let started_at_100ns = process_started_at(info.hProcess)?;
    Ok(ElevatedProcess {
        job: None,
        handle: OwnedHandle(info.hProcess),
        process_id,
        started_at_100ns,
        settled: false,
    })
}

pub(crate) fn authenticate_worker_peer(
    pipe: &PipeConnection,
    worker: &ElevatedProcess,
    controller: &OwnedFile,
) -> Result<(), String> {
    let process_id = pipe.client_process_id()?;
    if process_id != worker.process_id {
        return Err("lifecycle_worker_pipe_pid_mismatch".to_string());
    }
    let peer = process_evidence(process_id)?;
    let parent = current_token()?;
    if peer.started_at_100ns != worker.started_at_100ns
        || !peer.token.elevated
        || peer.token.sid != parent.sid
        || peer.token.session_id != parent.session_id
        || peer.image.identity() != controller.identity()
        || peer.image.sha256 != controller.sha256
    {
        return Err("lifecycle_worker_peer_identity_invalid".to_string());
    }
    if pipe.client_process_id()? != process_id {
        return Err("lifecycle_worker_pipe_pid_changed".to_string());
    }
    Ok(())
}

pub(crate) fn authenticate_parent_peer(
    pipe: &PipeConnection,
    controller: &OwnedFile,
) -> Result<PeerBinding, String> {
    let process_id = pipe.server_process_id()?;
    let peer = process_evidence(process_id)?;
    let worker = current_token()?;
    if peer.token.elevated
        || peer.token.sid != worker.sid
        || peer.token.session_id != worker.session_id
        || peer.image.identity() != controller.identity()
        || peer.image.sha256 != controller.sha256
    {
        return Err("lifecycle_parent_peer_identity_invalid".to_string());
    }
    if pipe.server_process_id()? != process_id {
        return Err("lifecycle_parent_pipe_pid_changed".to_string());
    }
    Ok(PeerBinding {
        process_id,
        started_at_100ns: peer.started_at_100ns,
        image_identity: peer.image.identity(),
        image_sha256: peer.image.sha256_hex(),
    })
}

pub(crate) fn current_controller_binding(controller: &OwnedFile) -> Result<PeerBinding, String> {
    Ok(PeerBinding {
        process_id: std::process::id(),
        started_at_100ns: current_process_started_at()?,
        image_identity: controller.identity(),
        image_sha256: controller.sha256_hex(),
    })
}

pub(crate) fn create_protected_evidence_root(
    nonce: &str,
    pipe: &PipeConnection,
) -> Result<ProtectedEvidenceRoot, String> {
    let parent = process_evidence(pipe.server_process_id()?)?;
    let program_data = canonical_real_directory(Path::new(r"C:\ProgramData"), "programdata")?;
    if !program_data
        .to_string_lossy()
        .eq_ignore_ascii_case(r"C:\ProgramData")
    {
        return Err("lifecycle_programdata_final_path_invalid".to_string());
    }
    let root = PathBuf::from(format!("{EVIDENCE_ROOT_PREFIX}{nonce}"));
    match fs::symlink_metadata(&root) {
        Ok(_) => return Err("lifecycle_evidence_root_stale".to_string()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            return Err(format!(
                "lifecycle_evidence_root_probe_failed:{}",
                error.raw_os_error().unwrap_or_default()
            ));
        }
    }
    let sddl = format!(
        "O:BAD:P(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)(A;OICI;GR;;;{})",
        parent.token.sid_string
    );
    let descriptor = SecurityDescriptor::from_sddl(&sddl)?;
    let attributes = SECURITY_ATTRIBUTES {
        nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: descriptor.raw().cast(),
        bInheritHandle: 0,
    };
    let root_wide = wide(root.as_os_str());
    if unsafe {
        windows_sys::Win32::Storage::FileSystem::CreateDirectoryW(root_wide.as_ptr(), &attributes)
    } == 0
    {
        return Err("lifecycle_evidence_root_create_failed".to_string());
    }
    let canonical_root = canonical_real_directory(&root, "evidence_root")?;
    if !canonical_root
        .to_string_lossy()
        .eq_ignore_ascii_case(&root.to_string_lossy())
    {
        return Err("lifecycle_evidence_root_final_path_invalid".to_string());
    }
    let handle = unsafe {
        CreateFileW(
            root_wide.as_ptr(),
            FILE_READ_ATTRIBUTES,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            null(),
            OPEN_EXISTING,
            FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT,
            null_mut(),
        )
    };
    if handle == INVALID_HANDLE_VALUE {
        return Err("lifecycle_evidence_root_open_failed".to_string());
    }
    let handle = OwnedHandle(handle);
    let information = file_information_handle(handle.raw())
        .map_err(|_| "lifecycle_evidence_root_identity_failed".to_string())?;
    let final_path = final_path_handle(handle.raw())
        .map_err(|_| "lifecycle_evidence_root_handle_path_failed".to_string())?;
    if !final_path
        .to_string_lossy()
        .eq_ignore_ascii_case(&root.to_string_lossy())
    {
        return Err("lifecycle_evidence_root_handle_path_invalid".to_string());
    }
    Ok(ProtectedEvidenceRoot {
        root,
        identity: EvidenceRootIdentity {
            volume_serial: information.identity.volume_serial,
            file_index: information.identity.file_index,
        },
        _handle: handle,
    })
}

pub(crate) fn open_protected_evidence_root(
    value: &str,
    nonce: &str,
    expected_identity: EvidenceRootIdentity,
) -> Result<ProtectedEvidenceRoot, String> {
    if value != format!("{EVIDENCE_ROOT_PREFIX}{nonce}") {
        return Err("lifecycle_evidence_root_binding_invalid".to_string());
    }
    let root = PathBuf::from(value);
    let metadata = fs::symlink_metadata(&root)
        .map_err(|_| "lifecycle_evidence_root_metadata_failed".to_string())?;
    if !metadata.is_dir() || metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
        return Err("lifecycle_evidence_root_type_invalid".to_string());
    }
    let root_wide = wide(root.as_os_str());
    let handle = unsafe {
        CreateFileW(
            root_wide.as_ptr(),
            FILE_READ_ATTRIBUTES,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            null(),
            OPEN_EXISTING,
            FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT,
            null_mut(),
        )
    };
    if handle == INVALID_HANDLE_VALUE {
        return Err("lifecycle_evidence_root_open_failed".to_string());
    }
    let handle = OwnedHandle(handle);
    let information = file_information_handle(handle.raw())
        .map_err(|_| "lifecycle_evidence_root_identity_failed".to_string())?;
    let identity = EvidenceRootIdentity {
        volume_serial: information.identity.volume_serial,
        file_index: information.identity.file_index,
    };
    let final_path = final_path_handle(handle.raw())
        .map_err(|_| "lifecycle_evidence_root_handle_path_failed".to_string())?;
    if identity != expected_identity
        || !final_path
            .to_string_lossy()
            .eq_ignore_ascii_case(&root.to_string_lossy())
    {
        return Err("lifecycle_evidence_root_identity_mismatch".to_string());
    }
    Ok(ProtectedEvidenceRoot {
        root,
        identity,
        _handle: handle,
    })
}

pub(crate) fn verify_evidence_receipt(
    root: &ProtectedEvidenceRoot,
    receipt: &EvidenceReceipt,
) -> Result<VerifiedEvidenceFile, String> {
    if !valid_evidence_name(&receipt.name) || receipt.size == 0 {
        return Err("lifecycle_failure_evidence_receipt_invalid".to_string());
    }
    let file = OwnedFile::open(
        &root.root.join(&receipt.name),
        receipt.size,
        &receipt.sha256,
        "failure_evidence",
    )?;
    Ok(VerifiedEvidenceFile {
        receipt: receipt.clone(),
        file,
    })
}

fn evidence_receipt(name: &str, payload: &[u8]) -> EvidenceReceipt {
    EvidenceReceipt {
        name: name.to_string(),
        size: payload.len() as u64,
        sha256: hex_digest(&Sha256::digest(payload)),
    }
}

fn observe_file(path: &Path, label: &str) -> Observation<FileSnapshot> {
    match fs::symlink_metadata(path) {
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Observation::Absent;
        }
        Err(error) => {
            return Observation::Unknown(format!(
                "lifecycle_{label}_metadata_failed:{}",
                error.raw_os_error().unwrap_or_default()
            ));
        }
    }
    match OwnedFile::open_unchecked(path, label) {
        Ok(file) => Observation::Present(file.snapshot()),
        Err(reason) => Observation::Unknown(reason),
    }
}

fn observe_directory(path: &Path, label: &str) -> Observation<DirectorySnapshot> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => {
            if !metadata.is_dir() || metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
            {
                return Observation::Unknown(format!("lifecycle_{label}_type_invalid"));
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Observation::Absent;
        }
        Err(error) => {
            return Observation::Unknown(format!(
                "lifecycle_{label}_metadata_failed:{}",
                error.raw_os_error().unwrap_or_default()
            ));
        }
    }
    let handle = match OpenOptions::new()
        .read(true)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)
    {
        Ok(handle) => handle,
        Err(error) => {
            return Observation::Unknown(format!(
                "lifecycle_{label}_open_failed:{}",
                error.raw_os_error().unwrap_or_default()
            ));
        }
    };
    let information = match file_information(&handle) {
        Ok(information) => information,
        Err(error) => {
            return Observation::Unknown(format!(
                "lifecycle_{label}_identity_failed:{}",
                error.raw_os_error().unwrap_or_default()
            ));
        }
    };
    let final_path = match final_path(&handle) {
        Ok(path) => path,
        Err(error) => {
            return Observation::Unknown(format!(
                "lifecycle_{label}_final_path_failed:{}",
                error.raw_os_error().unwrap_or_default()
            ));
        }
    };
    Observation::Present(DirectorySnapshot {
        identity: information.identity,
        final_path: final_path.to_string_lossy().into_owned(),
    })
}

fn observe_service() -> Observation<ServiceSnapshot> {
    let manager = unsafe { OpenSCManagerW(null(), null(), SC_MANAGER_CONNECT) };
    if manager.is_null() {
        return Observation::Unknown("service_manager_open_failed".to_string());
    }
    let manager = ServiceHandle(manager);
    let service_name = wide(SERVICE_NAME);
    let service =
        unsafe { OpenServiceW(manager.raw(), service_name.as_ptr(), SERVICE_QUERY_STATUS) };
    if service.is_null() {
        return match unsafe { GetLastError() } {
            ERROR_SERVICE_DOES_NOT_EXIST => Observation::Absent,
            error => Observation::Unknown(format!("service_open_failed:{error}")),
        };
    }
    let service = ServiceHandle(service);
    let mut status: SERVICE_STATUS_PROCESS = unsafe { zeroed() };
    let mut returned = 0;
    if unsafe {
        QueryServiceStatusEx(
            service.raw(),
            SC_STATUS_PROCESS_INFO,
            (&mut status as *mut SERVICE_STATUS_PROCESS).cast(),
            size_of::<SERVICE_STATUS_PROCESS>() as u32,
            &mut returned,
        )
    } == 0
    {
        return Observation::Unknown(format!("service_status_failed:{}", unsafe {
            GetLastError()
        }));
    }
    let process_started_at_100ns = if status.dwProcessId == 0 {
        None
    } else {
        let process = unsafe {
            OpenProcess(
                PROCESS_QUERY_LIMITED_INFORMATION | SYNCHRONIZE,
                0,
                status.dwProcessId,
            )
        };
        if process.is_null() {
            return Observation::Unknown(format!("service_process_open_failed:{}", unsafe {
                GetLastError()
            }));
        }
        match process_started_at(OwnedHandle(process).raw()) {
            Ok(started_at) => Some(started_at),
            Err(reason) => return Observation::Unknown(reason),
        }
    };
    let mut revalidated: SERVICE_STATUS_PROCESS = unsafe { zeroed() };
    if unsafe {
        QueryServiceStatusEx(
            service.raw(),
            SC_STATUS_PROCESS_INFO,
            (&mut revalidated as *mut SERVICE_STATUS_PROCESS).cast(),
            size_of::<SERVICE_STATUS_PROCESS>() as u32,
            &mut returned,
        )
    } == 0
        || revalidated.dwCurrentState != status.dwCurrentState
        || revalidated.dwProcessId != status.dwProcessId
        || revalidated.dwWin32ExitCode != status.dwWin32ExitCode
        || revalidated.dwServiceSpecificExitCode != status.dwServiceSpecificExitCode
    {
        return Observation::Unknown("service_status_changed_during_capture".to_string());
    }
    Observation::Present(ServiceSnapshot {
        state: status.dwCurrentState,
        process_id: status.dwProcessId,
        process_started_at_100ns,
        win32_exit_code: status.dwWin32ExitCode,
        service_specific_exit_code: status.dwServiceSpecificExitCode,
    })
}

fn observe_uninstall_registry() -> Observation<RegistrySnapshot> {
    for (view, label) in [
        (KEY_WOW64_64KEY, RegistryView::Registry64),
        (KEY_WOW64_32KEY, RegistryView::Registry32),
    ] {
        match open_registry_key(HKEY_LOCAL_MACHINE, UNINSTALL_KEY, KEY_READ | view) {
            Ok(Some(key)) => match read_registry_string(key.raw(), INSTALL_LOCATION_VALUE) {
                Ok(value) => {
                    return Observation::Present(RegistrySnapshot {
                        view: label,
                        install_location: value,
                    })
                }
                Err(reason) => return Observation::Unknown(reason),
            },
            Ok(None) => {}
            Err(reason) => return Observation::Unknown(reason),
        }
    }
    Observation::Absent
}

fn observe_product_processes(
    controller_bindings: &[PeerBinding],
) -> Observation<Vec<ProcessSnapshot>> {
    let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
    if snapshot == INVALID_HANDLE_VALUE {
        return Observation::Unknown("process_snapshot_failed".to_string());
    }
    let snapshot = OwnedHandle(snapshot);
    let mut entry: PROCESSENTRY32W = unsafe { zeroed() };
    entry.dwSize = size_of::<PROCESSENTRY32W>() as u32;
    let mut processes = Vec::new();
    let mut ok = unsafe { Process32FirstW(snapshot.raw(), &mut entry) };
    if ok == 0 {
        return if unsafe { GetLastError() } == ERROR_NO_MORE_FILES {
            Observation::Present(Vec::new())
        } else {
            Observation::Unknown("process_enumeration_start_failed".to_string())
        };
    }
    while ok != 0 {
        processes.push(EnumeratedProcess {
            process_id: entry.th32ProcessID,
            parent_process_id: entry.th32ParentProcessID,
            executable_name: utf16_z(&entry.szExeFile),
        });
        ok = unsafe { Process32NextW(snapshot.raw(), &mut entry) };
    }
    if unsafe { GetLastError() } != ERROR_NO_MORE_FILES {
        return Observation::Unknown("process_enumeration_failed".to_string());
    }

    let mut found = Vec::new();
    let mut image_paths = HashMap::new();
    for process in &processes {
        let lower = process.executable_name.to_ascii_lowercase();
        let owned_webview = match is_batcave_owned_webview(process, &processes, |process_id| {
            cached_process_image_path(&mut image_paths, process_id)
        }) {
            Ok(owned) => owned,
            Err(reason) => return Observation::Unknown(reason),
        };
        if is_product_process_name(&lower) || owned_webview {
            if lower == "batcave-windows-lifecycle-proof.exe" {
                match exact_bound_controller_process(process.process_id, controller_bindings) {
                    Ok(true) => {
                        continue;
                    }
                    Ok(false) => {}
                    Err(reason) => return Observation::Unknown(reason),
                }
            }
            let path = match cached_process_image_path(&mut image_paths, process.process_id) {
                Ok(path) => Some(path),
                Err(reason) if lower == "uninstall.exe" || owned_webview => {
                    return Observation::Unknown(reason);
                }
                Err(_) => None,
            };
            let is_product = lower != "uninstall.exe"
                || path
                    .as_deref()
                    .is_some_and(|value| value.eq_ignore_ascii_case(UNINSTALLER_PATH));
            if is_product {
                found.push(ProcessSnapshot {
                    process_id: process.process_id,
                    parent_process_id: process.parent_process_id,
                    executable_name: process.executable_name.clone(),
                    executable_path: path,
                });
            }
        }
    }
    Observation::Present(found)
}

fn cached_process_image_path(
    cache: &mut HashMap<u32, Result<String, String>>,
    process_id: u32,
) -> Result<String, String> {
    cache
        .entry(process_id)
        .or_insert_with(|| process_image_path_by_pid(process_id))
        .clone()
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct EnumeratedProcess {
    process_id: u32,
    parent_process_id: u32,
    executable_name: String,
}

fn is_product_process_name(lower: &str) -> bool {
    lower == "batcave-monitor.exe"
        || lower == "batcave-collector-service.exe"
        || lower == "batcave-monitor-cli.exe"
        || lower.contains("batcave") && lower.contains("setup")
        || lower == "uninstall.exe"
        || lower == "batcave-windows-lifecycle-proof.exe"
}

fn is_batcave_owned_webview(
    process: &EnumeratedProcess,
    processes: &[EnumeratedProcess],
    mut image_path: impl FnMut(u32) -> Result<String, String>,
) -> Result<bool, String> {
    const MAX_WEBVIEW_ANCESTRY_DEPTH: usize = 32;
    if !process
        .executable_name
        .eq_ignore_ascii_case("msedgewebview2.exe")
    {
        return Ok(false);
    }

    let mut parent_process_id = process.parent_process_id;
    let mut visited = Vec::new();
    for _ in 0..MAX_WEBVIEW_ANCESTRY_DEPTH {
        if parent_process_id == 0 || visited.contains(&parent_process_id) {
            return Ok(false);
        }
        visited.push(parent_process_id);
        let Some(parent) = processes
            .iter()
            .find(|candidate| candidate.process_id == parent_process_id)
        else {
            return Ok(false);
        };
        if parent
            .executable_name
            .eq_ignore_ascii_case("batcave-monitor.exe")
        {
            let path = image_path(parent.process_id)
                .map_err(|_| "lifecycle_webview_monitor_path_unavailable".to_string())?;
            return Ok(path.eq_ignore_ascii_case(MONITOR_PATH));
        }
        if !parent
            .executable_name
            .eq_ignore_ascii_case("msedgewebview2.exe")
        {
            return Ok(false);
        }
        parent_process_id = parent.parent_process_id;
    }
    Ok(false)
}

fn exact_bound_controller_process(
    process_id: u32,
    bindings: &[PeerBinding],
) -> Result<bool, String> {
    let Some(binding) = bindings
        .iter()
        .find(|binding| binding.process_id == process_id)
    else {
        return Ok(false);
    };
    let process = process_evidence(process_id)?;
    Ok(controller_binding_matches(
        binding,
        process_id,
        process.started_at_100ns,
        process.image.identity(),
        &process.image.sha256_hex(),
    ))
}

fn controller_binding_matches(
    binding: &PeerBinding,
    process_id: u32,
    started_at_100ns: u64,
    image_identity: FileIdentity,
    image_sha256: &str,
) -> bool {
    binding.process_id == process_id
        && binding.started_at_100ns == started_at_100ns
        && binding.image_identity == image_identity
        && binding.image_sha256 == image_sha256
}

pub(super) fn observe_desktop_collector_runtime(
    desktop: &DesktopProcess,
) -> Result<DesktopCollectorRuntimeObservation, String> {
    let installed_service = match fs::symlink_metadata(SERVICE_PATH) {
        Ok(_) => Some(OwnedFile::open_unchecked(
            Path::new(SERVICE_PATH),
            "desktop_installed_service",
        )?),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(_) => return Err("lifecycle_desktop_installed_service_metadata_failed".to_string()),
    };
    let installed_service_observation = installed_service.as_ref().map(desktop_file_observation);
    match observe_service() {
        Observation::Absent => Ok(DesktopCollectorRuntimeObservation {
            installed_service: installed_service_observation,
            service_process: None,
            pipe_server_process_id: None,
        }),
        Observation::Present(service) if service.state != SERVICE_RUNNING => {
            Ok(DesktopCollectorRuntimeObservation {
                installed_service: installed_service_observation,
                service_process: None,
                pipe_server_process_id: None,
            })
        }
        Observation::Present(service) => {
            let installed_service = installed_service
                .ok_or_else(|| "lifecycle_desktop_running_service_binary_missing".to_string())?;
            let installed_service_observation = desktop_file_observation(&installed_service);
            let peer = crate::collector_service::windows_client::verified_service_peer_for_proof(
                desktop.executable_path(),
                desktop.executable_transport_identity(),
                &installed_service.path,
                installed_service.transport_identity(),
            )?;
            if peer.process_id() != service.process_id {
                return Err("lifecycle_desktop_service_peer_pid_mismatch".to_string());
            }
            if peer.executable_file_identity() != installed_service.transport_identity() {
                return Err("lifecycle_desktop_service_peer_file_mismatch".to_string());
            }
            Ok(DesktopCollectorRuntimeObservation {
                service_process: Some(DesktopServiceProcessObservation {
                    process_id: peer.process_id(),
                    started_at_100ns: peer.process_started_at(),
                    local_system: true,
                    executable_path: installed_service_observation.executable_path.clone(),
                    executable_size: installed_service_observation.executable_size,
                    executable_sha256: installed_service_observation.executable_sha256.clone(),
                }),
                pipe_server_process_id: Some(peer.process_id()),
                installed_service: Some(installed_service_observation),
            })
        }
        Observation::Unknown(reason) => Err(format!("lifecycle_desktop_service_unknown:{reason}")),
    }
}

pub(super) fn wait_for_foreground_window_identity(
    expected_window: isize,
    expected_process_id: u32,
    expected_started_at_100ns: u64,
) -> Result<(), String> {
    let deadline = Instant::now() + DESKTOP_PROCESS_TIMEOUT;
    loop {
        let window = unsafe { GetForegroundWindow() };
        if window as isize == expected_window
            && validate_window_process_identity(
                expected_window,
                expected_process_id,
                expected_started_at_100ns,
            )
            .is_ok()
        {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err("lifecycle_desktop_primary_focus_timeout".to_string());
        }
        std::thread::sleep(PROCESS_TREE_POLL_INTERVAL);
    }
}

pub(super) fn validate_window_process_identity(
    window: isize,
    expected_process_id: u32,
    expected_started_at_100ns: u64,
) -> Result<(), String> {
    if window == 0 {
        return Err("lifecycle_desktop_window_handle_invalid".to_string());
    }
    let mut process_id = 0;
    if unsafe { GetWindowThreadProcessId(window as _, &mut process_id) } == 0
        || process_id != expected_process_id
    {
        return Err("lifecycle_desktop_window_process_mismatch".to_string());
    }
    let process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, process_id) };
    if process.is_null()
        || process_started_at(OwnedHandle(process).raw())? != expected_started_at_100ns
    {
        return Err("lifecycle_desktop_window_generation_mismatch".to_string());
    }
    Ok(())
}

fn desktop_file_observation(file: &OwnedFile) -> DesktopFileObservation {
    DesktopFileObservation {
        executable_path: file.path.to_string_lossy().into_owned(),
        executable_size: file.size,
        executable_sha256: file.sha256_hex(),
    }
}

fn observe_desktop_process(
    process_id: u32,
    parent_process_id: Option<u32>,
    label: &str,
    expected_token: Option<&TokenEvidence>,
) -> Result<DesktopProcessObservation, String> {
    Ok(retain_desktop_process(process_id, parent_process_id, label, expected_token)?.observation)
}

fn retain_desktop_process(
    process_id: u32,
    parent_process_id: Option<u32>,
    label: &str,
    expected_token: Option<&TokenEvidence>,
) -> Result<RetainedDesktopProcess, String> {
    let process = unsafe {
        OpenProcess(
            PROCESS_QUERY_LIMITED_INFORMATION | SYNCHRONIZE,
            0,
            process_id,
        )
    };
    if process.is_null() {
        return Err(format!("lifecycle_{label}_process_open_failed"));
    }
    let process = OwnedHandle(process);
    let token = token_for_process(process.raw())?;
    if let Some(expected) = expected_token {
        if token.sid != expected.sid
            || token.session_id != expected.session_id
            || token.elevated
            || token.elevation_type != expected.elevation_type
        {
            return Err(format!("lifecycle_{label}_token_identity_invalid"));
        }
    }
    let path = process_image_path(process.raw())?;
    let image = OwnedFile::open_unchecked(Path::new(&path), label)?;
    let observation = DesktopProcessObservation {
        process_id,
        parent_process_id,
        started_at_100ns: process_started_at(process.raw())?,
        session_id: token.session_id,
        elevated: token.elevated,
        executable_path: image.path.to_string_lossy().into_owned(),
        executable_size: image.size,
        executable_sha256: image.sha256_hex(),
    };
    Ok(RetainedDesktopProcess {
        observation,
        process,
        image,
    })
}

fn observe_stable_desktop_process_tree(
    job: &Job,
    root_process_id: u32,
    expected_token: &TokenEvidence,
) -> Result<DesktopProcessTree, String> {
    let deadline = Instant::now() + DESKTOP_PROCESS_TIMEOUT;
    let mut previous = None;
    let mut stable_snapshots = 0;
    let mut last_failure = "lifecycle_desktop_process_tree_not_ready".to_string();
    loop {
        match capture_desktop_process_tree(job, root_process_id, expected_token) {
            Ok(tree) => {
                let signature = tree.observations.clone();
                if previous.as_ref() == Some(&signature) {
                    stable_snapshots += 1;
                } else {
                    previous = Some(signature);
                    stable_snapshots = 1;
                }
                if stable_snapshots >= DESKTOP_PROCESS_STABLE_SNAPSHOTS {
                    return Ok(tree);
                }
            }
            Err(reason) => {
                previous = None;
                stable_snapshots = 0;
                last_failure = reason;
            }
        }
        if Instant::now() >= deadline {
            return Err(format!(
                "lifecycle_desktop_process_tree_unstable:{last_failure}"
            ));
        }
        std::thread::sleep(DESKTOP_PROCESS_STABLE_INTERVAL);
    }
}

fn capture_desktop_process_tree(
    job: &Job,
    root_process_id: u32,
    expected_token: &TokenEvidence,
) -> Result<DesktopProcessTree, String> {
    let job_process_ids = job.process_ids()?;
    if !job_process_ids.contains(&root_process_id) {
        return Err("lifecycle_desktop_root_not_in_job".to_string());
    }
    let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
    if snapshot == INVALID_HANDLE_VALUE {
        return Err("lifecycle_desktop_process_snapshot_failed".to_string());
    }
    let snapshot = OwnedHandle(snapshot);
    let mut entry: PROCESSENTRY32W = unsafe { zeroed() };
    entry.dwSize = size_of::<PROCESSENTRY32W>() as u32;
    let mut parent_process_ids = std::collections::BTreeMap::new();
    let mut ok = unsafe { Process32FirstW(snapshot.raw(), &mut entry) };
    while ok != 0 {
        if job_process_ids.binary_search(&entry.th32ProcessID).is_ok() {
            parent_process_ids.insert(entry.th32ProcessID, entry.th32ParentProcessID);
        }
        ok = unsafe { Process32NextW(snapshot.raw(), &mut entry) };
    }
    if unsafe { GetLastError() } != ERROR_NO_MORE_FILES {
        return Err("lifecycle_desktop_process_enumeration_failed".to_string());
    }
    if parent_process_ids.len() != job_process_ids.len() {
        return Err("lifecycle_desktop_job_process_churn_detected".to_string());
    }

    let job_process_id_set = job_process_ids
        .iter()
        .copied()
        .collect::<std::collections::BTreeSet<_>>();
    let mut retained = Vec::new();
    let mut webview_process_ids = Vec::new();
    for process_id in job_process_ids {
        if process_id == root_process_id {
            continue;
        }
        let parent_process_id = *parent_process_ids
            .get(&process_id)
            .ok_or_else(|| "lifecycle_desktop_child_parent_missing".to_string())?;
        if parent_process_id == process_id || !job_process_id_set.contains(&parent_process_id) {
            return Err("lifecycle_desktop_child_parent_outside_job".to_string());
        }
        let process = retain_desktop_process(
            process_id,
            Some(parent_process_id),
            "desktop_child",
            Some(expected_token),
        )?;
        if process
            .observation
            .executable_path
            .rsplit('\\')
            .next()
            .is_some_and(|name| name.eq_ignore_ascii_case("msedgewebview2.exe"))
        {
            webview_process_ids.push(process_id);
        }
        retained.push(process);
    }
    retained.sort_by_key(|process| process.observation.process_id);
    webview_process_ids.sort_unstable();
    if retained.is_empty() || webview_process_ids.is_empty() {
        return Err("lifecycle_desktop_process_tree_not_ready".to_string());
    }
    let observations = retained
        .iter()
        .map(|process| process.observation.clone())
        .collect();
    Ok(DesktopProcessTree {
        retained,
        observations,
        webview_process_ids,
    })
}

fn require_file_hash(
    observation: &Observation<FileSnapshot>,
    expected: &str,
    label: &str,
) -> Result<(), String> {
    let snapshot = require_present(observation, label)?;
    if snapshot.sha256 != expected {
        return Err(format!("lifecycle_{label}_hash_mismatch"));
    }
    Ok(())
}

fn require_file_size_and_hash(
    observation: &Observation<FileSnapshot>,
    expected_size: u64,
    expected_sha256: &str,
    label: &str,
) -> Result<(), String> {
    let snapshot = require_present(observation, label)?;
    if snapshot.size != expected_size || snapshot.sha256 != expected_sha256 {
        return Err(format!("lifecycle_{label}_identity_mismatch"));
    }
    Ok(())
}

fn require_present<'a, T>(observation: &'a Observation<T>, label: &str) -> Result<&'a T, String> {
    match observation {
        Observation::Present(value) => Ok(value),
        Observation::Absent => Err(format!("lifecycle_{label}_absent")),
        Observation::Unknown(reason) => Err(format!("lifecycle_{label}_unknown:{reason}")),
    }
}

fn require_absent<T>(observation: &Observation<T>, label: &str) -> Result<(), String> {
    match observation {
        Observation::Absent => Ok(()),
        Observation::Present(_) => Err(format!("lifecycle_{label}_present")),
        Observation::Unknown(reason) => Err(format!("lifecycle_{label}_unknown:{reason}")),
    }
}

fn is_fixed_install_location(value: &str) -> bool {
    value.eq_ignore_ascii_case(INSTALL_ROOT)
        || value
            .strip_prefix('"')
            .and_then(|value| value.strip_suffix('"'))
            .is_some_and(|value| value.eq_ignore_ascii_case(INSTALL_ROOT))
}

fn require_fixed_install_root(
    observation: &Observation<DirectorySnapshot>,
    label: &str,
) -> Result<(), String> {
    let root = require_present(observation, label)?;
    if root.final_path.eq_ignore_ascii_case(INSTALL_ROOT) {
        Ok(())
    } else {
        Err(format!("lifecycle_{label}_final_path_invalid"))
    }
}

fn git_output(repo_root: &Path, args: &[&str], label: &str) -> Result<String, String> {
    let output = Command::new("git")
        .env_clear()
        .env(
            "SystemRoot",
            std::env::var_os("SystemRoot").unwrap_or_default(),
        )
        .env("PATH", std::env::var_os("PATH").unwrap_or_default())
        .arg("-C")
        .arg(repo_root)
        .args(args)
        .output()
        .map_err(|_| format!("lifecycle_git_{label}_start_failed"))?;
    if !output.status.success() {
        return Err(format!("lifecycle_git_{label}_failed"));
    }
    String::from_utf8(output.stdout)
        .map(|value| value.trim().to_string())
        .map_err(|_| format!("lifecycle_git_{label}_utf8_invalid"))
}

fn open_registry_key(root: HKEY, path: &str, access: u32) -> Result<Option<RegistryKey>, String> {
    let path = wide(path);
    let mut key = null_mut();
    let status = unsafe { RegOpenKeyExW(root, path.as_ptr(), 0, access, &mut key) };
    match status {
        ERROR_SUCCESS => Ok(Some(RegistryKey(key))),
        ERROR_FILE_NOT_FOUND | ERROR_PATH_NOT_FOUND => Ok(None),
        _ => Err(format!("registry_open_failed:{status}")),
    }
}

fn read_registry_string(key: HKEY, name: &str) -> Result<String, String> {
    let name = wide(name);
    let mut value_type = 0;
    let mut size = 0;
    let first = unsafe {
        RegQueryValueExW(
            key,
            name.as_ptr(),
            null(),
            &mut value_type,
            null_mut(),
            &mut size,
        )
    };
    if first != ERROR_SUCCESS
        || !matches!(value_type, REG_SZ | REG_EXPAND_SZ)
        || size == 0
        || size > 64 * 1024
        || size % 2 != 0
    {
        return Err(format!("registry_value_size_failed:{first}"));
    }
    let mut buffer = vec![0_u16; size as usize / 2];
    let second = unsafe {
        RegQueryValueExW(
            key,
            name.as_ptr(),
            null(),
            &mut value_type,
            buffer.as_mut_ptr().cast(),
            &mut size,
        )
    };
    if second != ERROR_SUCCESS {
        return Err(format!("registry_value_read_failed:{second}"));
    }
    while buffer.last() == Some(&0) {
        buffer.pop();
    }
    String::from_utf16(&buffer).map_err(|_| "registry_value_utf16_invalid".to_string())
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct TokenEvidence {
    sid: Vec<u8>,
    sid_string: String,
    session_id: u32,
    logon_luid: LogonLuid,
    elevated: bool,
    elevation_type: TOKEN_ELEVATION_TYPE,
}

fn current_token() -> Result<TokenEvidence, String> {
    current_primary_token().map(|(_, token)| token)
}

fn current_primary_token() -> Result<(OwnedHandle, TokenEvidence), String> {
    let mut token = null_mut();
    if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) } == 0 {
        return Err("lifecycle_process_token_open_failed".to_string());
    }
    let token = OwnedHandle(token);
    let evidence = token_evidence(&token)?;
    Ok((token, evidence))
}

fn token_for_process(process: HANDLE) -> Result<TokenEvidence, String> {
    let mut token = null_mut();
    if unsafe { OpenProcessToken(process, TOKEN_QUERY, &mut token) } == 0 {
        return Err("lifecycle_process_token_open_failed".to_string());
    }
    token_evidence(&OwnedHandle(token))
}

fn token_evidence(token: &OwnedHandle) -> Result<TokenEvidence, String> {
    let user = token_user_information(token.raw())?;
    let token_user = unsafe { &*(user.as_ptr().cast::<TOKEN_USER>()) };
    if token_user.User.Sid.is_null() || unsafe { IsValidSid(token_user.User.Sid) } == 0 {
        return Err("lifecycle_token_sid_invalid".to_string());
    }
    let sid_length = unsafe { GetLengthSid(token_user.User.Sid) } as usize;
    if sid_length == 0 {
        return Err("lifecycle_token_sid_empty".to_string());
    }
    let sid = unsafe { std::slice::from_raw_parts(token_user.User.Sid.cast::<u8>(), sid_length) }
        .to_vec();
    let sid_string = sid_string(token_user.User.Sid)?;
    let session_id = token_session_id(token.raw())?;
    let logon_luid = token_logon_luid(token.raw())?;
    let elevated = token_is_elevated(token.raw())?;
    let elevation_type = token_elevation_type(token.raw())?;
    Ok(TokenEvidence {
        sid,
        sid_string,
        session_id,
        logon_luid,
        elevated,
        elevation_type,
    })
}

fn token_logon_luid(token: HANDLE) -> Result<LogonLuid, String> {
    let mut statistics: TOKEN_STATISTICS = unsafe { zeroed() };
    let mut returned = 0;
    if unsafe {
        GetTokenInformation(
            token,
            TokenStatistics,
            (&mut statistics as *mut TOKEN_STATISTICS).cast(),
            size_of::<TOKEN_STATISTICS>() as u32,
            &mut returned,
        )
    } == 0
        || returned < size_of::<TOKEN_STATISTICS>() as u32
    {
        return Err("lifecycle_token_statistics_failed".to_string());
    }
    Ok(LogonLuid {
        low_part: statistics.AuthenticationId.LowPart,
        high_part: statistics.AuthenticationId.HighPart,
    })
}

fn token_user_information(token: HANDLE) -> Result<AlignedBuffer, String> {
    let mut required = 0;
    let first = unsafe { GetTokenInformation(token, TokenUser, null_mut(), 0, &mut required) };
    if first != 0 || required == 0 || unsafe { GetLastError() } != ERROR_INSUFFICIENT_BUFFER {
        return Err("lifecycle_token_query_size_failed".to_string());
    }
    let mut buffer = AlignedBuffer::new(required as usize);
    if unsafe {
        GetTokenInformation(
            token,
            TokenUser,
            buffer.as_mut_ptr().cast(),
            required,
            &mut required,
        )
    } == 0
    {
        return Err("lifecycle_token_query_failed".to_string());
    }
    Ok(buffer)
}

fn token_session_id(token: HANDLE) -> Result<u32, String> {
    let mut session_id = 0;
    let mut returned = 0;
    if unsafe {
        GetTokenInformation(
            token,
            TokenSessionId,
            (&mut session_id as *mut u32).cast(),
            size_of::<u32>() as u32,
            &mut returned,
        )
    } == 0
        || returned != size_of::<u32>() as u32
    {
        return Err("lifecycle_token_session_query_failed".to_string());
    }
    Ok(session_id)
}

fn token_elevation_type(token: HANDLE) -> Result<TOKEN_ELEVATION_TYPE, String> {
    let mut elevation_type = 0;
    let mut returned = 0;
    if unsafe {
        GetTokenInformation(
            token,
            TokenElevationType,
            (&mut elevation_type as *mut TOKEN_ELEVATION_TYPE).cast(),
            size_of::<TOKEN_ELEVATION_TYPE>() as u32,
            &mut returned,
        )
    } == 0
        || returned != size_of::<TOKEN_ELEVATION_TYPE>() as u32
    {
        return Err("lifecycle_token_elevation_type_query_failed".to_string());
    }
    Ok(elevation_type)
}

fn token_is_elevated(token: HANDLE) -> Result<bool, String> {
    let mut elevation = TOKEN_ELEVATION { TokenIsElevated: 0 };
    let mut returned = 0;
    if unsafe {
        GetTokenInformation(
            token,
            TokenElevation,
            (&mut elevation as *mut TOKEN_ELEVATION).cast(),
            size_of::<TOKEN_ELEVATION>() as u32,
            &mut returned,
        )
    } == 0
        || returned != size_of::<TOKEN_ELEVATION>() as u32
    {
        return Err("lifecycle_token_elevation_query_failed".to_string());
    }
    Ok(elevation.TokenIsElevated != 0)
}

fn sid_string(sid: *mut c_void) -> Result<String, String> {
    let mut value = null_mut();
    if unsafe { ConvertSidToStringSidW(sid, &mut value) } == 0 || value.is_null() {
        return Err("lifecycle_token_sid_string_failed".to_string());
    }
    let owned = LocalAllocation(value.cast());
    let mut length = 0;
    while unsafe { *value.add(length) } != 0 {
        length += 1;
    }
    let slice = unsafe { std::slice::from_raw_parts(value, length) };
    let result =
        String::from_utf16(slice).map_err(|_| "lifecycle_token_sid_utf16_invalid".to_string());
    drop(owned);
    result
}

struct ProcessEvidence {
    started_at_100ns: u64,
    token: TokenEvidence,
    image: OwnedFile,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct PeerBinding {
    pub(crate) process_id: u32,
    pub(crate) started_at_100ns: u64,
    image_identity: FileIdentity,
    image_sha256: String,
}

fn process_evidence(process_id: u32) -> Result<ProcessEvidence, String> {
    let process = unsafe {
        OpenProcess(
            PROCESS_QUERY_LIMITED_INFORMATION | SYNCHRONIZE,
            0,
            process_id,
        )
    };
    if process.is_null() {
        return Err("lifecycle_peer_process_open_failed".to_string());
    }
    let process = OwnedHandle(process);
    let started_at_100ns = process_started_at(process.raw())?;
    let token = token_for_process(process.raw())?;
    let image_path = process_image_path(process.raw())?;
    let image = OwnedFile::open_unchecked(Path::new(&image_path), "peer_controller")?;
    Ok(ProcessEvidence {
        started_at_100ns,
        token,
        image,
    })
}

fn process_started_at(process: HANDLE) -> Result<u64, String> {
    let mut created = Default::default();
    let mut exited = Default::default();
    let mut kernel = Default::default();
    let mut user = Default::default();
    if unsafe { GetProcessTimes(process, &mut created, &mut exited, &mut kernel, &mut user) } == 0 {
        return Err("lifecycle_process_times_failed".to_string());
    }
    Ok((u64::from(created.dwHighDateTime) << 32) | u64::from(created.dwLowDateTime))
}

fn process_image_path_by_pid(process_id: u32) -> Result<String, String> {
    let process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, process_id) };
    if process.is_null() {
        return Err("lifecycle_process_path_open_failed".to_string());
    }
    process_image_path(OwnedHandle(process).raw())
}

fn process_image_path(process: HANDLE) -> Result<String, String> {
    let mut buffer = vec![0_u16; 32_768];
    let mut length = buffer.len() as u32;
    if unsafe { QueryFullProcessImageNameW(process, 0, buffer.as_mut_ptr(), &mut length) } == 0 {
        return Err("lifecycle_process_path_failed".to_string());
    }
    buffer.truncate(length as usize);
    String::from_utf16(&buffer).map_err(|_| "lifecycle_process_path_utf16_invalid".to_string())
}

struct FileInformation {
    identity: FileIdentity,
    number_of_links: u32,
}

fn file_information(file: &File) -> std::io::Result<FileInformation> {
    file_information_handle(file.as_raw_handle() as HANDLE)
}

fn file_information_handle(handle: HANDLE) -> std::io::Result<FileInformation> {
    let mut information: BY_HANDLE_FILE_INFORMATION = unsafe { zeroed() };
    if unsafe { GetFileInformationByHandle(handle, &mut information) } == 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(FileInformation {
        identity: FileIdentity {
            volume_serial: information.dwVolumeSerialNumber,
            file_index: (u64::from(information.nFileIndexHigh) << 32)
                | u64::from(information.nFileIndexLow),
        },
        number_of_links: information.nNumberOfLinks,
    })
}

fn transport_file_identity(identity: FileIdentity) -> [u8; 32] {
    let mut digest = Sha256::new();
    digest.update(b"batcave_windows_file_identity_v1");
    digest.update(identity.volume_serial.to_le_bytes());
    digest.update(((identity.file_index >> 32) as u32).to_le_bytes());
    digest.update((identity.file_index as u32).to_le_bytes());
    digest.finalize().into()
}

fn final_path(file: &File) -> std::io::Result<PathBuf> {
    final_path_handle(file.as_raw_handle() as HANDLE)
}

fn final_path_handle(handle: HANDLE) -> std::io::Result<PathBuf> {
    let required = unsafe { GetFinalPathNameByHandleW(handle, null_mut(), 0, 0) };
    if required == 0 || required > 32_768 {
        return Err(std::io::Error::last_os_error());
    }
    let mut buffer = vec![0_u16; required as usize + 1];
    let written =
        unsafe { GetFinalPathNameByHandleW(handle, buffer.as_mut_ptr(), buffer.len() as u32, 0) };
    if written == 0 || written as usize >= buffer.len() {
        return Err(std::io::Error::last_os_error());
    }
    buffer.truncate(written as usize);
    let value = String::from_utf16(&buffer)
        .map_err(|_| std::io::Error::new(std::io::ErrorKind::InvalidData, "invalid utf16"))?;
    let path = crate::collector_service::windows_provisioner::strip_verbatim_disk_prefix(
        PathBuf::from(value),
    );
    if path.to_string_lossy().starts_with(r"\\") {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "network path",
        ));
    }
    Ok(path)
}

fn digest_handle(file: &mut File) -> std::io::Result<[u8; 32]> {
    file.seek(SeekFrom::Start(0))?;
    let mut hash = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hash.update(&buffer[..read]);
    }
    file.seek(SeekFrom::Start(0))?;
    Ok(hash.finalize().into())
}

fn hex_digest(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn hex_nibble(value: u8) -> Option<u8> {
    match value {
        b'0'..=b'9' => Some(value - b'0'),
        b'a'..=b'f' => Some(value - b'a' + 10),
        _ => None,
    }
}

fn create_event(reason: &str) -> Result<OwnedHandle, String> {
    let handle =
        unsafe { windows_sys::Win32::System::Threading::CreateEventW(null(), 1, 0, null()) };
    if handle.is_null() {
        return Err(reason.to_string());
    }
    Ok(OwnedHandle(handle))
}

fn wait_overlapped<T>(
    handle: HANDLE,
    pending: PendingOverlapped<T>,
    timeout_ms: u32,
    timeout_reason: &str,
) -> Result<(PendingOverlapped<T>, u32), String> {
    let wait = unsafe { WaitForSingleObject(pending.event.raw(), timeout_ms) };
    if wait == WAIT_TIMEOUT {
        unsafe { CancelIoEx(handle, pending.as_ptr()) };
        let cancel_wait =
            unsafe { WaitForSingleObject(pending.event.raw(), PROCESS_TREE_SETTLEMENT_TIMEOUT_MS) };
        if cancel_wait != WAIT_OBJECT_0 {
            std::mem::forget(pending);
            return Err(format!("{timeout_reason}_cancel_unsettled"));
        }
        let mut ignored = 0;
        unsafe { GetOverlappedResult(handle, pending.as_ptr(), &mut ignored, 0) };
        return Err(timeout_reason.to_string());
    }
    if wait != WAIT_OBJECT_0 {
        unsafe { CancelIoEx(handle, pending.as_ptr()) };
        let cancel_wait =
            unsafe { WaitForSingleObject(pending.event.raw(), PROCESS_TREE_SETTLEMENT_TIMEOUT_MS) };
        if cancel_wait != WAIT_OBJECT_0 {
            std::mem::forget(pending);
            return Err("lifecycle_pipe_wait_cancel_unsettled".to_string());
        }
        return Err("lifecycle_pipe_wait_failed".to_string());
    }
    let transferred = completed_overlapped_result(handle, &pending)?;
    Ok((pending, transferred))
}

fn completed_overlapped_result<T>(
    handle: HANDLE,
    pending: &PendingOverlapped<T>,
) -> Result<u32, String> {
    let mut transferred = 0;
    if unsafe { GetOverlappedResult(handle, pending.as_ptr(), &mut transferred, 0) } == 0 {
        let error = unsafe { GetLastError() };
        if error == ERROR_BROKEN_PIPE {
            return Ok(0);
        }
        return Err(format!("lifecycle_pipe_overlapped_failed:{error}"));
    }
    Ok(transferred)
}

fn overlapped_read(handle: HANDLE, buffer: &mut [u8], timeout: Duration) -> Result<usize, String> {
    let length =
        u32::try_from(buffer.len()).map_err(|_| "lifecycle_pipe_read_size_invalid".to_string())?;
    let timeout_ms = duration_ms(timeout, "lifecycle_pipe_read_timeout_invalid")?;
    let mut pending = PendingOverlapped::new(
        vec![0_u8; buffer.len()].into_boxed_slice(),
        "lifecycle_pipe_read_event_failed",
    )?;
    let result = unsafe {
        windows_sys::Win32::Storage::FileSystem::ReadFile(
            handle,
            pending.payload.as_mut_ptr().cast(),
            length,
            null_mut(),
            pending.as_mut_ptr(),
        )
    };
    if result != 0 {
        let transferred = completed_overlapped_result(handle, &pending)?;
        return copy_completed_read(&pending.payload, buffer, transferred);
    }
    match unsafe { GetLastError() } {
        ERROR_IO_PENDING => {
            wait_overlapped(handle, pending, timeout_ms, "lifecycle_pipe_read_timeout").and_then(
                |(pending, transferred)| copy_completed_read(&pending.payload, buffer, transferred),
            )
        }
        ERROR_BROKEN_PIPE => Ok(0),
        error => Err(format!("lifecycle_pipe_read_failed:{error}")),
    }
}

fn overlapped_write(handle: HANDLE, buffer: &[u8], timeout: Duration) -> Result<usize, String> {
    let length =
        u32::try_from(buffer.len()).map_err(|_| "lifecycle_pipe_write_size_invalid".to_string())?;
    let timeout_ms = duration_ms(timeout, "lifecycle_pipe_write_timeout_invalid")?;
    let mut pending = PendingOverlapped::new(
        buffer.to_vec().into_boxed_slice(),
        "lifecycle_pipe_write_event_failed",
    )?;
    let result = unsafe {
        windows_sys::Win32::Storage::FileSystem::WriteFile(
            handle,
            pending.payload.as_ptr().cast(),
            length,
            null_mut(),
            pending.as_mut_ptr(),
        )
    };
    if result != 0 {
        let transferred = completed_overlapped_result(handle, &pending)?;
        return validate_transfer_count(transferred, buffer.len(), "write");
    }
    match unsafe { GetLastError() } {
        ERROR_IO_PENDING => {
            wait_overlapped(handle, pending, timeout_ms, "lifecycle_pipe_write_timeout").and_then(
                |(_, transferred)| validate_transfer_count(transferred, buffer.len(), "write"),
            )
        }
        error => Err(format!("lifecycle_pipe_write_failed:{error}")),
    }
}

fn copy_completed_read(
    source: &[u8],
    destination: &mut [u8],
    transferred: u32,
) -> Result<usize, String> {
    let transferred =
        validate_transfer_count(transferred, source.len().min(destination.len()), "read")?;
    destination[..transferred].copy_from_slice(&source[..transferred]);
    Ok(transferred)
}

fn validate_transfer_count(
    transferred: u32,
    buffer_length: usize,
    operation: &str,
) -> Result<usize, String> {
    let transferred = transferred as usize;
    if transferred > buffer_length {
        Err(format!("lifecycle_pipe_{operation}_count_invalid"))
    } else {
        Ok(transferred)
    }
}

fn duration_ms(duration: Duration, reason: &str) -> Result<u32, String> {
    u32::try_from(duration.as_millis()).map_err(|_| reason.to_string())
}

struct PendingOverlapped<T> {
    overlapped: Box<OVERLAPPED>,
    event: OwnedHandle,
    payload: T,
}

impl<T> PendingOverlapped<T> {
    fn new(payload: T, reason: &str) -> Result<Self, String> {
        let event = create_event(reason)?;
        let mut overlapped = Box::new(unsafe { zeroed::<OVERLAPPED>() });
        overlapped.hEvent = event.raw();
        Ok(Self {
            overlapped,
            event,
            payload,
        })
    }

    fn as_ptr(&self) -> *mut OVERLAPPED {
        (&raw const *self.overlapped).cast_mut()
    }

    fn as_mut_ptr(&mut self) -> *mut OVERLAPPED {
        &raw mut *self.overlapped
    }
}

fn wait_handle_until(handle: HANDLE, deadline: Instant, label: &str) -> Result<u32, String> {
    let Some(remaining) = deadline.checked_duration_since(Instant::now()) else {
        return Ok(WAIT_TIMEOUT);
    };
    let wait = unsafe {
        WaitForSingleObject(
            handle,
            duration_ms(remaining, &format!("lifecycle_{label}_wait_invalid"))?,
        )
    };
    Ok(wait)
}

fn valid_evidence_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 96
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_'))
        && !name.contains("..")
}

fn system_directory() -> Result<PathBuf, String> {
    query_windows_directory(GetSystemDirectoryW, "system_directory")
}

fn windows_directory() -> Result<PathBuf, String> {
    query_windows_directory(GetSystemWindowsDirectoryW, "windows_directory")
}

fn query_windows_directory(
    query: unsafe extern "system" fn(*mut u16, u32) -> u32,
    label: &str,
) -> Result<PathBuf, String> {
    let mut buffer = vec![0_u16; WINDOWS_PATH_BUFFER_SIZE];
    let length = unsafe { query(buffer.as_mut_ptr(), buffer.len() as u32) } as usize;
    if length == 0 || length >= buffer.len() {
        return Err(format!("lifecycle_{label}_query_failed"));
    }
    buffer.truncate(length);
    canonical_real_directory(&PathBuf::from(OsString::from_wide(&buffer)), label)
}

fn build_fixed_environment_block(
    system: &Path,
    windows: &Path,
    evidence: &Path,
    command_processor: &Path,
) -> Result<Vec<u16>, String> {
    let entries = [
        ("ComSpec", command_processor),
        ("Path", system),
        ("SystemRoot", windows),
        ("TEMP", evidence),
        ("TMP", evidence),
        ("WINDIR", windows),
    ];
    let mut block = Vec::new();
    for (name, value) in entries {
        let mut entry = OsString::from(name);
        entry.push("=");
        entry.push(value.as_os_str());
        let encoded = entry.encode_wide().collect::<Vec<_>>();
        if encoded.contains(&0) {
            return Err("lifecycle_child_environment_value_invalid".to_string());
        }
        block.extend(encoded);
        block.push(0);
    }
    block.push(0);
    Ok(block)
}

fn current_user_directories(expected_token: &TokenEvidence) -> Result<(PathBuf, PathBuf), String> {
    require_no_thread_token()?;
    let (token, observed_token) = current_primary_token()?;
    if observed_token.sid != expected_token.sid
        || observed_token.session_id != expected_token.session_id
        || observed_token.logon_luid != expected_token.logon_luid
        || observed_token.elevated
        || observed_token.elevation_type != expected_token.elevation_type
    {
        return Err("lifecycle_desktop_profile_token_changed".to_string());
    }
    let profile_path = profile_directory_for_token(&token)?;
    let profile = OwnedDirectory::open(&profile_path, "desktop_profile")?;
    if !paths_equal(&profile.path, &profile_path) {
        return Err("lifecycle_desktop_profile_path_changed".to_string());
    }
    let local_app_data = local_app_data_for_token(&token)?;
    let token_directory = OwnedDirectory::open(&local_app_data, "desktop_local_app_data")?;
    let runtime_path = std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .ok_or_else(|| "lifecycle_desktop_runtime_local_app_data_missing".to_string())?;
    let runtime_directory = OwnedDirectory::open(&runtime_path, "desktop_runtime_local_app_data")?;
    if token_directory.identity != runtime_directory.identity
        || !token_directory
            .path
            .to_string_lossy()
            .eq_ignore_ascii_case(&runtime_directory.path.to_string_lossy())
    {
        return Err("lifecycle_desktop_runtime_local_app_data_mismatch".to_string());
    }
    Ok((profile.path, token_directory.path))
}

fn profile_directory_for_token(token: &OwnedHandle) -> Result<PathBuf, String> {
    let mut required = 0;
    unsafe {
        GetUserProfileDirectoryW(token.raw(), null_mut(), &mut required);
    }
    if required == 0 || unsafe { GetLastError() } != ERROR_INSUFFICIENT_BUFFER {
        return Err("lifecycle_desktop_profile_size_failed".to_string());
    }
    let mut buffer = vec![0_u16; required as usize];
    if unsafe { GetUserProfileDirectoryW(token.raw(), buffer.as_mut_ptr(), &mut required) } == 0 {
        return Err("lifecycle_desktop_profile_query_failed".to_string());
    }
    while buffer.last() == Some(&0) {
        buffer.pop();
    }
    let profile = PathBuf::from(OsString::from_wide(&buffer));
    Ok(profile)
}

fn local_app_data_for_token(token: &OwnedHandle) -> Result<PathBuf, String> {
    let mut value = null_mut();
    let result =
        unsafe { SHGetKnownFolderPath(&FOLDERID_LocalAppData, 0, token.raw(), &mut value) };
    if result < 0 || value.is_null() {
        return Err("lifecycle_parent_local_app_data_query_failed".to_string());
    }
    let mut length = 0_usize;
    while unsafe { *value.add(length) } != 0 {
        length += 1;
        if length > WINDOWS_PATH_BUFFER_SIZE {
            unsafe { CoTaskMemFree(value.cast()) };
            return Err("lifecycle_parent_local_app_data_path_invalid".to_string());
        }
    }
    let path = PathBuf::from(OsString::from_wide(unsafe {
        std::slice::from_raw_parts(value, length)
    }));
    unsafe { CoTaskMemFree(value.cast()) };
    Ok(path)
}

fn build_desktop_environment_block(
    profile: &Path,
    local_app_data: &Path,
    system: &Path,
    windows: &Path,
) -> Result<Vec<u16>, String> {
    let profile = profile
        .to_str()
        .ok_or_else(|| "lifecycle_desktop_profile_utf16_invalid".to_string())?;
    let system = system
        .to_str()
        .ok_or_else(|| "lifecycle_desktop_system_utf16_invalid".to_string())?;
    let windows = windows
        .to_str()
        .ok_or_else(|| "lifecycle_desktop_windows_utf16_invalid".to_string())?;
    let home_drive = profile
        .get(..2)
        .filter(|drive| drive.as_bytes().get(1) == Some(&b':'))
        .ok_or_else(|| "lifecycle_desktop_profile_path_invalid".to_string())?;
    let home_path = profile
        .get(2..)
        .filter(|path| path.starts_with('\\'))
        .ok_or_else(|| "lifecycle_desktop_profile_path_invalid".to_string())?;
    let system_drive = windows
        .get(..2)
        .filter(|drive| drive.as_bytes().get(1) == Some(&b':'))
        .ok_or_else(|| "lifecycle_desktop_windows_path_invalid".to_string())?;
    let local_app_data = local_app_data
        .to_str()
        .ok_or_else(|| "lifecycle_desktop_local_app_data_utf16_invalid".to_string())?;
    let roaming_app_data = Path::new(profile).join("AppData").join("Roaming");
    let temp = Path::new(local_app_data).join("Temp");
    let command_processor = Path::new(system).join("cmd.exe");
    let path = format!("{system};{windows}");
    let entries = [
        ("ALLUSERSPROFILE", format!(r"{system_drive}\ProgramData")),
        ("APPDATA", roaming_app_data.to_string_lossy().into_owned()),
        ("ComSpec", command_processor.to_string_lossy().into_owned()),
        ("HOMEDRIVE", home_drive.to_string()),
        ("HOMEPATH", home_path.to_string()),
        ("LOCALAPPDATA", local_app_data.to_string()),
        ("OS", "Windows_NT".to_string()),
        ("PATH", path),
        ("PATHEXT", ".COM;.EXE;.BAT;.CMD".to_string()),
        ("SystemDrive", system_drive.to_string()),
        ("SystemRoot", windows.to_string()),
        ("TEMP", temp.to_string_lossy().into_owned()),
        ("TMP", temp.to_string_lossy().into_owned()),
        ("USERPROFILE", profile.to_string()),
        ("windir", windows.to_string()),
    ];
    let mut block = Vec::new();
    for (name, value) in entries {
        let encoded = OsString::from(format!("{name}={value}"))
            .encode_wide()
            .collect::<Vec<_>>();
        if encoded.contains(&0) {
            return Err("lifecycle_desktop_environment_value_invalid".to_string());
        }
        block.extend(encoded);
        block.push(0);
    }
    block.push(0);
    Ok(block)
}

fn utf16_z(value: &[u16]) -> String {
    let end = value
        .iter()
        .position(|value| *value == 0)
        .unwrap_or(value.len());
    String::from_utf16_lossy(&value[..end])
}

fn wide(value: impl AsRef<OsStr>) -> Vec<u16> {
    value.as_ref().encode_wide().chain(Some(0)).collect()
}

struct OwnedHandle(HANDLE);

impl OwnedHandle {
    fn raw(&self) -> HANDLE {
        self.0
    }
}

impl Drop for OwnedHandle {
    fn drop(&mut self) {
        unsafe { CloseHandle(self.0) };
    }
}

struct ServiceHandle(SC_HANDLE);

impl ServiceHandle {
    fn raw(&self) -> SC_HANDLE {
        self.0
    }
}

impl Drop for ServiceHandle {
    fn drop(&mut self) {
        unsafe { CloseServiceHandle(self.0) };
    }
}

struct RegistryKey(HKEY);

impl RegistryKey {
    fn raw(&self) -> HKEY {
        self.0
    }
}

impl Drop for RegistryKey {
    fn drop(&mut self) {
        unsafe { RegCloseKey(self.0) };
    }
}

struct LocalAllocation(*mut c_void);

impl Drop for LocalAllocation {
    fn drop(&mut self) {
        unsafe {
            LocalFree(self.0);
        }
    }
}

struct SecurityDescriptor(LocalAllocation);

impl SecurityDescriptor {
    fn from_sddl(value: &str) -> Result<Self, String> {
        let value = wide(value);
        let mut descriptor = null_mut();
        if unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                value.as_ptr(),
                SDDL_REVISION_1,
                &mut descriptor,
                null_mut(),
            )
        } == 0
            || descriptor.is_null()
        {
            return Err("lifecycle_security_descriptor_invalid".to_string());
        }
        Ok(Self(LocalAllocation(descriptor)))
    }

    fn raw(&self) -> *mut c_void {
        self.0 .0
    }
}

struct AlignedBuffer(Vec<usize>);

impl AlignedBuffer {
    fn new(bytes: usize) -> Self {
        Self(vec![0; bytes.div_ceil(size_of::<usize>())])
    }

    fn as_ptr(&self) -> *const usize {
        self.0.as_ptr()
    }

    fn as_mut_ptr(&mut self) -> *mut usize {
        self.0.as_mut_ptr()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct ScratchEvidence {
        evidence: Option<ProtectedEvidenceRoot>,
        root: PathBuf,
    }

    impl ScratchEvidence {
        fn new(label: &str) -> Self {
            let root = std::env::temp_dir().join(format!(
                "batcave-lifecycle-{label}-{}",
                random_hex(16).expect("unique lifecycle scratch")
            ));
            fs::create_dir(&root).expect("create lifecycle scratch");
            let root =
                canonical_real_directory(&root, "scratch").expect("canonical lifecycle scratch");
            let root_wide = wide(root.as_os_str());
            let handle = unsafe {
                CreateFileW(
                    root_wide.as_ptr(),
                    FILE_READ_ATTRIBUTES,
                    FILE_SHARE_READ | FILE_SHARE_WRITE,
                    null(),
                    OPEN_EXISTING,
                    FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT,
                    null_mut(),
                )
            };
            assert_ne!(handle, INVALID_HANDLE_VALUE, "open lifecycle scratch");
            let information =
                file_information_handle(handle).expect("read lifecycle scratch identity");
            Self {
                evidence: Some(ProtectedEvidenceRoot {
                    root: root.clone(),
                    identity: EvidenceRootIdentity {
                        volume_serial: information.identity.volume_serial,
                        file_index: information.identity.file_index,
                    },
                    _handle: OwnedHandle(handle),
                }),
                root,
            }
        }

        fn evidence(&self) -> &ProtectedEvidenceRoot {
            self.evidence.as_ref().expect("scratch evidence")
        }
    }

    impl Drop for ScratchEvidence {
        fn drop(&mut self) {
            drop(self.evidence.take());
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    #[test]
    fn entropy_helpers_emit_canonical_hex() {
        for length in [32, 64] {
            let value = random_hex(length).expect("system entropy");
            assert_eq!(value.len(), length);
            assert!(value
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)));
        }
    }

    #[test]
    fn sha256_parser_accepts_only_canonical_lower_hex() {
        let value = "01".repeat(32);
        let parsed = parse_sha256(&value, "fixture").expect("canonical digest");
        assert_eq!(parsed[0], 1);
        assert_eq!(parsed[31], 1);
        assert_eq!(
            parse_sha256(&"A1".repeat(32), "fixture"),
            Err("lifecycle_fixture_sha256_invalid".to_string())
        );
        assert_eq!(
            parse_sha256("00", "fixture"),
            Err("lifecycle_fixture_sha256_invalid".to_string())
        );
    }

    #[test]
    fn parent_current_user_authority_rejects_token_profile_and_root_drift() {
        let authority = parent_current_user_authority();
        assert_eq!(validate_parent_current_user_authority(&authority), Ok(()));

        let mut token_drift = authority.clone();
        token_drift.logon_luid.low_part = 0;
        token_drift.logon_luid.high_part = 0;
        assert_eq!(
            validate_parent_current_user_authority(&token_drift),
            Err("lifecycle_parent_user_authority_identity_invalid".to_string())
        );

        let mut profile_drift = authority.clone();
        profile_drift.local_app_data.final_path = r"D:\Other\AppData\Local".to_string();
        assert_eq!(
            validate_parent_current_user_authority(&profile_drift),
            Err("lifecycle_parent_user_authority_path_invalid".to_string())
        );

        let mut root_drift = authority.clone();
        root_drift.data_root = Observation::Present(DirectorySnapshot {
            identity: FileIdentity {
                volume_serial: 1,
                file_index: 4,
            },
            final_path: r"C:\Users\proof\AppData\Local\Other".to_string(),
        });
        assert_eq!(
            validate_parent_current_user_authority(&root_drift),
            Err("lifecycle_parent_user_authority_path_invalid".to_string())
        );

        let mut unknown = authority;
        unknown.data_root = Observation::Unknown("access_denied".to_string());
        assert_eq!(
            validate_parent_current_user_authority(&unknown),
            Err("lifecycle_parent_user_authority_path_invalid".to_string())
        );

        let mut missing = parent_current_user_authority();
        missing.data_root = Observation::Absent;
        assert_eq!(
            validate_parent_current_user_authority(&missing),
            Err("lifecycle_parent_user_authority_path_invalid".to_string())
        );
    }

    #[test]
    fn parent_current_user_objects_reject_altered_present_values() {
        let before = parent_current_user_objects();
        assert_eq!(
            validate_parent_current_user_objects_preserved(&before, &before),
            Ok(())
        );

        let mut identity_drift = before.clone();
        let Observation::Present(settings) = &mut identity_drift.settings else {
            panic!("settings");
        };
        settings.identity.file_index += 1;
        assert_eq!(
            validate_parent_current_user_objects_preserved(&before, &identity_drift),
            Err("lifecycle_parent_user_objects_not_preserved".to_string())
        );

        let mut digest_drift = before.clone();
        let Observation::Present(cache) = &mut digest_drift.cache else {
            panic!("cache");
        };
        cache.sha256 = "f".repeat(64);
        assert_eq!(
            validate_parent_current_user_objects_preserved(&before, &digest_drift),
            Err("lifecycle_parent_user_objects_not_preserved".to_string())
        );

        let mut missing = before.clone();
        missing.diagnostics = Observation::Absent;
        assert!(validate_parent_current_user_objects_preserved(&before, &missing).is_err());

        let absent = ParentCurrentUserObjects {
            settings: Observation::Absent,
            cache: Observation::Absent,
            diagnostics: Observation::Absent,
        };
        assert!(validate_parent_current_user_objects_preserved(&absent, &absent).is_err());

        let mut unknown = before.clone();
        unknown.settings = Observation::Unknown("access_denied".to_string());
        assert!(validate_parent_current_user_objects_preserved(&before, &unknown).is_err());
    }

    #[test]
    fn checkpoint_root_pin_blocks_rename_but_leaf_observation_does_not() {
        let canonical_temp =
            crate::collector_service::windows_provisioner::strip_verbatim_disk_prefix(
                fs::canonicalize(std::env::temp_dir()).expect("canonical temp directory"),
            );
        let temporary = canonical_temp.join(format!(
            "BatCave-parent-user-pin-{}-{}",
            std::process::id(),
            random_hex(8).expect("nonce")
        ));
        fs::create_dir(&temporary).expect("temporary directory");
        let root = temporary.join("root");
        let renamed = temporary.join("renamed");
        fs::create_dir(&root).expect("root");
        let leaf = root.join("settings.json");
        fs::write(&leaf, b"settings").expect("leaf");

        let checkpoint_root =
            OwnedDirectory::open_without_delete_sharing(&root, "test_checkpoint_root")
                .expect("checkpoint root");
        let observed = ParentObservedFile::open(&leaf, &checkpoint_root, "test_checkpoint_leaf")
            .expect("observed leaf");
        assert!(fs::rename(&root, &renamed).is_err());

        drop(checkpoint_root);
        fs::remove_file(&leaf).expect("leaf delete remains shared");
        assert!(observed.revalidate().is_err());
        fs::rename(&root, &renamed).expect("root rename after checkpoint pin drops");
        fs::remove_dir(&renamed).expect("renamed root cleanup");
        fs::remove_dir(&temporary).expect("temporary directory cleanup");
    }

    fn parent_current_user_authority() -> ParentCurrentUserAuthority {
        ParentCurrentUserAuthority {
            user_sid: "S-1-5-21-1".to_string(),
            session_id: 1,
            logon_luid: LogonLuid {
                low_part: 2,
                high_part: 0,
            },
            profile: DirectorySnapshot {
                identity: FileIdentity {
                    volume_serial: 1,
                    file_index: 1,
                },
                final_path: r"C:\Users\proof".to_string(),
            },
            local_app_data: DirectorySnapshot {
                identity: FileIdentity {
                    volume_serial: 1,
                    file_index: 2,
                },
                final_path: r"C:\Users\proof\AppData\Local".to_string(),
            },
            resolved_data_root: r"C:\Users\proof\AppData\Local\BatCaveMonitor".to_string(),
            data_root: Observation::Present(DirectorySnapshot {
                identity: FileIdentity {
                    volume_serial: 1,
                    file_index: 3,
                },
                final_path: r"C:\Users\proof\AppData\Local\BatCaveMonitor".to_string(),
            }),
        }
    }

    fn parent_current_user_objects() -> ParentCurrentUserObjects {
        let file = |file_index| {
            Observation::Present(FileSnapshot {
                size: 1,
                sha256: "a".repeat(64),
                identity: FileIdentity {
                    volume_serial: 1,
                    file_index,
                },
            })
        };
        ParentCurrentUserObjects {
            settings: file(10),
            cache: file(11),
            diagnostics: file(12),
        }
    }

    #[test]
    fn evidence_names_are_fixed_leaves() {
        assert!(valid_evidence_name("initial-state.private.json"));
        assert!(!valid_evidence_name("../escape"));
        assert!(!valid_evidence_name("nested/path"));
    }

    #[test]
    fn controller_exclusion_requires_exact_authenticated_process_generation_and_image() {
        let identity = FileIdentity {
            volume_serial: 7,
            file_index: 11,
        };
        let binding = PeerBinding {
            process_id: 41,
            started_at_100ns: 1_000,
            image_identity: identity,
            image_sha256: "a".repeat(64),
        };
        assert!(controller_binding_matches(
            &binding,
            41,
            1_000,
            identity,
            &"a".repeat(64)
        ));
        assert!(!controller_binding_matches(
            &binding,
            41,
            1_001,
            identity,
            &"a".repeat(64)
        ));
        assert!(!controller_binding_matches(
            &binding,
            42,
            1_000,
            identity,
            &"a".repeat(64)
        ));
        assert!(!controller_binding_matches(
            &binding,
            41,
            1_000,
            FileIdentity {
                volume_serial: 7,
                file_index: 12,
            },
            &"a".repeat(64)
        ));
        assert!(!controller_binding_matches(
            &binding,
            41,
            1_000,
            identity,
            &"b".repeat(64)
        ));
        assert!(is_product_process_name("batcave-monitor-cli.exe"));
        assert!(is_product_process_name(
            "batcave-windows-lifecycle-proof.exe"
        ));
        assert!(!is_product_process_name("not-batcave.exe"));
    }

    #[test]
    fn webview_residue_requires_a_bounded_exact_installed_monitor_ancestry() {
        let processes = vec![
            EnumeratedProcess {
                process_id: 10,
                parent_process_id: 1,
                executable_name: "batcave-monitor.exe".to_string(),
            },
            EnumeratedProcess {
                process_id: 11,
                parent_process_id: 10,
                executable_name: "msedgewebview2.exe".to_string(),
            },
            EnumeratedProcess {
                process_id: 12,
                parent_process_id: 11,
                executable_name: "msedgewebview2.exe".to_string(),
            },
            EnumeratedProcess {
                process_id: 20,
                parent_process_id: 1,
                executable_name: "explorer.exe".to_string(),
            },
            EnumeratedProcess {
                process_id: 21,
                parent_process_id: 20,
                executable_name: "msedgewebview2.exe".to_string(),
            },
            EnumeratedProcess {
                process_id: 22,
                parent_process_id: 10,
                executable_name: "unrelated-helper.exe".to_string(),
            },
            EnumeratedProcess {
                process_id: 23,
                parent_process_id: 22,
                executable_name: "msedgewebview2.exe".to_string(),
            },
            EnumeratedProcess {
                process_id: 30,
                parent_process_id: 1,
                executable_name: "batcave-monitor.exe".to_string(),
            },
            EnumeratedProcess {
                process_id: 31,
                parent_process_id: 30,
                executable_name: "msedgewebview2.exe".to_string(),
            },
            EnumeratedProcess {
                process_id: 40,
                parent_process_id: 999,
                executable_name: "msedgewebview2.exe".to_string(),
            },
            EnumeratedProcess {
                process_id: 50,
                parent_process_id: 51,
                executable_name: "msedgewebview2.exe".to_string(),
            },
            EnumeratedProcess {
                process_id: 51,
                parent_process_id: 50,
                executable_name: "msedgewebview2.exe".to_string(),
            },
        ];

        let exact_path = |process_id| match process_id {
            10 => Ok(MONITOR_PATH.to_string()),
            30 => Ok(r"C:\Temp\batcave-monitor.exe".to_string()),
            _ => Err("unexpected process".to_string()),
        };
        assert_eq!(
            is_batcave_owned_webview(&processes[1], &processes, exact_path),
            Ok(true)
        );
        assert_eq!(
            is_batcave_owned_webview(&processes[2], &processes, exact_path),
            Ok(true)
        );
        assert_eq!(
            is_batcave_owned_webview(&processes[4], &processes, exact_path),
            Ok(false)
        );
        assert_eq!(
            is_batcave_owned_webview(&processes[6], &processes, exact_path),
            Ok(false)
        );
        assert_eq!(
            is_batcave_owned_webview(&processes[8], &processes, exact_path),
            Ok(false)
        );
        assert_eq!(
            is_batcave_owned_webview(&processes[9], &processes, exact_path),
            Ok(false)
        );
        assert_eq!(
            is_batcave_owned_webview(&processes[10], &processes, exact_path),
            Ok(false)
        );
        assert_eq!(
            is_batcave_owned_webview(&processes[1], &processes, |_| {
                Err("access denied".to_string())
            }),
            Err("lifecycle_webview_monitor_path_unavailable".to_string())
        );
        assert!(!is_product_process_name("msedgewebview2.exe"));
    }

    #[test]
    fn failure_evidence_receipt_binds_the_exact_leaf_bytes() {
        let scratch = ScratchEvidence::new("receipt");
        let name = "failure.private.json";
        let payload = br#"{"reason":"failed"}"#;
        fs::write(scratch.root.join(name), payload).expect("write receipt probe");
        let receipt = evidence_receipt(name, payload);
        assert_eq!(receipt.size, payload.len() as u64);
        assert_eq!(receipt.sha256.len(), 64);
        let guard =
            verify_evidence_receipt(scratch.evidence(), &receipt).expect("exact bytes must verify");
        assert_eq!(guard.receipt(), &receipt);
        assert_eq!(
            guard
                .read_all_exact("verified_receipt")
                .expect("read exact"),
            payload
        );
        assert!(
            fs::write(scratch.root.join(name), br#"{"reason":"raced"}"#).is_err(),
            "the retained receipt guard must deny a writer racing acknowledgement"
        );
        guard
            .revalidate()
            .expect("retained receipt authority remains exact");
        drop(guard);

        fs::write(scratch.root.join(name), br#"{"reason":"forged"}"#)
            .expect("replace receipt probe");
        assert_eq!(
            verify_evidence_receipt(scratch.evidence(), &receipt).err(),
            Some("lifecycle_failure_evidence_identity_mismatch".to_string())
        );
    }

    #[test]
    fn byte_writer_persists_and_receipts_the_exact_create_new_payload() {
        let scratch = ScratchEvidence::new("byte-writer");
        let name = "exact.private.json";
        let payload = b"{\n  \"shape\": \"pretty\"\n}";
        let receipt = scratch
            .evidence()
            .write_bytes_new(name, payload)
            .expect("write exact byte payload");
        assert_eq!(receipt, evidence_receipt(name, payload));
        assert_eq!(
            fs::read(scratch.root.join(name)).expect("read exact byte payload"),
            payload
        );
        assert_eq!(
            scratch
                .evidence()
                .write_bytes_new(name, b"replacement")
                .err(),
            Some("lifecycle_evidence_create_failed".to_string())
        );
    }

    #[test]
    fn owned_file_blocks_replacement_only_until_the_guard_is_dropped() {
        let scratch = ScratchEvidence::new("owned-file-lifetime");
        let path = scratch.root.join("installed.exe");
        fs::write(&path, b"installed").expect("write installed fixture");
        let guard = OwnedFile::open_unchecked(&path, "installed_fixture")
            .expect("retain installed fixture");
        assert!(
            OpenOptions::new().write(true).open(&path).is_err(),
            "retained source must block replacement"
        );
        drop(guard);
        fs::write(&path, b"replacement").expect("replacement allowed after explicit drop");
    }

    #[test]
    fn install_location_accepts_only_the_fixed_nsis_forms() {
        assert!(is_fixed_install_location(INSTALL_ROOT));
        assert!(is_fixed_install_location(&format!("\"{INSTALL_ROOT}\"")));
        assert!(!is_fixed_install_location(&format!("{INSTALL_ROOT}\\")));
        assert!(!is_fixed_install_location(&format!(" \"{INSTALL_ROOT}\"")));
        assert!(!is_fixed_install_location(
            r#""C:\Program Files\Other App""#
        ));
    }

    #[test]
    fn completed_pipe_reads_copy_only_confirmed_owned_bytes() {
        let source = [1_u8, 2, 3, 4];
        let mut destination = [0_u8; 4];
        assert_eq!(copy_completed_read(&source, &mut destination, 3), Ok(3));
        assert_eq!(destination, [1, 2, 3, 0]);
        assert_eq!(
            copy_completed_read(&source, &mut destination, 5),
            Err("lifecycle_pipe_read_count_invalid".to_string())
        );
    }

    #[test]
    fn child_environment_is_fixed_sorted_and_double_terminated() {
        let block = build_fixed_environment_block(
            Path::new(r"C:\Windows\System32"),
            Path::new(r"C:\Windows"),
            Path::new(r"C:\ProgramData\BatCaveLifecycleProof-v1-test"),
            Path::new(r"C:\Windows\System32\cmd.exe"),
        )
        .expect("fixed environment");
        assert!(block.ends_with(&[0, 0]));
        let entries = block[..block.len() - 1]
            .split(|value| *value == 0)
            .filter(|entry| !entry.is_empty())
            .map(String::from_utf16_lossy)
            .collect::<Vec<_>>();
        assert_eq!(
            entries,
            [
                r"ComSpec=C:\Windows\System32\cmd.exe",
                r"Path=C:\Windows\System32",
                r"SystemRoot=C:\Windows",
                r"TEMP=C:\ProgramData\BatCaveLifecycleProof-v1-test",
                r"TMP=C:\ProgramData\BatCaveLifecycleProof-v1-test",
                r"WINDIR=C:\Windows",
            ]
        );
    }

    #[test]
    fn desktop_environment_is_profile_bound_and_does_not_inherit_process_controls() {
        let block = build_desktop_environment_block(
            Path::new(r"D:\Users\proof-user"),
            Path::new(r"E:\Redirected\Local"),
            Path::new(r"C:\Windows\System32"),
            Path::new(r"C:\Windows"),
        )
        .expect("desktop environment");
        assert!(block.ends_with(&[0, 0]));
        let entries = block[..block.len() - 1]
            .split(|value| *value == 0)
            .filter(|entry| !entry.is_empty())
            .map(String::from_utf16_lossy)
            .collect::<Vec<_>>();
        assert_eq!(
            entries,
            [
                r"ALLUSERSPROFILE=C:\ProgramData",
                r"APPDATA=D:\Users\proof-user\AppData\Roaming",
                r"ComSpec=C:\Windows\System32\cmd.exe",
                r"HOMEDRIVE=D:",
                r"HOMEPATH=\Users\proof-user",
                r"LOCALAPPDATA=E:\Redirected\Local",
                r"OS=Windows_NT",
                r"PATH=C:\Windows\System32;C:\Windows",
                r"PATHEXT=.COM;.EXE;.BAT;.CMD",
                r"SystemDrive=C:",
                r"SystemRoot=C:\Windows",
                r"TEMP=E:\Redirected\Local\Temp",
                r"TMP=E:\Redirected\Local\Temp",
                r"USERPROFILE=D:\Users\proof-user",
                r"windir=C:\Windows",
            ]
        );
        assert!(entries.iter().all(|entry| {
            !entry.starts_with("WEBVIEW2_")
                && !entry.starts_with("BATCAVE_")
                && !entry.starts_with("RUST_")
        }));
    }

    #[test]
    fn transport_identity_matches_the_production_file_identity_domain() {
        let identity = FileIdentity {
            volume_serial: 0x1122_3344,
            file_index: 0x5566_7788_99aa_bbcc,
        };
        let mut digest = Sha256::new();
        digest.update(b"batcave_windows_file_identity_v1");
        digest.update(0x1122_3344_u32.to_le_bytes());
        digest.update(0x5566_7788_u32.to_le_bytes());
        digest.update(0x99aa_bbcc_u32.to_le_bytes());
        let expected: [u8; 32] = digest.finalize().into();
        assert_eq!(transport_file_identity(identity), expected);
    }

    #[test]
    fn launch_failure_preserves_unproven_settlement() {
        assert!(
            !launch_failure_after_settlement(
                "lifecycle_desktop_launch_failed".to_string(),
                Err("lifecycle_desktop_job_settlement_unproven".to_string()),
            )
            .process_tree_settled
        );
        assert!(
            launch_failure_after_settlement("lifecycle_desktop_launch_failed".to_string(), Ok(()),)
                .process_tree_settled
        );
    }

    #[test]
    fn suspended_child_receives_only_fixed_environment_and_working_directory() {
        struct Scratch {
            root: PathBuf,
            output: PathBuf,
        }

        impl Drop for Scratch {
            fn drop(&mut self) {
                let _ = fs::remove_file(&self.output);
                let _ = fs::remove_dir(&self.root);
            }
        }

        let system = system_directory().expect("system directory");
        let windows = windows_directory().expect("Windows directory");
        let root = std::env::temp_dir().join(format!(
            "batcave-lifecycle-child-{}",
            random_hex(16).expect("unique child probe")
        ));
        fs::create_dir(&root).expect("create child probe root");
        let scratch = Scratch {
            output: root.join("child-environment.txt"),
            root,
        };
        let launch =
            FixedLaunchContext::from_paths(&system, &windows, &scratch.root).expect("launch");
        let child = SuspendedChild::spawn(
            &std::env::current_exe().expect("test executable"),
            "--ignored --exact windows_lifecycle_proof::native::tests::fixed_environment_probe_child --nocapture",
            &launch,
            "child_environment_probe",
        )
        .expect("spawn fixed child");
        assert_ne!(unsafe { ResumeThread(child.thread.raw()) }, u32::MAX);
        assert_eq!(
            unsafe { WaitForSingleObject(child.process.raw(), 30_000) },
            WAIT_OBJECT_0
        );
        let mut exit_code = u32::MAX;
        assert_ne!(
            unsafe { GetExitCodeProcess(child.process.raw(), &mut exit_code) },
            0
        );
        assert_eq!(exit_code, 0);
        drop(child);

        let output = fs::read_to_string(&scratch.output).expect("child environment output");
        let lines = output.lines().collect::<Vec<_>>();
        assert_eq!(
            lines[..6],
            [
                format!("ComSpec={}", system.join("cmd.exe").display()),
                format!("Path={}", system.display()),
                format!("SystemRoot={}", windows.display()),
                format!("TEMP={}", scratch.root.display()),
                format!("TMP={}", scratch.root.display()),
                format!("WINDIR={}", windows.display()),
            ]
        );
        assert!(lines[6].eq_ignore_ascii_case(&scratch.root.to_string_lossy()));
    }

    #[test]
    fn failed_children_return_only_after_their_job_is_settled() {
        let executable = OwnedFile::open_current_executable().expect("owned test executable");

        let nonzero_root = ScratchEvidence::new("nonzero-child");
        let nonzero = executable
            .execute(
                nonzero_root.evidence(),
                "--ignored --exact windows_lifecycle_proof::native::tests::fixed_nonzero_child --nocapture",
                Duration::from_secs(30),
                "nonzero_child",
            )
            .expect("nonzero child settlement");
        assert!(matches!(
            nonzero.terminal.terminal,
            ProcessTerminal::Exited { exit_code: 23 }
        ));

        let timeout_root = ScratchEvidence::new("timeout-child");
        let timeout = executable
            .execute(
                timeout_root.evidence(),
                "--ignored --exact windows_lifecycle_proof::native::tests::fixed_timeout_child --nocapture",
                Duration::from_millis(100),
                "timeout_child",
            )
            .expect("timeout child settlement");
        assert_eq!(timeout.terminal.terminal, ProcessTerminal::TimedOut);
        assert_ne!(
            timeout.terminal.active_processes,
            Observation::Present(0),
            "the timeout snapshot must precede Job termination"
        );
    }

    #[test]
    #[ignore = "private child entry for the explicit environment probe"]
    fn fixed_environment_probe_child() {
        let mut environment = std::env::vars_os()
            .map(|(name, value)| format!("{}={}", name.to_string_lossy(), value.to_string_lossy()))
            .collect::<Vec<_>>();
        environment.sort_by_key(|entry| entry.to_ascii_lowercase());
        environment.push(
            std::env::current_dir()
                .expect("probe current directory")
                .to_string_lossy()
                .into_owned(),
        );
        fs::write("child-environment.txt", environment.join("\n"))
            .expect("write child environment");
    }

    #[test]
    #[ignore = "private child entry for process failure settlement"]
    fn fixed_nonzero_child() {
        std::process::exit(23);
    }

    #[test]
    #[ignore = "private child entry for process timeout settlement"]
    fn fixed_timeout_child() {
        std::thread::sleep(Duration::from_secs(30));
    }
}
