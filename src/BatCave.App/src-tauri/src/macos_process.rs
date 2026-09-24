use std::{
    collections::{HashMap, HashSet},
    ffi::{c_int, c_void},
    io,
    mem::{size_of, MaybeUninit},
    time::Instant,
};

use crate::contracts::{
    AccessState, MetricLimitationCode, MetricQuality, MetricQualityInfo, MetricSource,
    ProcessMetricQuality, ProcessSample,
};

const PROC_PIDLISTFDS: c_int = 1;
const PROC_PIDTASKINFO: c_int = 4;
const PROC_PIDTHREADINFO: c_int = 5;
const RUSAGE_INFO_V2: c_int = 2;
const PROC_PIDPATHINFO_MAXSIZE: u32 = 4096;
const MAXCOMLEN: usize = 16;
const PHYSICAL_FOOTPRINT_UNAVAILABLE: &str =
    "Resident memory uses the task-info fallback; physical footprint is unavailable.";

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

// proc_pidthinfo (flavor PROC_PIDTHREADINFO, arg 0); unused fields retain the C layout.
#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
struct ProcPidThInfo {
    total_time: u64,
    total_system: u64,
    total_user: u64,
    max_iosz: u32,
    run_state: i32,
    flags: i32,
    sleep_time: i32,
    curpri: i32,
    priority: i32,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
struct MachTimebaseInfo {
    numer: u32,
    denom: u32,
}

// struct timeval on LP64 Darwin: time_t tv_sec, suseconds_t tv_usec.
#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
struct KernelTimeval {
    seconds: i64,
    microseconds: i32,
}

// struct itimerval.
#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
struct KernelItimerval {
    interval: KernelTimeval,
    value: KernelTimeval,
}

// struct extern_proc from <sys/proc.h>; only the p_un.__p_starttime member of the
// leading union is needed, and timeval is its largest member.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
struct ExternProc {
    start_time: KernelTimeval,
    vmspace: *mut c_void,
    sigacts: *mut c_void,
    flag: i32,
    stat: u8,
    pid: i32,
    oppid: i32,
    dupfd: i32,
    user_stack: *mut c_void,
    exit_thread: *mut c_void,
    debugger: i32,
    sigwait: i32,
    estcpu: u32,
    cpticks: i32,
    pctcpu: u32,
    wchan: *mut c_void,
    wmesg: *mut c_void,
    swtime: u32,
    slptime: u32,
    realtimer: KernelItimerval,
    rtime: KernelTimeval,
    uticks: u64,
    sticks: u64,
    iticks: u64,
    traceflag: i32,
    tracep: *mut c_void,
    siglist: i32,
    textvp: *mut c_void,
    holdcnt: i32,
    sigmask: u32,
    sigignore: u32,
    sigcatch: u32,
    priority: u8,
    usrpri: u8,
    nice: i8,
    comm: [u8; MAXCOMLEN + 1],
    pgrp: *mut c_void,
    addr: *mut c_void,
    xstat: u16,
    acflag: u16,
    ru: *mut c_void,
}

// struct _pcred from <sys/sysctl.h>.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
struct EPcred {
    lock: [u8; 72],
    ucred: *mut c_void,
    ruid: u32,
    svuid: u32,
    rgid: u32,
    svgid: u32,
    refcnt: i32,
}

// struct _ucred; NGROUPS == NGROUPS_MAX == 16.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
struct EUcred {
    cr_ref: i32,
    cr_uid: u32,
    cr_ngroups: i16,
    cr_groups: [u32; 16],
}

// struct vmspace from <sys/vm.h> (opaque dummy layout).
#[repr(C)]
#[derive(Debug, Clone, Copy)]
struct EVmspace {
    dummy: i32,
    dummy2: *mut c_void,
    dummy3: [i32; 5],
    dummy4: [*mut c_void; 3],
}

// struct eproc inside kinfo_proc.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
struct Eproc {
    paddr: *mut c_void,
    sess: *mut c_void,
    pcred: EPcred,
    ucred: EUcred,
    vm: EVmspace,
    ppid: i32,
    pgid: i32,
    jobc: i16,
    tdev: i32,
    tpgid: i32,
    tsess: *mut c_void,
    wmesg: [u8; 8],
    xsize: i32,
    xrssize: i16,
    xccount: i16,
    xswrss: i16,
    flag: i32,
    login: [u8; 12],
    spare: [i32; 4],
}

// struct kinfo_proc from <sys/sysctl.h>, as returned by KERN_PROC sysctls.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
struct KinfoProc {
    proc_: ExternProc,
    eproc: Eproc,
}

#[derive(Debug, Clone, Copy)]
struct ProcessIdentity {
    pid: u32,
    parent_pid: u32,
    status: u32,
    comm: [u8; MAXCOMLEN + 1],
    seconds: u64,
    microseconds: u64,
}

impl ProcessIdentity {
    fn start_ms(&self) -> u64 {
        self.seconds
            .saturating_mul(1000)
            .saturating_add(self.microseconds / 1000)
    }

    fn comm(&self) -> String {
        nul_trimmed(&self.comm)
    }

    // Only the generation identity is compared: transient fields such as p_stat may
    // legitimately change between the listing and the per-pid recheck.
    fn same_generation(&self, other: &Self) -> bool {
        self.pid == other.pid
            && self.seconds == other.seconds
            && self.microseconds == other.microseconds
    }
}

impl From<&KinfoProc> for ProcessIdentity {
    fn from(info: &KinfoProc) -> Self {
        Self {
            pid: info.proc_.pid.max(0) as u32,
            parent_pid: info.eproc.ppid.max(0) as u32,
            status: info.proc_.stat as u32,
            comm: info.proc_.comm,
            seconds: info.proc_.start_time.seconds.max(0) as u64,
            microseconds: info.proc_.start_time.microseconds.max(0) as u64,
        }
    }
}

fn nul_trimmed(bytes: &[u8]) -> String {
    let end = bytes
        .iter()
        .position(|byte| *byte == 0)
        .unwrap_or(bytes.len());
    String::from_utf8_lossy(&bytes[..end]).into_owned()
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
    fn proc_pidpath(pid: c_int, buffer: *mut c_void, buffer_size: u32) -> c_int;
}

unsafe extern "C" {
    fn mach_timebase_info(info: *mut MachTimebaseInfo) -> c_int;
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

trait MacosProcessProbes {
    fn list_processes(&self) -> ProcessProbeResult<Vec<ProcessIdentity>>;
    fn identity(&self, pid: c_int) -> ProcessProbeResult<ProcessIdentity>;
    fn task_info(&self, pid: c_int) -> ProcessProbeResult<ProcTaskInfo>;
    fn rusage(&self, pid: c_int) -> ProcessProbeResult<RusageInfoV2>;
    fn thread_status(&self, pid: c_int) -> ProcessProbeResult<i32>;
    fn file_descriptor_count(&self, pid: c_int) -> ProcessProbeResult<u32>;
    fn executable_path(&self, pid: c_int) -> ProcessProbeResult<String>;
}

#[derive(Debug, Default)]
struct NativeMacosProcessProbes;

impl MacosProcessProbes for NativeMacosProcessProbes {
    fn list_processes(&self) -> ProcessProbeResult<Vec<ProcessIdentity>> {
        kernel_process_list().map_err(classify_probe_error)
    }

    fn identity(&self, pid: c_int) -> ProcessProbeResult<ProcessIdentity> {
        kernel_process_identity(pid).map_err(classify_probe_error)
    }

    fn task_info(&self, pid: c_int) -> ProcessProbeResult<ProcTaskInfo> {
        task_info(pid).map_err(classify_probe_error)
    }

    fn rusage(&self, pid: c_int) -> ProcessProbeResult<RusageInfoV2> {
        process_rusage(pid).map_err(classify_probe_error)
    }

    fn thread_status(&self, pid: c_int) -> ProcessProbeResult<i32> {
        thread_status(pid).map_err(classify_probe_error)
    }

    fn file_descriptor_count(&self, pid: c_int) -> ProcessProbeResult<u32> {
        file_descriptor_count(pid).map_err(classify_probe_error)
    }

    fn executable_path(&self, pid: c_int) -> ProcessProbeResult<String> {
        executable_path(pid).map_err(classify_probe_error)
    }
}

// (pid, start_time_ms) identifies one process generation across PID reuse.
type GenerationKey = (u32, u64);

#[derive(Default)]
pub struct MacosProcessCollector {
    exe_cache: HashMap<GenerationKey, String>,
    cpu_baselines: HashMap<GenerationKey, (u64, Instant)>,
    timebase: Option<(u64, u64)>,
    previous_sample_started_ms: Option<u64>,
}

impl MacosProcessCollector {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn collect(
        &mut self,
        sample_started_ms: u64,
    ) -> (Vec<ProcessSample>, MacosProcessCollection) {
        self.collect_with_probes(&NativeMacosProcessProbes, sample_started_ms, &Instant::now)
    }

    fn collect_with_probes(
        &mut self,
        probes: &impl MacosProcessProbes,
        sample_started_ms: u64,
        clock: &dyn Fn() -> Instant,
    ) -> (Vec<ProcessSample>, MacosProcessCollection) {
        let mut collection = MacosProcessCollection::default();
        let Ok(identities) = probes.list_processes() else {
            return (Vec::new(), collection);
        };
        let previous_sample_started_ms = self.previous_sample_started_ms.replace(sample_started_ms);
        // A zero start timestamp disables the "born after sampling began" filter.
        let cutoff = if sample_started_ms == 0 {
            u64::MAX
        } else {
            sample_started_ms
        };
        let mut seen = HashSet::with_capacity(identities.len());
        let mut processes = Vec::with_capacity(identities.len());
        for identity in identities {
            if let Some(process) = self.collect_process(
                identity,
                probes,
                cutoff,
                sample_started_ms,
                previous_sample_started_ms,
                clock,
                &mut seen,
                &mut collection,
            ) {
                processes.push(process);
            }
        }
        self.exe_cache.retain(|key, _| seen.contains(key));
        self.cpu_baselines.retain(|key, _| seen.contains(key));
        (processes, collection)
    }

    #[allow(clippy::too_many_arguments)]
    fn collect_process(
        &mut self,
        before: ProcessIdentity,
        probes: &impl MacosProcessProbes,
        cutoff: u64,
        sample_started_ms: u64,
        previous_sample_started_ms: Option<u64>,
        clock: &dyn Fn() -> Instant,
        seen: &mut HashSet<GenerationKey>,
        collection: &mut MacosProcessCollection,
    ) -> Option<ProcessSample> {
        let pid = before.pid as c_int;
        let start_ms = before.start_ms();
        if start_ms >= cutoff {
            collection.exited_count += 1;
            return None;
        }
        let generation = (before.pid, start_ms);

        let task = probes.task_info(pid);
        if matches!(task, Err(ProcessProbeFailure::Exited)) {
            collection.exited_count += 1;
            return None;
        }
        let rusage = probes.rusage(pid);
        if matches!(rusage, Err(ProcessProbeFailure::Exited)) {
            collection.exited_count += 1;
            return None;
        }
        let thread = probes.thread_status(pid);
        if matches!(thread, Err(ProcessProbeFailure::Exited)) {
            collection.exited_count += 1;
            return None;
        }
        let descriptors = probes.file_descriptor_count(pid);
        if matches!(descriptors, Err(ProcessProbeFailure::Exited)) {
            collection.exited_count += 1;
            return None;
        }

        let process = match probes.identity(pid) {
            Ok(after) if before.same_generation(&after) => self.build_sample(
                pid,
                generation,
                &before,
                task,
                rusage,
                thread,
                descriptors,
                probes,
                clock,
                sample_started_ms,
                previous_sample_started_ms,
            ),
            Ok(_) | Err(ProcessProbeFailure::Exited) => {
                collection.exited_count += 1;
                return None;
            }
            // None of the staged auxiliary values can be attributed after a failed recheck.
            Err(failure) => unverified_sample(&before, failure),
        };

        seen.insert(generation);
        match process.access_state {
            AccessState::Denied => collection.denied_count += 1,
            AccessState::Partial => collection.partial_count += 1,
            AccessState::Full => {}
        }
        Some(process)
    }

    #[allow(clippy::too_many_arguments)]
    fn build_sample(
        &mut self,
        pid: c_int,
        generation: GenerationKey,
        identity: &ProcessIdentity,
        task: ProcessProbeResult<ProcTaskInfo>,
        rusage: ProcessProbeResult<RusageInfoV2>,
        thread: ProcessProbeResult<i32>,
        descriptors: ProcessProbeResult<u32>,
        probes: &impl MacosProcessProbes,
        clock: &dyn Fn() -> Instant,
        sample_started_ms: u64,
        previous_sample_started_ms: Option<u64>,
    ) -> ProcessSample {
        // The access state is derived from the three metric-bearing probes
        // (task_info, rusage, file_descriptor_count). thread_status only refines the
        // display status and has a kernel-table fallback, so it never degrades access.
        let mut successful_probes = 0_u8;
        let mut denied_probes = 0_u8;
        let mut unavailable_probes = 0_u8;
        let mut quality = ProcessMetricQuality::default();

        let task_ok = task.is_ok();
        let resident_fallback = task.as_ref().map(|task| task.resident_size).unwrap_or(0);
        let memory_bytes;
        let mut private_bytes = 0;
        let mut virtual_memory_bytes = None;
        let mut threads = 0;
        let mut handles = 0;
        let mut io_read_total_bytes = 0;
        let mut io_write_total_bytes = 0;
        let mut cpu_percent = 0.0;

        match task {
            Ok(task) => {
                successful_probes += 1;
                virtual_memory_bytes = (task.virtual_size > 0).then_some(task.virtual_size);
                threads = task.thread_count.max(0) as u32;
                quality.threads = Some(MetricQualityInfo::new(
                    MetricQuality::Native,
                    MetricSource::Libproc,
                ));

                let cpu_ns =
                    self.mach_units_to_ns(task.total_user.saturating_add(task.total_system));
                let now = clock();
                quality.cpu = Some(match self.cpu_baselines.insert(generation, (cpu_ns, now)) {
                    Some((previous_ns, previous_at)) => {
                        let delta_cpu = cpu_ns.saturating_sub(previous_ns) as f64;
                        let delta_wall = now.duration_since(previous_at).as_nanos() as f64;
                        if delta_wall > 0.0 {
                            cpu_percent = round1(delta_cpu / delta_wall * 100.0);
                        }
                        MetricQualityInfo::new(MetricQuality::Native, MetricSource::Libproc)
                    }
                    None => {
                        // No baseline: if the process was born between collections,
                        // average its lifetime CPU instead of holding for a tick.
                        let born_since_last_collection = previous_sample_started_ms
                            .is_some_and(|previous| generation.1 >= previous);
                        let age_ms = sample_started_ms.saturating_sub(generation.1);
                        if born_since_last_collection && age_ms >= 250 {
                            cpu_percent =
                                round1(cpu_ns as f64 / (age_ms as f64 * 1e6) * 100.0);
                            MetricQualityInfo::new(MetricQuality::Native, MetricSource::Libproc)
                        } else {
                            MetricQualityInfo::new(MetricQuality::Held, MetricSource::Libproc)
                                .with_limitation(
                                    MetricLimitationCode::PendingBaseline,
                                    "Waiting for a second CPU sample.",
                                )
                        }
                    }
                });
            }
            Err(failure) => {
                count_probe_failure(failure, &mut denied_probes, &mut unavailable_probes);
                quality.threads = Some(
                    MetricQualityInfo::new(MetricQuality::Unavailable, MetricSource::Libproc)
                        .with_limitation(
                            failure.limitation_code(),
                            "Thread count is unavailable for this process.",
                        ),
                );
                quality.cpu = Some(
                    MetricQualityInfo::new(MetricQuality::Unavailable, MetricSource::Libproc)
                        .with_limitation(
                            failure.limitation_code(),
                            "Process CPU time is unavailable for this process.",
                        ),
                );
            }
        }

        match rusage {
            Ok(rusage) => {
                successful_probes += 1;
                (memory_bytes, private_bytes) = rusage_memory_values(resident_fallback, &rusage);
                io_read_total_bytes = rusage.disk_bytes_read;
                io_write_total_bytes = rusage.disk_bytes_written;
                quality.memory = Some(if rusage.resident_size > 0 {
                    MetricQualityInfo::new(MetricQuality::Native, MetricSource::Libproc)
                } else if task_ok {
                    MetricQualityInfo::new(MetricQuality::Partial, MetricSource::Libproc)
                        .with_limitation(
                            MetricLimitationCode::UnsupportedMetric,
                            PHYSICAL_FOOTPRINT_UNAVAILABLE,
                        )
                } else {
                    MetricQualityInfo::new(MetricQuality::Unavailable, MetricSource::Libproc)
                        .with_limitation(
                            MetricLimitationCode::UnsupportedMetric,
                            "Resident memory is unavailable for this process.",
                        )
                });
                quality.io = Some(MetricQualityInfo::new(
                    MetricQuality::Native,
                    MetricSource::Libproc,
                ));
            }
            Err(failure) => {
                count_probe_failure(failure, &mut denied_probes, &mut unavailable_probes);
                let limitation = failure.limitation_code();
                memory_bytes = resident_fallback;
                quality.memory = Some(if task_ok {
                    MetricQualityInfo::new(MetricQuality::Partial, MetricSource::Libproc)
                        .with_limitation(limitation, PHYSICAL_FOOTPRINT_UNAVAILABLE)
                } else {
                    MetricQualityInfo::new(MetricQuality::Unavailable, MetricSource::Libproc)
                        .with_limitation(
                            limitation,
                            "Resident memory is unavailable for this process.",
                        )
                });
                quality.io = Some(unavailable_io_quality(limitation));
            }
        }

        match descriptors {
            Ok(count) => {
                successful_probes += 1;
                handles = count;
                quality.handles = Some(MetricQualityInfo::new(
                    MetricQuality::Native,
                    MetricSource::Libproc,
                ));
            }
            Err(failure) => {
                count_probe_failure(failure, &mut denied_probes, &mut unavailable_probes);
                quality.handles = Some(
                    MetricQualityInfo::new(MetricQuality::Unavailable, MetricSource::Libproc)
                        .with_limitation(
                            failure.limitation_code(),
                            "File-descriptor count is unavailable for this process.",
                        ),
                );
            }
        }

        quality.network = Some(
            MetricQualityInfo::new(MetricQuality::Unavailable, MetricSource::Libproc)
                .with_limitation(
                    MetricLimitationCode::UnsupportedMetric,
                    "Per-process network attribution is unavailable on macOS.",
                ),
        );
        quality.other_io = Some(
            MetricQualityInfo::new(MetricQuality::Unavailable, MetricSource::Libproc)
                .with_limitation(
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

        let exe = self
            .exe_cache
            .entry(generation)
            .or_insert_with(|| probes.executable_path(pid).unwrap_or_default())
            .clone();
        if exe.is_empty() {
            self.exe_cache.remove(&generation);
        }
        // The kernel table proves the process exists; a failed proc_pidpath only means
        // the executable path is unknown, so fall back to p_comm rather than dropping.
        let name = exe
            .rsplit('/')
            .next()
            .filter(|name| !name.is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| identity.comm());

        ProcessSample {
            pid: pid.to_string(),
            parent_pid: (identity.parent_pid != 0).then(|| identity.parent_pid.to_string()),
            start_time_ms: generation.1,
            name,
            exe,
            status: process_status(identity.status, thread),
            cpu_percent,
            kernel_cpu_percent: None,
            memory_bytes,
            private_bytes,
            virtual_memory_bytes,
            io_read_total_bytes,
            io_write_total_bytes,
            other_io_total_bytes: None,
            io_read_bps: 0,
            io_write_bps: 0,
            other_io_bps: None,
            network_received_bps: None,
            network_transmitted_bps: None,
            threads,
            handles,
            access_state: access,
            quality: Some(quality),
        }
    }

    fn mach_units_to_ns(&mut self, units: u64) -> u64 {
        let (numer, denom) = *self.timebase.get_or_insert_with(mach_timebase);
        let denom = denom.max(1);
        ((units as u128 * numer as u128) / denom as u128).min(u64::MAX as u128) as u64
    }
}

// Keeps an unverifiable-recheck process publishable with only its verified identity.
fn unverified_sample(
    identity: &ProcessIdentity,
    recheck_failure: ProcessProbeFailure,
) -> ProcessSample {
    let mut quality = ProcessMetricQuality::default();
    let limitation = recheck_failure.limitation_code();
    for slot in [
        &mut quality.cpu,
        &mut quality.memory,
        &mut quality.io,
        &mut quality.threads,
        &mut quality.handles,
    ] {
        *slot = Some(
            MetricQualityInfo::new(MetricQuality::Unavailable, MetricSource::Libproc)
                .with_limitation(
                    limitation,
                    "Process identity could not be re-verified after probing.",
                ),
        );
    }
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
    ProcessSample {
        pid: identity.pid.to_string(),
        parent_pid: (identity.parent_pid != 0).then(|| identity.parent_pid.to_string()),
        start_time_ms: identity.start_ms(),
        name: identity.comm(),
        exe: String::new(),
        status: process_status(identity.status, Err(ProcessProbeFailure::Failed(0))),
        cpu_percent: 0.0,
        kernel_cpu_percent: None,
        memory_bytes: 0,
        private_bytes: 0,
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
        quality: Some(quality),
    }
}

fn count_probe_failure(
    failure: ProcessProbeFailure,
    denied_probes: &mut u8,
    unavailable_probes: &mut u8,
) {
    if failure == ProcessProbeFailure::Denied {
        *denied_probes += 1;
    } else {
        *unavailable_probes += 1;
    }
}

// Mirrors sysinfo's `From<u32> for ProcessStatus` + Debug strings for p_stat/pbi_status.
fn bsd_status_label(status: u32) -> String {
    match status {
        libc::SIDL => "Idle".to_string(),
        libc::SRUN => "Run".to_string(),
        libc::SSLEEP => "Sleep".to_string(),
        libc::SSTOP => "Stop".to_string(),
        libc::SZOMB => "Zombie".to_string(),
        other => format!("Unknown({other})"),
    }
}

// Mirrors sysinfo's `ThreadStatus -> ProcessStatus` mapping and Debug strings.
fn thread_status_label(run_state: i32) -> String {
    match run_state {
        libc::TH_STATE_RUNNING => "Run".to_string(),
        libc::TH_STATE_STOPPED => "Stop".to_string(),
        libc::TH_STATE_WAITING => "Sleep".to_string(),
        libc::TH_STATE_UNINTERRUPTIBLE => "Dead".to_string(),
        libc::TH_STATE_HALTED => "Parked".to_string(),
        other => format!("Unknown({other})"),
    }
}

// sysinfo seeds status from p_stat and only overlays the thread run_state
// when the process is running.
fn process_status(p_stat: u32, thread: ProcessProbeResult<i32>) -> String {
    let base = bsd_status_label(p_stat);
    if base == "Run" {
        if let Ok(run_state) = thread {
            return thread_status_label(run_state);
        }
    }
    base
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

fn unavailable_io_quality(limitation: MetricLimitationCode) -> MetricQualityInfo {
    MetricQualityInfo::new(MetricQuality::Unavailable, MetricSource::Libproc).with_limitation(
        limitation,
        "Native process read/write totals are unavailable.",
    )
}

fn round1(value: f64) -> f64 {
    (value * 10.0).round() / 10.0
}

fn sysctl_raw(mib: &mut [c_int; 4], buffer: *mut c_void, size: &mut usize) -> io::Result<()> {
    let result = unsafe {
        libc::sysctl(
            mib.as_mut_ptr(),
            mib.len() as u32,
            buffer,
            size,
            std::ptr::null_mut(),
            0,
        )
    };
    if result == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

// sysctl(KERN_PROC, KERN_PROC_ALL) reads the kernel process table, which needs no
// per-process permission — the same source `ps` uses.
fn kernel_process_list() -> io::Result<Vec<ProcessIdentity>> {
    let mut mib = [libc::CTL_KERN, libc::KERN_PROC, libc::KERN_PROC_ALL, 0];
    let mut size = 0_usize;
    sysctl_raw(&mut mib, std::ptr::null_mut(), &mut size)?;
    let mut capacity = size.saturating_add(size / 4).max(size_of::<KinfoProc>());
    loop {
        let mut buffer = vec![0_u8; capacity];
        let mut written = capacity;
        match sysctl_raw(&mut mib, buffer.as_mut_ptr().cast(), &mut written) {
            Ok(()) => {
                let stride = size_of::<KinfoProc>();
                if !written.is_multiple_of(stride) {
                    return Err(io::Error::from_raw_os_error(libc::EINVAL));
                }
                buffer.truncate(written);
                return Ok(buffer
                    .chunks_exact(stride)
                    .map(|chunk| {
                        // The byte buffer carries no KinfoProc alignment guarantee.
                        let entry =
                            unsafe { std::ptr::read_unaligned(chunk.as_ptr().cast::<KinfoProc>()) };
                        ProcessIdentity::from(&entry)
                    })
                    .collect());
            }
            Err(error) if error.raw_os_error() == Some(libc::ENOMEM) => {
                capacity = capacity.saturating_mul(2);
                if capacity > (64 << 20) {
                    return Err(error);
                }
            }
            Err(error) => return Err(error),
        }
    }
}

// sysctl(KERN_PROC, KERN_PROC_PID, pid) is the per-pid recheck; it works for every
// user, unlike PROC_PIDTBSDINFO which is denied for other users' processes.
fn kernel_process_identity(pid: c_int) -> io::Result<ProcessIdentity> {
    let mut mib = [libc::CTL_KERN, libc::KERN_PROC, libc::KERN_PROC_PID, pid];
    let mut value = MaybeUninit::<KinfoProc>::zeroed();
    let mut size = size_of::<KinfoProc>();
    sysctl_raw(&mut mib, value.as_mut_ptr().cast(), &mut size)?;
    if size < size_of::<KinfoProc>() {
        return Err(io::Error::from_raw_os_error(libc::ESRCH));
    }
    let identity = ProcessIdentity::from(unsafe { &*value.as_ptr() });
    if identity.pid != pid as u32 || identity.seconds == 0 || identity.microseconds >= 1_000_000 {
        return Err(io::Error::from_raw_os_error(libc::ESRCH));
    }
    Ok(identity)
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
            value.as_mut_ptr().cast(),
            expected,
        )
    };
    if result == expected {
        Ok(unsafe { value.assume_init() })
    } else {
        Err(io::Error::last_os_error())
    }
}

fn thread_status(pid: c_int) -> io::Result<i32> {
    let mut value = MaybeUninit::<ProcPidThInfo>::zeroed();
    let expected = size_of::<ProcPidThInfo>() as c_int;
    let result = unsafe {
        proc_pidinfo(
            pid,
            PROC_PIDTHREADINFO,
            0,
            value.as_mut_ptr().cast(),
            expected,
        )
    };
    if result == expected {
        Ok(unsafe { value.assume_init() }.run_state)
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

fn executable_path(pid: c_int) -> io::Result<String> {
    let mut buffer = vec![0_u8; PROC_PIDPATHINFO_MAXSIZE as usize];
    let length = unsafe { proc_pidpath(pid, buffer.as_mut_ptr().cast(), PROC_PIDPATHINFO_MAXSIZE) };
    if length <= 0 {
        return Err(io::Error::last_os_error());
    }
    buffer.truncate(length as usize);
    Ok(String::from_utf8_lossy(&buffer).into_owned())
}

fn mach_timebase() -> (u64, u64) {
    let mut info = MachTimebaseInfo::default();
    unsafe {
        mach_timebase_info(&mut info);
    }
    (info.numer as u64, info.denom as u64)
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
    use std::cell::Cell;
    use std::time::Duration;

    fn identity(pid: u32) -> ProcessIdentity {
        let mut comm = [0_u8; MAXCOMLEN + 1];
        comm[..4].copy_from_slice(b"test");
        ProcessIdentity {
            pid,
            parent_pid: 1,
            status: libc::SRUN,
            comm,
            seconds: 1,
            microseconds: 123_000,
        }
    }

    #[derive(Clone)]
    struct FixtureProbes {
        identities: Vec<ProcessIdentity>,
        recheck: Option<ProcessProbeResult<ProcessIdentity>>,
        task: ProcessProbeResult<ProcTaskInfo>,
        rusage: ProcessProbeResult<RusageInfoV2>,
        thread: ProcessProbeResult<i32>,
        descriptor_count: ProcessProbeResult<u32>,
        exe: ProcessProbeResult<String>,
        exe_calls: Cell<usize>,
    }

    impl FixtureProbes {
        fn full() -> Self {
            Self {
                identities: vec![identity(42)],
                recheck: None,
                task: Ok(ProcTaskInfo {
                    virtual_size: 8_192,
                    resident_size: 2_048,
                    thread_count: 4,
                    ..ProcTaskInfo::default()
                }),
                rusage: Ok(RusageInfoV2 {
                    resident_size: 4_096,
                    physical_footprint: 3_072,
                    disk_bytes_read: 400,
                    disk_bytes_written: 200,
                    ..RusageInfoV2::default()
                }),
                thread: Ok(libc::TH_STATE_RUNNING),
                descriptor_count: Ok(12),
                exe: Ok("/usr/bin/test".to_string()),
                exe_calls: Cell::new(0),
            }
        }
    }

    impl MacosProcessProbes for FixtureProbes {
        fn list_processes(&self) -> ProcessProbeResult<Vec<ProcessIdentity>> {
            Ok(self.identities.clone())
        }

        fn identity(&self, pid: c_int) -> ProcessProbeResult<ProcessIdentity> {
            if let Some(recheck) = self.recheck {
                return recheck;
            }
            self.identities
                .iter()
                .find(|identity| identity.pid == pid as u32)
                .copied()
                .map(Ok)
                .unwrap_or(Err(ProcessProbeFailure::Exited))
        }

        fn task_info(&self, _pid: c_int) -> ProcessProbeResult<ProcTaskInfo> {
            self.task
        }

        fn rusage(&self, _pid: c_int) -> ProcessProbeResult<RusageInfoV2> {
            self.rusage
        }

        fn thread_status(&self, _pid: c_int) -> ProcessProbeResult<i32> {
            self.thread
        }

        fn file_descriptor_count(&self, _pid: c_int) -> ProcessProbeResult<u32> {
            self.descriptor_count
        }

        fn executable_path(&self, _pid: c_int) -> ProcessProbeResult<String> {
            self.exe_calls.set(self.exe_calls.get() + 1);
            self.exe.clone()
        }
    }

    fn collector() -> MacosProcessCollector {
        MacosProcessCollector {
            timebase: Some((125, 3)),
            ..MacosProcessCollector::new()
        }
    }

    fn clock_at(instant: Instant) -> impl Fn() -> Instant {
        move || instant
    }

    #[test]
    fn first_sample_holds_cpu_until_a_baseline_exists() {
        let mut collector = collector();
        let probes = FixtureProbes::full();
        let (processes, summary) =
            collector.collect_with_probes(&probes, 2_000, &clock_at(Instant::now()));
        assert_eq!(summary.exited_count, 0);
        assert_eq!(processes.len(), 1);
        let process = &processes[0];
        assert_eq!(process.cpu_percent, 0.0);
        assert_eq!(process.access_state, AccessState::Full);
        let cpu = process.quality.as_ref().unwrap().cpu.as_ref().unwrap();
        assert_eq!(cpu.quality, MetricQuality::Held);
        assert_eq!(
            cpu.limitation_code,
            Some(MetricLimitationCode::PendingBaseline)
        );
    }

    fn born_identity(pid: u32, seconds: u64, microseconds: u64) -> ProcessIdentity {
        ProcessIdentity {
            seconds,
            microseconds,
            ..identity(pid)
        }
    }

    #[test]
    fn process_born_between_collections_reports_lifetime_average_cpu() {
        let mut collector = collector();
        let mut probes = FixtureProbes::full();
        // First collection sees no processes but establishes the previous tick.
        probes.identities = Vec::new();
        collector.collect_with_probes(&probes, 10_000, &clock_at(Instant::now()));

        // Born at 10_500, sampled at 11_000: 500 ms old.
        // 6_000_000 mach units * (125/3) = 250_000_000 ns = 50% of one core over 500 ms.
        probes.identities = vec![born_identity(42, 10, 500_000)];
        probes.task = Ok(ProcTaskInfo {
            total_user: 6_000_000,
            thread_count: 4,
            ..ProcTaskInfo::default()
        });
        let (processes, _) =
            collector.collect_with_probes(&probes, 11_000, &clock_at(Instant::now()));
        assert_eq!(processes.len(), 1);
        assert_eq!(processes[0].cpu_percent, 50.0);
        let cpu = processes[0].quality.as_ref().unwrap().cpu.as_ref().unwrap();
        assert_eq!(cpu.quality, MetricQuality::Native);
        assert_eq!(cpu.source, Some(MetricSource::Libproc));
    }

    #[test]
    fn process_younger_than_250ms_stays_held() {
        let mut collector = collector();
        let mut probes = FixtureProbes::full();
        probes.identities = Vec::new();
        collector.collect_with_probes(&probes, 10_000, &clock_at(Instant::now()));

        // Born at 10_900, sampled at 11_000: only 100 ms old.
        probes.identities = vec![born_identity(42, 10, 900_000)];
        let (processes, _) =
            collector.collect_with_probes(&probes, 11_000, &clock_at(Instant::now()));
        assert_eq!(processes.len(), 1);
        assert_eq!(processes[0].cpu_percent, 0.0);
        let cpu = processes[0].quality.as_ref().unwrap().cpu.as_ref().unwrap();
        assert_eq!(cpu.quality, MetricQuality::Held);
        assert_eq!(
            cpu.limitation_code,
            Some(MetricLimitationCode::PendingBaseline)
        );
    }

    #[test]
    fn process_present_before_first_collection_stays_held() {
        let mut collector = collector();
        let probes = FixtureProbes::full();
        // Identity fixture starts at 1_123 ms, well before the first sample.
        let (processes, _) =
            collector.collect_with_probes(&probes, 10_000, &clock_at(Instant::now()));
        assert_eq!(processes.len(), 1);
        let cpu = processes[0].quality.as_ref().unwrap().cpu.as_ref().unwrap();
        assert_eq!(cpu.quality, MetricQuality::Held);
    }

    #[test]
    fn second_sample_reports_native_cpu_percent_from_mach_units() {
        let mut collector = collector();
        let mut probes = FixtureProbes::full();
        probes.task = Ok(ProcTaskInfo {
            total_user: 3_000,
            total_system: 3_000,
            thread_count: 4,
            ..ProcTaskInfo::default()
        });
        let t0 = Instant::now();
        collector.collect_with_probes(&probes, 2_000, &clock_at(t0));
        // +24_000_000 mach units * (125/3) ns/unit = 1_000_000_000 ns = 100% of one core.
        probes.task = Ok(ProcTaskInfo {
            total_user: 3_000 + 24_000_000,
            total_system: 3_000,
            thread_count: 4,
            ..ProcTaskInfo::default()
        });
        let (processes, _) =
            collector.collect_with_probes(&probes, 2_000, &clock_at(t0 + Duration::from_secs(1)));
        assert_eq!(processes[0].cpu_percent, 100.0);
        let cpu = processes[0].quality.as_ref().unwrap().cpu.as_ref().unwrap();
        assert_eq!(cpu.quality, MetricQuality::Native);
        assert_eq!(cpu.source, Some(MetricSource::Libproc));
    }

    #[test]
    fn pid_reuse_resets_the_cpu_baseline_and_resolves_exe_again() {
        let mut collector = collector();
        let mut probes = FixtureProbes::full();
        let t0 = Instant::now();
        collector.collect_with_probes(&probes, 2_000, &clock_at(t0));
        collector.collect_with_probes(&probes, 2_000, &clock_at(t0 + Duration::from_secs(1)));
        assert_eq!(probes.exe_calls.get(), 1);

        // Same pid, new generation (later start time still before the sample cutoff).
        probes.identities = vec![ProcessIdentity {
            microseconds: 500_000,
            ..identity(42)
        }];
        let (processes, _) =
            collector.collect_with_probes(&probes, 2_000, &clock_at(t0 + Duration::from_secs(2)));
        assert_eq!(probes.exe_calls.get(), 2);
        let cpu = processes[0].quality.as_ref().unwrap().cpu.as_ref().unwrap();
        assert_eq!(cpu.quality, MetricQuality::Held);
        assert_eq!(
            cpu.limitation_code,
            Some(MetricLimitationCode::PendingBaseline)
        );
    }

    #[test]
    fn exit_between_probes_drops_the_row() {
        let mut collector = collector();
        let mut probes = FixtureProbes::full();
        probes.rusage = Err(ProcessProbeFailure::Exited);
        let (processes, summary) =
            collector.collect_with_probes(&probes, 2_000, &clock_at(Instant::now()));
        assert_eq!(summary.exited_count, 1);
        assert_eq!(summary.denied_count, 0);
        assert!(processes.is_empty());
    }

    #[test]
    fn identity_mismatch_on_recheck_drops_the_row() {
        let mut collector = collector();
        let mut probes = FixtureProbes::full();
        probes.recheck = Some(Ok(ProcessIdentity {
            microseconds: 124_000,
            ..identity(42)
        }));
        let (processes, summary) =
            collector.collect_with_probes(&probes, 2_000, &clock_at(Instant::now()));
        assert_eq!(summary.exited_count, 1);
        assert!(processes.is_empty());
    }

    #[test]
    fn vanished_at_recheck_drops_the_row() {
        let mut collector = collector();
        let mut probes = FixtureProbes::full();
        probes.recheck = Some(Err(ProcessProbeFailure::Exited));
        let (processes, summary) =
            collector.collect_with_probes(&probes, 2_000, &clock_at(Instant::now()));
        assert_eq!(summary.exited_count, 1);
        assert!(processes.is_empty());
    }

    #[test]
    fn birth_after_sample_start_is_dropped() {
        let mut collector = collector();
        let probes = FixtureProbes::full(); // born at 1123 ms
        let (processes, summary) =
            collector.collect_with_probes(&probes, 1_122, &clock_at(Instant::now()));
        assert_eq!(summary.exited_count, 1);
        assert!(processes.is_empty());
    }

    #[test]
    fn denied_probes_mark_the_row_denied_without_publishable_io() {
        let mut collector = collector();
        let probes = FixtureProbes {
            task: Err(ProcessProbeFailure::Denied),
            rusage: Err(ProcessProbeFailure::Denied),
            descriptor_count: Err(ProcessProbeFailure::Denied),
            ..FixtureProbes::full()
        };
        let (processes, summary) =
            collector.collect_with_probes(&probes, 2_000, &clock_at(Instant::now()));
        assert_eq!(summary.denied_count, 1);
        assert_eq!(summary.exited_count, 0);
        assert_eq!(processes[0].access_state, AccessState::Denied);
        assert_eq!(processes[0].io_read_total_bytes, 0);
        // Identity still comes from the kernel table and the exe path still resolves.
        assert_eq!(processes[0].pid, "42");
        assert_eq!(processes[0].name, "test");
        assert_eq!(processes[0].exe, "/usr/bin/test");
        assert_eq!(processes[0].parent_pid, Some("1".to_string()));
        let io = processes[0]
            .quality
            .as_ref()
            .and_then(|quality| quality.io.as_ref())
            .expect("I/O quality");
        assert_eq!(io.quality, MetricQuality::Unavailable);
        assert_eq!(io.limitation_code, Some(MetricLimitationCode::AccessDenied));
    }

    #[test]
    fn mixed_denial_and_failure_is_partial_not_denied() {
        let mut collector = collector();
        let probes = FixtureProbes {
            task: Err(ProcessProbeFailure::Denied),
            rusage: Err(ProcessProbeFailure::Failed(libc::EIO)),
            descriptor_count: Err(ProcessProbeFailure::Unsupported),
            ..FixtureProbes::full()
        };
        let (processes, summary) =
            collector.collect_with_probes(&probes, 2_000, &clock_at(Instant::now()));
        assert_eq!(summary.denied_count, 0);
        assert_eq!(summary.partial_count, 1);
        assert_eq!(processes[0].access_state, AccessState::Partial);
    }

    #[test]
    fn pidpath_failure_falls_back_to_comm_without_dropping() {
        let mut collector = collector();
        let mut probes = FixtureProbes::full();
        probes.exe = Err(ProcessProbeFailure::Denied);
        let (processes, summary) =
            collector.collect_with_probes(&probes, 2_000, &clock_at(Instant::now()));
        assert_eq!(summary.exited_count, 0);
        assert_eq!(processes.len(), 1);
        assert_eq!(processes[0].name, "test");
        assert_eq!(processes[0].exe, "");
    }

    #[test]
    fn memory_is_unavailable_when_no_resident_source_succeeded() {
        let mut collector = collector();
        let probes = FixtureProbes {
            task: Err(ProcessProbeFailure::Denied),
            rusage: Err(ProcessProbeFailure::Denied),
            descriptor_count: Ok(12),
            ..FixtureProbes::full()
        };
        let (processes, _) =
            collector.collect_with_probes(&probes, 2_000, &clock_at(Instant::now()));
        let memory = processes[0]
            .quality
            .as_ref()
            .and_then(|quality| quality.memory.as_ref())
            .expect("memory quality");
        assert_eq!(memory.quality, MetricQuality::Unavailable);
        assert_eq!(
            memory.limitation_code,
            Some(MetricLimitationCode::AccessDenied)
        );
        assert_eq!(processes[0].memory_bytes, 0);
    }

    #[test]
    fn memory_falls_back_to_task_info_resident_as_partial() {
        let mut collector = collector();
        let probes = FixtureProbes {
            rusage: Err(ProcessProbeFailure::Denied),
            ..FixtureProbes::full()
        };
        let (processes, _) =
            collector.collect_with_probes(&probes, 2_000, &clock_at(Instant::now()));
        let memory = processes[0]
            .quality
            .as_ref()
            .and_then(|quality| quality.memory.as_ref())
            .expect("memory quality");
        assert_eq!(memory.quality, MetricQuality::Partial);
        assert_eq!(memory.source, Some(MetricSource::Libproc));
        assert_eq!(processes[0].memory_bytes, 2_048);
    }

    #[test]
    fn exe_path_yields_the_basename_and_is_cached_per_generation() {
        let mut collector = collector();
        let probes = FixtureProbes::full();
        let t0 = Instant::now();
        let (processes, _) = collector.collect_with_probes(&probes, 2_000, &clock_at(t0));
        assert_eq!(processes[0].name, "test");
        assert_eq!(processes[0].exe, "/usr/bin/test");
        collector.collect_with_probes(&probes, 2_000, &clock_at(t0 + Duration::from_secs(1)));
        assert_eq!(probes.exe_calls.get(), 1);
    }

    #[test]
    fn caches_evict_generations_that_vanished() {
        let mut collector = collector();
        let mut probes = FixtureProbes::full();
        collector.collect_with_probes(&probes, 2_000, &clock_at(Instant::now()));
        assert_eq!(collector.exe_cache.len(), 1);
        assert_eq!(collector.cpu_baselines.len(), 1);
        probes.identities = Vec::new();
        let (processes, _) =
            collector.collect_with_probes(&probes, 2_000, &clock_at(Instant::now()));
        assert!(processes.is_empty());
        assert!(collector.exe_cache.is_empty());
        assert!(collector.cpu_baselines.is_empty());
    }

    #[test]
    fn status_falls_back_to_kernel_table_when_thread_probe_fails() {
        let mut collector = collector();
        let mut probes = FixtureProbes::full();
        probes.thread = Err(ProcessProbeFailure::Denied);
        let (processes, _) =
            collector.collect_with_probes(&probes, 2_000, &clock_at(Instant::now()));
        assert_eq!(processes[0].status, "Run");
        // thread_status failure never degrades the access state.
        assert_eq!(processes[0].access_state, AccessState::Full);
    }

    #[test]
    fn running_process_maps_thread_run_state_like_sysinfo() {
        assert_eq!(
            process_status(libc::SRUN, Ok(libc::TH_STATE_WAITING)),
            "Sleep"
        );
        assert_eq!(
            process_status(libc::SRUN, Ok(libc::TH_STATE_UNINTERRUPTIBLE)),
            "Dead"
        );
        assert_eq!(
            process_status(libc::SRUN, Ok(libc::TH_STATE_HALTED)),
            "Parked"
        );
        assert_eq!(
            process_status(libc::SSLEEP, Ok(libc::TH_STATE_RUNNING)),
            "Sleep"
        );
        assert_eq!(
            process_status(libc::SZOMB, Err(ProcessProbeFailure::Denied)),
            "Zombie"
        );
        assert_eq!(bsd_status_label(9), "Unknown(9)");
    }

    #[test]
    fn failed_recheck_keeps_identity_but_marks_metrics_unavailable() {
        let mut collector = collector();
        let mut probes = FixtureProbes::full();
        probes.recheck = Some(Err(ProcessProbeFailure::Denied));
        let (processes, summary) =
            collector.collect_with_probes(&probes, 2_000, &clock_at(Instant::now()));
        assert_eq!(summary.partial_count, 1);
        assert_eq!(summary.exited_count, 0);
        assert_eq!(processes[0].access_state, AccessState::Partial);
        assert_eq!(processes[0].name, "test");
        assert_eq!(processes[0].memory_bytes, 0);
        let cpu = processes[0].quality.as_ref().unwrap().cpu.as_ref().unwrap();
        assert_eq!(cpu.quality, MetricQuality::Unavailable);
    }

    #[test]
    fn zero_physical_footprint_clears_the_fallback_value() {
        let rusage = RusageInfoV2 {
            resident_size: 4_096,
            physical_footprint: 0,
            ..RusageInfoV2::default()
        };
        let (memory, private) = rusage_memory_values(999, &rusage);
        assert_eq!(memory, 4_096);
        assert_eq!(private, 0);
    }

    #[test]
    fn kinfo_proc_layout_matches_the_sysctl_stride() {
        // KERN_PROC_PID on the current process returns exactly one kinfo_proc.
        let mut mib = [
            libc::CTL_KERN,
            libc::KERN_PROC,
            libc::KERN_PROC_PID,
            std::process::id() as c_int,
        ];
        let mut size = 0_usize;
        sysctl_raw(&mut mib, std::ptr::null_mut(), &mut size).unwrap();
        assert!(
            size.is_multiple_of(size_of::<KinfoProc>()),
            "sysctl stride mismatch"
        );
        let identity = kernel_process_identity(std::process::id() as c_int).unwrap();
        assert_eq!(identity.pid, std::process::id());
        assert!(identity.seconds > 0);
        assert!(!identity.comm().is_empty());
    }

    #[test]
    fn current_process_collects_native_metrics() {
        let mut collector = MacosProcessCollector::new();
        let (processes, _summary) = collector.collect(now_ms_for_test());
        let pid = std::process::id().to_string();
        let current = processes
            .iter()
            .find(|process| process.pid == pid)
            .expect("current process present");
        assert_eq!(current.access_state, AccessState::Full);
        assert!(current.memory_bytes > 0);
        assert!(current.private_bytes > 0);
        assert!(current.threads > 0);
        assert!(!current.name.is_empty());
        assert!(current.exe.starts_with('/'));
        assert!(current.start_time_ms > 0);
    }

    fn now_ms_for_test() -> u64 {
        crate::telemetry::now_ms()
    }

    #[test]
    fn process_exit_is_not_classified_as_access_denial() {
        let error = io::Error::from_raw_os_error(libc::ESRCH);
        assert!(is_process_exited(&error));
        assert!(!is_access_denied(&error));
        assert_eq!(classify_probe_error(error), ProcessProbeFailure::Exited);
    }

    #[test]
    fn missing_process_is_dropped_as_ordinary_churn() {
        let mut collector = MacosProcessCollector::new();
        let (processes, _summary) = collector.collect(crate::telemetry::now_ms());
        // The live collector lists the current process; churn pids never appear.
        let pid = (i32::MAX - 1).to_string();
        assert!(!processes.iter().any(|process| process.pid == pid));
        assert!(processes
            .iter()
            .any(|process| process.pid == std::process::id().to_string()));
    }

    #[test]
    #[ignore]
    fn live_collect_reports_accurate_cpu_and_identity() {
        use std::process::{Child, Command, Stdio};
        // Kill the busy child even when an assertion below panics.
        struct KillOnDrop(Child);
        impl Drop for KillOnDrop {
            fn drop(&mut self) {
                let _ = self.0.kill();
                let _ = self.0.wait();
            }
        }
        let child = KillOnDrop(
            Command::new("yes")
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .expect("spawn yes"),
        );
        let child_pid = child.0.id() as c_int;
        std::thread::sleep(Duration::from_millis(300));

        let mut collector = MacosProcessCollector::new();
        let _ = collector.collect(crate::telemetry::now_ms());
        std::thread::sleep(Duration::from_secs(1));
        let (processes, _summary) = collector.collect(crate::telemetry::now_ms());

        let yes = processes
            .iter()
            .find(|process| process.pid == child_pid.to_string())
            .expect("yes process listed");
        eprintln!("yes cpu_percent = {}", yes.cpu_percent);
        assert!(
            (80.0..=120.0).contains(&yes.cpu_percent),
            "yes cpu_percent {} outside 80..120",
            yes.cpu_percent
        );

        // Kernel-table enumeration covers every process `ps` sees, including
        // root-owned daemons whose proc_pidinfo is denied to us.
        let ps_count = String::from_utf8(
            Command::new("sh")
                .args(["-c", "ps -A | wc -l"])
                .output()
                .expect("ps")
                .stdout,
        )
        .unwrap()
        .trim()
        .parse::<usize>()
        .unwrap()
            - 1; // header line
        eprintln!("collected={} ps={ps_count}", processes.len());
        let ratio = processes.len() as f64 / ps_count as f64;
        assert!(
            (0.97..=1.03).contains(&ratio),
            "process count {} vs ps {}",
            processes.len(),
            ps_count
        );

        let launchd = processes
            .iter()
            .find(|process| process.pid == "1")
            .expect("launchd (pid 1) is collected");
        eprintln!("pid1 name={} status={}", launchd.name, launchd.status);

        let mut mismatches = Vec::new();
        for process in processes
            .iter()
            .step_by((processes.len() / 20).max(1))
            .take(20)
        {
            let comm = String::from_utf8(
                Command::new("ps")
                    .args(["-o", "comm=", "-p", &process.pid])
                    .output()
                    .expect("ps -p")
                    .stdout,
            )
            .unwrap();
            let basename = comm.trim().rsplit('/').next().unwrap_or("").to_string();
            if !basename.is_empty() && process.name != basename {
                mismatches.push(format!(
                    "pid {}: {} vs {}",
                    process.pid, process.name, basename
                ));
            }
        }
        // proc_pidpath names differ from `ps comm` for processes that rewrite argv[0]
        // (e.g. Electron helpers); sysinfo had the same divergence. Flag only if it is
        // widespread — the actual differences are listed in the output.
        eprintln!("name mismatches: {mismatches:?}");
        assert!(
            mismatches.len() <= 4,
            "too many name mismatches: {mismatches:?}"
        );
        drop(child);
    }
}
