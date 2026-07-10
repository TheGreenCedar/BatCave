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

use crate::{cli_args, contracts::ProcessSample, telemetry::TelemetryCollector};

const HELPER_INTERVAL_MS: u64 = 500;
const HELPER_SNAPSHOT_GRACE: Duration = Duration::from_secs(2);
const HELPER_STOP_GRACE: Duration = Duration::from_secs(2);
const HELPER_FORCE_EXIT_GRACE: Duration = Duration::from_secs(2);
const HELPER_TOKEN_BYTES: usize = 32;
const MAX_PIPE_FRAME_BYTES: usize = 8 * 1024 * 1024;

#[derive(Debug)]
pub struct ElevatedHelperClient {
    data_file: PathBuf,
    stop_file: PathBuf,
    token: String,
    last_seq: u64,
    started_at: Instant,
    last_snapshot_at: Option<Instant>,
    read_buffer: Vec<u8>,
    #[cfg(windows)]
    pipe: Option<NamedPipeServer>,
    process: Option<ElevatedHelperProcess>,
    stopped: bool,
}

#[derive(Debug, Clone)]
pub enum ElevatedPoll {
    Fresh(Vec<ProcessSample>),
    Held,
    Pending,
}

impl ElevatedHelperClient {
    pub fn start(base_dir: &Path) -> Result<Self, String> {
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
            &session.token,
            std::process::id(),
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
            token: session.token,
            last_seq: 0,
            started_at: Instant::now(),
            last_snapshot_at: None,
            read_buffer: Vec::new(),
            #[cfg(windows)]
            pipe,
            process,
            stopped: false,
        })
    }

    pub fn poll_rows(&mut self) -> Result<ElevatedPoll, String> {
        if self
            .process
            .as_ref()
            .is_some_and(ElevatedHelperProcess::has_exited)
        {
            remove_artifacts_for_snapshot(&self.data_file);
            return Err("admin_mode_helper_exited".to_string());
        }

        #[cfg(windows)]
        if let Some(pipe) = &mut self.pipe {
            let mut rows = None;
            for payload in pipe.read_payloads(&mut self.read_buffer)? {
                if let Some(snapshot_rows) = self.accept_snapshot_payload(&payload)? {
                    rows = Some(snapshot_rows);
                }
            }
            if let Some(rows) = rows {
                self.last_snapshot_at = Some(Instant::now());
                return Ok(ElevatedPoll::Fresh(rows));
            }
        }

        let since = self.last_snapshot_at.unwrap_or(self.started_at).elapsed();
        if since > HELPER_SNAPSHOT_GRACE {
            return Err("admin_mode_snapshot_timeout".to_string());
        }
        Ok(if self.last_snapshot_at.is_some() {
            ElevatedPoll::Held
        } else {
            ElevatedPoll::Pending
        })
    }

    pub fn stop(&mut self) -> Result<(), String> {
        self.stop_with_timeouts(HELPER_STOP_GRACE, HELPER_FORCE_EXIT_GRACE)
    }

    fn stop_with_timeouts(
        &mut self,
        graceful_timeout: Duration,
        forced_timeout: Duration,
    ) -> Result<(), String> {
        if self.stopped {
            return Ok(());
        }
        self.stopped = true;

        let stop_write_error = fs::write(&self.stop_file, "stop").err();
        let exited = self
            .process
            .as_ref()
            .is_none_or(|process| process.wait(graceful_timeout));
        let exited = if exited {
            true
        } else {
            self.process
                .as_ref()
                .is_some_and(|process| process.terminate() && process.wait(forced_timeout))
        };

        if exited {
            remove_artifacts_for_snapshot(&self.data_file);
            Ok(())
        } else {
            remove_snapshot_artifacts(&self.data_file);
            let suffix = stop_write_error
                .map(|error| format!(":stop_signal_failed:{error}"))
                .unwrap_or_default();
            Err(format!("admin_mode_helper_termination_failed{suffix}"))
        }
    }

    pub fn remove_stale_artifacts(base_dir: &Path) {
        remove_helper_artifacts(base_dir);
    }

    fn accept_snapshot_payload(
        &mut self,
        payload: &str,
    ) -> Result<Option<Vec<ProcessSample>>, String> {
        let snapshot = serde_json::from_str::<ElevatedSnapshot>(payload)
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
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ElevatedHelperArgs {
    pipe_name: String,
    stop_file: PathBuf,
    token: String,
    parent_pid: u32,
}

pub fn run_cli(args: &[String]) -> Option<i32> {
    if !args.iter().any(|arg| arg == "--elevated-helper") {
        return None;
    }

    match parse_helper_args(args).and_then(validate_helper_args) {
        Ok(helper_args) => Some(run_elevated_helper(helper_args)),
        Err(error) => {
            eprintln!("{error}");
            Some(2)
        }
    }
}

#[cfg(windows)]
fn run_elevated_helper(args: ElevatedHelperArgs) -> i32 {
    let mut seq = 0_u64;
    if let Err(error) = start_helper_exit_watchdog(args.parent_pid, args.stop_file.clone()) {
        eprintln!("{error}");
        return 1;
    }
    let pipe = match PipeClient::connect(&args.pipe_name) {
        Ok(pipe) => pipe,
        Err(error) => {
            eprintln!("{error}");
            return 1;
        }
    };
    let collector = TelemetryCollector::for_elevated_helper();
    while !args.stop_file.exists() {
        seq = seq.saturating_add(1);
        match collector
            .collect()
            .map(|sample| sample.processes)
            .and_then(|rows| pipe.write_snapshot(&args.token, seq, rows))
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

#[cfg(windows)]
fn start_helper_exit_watchdog(parent_pid: u32, stop_file: PathBuf) -> Result<(), String> {
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

    thread::Builder::new()
        .name("batcave-admin-watchdog".to_string())
        .spawn(move || {
            let parent_handle = parent_handle_value as HANDLE;
            loop {
                if stop_file.exists() {
                    unsafe { CloseHandle(parent_handle) };
                    std::process::exit(0);
                }

                match unsafe { WaitForSingleObject(parent_handle, 50) } {
                    WAIT_TIMEOUT => {}
                    WAIT_OBJECT_0 => {
                        unsafe { CloseHandle(parent_handle) };
                        std::process::exit(0);
                    }
                    _ => {
                        unsafe { CloseHandle(parent_handle) };
                        std::process::exit(1);
                    }
                }
            }
        })
        .map(|_| ())
        .map_err(|error| {
            unsafe { CloseHandle(parent_handle) };
            format!("admin_mode_parent_watch_start_failed:{error}")
        })
}

#[cfg(not(windows))]
fn run_elevated_helper(_args: ElevatedHelperArgs) -> i32 {
    eprintln!("admin_mode_requires_windows");
    1
}

fn snapshot_payload(token: &str, seq: u64, rows: Vec<ProcessSample>) -> Result<Vec<u8>, String> {
    let snapshot = ElevatedSnapshot {
        token: token.to_string(),
        seq,
        rows,
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
    ) -> Result<(), String> {
        let payload = snapshot_payload(token, seq, rows)?;
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
    let pipe_name = required_value(args, "--pipe-name")?;
    let stop_file = PathBuf::from(required_value(args, "--stop-file")?);
    let token = required_value(args, "--token")?;
    let parent_pid = required_value(args, "--parent-pid")?
        .parse::<u32>()
        .map_err(|_| "invalid_elevated_helper_parent_pid".to_string())?;
    Ok(ElevatedHelperArgs {
        pipe_name,
        stop_file,
        token,
        parent_pid,
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
    reject_reparse_path(helper_root, true)?;
    reject_reparse_path(run_dir, true)?;
    reject_reparse_path(&args.stop_file, false)?;

    Ok(args)
}

fn reject_unknown_helper_args(args: &[String]) -> Result<(), String> {
    let known_with_value = ["--pipe-name", "--stop-file", "--token", "--parent-pid"];
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

    fn terminate(&self) -> bool {
        use windows_sys::Win32::System::Threading::TerminateProcess;

        unsafe { TerminateProcess(self.handle, 1) != 0 }
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
}

#[cfg(windows)]
fn launch_elevated_helper(
    pipe_name: &str,
    stop_file: &Path,
    token: &str,
    parent_pid: u32,
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
        "--elevated-helper --pipe-name \"{}\" --stop-file \"{}\" --token \"{}\" --parent-pid {}",
        pipe_name,
        stop_file.display(),
        token,
        parent_pid,
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
        return Err("admin_mode_launch_missing_process_handle".to_string());
    }

    Ok(Some(ElevatedHelperProcess {
        handle: info.hProcess,
    }))
}

#[cfg(not(windows))]
fn launch_elevated_helper(
    _pipe_name: &str,
    _stop_file: &Path,
    _token: &str,
    _parent_pid: u32,
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
            parse_helper_args(&args),
            Err("missing_value_for_argument:--pipe-name".to_string())
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
        assert_eq!(first.data_file.parent(), first.stop_file.parent());
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

        assert_eq!(first[0].pid, "10");

        assert!(client
            .accept_snapshot_payload(&payload(&token, 2, vec![sample("20")]))
            .expect("stale snapshot is ok")
            .is_none());

        let newer = client
            .accept_snapshot_payload(&payload(&token, 3, vec![sample("30")]))
            .expect("new snapshot reads")
            .expect("new rows exist");

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
            token: token.clone(),
            parent_pid: 1,
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
    fn helper_rows_hold_for_two_seconds_then_fail_closed() {
        let base_dir = test_dir("hold");
        let mut client = test_client(&base_dir);
        client.last_snapshot_at = Some(Instant::now());
        assert!(matches!(
            client.poll_rows().expect("recent rows hold"),
            ElevatedPoll::Held
        ));

        client.last_snapshot_at =
            Some(Instant::now() - HELPER_SNAPSHOT_GRACE - Duration::from_millis(1));
        assert_eq!(
            client.poll_rows().expect_err("expired rows fail closed"),
            "admin_mode_snapshot_timeout"
        );
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

        client.stop().expect("helper stops");

        assert!(!client.stop_file.exists());
        assert!(!client.data_file.exists());
        assert!(!temp_file.exists());
        let _ = fs::remove_dir_all(base_dir);
    }

    #[cfg(windows)]
    #[test]
    fn stop_force_terminates_process_after_grace_timeout() {
        use windows_sys::Win32::System::Threading::{
            OpenProcess, PROCESS_SYNCHRONIZE, PROCESS_TERMINATE,
        };

        let base_dir = test_dir("force-stop");
        let mut child = std::process::Command::new("powershell.exe")
            .args(["-NoProfile", "-Command", "Start-Sleep -Seconds 60"])
            .spawn()
            .expect("test process starts");
        let handle = unsafe { OpenProcess(PROCESS_SYNCHRONIZE | PROCESS_TERMINATE, 0, child.id()) };
        assert!(!handle.is_null(), "test process handle opens");

        let mut client = test_client(&base_dir);
        client.process = Some(ElevatedHelperProcess { handle });
        client
            .stop_with_timeouts(Duration::ZERO, Duration::from_secs(2))
            .expect("helper is force terminated");

        assert!(client
            .process
            .as_ref()
            .is_some_and(ElevatedHelperProcess::has_exited));
        child.wait().expect("test process is reaped");
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

    fn test_client(base_dir: &Path) -> ElevatedHelperClient {
        fs::create_dir_all(base_dir).expect("test dir exists");
        ElevatedHelperClient {
            data_file: base_dir.join("snapshot.json"),
            stop_file: base_dir.join("stop.signal"),
            token: "token".to_string(),
            last_seq: 0,
            started_at: Instant::now(),
            last_snapshot_at: None,
            read_buffer: Vec::new(),
            #[cfg(windows)]
            pipe: None,
            process: None,
            stopped: false,
        }
    }

    fn payload(token: &str, seq: u64, rows: Vec<ProcessSample>) -> String {
        String::from_utf8(snapshot_payload(token, seq, rows).expect("snapshot serializes"))
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
