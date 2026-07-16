use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};
use windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT;

use super::{etw_lease::ProtectedEtwLeaseRoot, protocol::COLLECTOR_SERVICE_NAME};

const PROVISION_SWITCH: &str = "--provision";
const PRODUCT_DIRECTORY_NAME: &str = "BatCave Monitor";
const SERVICE_EXECUTABLE_NAME: &str = "batcave-collector-service.exe";
const LEGACY_WINDOWS_CLI_NAME: &str = "batcave-monitor-cli.exe";
const SERVICE_ACCOUNT: &str = "LocalSystem";
const SERVICE_OWNER_MARKER: &str = "dev.batcave.monitor/service-v1";
const SERVICE_FAILURE_VALUE: &str = "BatCaveLastFailure";
const SERVICE_TYPE_OWN_PROCESS: u32 = 0x10;
const ERROR_FILE_NOT_FOUND_CODE: u32 = 2;
const ERROR_PATH_NOT_FOUND_CODE: u32 = 3;

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

fn missing_service_cleanup_required(product_root_exists: bool, service_root_exists: bool) -> bool {
    product_root_exists || service_root_exists
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ProvisionVerb {
    PrepareUpgrade,
    Install,
    Uninstall,
}

pub(crate) fn run_cli(args: &[String]) -> Option<i32> {
    let verb = match args {
        [switch, verb] if switch == PROVISION_SWITCH => match verb.as_str() {
            "prepare-upgrade" => ProvisionVerb::PrepareUpgrade,
            "install" => ProvisionVerb::Install,
            "uninstall" => ProvisionVerb::Uninstall,
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
        ProvisionVerb::Install => native::install(),
        ProvisionVerb::Uninstall => native::uninstall(),
    }
}

pub(crate) fn open_protected_etw_lease_root() -> Result<ProtectedEtwLeaseRoot, String> {
    native::open_protected_etw_lease_root()
}

pub(crate) fn record_service_failure(category: &str) {
    let _ = native::record_service_failure(category);
}

pub(crate) fn clear_service_failure() {
    let _ = native::clear_service_failure();
}

pub(crate) fn acquire_service_lifecycle_marker() -> Result<impl std::fmt::Debug, String> {
    native::acquire_service_lifecycle_marker()
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

fn validate_current_service_path(current: &Path, program_files: &Path) -> Result<(), String> {
    if fixed_path_eq(current, &expected_service_path(program_files)) {
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

fn strip_verbatim_disk_prefix(path: PathBuf) -> PathBuf {
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
        mem::size_of,
        os::windows::ffi::{OsStrExt, OsStringExt},
        ptr, thread,
        time::{Duration, Instant},
    };

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
                GetSecurityInfo, SE_FILE_OBJECT,
            },
            CreateWellKnownSid, EqualSid, GetAce, GetAclInformation, GetSecurityDescriptorControl,
            GetSecurityDescriptorDacl, GetTokenInformation, LookupAccountNameW,
            LookupPrivilegeValueW, TokenElevation, WinBuiltinAdministratorsSid, WinInteractiveSid,
            WinLocalSystemSid, ACCESS_ALLOWED_ACE, ACL_SIZE_INFORMATION, CONTAINER_INHERIT_ACE,
            DACL_SECURITY_INFORMATION, INHERIT_ONLY_ACE, LUID_AND_ATTRIBUTES, OBJECT_INHERIT_ACE,
            OWNER_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR, PSID, SECURITY_ATTRIBUTES,
            SECURITY_MAX_SID_SIZE, SE_DACL_PROTECTED, SE_PRIVILEGE_ENABLED, SID_NAME_USE,
            TOKEN_ADJUST_PRIVILEGES, TOKEN_ELEVATION, TOKEN_PRIVILEGES, TOKEN_QUERY,
        },
        Storage::FileSystem::{
            CreateDirectoryW, CreateFileW, DeleteFileW, FileDispositionInfo,
            GetFileInformationByHandle, GetFinalPathNameByHandleW, ReadFile, RemoveDirectoryW,
            SetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION, DELETE, FILE_ADD_SUBDIRECTORY,
            FILE_APPEND_DATA, FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_REPARSE_POINT,
            FILE_DELETE_CHILD, FILE_DISPOSITION_INFO, FILE_FLAG_BACKUP_SEMANTICS,
            FILE_FLAG_OPEN_REPARSE_POINT, FILE_READ_ATTRIBUTES, FILE_SHARE_DELETE, FILE_SHARE_READ,
            FILE_SHARE_WRITE, FILE_WRITE_DATA, OPEN_ALWAYS, OPEN_EXISTING, READ_CONTROL, WRITE_DAC,
            WRITE_OWNER,
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
                GetCurrentProcess, OpenProcess, OpenProcessToken, WaitForSingleObject,
                PROCESS_QUERY_LIMITED_INFORMATION,
            },
        },
        UI::Shell::{
            SHGetFolderPathW, CSIDL_COMMON_APPDATA, CSIDL_PROGRAM_FILES, SHGFP_TYPE_CURRENT,
        },
    };

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
    const SERVICE_REQUIRED_PRIVILEGES: [&str; 2] =
        ["SeChangeNotifyPrivilege", "SeSystemProfilePrivilege"];
    const SERVICE_OPERATION_TIMEOUT: Duration = Duration::from_secs(30);
    const SERVICE_POLL_INTERVAL: Duration = Duration::from_millis(100);
    const SERVICE_QUERY_STATUS_MASK: u32 = 0x0000_0004;
    const SERVICE_LIFECYCLE_FILE_NAME: &str = "process-owner.v1.lock";
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

    impl Drop for OwnedRegistryKey {
        fn drop(&mut self) {
            unsafe {
                RegCloseKey(self.0);
            }
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
            principals.service = Some(account_sid(&format!(
                "NT SERVICE\\{COLLECTOR_SERVICE_NAME}"
            ))?);
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
        let path = wide_path(&roots.service.join(SERVICE_LIFECYCLE_FILE_NAME));
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
        let path = wide_path(&roots.service.join(SERVICE_LIFECYCLE_FILE_NAME));
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

        let size = (u64::from(info.nFileSizeHigh) << 32) | u64::from(info.nFileSizeLow);
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

    pub(super) fn install() -> Result<(), String> {
        require_elevated()?;
        let image = verify_current_binary_path()?;
        let manager = open_manager(SC_MANAGER_CONNECT | SC_MANAGER_CREATE_SERVICE)?;
        if let Some(service) = open_service(&manager, SERVICE_ALL_ACCESS)? {
            validate_service_contract(&service, image.path())?;
            let _protected_root = open_protected_etw_lease_root()?;
            start_service_and_wait(&service)?;
            validate_service_contract(&service, image.path())?;
            return retire_legacy_cli(&image);
        }

        let service = create_service(&manager, image.path())?;
        let mut roots_created = RootCreationJournal::default();
        let install_result = (|| {
            configure_new_service(&service)?;
            set_owner_marker()?;
            provision_roots(&mut roots_created)?;
            validate_service_contract(&service, image.path())?;
            start_service_and_wait(&service)?;
            validate_service_contract(&service, image.path())
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
        retire_legacy_cli(&image)
    }

    pub(super) fn uninstall() -> Result<(), String> {
        require_elevated()?;
        let image = verify_current_binary_path()?;
        let manager = open_manager(SC_MANAGER_CONNECT)?;
        let Some(service) = open_service(&manager, SERVICE_ALL_ACCESS)? else {
            let roots = fixed_roots()?;
            if missing_service_cleanup_required(
                path_exists_no_follow(&roots.product)?,
                path_exists_no_follow(&roots.service)?,
            ) {
                cleanup_roots_if_owned(true, &SecurityPrincipals::load_with_service()?)?;
            }
            return retire_legacy_cli(&image);
        };
        validate_service_contract(&service, image.path())?;
        let was_running = query_service_status(&service)?.dwCurrentState == SERVICE_RUNNING;
        let _protected_root = open_protected_etw_lease_root()?;
        stop_service_and_wait(&service, true)?;
        let principals = SecurityPrincipals::load_with_service()?;
        if unsafe { DeleteService(service.raw()) } == 0 {
            let delete_error = last_error("collector_service_delete_failed");
            if was_running {
                return match start_service_and_wait(&service) {
                    Ok(()) => Err(delete_error),
                    Err(restart) => Err(format!(
                        "{delete_error};collector_service_restart_failed:{restart}"
                    )),
                };
            }
            return Err(delete_error);
        }
        drop(service);
        wait_service_deleted(&manager)?;
        drop(_protected_root);
        cleanup_roots_if_owned(true, &principals)?;
        retire_legacy_cli(&image)
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

    fn validate_service_dacl(service: &OwnedScHandle) -> Result<(), String> {
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
        let aces = read_aces(dacl, &principals)?;
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
                return Err("collector_service_delete_timeout".to_string());
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
            SERVICE_LIFECYCLE_FILE_NAME,
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
        for leaf in [ETW_LEASE_FILE_NAME, ETW_OWNER_LOCK_FILE_NAME] {
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
            if error == ERROR_FILE_NOT_FOUND {
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
        drop(security);
        Ok(SecurityPolicy {
            owner,
            dacl_protected: control & SE_DACL_PROTECTED != 0,
            reparse: false,
            aces,
        })
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
    }

    fn verify_current_binary_path() -> Result<VerifiedServiceImage, String> {
        let program_files = known_folder(CSIDL_PROGRAM_FILES)?;
        let current = std::env::current_exe()
            .map_err(|error| format!("collector_service_executable_path_failed:{error}"))?;
        validate_current_service_path(&current, &program_files)?;
        let install_dir = current
            .parent()
            .ok_or_else(|| "collector_service_install_directory_missing".to_string())?;
        let principals = SecurityPrincipals::load_base()?;
        let program_files_handle = open_directory(
            &program_files,
            "collector_service_program_files_open_failed",
        )?;
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
        let image = open_file(&current, "collector_service_executable_open_failed")?;
        if !fixed_path_eq(
            &final_path(&image, "collector_service_executable_final_path_failed")?,
            &current,
        ) {
            return Err("collector_service_executable_identity_invalid".to_string());
        }
        validate_no_untrusted_writer(image.raw(), &principals, false, false)?;
        Ok(VerifiedServiceImage {
            path: current,
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
        assert_eq!(
            validate_current_service_path(&expected, program_files),
            Ok(())
        );
        assert_eq!(
            validate_current_service_path(
                Path::new(r"C:\Users\standard\BatCave Monitor\batcave-collector-service.exe"),
                program_files,
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
    }

    #[test]
    fn absent_leaf_and_absent_parent_are_both_missing_paths() {
        assert!(is_missing_path_error(ERROR_FILE_NOT_FOUND_CODE));
        assert!(is_missing_path_error(ERROR_PATH_NOT_FOUND_CODE));
        assert!(!is_missing_path_error(5));
    }

    #[test]
    fn missing_service_cleanup_retries_only_when_owned_roots_remain() {
        assert!(!missing_service_cleanup_required(false, false));
        assert!(missing_service_cleanup_required(true, false));
        assert!(missing_service_cleanup_required(false, true));
        assert!(missing_service_cleanup_required(true, true));
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
        assert_eq!(
            native::validate_clean_stopped_status(&failed),
            Err("collector_service_stop_reported_failure:1066:1".to_string())
        );

        let stale_specific = SERVICE_STATUS_PROCESS {
            dwServiceSpecificExitCode: 9,
            ..clean
        };
        assert_eq!(
            native::validate_clean_stopped_status(&stale_specific),
            Ok(())
        );

        let running = SERVICE_STATUS_PROCESS {
            dwCurrentState: SERVICE_RUNNING,
            ..clean
        };
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
}
