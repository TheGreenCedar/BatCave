use std::{
    ffi::{c_int, c_void},
    io,
    mem::{size_of, MaybeUninit},
};

use crate::contracts::{
    AccessState, MetricLimitationCode, MetricQuality, MetricQualityInfo, MetricSource,
    ProcessMetricQuality, ProcessSample,
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
    pub exited_count: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MacosProcessOutcome {
    Access(AccessState),
    Exited,
}

#[derive(Debug, Default)]
pub struct MacosProcessCollector;

impl MacosProcessCollector {
    pub fn new() -> Self {
        Self
    }

    pub fn enrich(&mut self, processes: &mut Vec<ProcessSample>) -> MacosProcessCollection {
        let mut collection = MacosProcessCollection::default();
        processes.retain_mut(|process| {
            let Ok(pid) = process.pid.parse::<c_int>() else {
                process.access_state = AccessState::Partial;
                collection.partial_count += 1;
                return true;
            };

            let outcome = enrich_process(process, pid);
            match outcome {
                MacosProcessOutcome::Access(AccessState::Denied) => collection.denied_count += 1,
                MacosProcessOutcome::Access(AccessState::Partial) => collection.partial_count += 1,
                MacosProcessOutcome::Access(AccessState::Full) => {}
                MacosProcessOutcome::Exited => {
                    collection.exited_count += 1;
                    return false;
                }
            }
            true
        });
        collection
    }
}

fn enrich_process(process: &mut ProcessSample, pid: c_int) -> MacosProcessOutcome {
    let mut successful_probes = 0_u8;
    let mut denied_probes = 0_u8;
    let quality = process
        .quality
        .get_or_insert_with(ProcessMetricQuality::default);

    match process_rusage(pid) {
        Ok(rusage) => {
            successful_probes += 1;
            (process.memory_bytes, process.private_bytes) =
                rusage_memory_values(process.memory_bytes, &rusage);
            process.io_read_total_bytes = rusage.disk_bytes_read;
            process.io_write_total_bytes = rusage.disk_bytes_written;
            quality.memory = Some(rusage_memory_quality(&rusage));
            quality.io = Some(MetricQualityInfo::new(
                MetricQuality::Native,
                MetricSource::Libproc,
            ));
        }
        Err(error) => {
            if is_process_exited(&error) {
                return MacosProcessOutcome::Exited;
            }
            let access_denied = is_access_denied(&error);
            denied_probes += usize::from(access_denied) as u8;
            let limitation = if access_denied {
                MetricLimitationCode::AccessDenied
            } else {
                MetricLimitationCode::CollectorFailure
            };
            process.private_bytes = 0;
            quality.memory = Some(
                MetricQualityInfo::new(MetricQuality::Partial, MetricSource::Sysinfo)
                    .with_limitation(limitation, PHYSICAL_FOOTPRINT_UNAVAILABLE),
            );
            process.io_read_total_bytes = 0;
            process.io_write_total_bytes = 0;
            quality.io = Some(unavailable_io_quality(limitation));
        }
    }

    match task_info(pid) {
        Ok(task) => {
            successful_probes += 1;
            process.virtual_memory_bytes = (task.virtual_size > 0).then_some(task.virtual_size);
            process.threads = task.thread_count.max(0) as u32;
            quality.threads = Some(MetricQualityInfo::new(
                MetricQuality::Native,
                MetricSource::Libproc,
            ));
        }
        Err(error) => {
            if is_process_exited(&error) {
                return MacosProcessOutcome::Exited;
            }
            let access_denied = is_access_denied(&error);
            denied_probes += usize::from(access_denied) as u8;
            process.threads = 0;
            quality.threads = Some(
                MetricQualityInfo::new(MetricQuality::Unavailable, MetricSource::Libproc)
                    .with_limitation(
                        if access_denied {
                            MetricLimitationCode::AccessDenied
                        } else {
                            MetricLimitationCode::CollectorFailure
                        },
                        "Thread count is unavailable for this process.",
                    ),
            );
        }
    }

    match file_descriptor_count(pid) {
        Ok(count) => {
            successful_probes += 1;
            process.handles = count;
            quality.handles = Some(MetricQualityInfo::new(
                MetricQuality::Native,
                MetricSource::Libproc,
            ));
        }
        Err(error) => {
            if is_process_exited(&error) {
                return MacosProcessOutcome::Exited;
            }
            let access_denied = is_access_denied(&error);
            denied_probes += usize::from(access_denied) as u8;
            process.handles = 0;
            quality.handles = Some(
                MetricQualityInfo::new(MetricQuality::Unavailable, MetricSource::Libproc)
                    .with_limitation(
                        if access_denied {
                            MetricLimitationCode::AccessDenied
                        } else {
                            MetricLimitationCode::CollectorFailure
                        },
                        "File-descriptor count is unavailable for this process.",
                    ),
            );
        }
    }

    process.network_received_bps = None;
    process.network_transmitted_bps = None;
    quality.network = Some(
        MetricQualityInfo::new(MetricQuality::Unavailable, MetricSource::Libproc).with_limitation(
            MetricLimitationCode::UnsupportedMetric,
            "Per-process network attribution is unavailable on macOS.",
        ),
    );
    quality.other_io = Some(
        MetricQualityInfo::new(MetricQuality::Unavailable, MetricSource::Libproc).with_limitation(
            MetricLimitationCode::UnsupportedMetric,
            "Other per-process I/O is unavailable on macOS.",
        ),
    );

    let access = if successful_probes == 3 {
        AccessState::Full
    } else if successful_probes == 0 && denied_probes > 0 {
        AccessState::Denied
    } else {
        AccessState::Partial
    };
    process.access_state = access;
    MacosProcessOutcome::Access(access)
}

fn rusage_memory_values(fallback_resident: u64, rusage: &RusageInfoV2) -> (u64, u64) {
    (
        if rusage.resident_size > 0 {
            rusage.resident_size
        } else {
            fallback_resident
        },
        rusage.physical_footprint,
    )
}

fn rusage_memory_quality(rusage: &RusageInfoV2) -> MetricQualityInfo {
    if rusage.resident_size > 0 {
        MetricQualityInfo::new(MetricQuality::Native, MetricSource::Libproc)
    } else {
        MetricQualityInfo::new(MetricQuality::Partial, MetricSource::Sysinfo).with_limitation(
            MetricLimitationCode::UnsupportedMetric,
            PHYSICAL_FOOTPRINT_UNAVAILABLE,
        )
    }
}

fn unavailable_io_quality(limitation: MetricLimitationCode) -> MetricQualityInfo {
    MetricQualityInfo::new(MetricQuality::Unavailable, MetricSource::Libproc).with_limitation(
        limitation,
        "Native process read/write totals are unavailable.",
    )
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
    matches!(error.raw_os_error(), Some(libc::EPERM) | Some(libc::EACCES))
}

fn is_process_exited(error: &io::Error) -> bool {
    error.raw_os_error() == Some(libc::ESRCH)
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
            io_read_total_bytes: 0,
            io_write_total_bytes: 0,
            other_io_total_bytes: None,
            io_read_bps: 0,
            io_write_bps: 0,
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
        assert_eq!(
            rows[0]
                .quality
                .as_ref()
                .and_then(|quality| quality.io.as_ref())
                .map(|quality| (quality.quality, quality.source)),
            Some((MetricQuality::Native, Some(MetricSource::Libproc)))
        );
    }

    #[test]
    fn missing_process_is_dropped_as_ordinary_churn() {
        let mut rows = vec![sample(i32::MAX as u32)];
        let summary = MacosProcessCollector::new().enrich(&mut rows);
        assert_eq!(summary.denied_count, 0);
        assert_eq!(summary.exited_count, 1);
        assert!(rows.is_empty());
    }

    #[test]
    fn process_exit_is_not_classified_as_access_denial() {
        let error = io::Error::from_raw_os_error(libc::ESRCH);
        assert!(is_process_exited(&error));
        assert!(!is_access_denied(&error));
    }

    #[test]
    fn failed_direct_io_never_claims_a_sysinfo_estimate() {
        for limitation in [
            MetricLimitationCode::AccessDenied,
            MetricLimitationCode::CollectorFailure,
        ] {
            let quality = unavailable_io_quality(limitation);
            assert_eq!(quality.quality, MetricQuality::Unavailable);
            assert_eq!(quality.source, Some(MetricSource::Libproc));
            assert_eq!(quality.limitation_code, Some(limitation));
        }
    }

    #[test]
    fn zero_physical_footprint_clears_the_sysinfo_fallback_value() {
        let mut process = sample(std::process::id());
        let rusage = RusageInfoV2 {
            resident_size: 4_096,
            physical_footprint: 0,
            ..RusageInfoV2::default()
        };

        (process.memory_bytes, process.private_bytes) =
            rusage_memory_values(process.memory_bytes, &rusage);

        assert_eq!(process.memory_bytes, 4_096);
        assert_eq!(process.private_bytes, 0);
    }

    #[test]
    fn partial_rusage_keeps_resident_fallback_publishable_but_not_native() {
        let rusage = RusageInfoV2 {
            resident_size: 0,
            physical_footprint: 8_192,
            ..RusageInfoV2::default()
        };

        let quality = rusage_memory_quality(&rusage);

        assert_eq!(quality.quality, MetricQuality::Partial);
        assert_eq!(quality.source, Some(MetricSource::Sysinfo));
    }
}
