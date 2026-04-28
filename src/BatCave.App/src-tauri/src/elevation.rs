use std::{
    fs,
    path::{Path, PathBuf},
    thread,
    time::Duration,
};

#[cfg(windows)]
use std::{ffi::OsStr, iter::once, os::windows::ffi::OsStrExt};

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

#[derive(Debug, Clone)]
pub struct ElevatedHelperClient {
    data_file: PathBuf,
    stop_file: PathBuf,
    token: String,
    last_seq: u64,
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
        let _ = fs::remove_file(&data_file);
        let _ = fs::remove_file(&stop_file);

        launch_elevated_helper(&data_file, &stop_file, &token)?;

        Ok(Self {
            data_file,
            stop_file,
            token,
            last_seq: 0,
        })
    }

    pub fn poll_rows(&mut self) -> Result<Option<Vec<ProcessSample>>, String> {
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
fn launch_elevated_helper(data_file: &Path, stop_file: &Path, token: &str) -> Result<(), String> {
    use std::{mem::size_of, ptr::null};

    use windows_sys::Win32::{
        Foundation::CloseHandle,
        UI::{
            Shell::{ShellExecuteExW, SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW},
            WindowsAndMessaging::SW_HIDE,
        },
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
    if !info.hProcess.is_null() {
        unsafe {
            CloseHandle(info.hProcess);
        }
    }

    Ok(())
}

#[cfg(not(windows))]
fn launch_elevated_helper(
    _data_file: &Path,
    _stop_file: &Path,
    _token: &str,
) -> Result<(), String> {
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
}
