use std::{
    fs,
    path::{Path, PathBuf},
    thread,
    time::Duration,
};

#[cfg(windows)]
use std::{ffi::OsStr, iter::once, os::windows::ffi::OsStrExt};

#[cfg(windows)]
use windows_sys::Win32::Foundation::HANDLE;

use serde::{Deserialize, Serialize};

use crate::{
    atomic_json::{write_json_atomic, AtomicJsonErrorLabels},
    cli_args,
    contracts::ProcessSample,
    telemetry::TelemetryCollector,
};

const HELPER_INTERVAL_MS: u64 = 500;
const HELPER_TOKEN_BYTES: usize = 32;
const ADMIN_SNAPSHOT_JSON_ERRORS: AtomicJsonErrorLabels = AtomicJsonErrorLabels {
    write_failed: "admin_mode_snapshot_write_failed",
    serialize_failed: "admin_mode_snapshot_serialize_failed",
    replace_failed: "admin_mode_snapshot_replace_failed",
    rename_failed: "admin_mode_snapshot_rename_failed",
    serialize_error_includes_path: false,
};

#[derive(Debug)]
pub struct ElevatedHelperClient {
    data_file: PathBuf,
    stop_file: PathBuf,
    token: String,
    last_seq: u64,
    process: Option<ElevatedHelperProcess>,
}

impl ElevatedHelperClient {
    pub fn start(base_dir: &Path) -> Result<Self, String> {
        remove_helper_artifacts(base_dir);
        let session = prepare_helper_session(base_dir)?;

        let process =
            launch_elevated_helper(&session.data_file, &session.stop_file, &session.token)?;

        Ok(Self {
            data_file: session.data_file,
            stop_file: session.stop_file,
            token: session.token,
            last_seq: 0,
            process,
        })
    }

    pub fn poll_rows(&mut self) -> Result<Option<Vec<ProcessSample>>, String> {
        if self
            .process
            .as_ref()
            .is_some_and(ElevatedHelperProcess::has_exited)
        {
            remove_artifacts_for_snapshot(&self.data_file);
            return Err("admin_mode_helper_exited".to_string());
        }

        if !self.data_file.exists() {
            return Ok(None);
        }

        let payload = fs::read_to_string(&self.data_file).map_err(|error| {
            format!(
                "admin_mode_snapshot_read_failed path={} error={}",
                self.data_file.display(),
                error
            )
        })?;
        let snapshot = serde_json::from_str::<ElevatedSnapshot>(&payload)
            .map_err(|error| format!("admin_mode_snapshot_parse_failed:{error}"))?;

        if snapshot.token != self.token {
            return Err("admin_mode_snapshot_token_mismatch".to_string());
        }
        if snapshot.seq <= self.last_seq {
            return Ok(None);
        }

        self.last_seq = snapshot.seq;
        Ok(Some(snapshot.rows))
    }

    pub fn stop(&self) {
        let _ = fs::write(&self.stop_file, "stop");
        if self
            .process
            .as_ref()
            .is_none_or(|process| process.wait(Duration::from_millis(2_000)))
        {
            remove_artifacts_for_snapshot(&self.data_file);
        } else {
            remove_snapshot_artifacts(&self.data_file);
        }
    }

    pub fn remove_stale_artifacts(base_dir: &Path) {
        remove_helper_artifacts(base_dir);
    }
}

impl Drop for ElevatedHelperClient {
    fn drop(&mut self) {
        self.stop();
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
struct ElevatedSnapshot {
    token: String,
    seq: u64,
    rows: Vec<ProcessSample>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ElevatedHelperArgs {
    data_file: PathBuf,
    stop_file: PathBuf,
    token: String,
}

pub fn run_cli(args: &[String]) -> Option<i32> {
    if !args.iter().any(|arg| arg == "--elevated-helper") {
        return None;
    }

    match parse_helper_args(args) {
        Ok(helper_args) => Some(run_elevated_helper(helper_args)),
        Err(error) => {
            eprintln!("{error}");
            Some(2)
        }
    }
}

fn run_elevated_helper(args: ElevatedHelperArgs) -> i32 {
    let mut seq = 0_u64;
    let collector = TelemetryCollector::new();
    while !args.stop_file.exists() {
        seq = seq.saturating_add(1);
        match collector
            .collect()
            .map(|sample| sample.processes)
            .and_then(|rows| write_snapshot(&args.data_file, &args.token, seq, rows))
        {
            Ok(()) => {}
            Err(error) => {
                eprintln!("{error}");
                return 1;
            }
        }
        thread::sleep(Duration::from_millis(HELPER_INTERVAL_MS));
    }

    remove_snapshot_artifacts(&args.data_file);
    0
}

fn write_snapshot(
    data_file: &Path,
    token: &str,
    seq: u64,
    rows: Vec<ProcessSample>,
) -> Result<(), String> {
    validate_helper_output_path(data_file)?;
    let snapshot = ElevatedSnapshot {
        token: token.to_string(),
        seq,
        rows,
    };
    write_json_atomic(data_file, &snapshot, ADMIN_SNAPSHOT_JSON_ERRORS)
}

#[derive(Debug)]
struct ElevatedHelperSession {
    data_file: PathBuf,
    stop_file: PathBuf,
    token: String,
}

fn prepare_helper_session(base_dir: &Path) -> Result<ElevatedHelperSession, String> {
    let token = new_helper_token()?;
    let helper_root = base_dir.join("elevated-helper");
    reject_reparse_path(&helper_root, true)?;
    fs::create_dir_all(&helper_root).map_err(|error| {
        format!(
            "admin_mode_prepare_failed path={} error={}",
            helper_root.display(),
            error
        )
    })?;
    reject_reparse_path(&helper_root, true)?;

    let helper_dir = helper_root.join(format!("run-{token}"));
    fs::create_dir(&helper_dir).map_err(|error| {
        format!(
            "admin_mode_prepare_failed path={} error={}",
            helper_dir.display(),
            error
        )
    })?;
    reject_reparse_path(&helper_dir, true)?;

    Ok(ElevatedHelperSession {
        data_file: helper_dir.join("snapshot.json"),
        stop_file: helper_dir.join("stop.signal"),
        token,
    })
}

fn new_helper_token() -> Result<String, String> {
    let mut bytes = [0_u8; HELPER_TOKEN_BYTES];
    fill_random_bytes(&mut bytes)?;
    Ok(hex_lower(&bytes))
}

#[cfg(windows)]
fn fill_random_bytes(bytes: &mut [u8]) -> Result<(), String> {
    use windows_sys::Win32::Security::Cryptography::{
        BCryptGenRandom, BCRYPT_USE_SYSTEM_PREFERRED_RNG,
    };

    let status = unsafe {
        BCryptGenRandom(
            std::ptr::null_mut(),
            bytes.as_mut_ptr(),
            bytes.len() as u32,
            BCRYPT_USE_SYSTEM_PREFERRED_RNG,
        )
    };
    if status != 0 {
        return Err(format!("admin_mode_token_random_failed status={status}"));
    }
    Ok(())
}

#[cfg(all(unix, not(windows)))]
fn fill_random_bytes(bytes: &mut [u8]) -> Result<(), String> {
    use std::io::Read;

    fs::File::open("/dev/urandom")
        .and_then(|mut file| file.read_exact(bytes))
        .map_err(|error| format!("admin_mode_token_random_failed:{error}"))
}

#[cfg(not(any(windows, unix)))]
fn fill_random_bytes(bytes: &mut [u8]) -> Result<(), String> {
    let seed = format!("{}-{}", std::process::id(), crate::telemetry::now_ms());
    for (index, byte) in bytes.iter_mut().enumerate() {
        *byte = seed.as_bytes()[index % seed.len()] ^ index as u8;
    }
    Ok(())
}

fn hex_lower(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        encoded.push(HEX[(byte >> 4) as usize] as char);
        encoded.push(HEX[(byte & 0x0f) as usize] as char);
    }
    encoded
}

fn validate_helper_output_path(data_file: &Path) -> Result<(), String> {
    let Some(parent) = data_file.parent() else {
        return Err(format!(
            "admin_mode_helper_path_missing_parent path={}",
            data_file.display()
        ));
    };
    reject_reparse_path(parent, true)?;
    reject_reparse_path(data_file, false)?;
    reject_reparse_path(&snapshot_temp_file(data_file), false)
}

fn reject_reparse_path(path: &Path, allow_directory: bool) -> Result<(), String> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => {
            return Err(format!(
                "admin_mode_helper_path_check_failed path={} error={}",
                path.display(),
                error
            ));
        }
    };
    if metadata.file_type().is_symlink() || is_windows_reparse_point(&metadata) {
        return Err(format!(
            "admin_mode_helper_path_reparse_rejected path={}",
            path.display()
        ));
    }
    if !allow_directory && path.is_dir() {
        return Err(format!(
            "admin_mode_helper_path_not_file path={}",
            path.display()
        ));
    }
    Ok(())
}

#[cfg(windows)]
fn is_windows_reparse_point(metadata: &fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    use windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT;

    metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
}

#[cfg(not(windows))]
fn is_windows_reparse_point(_metadata: &fs::Metadata) -> bool {
    false
}

fn snapshot_temp_file(data_file: &Path) -> PathBuf {
    data_file.with_extension(format!(
        "{}.tmp",
        data_file
            .extension()
            .and_then(|extension| extension.to_str())
            .unwrap_or("json")
    ))
}

fn remove_helper_artifacts(base_dir: &Path) {
    let helper_dir = base_dir.join("elevated-helper");
    if reject_reparse_path(&helper_dir, true).is_err() {
        return;
    }
    let mut data_files = vec![helper_dir.join("snapshot.json")];
    if let Ok(entries) = fs::read_dir(&helper_dir) {
        for entry in entries.flatten() {
            if entry.file_type().is_ok_and(|file_type| file_type.is_dir()) {
                data_files.push(entry.path().join("snapshot.json"));
            }
        }
    }

    let mut has_artifacts = false;
    for data_file in &data_files {
        let stop_file = data_file.with_file_name("stop.signal");
        if data_file.exists() || snapshot_temp_file(data_file).exists() || stop_file.exists() {
            has_artifacts = true;
            let _ = fs::write(&stop_file, "stop");
        }
    }
    if has_artifacts {
        thread::sleep(Duration::from_millis(HELPER_INTERVAL_MS + 100));
    }
    for data_file in data_files {
        remove_artifacts_for_snapshot(&data_file);
    }
}

fn remove_artifacts_for_snapshot(data_file: &Path) {
    remove_snapshot_artifacts(data_file);
    let _ = fs::remove_file(data_file.with_file_name("stop.signal"));
    remove_empty_run_dir(data_file);
}

fn remove_snapshot_artifacts(data_file: &Path) {
    let _ = fs::remove_file(data_file);
    let _ = fs::remove_file(snapshot_temp_file(data_file));
}

fn remove_empty_run_dir(data_file: &Path) {
    let Some(parent) = data_file.parent() else {
        return;
    };
    if parent
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.starts_with("run-"))
    {
        let _ = fs::remove_dir(parent);
    }
}

fn parse_helper_args(args: &[String]) -> Result<ElevatedHelperArgs, String> {
    reject_unknown_helper_args(args)?;
    Ok(ElevatedHelperArgs {
        data_file: PathBuf::from(required_value(args, "--data-file")?),
        stop_file: PathBuf::from(required_value(args, "--stop-file")?),
        token: required_value(args, "--token")?,
    })
}

fn reject_unknown_helper_args(args: &[String]) -> Result<(), String> {
    let known_with_value = ["--data-file", "--stop-file", "--token"];
    let known_flags = ["--elevated-helper"];
    cli_args::reject_unknown_args(args, &known_with_value, &known_flags)
}

fn required_value(args: &[String], name: &str) -> Result<String, String> {
    let Some(index) = args.iter().position(|arg| arg == name) else {
        return Err(format!("missing_required_argument:{name}"));
    };
    let value = args
        .get(index + 1)
        .ok_or_else(|| format!("missing_value_for_argument:{name}"))?;
    if value.starts_with("--") {
        return Err(format!("missing_value_for_argument:{name}"));
    }

    Ok(value.clone())
}

#[cfg(windows)]
#[derive(Debug)]
struct ElevatedHelperProcess {
    handle: HANDLE,
}

#[cfg(windows)]
unsafe impl Send for ElevatedHelperProcess {}

#[cfg(windows)]
impl ElevatedHelperProcess {
    fn has_exited(&self) -> bool {
        use windows_sys::Win32::{
            Foundation::WAIT_OBJECT_0, System::Threading::WaitForSingleObject,
        };

        unsafe { WaitForSingleObject(self.handle, 0) == WAIT_OBJECT_0 }
    }

    fn wait(&self, timeout: Duration) -> bool {
        use windows_sys::Win32::{
            Foundation::WAIT_OBJECT_0, System::Threading::WaitForSingleObject,
        };

        unsafe {
            WaitForSingleObject(
                self.handle,
                timeout.as_millis().min(u32::MAX as u128) as u32,
            ) == WAIT_OBJECT_0
        }
    }
}

#[cfg(windows)]
impl Drop for ElevatedHelperProcess {
    fn drop(&mut self) {
        unsafe {
            windows_sys::Win32::Foundation::CloseHandle(self.handle);
        }
    }
}

#[cfg(not(windows))]
#[derive(Debug)]
struct ElevatedHelperProcess;

#[cfg(not(windows))]
impl ElevatedHelperProcess {
    fn has_exited(&self) -> bool {
        false
    }

    fn wait(&self, _timeout: Duration) -> bool {
        true
    }
}

#[cfg(windows)]
fn launch_elevated_helper(
    data_file: &Path,
    stop_file: &Path,
    token: &str,
) -> Result<Option<ElevatedHelperProcess>, String> {
    use std::{mem::size_of, ptr::null};

    use windows_sys::Win32::UI::{
        Shell::{ShellExecuteExW, SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW},
        WindowsAndMessaging::SW_HIDE,
    };

    let exe = std::env::current_exe()
        .map_err(|error| format!("admin_mode_current_exe_failed:{error}"))?;
    let exe_w = wide_os(exe.as_os_str());
    let verb_w = wide("runas");
    let params_w = wide(&format!(
        "--elevated-helper --data-file \"{}\" --stop-file \"{}\" --token \"{}\"",
        data_file.display(),
        stop_file.display(),
        token
    ));
    let mut info = SHELLEXECUTEINFOW {
        cbSize: size_of::<SHELLEXECUTEINFOW>() as u32,
        fMask: SEE_MASK_NOCLOSEPROCESS,
        hwnd: 0 as _,
        lpVerb: verb_w.as_ptr(),
        lpFile: exe_w.as_ptr(),
        lpParameters: params_w.as_ptr(),
        lpDirectory: null(),
        nShow: SW_HIDE,
        hInstApp: 0 as _,
        lpIDList: std::ptr::null_mut(),
        lpClass: null(),
        hkeyClass: 0 as _,
        dwHotKey: 0,
        Anonymous: Default::default(),
        hProcess: 0 as _,
    };

    let ok = unsafe { ShellExecuteExW(&mut info) };
    if ok == 0 {
        return Err("admin_mode_launch_failed_or_cancelled".to_string());
    }
    if info.hProcess.is_null() {
        return Ok(None);
    }

    Ok(Some(ElevatedHelperProcess {
        handle: info.hProcess,
    }))
}

#[cfg(not(windows))]
fn launch_elevated_helper(
    _data_file: &Path,
    _stop_file: &Path,
    _token: &str,
) -> Result<Option<ElevatedHelperProcess>, String> {
    Err("admin_mode_requires_windows".to_string())
}

#[cfg(windows)]
fn wide(value: &str) -> Vec<u16> {
    OsStr::new(value).encode_wide().chain(once(0)).collect()
}

#[cfg(windows)]
fn wide_os(value: &OsStr) -> Vec<u16> {
    value.encode_wide().chain(once(0)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contracts::AccessState;

    #[test]
    fn helper_requires_path_token_arguments() {
        let args = vec!["--elevated-helper".to_string()];

        assert_eq!(
            parse_helper_args(&args),
            Err("missing_required_argument:--data-file".to_string())
        );
    }

    #[test]
    fn helper_rejects_missing_values() {
        let args = vec![
            "--elevated-helper".to_string(),
            "--data-file".to_string(),
            "--stop-file".to_string(),
            "stop".to_string(),
            "--token".to_string(),
            "token".to_string(),
        ];

        assert_eq!(
            parse_helper_args(&args),
            Err("missing_value_for_argument:--data-file".to_string())
        );
    }

    #[test]
    fn poll_rows_returns_none_when_snapshot_is_missing() {
        let base_dir = test_dir("missing");
        let mut client = test_client(&base_dir);

        assert!(client
            .poll_rows()
            .expect("missing snapshot is ok")
            .is_none());

        let _ = fs::remove_dir_all(base_dir);
    }

    #[test]
    fn helper_session_uses_random_token_and_per_run_paths() {
        let base_dir = test_dir("session");

        let first = prepare_helper_session(&base_dir).expect("first session prepares");
        let second = prepare_helper_session(&base_dir).expect("second session prepares");

        assert_eq!(first.token.len(), HELPER_TOKEN_BYTES * 2);
        assert_eq!(second.token.len(), HELPER_TOKEN_BYTES * 2);
        assert_ne!(first.token, second.token);
        assert_eq!(
            first.data_file.file_name().and_then(|name| name.to_str()),
            Some("snapshot.json")
        );
        assert_eq!(
            first.stop_file.file_name().and_then(|name| name.to_str()),
            Some("stop.signal")
        );
        assert_eq!(first.data_file.parent(), first.stop_file.parent());
        assert!(first
            .data_file
            .parent()
            .and_then(|parent| parent.file_name())
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.starts_with("run-")));

        let _ = fs::remove_dir_all(base_dir);
    }

    #[test]
    fn helper_write_rejects_directory_snapshot_target() {
        let base_dir = test_dir("directory-target");
        let data_file = base_dir.join("snapshot.json");
        fs::create_dir_all(&data_file).expect("directory target exists");

        let error = write_snapshot(&data_file, "token", 1, Vec::new())
            .expect_err("directory target is rejected");

        assert!(error.starts_with("admin_mode_helper_path_not_file path="));
        let _ = fs::remove_dir_all(base_dir);
    }

    #[cfg(any(windows, unix))]
    #[test]
    fn helper_session_rejects_reparse_helper_root() {
        let base_dir = test_dir("reparse-helper-root");
        let real_dir = base_dir.join("real");
        let helper_root = base_dir.join("elevated-helper");
        fs::create_dir_all(&real_dir).expect("real dir exists");
        if symlink_dir(&real_dir, &helper_root).is_err() {
            let _ = fs::remove_dir_all(base_dir);
            return;
        }

        let error = prepare_helper_session(&base_dir).expect_err("helper root is rejected");

        assert!(error.starts_with("admin_mode_helper_path_reparse_rejected path="));
        let _ = fs::remove_dir_all(base_dir);
    }

    #[cfg(unix)]
    #[test]
    fn helper_write_rejects_symlink_parent() {
        let base_dir = test_dir("symlink-parent");
        let real_dir = base_dir.join("real");
        let link_dir = base_dir.join("link");
        fs::create_dir_all(&real_dir).expect("real dir exists");
        symlink_dir(&real_dir, &link_dir).expect("symlink parent exists");

        let error = write_snapshot(&link_dir.join("snapshot.json"), "token", 1, Vec::new())
            .expect_err("symlink parent is rejected");

        assert!(error.starts_with("admin_mode_helper_path_reparse_rejected path="));
        let _ = fs::remove_dir_all(base_dir);
    }

    #[cfg(windows)]
    fn symlink_dir(src: &Path, dst: &Path) -> std::io::Result<()> {
        std::os::windows::fs::symlink_dir(src, dst)
    }

    #[cfg(unix)]
    fn symlink_dir(src: &Path, dst: &Path) -> std::io::Result<()> {
        std::os::unix::fs::symlink(src, dst)
    }

    #[test]
    fn poll_rows_rejects_token_mismatch() {
        let base_dir = test_dir("token");
        let mut client = test_client(&base_dir);
        write_snapshot(&client.data_file, "wrong", 1, Vec::new()).expect("snapshot writes");

        let error = client.poll_rows().expect_err("token mismatch fails");

        assert_eq!(error, "admin_mode_snapshot_token_mismatch");
        let _ = fs::remove_dir_all(base_dir);
    }

    #[test]
    fn poll_rows_ignores_stale_sequence_and_accepts_newer_rows() {
        let base_dir = test_dir("seq");
        let mut client = test_client(&base_dir);
        write_snapshot(&client.data_file, &client.token, 2, vec![sample("10")])
            .expect("snapshot writes");

        let first = client
            .poll_rows()
            .expect("fresh snapshot reads")
            .expect("fresh rows exist");

        assert_eq!(first[0].pid, "10");

        write_snapshot(&client.data_file, &client.token, 2, vec![sample("20")])
            .expect("stale snapshot writes");
        assert!(client.poll_rows().expect("stale snapshot is ok").is_none());

        write_snapshot(&client.data_file, &client.token, 3, vec![sample("30")])
            .expect("new snapshot writes");
        let newer = client
            .poll_rows()
            .expect("new snapshot reads")
            .expect("new rows exist");

        assert_eq!(newer[0].pid, "30");
        let _ = fs::remove_dir_all(base_dir);
    }

    #[test]
    fn poll_rows_surfaces_parse_and_read_failures() {
        let parse_dir = test_dir("parse");
        let mut parse_client = test_client(&parse_dir);
        fs::write(&parse_client.data_file, "{not-json").expect("bad snapshot writes");

        let parse_error = parse_client.poll_rows().expect_err("invalid json fails");

        assert!(parse_error.starts_with("admin_mode_snapshot_parse_failed:"));

        let read_dir = test_dir("read");
        let mut read_client = test_client(&read_dir);
        fs::create_dir_all(&read_client.data_file).expect("directory fixture writes");

        let read_error = read_client.poll_rows().expect_err("directory read fails");

        assert!(read_error.starts_with("admin_mode_snapshot_read_failed path="));
        let _ = fs::remove_dir_all(parse_dir);
        let _ = fs::remove_dir_all(read_dir);
    }

    #[test]
    fn stop_signals_helper_and_removes_snapshot_artifacts() {
        let base_dir = test_dir("stop");
        let client = test_client(&base_dir);
        let temp_file = snapshot_temp_file(&client.data_file);
        fs::write(&client.data_file, "{}").expect("snapshot fixture writes");
        fs::write(&temp_file, "{}").expect("temp fixture writes");

        client.stop();

        assert!(!client.stop_file.exists());
        assert!(!client.data_file.exists());
        assert!(!temp_file.exists());
        let _ = fs::remove_dir_all(base_dir);
    }

    #[test]
    fn stale_helper_artifacts_are_removed_from_base_dir() {
        let base_dir = test_dir("stale-artifacts");
        let helper_dir = base_dir.join("elevated-helper");
        fs::create_dir_all(&helper_dir).expect("helper dir exists");
        let data_file = helper_dir.join("snapshot.json");
        let temp_file = snapshot_temp_file(&data_file);
        let stop_file = helper_dir.join("stop.signal");
        fs::write(&data_file, "{}").expect("snapshot fixture writes");
        fs::write(&temp_file, "{}").expect("temp fixture writes");
        fs::write(&stop_file, "stop").expect("stop fixture writes");

        ElevatedHelperClient::remove_stale_artifacts(&base_dir);

        assert!(!data_file.exists());
        assert!(!temp_file.exists());
        assert!(!stop_file.exists());
        let _ = fs::remove_dir_all(base_dir);
    }

    #[test]
    fn helper_exit_removes_snapshot_artifacts() {
        let base_dir = test_dir("helper-exit");
        fs::create_dir_all(&base_dir).expect("test dir exists");
        let data_file = base_dir.join("snapshot.json");
        let temp_file = snapshot_temp_file(&data_file);
        let stop_file = base_dir.join("stop.signal");
        fs::write(&data_file, "{}").expect("snapshot fixture writes");
        fs::write(&temp_file, "{}").expect("temp fixture writes");
        fs::write(&stop_file, "stop").expect("stop fixture writes");

        let exit_code = run_elevated_helper(ElevatedHelperArgs {
            data_file: data_file.clone(),
            stop_file,
            token: "token".to_string(),
        });

        assert_eq!(exit_code, 0);
        assert!(!data_file.exists());
        assert!(!temp_file.exists());
        let _ = fs::remove_dir_all(base_dir);
    }

    fn test_client(base_dir: &Path) -> ElevatedHelperClient {
        fs::create_dir_all(base_dir).expect("test dir exists");
        ElevatedHelperClient {
            data_file: base_dir.join("snapshot.json"),
            stop_file: base_dir.join("stop.signal"),
            token: "token".to_string(),
            last_seq: 0,
            process: None,
        }
    }

    fn test_dir(name: &str) -> PathBuf {
        let path =
            std::env::temp_dir().join(format!("batcave-elevation-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&path);
        path
    }

    fn sample(pid: &str) -> ProcessSample {
        ProcessSample {
            pid: pid.to_string(),
            parent_pid: None,
            start_time_ms: 1,
            name: "Sample".to_string(),
            exe: "sample.exe".to_string(),
            status: "running".to_string(),
            cpu_percent: 0.0,
            kernel_cpu_percent: None,
            memory_bytes: 1,
            private_bytes: 1,
            virtual_memory_bytes: 1,
            disk_read_total_bytes: 0,
            disk_write_total_bytes: 0,
            other_io_total_bytes: None,
            disk_read_bps: 0,
            disk_write_bps: 0,
            other_io_bps: None,
            network_received_bps: None,
            network_transmitted_bps: None,
            threads: 1,
            handles: 1,
            access_state: AccessState::Full,
            quality: None,
        }
    }
}
