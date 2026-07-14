use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

use windows_sys::Win32::{
    Foundation::{GetLastError, ERROR_CALL_NOT_IMPLEMENTED, ERROR_SERVICE_SPECIFIC_ERROR},
    System::Services::{
        RegisterServiceCtrlHandlerExW, SetServiceStatus, StartServiceCtrlDispatcherW,
        SERVICE_ACCEPT_SHUTDOWN, SERVICE_ACCEPT_STOP, SERVICE_CONTROL_INTERROGATE,
        SERVICE_CONTROL_SHUTDOWN, SERVICE_CONTROL_STOP, SERVICE_RUNNING, SERVICE_START_PENDING,
        SERVICE_STATUS, SERVICE_STATUS_HANDLE, SERVICE_STOPPED, SERVICE_STOP_PENDING,
        SERVICE_TABLE_ENTRYW, SERVICE_WIN32_OWN_PROCESS,
    },
};

use crate::{
    collector_engine::{CollectorEngine, CollectorEngineConfig},
    telemetry::TelemetryCollector,
};

use super::{
    host::{new_instance_id, service_identity, CollectorSnapshotSource, SnapshotProvider},
    protocol::COLLECTOR_SERVICE_NAME,
    windows_transport::run_pipe_server,
};

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

    let result = run_service_body(status_handle, Arc::clone(&stop));
    stop.store(true, Ordering::Release);
    let (exit_code, service_specific) = if result.is_ok() {
        (0, 0)
    } else {
        (ERROR_SERVICE_SPECIFIC_ERROR, 1)
    };
    let _ = report_stopped(status_handle, exit_code, service_specific);
    if let Err(error) = result {
        eprintln!("{error}");
    }
}

fn run_service_body(
    status_handle: SERVICE_STATUS_HANDLE,
    stop: Arc<AtomicBool>,
) -> Result<(), String> {
    let instance_id = new_instance_id();
    let identity = service_identity(instance_id.clone());
    let engine = CollectorEngine::start(
        Box::new(TelemetryCollector::for_collector_service()),
        CollectorEngineConfig {
            interval: SAMPLE_INTERVAL,
            metric_window: METRIC_WINDOW,
            paused: false,
            automatic: true,
        },
        Arc::new(|| {}),
    )?;
    let collector = engine.handle();
    let initial = collector.refresh_now().map(|_| ());
    if initial.is_err() {
        let _ = engine.shutdown();
        return initial;
    }
    let snapshots: Arc<dyn SnapshotProvider> =
        Arc::new(CollectorSnapshotSource::new(collector, instance_id));
    report_status(status_handle, SERVICE_RUNNING, 0, 0)?;

    let transport = run_pipe_server(Arc::clone(&stop), identity, snapshots);
    stop.store(true, Ordering::Release);
    let _ = report_status(status_handle, SERVICE_STOP_PENDING, 2, 0);
    let shutdown = engine.shutdown();
    match (transport, shutdown) {
        (Ok(()), Ok(())) => Ok(()),
        (Err(error), Ok(())) | (Ok(()), Err(error)) => Err(error),
        (Err(transport), Err(shutdown)) => Err(format!("{transport};{shutdown}")),
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
