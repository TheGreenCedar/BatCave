use std::{
    fs,
    mem::{size_of, zeroed},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};

use sha2::{Digest, Sha256};
use windows_sys::Win32::{
    Foundation::{
        GetLastError, ERROR_CALL_NOT_IMPLEMENTED, ERROR_INVALID_PARAMETER,
        ERROR_SERVICE_SPECIFIC_ERROR, WAIT_OBJECT_0, WAIT_TIMEOUT,
    },
    System::Services::{
        RegisterServiceCtrlHandlerExW, SetServiceStatus, StartServiceCtrlDispatcherW,
        SERVICE_ACCEPT_SHUTDOWN, SERVICE_ACCEPT_STOP, SERVICE_CONTROL_INTERROGATE,
        SERVICE_CONTROL_SHUTDOWN, SERVICE_CONTROL_STOP, SERVICE_RUNNING, SERVICE_START_PENDING,
        SERVICE_STATUS, SERVICE_STATUS_HANDLE, SERVICE_STOPPED, SERVICE_STOP_PENDING,
        SERVICE_TABLE_ENTRYW, SERVICE_WIN32_OWN_PROCESS,
    },
    System::Threading::{
        GetCurrentProcess, GetCurrentProcessId, OpenProcess, WaitForSingleObject,
        PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SYNCHRONIZE,
    },
};
use windows_sys::{core::GUID, Wdk::System::SystemInformation::NtQuerySystemInformation};

use crate::{
    collector_engine::{CollectorEngine, CollectorEngineConfig},
    telemetry::TelemetryCollector,
};

use super::{
    etw_lease::{
        decide_etw_recovery, EtwControllerIdentityV1, EtwControllerObservation, EtwExpectedOwnerV1,
        EtwLeaseObservation, EtwLeasePhase, EtwLeaseSnapshot, EtwLeaseStore, EtwLeaseV1,
        EtwReclaimAttempt, EtwRecoveryDecision, EtwSessionObservation, ProtectedEtwLeaseRoot,
        WindowsEtwOwnerAcquire, WindowsEtwOwnerGuard, ETW_LEASE_SCHEMA_VERSION,
    },
    host::{new_instance_id, service_identity, CollectorSnapshotSource, SnapshotProvider},
    protocol::COLLECTOR_SERVICE_NAME,
    windows_provisioner,
    windows_transport::{process_started_at, run_pipe_server, OwnedHandle},
};
use crate::windows_network::{NetworkAttributionMonitor, NetworkAttributionSettlement};

const SAMPLE_INTERVAL: std::time::Duration = std::time::Duration::from_secs(1);
const METRIC_WINDOW: std::time::Duration = std::time::Duration::from_secs(60);
const SERVICE_WAIT_HINT_MS: u32 = 10_000;

pub(crate) fn run() -> i32 {
    let mut service_name = wide(COLLECTOR_SERVICE_NAME);
    let table = [
        SERVICE_TABLE_ENTRYW {
            lpServiceName: service_name.as_mut_ptr(),
            lpServiceProc: Some(service_main),
        },
        SERVICE_TABLE_ENTRYW {
            lpServiceName: std::ptr::null_mut(),
            lpServiceProc: None,
        },
    ];
    if unsafe { StartServiceCtrlDispatcherW(table.as_ptr()) } == 0 {
        eprintln!("collector_service_dispatcher_failed:{}", unsafe {
            GetLastError()
        });
        1
    } else {
        0
    }
}

unsafe extern "system" fn service_main(_argument_count: u32, _arguments: *mut *mut u16) {
    let stop = Arc::new(AtomicBool::new(false));
    let context = Box::new(ServiceControlContext {
        stop: Arc::clone(&stop),
    });
    let service_name = wide(COLLECTOR_SERVICE_NAME);
    let status_handle = unsafe {
        RegisterServiceCtrlHandlerExW(
            service_name.as_ptr(),
            Some(service_control_handler),
            (&*context as *const ServiceControlContext).cast(),
        )
    };
    if status_handle.is_null() {
        return;
    }
    if report_status(status_handle, SERVICE_START_PENDING, 1, 0).is_err() {
        return;
    }

    let _lifecycle_marker = match windows_provisioner::acquire_service_lifecycle_marker() {
        Ok(marker) => marker,
        Err(error) => {
            if let Err(record_error) = windows_provisioner::record_service_failure("lifecycle") {
                eprintln!("{record_error}");
            }
            eprintln!("{error}");
            let _ = report_stopped(status_handle, ERROR_SERVICE_SPECIFIC_ERROR, 1);
            return;
        }
    };

    let result = run_service_body(status_handle, Arc::clone(&stop));
    stop.store(true, Ordering::Release);
    let (exit_code, service_specific) = if result.is_ok() {
        (0, 0)
    } else {
        (ERROR_SERVICE_SPECIFIC_ERROR, 1)
    };
    if let Err(error) = &result {
        if let Err(record_error) =
            windows_provisioner::record_service_failure(sanitized_failure_reason(error))
        {
            eprintln!("{record_error}");
        }
        eprintln!("{error}");
    }
    let _ = report_stopped(status_handle, exit_code, service_specific);
}

fn sanitized_failure_reason(error: &str) -> &'static str {
    let codes = error
        .split(';')
        .map(|part| part.split(':').next().unwrap_or(part))
        .collect::<Vec<_>>();
    let has_code = |prefix: &str| codes.iter().any(|code| code.starts_with(prefix));
    let has_settlement_code = codes.iter().any(|code| {
        (code.starts_with("collector_service_etw_") || code.starts_with("network_attribution_"))
            && (code.contains("settlement")
                || code.contains("stopping")
                || code.contains("shutdown")
                || code.contains("session_not_absent")
                || code.contains("lease_remove")
                || code.contains("close_trace")
                || code.contains("consumer_join")
                || code.contains("stop_trace"))
    });

    if has_code("collector_service_etw_recovery_") {
        "etw_recovery"
    } else if has_settlement_code {
        "etw_settlement"
    } else if has_code("collector_service_etw_") || has_code("network_attribution_") {
        "etw_startup"
    } else if has_code("collector_service_lifecycle_")
        || has_code("collector_service_status_failed")
        || has_code("collector_service_failure_clear_failed")
    {
        "lifecycle"
    } else if has_code("collector_service_pipe_")
        || has_code("collector_service_client_")
        || has_code("collector_service_transport_")
        || has_code("collector_service_executable_resolve_failed")
        || has_code("collector_service_executable_canonicalize_failed")
        || has_code("collector_service_executable_parent_missing")
    {
        "transport"
    } else if has_code("collector_engine") || has_code("collector_") {
        "collector"
    } else {
        "startup"
    }
}

fn complete_service_readiness(
    clear_failure: impl FnOnce() -> Result<(), String>,
    report_running: impl FnOnce() -> Result<(), String>,
) -> Result<(), String> {
    clear_failure()?;
    report_running()
}

fn run_service_body(
    status_handle: SERVICE_STATUS_HANDLE,
    stop: Arc<AtomicBool>,
) -> Result<(), String> {
    // The root capability retains the verifier's no-delete handles for the
    // complete service/ETW lifetime.
    let protected_etw_root = windows_provisioner::open_protected_etw_lease_root()
        .map_err(|error| format!("collector_service_etw_root_open_failed:{error}"))?;
    let instance_id = new_instance_id();
    let identity = service_identity(instance_id.clone());
    let (mut etw_lifecycle, network_monitor) =
        ServiceEtwLifecycle::start(&protected_etw_root, &instance_id)
            .map_err(|error| format!("collector_service_etw_startup_failed:{error}"))?;
    let engine = match CollectorEngine::start(
        Box::new(TelemetryCollector::for_collector_service(network_monitor)),
        CollectorEngineConfig {
            interval: SAMPLE_INTERVAL,
            metric_window: METRIC_WINDOW,
            paused: false,
            automatic: true,
        },
        Arc::new(|| {}),
    ) {
        Ok(engine) => engine,
        Err(error) => {
            return Err(combine_results(
                Err(error),
                etw_lifecycle.finish_after_monitor_drop(),
            ))
        }
    };
    let collector = engine.handle();
    let initial = collector.refresh_now().map(|_| ());
    if initial.is_err() {
        return Err(combine_results(
            initial,
            etw_lifecycle.shutdown_engine(&engine),
        ));
    }
    let snapshots: Arc<dyn SnapshotProvider> =
        Arc::new(CollectorSnapshotSource::new(collector, instance_id));
    let transport = run_pipe_server(Arc::clone(&stop), identity, snapshots, || {
        complete_service_readiness(windows_provisioner::clear_service_failure, || {
            report_status(status_handle, SERVICE_RUNNING, 0, 0)
        })
    });
    stop.store(true, Ordering::Release);
    let _ = report_status(status_handle, SERVICE_STOP_PENDING, 2, 0);
    let shutdown = etw_lifecycle.shutdown_engine(&engine);
    match (transport, shutdown) {
        (Ok(()), Ok(())) => Ok(()),
        (Err(error), Ok(())) | (Ok(()), Err(error)) => Err(error),
        (Err(transport), Err(shutdown)) => Err(format!("{transport};{shutdown}")),
    }
}

struct ServiceEtwLifecycle {
    _owner: WindowsEtwOwnerGuard,
    store: EtwLeaseStore,
    snapshot: EtwLeaseSnapshot,
    lease: EtwLeaseV1,
    settlement: NetworkAttributionSettlement,
}

impl ServiceEtwLifecycle {
    fn start(
        root: &ProtectedEtwLeaseRoot,
        service_instance: &str,
    ) -> Result<(Self, NetworkAttributionMonitor), String> {
        let owner = match WindowsEtwOwnerGuard::try_acquire(root)? {
            WindowsEtwOwnerAcquire::Acquired(owner) => owner,
            WindowsEtwOwnerAcquire::Contended => {
                return Err("collector_service_etw_owner_contended".to_string())
            }
        };
        let store = EtwLeaseStore::new(root);
        let expected = expected_etw_owner(root)?;
        let snapshot = store
            .observe(owner.authority())
            .map_err(|error| format!("collector_service_etw_lease_observe_failed:{error:?}"))?;
        let snapshot = recover_etw_start(&expected, &store, &owner, snapshot)?;

        let mut lease = EtwLeaseV1 {
            schema_version: ETW_LEASE_SCHEMA_VERSION,
            phase: EtwLeasePhase::Intent,
            install_id: expected.install_id,
            service_generation: expected.service_generation,
            service_instance_id: digest16(service_instance.as_bytes()),
            boot_identity: expected.boot_identity,
            controller: current_controller_identity()?,
            session: expected.session,
        };
        store
            .replace(owner.authority(), &snapshot, &lease)
            .map_err(|error| format!("collector_service_etw_intent_write_failed:{error:?}"))?;
        let intent_snapshot = store
            .observe(owner.authority())
            .map_err(|error| format!("collector_service_etw_intent_observe_failed:{error:?}"))?;

        let (mut monitor, settlement) = match NetworkAttributionMonitor::new_for_collector_service()
        {
            Ok(started) => started,
            Err(error) => {
                let cleanup =
                    remove_exact_lease_if_session_absent(&store, &owner, &intent_snapshot);
                return Err(combine_results(Err(error), cleanup));
            }
        };

        lease.phase = EtwLeasePhase::Active;
        if let Err(error) = store.replace(owner.authority(), &intent_snapshot, &lease) {
            let shutdown = monitor.shutdown();
            let cleanup = remove_exact_lease_after_clean_settlement(
                &store,
                &owner,
                &intent_snapshot,
                &settlement,
            );
            return Err(combine_results(
                Err(format!(
                    "collector_service_etw_active_write_failed:{error:?}"
                )),
                combine_unit_results(shutdown, cleanup),
            ));
        }
        let active_snapshot = store
            .observe(owner.authority())
            .map_err(|error| format!("collector_service_etw_active_observe_failed:{error:?}"))?;

        Ok((
            Self {
                _owner: owner,
                store,
                snapshot: active_snapshot,
                lease,
                settlement,
            },
            monitor,
        ))
    }

    fn mark_stopping(&mut self) -> Result<(), String> {
        self.lease.phase = EtwLeasePhase::Stopping;
        self.store
            .replace(self._owner.authority(), &self.snapshot, &self.lease)
            .map_err(|error| format!("collector_service_etw_stopping_write_failed:{error:?}"))?;
        self.snapshot = self
            .store
            .observe(self._owner.authority())
            .map_err(|error| format!("collector_service_etw_stopping_observe_failed:{error:?}"))?;
        Ok(())
    }

    fn shutdown_engine(&mut self, engine: &CollectorEngine) -> Result<(), String> {
        let stopping = self.mark_stopping();
        let shutdown = engine.shutdown();
        if stopping.is_err() || shutdown.is_err() {
            return Err(combine_results(stopping, shutdown));
        }
        self.remove_after_absence()
    }

    fn finish_after_monitor_drop(&mut self) -> Result<(), String> {
        self.mark_stopping()?;
        self.remove_after_absence()
    }

    fn remove_after_absence(&self) -> Result<(), String> {
        self.settlement.require_clean()?;
        if NetworkAttributionMonitor::observe_session() != EtwSessionObservation::Absent {
            return Err("collector_service_etw_session_not_absent".to_string());
        }
        self.store
            .remove_after_proven_absence(self._owner.authority(), &self.snapshot)
            .map(|_| ())
            .map_err(|error| format!("collector_service_etw_lease_remove_failed:{error:?}"))
    }
}

fn remove_exact_lease_after_clean_settlement(
    store: &EtwLeaseStore,
    owner: &WindowsEtwOwnerGuard,
    snapshot: &EtwLeaseSnapshot,
    settlement: &NetworkAttributionSettlement,
) -> Result<(), String> {
    settlement.require_clean()?;
    remove_exact_lease_if_session_absent(store, owner, snapshot)
}

fn recover_etw_start(
    expected: &EtwExpectedOwnerV1,
    store: &EtwLeaseStore,
    owner: &WindowsEtwOwnerGuard,
    snapshot: EtwLeaseSnapshot,
) -> Result<EtwLeaseSnapshot, String> {
    let session = NetworkAttributionMonitor::observe_session();
    let observed_controller = match snapshot.observation() {
        EtwLeaseObservation::Trusted(lease) if lease.boot_identity == expected.boot_identity => {
            observe_controller(&lease.controller)
        }
        _ => EtwControllerObservation::QueryUnavailable,
    };
    // The caller holds both the service lifecycle marker and the exclusive ETW
    // owner guard. If the exact ETW session is also absent, no other process can
    // still write this lease, even when Windows will not reopen the dead
    // LocalSystem PID for an advisory creation-time query.
    let controller = controller_with_exclusive_owner(&session, observed_controller);
    if let EtwLeaseObservation::Trusted(lease) = snapshot.observation() {
        if stale_prior_generation_can_be_discarded(expected, lease, &session, &controller) {
            return remove_stale_lease(store, owner, &snapshot);
        }
    }
    let decision = decide_etw_recovery(
        expected,
        snapshot.observation(),
        &session,
        &controller,
        EtwReclaimAttempt::NotAttempted,
    );
    match decision {
        EtwRecoveryDecision::StartFresh {
            discard_stale_lease: false,
        } => Ok(snapshot),
        EtwRecoveryDecision::StartFresh {
            discard_stale_lease: true,
        } => remove_stale_lease(store, owner, &snapshot),
        EtwRecoveryDecision::ReclaimExact { .. } => {
            let EtwLeaseObservation::Trusted(mut lease) = snapshot.observation().clone() else {
                unreachable!("exact reclaim requires a trusted lease")
            };
            lease.phase = EtwLeasePhase::Stopping;
            store
                .replace(owner.authority(), &snapshot, &lease)
                .map_err(|error| {
                    format!("collector_service_etw_recovery_stopping_write_failed:{error:?}")
                })?;
            let stopping = store.observe(owner.authority()).map_err(|error| {
                format!("collector_service_etw_recovery_stopping_observe_failed:{error:?}")
            })?;

            if let Err(error) = NetworkAttributionMonitor::stop_session_if_exact(&lease.session) {
                let retained = decide_etw_recovery(
                    expected,
                    stopping.observation(),
                    &NetworkAttributionMonitor::observe_session(),
                    &observe_controller(&lease.controller),
                    EtwReclaimAttempt::StopFailed,
                );
                return Err(format!(
                    "collector_service_etw_recovery_stop_failed:{error}:{retained:?}"
                ));
            }

            let after_stop = decide_etw_recovery(
                expected,
                stopping.observation(),
                &NetworkAttributionMonitor::observe_session(),
                &observe_controller(&lease.controller),
                EtwReclaimAttempt::NotAttempted,
            );
            if after_stop
                != (EtwRecoveryDecision::StartFresh {
                    discard_stale_lease: true,
                })
            {
                return Err(format!(
                    "collector_service_etw_recovery_incomplete:{after_stop:?}"
                ));
            }
            remove_stale_lease(store, owner, &stopping)
        }
        EtwRecoveryDecision::Conflict(_) | EtwRecoveryDecision::Retain(_) => Err(format!(
            "collector_service_etw_recovery_blocked:{decision:?}"
        )),
    }
}

fn stale_prior_generation_can_be_discarded(
    expected: &EtwExpectedOwnerV1,
    lease: &EtwLeaseV1,
    session: &EtwSessionObservation,
    controller: &EtwControllerObservation,
) -> bool {
    if lease.service_generation == expected.service_generation
        || lease.phase != EtwLeasePhase::Stopping
    {
        return false;
    }
    let mut prior = expected.clone();
    prior.service_generation = lease.service_generation;
    decide_etw_recovery(
        &prior,
        &EtwLeaseObservation::Trusted(lease.clone()),
        session,
        controller,
        EtwReclaimAttempt::NotAttempted,
    ) == (EtwRecoveryDecision::StartFresh {
        discard_stale_lease: true,
    })
}

fn controller_with_exclusive_owner(
    session: &EtwSessionObservation,
    controller: EtwControllerObservation,
) -> EtwControllerObservation {
    if matches!(session, EtwSessionObservation::Absent)
        && matches!(controller, EtwControllerObservation::QueryUnavailable)
    {
        EtwControllerObservation::Absent
    } else {
        controller
    }
}

fn remove_stale_lease(
    store: &EtwLeaseStore,
    owner: &WindowsEtwOwnerGuard,
    snapshot: &EtwLeaseSnapshot,
) -> Result<EtwLeaseSnapshot, String> {
    if NetworkAttributionMonitor::observe_session() != EtwSessionObservation::Absent {
        return Err("collector_service_etw_recovery_session_not_absent".to_string());
    }
    store
        .remove_after_proven_absence(owner.authority(), snapshot)
        .map_err(|error| format!("collector_service_etw_recovery_lease_remove_failed:{error:?}"))?;
    store
        .observe(owner.authority())
        .map_err(|error| format!("collector_service_etw_recovery_lease_observe_failed:{error:?}"))
}

fn observe_controller(expected: &EtwControllerIdentityV1) -> EtwControllerObservation {
    let process = match OwnedHandle::new(unsafe {
        OpenProcess(
            PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SYNCHRONIZE,
            0,
            expected.process_id,
        )
    }) {
        Some(process) => process,
        None if unsafe { GetLastError() } == ERROR_INVALID_PARAMETER => {
            return EtwControllerObservation::Absent
        }
        None => return EtwControllerObservation::QueryUnavailable,
    };
    match unsafe { WaitForSingleObject(process.raw(), 0) } {
        WAIT_OBJECT_0 => EtwControllerObservation::Absent,
        WAIT_TIMEOUT => match process_started_at(process.raw()) {
            Ok(process_started_at) => EtwControllerObservation::Present(EtwControllerIdentityV1 {
                process_id: expected.process_id,
                process_started_at,
            }),
            Err(_) => EtwControllerObservation::QueryUnavailable,
        },
        _ => EtwControllerObservation::QueryUnavailable,
    }
}

fn remove_exact_lease_if_session_absent(
    store: &EtwLeaseStore,
    owner: &WindowsEtwOwnerGuard,
    snapshot: &EtwLeaseSnapshot,
) -> Result<(), String> {
    if NetworkAttributionMonitor::observe_session() != EtwSessionObservation::Absent {
        return Err("collector_service_etw_failed_start_retained".to_string());
    }
    store
        .remove_after_proven_absence(owner.authority(), snapshot)
        .map(|_| ())
        .map_err(|error| format!("collector_service_etw_failed_start_cleanup_failed:{error:?}"))
}

fn expected_etw_owner(root: &ProtectedEtwLeaseRoot) -> Result<EtwExpectedOwnerV1, String> {
    Ok(EtwExpectedOwnerV1 {
        install_id: root.install_id(),
        service_generation: service_generation()?,
        boot_identity: boot_identity()?,
        session: NetworkAttributionMonitor::session_identity(),
    })
}

fn service_generation() -> Result<[u8; 16], String> {
    let executable = std::env::current_exe()
        .map_err(|error| format!("collector_service_executable_path_failed:{error}"))?;
    let bytes = fs::read(executable)
        .map_err(|error| format!("collector_service_executable_read_failed:{error}"))?;
    Ok(digest16(&bytes))
}

fn current_controller_identity() -> Result<EtwControllerIdentityV1, String> {
    let process_started_at = process_started_at(unsafe { GetCurrentProcess() })?;
    Ok(EtwControllerIdentityV1 {
        process_id: unsafe { GetCurrentProcessId() },
        process_started_at,
    })
}

#[repr(C)]
struct SystemBootEnvironmentInformation {
    boot_identifier: GUID,
    firmware_type: u32,
    boot_flags: u64,
}

fn boot_identity() -> Result<[u8; 16], String> {
    let mut information: SystemBootEnvironmentInformation = unsafe { zeroed() };
    let status = unsafe {
        NtQuerySystemInformation(
            90,
            (&mut information as *mut SystemBootEnvironmentInformation).cast(),
            size_of::<SystemBootEnvironmentInformation>() as u32,
            std::ptr::null_mut(),
        )
    };
    if status < 0 {
        return Err(format!("collector_service_boot_identity_failed:{status}"));
    }
    let guid = information.boot_identifier;
    let mut identity = [0_u8; 16];
    identity[..4].copy_from_slice(&guid.data1.to_le_bytes());
    identity[4..6].copy_from_slice(&guid.data2.to_le_bytes());
    identity[6..8].copy_from_slice(&guid.data3.to_le_bytes());
    identity[8..].copy_from_slice(&guid.data4);
    if identity == [0; 16] {
        Err("collector_service_boot_identity_zero".to_string())
    } else {
        Ok(identity)
    }
}

fn digest16(bytes: &[u8]) -> [u8; 16] {
    let digest = Sha256::digest(bytes);
    let mut value = [0_u8; 16];
    value.copy_from_slice(&digest[..16]);
    value
}

fn combine_results(first: Result<(), String>, second: Result<(), String>) -> String {
    match (first, second) {
        (Err(first), Err(second)) => format!("{first};{second}"),
        (Err(error), Ok(())) | (Ok(()), Err(error)) => error,
        (Ok(()), Ok(())) => String::new(),
    }
}

fn combine_unit_results(
    first: Result<(), String>,
    second: Result<(), String>,
) -> Result<(), String> {
    match (first, second) {
        (Ok(()), Ok(())) => Ok(()),
        (Err(first), Err(second)) => Err(format!("{first};{second}")),
        (Err(error), Ok(())) | (Ok(()), Err(error)) => Err(error),
    }
}

struct ServiceControlContext {
    stop: Arc<AtomicBool>,
}

unsafe extern "system" fn service_control_handler(
    control: u32,
    _event_type: u32,
    _event_data: *mut core::ffi::c_void,
    context: *mut core::ffi::c_void,
) -> u32 {
    if context.is_null() {
        return ERROR_CALL_NOT_IMPLEMENTED;
    }
    let context = unsafe { &*(context.cast::<ServiceControlContext>()) };
    apply_service_control(control, &context.stop)
}

fn apply_service_control(control: u32, stop: &AtomicBool) -> u32 {
    match control {
        SERVICE_CONTROL_STOP | SERVICE_CONTROL_SHUTDOWN => {
            stop.store(true, Ordering::Release);
            0
        }
        SERVICE_CONTROL_INTERROGATE => 0,
        _ => ERROR_CALL_NOT_IMPLEMENTED,
    }
}

fn report_status(
    handle: SERVICE_STATUS_HANDLE,
    state: u32,
    checkpoint: u32,
    wait_hint: u32,
) -> Result<(), String> {
    let status = status(state, checkpoint, wait_hint, 0, 0);
    if unsafe { SetServiceStatus(handle, &status) } == 0 {
        Err(format!("collector_service_status_failed:{}", unsafe {
            GetLastError()
        }))
    } else {
        Ok(())
    }
}

fn report_stopped(
    handle: SERVICE_STATUS_HANDLE,
    win32_exit_code: u32,
    service_specific_exit_code: u32,
) -> Result<(), String> {
    let status = status(
        SERVICE_STOPPED,
        0,
        0,
        win32_exit_code,
        service_specific_exit_code,
    );
    if unsafe { SetServiceStatus(handle, &status) } == 0 {
        Err(format!("collector_service_status_failed:{}", unsafe {
            GetLastError()
        }))
    } else {
        Ok(())
    }
}

fn status(
    state: u32,
    checkpoint: u32,
    wait_hint: u32,
    win32_exit_code: u32,
    service_specific_exit_code: u32,
) -> SERVICE_STATUS {
    let controls = if state == SERVICE_RUNNING {
        SERVICE_ACCEPT_STOP | SERVICE_ACCEPT_SHUTDOWN
    } else {
        0
    };
    SERVICE_STATUS {
        dwServiceType: SERVICE_WIN32_OWN_PROCESS,
        dwCurrentState: state,
        dwControlsAccepted: controls,
        dwWin32ExitCode: win32_exit_code,
        dwServiceSpecificExitCode: service_specific_exit_code,
        dwCheckPoint: checkpoint,
        dwWaitHint: if wait_hint == 0
            && matches!(state, SERVICE_START_PENDING | SERVICE_STOP_PENDING)
        {
            SERVICE_WAIT_HINT_MS
        } else {
            wait_hint
        },
    }
}

fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collector_service::etw_lease::{EtwLeaseConflict, EtwSessionIdentityV1};

    fn expected_owner() -> EtwExpectedOwnerV1 {
        EtwExpectedOwnerV1 {
            install_id: [1; 16],
            service_generation: [2; 16],
            boot_identity: [3; 16],
            session: EtwSessionIdentityV1 {
                name: "BatCave Collector Process Network v1".to_string(),
                provider_id: [4; 16],
                session_flags: 1,
                configuration_digest: [5; 32],
            },
        }
    }

    #[test]
    fn service_failures_expose_only_stable_failure_categories() {
        assert_eq!(
            sanitized_failure_reason("collector_service_pipe_connect_failed:232"),
            "transport"
        );
        assert_eq!(
            sanitized_failure_reason("collector_service_client_spawn_failed:resource exhausted"),
            "transport"
        );
        assert_eq!(
            sanitized_failure_reason(
                "collector_service_executable_canonicalize_failed:access denied"
            ),
            "transport"
        );
        assert_eq!(
            sanitized_failure_reason(
                "collector_service_etw_startup_failed:collector_service_executable_path_failed:access denied"
            ),
            "etw_startup"
        );
        assert_eq!(
            sanitized_failure_reason(
                "collector_service_etw_startup_failed:collector_service_executable_read_failed:access denied"
            ),
            "etw_startup"
        );
        assert_eq!(
            sanitized_failure_reason(
                "collector_service_transport_policy_failed:Unauthorized:collector_service_directory_invalid"
            ),
            "transport"
        );
        assert_eq!(
            sanitized_failure_reason("collector_service_lifecycle_file_missing"),
            "lifecycle"
        );
        assert_eq!(
            sanitized_failure_reason("collector_service_status_failed:5"),
            "lifecycle"
        );
        assert_eq!(
            sanitized_failure_reason("collector_service_failure_clear_failed:5"),
            "lifecycle"
        );
        assert_eq!(
            sanitized_failure_reason("collector_service_etw_recovery_incomplete:Retain"),
            "etw_recovery"
        );
        assert_eq!(
            sanitized_failure_reason("network_attribution_settlement_unproven"),
            "etw_settlement"
        );
        assert_eq!(
            sanitized_failure_reason("collector_service_etw_session_not_absent"),
            "etw_settlement"
        );
        assert_eq!(
            sanitized_failure_reason("network_attribution_close_trace_failed:7007"),
            "etw_settlement"
        );
        assert_eq!(
            sanitized_failure_reason("network_attribution_consumer_join_timeout"),
            "etw_settlement"
        );
        assert_eq!(
            sanitized_failure_reason("network_attribution_stop_trace_failed:5"),
            "etw_settlement"
        );
        assert_eq!(
            sanitized_failure_reason(
                "collector_service_pipe_accept_failed:5;network_attribution_consumer_join_timeout"
            ),
            "etw_settlement"
        );
        assert_eq!(
            sanitized_failure_reason("collector_service_etw_intent_write_failed:AccessDenied"),
            "etw_startup"
        );
        assert_eq!(
            sanitized_failure_reason(
                "collector_service_pipe_accept_failed:5;private lifecycle transport text"
            ),
            "transport"
        );
        assert_eq!(sanitized_failure_reason("private path text"), "startup");
    }

    #[test]
    fn service_readiness_requires_the_failure_marker_to_clear() {
        let mut running_reported = false;
        assert_eq!(
            complete_service_readiness(
                || Err("collector_service_failure_clear_failed:5".to_string()),
                || {
                    running_reported = true;
                    Ok(())
                },
            ),
            Err("collector_service_failure_clear_failed:5".to_string())
        );
        assert!(!running_reported);

        assert_eq!(
            complete_service_readiness(
                || Ok(()),
                || {
                    running_reported = true;
                    Ok(())
                },
            ),
            Ok(())
        );
        assert!(running_reported);
    }

    #[test]
    fn service_etw_recovery_rejects_unowned_or_untrusted_state() {
        let expected = expected_owner();
        assert_eq!(
            decide_etw_recovery(
                &expected,
                &EtwLeaseObservation::Absent,
                &EtwSessionObservation::Present(expected.session.clone()),
                &EtwControllerObservation::Absent,
                EtwReclaimAttempt::NotAttempted,
            ),
            EtwRecoveryDecision::Conflict(EtwLeaseConflict::SessionWithoutTrustedLease)
        );
        assert_eq!(
            decide_etw_recovery(
                &expected,
                &EtwLeaseObservation::Corrupt,
                &EtwSessionObservation::Absent,
                &EtwControllerObservation::Absent,
                EtwReclaimAttempt::NotAttempted,
            ),
            EtwRecoveryDecision::Conflict(EtwLeaseConflict::CorruptLease)
        );
    }

    #[test]
    fn exclusive_owner_and_absent_session_corroborate_an_unqueryable_controller() {
        assert_eq!(
            controller_with_exclusive_owner(
                &EtwSessionObservation::Absent,
                EtwControllerObservation::QueryUnavailable,
            ),
            EtwControllerObservation::Absent
        );
        assert_eq!(
            controller_with_exclusive_owner(
                &EtwSessionObservation::QueryUnavailable,
                EtwControllerObservation::QueryUnavailable,
            ),
            EtwControllerObservation::QueryUnavailable
        );
        let live = EtwControllerIdentityV1 {
            process_id: 7,
            process_started_at: 11,
        };
        assert_eq!(
            controller_with_exclusive_owner(
                &EtwSessionObservation::Absent,
                EtwControllerObservation::Present(live.clone()),
            ),
            EtwControllerObservation::Present(live)
        );
    }

    #[test]
    fn prior_generation_is_discarded_only_as_proven_stale_metadata() {
        let expected = expected_owner();
        let lease = EtwLeaseV1 {
            schema_version: ETW_LEASE_SCHEMA_VERSION,
            phase: EtwLeasePhase::Stopping,
            install_id: expected.install_id,
            service_generation: [9; 16],
            service_instance_id: [8; 16],
            boot_identity: expected.boot_identity,
            controller: EtwControllerIdentityV1 {
                process_id: 7,
                process_started_at: 11,
            },
            session: expected.session.clone(),
        };
        assert!(stale_prior_generation_can_be_discarded(
            &expected,
            &lease,
            &EtwSessionObservation::Absent,
            &EtwControllerObservation::Absent,
        ));
        assert!(!stale_prior_generation_can_be_discarded(
            &expected,
            &lease,
            &EtwSessionObservation::Present(expected.session.clone()),
            &EtwControllerObservation::Absent,
        ));
        for phase in [EtwLeasePhase::Intent, EtwLeasePhase::Active] {
            let mut unsafe_phase = lease.clone();
            unsafe_phase.phase = phase;
            assert!(!stale_prior_generation_can_be_discarded(
                &expected,
                &unsafe_phase,
                &EtwSessionObservation::Absent,
                &EtwControllerObservation::Absent,
            ));
        }
    }

    #[test]
    fn current_controller_is_observed_by_pid_and_creation_time() {
        let current = current_controller_identity().expect("current controller identity");
        assert_eq!(
            observe_controller(&current),
            EtwControllerObservation::Present(current)
        );
    }

    #[test]
    fn scm_status_accepts_controls_only_while_running() {
        let pending = status(SERVICE_START_PENDING, 1, 0, 0, 0);
        assert_eq!(pending.dwControlsAccepted, 0);
        assert_eq!(pending.dwWaitHint, SERVICE_WAIT_HINT_MS);
        let running = status(SERVICE_RUNNING, 0, 0, 0, 0);
        assert_eq!(
            running.dwControlsAccepted,
            SERVICE_ACCEPT_STOP | SERVICE_ACCEPT_SHUTDOWN
        );
        let stopping = status(SERVICE_STOP_PENDING, 2, 0, 0, 0);
        assert_eq!(stopping.dwControlsAccepted, 0);
    }

    #[test]
    fn only_stop_shutdown_and_interrogate_are_accepted() {
        let stop = AtomicBool::new(false);
        assert_eq!(apply_service_control(SERVICE_CONTROL_INTERROGATE, &stop), 0);
        assert!(!stop.load(Ordering::Acquire));
        assert_eq!(apply_service_control(SERVICE_CONTROL_STOP, &stop), 0);
        assert!(stop.load(Ordering::Acquire));
        stop.store(false, Ordering::Release);
        assert_eq!(
            apply_service_control(999, &stop),
            ERROR_CALL_NOT_IMPLEMENTED
        );
        assert!(!stop.load(Ordering::Acquire));
    }
}
