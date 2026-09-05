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
const PROC_PIDTBSDINFO: c_int = 3;
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

// proc_bsdinfo from the platform SDK; unused fields retain the C layout.
#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
struct ProcBsdInfo {
    status_fields: [u32; 3],
    pid: u32,
    parent_pid: u32,
    credentials: [u32; 7],
    comm: [u8; 16],
    name: [u8; 32],
    accounting: [u32; 6],
    start_seconds: u64,
    start_microseconds: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ProcessBirth {
    pid: u32,
    parent_pid: u32,
    seconds: u64,
    microseconds: u64,
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
enum ProcessProbeFailure {
    Exited,
    Denied,
    Unsupported,
    Failed(i32),
}

impl ProcessProbeFailure {
    fn limitation_code(self) -> MetricLimitationCode {
        match self {
            Self::Denied => MetricLimitationCode::AccessDenied,
            Self::Unsupported => MetricLimitationCode::UnsupportedMetric,
            Self::Exited | Self::Failed(_) => MetricLimitationCode::CollectorFailure,
        }
    }
}

type ProcessProbeResult<T> = Result<T, ProcessProbeFailure>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MacosProcessOutcome {
    Access(AccessState),
    Exited,
}

trait MacosProcessProbes {
    fn birth(&self, pid: c_int) -> ProcessProbeResult<ProcessBirth>;
    fn rusage(&self, pid: c_int) -> ProcessProbeResult<RusageInfoV2>;
    fn task_info(&self, pid: c_int) -> ProcessProbeResult<ProcTaskInfo>;
    fn file_descriptor_count(&self, pid: c_int) -> ProcessProbeResult<u32>;
}

#[derive(Debug, Default)]
struct NativeMacosProcessProbes;

impl MacosProcessProbes for NativeMacosProcessProbes {
    fn birth(&self, pid: c_int) -> ProcessProbeResult<ProcessBirth> {
        process_birth(pid).map_err(classify_probe_error)
    }

    fn rusage(&self, pid: c_int) -> ProcessProbeResult<RusageInfoV2> {
        process_rusage(pid).map_err(classify_probe_error)
    }

    fn task_info(&self, pid: c_int) -> ProcessProbeResult<ProcTaskInfo> {
        task_info(pid).map_err(classify_probe_error)
    }

    fn file_descriptor_count(&self, pid: c_int) -> ProcessProbeResult<u32> {
        file_descriptor_count(pid).map_err(classify_probe_error)
    }
}

#[derive(Debug, Default)]
pub struct MacosProcessCollector;

impl MacosProcessCollector {
    pub fn new() -> Self {
        Self
    }

    pub fn enrich(
        &mut self,
        processes: &mut Vec<ProcessSample>,
        sample_started_ms: u64,
    ) -> MacosProcessCollection {
        self.enrich_with_probes(processes, &NativeMacosProcessProbes, sample_started_ms)
    }

    fn enrich_with_probes(
        &mut self,
        processes: &mut Vec<ProcessSample>,
        probes: &impl MacosProcessProbes,
        sample_started_ms: u64,
    ) -> MacosProcessCollection {
        let mut collection = MacosProcessCollection::default();
        processes.retain_mut(|process| {
            let Ok(pid) = process.pid.parse::<c_int>() else {
                process.access_state = AccessState::Partial;
                collection.partial_count += 1;
                return true;
            };

            let outcome = enrich_process(process, pid, probes, sample_started_ms);
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

fn enrich_process(
    process: &mut ProcessSample,
    pid: c_int,
    probes: &impl MacosProcessProbes,
    sample_started_ms: u64,
) -> MacosProcessOutcome {
    // sysinfo's initial identity is second-granularity. Do not enrich unknown generations.
    if process.start_time_ms == 0 || sample_started_ms == 0 {
        process.access_state = AccessState::Partial;
        return MacosProcessOutcome::Access(AccessState::Partial);
    }
    let before = match probes.birth(pid) {
        Ok(birth) => birth,
        Err(ProcessProbeFailure::Exited) => return MacosProcessOutcome::Exited,
        Err(_) => {
            process.access_state = AccessState::Partial;
            return MacosProcessOutcome::Access(AccessState::Partial);
        }
    };
    let start_ms = before
        .seconds
        .saturating_mul(1000)
        .saturating_add(before.microseconds / 1000);
    let matches_baseline = if process.start_time_ms.is_multiple_of(1000) {
        process.start_time_ms / 1000 == before.seconds
    } else {
        process.start_time_ms == start_ms
    };
    if before.pid != pid as u32 || !matches_baseline || start_ms >= sample_started_ms {
        return MacosProcessOutcome::Exited;
    }
    let mut enriched = process.clone();
    let outcome = enrich_process_metrics(&mut enriched, pid, probes);
    if outcome == MacosProcessOutcome::Exited {
        return outcome;
    }
    match probes.birth(pid) {
        Ok(after) if after == before => {
            enriched.start_time_ms = start_ms;
            enriched.parent_pid = (before.parent_pid != 0).then(|| before.parent_pid.to_string());
            *process = enriched;
            outcome
        }
        Ok(_) | Err(ProcessProbeFailure::Exited) => MacosProcessOutcome::Exited,
        Err(_) => {
            // None of the staged auxiliary values can be attributed after a failed recheck.
            process.access_state = AccessState::Partial;
            MacosProcessOutcome::Access(AccessState::Partial)
        }
    }
}

fn enrich_process_metrics(
    process: &mut ProcessSample,
    pid: c_int,
    probes: &impl MacosProcessProbes,
) -> MacosProcessOutcome {
    let mut successful_probes = 0_u8;
    let mut denied_probes = 0_u8;
    let mut unavailable_probes = 0_u8;
    let quality = process
        .quality
        .get_or_insert_with(ProcessMetricQuality::default);

    match probes.rusage(pid) {
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
        Err(failure) => {
            if failure == ProcessProbeFailure::Exited {
                return MacosProcessOutcome::Exited;
            }
            if failure == ProcessProbeFailure::Denied {
                denied_probes += 1;
            } else {
                unavailable_probes += 1;
            }
            let limitation = failure.limitation_code();
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

    match probes.task_info(pid) {
        Ok(task) => {
            successful_probes += 1;
            process.virtual_memory_bytes = (task.virtual_size > 0).then_some(task.virtual_size);
            process.threads = task.thread_count.max(0) as u32;
            quality.threads = Some(MetricQualityInfo::new(
                MetricQuality::Native,
                MetricSource::Libproc,
            ));
        }
        Err(failure) => {
            if failure == ProcessProbeFailure::Exited {
                return MacosProcessOutcome::Exited;
            }
            if failure == ProcessProbeFailure::Denied {
                denied_probes += 1;
            } else {
                unavailable_probes += 1;
            }
            process.threads = 0;
            quality.threads = Some(
                MetricQualityInfo::new(MetricQuality::Unavailable, MetricSource::Libproc)
                    .with_limitation(
                        failure.limitation_code(),
                        "Thread count is unavailable for this process.",
                    ),
            );
        }
    }

    match probes.file_descriptor_count(pid) {
        Ok(count) => {
            successful_probes += 1;
            process.handles = count;
            quality.handles = Some(MetricQualityInfo::new(
                MetricQuality::Native,
                MetricSource::Libproc,
            ));
        }
        Err(failure) => {
            if failure == ProcessProbeFailure::Exited {
                return MacosProcessOutcome::Exited;
            }
            if failure == ProcessProbeFailure::Denied {
                denied_probes += 1;
            } else {
                unavailable_probes += 1;
            }
            process.handles = 0;
            quality.handles = Some(
                MetricQualityInfo::new(MetricQuality::Unavailable, MetricSource::Libproc)
                    .with_limitation(
                        failure.limitation_code(),
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
    } else if successful_probes == 0 && denied_probes > 0 && unavailable_probes == 0 {
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

fn process_birth(pid: c_int) -> io::Result<ProcessBirth> {
    let mut value = MaybeUninit::<ProcBsdInfo>::zeroed();
    let expected = size_of::<ProcBsdInfo>() as c_int;
    let result = unsafe {
        proc_pidinfo(
            pid,
            PROC_PIDTBSDINFO,
            0,
            value.as_mut_ptr().cast(),
            expected,
        )
    };
    if result != expected {
        return Err(io::Error::last_os_error());
    }
    let value = unsafe { value.assume_init() };
    if value.pid != pid as u32 || value.start_seconds == 0 || value.start_microseconds >= 1_000_000
    {
        return Err(io::Error::from_raw_os_error(libc::ESRCH));
    }
    Ok(ProcessBirth {
        pid: value.pid,
        parent_pid: value.parent_pid,
        seconds: value.start_seconds,
        microseconds: value.start_microseconds,
    })
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
        return Err(io::Error::last_os_error());
    }
    Ok((bytes as usize / 8).min(u32::MAX as usize) as u32)
}

fn is_access_denied(error: &io::Error) -> bool {
    matches!(error.raw_os_error(), Some(libc::EPERM) | Some(libc::EACCES))
}

fn is_process_exited(error: &io::Error) -> bool {
    error.raw_os_error() == Some(libc::ESRCH)
}

fn classify_probe_error(error: io::Error) -> ProcessProbeFailure {
    if is_process_exited(&error) {
        return ProcessProbeFailure::Exited;
    }
    if is_access_denied(&error) {
        return ProcessProbeFailure::Denied;
    }
    let code = error.raw_os_error().unwrap_or(0);
    if [libc::ENOSYS, libc::ENOTSUP].contains(&code) {
        ProcessProbeFailure::Unsupported
    } else {
        ProcessProbeFailure::Failed(code)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contracts::{ProcessMetricQuality, ProcessSample};
    use std::cell::Cell;

    #[derive(Clone)]
    struct FixtureProbes {
        before: ProcessProbeResult<ProcessBirth>,
        after: ProcessProbeResult<ProcessBirth>,
        birth_calls: Cell<usize>,
        rusage: ProcessProbeResult<RusageInfoV2>,
        task: ProcessProbeResult<ProcTaskInfo>,
        descriptor_count: ProcessProbeResult<u32>,
    }

    impl FixtureProbes {
        fn full() -> Self {
            Self {
                before: Ok(ProcessBirth {
                    pid: 42,
                    parent_pid: 1,
                    seconds: 1,
                    microseconds: 123_000,
                }),
                after: Ok(ProcessBirth {
                    pid: 42,
                    parent_pid: 1,
                    seconds: 1,
                    microseconds: 123_000,
                }),
                birth_calls: Cell::new(0),
                rusage: Ok(RusageInfoV2 {
                    resident_size: 4_096,
                    physical_footprint: 3_072,
                    disk_bytes_read: 400,
                    disk_bytes_written: 200,
                    ..RusageInfoV2::default()
                }),
                task: Ok(ProcTaskInfo {
                    virtual_size: 8_192,
                    thread_count: 4,
                    ..ProcTaskInfo::default()
                }),
                descriptor_count: Ok(12),
            }
        }
    }

    impl MacosProcessProbes for FixtureProbes {
        fn birth(&self, _pid: c_int) -> ProcessProbeResult<ProcessBirth> {
            let calls = self.birth_calls.get();
            self.birth_calls.set(calls + 1);
            if calls == 0 {
                self.before
            } else {
                self.after
            }
        }

        fn rusage(&self, _pid: c_int) -> ProcessProbeResult<RusageInfoV2> {
            self.rusage
        }

        fn task_info(&self, _pid: c_int) -> ProcessProbeResult<ProcTaskInfo> {
            self.task
        }

        fn file_descriptor_count(&self, _pid: c_int) -> ProcessProbeResult<u32> {
            self.descriptor_count
        }
    }

    fn sample(pid: u32) -> ProcessSample {
        ProcessSample {
            pid: pid.to_string(),
            parent_pid: None,
            start_time_ms: 1_000,
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
    fn generation_change_during_native_probes_discards_enrichment() {
        let mut rows = vec![sample(42)];
        let mut probes = FixtureProbes::full();
        probes.after = Ok(ProcessBirth {
            microseconds: 124_000,
            ..probes.before.unwrap()
        });
        let summary = MacosProcessCollector::new().enrich_with_probes(&mut rows, &probes, 2_000);
        assert_eq!(summary.exited_count, 1);
        assert!(rows.is_empty());
    }

    #[test]
    fn missing_final_birth_discards_staged_values_and_unknown_birth_skips_probes() {
        let mut rows = vec![sample(42)];
        let mut probes = FixtureProbes::full();
        probes.after = Err(ProcessProbeFailure::Denied);
        let summary = MacosProcessCollector::new().enrich_with_probes(&mut rows, &probes, 2_000);
        assert_eq!(summary.partial_count, 1);
        assert_eq!(rows[0].memory_bytes, 1);
        assert_eq!(rows[0].io_read_total_bytes, 0);
        assert_eq!(rows[0].handles, 0);
        let probes = FixtureProbes::full();
        rows[0].start_time_ms = 0;
        MacosProcessCollector::new().enrich_with_probes(&mut rows, &probes, 2_000);
        assert_eq!(probes.birth_calls.get(), 0);
    }

    #[test]
    fn initial_birth_mismatch_discards_the_old_sample() {
        let mut rows = vec![sample(42)];
        let mut probes = FixtureProbes::full();
        probes.before = Ok(ProcessBirth {
            seconds: 2,
            ..probes.before.unwrap()
        });
        let summary = MacosProcessCollector::new().enrich_with_probes(&mut rows, &probes, 2_000);
        assert_eq!(summary.exited_count, 1);
        assert!(rows.is_empty());
    }

    #[test]
    fn birth_after_sample_start_is_dropped_even_inside_the_same_sysinfo_second() {
        let mut rows = vec![sample(42)];
        let probes = FixtureProbes::full();
        let summary = MacosProcessCollector::new().enrich_with_probes(&mut rows, &probes, 1_122);
        assert_eq!(summary.exited_count, 1);
        assert!(rows.is_empty());
    }

    #[test]
    fn current_process_exposes_native_macos_metrics() {
        let mut rows = vec![sample(std::process::id())];
        rows[0].start_time_ms = process_birth(std::process::id() as c_int).unwrap().seconds * 1000;
        let summary = MacosProcessCollector::new().enrich(&mut rows, crate::telemetry::now_ms());
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
        let summary = MacosProcessCollector::new().enrich(&mut rows, crate::telemetry::now_ms());
        assert_eq!(summary.denied_count, 0);
        assert_eq!(summary.exited_count, 1);
        assert!(rows.is_empty());
    }

    #[test]
    fn process_exit_is_not_classified_as_access_denial() {
        let error = io::Error::from_raw_os_error(libc::ESRCH);
        assert!(is_process_exited(&error));
        assert!(!is_access_denied(&error));
        assert_eq!(classify_probe_error(error), ProcessProbeFailure::Exited);
    }

    #[test]
    fn exit_fixture_drops_churn_without_degrading_access() {
        let mut rows = vec![sample(42)];
        let mut probes = FixtureProbes::full();
        probes.rusage = Err(ProcessProbeFailure::Exited);

        let summary = MacosProcessCollector::new().enrich_with_probes(&mut rows, &probes, 2_000);

        assert_eq!(summary.exited_count, 1);
        assert_eq!(summary.denied_count, 0);
        assert_eq!(summary.partial_count, 0);
        assert!(rows.is_empty());
    }

    #[test]
    fn denial_fixture_preserves_the_row_without_publishable_io() {
        let mut rows = vec![sample(42)];
        let probes = FixtureProbes {
            rusage: Err(ProcessProbeFailure::Denied),
            task: Err(ProcessProbeFailure::Denied),
            descriptor_count: Err(ProcessProbeFailure::Denied),
            ..FixtureProbes::full()
        };

        let summary = MacosProcessCollector::new().enrich_with_probes(&mut rows, &probes, 2_000);

        assert_eq!(summary.denied_count, 1);
        assert_eq!(summary.exited_count, 0);
        assert_eq!(rows[0].access_state, AccessState::Denied);
        assert_eq!(rows[0].io_read_total_bytes, 0);
        assert_eq!(rows[0].io_write_total_bytes, 0);
        let io = rows[0]
            .quality
            .as_ref()
            .and_then(|quality| quality.io.as_ref())
            .expect("I/O quality");
        assert_eq!(io.quality, MetricQuality::Unavailable);
        assert_eq!(io.limitation_code, Some(MetricLimitationCode::AccessDenied));
    }

    #[test]
    fn partial_enrichment_fixture_keeps_independent_probe_truth() {
        let mut rows = vec![sample(42)];
        let mut probes = FixtureProbes::full();
        probes.task = Err(ProcessProbeFailure::Unsupported);

        let summary = MacosProcessCollector::new().enrich_with_probes(&mut rows, &probes, 2_000);

        assert_eq!(summary.partial_count, 1);
        assert_eq!(rows[0].access_state, AccessState::Partial);
        assert_eq!(rows[0].private_bytes, 3_072);
        assert_eq!(rows[0].io_read_total_bytes, 400);
        assert_eq!(rows[0].handles, 12);
        let quality = rows[0].quality.as_ref().expect("process quality");
        assert_eq!(
            quality.io.as_ref().map(|quality| quality.quality),
            Some(MetricQuality::Native)
        );
        assert_eq!(
            quality
                .threads
                .as_ref()
                .and_then(|quality| quality.limitation_code),
            Some(MetricLimitationCode::UnsupportedMetric)
        );
        assert_eq!(
            quality
                .network
                .as_ref()
                .and_then(|quality| quality.limitation_code),
            Some(MetricLimitationCode::UnsupportedMetric)
        );
    }

    #[test]
    fn mixed_denial_and_collector_failure_is_partial_not_denied() {
        let mut rows = vec![sample(42)];
        let probes = FixtureProbes {
            rusage: Err(ProcessProbeFailure::Denied),
            task: Err(ProcessProbeFailure::Failed(libc::EIO)),
            descriptor_count: Err(ProcessProbeFailure::Unsupported),
            ..FixtureProbes::full()
        };

        let summary = MacosProcessCollector::new().enrich_with_probes(&mut rows, &probes, 2_000);

        assert_eq!(summary.denied_count, 0);
        assert_eq!(summary.partial_count, 1);
        assert_eq!(rows[0].access_state, AccessState::Partial);
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
