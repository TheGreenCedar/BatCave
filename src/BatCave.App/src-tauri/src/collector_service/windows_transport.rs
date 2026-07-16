use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    thread::JoinHandle,
    time::{Duration, Instant},
};

use sha2::{Digest, Sha256};
use windows_sys::Win32::{
    Foundation::{
        CloseHandle, GetLastError, ERROR_BROKEN_PIPE, ERROR_INSUFFICIENT_BUFFER, ERROR_NO_DATA,
        ERROR_PIPE_BUSY, ERROR_PIPE_CONNECTED, ERROR_PIPE_LISTENING, HANDLE, INVALID_HANDLE_VALUE,
    },
    Security::{
        Authorization::ConvertStringSecurityDescriptorToSecurityDescriptorW, GetLengthSid,
        GetTokenInformation, IsValidSid, RevertToSelf, TokenElevation, TokenSessionId, TokenUser,
        PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES, TOKEN_ELEVATION, TOKEN_INFORMATION_CLASS,
        TOKEN_QUERY, TOKEN_USER,
    },
    Storage::FileSystem::{
        CreateFileW, GetFileInformationByHandle, GetFileVersionInfoSizeW, GetFileVersionInfoW,
        ReadFile, VerQueryValueW, WriteFile, BY_HANDLE_FILE_INFORMATION, FILE_ATTRIBUTE_NORMAL,
        FILE_FLAG_FIRST_PIPE_INSTANCE, FILE_READ_ATTRIBUTES, FILE_SHARE_READ, OPEN_EXISTING,
        PIPE_ACCESS_DUPLEX, VS_FFI_SIGNATURE, VS_FIXEDFILEINFO,
    },
    System::{
        Pipes::{
            ConnectNamedPipe, CreateNamedPipeW, DisconnectNamedPipe, GetNamedPipeClientProcessId,
            GetNamedPipeClientSessionId, ImpersonateNamedPipeClient, PeekNamedPipe, PIPE_NOWAIT,
            PIPE_READMODE_BYTE, PIPE_REJECT_REMOTE_CLIENTS, PIPE_TYPE_BYTE,
        },
        Threading::{
            GetCurrentThread, GetProcessTimes, OpenProcess, OpenProcessToken, OpenThreadToken,
            QueryFullProcessImageNameW, PROCESS_QUERY_LIMITED_INFORMATION,
        },
    },
};

use super::{
    authorization::VerifiedPeer,
    framing::FrameDecoder,
    host::{extract_request_id, failure_reply, ServiceSession, SnapshotProvider},
    protocol::{ServiceIdentityV1, MAX_CLIENTS},
    transport_policy::{
        ClientTrustPolicy, ExecutableReleaseEvidence, VerifiedClientEvidence, PIPE_SDDL,
    },
};

pub(crate) const PIPE_NAME: &str = r"\\.\pipe\BatCaveCollector.v1";
const PIPE_BUFFER_BYTES: u32 = 64 * 1024;
const PIPE_POLL_INTERVAL: Duration = Duration::from_millis(10);
const CLIENT_IDLE_TIMEOUT: Duration = Duration::from_secs(30);
const CLIENT_WRITE_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_REQUESTS_PER_CONNECTION: usize = 4_096;

pub(crate) fn run_pipe_server(
    stop: Arc<AtomicBool>,
    identity: ServiceIdentityV1,
    snapshots: Arc<dyn SnapshotProvider>,
    ready: impl FnOnce() -> Result<(), String>,
) -> Result<(), String> {
    let service_directory = std::env::current_exe()
        .map_err(|error| format!("collector_service_executable_resolve_failed:{error}"))?
        .canonicalize()
        .map_err(|error| format!("collector_service_executable_canonicalize_failed:{error}"))?
        .parent()
        .ok_or_else(|| "collector_service_executable_parent_missing".to_string())?
        .to_string_lossy()
        .into_owned();
    let policy = ClientTrustPolicy::new(&service_directory).map_err(|error| error.to_string())?;
    let mut workers = Vec::<JoinHandle<()>>::new();
    let mut pipe = bind_before_ready(|| PipeConnection::create(true), ready)?;

    while !stop.load(Ordering::Acquire) {
        reap_workers(&mut workers);
        if workers.len() >= MAX_CLIENTS {
            std::thread::sleep(PIPE_POLL_INTERVAL);
            continue;
        }

        let connected = loop {
            if stop.load(Ordering::Acquire) {
                break false;
            }
            match pipe.connect_state()? {
                PipeConnectState::Connected => break true,
                PipeConnectState::Abandoned => break false,
                PipeConnectState::Listening => std::thread::sleep(PIPE_POLL_INTERVAL),
            }
        };
        if stop.load(Ordering::Acquire) {
            break;
        }
        // Create the successor while this instance still owns the namespace.
        let next_pipe = PipeConnection::create(false)?;
        if !connected {
            pipe = next_pipe;
            continue;
        }
        let connected_pipe = std::mem::replace(&mut pipe, next_pipe);

        let peer = match verify_pipe_peer(&connected_pipe, &policy) {
            Ok(peer) => peer,
            Err(_) => continue,
        };
        let worker_stop = Arc::clone(&stop);
        let worker_identity = identity.clone();
        let worker_snapshots = Arc::clone(&snapshots);
        workers.push(
            std::thread::Builder::new()
                .name("batcave-collector-client".to_string())
                .spawn(move || {
                    let _ = serve_client(
                        connected_pipe,
                        peer,
                        worker_stop,
                        worker_identity,
                        worker_snapshots,
                    );
                })
                .map_err(|error| format!("collector_service_client_spawn_failed:{error}"))?,
        );
    }

    for worker in workers {
        let _ = worker.join();
    }
    Ok(())
}

fn bind_before_ready<T>(
    bind: impl FnOnce() -> Result<T, String>,
    ready: impl FnOnce() -> Result<(), String>,
) -> Result<T, String> {
    let bound = bind()?;
    ready()?;
    Ok(bound)
}

fn reap_workers(workers: &mut Vec<JoinHandle<()>>) {
    let mut index = 0;
    while index < workers.len() {
        if workers[index].is_finished() {
            let worker = workers.swap_remove(index);
            let _ = worker.join();
        } else {
            index += 1;
        }
    }
}

fn serve_client(
    pipe: PipeConnection,
    peer: VerifiedPeer,
    stop: Arc<AtomicBool>,
    identity: ServiceIdentityV1,
    snapshots: Arc<dyn SnapshotProvider>,
) -> Result<(), String> {
    let mut session = ServiceSession::new(identity, snapshots);
    let mut decoder = FrameDecoder::default();
    let mut last_activity = Instant::now();
    let mut request_count = 0_usize;

    while !stop.load(Ordering::Acquire) && last_activity.elapsed() < CLIENT_IDLE_TIMEOUT {
        let Some(bytes) = pipe.read_available()? else {
            std::thread::sleep(PIPE_POLL_INTERVAL);
            continue;
        };
        last_activity = Instant::now();
        let payloads = decoder
            .push(&bytes)
            .map_err(|error| format!("collector_service_frame_rejected:{error}"))?;
        for payload in payloads {
            request_count = request_count.saturating_add(1);
            if request_count > MAX_REQUESTS_PER_CONNECTION {
                return Err("collector_service_connection_request_limit_exceeded".to_string());
            }
            let reply = match session.handle_payload(&peer, &payload) {
                Ok(reply) => reply,
                Err(failure) => {
                    let Some(request_id) = extract_request_id(&payload) else {
                        return Err(format!("collector_service_request_rejected:{failure}"));
                    };
                    failure_reply(request_id, &failure).map_err(|error| error.to_string())?
                }
            };
            pipe.write_all(&reply.frame, &stop)?;
            if reply.close {
                return Ok(());
            }
        }
    }
    Ok(())
}

fn verify_pipe_peer(
    pipe: &PipeConnection,
    policy: &ClientTrustPolicy,
) -> Result<VerifiedPeer, String> {
    let (process_id, pipe_session_id) = pipe.client_identity()?;
    let process =
        OwnedHandle::new(unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, process_id) })
            .ok_or_else(last_error("collector_service_peer_process_open_failed"))?;
    let process_started_at = process_started_at(process.raw())?;
    let executable_path = process_image_path(process.raw())?;
    let canonical_path = PathBuf::from(&executable_path)
        .canonicalize()
        .map_err(|error| format!("collector_service_peer_path_canonicalize_failed:{error}"))?;
    let canonical_path_wide = wide(&canonical_path.to_string_lossy());
    let executable = OwnedHandle::new(unsafe {
        CreateFileW(
            canonical_path_wide.as_ptr(),
            FILE_READ_ATTRIBUTES,
            FILE_SHARE_READ,
            std::ptr::null(),
            OPEN_EXISTING,
            FILE_ATTRIBUTE_NORMAL,
            std::ptr::null_mut(),
        )
    })
    .ok_or_else(last_error("collector_service_peer_image_open_failed"))?;
    let executable_file_identity = file_identity(executable.raw())?;
    let executable_release = executable_release(&canonical_path_wide)?;

    let mut process_token = std::ptr::null_mut();
    if unsafe { OpenProcessToken(process.raw(), TOKEN_QUERY, &mut process_token) } == 0 {
        return Err(last_error_message(
            "collector_service_peer_process_token_open_failed",
        ));
    }
    let process_token = OwnedHandle::new(process_token)
        .ok_or_else(|| "collector_service_peer_process_token_invalid".to_string())?;
    let process_token = token_evidence(process_token.raw())?;
    let impersonated_token = impersonated_pipe_token(pipe.raw())?;

    let (confirmed_process_id, confirmed_session_id) = pipe.client_identity()?;
    if confirmed_process_id != process_id || confirmed_session_id != pipe_session_id {
        return Err("collector_service_peer_pipe_identity_changed".to_string());
    }

    policy
        .verify(VerifiedClientEvidence {
            process_id,
            process_started_at,
            pipe_session_id,
            process_token_session_id: process_token.session_id,
            impersonated_token_session_id: impersonated_token.session_id,
            process_principal_identity: process_token.principal_identity,
            impersonated_principal_identity: impersonated_token.principal_identity,
            executable_file_identity,
            process_token_elevated: process_token.elevated,
            impersonated_token_elevated: impersonated_token.elevated,
            executable_path: canonical_path.to_string_lossy().into_owned(),
            executable_release,
        })
        .map_err(|error| error.to_string())
}

pub(super) struct TokenEvidence {
    pub(super) session_id: u32,
    pub(super) principal_identity: [u8; 32],
    pub(super) elevated: bool,
}

pub(super) fn token_evidence(token: HANDLE) -> Result<TokenEvidence, String> {
    let user = token_information(token, TokenUser)?;
    let token_user = unsafe { &*(user.as_ptr().cast::<TOKEN_USER>()) };
    if token_user.User.Sid.is_null() || unsafe { IsValidSid(token_user.User.Sid) } == 0 {
        return Err("collector_service_peer_token_sid_invalid".to_string());
    }
    let sid_length = unsafe { GetLengthSid(token_user.User.Sid) } as usize;
    if sid_length == 0 {
        return Err("collector_service_peer_token_sid_empty".to_string());
    }
    let sid = unsafe { std::slice::from_raw_parts(token_user.User.Sid.cast::<u8>(), sid_length) };
    let principal_identity = Sha256::digest(sid).into();

    let session = token_information(token, TokenSessionId)?;
    let session_id = unsafe { *(session.as_ptr().cast::<u32>()) };
    let elevation = token_information(token, TokenElevation)?;
    let elevated =
        unsafe { (*(elevation.as_ptr().cast::<TOKEN_ELEVATION>())).TokenIsElevated != 0 };
    Ok(TokenEvidence {
        session_id,
        principal_identity,
        elevated,
    })
}

fn impersonated_pipe_token(pipe: HANDLE) -> Result<TokenEvidence, String> {
    if unsafe { ImpersonateNamedPipeClient(pipe) } == 0 {
        return Err(last_error_message(
            "collector_service_peer_impersonation_failed",
        ));
    }
    let mut guard = RevertGuard(true);
    let result = (|| {
        let mut token = std::ptr::null_mut();
        if unsafe { OpenThreadToken(GetCurrentThread(), TOKEN_QUERY, 1, &mut token) } == 0 {
            return Err(last_error_message(
                "collector_service_peer_thread_token_open_failed",
            ));
        }
        let token = OwnedHandle::new(token)
            .ok_or_else(|| "collector_service_peer_thread_token_invalid".to_string())?;
        token_evidence(token.raw())
    })();
    if unsafe { RevertToSelf() } == 0 {
        return Err(last_error_message("collector_service_peer_revert_failed"));
    }
    guard.0 = false;
    result
}

struct RevertGuard(bool);

impl Drop for RevertGuard {
    fn drop(&mut self) {
        if self.0 {
            unsafe {
                RevertToSelf();
            }
        }
    }
}

fn token_information(
    token: HANDLE,
    class: TOKEN_INFORMATION_CLASS,
) -> Result<AlignedBuffer, String> {
    let mut required = 0_u32;
    let first =
        unsafe { GetTokenInformation(token, class, std::ptr::null_mut(), 0, &mut required) };
    if first != 0 || required == 0 || unsafe { GetLastError() } != ERROR_INSUFFICIENT_BUFFER {
        return Err(last_error_message(
            "collector_service_peer_token_query_size_failed",
        ));
    }
    let mut buffer = AlignedBuffer::new(required as usize);
    if unsafe {
        GetTokenInformation(
            token,
            class,
            buffer.as_mut_ptr().cast(),
            required,
            &mut required,
        )
    } == 0
    {
        return Err(last_error_message(
            "collector_service_peer_token_query_failed",
        ));
    }
    Ok(buffer)
}

struct AlignedBuffer(Vec<usize>);

impl AlignedBuffer {
    fn new(bytes: usize) -> Self {
        Self(vec![0; bytes.div_ceil(std::mem::size_of::<usize>())])
    }

    fn as_ptr(&self) -> *const usize {
        self.0.as_ptr()
    }

    fn as_mut_ptr(&mut self) -> *mut usize {
        self.0.as_mut_ptr()
    }
}

pub(super) fn process_started_at(process: HANDLE) -> Result<u64, String> {
    let mut created = Default::default();
    let mut exited = Default::default();
    let mut kernel = Default::default();
    let mut user = Default::default();
    if unsafe { GetProcessTimes(process, &mut created, &mut exited, &mut kernel, &mut user) } == 0 {
        return Err(last_error_message(
            "collector_service_peer_process_times_failed",
        ));
    }
    Ok((u64::from(created.dwHighDateTime) << 32) | u64::from(created.dwLowDateTime))
}

pub(super) fn process_image_path(process: HANDLE) -> Result<String, String> {
    let mut buffer = vec![0_u16; 32_768];
    let mut length = buffer.len() as u32;
    if unsafe { QueryFullProcessImageNameW(process, 0, buffer.as_mut_ptr(), &mut length) } == 0 {
        return Err(last_error_message(
            "collector_service_peer_process_path_failed",
        ));
    }
    buffer.truncate(length as usize);
    String::from_utf16(&buffer)
        .map_err(|_| "collector_service_peer_process_path_utf16_invalid".to_string())
}

pub(super) fn file_identity(file: HANDLE) -> Result<[u8; 32], String> {
    let mut info = BY_HANDLE_FILE_INFORMATION::default();
    if unsafe { GetFileInformationByHandle(file, &mut info) } == 0 {
        return Err(last_error_message(
            "collector_service_peer_file_identity_failed",
        ));
    }
    let mut digest = Sha256::new();
    digest.update(b"batcave_windows_file_identity_v1");
    digest.update(info.dwVolumeSerialNumber.to_le_bytes());
    digest.update(info.nFileIndexHigh.to_le_bytes());
    digest.update(info.nFileIndexLow.to_le_bytes());
    Ok(digest.finalize().into())
}

pub(super) fn executable_release(path: &[u16]) -> Result<ExecutableReleaseEvidence, String> {
    let mut ignored = 0_u32;
    let size = unsafe { GetFileVersionInfoSizeW(path.as_ptr(), &mut ignored) };
    if size == 0 {
        return Err(last_error_message(
            "collector_service_peer_file_version_size_failed",
        ));
    }
    let mut buffer = AlignedBuffer::new(size as usize);
    if unsafe { GetFileVersionInfoW(path.as_ptr(), 0, size, buffer.as_mut_ptr().cast()) } == 0 {
        return Err(last_error_message(
            "collector_service_peer_file_version_read_failed",
        ));
    }
    let mut value = std::ptr::null_mut();
    let mut value_len = 0_u32;
    let root = wide("\\");
    if unsafe {
        VerQueryValueW(
            buffer.as_ptr().cast(),
            root.as_ptr(),
            &mut value,
            &mut value_len,
        )
    } == 0
        || value.is_null()
        || value_len < std::mem::size_of::<VS_FIXEDFILEINFO>() as u32
    {
        return Err("collector_service_peer_file_version_invalid".to_string());
    }
    let version = unsafe { &*(value.cast::<VS_FIXEDFILEINFO>()) };
    if version.dwSignature != VS_FFI_SIGNATURE as u32 {
        return Err("collector_service_peer_file_version_signature_invalid".to_string());
    }
    let fixed = ExecutableReleaseEvidence {
        major: (version.dwFileVersionMS >> 16) as u16,
        minor: version.dwFileVersionMS as u16,
        patch: (version.dwFileVersionLS >> 16) as u16,
        product_version: product_version_string(&buffer)?,
    };
    Ok(fixed)
}

fn product_version_string(buffer: &AlignedBuffer) -> Result<String, String> {
    let mut translations = std::ptr::null_mut();
    let mut translations_len = 0_u32;
    let translations_path = wide(r"\VarFileInfo\Translation");
    if unsafe {
        VerQueryValueW(
            buffer.as_ptr().cast(),
            translations_path.as_ptr(),
            &mut translations,
            &mut translations_len,
        )
    } == 0
        || translations.is_null()
        || translations_len < 4
        || !translations_len.is_multiple_of(4)
    {
        return Err("collector_service_peer_product_version_translation_invalid".to_string());
    }

    let translations = unsafe {
        std::slice::from_raw_parts(translations.cast::<u16>(), translations_len as usize / 2)
    };
    for pair in translations.chunks_exact(2) {
        let query = wide(&format!(
            r"\StringFileInfo\{:04x}{:04x}\ProductVersion",
            pair[0], pair[1]
        ));
        let mut value = std::ptr::null_mut();
        let mut value_len = 0_u32;
        if unsafe {
            VerQueryValueW(
                buffer.as_ptr().cast(),
                query.as_ptr(),
                &mut value,
                &mut value_len,
            )
        } == 0
            || value.is_null()
            || value_len <= 1
        {
            continue;
        }
        let product_version = unsafe {
            std::slice::from_raw_parts(value.cast::<u16>(), value_len.saturating_sub(1) as usize)
        };
        let product_version = String::from_utf16(product_version)
            .map_err(|_| "collector_service_peer_product_version_utf16_invalid".to_string())?;
        if !product_version.is_empty() {
            return Ok(product_version);
        }
    }

    Err("collector_service_peer_product_version_missing".to_string())
}

struct PipeConnection {
    handle: OwnedHandle,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PipeConnectState {
    Listening,
    Connected,
    Abandoned,
}

fn classify_pipe_connect(connected: bool, error: u32) -> Result<PipeConnectState, String> {
    if connected || error == ERROR_PIPE_CONNECTED {
        Ok(PipeConnectState::Connected)
    } else {
        match error {
            ERROR_PIPE_LISTENING => Ok(PipeConnectState::Listening),
            ERROR_NO_DATA => Ok(PipeConnectState::Abandoned),
            error => Err(format!("collector_service_pipe_connect_failed:{error}")),
        }
    }
}

impl PipeConnection {
    fn create(first_instance: bool) -> Result<Self, String> {
        let mut security = PipeSecurity::new()?;
        let pipe_name = wide(PIPE_NAME);
        let first = if first_instance {
            FILE_FLAG_FIRST_PIPE_INSTANCE
        } else {
            0
        };
        let handle = unsafe {
            CreateNamedPipeW(
                pipe_name.as_ptr(),
                PIPE_ACCESS_DUPLEX | first,
                PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_NOWAIT | PIPE_REJECT_REMOTE_CLIENTS,
                MAX_CLIENTS as u32,
                PIPE_BUFFER_BYTES,
                PIPE_BUFFER_BYTES,
                0,
                security.attributes(),
            )
        };
        let handle = OwnedHandle::new(handle)
            .ok_or_else(last_error("collector_service_pipe_create_failed"))?;
        Ok(Self { handle })
    }

    fn raw(&self) -> HANDLE {
        self.handle.raw()
    }

    fn connect_state(&self) -> Result<PipeConnectState, String> {
        let connected = unsafe { ConnectNamedPipe(self.raw(), std::ptr::null_mut()) } != 0;
        classify_pipe_connect(connected, unsafe { GetLastError() })
    }

    fn client_identity(&self) -> Result<(u32, u32), String> {
        let mut process_id = 0_u32;
        let mut session_id = 0_u32;
        if unsafe { GetNamedPipeClientProcessId(self.raw(), &mut process_id) } == 0
            || unsafe { GetNamedPipeClientSessionId(self.raw(), &mut session_id) } == 0
        {
            return Err(last_error_message(
                "collector_service_pipe_client_identity_failed",
            ));
        }
        if process_id == 0 {
            return Err("collector_service_pipe_client_identity_invalid".to_string());
        }
        Ok((process_id, session_id))
    }

    fn read_available(&self) -> Result<Option<Vec<u8>>, String> {
        let mut available = 0_u32;
        if unsafe {
            PeekNamedPipe(
                self.raw(),
                std::ptr::null_mut(),
                0,
                std::ptr::null_mut(),
                &mut available,
                std::ptr::null_mut(),
            )
        } == 0
        {
            return match unsafe { GetLastError() } {
                ERROR_BROKEN_PIPE | ERROR_NO_DATA => {
                    Err("collector_service_pipe_disconnected".to_string())
                }
                error => Err(format!("collector_service_pipe_peek_failed:{error}")),
            };
        }
        if available == 0 {
            return Ok(None);
        }
        let mut bytes = vec![0_u8; available.min(PIPE_BUFFER_BYTES) as usize];
        let mut read = 0_u32;
        if unsafe {
            ReadFile(
                self.raw(),
                bytes.as_mut_ptr().cast(),
                bytes.len() as u32,
                &mut read,
                std::ptr::null_mut(),
            )
        } == 0
        {
            return match unsafe { GetLastError() } {
                ERROR_BROKEN_PIPE | ERROR_NO_DATA => {
                    Err("collector_service_pipe_disconnected".to_string())
                }
                error => Err(format!("collector_service_pipe_read_failed:{error}")),
            };
        }
        bytes.truncate(read as usize);
        Ok((!bytes.is_empty()).then_some(bytes))
    }

    fn write_all(&self, bytes: &[u8], stop: &AtomicBool) -> Result<(), String> {
        let deadline = Instant::now() + CLIENT_WRITE_TIMEOUT;
        let mut offset = 0_usize;
        while offset < bytes.len() {
            if stop.load(Ordering::Acquire) {
                return Err("collector_service_stopping".to_string());
            }
            if Instant::now() >= deadline {
                return Err("collector_service_pipe_write_timeout".to_string());
            }
            let chunk = (bytes.len() - offset).min(PIPE_BUFFER_BYTES as usize);
            let mut written = 0_u32;
            let ok = unsafe {
                WriteFile(
                    self.raw(),
                    bytes[offset..offset + chunk].as_ptr().cast(),
                    chunk as u32,
                    &mut written,
                    std::ptr::null_mut(),
                )
            };
            if ok != 0 && written > 0 {
                offset += written as usize;
                continue;
            }
            if ok == 0 {
                match unsafe { GetLastError() } {
                    ERROR_PIPE_BUSY => {}
                    ERROR_BROKEN_PIPE | ERROR_NO_DATA => {
                        return Err("collector_service_pipe_disconnected".to_string())
                    }
                    error => return Err(format!("collector_service_pipe_write_failed:{error}")),
                }
            }
            std::thread::sleep(PIPE_POLL_INTERVAL);
        }
        Ok(())
    }
}

unsafe impl Send for PipeConnection {}

impl Drop for PipeConnection {
    fn drop(&mut self) {
        unsafe {
            DisconnectNamedPipe(self.raw());
        }
    }
}

pub(super) struct OwnedHandle(HANDLE);

impl OwnedHandle {
    pub(super) fn new(handle: HANDLE) -> Option<Self> {
        (!handle.is_null() && handle != INVALID_HANDLE_VALUE).then_some(Self(handle))
    }

    pub(super) fn raw(&self) -> HANDLE {
        self.0
    }
}

unsafe impl Send for OwnedHandle {}

impl Drop for OwnedHandle {
    fn drop(&mut self) {
        unsafe {
            CloseHandle(self.0);
        }
    }
}

struct PipeSecurity {
    descriptor: PSECURITY_DESCRIPTOR,
    attributes: SECURITY_ATTRIBUTES,
}

impl PipeSecurity {
    fn new() -> Result<Self, String> {
        let mut descriptor = std::ptr::null_mut();
        let sddl = wide(PIPE_SDDL);
        if unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                sddl.as_ptr(),
                1,
                &mut descriptor,
                std::ptr::null_mut(),
            )
        } == 0
        {
            return Err(last_error_message("collector_service_pipe_security_failed"));
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

    fn attributes(&mut self) -> *const SECURITY_ATTRIBUTES {
        &self.attributes
    }
}

impl Drop for PipeSecurity {
    fn drop(&mut self) {
        if !self.descriptor.is_null() {
            unsafe {
                windows_sys::Win32::Foundation::LocalFree(self.descriptor.cast());
            }
        }
    }
}

fn last_error(context: &'static str) -> impl FnOnce() -> String {
    move || last_error_message(context)
}

pub(super) fn last_error_message(context: &str) -> String {
    format!("{context}:{}", unsafe { GetLastError() })
}

pub(super) fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn client_restricted_pipe_security_descriptor_is_accepted_by_windows() {
        PipeSecurity::new().expect("valid protected pipe security descriptor");
    }

    #[test]
    fn abandoned_nonblocking_pipe_instance_does_not_fail_the_listener() {
        assert_eq!(
            classify_pipe_connect(false, ERROR_NO_DATA),
            Ok(PipeConnectState::Abandoned)
        );
        assert_eq!(
            classify_pipe_connect(false, ERROR_PIPE_LISTENING),
            Ok(PipeConnectState::Listening)
        );
        assert_eq!(
            classify_pipe_connect(false, ERROR_PIPE_CONNECTED),
            Ok(PipeConnectState::Connected)
        );
        assert!(classify_pipe_connect(false, 5).is_err());
    }

    #[test]
    fn service_readiness_is_published_only_after_the_pipe_is_bound() {
        let mut ready_called = false;
        assert_eq!(
            bind_before_ready(
                || Err::<(), _>("bind failed".to_string()),
                || {
                    ready_called = true;
                    Ok(())
                },
            ),
            Err("bind failed".to_string())
        );
        assert!(!ready_called);

        assert_eq!(
            bind_before_ready(
                || Ok("bound"),
                || {
                    ready_called = true;
                    Ok(())
                },
            ),
            Ok("bound")
        );
        assert!(ready_called);
    }
}
