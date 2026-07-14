use std::{
    path::PathBuf,
    time::{Duration, Instant},
};

use sha2::{Digest, Sha256};
use windows_sys::Win32::{
    Foundation::{
        GetLastError, ERROR_ACCESS_DENIED, ERROR_BROKEN_PIPE, ERROR_NO_DATA, ERROR_PIPE_BUSY,
        HANDLE,
    },
    Security::{CreateWellKnownSid, WinLocalSystemSid, TOKEN_QUERY},
    Storage::FileSystem::{
        CreateFileW, ReadFile, WriteFile, FILE_ATTRIBUTE_NORMAL, FILE_READ_ATTRIBUTES,
        FILE_READ_DATA, FILE_SHARE_READ, FILE_WRITE_DATA, OPEN_EXISTING,
    },
    System::{
        Pipes::{GetNamedPipeServerProcessId, PeekNamedPipe, WaitNamedPipeW},
        Services::{
            CloseServiceHandle, OpenSCManagerW, OpenServiceW, QueryServiceStatusEx,
            SC_MANAGER_CONNECT, SC_STATUS_PROCESS_INFO, SERVICE_QUERY_STATUS, SERVICE_RUNNING,
            SERVICE_STATUS_PROCESS, SERVICE_WIN32_OWN_PROCESS,
        },
        Threading::{OpenProcess, OpenProcessToken, PROCESS_QUERY_LIMITED_INFORMATION},
    },
};

use super::{
    authorization::VerifiedServicePeer,
    client::{ClientFailure, ClientFailureKind, ClientTransport},
    framing::{encode_json_frame, FrameDecoder},
    protocol::{
        decode_response, ClientRequestV1, ReleaseIdentityV1, ServiceResponseV1,
        COLLECTOR_SERVICE_NAME,
    },
    transport_policy::DESKTOP_EXECUTABLE_NAME,
    windows_transport::{
        executable_release, file_identity, last_error_message, process_image_path,
        process_started_at, token_evidence, wide, OwnedHandle, PIPE_NAME,
    },
};

const SERVICE_EXECUTABLE_NAME: &str = "batcave-collector-service.exe";
const CONNECT_TIMEOUT_MS: u32 = 250;
const OPERATION_TIMEOUT: Duration = Duration::from_secs(2);
const PIPE_POLL_INTERVAL: Duration = Duration::from_millis(5);
const PIPE_BUFFER_BYTES: usize = 64 * 1024;
const ERROR_SERVICE_DOES_NOT_EXIST: u32 = 1_060;

pub(crate) struct WindowsServiceTransport {
    pipe: OwnedHandle,
    peer: VerifiedServicePeer,
    decoder: FrameDecoder,
}

impl WindowsServiceTransport {
    pub(crate) fn connect() -> Result<Self, ClientFailure> {
        let pipe_name = wide(PIPE_NAME);
        if unsafe { WaitNamedPipeW(pipe_name.as_ptr(), CONNECT_TIMEOUT_MS) } == 0 {
            return Err(classify_unavailable("collector_service_pipe_wait_failed"));
        }
        let pipe = OwnedHandle::new(unsafe {
            CreateFileW(
                pipe_name.as_ptr(),
                FILE_READ_DATA | FILE_WRITE_DATA,
                0,
                std::ptr::null(),
                OPEN_EXISTING,
                FILE_ATTRIBUTE_NORMAL,
                std::ptr::null_mut(),
            )
        })
        .ok_or_else(|| classify_unavailable("collector_service_pipe_open_failed"))?;
        let peer = verify_service_peer(pipe.raw())?;
        Ok(Self {
            pipe,
            peer,
            decoder: FrameDecoder::default(),
        })
    }

    fn write_request(&self, request: &ClientRequestV1) -> Result<(), ClientFailure> {
        let bytes = encode_json_frame(request).map_err(|failure| {
            ClientFailure::new(ClientFailureKind::Incompatible, failure.detail)
        })?;
        let deadline = Instant::now() + OPERATION_TIMEOUT;
        let mut offset = 0_usize;
        while offset < bytes.len() {
            if Instant::now() >= deadline {
                return Err(ClientFailure::new(
                    ClientFailureKind::Failed,
                    "collector_service_client_write_timeout",
                ));
            }
            let chunk = (bytes.len() - offset).min(PIPE_BUFFER_BYTES);
            let mut written = 0_u32;
            let ok = unsafe {
                WriteFile(
                    self.pipe.raw(),
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
                    ERROR_PIPE_BUSY | ERROR_NO_DATA => {}
                    ERROR_BROKEN_PIPE => {
                        return Err(ClientFailure::new(
                            ClientFailureKind::Failed,
                            "collector_service_pipe_disconnected",
                        ))
                    }
                    error => {
                        return Err(ClientFailure::new(
                            ClientFailureKind::Failed,
                            format!("collector_service_client_write_failed:{error}"),
                        ))
                    }
                }
            }
            std::thread::sleep(PIPE_POLL_INTERVAL);
        }
        Ok(())
    }

    fn read_response(&mut self) -> Result<ServiceResponseV1, ClientFailure> {
        let deadline = Instant::now() + OPERATION_TIMEOUT;
        loop {
            if Instant::now() >= deadline {
                return Err(ClientFailure::new(
                    ClientFailureKind::Failed,
                    "collector_service_client_read_timeout",
                ));
            }
            let mut available = 0_u32;
            if unsafe {
                PeekNamedPipe(
                    self.pipe.raw(),
                    std::ptr::null_mut(),
                    0,
                    std::ptr::null_mut(),
                    &mut available,
                    std::ptr::null_mut(),
                )
            } == 0
            {
                return Err(pipe_failure("collector_service_client_peek_failed"));
            }
            if available == 0 {
                std::thread::sleep(PIPE_POLL_INTERVAL);
                continue;
            }
            let mut bytes = vec![0_u8; (available as usize).min(PIPE_BUFFER_BYTES)];
            let mut read = 0_u32;
            if unsafe {
                ReadFile(
                    self.pipe.raw(),
                    bytes.as_mut_ptr().cast(),
                    bytes.len() as u32,
                    &mut read,
                    std::ptr::null_mut(),
                )
            } == 0
            {
                return Err(pipe_failure("collector_service_client_read_failed"));
            }
            bytes.truncate(read as usize);
            let frames = self.decoder.push(&bytes).map_err(|failure| {
                ClientFailure::new(ClientFailureKind::Incompatible, failure.detail)
            })?;
            if frames.is_empty() {
                continue;
            }
            if frames.len() != 1 || self.decoder.buffered_bytes() != 0 {
                return Err(ClientFailure::new(
                    ClientFailureKind::Incompatible,
                    "collector_service_unsolicited_response_batch",
                ));
            }
            return decode_response(&frames[0]).map_err(|failure| {
                ClientFailure::new(ClientFailureKind::Incompatible, failure.detail)
            });
        }
    }
}

impl ClientTransport for WindowsServiceTransport {
    fn verified_peer(&self) -> &VerifiedServicePeer {
        &self.peer
    }

    fn exchange(&mut self, request: &ClientRequestV1) -> Result<ServiceResponseV1, ClientFailure> {
        self.write_request(request)?;
        self.read_response()
    }
}

fn verify_service_peer(pipe: HANDLE) -> Result<VerifiedServicePeer, ClientFailure> {
    let process_id = pipe_server_process_id(pipe)?;
    let first_probe = service_probe()?;
    first_probe.verify_running_process(process_id)?;

    let peer_failure = |detail| classify_peer_verification_failure(pipe, process_id, detail);

    let process =
        OwnedHandle::new(unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, process_id) })
            .ok_or_else(|| {
                peer_failure(last_error_message(
                    "collector_service_server_process_open_failed",
                ))
            })?;
    let process_started_at = process_started_at(process.raw()).map_err(peer_failure)?;
    let executable_path = process_image_path(process.raw()).map_err(peer_failure)?;
    let canonical_path = PathBuf::from(&executable_path)
        .canonicalize()
        .map_err(|error| {
            peer_failure(format!(
                "collector_service_server_path_canonicalize_failed:{error}"
            ))
        })?;
    verify_service_path(&canonical_path).map_err(|failure| peer_failure(failure.detail))?;
    let path_wide = wide(&canonical_path.to_string_lossy());
    let executable = OwnedHandle::new(unsafe {
        CreateFileW(
            path_wide.as_ptr(),
            FILE_READ_ATTRIBUTES,
            FILE_SHARE_READ,
            std::ptr::null(),
            OPEN_EXISTING,
            FILE_ATTRIBUTE_NORMAL,
            std::ptr::null_mut(),
        )
    })
    .ok_or_else(|| {
        peer_failure(last_error_message(
            "collector_service_server_image_open_failed",
        ))
    })?;
    let executable_file_identity = file_identity(executable.raw()).map_err(peer_failure)?;
    let executable_release = executable_release(&path_wide).map_err(peer_failure)?;

    let mut token = std::ptr::null_mut();
    if unsafe { OpenProcessToken(process.raw(), TOKEN_QUERY, &mut token) } == 0 {
        return Err(peer_failure(last_error_message(
            "collector_service_server_token_open_failed",
        )));
    }
    let token = OwnedHandle::new(token)
        .ok_or_else(|| peer_failure("collector_service_server_token_invalid"))?;
    let token = token_evidence(token.raw()).map_err(peer_failure)?;
    let local_system_identity =
        local_system_principal_identity().map_err(|failure| peer_failure(failure.detail))?;
    if token.principal_identity != local_system_identity || token.elevated == false {
        return Err(peer_failure(
            "collector_service_server_principal_not_local_system",
        ));
    }

    match observe_peer_continuity(pipe, process_id)? {
        PeerContinuity::Stable => {}
        continuity => {
            return Err(classify_peer_continuity(
                "collector_service_server_process_identity_changed",
                continuity,
            ))
        }
    }

    VerifiedServicePeer::from_transport_verification(
        process_id,
        process_started_at,
        token.principal_identity,
        executable_file_identity,
        ReleaseIdentityV1 {
            app_version: executable_release.product_version,
            source_commit_sha: None,
        },
    )
    .map_err(|failure| unauthorized(failure.detail))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PeerContinuity {
    Stable,
    Stopped,
    Restarted,
}

fn classify_peer_verification_failure(
    pipe: HANDLE,
    expected_process_id: u32,
    detail: impl Into<String>,
) -> ClientFailure {
    let detail = detail.into();
    match observe_peer_continuity(pipe, expected_process_id) {
        Ok(continuity) => classify_peer_continuity(detail, continuity),
        Err(failure) => failure,
    }
}

fn classify_peer_continuity(
    detail: impl Into<String>,
    continuity: PeerContinuity,
) -> ClientFailure {
    match continuity {
        PeerContinuity::Stable => unauthorized(detail),
        PeerContinuity::Stopped => ClientFailure::new(
            ClientFailureKind::Stopped,
            "collector_service_stopped_during_peer_verification",
        ),
        PeerContinuity::Restarted => ClientFailure::new(
            ClientFailureKind::Restarted,
            "collector_service_restarted_during_peer_verification",
        ),
    }
}

fn observe_peer_continuity(
    pipe: HANDLE,
    expected_process_id: u32,
) -> Result<PeerContinuity, ClientFailure> {
    let probe = service_probe()?;
    if !probe.running {
        return Ok(PeerContinuity::Stopped);
    }
    if !probe.own_process || probe.process_id == 0 || probe.process_id != expected_process_id {
        return Ok(PeerContinuity::Restarted);
    }

    let mut pipe_process_id = 0_u32;
    if unsafe { GetNamedPipeServerProcessId(pipe, &mut pipe_process_id) } == 0 {
        return if unsafe { GetLastError() } == ERROR_ACCESS_DENIED {
            Err(unauthorized(
                "collector_service_server_pipe_reprobe_unauthorized",
            ))
        } else {
            Ok(PeerContinuity::Restarted)
        };
    }
    if pipe_process_id != expected_process_id {
        return Ok(PeerContinuity::Restarted);
    }
    Ok(PeerContinuity::Stable)
}

fn verify_service_path(path: &std::path::Path) -> Result<(), ClientFailure> {
    let current = std::env::current_exe()
        .map_err(|error| unauthorized(format!("collector_service_desktop_path_failed:{error}")))?
        .canonicalize()
        .map_err(|error| {
            unauthorized(format!(
                "collector_service_desktop_path_canonicalize_failed:{error}"
            ))
        })?;
    let desktop_name = current
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default();
    if !desktop_name.eq_ignore_ascii_case(DESKTOP_EXECUTABLE_NAME) {
        return Err(unauthorized(
            "collector_service_desktop_executable_name_invalid",
        ));
    }
    let same_directory = current.parent().and_then(|directory| {
        path.parent().map(|parent| {
            parent
                .to_string_lossy()
                .eq_ignore_ascii_case(&directory.to_string_lossy())
        })
    });
    let service_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default();
    if same_directory != Some(true) || !service_name.eq_ignore_ascii_case(SERVICE_EXECUTABLE_NAME) {
        return Err(unauthorized(
            "collector_service_server_executable_unauthorized",
        ));
    }
    Ok(())
}

fn pipe_server_process_id(pipe: HANDLE) -> Result<u32, ClientFailure> {
    let mut process_id = 0_u32;
    if unsafe { GetNamedPipeServerProcessId(pipe, &mut process_id) } == 0 || process_id == 0 {
        return Err(unauthorized(last_error_message(
            "collector_service_server_pipe_identity_failed",
        )));
    }
    Ok(process_id)
}

fn local_system_principal_identity() -> Result<[u8; 32], ClientFailure> {
    let mut sid = vec![0_u8; 68];
    let mut size = sid.len() as u32;
    if unsafe {
        CreateWellKnownSid(
            WinLocalSystemSid,
            std::ptr::null_mut(),
            sid.as_mut_ptr().cast(),
            &mut size,
        )
    } == 0
        || size == 0
    {
        return Err(unauthorized(last_error_message(
            "collector_service_local_system_sid_failed",
        )));
    }
    sid.truncate(size as usize);
    Ok(Sha256::digest(&sid).into())
}

#[derive(Clone, Copy)]
struct ServiceProbe {
    running: bool,
    own_process: bool,
    process_id: u32,
}

impl ServiceProbe {
    fn verify_running_process(self, process_id: u32) -> Result<(), ClientFailure> {
        if !self.running {
            return Err(ClientFailure::new(
                ClientFailureKind::Stopped,
                "collector_service_stopped",
            ));
        }
        if !self.own_process || self.process_id == 0 {
            return Err(unauthorized("collector_service_scm_process_mismatch"));
        }
        if self.process_id != process_id {
            return Err(ClientFailure::new(
                ClientFailureKind::Restarted,
                "collector_service_restarted_during_peer_verification",
            ));
        }
        Ok(())
    }
}

fn service_probe() -> Result<ServiceProbe, ClientFailure> {
    let manager = ServiceHandle::new(unsafe {
        OpenSCManagerW(std::ptr::null(), std::ptr::null(), SC_MANAGER_CONNECT)
    })
    .ok_or_else(|| classify_scm_error("collector_service_scm_open_failed"))?;
    let name = wide(COLLECTOR_SERVICE_NAME);
    let service = ServiceHandle::new(unsafe {
        OpenServiceW(manager.raw(), name.as_ptr(), SERVICE_QUERY_STATUS)
    })
    .ok_or_else(|| classify_scm_error("collector_service_open_failed"))?;
    let mut status = SERVICE_STATUS_PROCESS::default();
    let mut required = 0_u32;
    if unsafe {
        QueryServiceStatusEx(
            service.raw(),
            SC_STATUS_PROCESS_INFO,
            (&mut status as *mut SERVICE_STATUS_PROCESS).cast(),
            std::mem::size_of::<SERVICE_STATUS_PROCESS>() as u32,
            &mut required,
        )
    } == 0
    {
        return Err(classify_scm_error("collector_service_status_query_failed"));
    }
    Ok(ServiceProbe {
        running: status.dwCurrentState == SERVICE_RUNNING,
        own_process: status.dwServiceType & SERVICE_WIN32_OWN_PROCESS != 0,
        process_id: status.dwProcessId,
    })
}

struct ServiceHandle(windows_sys::Win32::System::Services::SC_HANDLE);

impl ServiceHandle {
    fn new(handle: windows_sys::Win32::System::Services::SC_HANDLE) -> Option<Self> {
        (!handle.is_null()).then_some(Self(handle))
    }

    fn raw(&self) -> windows_sys::Win32::System::Services::SC_HANDLE {
        self.0
    }
}

impl Drop for ServiceHandle {
    fn drop(&mut self) {
        unsafe {
            CloseServiceHandle(self.0);
        }
    }
}

fn classify_unavailable(context: &str) -> ClientFailure {
    let error = unsafe { GetLastError() };
    if error == ERROR_ACCESS_DENIED {
        return ClientFailure::new(
            ClientFailureKind::Unauthorized,
            format!("{context}:{error}"),
        );
    }
    match service_probe() {
        Err(failure) => failure,
        Ok(probe) if !probe.running => {
            ClientFailure::new(ClientFailureKind::Stopped, "collector_service_stopped")
        }
        Ok(_) => ClientFailure::new(ClientFailureKind::Failed, format!("{context}:{error}")),
    }
}

fn classify_scm_error(context: &str) -> ClientFailure {
    let error = unsafe { GetLastError() };
    ClientFailure::new(
        match error {
            ERROR_SERVICE_DOES_NOT_EXIST => ClientFailureKind::NotInstalled,
            ERROR_ACCESS_DENIED => ClientFailureKind::Unauthorized,
            _ => ClientFailureKind::Failed,
        },
        format!("{context}:{error}"),
    )
}

fn pipe_failure(context: &str) -> ClientFailure {
    let error = unsafe { GetLastError() };
    ClientFailure::new(
        ClientFailureKind::Failed,
        match error {
            ERROR_BROKEN_PIPE | ERROR_NO_DATA => "collector_service_pipe_disconnected".to_string(),
            _ => format!("{context}:{error}"),
        },
    )
}

fn unauthorized(detail: impl Into<String>) -> ClientFailure {
    ClientFailure::new(ClientFailureKind::Unauthorized, detail)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn operation_deadline_is_bounded_below_host_idle_timeout() {
        assert!(OPERATION_TIMEOUT < Duration::from_secs(30));
        assert!(CONNECT_TIMEOUT_MS <= 1_000);
    }

    #[test]
    fn client_uses_fixed_versioned_local_pipe_and_exact_data_rights() {
        assert_eq!(PIPE_NAME, r"\\.\pipe\BatCaveCollector.v1");
        assert_eq!(FILE_READ_DATA | FILE_WRITE_DATA, 0x0000_0003);
    }

    #[test]
    fn peer_failure_classification_distinguishes_stable_stop_and_restart() {
        assert_eq!(
            classify_peer_continuity("token denied", PeerContinuity::Stable).kind,
            ClientFailureKind::Unauthorized
        );
        assert_eq!(
            classify_peer_continuity("process vanished", PeerContinuity::Stopped).kind,
            ClientFailureKind::Stopped
        );
        let restarted = classify_peer_continuity("process changed", PeerContinuity::Restarted);
        assert_eq!(restarted.kind, ClientFailureKind::Restarted);
        assert_eq!(
            super::super::client::status_from_failure(&restarted, false).state,
            crate::contracts::RuntimeCollectorServiceState::Recovering
        );
    }
}
