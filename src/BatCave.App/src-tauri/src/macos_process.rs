use std::{
    ffi::{c_int, c_void},
    io,
    mem::{size_of, MaybeUninit},
};

use crate::contracts::{
    AccessState, MetricQuality, MetricQualityInfo, MetricSource, ProcessMetricQuality,
    ProcessSample,
};

const PROC_PIDLISTFDS: c_int = 1;
const PROC_PIDTASKINFO: c_int = 4;
const RUSAGE_INFO_V2: c_int = 2;
const PHYSICAL_FOOTPRINT_UNAVAILABLE: &str =
    "Resident memory uses the sysinfo fallback; physical footprint is unavailable.";

#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
struct ProcTaskInfo {
    virtual_size: u64,
    resident_size: u64,
    total_user: u64,
    total_system: u64,
    threads_user: u64,
    threads_system: u64,
    policy: i32,
    faults: i32,
    pageins: i32,
    cow_faults: i32,
    messages_sent: i32,
    messages_received: i32,
    syscalls_mach: i32,
    syscalls_unix: i32,
    context_switches: i32,
    thread_count: i32,
    running_thread_count: i32,
    priority: i32,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
struct RusageInfoV2 {
    uuid: [u8; 16],
    user_time: u64,
    system_time: u64,
    package_idle_wakeups: u64,
    interrupt_wakeups: u64,
    pageins: u64,
    wired_size: u64,
    resident_size: u64,
    physical_footprint: u64,
    process_start_abstime: u64,
    process_exit_abstime: u64,
    child_user_time: u64,
    child_system_time: u64,
    child_package_idle_wakeups: u64,
    child_interrupt_wakeups: u64,
    child_pageins: u64,
    child_elapsed_abstime: u64,
    disk_bytes_read: u64,
    disk_bytes_written: u64,
}

#[link(name = "proc")]
unsafe extern "C" {
    fn proc_pidinfo(
        pid: c_int,
        flavor: c_int,
        arg: u64,
        buffer: *mut c_void,
        buffer_size: c_int,
    ) -> c_int;
    fn proc_pid_rusage(pid: c_int, flavor: c_int, buffer: *mut c_void) -> c_int;
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct MacosProcessCollection {
    pub denied_count: usize,
    pub partial_count: usize,
}

#[derive(Debug, Default)]
pub struct MacosProcessCollector;

impl MacosProcessCollector {
    pub fn new() -> Self {
        Self
    }

    pub fn enrich(&mut self, processes: &mut [ProcessSample]) -> MacosProcessCollection {
        let mut collection = MacosProcessCollection::default();
        for process in processes {
            let Ok(pid) = process.pid.parse::<c_int>() else {
                process.access_state = AccessState::Partial;
                collection.partial_count += 1;
                continue;
            };

            let outcome = enrich_process(process, pid);
            match outcome {
                AccessState::Denied => collection.denied_count += 1,
                AccessState::Partial => collection.partial_count += 1,
                AccessState::Full => {}
            }
        }
        collection
    }
}

fn enrich_process(process: &mut ProcessSample, pid: c_int) -> AccessState {
    let mut successful_probes = 0_u8;
    let mut denied_probes = 0_u8;
    let quality = process
        .quality
        .get_or_insert_with(ProcessMetricQuality::default);

    match process_rusage(pid) {
        Ok(rusage) => {
            successful_probes += 1;
            if rusage.resident_size > 0 {
                process.memory_bytes = rusage.resident_size;
            }
            if rusage.physical_footprint > 0 {
                process.private_bytes = rusage.physical_footprint;
            }
            process.disk_read_total_bytes = rusage.disk_bytes_read;
            process.disk_write_total_bytes = rusage.disk_bytes_written;
            quality.memory = Some(MetricQualityInfo::new(
                MetricQuality::Native,
                MetricSource::DirectApi,
            ));
            quality.disk = Some(MetricQualityInfo::new(
                MetricQuality::Native,
                MetricSource::DirectApi,
            ));
        }
        Err(error) => {
            denied_probes += usize::from(is_access_denied(&error)) as u8;
            process.private_bytes = 0;
            quality.memory = Some(
                MetricQualityInfo::new(MetricQuality::Partial, MetricSource::Sysinfo)
                    .with_message(PHYSICAL_FOOTPRINT_UNAVAILABLE),
            );
            quality.disk = Some(
                MetricQualityInfo::new(MetricQuality::Estimated, MetricSource::Sysinfo)
                    .with_message(
                        "Native process disk totals are unavailable; using the sysinfo fallback.",
                    ),
            );
        }
    }

    match task_info(pid) {
        Ok(task) => {
            successful_probes += 1;
            process.virtual_memory_bytes = (task.virtual_size > 0).then_some(task.virtual_size);
            process.threads = task.thread_count.max(0) as u32;
            quality.threads = Some(MetricQualityInfo::new(
                MetricQuality::Native,
                MetricSource::DirectApi,
            ));
        }
        Err(error) => {
            denied_probes += usize::from(is_access_denied(&error)) as u8;
            process.threads = 0;
            quality.threads = Some(
                MetricQualityInfo::new(MetricQuality::Unavailable, MetricSource::DirectApi)
                    .with_message("Thread count is unavailable for this process."),
            );
        }
    }

    match file_descriptor_count(pid) {
        Ok(count) => {
            successful_probes += 1;
            process.handles = count;
            quality.handles = Some(MetricQualityInfo::new(
                MetricQuality::Native,
                MetricSource::DirectApi,
            ));
        }
        Err(error) => {
            denied_probes += usize::from(is_access_denied(&error)) as u8;
            process.handles = 0;
            quality.handles = Some(
                MetricQualityInfo::new(MetricQuality::Unavailable, MetricSource::DirectApi)
                    .with_message("File-descriptor count is unavailable for this process."),
            );
        }
    }

    process.network_received_bps = None;
    process.network_transmitted_bps = None;
    quality.network = Some(
        MetricQualityInfo::new(MetricQuality::Unavailable, MetricSource::DirectApi)
            .with_message("Per-process network attribution is unavailable on macOS."),
    );
    quality.other_io = Some(
        MetricQualityInfo::new(MetricQuality::Unavailable, MetricSource::DirectApi)
            .with_message("Other per-process I/O is unavailable on macOS."),
    );

    let access = if successful_probes == 3 {
        AccessState::Full
    } else if successful_probes == 0 && denied_probes > 0 {
        AccessState::Denied
    } else {
        AccessState::Partial
    };
    process.access_state = access;
    access
}

fn process_rusage(pid: c_int) -> io::Result<RusageInfoV2> {
    let mut value = MaybeUninit::<RusageInfoV2>::zeroed();
    let result =
        unsafe { proc_pid_rusage(pid, RUSAGE_INFO_V2, value.as_mut_ptr().cast::<c_void>()) };
    if result == 0 {
        Ok(unsafe { value.assume_init() })
    } else {
        Err(io::Error::last_os_error())
    }
}

fn task_info(pid: c_int) -> io::Result<ProcTaskInfo> {
    let mut value = MaybeUninit::<ProcTaskInfo>::zeroed();
    let expected = size_of::<ProcTaskInfo>() as c_int;
    let result = unsafe {
        proc_pidinfo(
            pid,
            PROC_PIDTASKINFO,
            0,
            value.as_mut_ptr().cast::<c_void>(),
            expected,
        )
    };
    if result == expected {
        Ok(unsafe { value.assume_init() })
    } else {
        Err(io::Error::last_os_error())
    }
}

fn file_descriptor_count(pid: c_int) -> io::Result<u32> {
    let bytes = unsafe { proc_pidinfo(pid, PROC_PIDLISTFDS, 0, std::ptr::null_mut(), 0) };
    if bytes < 0 {
        return Err(io::Error::last_os_error());
    }
    if bytes == 0 && unsafe { libc::kill(pid, 0) } != 0 {
        let error = io::Error::last_os_error();
        if error.raw_os_error() == Some(libc::ESRCH) {
            return Err(error);
        }
    }
    Ok((bytes as usize / 8).min(u32::MAX as usize) as u32)
}

fn is_access_denied(error: &io::Error) -> bool {
    matches!(
        error.raw_os_error(),
        Some(libc::EPERM) | Some(libc::EACCES) | Some(libc::ESRCH)
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contracts::{ProcessMetricQuality, ProcessSample};

    fn sample(pid: u32) -> ProcessSample {
        ProcessSample {
            pid: pid.to_string(),
            parent_pid: None,
            start_time_ms: 0,
            name: "test".to_string(),
            exe: String::new(),
            status: "Run".to_string(),
            cpu_percent: 0.0,
            kernel_cpu_percent: None,
            memory_bytes: 1,
            private_bytes: 1,
            virtual_memory_bytes: None,
            disk_read_total_bytes: 0,
            disk_write_total_bytes: 0,
            other_io_total_bytes: None,
            disk_read_bps: 0,
            disk_write_bps: 0,
            other_io_bps: None,
            network_received_bps: None,
            network_transmitted_bps: None,
            threads: 0,
            handles: 0,
            access_state: AccessState::Partial,
            quality: Some(ProcessMetricQuality::default()),
        }
    }

    #[test]
    fn current_process_exposes_native_macos_metrics() {
        let mut rows = vec![sample(std::process::id())];
        let summary = MacosProcessCollector::new().enrich(&mut rows);
        assert_eq!(summary.denied_count, 0);
        assert_eq!(rows[0].access_state, AccessState::Full);
        assert!(rows[0].memory_bytes > 0);
        assert!(rows[0].private_bytes > 0);
        assert!(rows[0].threads > 0);
        assert!(rows[0]
            .quality
            .as_ref()
            .unwrap()
            .network
            .as_ref()
            .is_some_and(|quality| quality.quality == MetricQuality::Unavailable));
    }

    #[test]
    fn missing_process_is_retained_as_denied() {
        let mut rows = vec![sample(i32::MAX as u32)];
        let summary = MacosProcessCollector::new().enrich(&mut rows);
        assert_eq!(summary.denied_count, 1);
        assert_eq!(rows[0].access_state, AccessState::Denied);
        assert_eq!(rows[0].memory_bytes, 1);
        assert_eq!(rows[0].private_bytes, 0);
        assert_eq!(
            rows[0]
                .quality
                .as_ref()
                .and_then(|quality| quality.memory.as_ref())
                .map(|quality| quality.quality),
            Some(MetricQuality::Partial)
        );
    }
}
