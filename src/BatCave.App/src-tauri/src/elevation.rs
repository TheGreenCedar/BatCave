#[cfg(test)]
use std::collections::VecDeque;

use std::{
    fs,
    path::{Component, Path, PathBuf},
    thread,
    time::{Duration, Instant},
};

#[cfg(windows)]
use std::{ffi::OsStr, iter::once, os::windows::ffi::OsStrExt};

#[cfg(windows)]
use windows_sys::Win32::Foundation::HANDLE;

use serde::{Deserialize, Serialize};

#[cfg(windows)]
use crate::telemetry::TelemetryCollector;
use crate::{cli_args, contracts::ProcessSample};

const HELPER_INTERVAL_MS: u64 = 500;
const HELPER_STALE_GRACE: Duration = Duration::from_secs(3);
const HELPER_FAILURE_GRACE: Duration = Duration::from_secs(15);
const HELPER_STOP_GRACE: Duration = Duration::from_secs(2);
const HELPER_FORCE_EXIT_GRACE: Duration = Duration::from_secs(2);
const HELPER_LAUNCHER_STOP_GRACE: Duration = Duration::from_secs(5);
const HELPER_WATCHDOG_POLL: Duration = Duration::from_millis(50);
const HELPER_TOKEN_BYTES: usize = 32;
const MAX_PIPE_FRAME_BYTES: usize = 8 * 1024 * 1024;
const ELEVATED_HELPER_FLAG: &str = "--elevated-helper";
const ELEVATED_HELPER_LAUNCHER_FLAG: &str = "--elevated-helper-launcher";
pub(crate) const HELPER_LAUNCHER_EXIT_FAILED: u32 = 1;
pub(crate) const HELPER_LAUNCHER_EXIT_NO_CHILD: u32 = 2;
pub(crate) const HELPER_LAUNCHER_EXIT_SETTLED_FAILURE: u32 = 3;

#[derive(Debug)]
pub struct ElevatedHelperClient {
    data_file: PathBuf,
    stop_file: PathBuf,
    accepted_file: PathBuf,
    token: String,
    last_seq: u64,
    accepted_at: Option<Instant>,
    last_snapshot_at: Option<Instant>,
    collect_process_network: bool,
    last_warnings: Vec<String>,
    recovering_detail: Option<String>,
    read_buffer: Vec<u8>,
    #[cfg(windows)]
    pipe: Option<NamedPipeServer>,
    process: Option<ElevatedHelperProcess>,
    stopped: bool,
    #[cfg(test)]
    scripted_polls: VecDeque<Result<ElevatedPoll, String>>,
    #[cfg(test)]
    scripted_exit_code: Option<u32>,
}

#[derive(Debug, Clone)]
pub enum ElevatedPoll {
    Fresh {
        rows: Vec<ProcessSample>,
        warnings: Vec<String>,
    },
    Held {
        warnings: Vec<String>,
    },
    Recovering(String),
    Pending,
}

impl ElevatedHelperClient {
    pub fn start(base_dir: &Path, collect_process_network: bool) -> Result<Self, String> {
        let owner_window = current_process_owner_window()?;
        remove_helper_artifacts(base_dir);
        let session = prepare_helper_session(base_dir)?;

        #[cfg(windows)]
        let pipe = match NamedPipeServer::create(&session.pipe_name) {
            Ok(pipe) => Some(pipe),
            Err(error) => {
                remove_artifacts_for_snapshot(&session.data_file);
                return Err(error);
            }
        };

        let process = match launch_elevated_helper(
            &session.pipe_name,
            &session.stop_file,
            &session.accepted_file,
            &session.token,
            std::process::id(),
            owner_window,
            collect_process_network,
        ) {
            Ok(process) => process,
            Err(error) => {
                remove_artifacts_for_snapshot(&session.data_file);
                return Err(error);
            }
        };

        Ok(Self {
            data_file: session.data_file,
            stop_file: session.stop_file,
            accepted_file: session.accepted_file,
            token: session.token,
            last_seq: 0,
            accepted_at: None,
            last_snapshot_at: None,
            collect_process_network,
            last_warnings: Vec::new(),
            recovering_detail: None,
            read_buffer: Vec::new(),
            #[cfg(windows)]
            pipe,
            process,
            stopped: false,
            #[cfg(test)]
            scripted_polls: VecDeque::new(),
            #[cfg(test)]
            scripted_exit_code: None,
        })
    }

    pub fn poll_rows(&mut self) -> Result<ElevatedPoll, String> {
        #[cfg(test)]
        if let Some(poll) = self.scripted_polls.pop_front() {
            return poll;
        }

        if self.launcher_has_exited() {
            return Err(self.launcher_exit_detail());
        }

        if self.accepted_at.is_none() && self.accepted_file.exists() {
            self.accepted_at = Some(Instant::now());
        }

        #[cfg(windows)]
        if let Some(pipe) = &mut self.pipe {
            let mut snapshot = None;
            for payload in pipe.read_payloads(&mut self.read_buffer)? {
                if let Some(next) = self.accept_snapshot_payload(&payload)? {
                    if matches!(next, ElevatedPoll::Fresh { .. }) {
                        self.last_snapshot_at = Some(Instant::now());
                    }
                    snapshot = Some(next);
                }
            }
            if let Some(snapshot) = snapshot {
                if matches!(snapshot, ElevatedPoll::Recovering(_))
                    && self
                        .last_snapshot_at
                        .or(self.accepted_at)
                        .is_some_and(|at| at.elapsed() > HELPER_FAILURE_GRACE)
                {
                    return Err("admin_mode_snapshot_timeout".to_string());
                }
                return Ok(snapshot);
            }
        }

        let Some(since) = self
            .last_snapshot_at
            .or(self.accepted_at)
            .map(|at| at.elapsed())
        else {
            return Ok(ElevatedPoll::Pending);
        };
        if since > HELPER_FAILURE_GRACE {
            return Err("admin_mode_snapshot_timeout".to_string());
        }
        if let Some(detail) = &self.recovering_detail {
            return Ok(ElevatedPoll::Recovering(detail.clone()));
        }
        Ok(
            if self.last_snapshot_at.is_some() && since > HELPER_STALE_GRACE {
                ElevatedPoll::Recovering("admin_mode_snapshot_delayed".to_string())
            } else if self.last_snapshot_at.is_some() {
                ElevatedPoll::Held {
                    warnings: self.last_warnings.clone(),
                }
            } else {
                ElevatedPoll::Pending
            },
        )
    }

    pub fn collects_process_network(&self) -> bool {
        self.collect_process_network
    }

    pub fn stop(&mut self) -> Result<(), String> {
        self.stop_with_timeout(HELPER_LAUNCHER_STOP_GRACE)
    }

    fn stop_with_timeout(&mut self, graceful_timeout: Duration) -> Result<(), String> {
        if self.stopped {
            return Ok(());
        }

        let stop_write_error = fs::write(&self.stop_file, "stop").err();
        let stop_write_suffix = stop_write_error
            .as_ref()
            .map(|error| format!(":stop_signal_failed:{error}"))
            .unwrap_or_default();

        if let Some(error) = &stop_write_error {
            if !self.launcher_has_exited() {
                return Err(format!("admin_mode_stop_signal_failed:{error}"));
            }
        }

        let exited = self.launcher_has_exited()
            || self
                .process
                .as_ref()
                .is_none_or(|process| process.wait(graceful_timeout));
        if !exited {
            return Err(format!(
                "admin_mode_helper_settlement_timeout{stop_write_suffix}"
            ));
        }

        match self.launcher_exit_code() {
            Ok(0) => self.finish_stopped_session(),
            Ok(HELPER_LAUNCHER_EXIT_NO_CHILD) if !self.accepted_file.exists() => {
                self.finish_stopped_session()
            }
            Ok(HELPER_LAUNCHER_EXIT_SETTLED_FAILURE) => self.finish_stopped_session(),
            Ok(HELPER_LAUNCHER_EXIT_NO_CHILD) => Err(format!(
                "admin_mode_launcher_no_child_after_acceptance{stop_write_suffix}"
            )),
            Ok(exit_code) => Err(format!(
                "admin_mode_launcher_exit_failed:code={exit_code}{stop_write_suffix}"
            )),
            Err(error) => Err(format!("{error}{stop_write_suffix}")),
        }
    }

    pub fn remove_stale_artifacts(base_dir: &Path) {
        remove_helper_artifacts(base_dir);
    }

    #[cfg(test)]
    pub(crate) fn scripted_for_test(
        base_dir: &Path,
        polls: Vec<Result<ElevatedPoll, String>>,
    ) -> Self {
        fs::create_dir_all(base_dir).expect("scripted helper directory exists");
        Self {
            data_file: base_dir.join("snapshot.json"),
            stop_file: base_dir.join("stop.signal"),
            accepted_file: base_dir.join("accepted.signal"),
            token: "scripted-token".to_string(),
            last_seq: 0,
            accepted_at: None,
            last_snapshot_at: None,
            collect_process_network: false,
            last_warnings: Vec::new(),
            recovering_detail: None,
            read_buffer: Vec::new(),
            #[cfg(windows)]
            pipe: None,
            process: None,
            stopped: false,
            scripted_polls: polls.into(),
            scripted_exit_code: None,
        }
    }

    #[cfg(test)]
    pub(crate) fn scripted_launcher_exit_for_test(
        base_dir: &Path,
        exit_code: u32,
        accepted: bool,
    ) -> Self {
        let mut client = Self::scripted_for_test(base_dir, Vec::new());
        fs::write(&client.data_file, "{}").expect("scripted snapshot writes");
        fs::write(snapshot_temp_file(&client.data_file), "{}").expect("scripted temp writes");
        if accepted {
            fs::write(&client.accepted_file, "accepted")
                .expect("scripted acceptance signal writes");
        }
        client.scripted_exit_code = Some(exit_code);
        client
    }

    fn launcher_has_exited(&self) -> bool {
        #[cfg(test)]
        if self.scripted_exit_code.is_some() {
            return true;
        }

        self.process
            .as_ref()
            .is_some_and(ElevatedHelperProcess::has_exited)
    }

    fn launcher_exit_code(&self) -> Result<u32, String> {
        #[cfg(test)]
        if let Some(exit_code) = self.scripted_exit_code {
            return Ok(exit_code);
        }

        self.process
            .as_ref()
            .map_or(Ok(0), ElevatedHelperProcess::exit_code)
    }

    fn launcher_exit_detail(&self) -> String {
        match self.launcher_exit_code() {
            Ok(HELPER_LAUNCHER_EXIT_NO_CHILD) if !self.accepted_file.exists() => {
                "admin_mode_launch_failed_or_cancelled".to_string()
            }
            Ok(exit_code) => format!("admin_mode_helper_exited:launcher_code={exit_code}"),
            Err(error) => error,
        }
    }

    fn finish_stopped_session(&mut self) -> Result<(), String> {
        remove_artifacts_for_snapshot(&self.data_file);
        let residue = [
            self.data_file.clone(),
            snapshot_temp_file(&self.data_file),
            self.stop_file.clone(),
            self.accepted_file.clone(),
        ]
        .into_iter()
        .find(|path| path.exists());
        if let Some(path) = residue {
            return Err(format!(
                "admin_mode_helper_cleanup_failed:path={}",
                path.display()
            ));
        }
        self.stopped = true;
        Ok(())
    }

    fn accept_snapshot_payload(&mut self, payload: &str) -> Result<Option<ElevatedPoll>, String> {
        let snapshot = serde_json::from_str::<ElevatedSnapshot>(payload)
            .map_err(|error| format!("admin_mode_snapshot_parse_failed:{error}"))?;

        if snapshot.token != self.token {
            return Err("admin_mode_snapshot_token_mismatch".to_string());
        }
        if snapshot.seq <= self.last_seq {
            return Ok(None);
        }

        self.accepted_at.get_or_insert_with(Instant::now);
        self.last_seq = snapshot.seq;
        Ok(Some(if let Some(error) = snapshot.error {
            self.recovering_detail = Some(error.clone());
            ElevatedPoll::Recovering(error)
        } else {
            self.recovering_detail = None;
            self.last_warnings = snapshot.warnings.clone();
            ElevatedPoll::Fresh {
                rows: snapshot.rows,
                warnings: snapshot.warnings,
            }
        }))
    }
}

impl Drop for ElevatedHelperClient {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
struct ElevatedSnapshot {
    token: String,
    seq: u64,
    rows: Vec<ProcessSample>,
    #[serde(default)]
    warnings: Vec<String>,
    #[serde(default)]
    error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ElevatedHelperArgs {
    pipe_name: String,
    stop_file: PathBuf,
    accepted_file: PathBuf,
    token: String,
    parent_pid: u32,
    owner_window: usize,
    collect_process_network: bool,
}

pub fn run_cli(args: &[String]) -> Option<i32> {
    let (flag, run) = if args.iter().any(|arg| arg == ELEVATED_HELPER_LAUNCHER_FLAG) {
        (
            ELEVATED_HELPER_LAUNCHER_FLAG,
            run_elevated_helper_launcher as fn(ElevatedHelperArgs) -> i32,
        )
    } else if args.iter().any(|arg| arg == ELEVATED_HELPER_FLAG) {
        (
            ELEVATED_HELPER_FLAG,
            run_elevated_helper as fn(ElevatedHelperArgs) -> i32,
        )
    } else {
        return None;
    };

    match parse_helper_args(args, flag).and_then(validate_helper_args) {
        Ok(helper_args) => Some(run(helper_args)),
        Err(error) => {
            eprintln!("{error}");
            Some(2)
        }
    }
}

#[cfg(windows)]
fn run_elevated_helper(args: ElevatedHelperArgs) -> i32 {
    let mut seq = 0_u64;
    let mut watchdog = match ParentExitWatchdog::start(args.parent_pid) {
        Ok(watchdog) => watchdog,
        Err(error) => {
            eprintln!("{error}");
            return 1;
        }
    };
    let pipe = match PipeClient::connect(&args.pipe_name) {
        Ok(pipe) => pipe,
        Err(error) => {
            eprintln!("{error}");
            return 1;
        }
    };
    let collector = TelemetryCollector::for_elevated_helper(args.collect_process_network);
    while !args.stop_file.exists() && !watchdog.parent_exited() {
        seq = seq.saturating_add(1);
        let write = match collector.collect() {
            Ok(sample) => {
                pipe.write_snapshot(&args.token, seq, sample.processes, sample.warnings, None)
            }
            Err(error) => {
                pipe.write_snapshot(&args.token, seq, Vec::new(), Vec::new(), Some(error))
            }
        };
        if let Err(error) = write {
            eprintln!("{error}");
            return 1;
        }
        thread::sleep(Duration::from_millis(HELPER_INTERVAL_MS));
    }

    match watchdog.stop() {
        Ok(()) => 0,
        Err(error) => {
            eprintln!("{error}");
            1
        }
    }
}

#[cfg(windows)]
fn run_elevated_helper_launcher(args: ElevatedHelperArgs) -> i32 {
    if args.stop_file.exists() {
        return 0;
    }
    let process = match shell_execute_elevated_helper(&args) {
        Ok(process) => process,
        Err(ShellExecuteElevatedError::NoChild(error)) => {
            eprintln!("{error}");
            return HELPER_LAUNCHER_EXIT_NO_CHILD as i32;
        }
        Err(ShellExecuteElevatedError::OwnershipUnknown(error)) => {
            eprintln!("{error}");
            return HELPER_LAUNCHER_EXIT_FAILED as i32;
        }
    };
    if let Err(error) = fs::write(&args.accepted_file, "accepted") {
        eprintln!("admin_mode_acceptance_signal_failed:{error}");
        settle_elevated_helper(process, &args.stop_file);
        return HELPER_LAUNCHER_EXIT_SETTLED_FAILURE as i32;
    }
    match supervise_elevated_helper(process, &args.stop_file) {
        Ok(()) => 0,
        Err(error) => {
            eprintln!("{error}");
            HELPER_LAUNCHER_EXIT_SETTLED_FAILURE as i32
        }
    }
}

#[cfg(windows)]
#[derive(Debug)]
struct ParentExitWatchdog {
    cancel: std::sync::Arc<std::sync::atomic::AtomicBool>,
    parent_exited: std::sync::Arc<std::sync::atomic::AtomicBool>,
    worker: Option<thread::JoinHandle<Result<(), String>>>,
}

#[cfg(windows)]
impl ParentExitWatchdog {
    fn start(parent_pid: u32) -> Result<Self, String> {
        use windows_sys::Win32::{
            Foundation::{CloseHandle, GetLastError, WAIT_OBJECT_0, WAIT_TIMEOUT},
            System::Threading::{OpenProcess, WaitForSingleObject, PROCESS_SYNCHRONIZE},
        };

        let parent_handle = unsafe { OpenProcess(PROCESS_SYNCHRONIZE, 0, parent_pid) };
        if parent_handle.is_null() {
            return Err(format!("admin_mode_parent_watch_failed error={}", unsafe {
                GetLastError()
            }));
        }
        let parent_handle_value = parent_handle as usize;
        let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let parent_exited = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let worker_cancel = std::sync::Arc::clone(&cancel);
        let worker_parent_exited = std::sync::Arc::clone(&parent_exited);

        let worker = thread::Builder::new()
            .name("batcave-admin-watchdog".to_string())
            .spawn(move || {
                let parent_handle = parent_handle_value as HANDLE;
                let result = loop {
                    if worker_cancel.load(std::sync::atomic::Ordering::Acquire) {
                        break Ok(());
                    }

                    match unsafe {
                        WaitForSingleObject(parent_handle, HELPER_WATCHDOG_POLL.as_millis() as u32)
                    } {
                        WAIT_TIMEOUT => {}
                        WAIT_OBJECT_0 => {
                            worker_parent_exited.store(true, std::sync::atomic::Ordering::Release);
                            break Ok(());
                        }
                        _ => {
                            break Err(format!(
                                "admin_mode_parent_watch_failed error={}",
                                unsafe { GetLastError() }
                            ));
                        }
                    }
                };
                unsafe { CloseHandle(parent_handle) };
                result
            })
            .map_err(|error| {
                unsafe { CloseHandle(parent_handle) };
                format!("admin_mode_parent_watch_start_failed:{error}")
            })?;

        Ok(Self {
            cancel,
            parent_exited,
            worker: Some(worker),
        })
    }

    fn parent_exited(&self) -> bool {
        self.parent_exited
            .load(std::sync::atomic::Ordering::Acquire)
    }

    fn stop(&mut self) -> Result<(), String> {
        self.cancel
            .store(true, std::sync::atomic::Ordering::Release);
        self.worker.take().map_or(Ok(()), |worker| {
            worker
                .join()
                .map_err(|_| "admin_mode_parent_watch_join_failed".to_string())?
        })
    }
}

#[cfg(windows)]
impl Drop for ParentExitWatchdog {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}

#[cfg(not(windows))]
fn run_elevated_helper(_args: ElevatedHelperArgs) -> i32 {
    eprintln!("admin_mode_requires_windows");
    1
}

#[cfg(not(windows))]
fn run_elevated_helper_launcher(_args: ElevatedHelperArgs) -> i32 {
    eprintln!("admin_mode_requires_windows");
    1
}

fn snapshot_payload(
    token: &str,
    seq: u64,
    rows: Vec<ProcessSample>,
    warnings: Vec<String>,
    error: Option<String>,
) -> Result<Vec<u8>, String> {
    let snapshot = ElevatedSnapshot {
        token: token.to_string(),
        seq,
        rows,
        warnings,
        error,
    };
    serde_json::to_vec(&snapshot)
        .map_err(|error| format!("admin_mode_snapshot_serialize_failed:{error}"))
}

#[cfg(windows)]
#[derive(Debug)]
struct NamedPipeServer {
    handle: HANDLE,
    connected: bool,
}

#[cfg(windows)]
impl NamedPipeServer {
    fn create(pipe_name: &str) -> Result<Self, String> {
        use windows_sys::Win32::{
            Storage::FileSystem::{FILE_FLAG_FIRST_PIPE_INSTANCE, PIPE_ACCESS_INBOUND},
            System::Pipes::{CreateNamedPipeW, PIPE_NOWAIT, PIPE_READMODE_BYTE, PIPE_TYPE_BYTE},
        };

        let mut pipe_security = PipeSecurity::admin_only()?;
        let pipe_name_w = wide(pipe_name);
        let handle = unsafe {
            CreateNamedPipeW(
                pipe_name_w.as_ptr(),
                PIPE_ACCESS_INBOUND | FILE_FLAG_FIRST_PIPE_INSTANCE,
                PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_NOWAIT,
                1,
                MAX_PIPE_FRAME_BYTES.min(u32::MAX as usize) as u32,
                MAX_PIPE_FRAME_BYTES.min(u32::MAX as usize) as u32,
                0,
                pipe_security.attributes(),
            )
        };
        if handle == windows_sys::Win32::Foundation::INVALID_HANDLE_VALUE {
            return Err(format!("admin_mode_pipe_create_failed error={}", unsafe {
                windows_sys::Win32::Foundation::GetLastError()
            }));
        }

        Ok(Self {
            handle,
            connected: false,
        })
    }

    fn read_payloads(&mut self, buffer: &mut Vec<u8>) -> Result<Vec<String>, String> {
        self.connect_if_ready()?;
        if !self.connected {
            return Ok(Vec::new());
        }

        self.read_available(buffer)?;
        drain_payloads(buffer)
    }

    fn connect_if_ready(&mut self) -> Result<(), String> {
        use windows_sys::Win32::{
            Foundation::{GetLastError, ERROR_PIPE_CONNECTED, ERROR_PIPE_LISTENING},
            System::Pipes::ConnectNamedPipe,
        };

        if self.connected {
            return Ok(());
        }

        let ok = unsafe { ConnectNamedPipe(self.handle, std::ptr::null_mut()) };
        if ok != 0 {
            self.connected = true;
            return Ok(());
        }

        match unsafe { GetLastError() } {
            ERROR_PIPE_CONNECTED => {
                self.connected = true;
                Ok(())
            }
            ERROR_PIPE_LISTENING => Ok(()),
            error => Err(format!("admin_mode_pipe_connect_failed error={error}")),
        }
    }

    fn read_available(&self, buffer: &mut Vec<u8>) -> Result<(), String> {
        use windows_sys::Win32::{
            Foundation::{GetLastError, ERROR_BROKEN_PIPE, ERROR_NO_DATA},
            Storage::FileSystem::ReadFile,
            System::Pipes::PeekNamedPipe,
        };

        loop {
            let mut available = 0_u32;
            let ok = unsafe {
                PeekNamedPipe(
                    self.handle,
                    std::ptr::null_mut(),
                    0,
                    std::ptr::null_mut(),
                    &mut available,
                    std::ptr::null_mut(),
                )
            };
            if ok == 0 {
                return match unsafe { GetLastError() } {
                    ERROR_NO_DATA => Ok(()),
                    ERROR_BROKEN_PIPE => Err("admin_mode_pipe_disconnected".to_string()),
                    error => Err(format!("admin_mode_pipe_peek_failed error={error}")),
                };
            }
            if available == 0 {
                return Ok(());
            }

            let chunk_len = available.min(64 * 1024) as usize;
            let start = buffer.len();
            buffer.resize(start + chunk_len, 0);
            let mut read = 0_u32;
            let ok = unsafe {
                ReadFile(
                    self.handle,
                    buffer[start..].as_mut_ptr().cast(),
                    chunk_len as u32,
                    &mut read,
                    std::ptr::null_mut(),
                )
            };
            if ok == 0 {
                buffer.truncate(start);
                return match unsafe { GetLastError() } {
                    ERROR_NO_DATA => Ok(()),
                    ERROR_BROKEN_PIPE => Err("admin_mode_pipe_disconnected".to_string()),
                    error => Err(format!("admin_mode_pipe_read_failed error={error}")),
                };
            }
            buffer.truncate(start + read as usize);
        }
    }
}

#[cfg(windows)]
unsafe impl Send for NamedPipeServer {}

#[cfg(windows)]
struct PipeSecurity {
    descriptor: windows_sys::Win32::Security::PSECURITY_DESCRIPTOR,
    attributes: windows_sys::Win32::Security::SECURITY_ATTRIBUTES,
}

#[cfg(windows)]
impl PipeSecurity {
    fn admin_only() -> Result<Self, String> {
        use windows_sys::Win32::{
            Foundation::GetLastError,
            Security::{
                Authorization::ConvertStringSecurityDescriptorToSecurityDescriptorW,
                PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES,
            },
        };

        let mut descriptor: PSECURITY_DESCRIPTOR = std::ptr::null_mut();
        let sddl = wide("D:P(A;;GA;;;SY)(A;;GA;;;BA)");
        let ok = unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                sddl.as_ptr(),
                1,
                &mut descriptor,
                std::ptr::null_mut(),
            )
        };
        if ok == 0 {
            return Err(format!(
                "admin_mode_pipe_security_failed error={}",
                unsafe { GetLastError() }
            ));
        }

        Ok(Self {
            descriptor,
            attributes: SECURITY_ATTRIBUTES {
                nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
                lpSecurityDescriptor: descriptor.cast(),
                bInheritHandle: 0,
            },
        })
    }

    fn attributes(&mut self) -> *const windows_sys::Win32::Security::SECURITY_ATTRIBUTES {
        &self.attributes
    }
}

#[cfg(windows)]
impl Drop for PipeSecurity {
    fn drop(&mut self) {
        if !self.descriptor.is_null() {
            unsafe {
                windows_sys::Win32::Foundation::LocalFree(self.descriptor.cast());
            }
        }
    }
}

#[cfg(windows)]
impl Drop for NamedPipeServer {
    fn drop(&mut self) {
        unsafe {
            windows_sys::Win32::Foundation::CloseHandle(self.handle);
        }
    }
}

#[cfg(windows)]
struct PipeClient {
    handle: HANDLE,
}

#[cfg(windows)]
impl PipeClient {
    fn connect(pipe_name: &str) -> Result<Self, String> {
        use windows_sys::Win32::{
            Foundation::{GENERIC_WRITE, INVALID_HANDLE_VALUE},
            Storage::FileSystem::{CreateFileW, FILE_ATTRIBUTE_NORMAL, OPEN_EXISTING},
        };

        let pipe_name_w = wide(pipe_name);
        let handle = unsafe {
            CreateFileW(
                pipe_name_w.as_ptr(),
                GENERIC_WRITE,
                0,
                std::ptr::null_mut(),
                OPEN_EXISTING,
                FILE_ATTRIBUTE_NORMAL,
                std::ptr::null_mut(),
            )
        };
        if handle == INVALID_HANDLE_VALUE {
            return Err(format!("admin_mode_pipe_open_failed error={}", unsafe {
                windows_sys::Win32::Foundation::GetLastError()
            }));
        }

        Ok(Self { handle })
    }

    fn write_snapshot(
        &self,
        token: &str,
        seq: u64,
        rows: Vec<ProcessSample>,
        warnings: Vec<String>,
        error: Option<String>,
    ) -> Result<(), String> {
        let payload = snapshot_payload(token, seq, rows, warnings, error)?;
        if payload.len() > u32::MAX as usize {
            return Err("admin_mode_snapshot_too_large".to_string());
        }

        let len = (payload.len() as u32).to_le_bytes();
        self.write_all(&len)?;
        self.write_all(&payload)
    }

    fn write_all(&self, mut bytes: &[u8]) -> Result<(), String> {
        use windows_sys::Win32::Storage::FileSystem::WriteFile;

        while !bytes.is_empty() {
            let mut written = 0_u32;
            let len = bytes.len().min(u32::MAX as usize) as u32;
            let ok = unsafe {
                WriteFile(
                    self.handle,
                    bytes.as_ptr().cast(),
                    len,
                    &mut written,
                    std::ptr::null_mut(),
                )
            };
            if ok == 0 {
                return Err(format!("admin_mode_pipe_write_failed error={}", unsafe {
                    windows_sys::Win32::Foundation::GetLastError()
                }));
            }
            if written == 0 {
                return Err("admin_mode_pipe_write_failed error=zero_bytes".to_string());
            }
            bytes = &bytes[written as usize..];
        }

        Ok(())
    }
}

#[cfg(windows)]
impl Drop for PipeClient {
    fn drop(&mut self) {
        unsafe {
            windows_sys::Win32::Foundation::CloseHandle(self.handle);
        }
    }
}

fn drain_payloads(buffer: &mut Vec<u8>) -> Result<Vec<String>, String> {
    let mut payloads = Vec::new();
    let mut offset = 0_usize;
    while buffer.len().saturating_sub(offset) >= 4 {
        let len = u32::from_le_bytes(
            buffer[offset..offset + 4]
                .try_into()
                .expect("length prefix slice is fixed"),
        ) as usize;
        if len > MAX_PIPE_FRAME_BYTES {
            return Err("admin_mode_snapshot_too_large".to_string());
        }
        if buffer.len().saturating_sub(offset + 4) < len {
            break;
        }

        let start = offset + 4;
        let end = start + len;
        let payload = std::str::from_utf8(&buffer[start..end])
            .map_err(|error| format!("admin_mode_snapshot_utf8_failed:{error}"))?
            .to_string();
        payloads.push(payload);
        offset = end;
    }

    buffer.drain(..offset);
    Ok(payloads)
}

#[derive(Debug)]
struct ElevatedHelperSession {
    data_file: PathBuf,
    stop_file: PathBuf,
    accepted_file: PathBuf,
    pipe_name: String,
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
        accepted_file: helper_dir.join("accepted.signal"),
        pipe_name: format!(r"\\.\pipe\batcave-elevated-{token}"),
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
        let accepted_file = data_file.with_file_name("accepted.signal");
        if data_file.exists()
            || snapshot_temp_file(data_file).exists()
            || stop_file.exists()
            || accepted_file.exists()
        {
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
    let _ = fs::remove_file(data_file.with_file_name("accepted.signal"));
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

fn parse_helper_args(args: &[String], flag: &str) -> Result<ElevatedHelperArgs, String> {
    reject_unknown_helper_args(args, flag)?;
    let pipe_name = required_value(args, "--pipe-name")?;
    let stop_file = PathBuf::from(required_value(args, "--stop-file")?);
    let accepted_file = PathBuf::from(required_value(args, "--accepted-file")?);
    let token = required_value(args, "--token")?;
    let parent_pid = required_value(args, "--parent-pid")?
        .parse::<u32>()
        .map_err(|_| "invalid_elevated_helper_parent_pid".to_string())?;
    let owner_window = required_value(args, "--owner-window")?
        .parse::<usize>()
        .map_err(|_| "invalid_elevated_helper_owner_window".to_string())?;
    let collect_process_network = required_value(args, "--collect-process-network")?
        .parse::<bool>()
        .map_err(|_| "invalid_elevated_helper_process_network".to_string())?;
    Ok(ElevatedHelperArgs {
        pipe_name,
        stop_file,
        accepted_file,
        token,
        parent_pid,
        owner_window,
        collect_process_network,
    })
}

fn validate_helper_args(args: ElevatedHelperArgs) -> Result<ElevatedHelperArgs, String> {
    validate_helper_args_for_base(args, &crate::runtime_store::default_base_dir())
}

fn validate_helper_args_for_base(
    args: ElevatedHelperArgs,
    base_dir: &Path,
) -> Result<ElevatedHelperArgs, String> {
    if args.token.len() != HELPER_TOKEN_BYTES * 2
        || !args
            .token
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err("invalid_elevated_helper_token".to_string());
    }

    if args.pipe_name != format!(r"\\.\pipe\batcave-elevated-{}", args.token) {
        return Err("invalid_elevated_helper_pipe_name".to_string());
    }
    if args.parent_pid == 0 {
        return Err("invalid_elevated_helper_parent_pid".to_string());
    }
    if !owner_window_matches_parent(args.owner_window, args.parent_pid) {
        return Err("invalid_elevated_helper_owner_window".to_string());
    }

    let expected_run_dir = format!("run-{}", args.token);
    let run_dir = args
        .stop_file
        .parent()
        .ok_or_else(|| "invalid_elevated_helper_stop_file".to_string())?;
    let helper_root = run_dir
        .parent()
        .ok_or_else(|| "invalid_elevated_helper_stop_file".to_string())?;
    let valid_shape = args.stop_file.is_absolute()
        && args.stop_file.file_name().and_then(|name| name.to_str()) == Some("stop.signal")
        && run_dir.file_name().and_then(|name| name.to_str()) == Some(expected_run_dir.as_str())
        && helper_root.file_name().and_then(|name| name.to_str()) == Some("elevated-helper")
        && helper_root == base_dir.join("elevated-helper")
        && !args
            .stop_file
            .components()
            .any(|component| matches!(component, Component::CurDir | Component::ParentDir));
    if !valid_shape {
        return Err("invalid_elevated_helper_stop_file".to_string());
    }
    let valid_accepted_shape = args.accepted_file.is_absolute()
        && args
            .accepted_file
            .file_name()
            .and_then(|name| name.to_str())
            == Some("accepted.signal")
        && args.accepted_file.parent() == Some(run_dir)
        && !args
            .accepted_file
            .components()
            .any(|component| matches!(component, Component::CurDir | Component::ParentDir));
    if !valid_accepted_shape {
        return Err("invalid_elevated_helper_accepted_file".to_string());
    }
    reject_reparse_path(helper_root, true)?;
    reject_reparse_path(run_dir, true)?;
    reject_reparse_path(&args.stop_file, false)?;
    reject_reparse_path(&args.accepted_file, false)?;

    Ok(args)
}

fn reject_unknown_helper_args(args: &[String], flag: &str) -> Result<(), String> {
    let known_with_value = [
        "--pipe-name",
        "--stop-file",
        "--accepted-file",
        "--token",
        "--parent-pid",
        "--owner-window",
        "--collect-process-network",
    ];
    let known_flags = [flag];
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
fn current_process_owner_window() -> Result<usize, String> {
    use windows_sys::Win32::{
        Foundation::{HWND, LPARAM},
        UI::WindowsAndMessaging::{EnumWindows, GetWindowThreadProcessId, IsWindowVisible},
    };

    struct WindowSearch {
        process_id: u32,
        window: HWND,
    }

    unsafe extern "system" fn visit_window(window: HWND, state: LPARAM) -> windows_sys::core::BOOL {
        let search = unsafe { &mut *(state as *mut WindowSearch) };
        let mut process_id = 0_u32;
        unsafe { GetWindowThreadProcessId(window, &mut process_id) };
        if process_id == search.process_id && unsafe { IsWindowVisible(window) } != 0 {
            search.window = window;
            return 0;
        }
        1
    }

    let mut search = WindowSearch {
        process_id: std::process::id(),
        window: std::ptr::null_mut(),
    };
    unsafe {
        EnumWindows(
            Some(visit_window),
            &mut search as *mut WindowSearch as LPARAM,
        )
    };
    if search.window.is_null() {
        return Err("admin_mode_owner_window_not_found".to_string());
    }
    Ok(search.window as usize)
}

#[cfg(not(windows))]
fn current_process_owner_window() -> Result<usize, String> {
    Ok(0)
}

#[cfg(windows)]
fn owner_window_matches_parent(owner_window: usize, parent_pid: u32) -> bool {
    use windows_sys::Win32::{Foundation::HWND, UI::WindowsAndMessaging::GetWindowThreadProcessId};

    if owner_window == 0 {
        return true;
    }
    let mut owner_pid = 0_u32;
    unsafe { GetWindowThreadProcessId(owner_window as HWND, &mut owner_pid) };
    owner_pid == parent_pid
}

#[cfg(not(windows))]
fn owner_window_matches_parent(owner_window: usize, _parent_pid: u32) -> bool {
    owner_window == 0
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

    fn terminate(&self) -> bool {
        use windows_sys::Win32::System::Threading::TerminateProcess;

        unsafe { TerminateProcess(self.handle, 1) != 0 }
    }

    fn exit_code(&self) -> Result<u32, String> {
        use windows_sys::Win32::System::Threading::GetExitCodeProcess;

        let mut exit_code = 0_u32;
        if unsafe { GetExitCodeProcess(self.handle, &mut exit_code) } == 0 {
            Err(format!(
                "admin_mode_launcher_exit_status_failed:{}",
                std::io::Error::last_os_error()
            ))
        } else {
            Ok(exit_code)
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

    fn terminate(&self) -> bool {
        true
    }

    fn exit_code(&self) -> Result<u32, String> {
        Ok(0)
    }
}

#[cfg(windows)]
fn launch_elevated_helper(
    pipe_name: &str,
    stop_file: &Path,
    accepted_file: &Path,
    token: &str,
    parent_pid: u32,
    owner_window: usize,
    collect_process_network: bool,
) -> Result<Option<ElevatedHelperProcess>, String> {
    use std::{os::windows::io::IntoRawHandle, os::windows::process::CommandExt};

    use windows_sys::Win32::System::Threading::CREATE_NO_WINDOW;

    let exe = std::env::current_exe()
        .map_err(|error| format!("admin_mode_current_exe_failed:{error}"))?;
    let mut command = std::process::Command::new(exe);
    command
        .arg(ELEVATED_HELPER_LAUNCHER_FLAG)
        .arg("--pipe-name")
        .arg(pipe_name)
        .arg("--stop-file")
        .arg(stop_file)
        .arg("--accepted-file")
        .arg(accepted_file)
        .arg("--token")
        .arg(token)
        .arg("--parent-pid")
        .arg(parent_pid.to_string())
        .arg("--owner-window")
        .arg(owner_window.to_string())
        .arg("--collect-process-network")
        .arg(collect_process_network.to_string());
    command.creation_flags(CREATE_NO_WINDOW);
    let child = command
        .spawn()
        .map_err(|error| format!("admin_mode_launcher_spawn_failed:{error}"))?;
    Ok(Some(ElevatedHelperProcess {
        handle: child.into_raw_handle() as HANDLE,
    }))
}

#[cfg(windows)]
#[derive(Debug)]
enum ShellExecuteElevatedError {
    NoChild(String),
    OwnershipUnknown(String),
}

#[cfg(windows)]
fn shell_execute_elevated_helper(
    args: &ElevatedHelperArgs,
) -> Result<ElevatedHelperProcess, ShellExecuteElevatedError> {
    use std::{mem::size_of, ptr::null};

    use windows_sys::Win32::{
        Foundation::GetLastError,
        UI::{
            Shell::{ShellExecuteExW, SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW},
            WindowsAndMessaging::SW_HIDE,
        },
    };

    let exe = std::env::current_exe().map_err(|error| {
        ShellExecuteElevatedError::NoChild(format!("admin_mode_current_exe_failed:{error}"))
    })?;
    let exe_w = wide_os(exe.as_os_str());
    let verb_w = wide("runas");
    let params_w = wide(&format!(
        "--elevated-helper --pipe-name \"{}\" --stop-file \"{}\" --accepted-file \"{}\" --token \"{}\" --parent-pid {} --owner-window {} --collect-process-network {}",
        args.pipe_name,
        args.stop_file.display(),
        args.accepted_file.display(),
        args.token,
        args.parent_pid,
        args.owner_window,
        args.collect_process_network,
    ));
    let mut info = SHELLEXECUTEINFOW {
        cbSize: size_of::<SHELLEXECUTEINFOW>() as u32,
        fMask: SEE_MASK_NOCLOSEPROCESS,
        hwnd: args.owner_window as _,
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
        let error = unsafe { GetLastError() };
        return Err(ShellExecuteElevatedError::NoChild(format!(
            "admin_mode_launch_failed_or_cancelled:error={error}"
        )));
    }
    if info.hProcess.is_null() {
        return Err(ShellExecuteElevatedError::OwnershipUnknown(
            "admin_mode_launch_missing_process_handle".to_string(),
        ));
    }

    Ok(ElevatedHelperProcess {
        handle: info.hProcess,
    })
}

#[cfg(windows)]
fn supervise_elevated_helper(
    process: ElevatedHelperProcess,
    stop_file: &Path,
) -> Result<(), String> {
    loop {
        if process.has_exited() {
            return if stop_file.exists() {
                Ok(())
            } else {
                Err("admin_mode_helper_exited".to_string())
            };
        }
        if stop_file.exists() {
            if process.wait(HELPER_STOP_GRACE)
                || (process.terminate() && process.wait(HELPER_FORCE_EXIT_GRACE))
            {
                return Ok(());
            }
            eprintln!("admin_mode_helper_termination_pending");
        }
        thread::sleep(HELPER_WATCHDOG_POLL);
    }
}

#[cfg(windows)]
fn settle_elevated_helper(process: ElevatedHelperProcess, stop_file: &Path) {
    while !process.has_exited() {
        if stop_file.exists() {
            let _ = process.terminate();
        }
        let _ = process.wait(HELPER_WATCHDOG_POLL);
    }
}

#[cfg(not(windows))]
fn launch_elevated_helper(
    _pipe_name: &str,
    _stop_file: &Path,
    _accepted_file: &Path,
    _token: &str,
    _parent_pid: u32,
    _owner_window: usize,
    _collect_process_network: bool,
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
            parse_helper_args(&args, ELEVATED_HELPER_FLAG),
            Err("missing_required_argument:--pipe-name".to_string())
        );
    }

    #[test]
    fn helper_rejects_missing_values() {
        let args = vec![
            "--elevated-helper".to_string(),
            "--pipe-name".to_string(),
            "--stop-file".to_string(),
            "stop".to_string(),
            "--token".to_string(),
            "token".to_string(),
        ];

        assert_eq!(
            parse_helper_args(&args, ELEVATED_HELPER_FLAG),
            Err("missing_value_for_argument:--pipe-name".to_string())
        );
    }

    #[test]
    fn helper_modes_reject_each_others_flags() {
        assert_eq!(
            parse_helper_args(
                &[ELEVATED_HELPER_FLAG.to_string()],
                ELEVATED_HELPER_LAUNCHER_FLAG
            ),
            Err("unknown_argument:--elevated-helper".to_string())
        );
        assert_eq!(
            parse_helper_args(
                &[ELEVATED_HELPER_LAUNCHER_FLAG.to_string()],
                ELEVATED_HELPER_FLAG
            ),
            Err("unknown_argument:--elevated-helper-launcher".to_string())
        );
    }

    #[test]
    fn poll_rows_returns_pending_before_first_snapshot() {
        let base_dir = test_dir("missing");
        let mut client = test_client(&base_dir);

        assert!(matches!(
            client.poll_rows().expect("missing snapshot is pending"),
            ElevatedPoll::Pending
        ));

        let _ = fs::remove_dir_all(base_dir);
    }

    #[test]
    fn accepted_helper_without_a_snapshot_times_out() {
        let base_dir = test_dir("accepted-timeout");
        let mut client = test_client(&base_dir);
        client.accepted_at = Some(Instant::now() - HELPER_FAILURE_GRACE - Duration::from_millis(1));

        assert_eq!(
            client
                .poll_rows()
                .expect_err("accepted helper without a snapshot fails closed"),
            "admin_mode_snapshot_timeout"
        );

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
        assert!(first.pipe_name.starts_with(r"\\.\pipe\batcave-elevated-"));
        assert_eq!(
            first.pipe_name,
            format!(r"\\.\pipe\batcave-elevated-{}", first.token)
        );
        assert_eq!(
            first.data_file.file_name().and_then(|name| name.to_str()),
            Some("snapshot.json")
        );
        assert_eq!(
            first.stop_file.file_name().and_then(|name| name.to_str()),
            Some("stop.signal")
        );
        assert_eq!(
            first
                .accepted_file
                .file_name()
                .and_then(|name| name.to_str()),
            Some("accepted.signal")
        );
        assert_eq!(first.data_file.parent(), first.stop_file.parent());
        assert_eq!(first.data_file.parent(), first.accepted_file.parent());
        assert!(first
            .data_file
            .parent()
            .and_then(|parent| parent.file_name())
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.starts_with("run-")));

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
    fn symlink_dir(src: &Path, dst: &Path) -> std::io::Result<()> {
        std::os::unix::fs::symlink(src, dst)
    }

    #[cfg(windows)]
    fn symlink_dir(src: &Path, dst: &Path) -> std::io::Result<()> {
        std::os::windows::fs::symlink_dir(src, dst)
    }

    #[test]
    fn poll_rows_rejects_token_mismatch() {
        let base_dir = test_dir("token");
        let mut client = test_client(&base_dir);
        let payload = payload("wrong", 1, Vec::new());

        let error = client
            .accept_snapshot_payload(&payload)
            .expect_err("token mismatch fails");

        assert_eq!(error, "admin_mode_snapshot_token_mismatch");
        let _ = fs::remove_dir_all(base_dir);
    }

    #[test]
    fn poll_rows_ignores_stale_sequence_and_accepts_newer_rows() {
        let base_dir = test_dir("seq");
        let mut client = test_client(&base_dir);
        let token = client.token.clone();

        let first = client
            .accept_snapshot_payload(&payload(&token, 2, vec![sample("10")]))
            .expect("fresh snapshot reads")
            .expect("fresh rows exist");

        let ElevatedPoll::Fresh { rows: first, .. } = first else {
            panic!("fresh rows expected");
        };
        assert_eq!(first[0].pid, "10");

        assert!(client
            .accept_snapshot_payload(&payload(&token, 2, vec![sample("20")]))
            .expect("stale snapshot is ok")
            .is_none());

        let newer = client
            .accept_snapshot_payload(&payload(&token, 3, vec![sample("30")]))
            .expect("new snapshot reads")
            .expect("new rows exist");

        let ElevatedPoll::Fresh { rows: newer, .. } = newer else {
            panic!("fresh rows expected");
        };
        assert_eq!(newer[0].pid, "30");
        let _ = fs::remove_dir_all(base_dir);
    }

    #[test]
    fn client_snapshot_payload_surfaces_parse_failures() {
        let parse_dir = test_dir("parse");
        let mut parse_client = test_client(&parse_dir);

        let parse_error = parse_client
            .accept_snapshot_payload("{not-json")
            .expect_err("invalid json fails");

        assert!(parse_error.starts_with("admin_mode_snapshot_parse_failed:"));
        let _ = fs::remove_dir_all(parse_dir);
    }

    #[test]
    fn helper_arguments_are_bound_to_one_local_session() {
        let token = "a".repeat(HELPER_TOKEN_BYTES * 2);
        let base_dir = test_dir("validated-args");
        let run_dir = base_dir
            .join("elevated-helper")
            .join(format!("run-{token}"));
        fs::create_dir_all(&run_dir).expect("run dir exists");
        let valid = ElevatedHelperArgs {
            pipe_name: format!(r"\\.\pipe\batcave-elevated-{token}"),
            stop_file: run_dir.join("stop.signal"),
            accepted_file: run_dir.join("accepted.signal"),
            token: token.clone(),
            parent_pid: 1,
            owner_window: 0,
            collect_process_network: false,
        };

        assert_eq!(
            validate_helper_args_for_base(valid.clone(), &base_dir),
            Ok(valid.clone())
        );
        assert_eq!(
            validate_helper_args_for_base(
                ElevatedHelperArgs {
                    pipe_name: format!(r"\\server\pipe\batcave-elevated-{token}"),
                    ..valid.clone()
                },
                &base_dir
            ),
            Err("invalid_elevated_helper_pipe_name".to_string())
        );
        assert_eq!(
            validate_helper_args_for_base(
                ElevatedHelperArgs {
                    stop_file: run_dir
                        .join("..")
                        .join(format!("run-{token}"))
                        .join("stop.signal"),
                    ..valid.clone()
                },
                &base_dir
            ),
            Err("invalid_elevated_helper_stop_file".to_string())
        );
        assert_eq!(
            validate_helper_args_for_base(
                ElevatedHelperArgs {
                    parent_pid: 0,
                    ..valid.clone()
                },
                &base_dir
            ),
            Err("invalid_elevated_helper_parent_pid".to_string())
        );
        assert_eq!(
            validate_helper_args_for_base(
                ElevatedHelperArgs {
                    token: "A".repeat(HELPER_TOKEN_BYTES * 2),
                    ..valid
                },
                &base_dir
            ),
            Err("invalid_elevated_helper_token".to_string())
        );

        let _ = fs::remove_dir_all(base_dir);
    }

    #[test]
    fn helper_rows_recover_before_hard_timeout() {
        let base_dir = test_dir("hold");
        let mut client = test_client(&base_dir);
        client.last_snapshot_at = Some(Instant::now());
        assert!(matches!(
            client.poll_rows().expect("recent rows hold"),
            ElevatedPoll::Held { .. }
        ));

        client.last_snapshot_at =
            Some(Instant::now() - HELPER_STALE_GRACE - Duration::from_millis(1));
        assert!(matches!(
            client.poll_rows().expect("delayed rows recover"),
            ElevatedPoll::Recovering(_)
        ));

        client.last_snapshot_at =
            Some(Instant::now() - HELPER_FAILURE_GRACE - Duration::from_millis(1));
        assert_eq!(
            client.poll_rows().expect_err("expired rows fail closed"),
            "admin_mode_snapshot_timeout"
        );
        let _ = fs::remove_dir_all(base_dir);
    }

    #[test]
    fn helper_error_frame_recovers_with_next_snapshot() {
        let base_dir = test_dir("recover-frame");
        let mut client = test_client(&base_dir);
        let token = client.token.clone();
        let error_payload = String::from_utf8(
            snapshot_payload(
                &token,
                1,
                Vec::new(),
                Vec::new(),
                Some("temporary collector failure".to_string()),
            )
            .expect("error snapshot serializes"),
        )
        .expect("snapshot is utf8");

        assert!(matches!(
            client
                .accept_snapshot_payload(&error_payload)
                .expect("error frame is valid")
                .expect("error frame advances"),
            ElevatedPoll::Recovering(_)
        ));
        assert!(matches!(
            client.poll_rows().expect("error state remains recovering"),
            ElevatedPoll::Recovering(_)
        ));
        assert!(matches!(
            client
                .accept_snapshot_payload(&payload(&token, 2, vec![sample("10")]))
                .expect("fresh frame is valid")
                .expect("fresh frame advances"),
            ElevatedPoll::Fresh { .. }
        ));
        let _ = fs::remove_dir_all(base_dir);
    }

    #[test]
    fn length_prefixed_payloads_drain_complete_frames() {
        let first = payload("token", 1, vec![sample("10")]);
        let second = payload("token", 2, vec![sample("20")]);
        let mut buffer = Vec::new();
        append_frame(&mut buffer, first.as_bytes());
        append_frame(&mut buffer, second.as_bytes());
        buffer.extend_from_slice(&3_u32.to_le_bytes());
        buffer.extend_from_slice(b"ab");

        let drained = drain_payloads(&mut buffer).expect("frames drain");

        assert_eq!(drained, vec![first, second]);
        assert_eq!(buffer, {
            let mut partial = Vec::new();
            partial.extend_from_slice(&3_u32.to_le_bytes());
            partial.extend_from_slice(b"ab");
            partial
        });
    }

    #[test]
    fn stop_signals_helper_and_removes_snapshot_artifacts() {
        let base_dir = test_dir("stop");
        let mut client = test_client(&base_dir);
        let temp_file = snapshot_temp_file(&client.data_file);
        fs::write(&client.data_file, "{}").expect("snapshot fixture writes");
        fs::write(&temp_file, "{}").expect("temp fixture writes");
        fs::write(&client.accepted_file, "accepted").expect("accepted fixture writes");

        client.stop().expect("helper stops");

        assert!(!client.stop_file.exists());
        assert!(!client.data_file.exists());
        assert!(!temp_file.exists());
        assert!(!client.accepted_file.exists());
        let _ = fs::remove_dir_all(base_dir);
    }

    #[test]
    fn pre_child_launcher_exit_cleans_session_for_retry() {
        let base_dir = test_dir("pre-child-exit");
        let mut client = ElevatedHelperClient::scripted_launcher_exit_for_test(
            &base_dir,
            HELPER_LAUNCHER_EXIT_NO_CHILD,
            false,
        );
        let data_file = client.data_file.clone();
        let temp_file = snapshot_temp_file(&data_file);
        let stop_file = client.stop_file.clone();
        let accepted_file = client.accepted_file.clone();

        assert_eq!(
            client
                .poll_rows()
                .expect_err("pre-child launcher exit is explicit"),
            "admin_mode_launch_failed_or_cancelled"
        );
        client
            .stop()
            .expect("pre-child exit proves no child remains");

        assert!(client.stopped);
        assert!(!data_file.exists());
        assert!(!temp_file.exists());
        assert!(!stop_file.exists());
        assert!(!accepted_file.exists());
        let _ = fs::remove_dir_all(base_dir);
    }

    #[test]
    fn unresolved_post_child_launcher_failure_retains_session_ownership() {
        let base_dir = test_dir("post-child-exit");
        let mut client = ElevatedHelperClient::scripted_launcher_exit_for_test(
            &base_dir,
            HELPER_LAUNCHER_EXIT_FAILED,
            true,
        );

        assert_eq!(
            client
                .poll_rows()
                .expect_err("post-child launcher exit is explicit"),
            "admin_mode_helper_exited:launcher_code=1"
        );
        assert_eq!(
            client.stop().expect_err("post-child failure stays owned"),
            "admin_mode_launcher_exit_failed:code=1"
        );

        assert!(!client.stopped);
        assert!(client.data_file.exists());
        assert!(client.stop_file.exists());
        assert!(client.accepted_file.exists());
        let _ = fs::remove_dir_all(base_dir);
    }

    #[test]
    fn settled_post_child_launcher_failure_cleans_session_for_retry() {
        let base_dir = test_dir("settled-post-child-exit");
        let mut client = ElevatedHelperClient::scripted_launcher_exit_for_test(
            &base_dir,
            HELPER_LAUNCHER_EXIT_SETTLED_FAILURE,
            true,
        );

        assert_eq!(
            client
                .poll_rows()
                .expect_err("settled helper failure is explicit"),
            "admin_mode_helper_exited:launcher_code=3"
        );
        client
            .stop()
            .expect("settled child ownership permits cleanup");

        assert!(client.stopped);
        assert!(!client.data_file.exists());
        assert!(!client.stop_file.exists());
        assert!(!client.accepted_file.exists());
        let _ = fs::remove_dir_all(base_dir);
    }

    #[test]
    fn no_child_exit_after_acceptance_retains_session_ownership() {
        let base_dir = test_dir("accepted-no-child-exit");
        let mut client = ElevatedHelperClient::scripted_launcher_exit_for_test(
            &base_dir,
            HELPER_LAUNCHER_EXIT_NO_CHILD,
            true,
        );

        assert_eq!(
            client
                .poll_rows()
                .expect_err("acceptance prevents denial classification"),
            "admin_mode_helper_exited:launcher_code=2"
        );
        assert_eq!(
            client
                .stop()
                .expect_err("accepted sessions require ownership settlement"),
            "admin_mode_launcher_no_child_after_acceptance"
        );

        assert!(!client.stopped);
        assert!(client.data_file.exists());
        assert!(client.stop_file.exists());
        assert!(client.accepted_file.exists());
        let _ = fs::remove_dir_all(base_dir);
    }

    #[cfg(windows)]
    #[test]
    fn parent_watchdog_is_cancellable_and_joined() {
        let mut watchdog = ParentExitWatchdog::start(std::process::id()).expect("watchdog starts");

        assert!(!watchdog.parent_exited());
        watchdog.stop().expect("watchdog joins");
    }

    #[cfg(windows)]
    #[test]
    fn parent_watchdog_reports_parent_exit_without_exiting_the_helper_process() {
        let mut parent = std::process::Command::new("powershell.exe")
            .args(["-NoProfile", "-Command", "Start-Sleep -Seconds 60"])
            .spawn()
            .expect("parent fixture starts");
        let mut watchdog = ParentExitWatchdog::start(parent.id()).expect("watchdog starts");

        parent.kill().expect("parent fixture stops");
        parent.wait().expect("parent fixture is reaped");
        let deadline = Instant::now() + Duration::from_secs(2);
        while !watchdog.parent_exited() && Instant::now() < deadline {
            thread::sleep(HELPER_WATCHDOG_POLL);
        }

        assert!(watchdog.parent_exited());
        watchdog.stop().expect("watchdog joins");
    }

    #[cfg(windows)]
    #[test]
    fn stop_does_not_kill_broker_when_stop_signal_write_fails() {
        use windows_sys::Win32::System::Threading::{OpenProcess, PROCESS_SYNCHRONIZE};

        let base_dir = test_dir("force-stop");
        let mut child = std::process::Command::new("powershell.exe")
            .args(["-NoProfile", "-Command", "Start-Sleep -Seconds 60"])
            .spawn()
            .expect("test process starts");
        let sync_only_handle = unsafe { OpenProcess(PROCESS_SYNCHRONIZE, 0, child.id()) };
        assert!(
            !sync_only_handle.is_null(),
            "sync-only process handle opens"
        );

        let mut client = test_client(&base_dir);
        client.process = Some(ElevatedHelperProcess {
            handle: sync_only_handle,
        });
        assert_eq!(
            client
                .stop_with_timeout(Duration::ZERO)
                .expect_err("live broker remains owned"),
            "admin_mode_helper_settlement_timeout"
        );
        assert!(!client.stopped, "unsettled broker remains retryable");
        assert!(client.stop_file.exists(), "stop signal is retained");
        fs::remove_file(&client.stop_file).expect("stop signal fixture removes");
        fs::create_dir(&client.stop_file).expect("stop signal write blocker creates");

        let error = client
            .stop_with_timeout(Duration::ZERO)
            .expect_err("failed stop signal retains the live broker");
        assert!(
            error.starts_with("admin_mode_stop_signal_failed:"),
            "unexpected error: {error}"
        );

        assert!(!client.stopped);
        assert!(
            client.stop_file.is_dir(),
            "failed stop artifact is retained"
        );
        assert!(!client
            .process
            .as_ref()
            .is_some_and(ElevatedHelperProcess::has_exited));
        child
            .kill()
            .expect("test process is terminated by its owner");
        child.wait().expect("test process is reaped");
        let _ = fs::remove_dir_all(base_dir);
    }

    #[cfg(windows)]
    #[test]
    fn stop_does_not_kill_an_accepted_broker_before_child_settlement() {
        use windows_sys::Win32::System::Threading::{
            OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SYNCHRONIZE, PROCESS_TERMINATE,
        };

        let base_dir = test_dir("accepted-broker-settlement");
        let mut child = std::process::Command::new("powershell.exe")
            .args(["-NoProfile", "-Command", "Start-Sleep -Seconds 60"])
            .spawn()
            .expect("test process starts");
        let handle = unsafe {
            OpenProcess(
                PROCESS_SYNCHRONIZE | PROCESS_TERMINATE | PROCESS_QUERY_LIMITED_INFORMATION,
                0,
                child.id(),
            )
        };
        assert!(!handle.is_null(), "test process handle opens");

        let mut client = test_client(&base_dir);
        fs::write(&client.accepted_file, "accepted").expect("accepted fixture writes");
        client.process = Some(ElevatedHelperProcess { handle });

        assert_eq!(
            client
                .stop_with_timeout(Duration::ZERO)
                .expect_err("accepted broker stays owned until child settlement"),
            "admin_mode_helper_settlement_timeout"
        );
        assert!(!client.stopped);
        assert!(client.stop_file.exists(), "stop request remains observable");
        assert!(
            client.accepted_file.exists(),
            "acceptance proof is retained"
        );
        assert!(!client
            .process
            .as_ref()
            .is_some_and(ElevatedHelperProcess::has_exited));

        child
            .kill()
            .expect("test process is terminated by its owner");
        child.wait().expect("test process is reaped");
        let _ = fs::remove_dir_all(base_dir);
    }

    #[cfg(windows)]
    #[test]
    fn stop_accepts_zero_launcher_exit_and_removes_artifacts() {
        use std::os::windows::io::IntoRawHandle;

        let base_dir = test_dir("clean-launcher-stop");
        let mut client = test_client(&base_dir);
        fs::write(&client.data_file, "{}").expect("snapshot fixture writes");
        let child = std::process::Command::new("cmd.exe")
            .args(["/C", "exit", "0"])
            .spawn()
            .expect("clean launcher fixture starts");
        client.process = Some(ElevatedHelperProcess {
            handle: child.into_raw_handle() as HANDLE,
        });

        client
            .stop_with_timeout(Duration::from_secs(2))
            .expect("zero launcher exit proves shutdown");

        assert!(client.stopped);
        assert!(!client.stop_file.exists());
        assert!(!client.data_file.exists());
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
        let accepted_file = helper_dir.join("accepted.signal");
        fs::write(&data_file, "{}").expect("snapshot fixture writes");
        fs::write(&temp_file, "{}").expect("temp fixture writes");
        fs::write(&stop_file, "stop").expect("stop fixture writes");
        fs::write(&accepted_file, "accepted").expect("accepted fixture writes");

        ElevatedHelperClient::remove_stale_artifacts(&base_dir);

        assert!(!data_file.exists());
        assert!(!temp_file.exists());
        assert!(!stop_file.exists());
        assert!(!accepted_file.exists());
        let _ = fs::remove_dir_all(base_dir);
    }

    fn test_client(base_dir: &Path) -> ElevatedHelperClient {
        let mut client = ElevatedHelperClient::scripted_for_test(base_dir, Vec::new());
        client.token = "token".to_string();
        client
    }

    fn payload(token: &str, seq: u64, rows: Vec<ProcessSample>) -> String {
        String::from_utf8(
            snapshot_payload(token, seq, rows, Vec::new(), None).expect("snapshot serializes"),
        )
        .expect("snapshot is utf8")
    }

    fn append_frame(buffer: &mut Vec<u8>, payload: &[u8]) {
        buffer.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        buffer.extend_from_slice(payload);
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
            virtual_memory_bytes: Some(1),
            io_read_total_bytes: 0,
            io_write_total_bytes: 0,
            other_io_total_bytes: None,
            io_read_bps: 0,
            io_write_bps: 0,
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
