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
    telemetry::{now_ms, TelemetryCollector},
};

const HELPER_INTERVAL_MS: u64 = 500;
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
        let helper_dir = base_dir.join("elevated-helper");
        fs::create_dir_all(&helper_dir).map_err(|error| {
            format!(
                "admin_mode_prepare_failed path={} error={}",
                helper_dir.display(),
                error
            )
        })?;
        let token = format!("{}-{}", std::process::id(), now_ms());
        let data_file = helper_dir.join("snapshot.json");
        let stop_file = helper_dir.join("stop.signal");
        remove_helper_artifacts(base_dir);

        let process = launch_elevated_helper(&data_file, &stop_file, &token)?;

        Ok(Self {
            data_file,
            stop_file,
            token,
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
    let snapshot = ElevatedSnapshot {
        token: token.to_string(),
        seq,
        rows,
    };
    write_json_atomic(data_file, &snapshot, ADMIN_SNAPSHOT_JSON_ERRORS)
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
    let data_file = helper_dir.join("snapshot.json");
    let stop_file = helper_dir.join("stop.signal");
    let has_artifacts =
        data_file.exists() || snapshot_temp_file(&data_file).exists() || stop_file.exists();
    if has_artifacts {
        let _ = fs::create_dir_all(&helper_dir);
        let _ = fs::write(&stop_file, "stop");
        thread::sleep(Duration::from_millis(HELPER_INTERVAL_MS + 100));
    }
    remove_artifacts_for_snapshot(&data_file);
}

fn remove_artifacts_for_snapshot(data_file: &Path) {
    remove_snapshot_artifacts(data_file);
    let _ = fs::remove_file(data_file.with_file_name("stop.signal"));
}

fn remove_snapshot_artifacts(data_file: &Path) {
    let _ = fs::remove_file(data_file);
    let _ = fs::remove_file(snapshot_temp_file(data_file));
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
