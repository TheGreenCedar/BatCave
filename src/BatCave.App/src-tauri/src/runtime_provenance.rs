use std::{env, path::Path};

#[cfg(target_os = "linux")]
use std::process::Command;

use crate::contracts::{
    RuntimeAdminModeState, RuntimeAdminModeStatus, RuntimeEnvironment, RuntimeInstallKind,
    RuntimePlatform, RuntimePrivilegedSource, RuntimeProcessElevation,
};

#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(dead_code)] // Variants are selected by platform-specific token APIs.
enum ProcessElevation {
    Elevated,
    Standard,
    Unavailable(String),
    NotApplicable,
}

#[derive(Debug, Clone)]
pub(crate) struct RuntimeProvenance {
    environment: RuntimeEnvironment,
    process_elevation: ProcessElevation,
}

impl RuntimeProvenance {
    pub(crate) fn detect(base_dir: &Path) -> Self {
        let platform = current_platform();
        let executable = env::current_exe()
            .ok()
            .and_then(|path| path.into_os_string().into_string().ok());
        let nsis_install_evidence = nsis_install_locations();
        let install_kind = classify_install_kind(&InstallEvidence {
            platform,
            executable: executable.as_deref(),
            debug_build: cfg!(debug_assertions),
            appimage: env::var("APPIMAGE").ok().as_deref(),
            nsis_install_locations: &nsis_install_evidence.locations,
            nsis_probe_failed: nsis_install_evidence.probe_failed,
            deb_package_owns_executable: executable
                .as_deref()
                .is_some_and(deb_package_owns_executable),
        });
        let process_elevation = detect_process_elevation();

        Self {
            environment: RuntimeEnvironment {
                platform,
                admin_mode_available: platform == RuntimePlatform::Windows,
                process_elevation: process_elevation.contract_value(),
                install_kind,
                data_directory: Some(base_dir.display().to_string()),
            },
            process_elevation,
        }
    }

    pub(crate) fn environment(&self) -> &RuntimeEnvironment {
        &self.environment
    }

    pub(crate) fn process_is_elevated(&self) -> bool {
        self.process_elevation == ProcessElevation::Elevated
    }

    pub(crate) fn privilege_warning(&self) -> Option<&str> {
        match &self.process_elevation {
            ProcessElevation::Unavailable(detail) => Some(detail),
            _ => None,
        }
    }

    pub(crate) fn admin_mode_status(&self) -> RuntimeAdminModeStatus {
        let (state, source, detail) = match &self.process_elevation {
            ProcessElevation::Elevated => (
                RuntimeAdminModeState::Active,
                RuntimePrivilegedSource::CurrentProcess,
                None,
            ),
            ProcessElevation::Standard => (
                RuntimeAdminModeState::Off,
                RuntimePrivilegedSource::None,
                None,
            ),
            ProcessElevation::Unavailable(detail) => (
                RuntimeAdminModeState::Failed,
                RuntimePrivilegedSource::None,
                Some(detail.clone()),
            ),
            ProcessElevation::NotApplicable => (
                RuntimeAdminModeState::Unavailable,
                RuntimePrivilegedSource::None,
                None,
            ),
        };

        RuntimeAdminModeStatus {
            state,
            source,
            detail,
            last_success_at_ms: None,
            collector_service: None,
        }
    }

    #[cfg(test)]
    pub(crate) fn windows_for_test(process_elevation: RuntimeProcessElevation) -> Self {
        let process_elevation = match process_elevation {
            RuntimeProcessElevation::Elevated => ProcessElevation::Elevated,
            RuntimeProcessElevation::Standard => ProcessElevation::Standard,
            RuntimeProcessElevation::Unknown => {
                ProcessElevation::Unavailable("process_token_elevation_failed error=5".to_string())
            }
            RuntimeProcessElevation::NotApplicable => ProcessElevation::NotApplicable,
        };
        Self {
            environment: RuntimeEnvironment {
                platform: RuntimePlatform::Windows,
                admin_mode_available: true,
                process_elevation: process_elevation.contract_value(),
                install_kind: RuntimeInstallKind::Nsis,
                data_directory: None,
            },
            process_elevation,
        }
    }
}

impl ProcessElevation {
    fn contract_value(&self) -> RuntimeProcessElevation {
        match self {
            Self::Elevated => RuntimeProcessElevation::Elevated,
            Self::Standard => RuntimeProcessElevation::Standard,
            Self::Unavailable(_) => RuntimeProcessElevation::Unknown,
            Self::NotApplicable => RuntimeProcessElevation::NotApplicable,
        }
    }
}

struct InstallEvidence<'a> {
    platform: RuntimePlatform,
    executable: Option<&'a str>,
    debug_build: bool,
    appimage: Option<&'a str>,
    nsis_install_locations: &'a [String],
    nsis_probe_failed: bool,
    deb_package_owns_executable: bool,
}

struct NsisInstallEvidence {
    locations: Vec<String>,
    probe_failed: bool,
}

fn classify_install_kind(evidence: &InstallEvidence<'_>) -> RuntimeInstallKind {
    match evidence.platform {
        RuntimePlatform::Windows => {
            let Some(executable) = evidence.executable else {
                return RuntimeInstallKind::Unknown;
            };
            if evidence
                .nsis_install_locations
                .iter()
                .any(|location| windows_executable_is_installed_at(executable, location))
            {
                RuntimeInstallKind::Nsis
            } else if is_development_runtime(evidence) {
                RuntimeInstallKind::Development
            } else if evidence.nsis_probe_failed {
                RuntimeInstallKind::Unknown
            } else {
                RuntimeInstallKind::Portable
            }
        }
        RuntimePlatform::Linux => {
            if evidence
                .appimage
                .is_some_and(|path| !path.trim().is_empty())
            {
                RuntimeInstallKind::Appimage
            } else if evidence.deb_package_owns_executable {
                RuntimeInstallKind::Deb
            } else if evidence.executable.is_none() {
                RuntimeInstallKind::Unknown
            } else if is_development_runtime(evidence) {
                RuntimeInstallKind::Development
            } else {
                RuntimeInstallKind::Portable
            }
        }
        RuntimePlatform::Macos => {
            let Some(executable) = evidence.executable else {
                return RuntimeInstallKind::Unknown;
            };
            if is_development_runtime(evidence) {
                RuntimeInstallKind::Development
            } else if macos_app_bundle_path(executable) {
                RuntimeInstallKind::AppBundle
            } else {
                RuntimeInstallKind::Portable
            }
        }
        RuntimePlatform::Fixture => RuntimeInstallKind::Portable,
    }
}

fn is_development_runtime(evidence: &InstallEvidence<'_>) -> bool {
    evidence.debug_build
        && evidence.executable.is_some_and(|executable| {
            normalized_path(executable)
                .split('/')
                .any(|component| component.eq_ignore_ascii_case("debug"))
        })
}

fn windows_executable_is_installed_at(executable: &str, install_location: &str) -> bool {
    let executable = normalized_windows_path(executable);
    let install_location = normalized_windows_path(install_location);
    executable
        .rsplit_once('/')
        .is_some_and(|(directory, _)| directory == install_location)
}

fn normalized_windows_path(path: &str) -> String {
    let normalized = path
        .trim()
        .trim_matches('"')
        .replace('\\', "/")
        .trim_end_matches('/')
        .to_lowercase();
    normalized
        .strip_prefix("//?/")
        .unwrap_or(&normalized)
        .to_string()
}

fn normalized_path(path: &str) -> String {
    path.trim().trim_matches('"').replace('\\', "/")
}

fn macos_app_bundle_path(executable: &str) -> bool {
    normalized_path(executable).contains(".app/Contents/MacOS/")
}

fn current_platform() -> RuntimePlatform {
    if cfg!(windows) {
        RuntimePlatform::Windows
    } else if cfg!(target_os = "linux") {
        RuntimePlatform::Linux
    } else if cfg!(target_os = "macos") {
        RuntimePlatform::Macos
    } else {
        RuntimePlatform::Fixture
    }
}

#[cfg(windows)]
fn detect_process_elevation() -> ProcessElevation {
    use std::{ffi::c_void, mem::size_of, ptr::null_mut};

    use windows_sys::Win32::{
        Foundation::{CloseHandle, GetLastError, HANDLE},
        Security::{GetTokenInformation, TokenElevation, TOKEN_ELEVATION, TOKEN_QUERY},
        System::Threading::{GetCurrentProcess, OpenProcessToken},
    };

    struct OwnedHandle(HANDLE);

    impl Drop for OwnedHandle {
        fn drop(&mut self) {
            unsafe {
                CloseHandle(self.0);
            }
        }
    }

    unsafe {
        let mut token = null_mut();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) == 0 {
            return ProcessElevation::Unavailable(format!(
                "process_token_open_failed error={}",
                GetLastError()
            ));
        }
        let token = OwnedHandle(token);
        let mut elevation = TOKEN_ELEVATION::default();
        let mut returned = 0;
        if GetTokenInformation(
            token.0,
            TokenElevation,
            &mut elevation as *mut TOKEN_ELEVATION as *mut c_void,
            size_of::<TOKEN_ELEVATION>() as u32,
            &mut returned,
        ) == 0
        {
            return ProcessElevation::Unavailable(format!(
                "process_token_elevation_failed error={}",
                GetLastError()
            ));
        }

        if elevation.TokenIsElevated != 0 {
            ProcessElevation::Elevated
        } else {
            ProcessElevation::Standard
        }
    }
}

#[cfg(not(windows))]
fn detect_process_elevation() -> ProcessElevation {
    ProcessElevation::NotApplicable
}

#[cfg(windows)]
fn nsis_install_locations() -> NsisInstallEvidence {
    use std::{mem::size_of, ptr::null_mut};

    use windows_sys::Win32::{
        Foundation::{ERROR_FILE_NOT_FOUND, ERROR_PATH_NOT_FOUND, ERROR_SUCCESS},
        System::Registry::{
            RegCloseKey, RegOpenKeyExW, RegQueryValueExW, HKEY, HKEY_CURRENT_USER,
            HKEY_LOCAL_MACHINE, KEY_READ, KEY_WOW64_32KEY, KEY_WOW64_64KEY, REG_EXPAND_SZ, REG_SZ,
        },
    };

    const UNINSTALL_KEY: &str =
        r"Software\Microsoft\Windows\CurrentVersion\Uninstall\BatCave Monitor";
    const INSTALL_LOCATION: &str = "InstallLocation";

    fn wide(value: &str) -> Vec<u16> {
        value.encode_utf16().chain(std::iter::once(0)).collect()
    }

    enum RegistryInstallLocation {
        Absent,
        Found(String),
        Unavailable,
    }

    unsafe fn read_install_location(root: HKEY, view: u32) -> RegistryInstallLocation {
        let subkey = wide(UNINSTALL_KEY);
        let value_name = wide(INSTALL_LOCATION);
        let mut key = null_mut();
        let open_status =
            unsafe { RegOpenKeyExW(root, subkey.as_ptr(), 0, KEY_READ | view, &mut key) };
        if matches!(open_status, ERROR_FILE_NOT_FOUND | ERROR_PATH_NOT_FOUND) {
            return RegistryInstallLocation::Absent;
        }
        if open_status != ERROR_SUCCESS {
            return RegistryInstallLocation::Unavailable;
        }

        let mut value_type = 0;
        let mut byte_count = 0;
        let size_status = unsafe {
            RegQueryValueExW(
                key,
                value_name.as_ptr(),
                null_mut(),
                &mut value_type,
                null_mut(),
                &mut byte_count,
            )
        };
        if size_status != ERROR_SUCCESS
            || !matches!(value_type, REG_SZ | REG_EXPAND_SZ)
            || byte_count < 2
        {
            unsafe { RegCloseKey(key) };
            return RegistryInstallLocation::Unavailable;
        }

        let mut buffer = vec![0_u16; byte_count as usize / size_of::<u16>()];
        let read_status = unsafe {
            RegQueryValueExW(
                key,
                value_name.as_ptr(),
                null_mut(),
                &mut value_type,
                buffer.as_mut_ptr().cast(),
                &mut byte_count,
            )
        };
        unsafe { RegCloseKey(key) };
        if read_status != ERROR_SUCCESS {
            return RegistryInstallLocation::Unavailable;
        }

        let length = buffer
            .iter()
            .position(|value| *value == 0)
            .unwrap_or(buffer.len());
        match String::from_utf16(&buffer[..length]) {
            Ok(location) if !location.trim().is_empty() => RegistryInstallLocation::Found(location),
            _ => RegistryInstallLocation::Unavailable,
        }
    }

    let mut locations = Vec::new();
    let mut probe_failed = false;
    for root in [HKEY_LOCAL_MACHINE, HKEY_CURRENT_USER] {
        for view in [KEY_WOW64_64KEY, KEY_WOW64_32KEY] {
            match unsafe { read_install_location(root, view) } {
                RegistryInstallLocation::Found(location) => {
                    if !locations.contains(&location) {
                        locations.push(location);
                    }
                }
                RegistryInstallLocation::Unavailable => probe_failed = true,
                RegistryInstallLocation::Absent => {}
            }
        }
    }
    NsisInstallEvidence {
        locations,
        probe_failed,
    }
}

#[cfg(not(windows))]
fn nsis_install_locations() -> NsisInstallEvidence {
    NsisInstallEvidence {
        locations: Vec::new(),
        probe_failed: false,
    }
}

#[cfg(target_os = "linux")]
fn deb_package_owns_executable(executable: &str) -> bool {
    if executable.is_empty() {
        return false;
    }
    let Ok(output) = Command::new("dpkg-query")
        .arg("-S")
        .arg(executable)
        .output()
    else {
        return false;
    };
    output.status.success()
        && String::from_utf8_lossy(&output.stdout).lines().any(|line| {
            line.rsplit_once(": ")
                .is_some_and(|(_, path)| path == executable)
        })
}

#[cfg(not(target_os = "linux"))]
fn deb_package_owns_executable(_executable: &str) -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows_installed_provenance_requires_matching_registry_location() {
        let locations = vec![r"C:\Program Files\BatCave Monitor".to_string()];
        let installed = InstallEvidence {
            platform: RuntimePlatform::Windows,
            executable: Some(r"C:\Program Files\BatCave Monitor\batcave-monitor.exe"),
            debug_build: false,
            appimage: None,
            nsis_install_locations: &locations,
            nsis_probe_failed: false,
            deb_package_owns_executable: false,
        };
        let copied = InstallEvidence {
            executable: Some(r"D:\Tools\batcave-monitor.exe"),
            ..installed
        };

        assert_eq!(classify_install_kind(&installed), RuntimeInstallKind::Nsis);
        assert_eq!(classify_install_kind(&copied), RuntimeInstallKind::Portable);
    }

    #[test]
    fn windows_development_and_portable_builds_remain_distinct() {
        let development = InstallEvidence {
            platform: RuntimePlatform::Windows,
            executable: Some(r"C:\src\BatCave\target\debug\batcave-monitor.exe"),
            debug_build: true,
            appimage: None,
            nsis_install_locations: &[],
            nsis_probe_failed: false,
            deb_package_owns_executable: false,
        };
        let portable = InstallEvidence {
            debug_build: false,
            ..development
        };

        assert_eq!(
            classify_install_kind(&development),
            RuntimeInstallKind::Development
        );
        assert_eq!(
            classify_install_kind(&portable),
            RuntimeInstallKind::Portable
        );
    }

    #[test]
    fn windows_missing_executable_or_failed_registry_probe_is_unknown() {
        let unavailable_executable = InstallEvidence {
            platform: RuntimePlatform::Windows,
            executable: None,
            debug_build: false,
            appimage: None,
            nsis_install_locations: &[],
            nsis_probe_failed: false,
            deb_package_owns_executable: false,
        };
        let unavailable_registry = InstallEvidence {
            executable: Some(r"D:\Tools\batcave-monitor.exe"),
            nsis_probe_failed: true,
            ..unavailable_executable
        };

        assert_eq!(
            classify_install_kind(&unavailable_executable),
            RuntimeInstallKind::Unknown
        );
        assert_eq!(
            classify_install_kind(&unavailable_registry),
            RuntimeInstallKind::Unknown
        );
    }

    #[test]
    fn windows_positive_install_match_survives_an_unrelated_probe_failure() {
        let locations = vec![r"C:\Program Files\BatCave Monitor".to_string()];
        let installed = InstallEvidence {
            platform: RuntimePlatform::Windows,
            executable: Some(r"C:\Program Files\BatCave Monitor\batcave-monitor.exe"),
            debug_build: false,
            appimage: None,
            nsis_install_locations: &locations,
            nsis_probe_failed: true,
            deb_package_owns_executable: false,
        };

        assert_eq!(classify_install_kind(&installed), RuntimeInstallKind::Nsis);
    }

    #[test]
    fn windows_registry_match_is_case_quote_and_separator_insensitive() {
        assert!(windows_executable_is_installed_at(
            r"C:\PROGRAM FILES\BatCave Monitor\batcave-monitor.exe",
            r#""c:/Program Files/BatCave Monitor/""#,
        ));
    }

    #[test]
    fn linux_package_evidence_precedes_development_fallback() {
        let appimage = InstallEvidence {
            platform: RuntimePlatform::Linux,
            executable: Some("/tmp/.mount_BatCave/batcave-monitor"),
            debug_build: false,
            appimage: Some("/home/test/BatCave.AppImage"),
            nsis_install_locations: &[],
            nsis_probe_failed: false,
            deb_package_owns_executable: false,
        };
        let deb = InstallEvidence {
            executable: Some("/usr/bin/batcave-monitor"),
            appimage: None,
            deb_package_owns_executable: true,
            ..appimage
        };

        assert_eq!(
            classify_install_kind(&appimage),
            RuntimeInstallKind::Appimage
        );
        assert_eq!(classify_install_kind(&deb), RuntimeInstallKind::Deb);
    }

    #[test]
    fn macos_distinguishes_development_app_bundle_and_standalone_binary() {
        let development = InstallEvidence {
            platform: RuntimePlatform::Macos,
            executable: Some("/repo/target/debug/batcave-monitor"),
            debug_build: true,
            appimage: None,
            nsis_install_locations: &[],
            nsis_probe_failed: false,
            deb_package_owns_executable: false,
        };
        let app_bundle = InstallEvidence {
            executable: Some("/Applications/BatCave Monitor.app/Contents/MacOS/batcave-monitor"),
            debug_build: false,
            ..development
        };
        let standalone = InstallEvidence {
            executable: Some("/usr/local/bin/batcave-monitor"),
            ..app_bundle
        };

        assert_eq!(
            classify_install_kind(&development),
            RuntimeInstallKind::Development
        );
        assert_eq!(
            classify_install_kind(&app_bundle),
            RuntimeInstallKind::AppBundle
        );
        assert_eq!(
            classify_install_kind(&standalone),
            RuntimeInstallKind::Portable
        );
    }

    #[test]
    fn unsupported_platform_has_explicit_privilege_absence() {
        let provenance = RuntimeProvenance {
            environment: RuntimeEnvironment {
                platform: RuntimePlatform::Macos,
                admin_mode_available: false,
                process_elevation: RuntimeProcessElevation::NotApplicable,
                install_kind: RuntimeInstallKind::AppBundle,
                data_directory: None,
            },
            process_elevation: ProcessElevation::NotApplicable,
        };

        assert_eq!(
            provenance.admin_mode_status().state,
            RuntimeAdminModeState::Unavailable
        );
        assert!(!provenance.environment().admin_mode_available);
    }

    #[test]
    fn windows_token_result_owns_active_or_standard_state() {
        let environment = RuntimeEnvironment {
            platform: RuntimePlatform::Windows,
            admin_mode_available: true,
            process_elevation: RuntimeProcessElevation::Standard,
            install_kind: RuntimeInstallKind::Nsis,
            data_directory: None,
        };
        let elevated = RuntimeProvenance {
            environment: RuntimeEnvironment {
                process_elevation: RuntimeProcessElevation::Elevated,
                ..environment.clone()
            },
            process_elevation: ProcessElevation::Elevated,
        };
        let standard = RuntimeProvenance {
            environment,
            process_elevation: ProcessElevation::Standard,
        };

        assert!(elevated.process_is_elevated());
        assert_eq!(
            elevated.admin_mode_status().state,
            RuntimeAdminModeState::Active
        );
        assert_eq!(
            elevated.admin_mode_status().source,
            RuntimePrivilegedSource::CurrentProcess
        );
        assert_eq!(
            elevated.environment().process_elevation,
            RuntimeProcessElevation::Elevated
        );
        assert!(!standard.process_is_elevated());
        assert_eq!(
            standard.admin_mode_status().state,
            RuntimeAdminModeState::Off
        );
        assert_eq!(
            standard.admin_mode_status().source,
            RuntimePrivilegedSource::None
        );
    }

    #[test]
    fn unavailable_windows_token_never_claims_elevation() {
        let provenance = RuntimeProvenance {
            environment: RuntimeEnvironment {
                platform: RuntimePlatform::Windows,
                admin_mode_available: true,
                process_elevation: RuntimeProcessElevation::Unknown,
                install_kind: RuntimeInstallKind::Portable,
                data_directory: None,
            },
            process_elevation: ProcessElevation::Unavailable(
                "process_token_elevation_failed error=5".to_string(),
            ),
        };

        assert!(!provenance.process_is_elevated());
        assert_eq!(
            provenance.privilege_warning(),
            Some("process_token_elevation_failed error=5")
        );
        assert_eq!(
            provenance.admin_mode_status().state,
            RuntimeAdminModeState::Failed
        );
        assert_eq!(
            provenance.environment().process_elevation,
            RuntimeProcessElevation::Unknown
        );
    }
}
