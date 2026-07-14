#[cfg(target_os = "linux")]
use sha2::{Digest, Sha256};

#[cfg(target_os = "linux")]
const FIXED_PAYLOAD: &[u8] = b"BatCave Linux owned-descriptor transport spike bytes\n";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum HostSupport {
    LinuxSpikeAvailable,
    UnsupportedHost,
}

fn host_support() -> HostSupport {
    if cfg!(target_os = "linux") {
        HostSupport::LinuxSpikeAvailable
    } else {
        HostSupport::UnsupportedHost
    }
}

#[test]
fn unsupported_hosts_fail_explicitly_without_a_fallback() {
    if cfg!(target_os = "linux") {
        assert_eq!(host_support(), HostSupport::LinuxSpikeAvailable);
    } else {
        assert_eq!(host_support(), HostSupport::UnsupportedHost);
    }
}

#[cfg(target_os = "linux")]
mod linux {
    use super::*;
    use std::ffi::CString;
    use std::fs::{self, File, OpenOptions};
    use std::io::{Read, Seek, SeekFrom, Write};
    use std::os::fd::{AsRawFd, FromRawFd, RawFd};
    use std::os::unix::fs::OpenOptionsExt;
    use std::os::unix::process::CommandExt;
    use std::path::{Path, PathBuf};
    use std::process::{Child, Command, ExitStatus, Stdio};
    use std::sync::{Arc, Mutex, MutexGuard};
    use std::thread;
    use std::time::{Duration, Instant};

    const CONSUMER_FD: RawFd = 198;
    const CHILD_MARKER: &str = "BATCAVE_LINUX_TRANSPORT_SPIKE_CHILD";
    const ASSET_NAME: &str = "selected-artifact.bin";
    const NORMAL_TIMEOUT: Duration = Duration::from_secs(2);
    const HOSTILE_TIMEOUT: Duration = Duration::from_millis(40);
    const TERMINATION_GRACE: Duration = Duration::from_millis(200);
    const SETTLEMENT_TIMEOUT: Duration = Duration::from_secs(3);
    static TEST_LOCK: Mutex<()> = Mutex::new(());

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    pub(super) enum Transport {
        InheritedReadOnlyDescriptor,
        ChildPrivateProcFd,
    }

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    pub(super) enum FailureBoundary {
        Acquisition,
        Authority,
        Consumption,
        Timeout,
        Settlement,
        Cleanup,
    }

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    pub(super) enum Disposition {
        SyntheticConsumed,
        Failed,
    }

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    enum Phase {
        Acquired,
        Consuming,
        RetainedUnsettled,
        RetainedCleanupFailed,
        Closed,
    }

    #[derive(Clone, Copy)]
    enum ConsumerBehavior {
        ReadDirect,
        ReadThroughProcFd,
        SleepPastTimeout,
        SpawnSurvivingDescendant,
    }

    #[derive(Clone, Copy)]
    enum TestFault {
        None,
        SubstituteDescriptor,
        CleanupFailsOnce,
        SupervisorFailsAfterSpawn,
    }

    pub(super) struct ClosedPlan {
        transport: Transport,
        behavior: ConsumerBehavior,
        timeout: Duration,
        fault: TestFault,
        expected_size: usize,
        expected_sha256: [u8; 32],
    }

    pub(super) fn closed_plan(transport: Transport) -> ClosedPlan {
        ClosedPlan {
            transport,
            behavior: match transport {
                Transport::InheritedReadOnlyDescriptor => ConsumerBehavior::ReadDirect,
                Transport::ChildPrivateProcFd => ConsumerBehavior::ReadThroughProcFd,
            },
            timeout: NORMAL_TIMEOUT,
            fault: TestFault::None,
            expected_size: FIXED_PAYLOAD.len(),
            expected_sha256: digest(FIXED_PAYLOAD),
        }
    }

    pub(super) fn timeout_plan() -> ClosedPlan {
        ClosedPlan {
            behavior: ConsumerBehavior::SleepPastTimeout,
            timeout: HOSTILE_TIMEOUT,
            ..closed_plan(Transport::InheritedReadOnlyDescriptor)
        }
    }

    pub(super) fn descendant_plan() -> ClosedPlan {
        ClosedPlan {
            behavior: ConsumerBehavior::SpawnSurvivingDescendant,
            ..closed_plan(Transport::InheritedReadOnlyDescriptor)
        }
    }

    pub(super) fn substitution_plan() -> ClosedPlan {
        ClosedPlan {
            fault: TestFault::SubstituteDescriptor,
            ..closed_plan(Transport::InheritedReadOnlyDescriptor)
        }
    }

    pub(super) fn cleanup_failure_plan() -> ClosedPlan {
        ClosedPlan {
            fault: TestFault::CleanupFailsOnce,
            ..closed_plan(Transport::InheritedReadOnlyDescriptor)
        }
    }

    pub(super) fn supervisor_failure_plan() -> ClosedPlan {
        ClosedPlan {
            behavior: ConsumerBehavior::SleepPastTimeout,
            fault: TestFault::SupervisorFailsAfterSpawn,
            ..closed_plan(Transport::InheritedReadOnlyDescriptor)
        }
    }

    #[derive(Debug, Eq, PartialEq)]
    pub(super) struct SpikeError {
        boundary: FailureBoundary,
        message: &'static str,
    }

    impl SpikeError {
        pub(super) fn boundary(&self) -> FailureBoundary {
            self.boundary
        }

        pub(super) fn message(&self) -> &'static str {
            self.message
        }
    }

    #[derive(Debug)]
    pub(super) struct Outcome {
        disposition: Disposition,
        transport: Transport,
        failures: Vec<FailureBoundary>,
        observed_size: Option<usize>,
        observed_sha256: Option<String>,
        group_settled: bool,
        cleanup_completed: bool,
        residue_absent: bool,
        fixed_consumer_completed: bool,
        package_bytes_executed: bool,
        native_proven: bool,
    }

    impl Outcome {
        pub(super) fn disposition(&self) -> Disposition {
            self.disposition
        }

        pub(super) fn transport(&self) -> Transport {
            self.transport
        }

        pub(super) fn failures(&self) -> &[FailureBoundary] {
            &self.failures
        }

        pub(super) fn observed_size(&self) -> Option<usize> {
            self.observed_size
        }

        pub(super) fn observed_sha256(&self) -> Option<&str> {
            self.observed_sha256.as_deref()
        }

        pub(super) fn group_settled(&self) -> bool {
            self.group_settled
        }

        pub(super) fn cleanup_completed(&self) -> bool {
            self.cleanup_completed
        }

        pub(super) fn residue_absent(&self) -> bool {
            self.residue_absent
        }

        pub(super) fn fixed_consumer_completed(&self) -> bool {
            self.fixed_consumer_completed
        }

        pub(super) fn claims_proof(&self) -> bool {
            self.package_bytes_executed || self.native_proven
        }
    }

    struct CompletionSeal;

    pub(super) struct Completion {
        seal: Arc<CompletionSeal>,
        outcome: Outcome,
    }

    impl Completion {
        pub(super) fn outcome(&self) -> &Outcome {
            &self.outcome
        }
    }

    struct OwnedTransport {
        descriptor: File,
        inode: u64,
        device: u64,
        seals: i32,
    }

    pub(super) struct Authority {
        plan: ClosedPlan,
        owned: Option<OwnedTransport>,
        phase: Phase,
        seal: Arc<CompletionSeal>,
        cleanup_fails_once: bool,
        unsettled: Option<UnsettledProcessGroup>,
        retained_subreaper: Option<SubreaperGuard>,
    }

    pub(super) fn acquire(plan: ClosedPlan, verified_root: &Path) -> Result<Authority, SpikeError> {
        let root_metadata = fs::symlink_metadata(verified_root).map_err(|_| SpikeError {
            boundary: FailureBoundary::Acquisition,
            message: "verified root is unavailable",
        })?;
        if !root_metadata.is_dir() || root_metadata.file_type().is_symlink() {
            return Err(SpikeError {
                boundary: FailureBoundary::Acquisition,
                message: "verified root must be a non-link directory",
            });
        }

        let source_path = verified_root.join(ASSET_NAME);
        let source_metadata = fs::symlink_metadata(&source_path).map_err(|_| SpikeError {
            boundary: FailureBoundary::Acquisition,
            message: "selected artifact is unavailable",
        })?;
        if !source_metadata.is_file() || source_metadata.file_type().is_symlink() {
            return Err(SpikeError {
                boundary: FailureBoundary::Acquisition,
                message: "selected artifact must be a regular non-link file",
            });
        }

        let bytes = fs::read(&source_path).map_err(|_| SpikeError {
            boundary: FailureBoundary::Acquisition,
            message: "selected artifact could not be read",
        })?;
        if bytes.len() != plan.expected_size || digest(&bytes) != plan.expected_sha256 {
            return Err(SpikeError {
                boundary: FailureBoundary::Acquisition,
                message: "selected artifact does not match the closed binding",
            });
        }

        let owned = create_sealed_read_only_memfd(&bytes)?;
        Ok(Authority {
            cleanup_fails_once: matches!(plan.fault, TestFault::CleanupFailsOnce),
            plan,
            owned: Some(owned),
            phase: Phase::Acquired,
            seal: Arc::new(CompletionSeal),
            unsettled: None,
            retained_subreaper: None,
        })
    }

    impl Authority {
        pub(super) fn consume(&mut self) -> Result<Completion, SpikeError> {
            if self.phase != Phase::Acquired {
                return Err(SpikeError {
                    boundary: FailureBoundary::Authority,
                    message: "authority is not in the acquired phase",
                });
            }
            self.phase = Phase::Consuming;

            let subreaper = match SubreaperGuard::enable() {
                Ok(guard) => guard,
                Err(error) => {
                    self.owned.take();
                    self.phase = Phase::Closed;
                    return Err(error);
                }
            };
            let descriptor = self
                .owned
                .as_ref()
                .expect("acquired authority owns a descriptor");
            if let Err(error) = validate_owned_descriptor(descriptor) {
                self.owned.take();
                self.phase = Phase::Closed;
                return Err(error);
            }

            let decoy = if matches!(self.plan.fault, TestFault::SubstituteDescriptor) {
                match create_sealed_read_only_memfd(b"hostile substituted bytes") {
                    Ok(decoy) => Some(decoy),
                    Err(error) => {
                        self.owned.take();
                        self.phase = Phase::Closed;
                        return Err(error);
                    }
                }
            } else {
                None
            };
            let launch_descriptor = decoy.as_ref().unwrap_or(descriptor).descriptor.as_raw_fd();
            let launched = launch_fixed_consumer(
                launch_descriptor,
                self.plan.behavior,
                self.plan.timeout,
                matches!(self.plan.fault, TestFault::SupervisorFailsAfterSpawn),
            );

            let mut failures = Vec::new();
            let mut observed = None;
            let mut fixed_consumer_completed = false;
            let group_settled;

            match launched {
                Ok(mut result) => {
                    group_settled = result.group_settled;
                    if result.timed_out {
                        failures.push(FailureBoundary::Timeout);
                    } else if result.unexpected_descendant {
                        failures.push(FailureBoundary::Settlement);
                    } else if result
                        .status
                        .as_ref()
                        .is_some_and(|status| status.success())
                    {
                        fixed_consumer_completed = true;
                        observed = Some((self.plan.expected_size, self.plan.expected_sha256));
                    } else {
                        failures.push(FailureBoundary::Consumption);
                    }
                    if !result.group_settled && !failures.contains(&FailureBoundary::Settlement) {
                        failures.push(FailureBoundary::Settlement);
                    }
                    self.unsettled = result.unsettled.take();
                }
                Err(mut error) => {
                    group_settled = error.unsettled.is_none();
                    failures.append(&mut error.failures);
                    self.unsettled = error.unsettled.take();
                }
            }

            if !group_settled {
                self.retained_subreaper = Some(subreaper);
                self.phase = Phase::RetainedUnsettled;
            } else if self.cleanup_fails_once {
                self.cleanup_fails_once = false;
                failures.push(FailureBoundary::Cleanup);
                self.phase = Phase::RetainedCleanupFailed;
            } else {
                self.owned.take();
                self.phase = Phase::Closed;
            }

            let cleanup_completed = self.phase == Phase::Closed;
            let successful = failures.is_empty() && fixed_consumer_completed && cleanup_completed;
            let outcome = Outcome {
                disposition: if successful {
                    Disposition::SyntheticConsumed
                } else {
                    Disposition::Failed
                },
                transport: self.plan.transport,
                failures,
                observed_size: observed.as_ref().map(|(size, _)| *size),
                observed_sha256: observed.map(|(_, sha256)| hex_digest(sha256)),
                group_settled,
                cleanup_completed,
                residue_absent: group_settled,
                fixed_consumer_completed,
                package_bytes_executed: false,
                native_proven: false,
            };
            Ok(Completion {
                seal: Arc::clone(&self.seal),
                outcome,
            })
        }

        pub(super) fn close(&mut self) -> Result<(), SpikeError> {
            match self.phase {
                Phase::Acquired | Phase::RetainedCleanupFailed => {
                    self.owned.take();
                    self.phase = Phase::Closed;
                    Ok(())
                }
                Phase::RetainedUnsettled => Err(SpikeError {
                    boundary: FailureBoundary::Settlement,
                    message: "unsettled process ownership prevents descriptor close",
                }),
                Phase::Consuming => Err(SpikeError {
                    boundary: FailureBoundary::Authority,
                    message: "authority cannot close during consumption",
                }),
                Phase::Closed => Err(SpikeError {
                    boundary: FailureBoundary::Authority,
                    message: "authority is already closed",
                }),
            }
        }

        pub(super) fn settle_retained(&mut self) -> Result<(), SpikeError> {
            if self.phase != Phase::RetainedUnsettled {
                return Err(SpikeError {
                    boundary: FailureBoundary::Authority,
                    message: "authority has no retained process ownership",
                });
            }
            let unsettled = self
                .unsettled
                .as_mut()
                .expect("retained phase owns the unresolved process group");
            terminate_process_group(unsettled.process_group, &mut unsettled.child).map_err(
                |_| SpikeError {
                    boundary: FailureBoundary::Settlement,
                    message: "retained process group termination failed",
                },
            )?;
            let settled = settle_process_group(unsettled.process_group, &mut unsettled.child)
                .map_err(|_| SpikeError {
                    boundary: FailureBoundary::Settlement,
                    message: "retained process group settlement failed",
                })?;
            if !settled {
                return Err(SpikeError {
                    boundary: FailureBoundary::Settlement,
                    message: "retained process group remains unsettled",
                });
            }

            self.unsettled.take();
            self.retained_subreaper.take();
            self.owned.take();
            self.phase = Phase::Closed;
            Ok(())
        }

        pub(super) fn accepts_completion(&self, completion: &Completion) -> bool {
            Arc::ptr_eq(&self.seal, &completion.seal)
        }

        pub(super) fn retains_descriptor(&self) -> bool {
            self.owned.is_some()
        }

        pub(super) fn retains_process_group(&self) -> bool {
            self.unsettled.is_some() && self.retained_subreaper.is_some()
        }
    }

    struct LaunchResult {
        status: Option<ExitStatus>,
        timed_out: bool,
        unexpected_descendant: bool,
        group_settled: bool,
        unsettled: Option<UnsettledProcessGroup>,
    }

    struct LaunchFailure {
        failures: Vec<FailureBoundary>,
        unsettled: Option<UnsettledProcessGroup>,
    }

    struct UnsettledProcessGroup {
        child: Child,
        process_group: libc::pid_t,
    }

    impl LaunchFailure {
        fn before_spawn() -> Self {
            Self {
                failures: vec![FailureBoundary::Consumption],
                unsettled: None,
            }
        }

        fn after_spawn(
            mut failures: Vec<FailureBoundary>,
            child: Child,
            process_group: libc::pid_t,
        ) -> Self {
            if !failures.contains(&FailureBoundary::Settlement) {
                failures.push(FailureBoundary::Settlement);
            }
            Self {
                failures,
                unsettled: Some(UnsettledProcessGroup {
                    child,
                    process_group,
                }),
            }
        }
    }

    fn launch_fixed_consumer(
        descriptor: RawFd,
        behavior: ConsumerBehavior,
        timeout: Duration,
        fail_supervisor_after_spawn: bool,
    ) -> Result<LaunchResult, LaunchFailure> {
        let test_name = match behavior {
            ConsumerBehavior::ReadDirect => "linux::fixed_direct_consumer_entry",
            ConsumerBehavior::ReadThroughProcFd => "linux::fixed_proc_fd_consumer_entry",
            ConsumerBehavior::SleepPastTimeout => "linux::fixed_timeout_consumer_entry",
            ConsumerBehavior::SpawnSurvivingDescendant => "linux::fixed_descendant_spawner_entry",
        };
        let executable = std::env::current_exe().map_err(|_| LaunchFailure::before_spawn())?;
        let mut command = Command::new(executable);
        command
            .arg("--exact")
            .arg(test_name)
            .arg("--nocapture")
            .env_clear()
            .env(CHILD_MARKER, "1")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        unsafe {
            command.pre_exec(move || {
                if libc::setpgid(0, 0) != 0 {
                    return Err(std::io::Error::last_os_error());
                }
                if libc::dup2(descriptor, CONSUMER_FD) < 0 {
                    return Err(std::io::Error::last_os_error());
                }
                let flags = libc::fcntl(CONSUMER_FD, libc::F_GETFD);
                if flags < 0
                    || libc::fcntl(CONSUMER_FD, libc::F_SETFD, flags & !libc::FD_CLOEXEC) < 0
                {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }

        let mut child = command.spawn().map_err(|_| LaunchFailure::before_spawn())?;
        let process_group = child.id() as libc::pid_t;
        if fail_supervisor_after_spawn {
            return Err(LaunchFailure::after_spawn(Vec::new(), child, process_group));
        }

        let status = match wait_for_child(&mut child, timeout) {
            Ok(status) => status,
            Err(_) => {
                return Err(LaunchFailure::after_spawn(Vec::new(), child, process_group));
            }
        };
        let timed_out = status.is_none();
        let unexpected_descendant = status.is_some() && process_group_exists(process_group);
        let mut observed_failures = Vec::new();
        if timed_out {
            observed_failures.push(FailureBoundary::Timeout);
        } else if unexpected_descendant {
            observed_failures.push(FailureBoundary::Settlement);
        } else if status.as_ref().is_some_and(|value| !value.success()) {
            observed_failures.push(FailureBoundary::Consumption);
        }

        if !observed_failures.is_empty() {
            if terminate_process_group(process_group, &mut child).is_err() {
                return Err(LaunchFailure::after_spawn(
                    observed_failures,
                    child,
                    process_group,
                ));
            }
        }
        let group_settled = match settle_process_group(process_group, &mut child) {
            Ok(settled) => settled,
            Err(_) => {
                return Err(LaunchFailure::after_spawn(
                    observed_failures,
                    child,
                    process_group,
                ));
            }
        };
        let unsettled = if group_settled {
            None
        } else {
            Some(UnsettledProcessGroup {
                child,
                process_group,
            })
        };
        Ok(LaunchResult {
            status,
            timed_out,
            unexpected_descendant,
            group_settled,
            unsettled,
        })
    }

    fn wait_for_child(child: &mut Child, timeout: Duration) -> std::io::Result<Option<ExitStatus>> {
        let deadline = Instant::now() + timeout;
        loop {
            if let Some(status) = child.try_wait()? {
                return Ok(Some(status));
            }
            if Instant::now() >= deadline {
                return Ok(None);
            }
            thread::sleep(Duration::from_millis(5));
        }
    }

    fn terminate_process_group(
        process_group: libc::pid_t,
        child: &mut Child,
    ) -> std::io::Result<()> {
        signal_process_group(process_group, libc::SIGTERM)?;
        let deadline = Instant::now() + TERMINATION_GRACE;
        while process_group_exists(process_group) && Instant::now() < deadline {
            let _ = child.try_wait()?;
            reap_process_group(process_group)?;
            thread::sleep(Duration::from_millis(5));
        }
        if process_group_exists(process_group) {
            signal_process_group(process_group, libc::SIGKILL)?;
        }
        Ok(())
    }

    fn settle_process_group(
        process_group: libc::pid_t,
        child: &mut Child,
    ) -> std::io::Result<bool> {
        let deadline = Instant::now() + SETTLEMENT_TIMEOUT;
        loop {
            let _ = child.try_wait()?;
            reap_process_group(process_group)?;
            if !process_group_exists(process_group) {
                return Ok(true);
            }
            if Instant::now() >= deadline {
                return Ok(false);
            }
            thread::sleep(Duration::from_millis(5));
        }
    }

    fn signal_process_group(process_group: libc::pid_t, signal: i32) -> std::io::Result<()> {
        let result = unsafe { libc::kill(-process_group, signal) };
        if result == 0 {
            return Ok(());
        }
        let error = std::io::Error::last_os_error();
        if error.raw_os_error() == Some(libc::ESRCH) {
            Ok(())
        } else {
            Err(error)
        }
    }

    fn process_group_exists(process_group: libc::pid_t) -> bool {
        let result = unsafe { libc::kill(-process_group, 0) };
        if result == 0 {
            return true;
        }
        std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
    }

    fn reap_process_group(process_group: libc::pid_t) -> std::io::Result<()> {
        loop {
            let mut status = 0;
            let result = unsafe { libc::waitpid(-process_group, &mut status, libc::WNOHANG) };
            if result > 0 {
                continue;
            }
            if result == 0 {
                return Ok(());
            }
            let error = std::io::Error::last_os_error();
            if error.raw_os_error() == Some(libc::ECHILD) {
                return Ok(());
            }
            return Err(error);
        }
    }

    fn create_sealed_read_only_memfd(bytes: &[u8]) -> Result<OwnedTransport, SpikeError> {
        let name = CString::new("batcave-linux-owned-transport")
            .expect("fixed memfd name contains no NUL");
        let raw_fd = unsafe {
            libc::memfd_create(name.as_ptr(), libc::MFD_CLOEXEC | libc::MFD_ALLOW_SEALING)
        };
        if raw_fd < 0 {
            return Err(SpikeError {
                boundary: FailureBoundary::Acquisition,
                message: "memfd_create is unavailable",
            });
        }

        let mut writable = unsafe { File::from_raw_fd(raw_fd) };
        writable.write_all(bytes).map_err(|_| SpikeError {
            boundary: FailureBoundary::Acquisition,
            message: "owned memory file could not be written",
        })?;
        writable.flush().map_err(|_| SpikeError {
            boundary: FailureBoundary::Acquisition,
            message: "owned memory file could not be flushed",
        })?;
        writable.seek(SeekFrom::Start(0)).map_err(|_| SpikeError {
            boundary: FailureBoundary::Acquisition,
            message: "owned memory file could not be rewound",
        })?;

        let required_seals =
            libc::F_SEAL_WRITE | libc::F_SEAL_GROW | libc::F_SEAL_SHRINK | libc::F_SEAL_SEAL;
        if unsafe { libc::fcntl(writable.as_raw_fd(), libc::F_ADD_SEALS, required_seals) } < 0 {
            return Err(SpikeError {
                boundary: FailureBoundary::Acquisition,
                message: "owned memory file could not be sealed",
            });
        }

        let proc_path = format!("/proc/self/fd/{}", writable.as_raw_fd());
        let descriptor = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_CLOEXEC)
            .open(proc_path)
            .map_err(|_| SpikeError {
                boundary: FailureBoundary::Acquisition,
                message: "read-only owned descriptor could not be reopened",
            })?;
        let (device, inode) =
            descriptor_identity(descriptor.as_raw_fd()).map_err(|_| SpikeError {
                boundary: FailureBoundary::Acquisition,
                message: "owned descriptor identity is unavailable",
            })?;
        let (writer_device, writer_inode) =
            descriptor_identity(writable.as_raw_fd()).map_err(|_| SpikeError {
                boundary: FailureBoundary::Acquisition,
                message: "memory file identity is unavailable",
            })?;
        if (device, inode) != (writer_device, writer_inode) {
            return Err(SpikeError {
                boundary: FailureBoundary::Acquisition,
                message: "read-only descriptor identity changed while reopening",
            });
        }

        let flags = unsafe { libc::fcntl(descriptor.as_raw_fd(), libc::F_GETFL) };
        let seals = unsafe { libc::fcntl(descriptor.as_raw_fd(), libc::F_GET_SEALS) };
        if flags < 0
            || flags & libc::O_ACCMODE != libc::O_RDONLY
            || seals & required_seals != required_seals
        {
            return Err(SpikeError {
                boundary: FailureBoundary::Acquisition,
                message: "owned descriptor is not read-only and fully sealed",
            });
        }
        drop(writable);

        Ok(OwnedTransport {
            descriptor,
            inode,
            device,
            seals,
        })
    }

    fn validate_owned_descriptor(owned: &OwnedTransport) -> Result<(), SpikeError> {
        let (device, inode) =
            descriptor_identity(owned.descriptor.as_raw_fd()).map_err(|_| SpikeError {
                boundary: FailureBoundary::Consumption,
                message: "owned descriptor identity is unavailable before consumption",
            })?;
        let seals = unsafe { libc::fcntl(owned.descriptor.as_raw_fd(), libc::F_GET_SEALS) };
        let required_seals =
            libc::F_SEAL_WRITE | libc::F_SEAL_GROW | libc::F_SEAL_SHRINK | libc::F_SEAL_SEAL;
        if (device, inode) != (owned.device, owned.inode)
            || seals != owned.seals
            || seals & required_seals != required_seals
        {
            return Err(SpikeError {
                boundary: FailureBoundary::Consumption,
                message: "owned descriptor identity or seals changed",
            });
        }
        Ok(())
    }

    fn descriptor_identity(descriptor: RawFd) -> std::io::Result<(u64, u64)> {
        let mut stat = std::mem::MaybeUninit::<libc::stat>::uninit();
        if unsafe { libc::fstat(descriptor, stat.as_mut_ptr()) } != 0 {
            return Err(std::io::Error::last_os_error());
        }
        let stat = unsafe { stat.assume_init() };
        Ok((stat.st_dev, stat.st_ino))
    }

    struct SubreaperGuard {
        previous: libc::c_int,
    }

    impl SubreaperGuard {
        fn enable() -> Result<Self, SpikeError> {
            let mut previous = 0;
            if unsafe { libc::prctl(libc::PR_GET_CHILD_SUBREAPER, &mut previous) } != 0
                || unsafe { libc::prctl(libc::PR_SET_CHILD_SUBREAPER, 1) } != 0
            {
                return Err(SpikeError {
                    boundary: FailureBoundary::Settlement,
                    message: "Linux child-subreaper ownership is unavailable",
                });
            }
            Ok(Self { previous })
        }
    }

    impl Drop for SubreaperGuard {
        fn drop(&mut self) {
            unsafe {
                libc::prctl(libc::PR_SET_CHILD_SUBREAPER, self.previous);
            }
        }
    }

    pub(super) fn serial_test_guard() -> MutexGuard<'static, ()> {
        TEST_LOCK.lock().expect("Linux spike test lock is healthy")
    }

    pub(super) fn scratch_root(name: &str, bytes: &[u8]) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "batcave-linux-transport-{}-{name}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir(&root).expect("create spike root");
        fs::write(root.join(ASSET_NAME), bytes).expect("write spike payload");
        root
    }

    pub(super) fn remove_root(root: &Path) {
        fs::remove_dir_all(root).expect("remove spike root");
    }

    pub(super) fn expected_digest() -> String {
        hex_digest(digest(FIXED_PAYLOAD))
    }

    fn digest(bytes: &[u8]) -> [u8; 32] {
        Sha256::digest(bytes).into()
    }

    fn hex_digest(digest: [u8; 32]) -> String {
        let mut output = String::with_capacity(64);
        for byte in digest {
            use std::fmt::Write as _;
            write!(output, "{byte:02x}").expect("writing to a String cannot fail");
        }
        output
    }

    fn child_mode() -> bool {
        std::env::var_os(CHILD_MARKER).is_some()
    }

    fn read_fixed_descriptor(mut descriptor: File) {
        let mut bytes = Vec::new();
        descriptor
            .read_to_end(&mut bytes)
            .expect("fixed consumer reads inherited descriptor");
        assert_eq!(bytes, FIXED_PAYLOAD);
    }

    #[test]
    fn fixed_direct_consumer_entry() {
        if !child_mode() {
            return;
        }
        let descriptor = unsafe { File::from_raw_fd(CONSUMER_FD) };
        read_fixed_descriptor(descriptor);
    }

    #[test]
    fn fixed_proc_fd_consumer_entry() {
        if !child_mode() {
            return;
        }
        let descriptor = File::open(format!("/proc/self/fd/{CONSUMER_FD}"))
            .expect("fixed consumer opens its inherited descriptor path");
        read_fixed_descriptor(descriptor);
    }

    #[test]
    fn fixed_timeout_consumer_entry() {
        if !child_mode() {
            return;
        }
        thread::sleep(Duration::from_secs(30));
    }

    #[test]
    fn fixed_descendant_spawner_entry() {
        if !child_mode() {
            return;
        }
        let mut descendant = Command::new(std::env::current_exe().expect("current test binary"));
        descendant
            .arg("--exact")
            .arg("linux::fixed_descendant_holder_entry")
            .arg("--nocapture")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        descendant.spawn().expect("spawn fixed descendant");
    }

    #[test]
    fn fixed_descendant_holder_entry() {
        if !child_mode() {
            return;
        }
        thread::sleep(Duration::from_secs(30));
    }
}

#[cfg(target_os = "linux")]
mod linux_tests {
    use super::linux::*;
    use super::FIXED_PAYLOAD;
    use std::fs;

    #[test]
    fn sealed_memfd_supports_both_fixed_consumer_views() {
        let _guard = serial_test_guard();
        for transport in [
            Transport::InheritedReadOnlyDescriptor,
            Transport::ChildPrivateProcFd,
        ] {
            let root = scratch_root(&format!("transport-{transport:?}"), FIXED_PAYLOAD);
            let mut authority = acquire(closed_plan(transport), &root).expect("acquire transport");
            let completion = authority.consume().expect("consume exact bytes");
            assert!(authority.accepts_completion(&completion));
            let outcome = completion.outcome();
            assert_eq!(outcome.disposition(), Disposition::SyntheticConsumed);
            assert_eq!(outcome.transport(), transport);
            assert!(outcome.failures().is_empty());
            assert_eq!(outcome.observed_size(), Some(FIXED_PAYLOAD.len()));
            assert_eq!(outcome.observed_sha256(), Some(expected_digest().as_str()));
            assert!(outcome.fixed_consumer_completed());
            assert!(outcome.group_settled());
            assert!(outcome.cleanup_completed());
            assert!(outcome.residue_absent());
            assert!(!outcome.claims_proof());
            assert!(!authority.retains_descriptor());
            remove_root(&root);
        }
    }

    #[test]
    fn source_replacement_cannot_change_owned_bytes_or_expose_private_identity() {
        let _guard = serial_test_guard();
        let root = scratch_root("source-replacement", FIXED_PAYLOAD);
        let source = root.join("selected-artifact.bin");
        let mut authority = acquire(closed_plan(Transport::InheritedReadOnlyDescriptor), &root)
            .expect("acquire transport");
        fs::rename(&source, root.join("selected-artifact.original")).expect("move source");
        fs::write(&source, b"hostile replacement bytes").expect("replace source");

        let completion = authority.consume().expect("consume owned bytes");
        assert_eq!(
            completion.outcome().disposition(),
            Disposition::SyntheticConsumed
        );
        let rendered = format!("{:?}", completion.outcome());
        assert!(!rendered.contains(root.to_string_lossy().as_ref()));
        assert!(!rendered.contains("/proc/self/fd/"));
        assert!(!rendered.contains("native_execution_receipt"));
        assert!(!rendered.contains("evidence_packet"));
        assert!(!completion.outcome().claims_proof());
        remove_root(&root);
    }

    #[test]
    fn substituted_descriptor_fails_consumption_and_settles() {
        let _guard = serial_test_guard();
        let root = scratch_root("descriptor-substitution", FIXED_PAYLOAD);
        let mut authority = acquire(substitution_plan(), &root).expect("acquire transport");
        let completion = authority.consume().expect("derive failed outcome");
        let outcome = completion.outcome();
        assert_eq!(outcome.disposition(), Disposition::Failed);
        assert_eq!(outcome.failures(), &[FailureBoundary::Consumption]);
        assert!(outcome.group_settled());
        assert!(outcome.cleanup_completed());
        assert!(outcome.residue_absent());
        assert!(!outcome.fixed_consumer_completed());
        assert!(!outcome.claims_proof());
        remove_root(&root);
    }

    #[test]
    fn replay_and_early_close_fail_before_launch() {
        let _guard = serial_test_guard();
        let root = scratch_root("replay", FIXED_PAYLOAD);
        let mut authority = acquire(closed_plan(Transport::InheritedReadOnlyDescriptor), &root)
            .expect("acquire transport");
        authority.consume().expect("first consume");
        let replay = authority.consume().err().expect("replay fails");
        assert_eq!(replay.boundary(), FailureBoundary::Authority);

        let mut closed = acquire(closed_plan(Transport::ChildPrivateProcFd), &root)
            .expect("acquire second transport");
        closed.close().expect("early close");
        let early_close = closed.consume().err().expect("closed authority fails");
        assert_eq!(early_close.boundary(), FailureBoundary::Authority);
        remove_root(&root);
    }

    #[test]
    fn timeout_terminates_reaps_and_cleans_without_proof() {
        let _guard = serial_test_guard();
        let root = scratch_root("timeout", FIXED_PAYLOAD);
        let mut authority = acquire(timeout_plan(), &root).expect("acquire transport");
        let completion = authority.consume().expect("derive timeout outcome");
        let outcome = completion.outcome();
        assert_eq!(outcome.disposition(), Disposition::Failed);
        assert_eq!(outcome.failures(), &[FailureBoundary::Timeout]);
        assert!(outcome.group_settled());
        assert!(outcome.cleanup_completed());
        assert!(outcome.residue_absent());
        assert!(!outcome.fixed_consumer_completed());
        assert!(!outcome.claims_proof());
        remove_root(&root);
    }

    #[test]
    fn surviving_descendant_is_detected_terminated_reaped_and_failed() {
        let _guard = serial_test_guard();
        let root = scratch_root("descendant", FIXED_PAYLOAD);
        let mut authority = acquire(descendant_plan(), &root).expect("acquire transport");
        let completion = authority.consume().expect("derive descendant outcome");
        let outcome = completion.outcome();
        assert_eq!(outcome.disposition(), Disposition::Failed);
        assert_eq!(outcome.failures(), &[FailureBoundary::Settlement]);
        assert!(outcome.group_settled());
        assert!(outcome.cleanup_completed());
        assert!(outcome.residue_absent());
        assert!(!outcome.fixed_consumer_completed());
        assert!(!outcome.claims_proof());
        remove_root(&root);
    }

    #[test]
    fn cleanup_failure_retains_descriptor_until_explicit_retry() {
        let _guard = serial_test_guard();
        let root = scratch_root("cleanup", FIXED_PAYLOAD);
        let mut authority = acquire(cleanup_failure_plan(), &root).expect("acquire transport");
        let completion = authority.consume().expect("derive cleanup outcome");
        let outcome = completion.outcome();
        assert_eq!(outcome.disposition(), Disposition::Failed);
        assert_eq!(outcome.failures(), &[FailureBoundary::Cleanup]);
        assert!(outcome.group_settled());
        assert!(!outcome.cleanup_completed());
        assert!(outcome.residue_absent());
        assert!(outcome.fixed_consumer_completed());
        assert!(!outcome.claims_proof());
        assert!(authority.retains_descriptor());
        authority.close().expect("retry cleanup");
        assert!(!authority.retains_descriptor());
        remove_root(&root);
    }

    #[test]
    fn post_spawn_supervisor_failure_retains_group_and_descriptor_until_settlement_retry() {
        let _guard = serial_test_guard();
        let root = scratch_root("supervisor-failure", FIXED_PAYLOAD);
        let mut authority = acquire(supervisor_failure_plan(), &root).expect("acquire transport");
        let completion = authority
            .consume()
            .expect("derive unresolved supervisor outcome");
        let outcome = completion.outcome();
        assert_eq!(outcome.disposition(), Disposition::Failed);
        assert_eq!(outcome.failures(), &[FailureBoundary::Settlement]);
        assert!(!outcome.group_settled());
        assert!(!outcome.cleanup_completed());
        assert!(!outcome.residue_absent());
        assert!(!outcome.fixed_consumer_completed());
        assert!(!outcome.claims_proof());
        assert!(authority.retains_descriptor());
        assert!(authority.retains_process_group());

        let close_error = authority.close().err().expect("early close fails");
        assert_eq!(close_error.boundary(), FailureBoundary::Settlement);
        assert!(authority.retains_descriptor());
        assert!(authority.retains_process_group());

        authority
            .settle_retained()
            .expect("terminate, reap, and close retained ownership");
        assert!(!authority.retains_descriptor());
        assert!(!authority.retains_process_group());
        remove_root(&root);
    }

    #[test]
    fn linked_and_mismatched_sources_fail_acquisition() {
        let _guard = serial_test_guard();
        let wrong_root = scratch_root("wrong", b"wrong bytes");
        let error = acquire(
            closed_plan(Transport::InheritedReadOnlyDescriptor),
            &wrong_root,
        )
        .err()
        .expect("wrong bytes fail");
        assert_eq!(error.boundary(), FailureBoundary::Acquisition);
        assert_eq!(
            error.message(),
            "selected artifact does not match the closed binding"
        );
        remove_root(&wrong_root);

        use std::os::unix::fs::symlink;
        let real_root = scratch_root("real", FIXED_PAYLOAD);
        let linked_root = real_root.with_extension("linked");
        let _ = fs::remove_file(&linked_root);
        symlink(&real_root, &linked_root).expect("link root");
        let error = acquire(closed_plan(Transport::ChildPrivateProcFd), &linked_root)
            .err()
            .expect("linked root fails");
        assert_eq!(error.boundary(), FailureBoundary::Acquisition);
        assert_eq!(
            error.message(),
            "verified root must be a non-link directory"
        );
        fs::remove_file(linked_root).expect("remove linked root");
        remove_root(&real_root);
    }
}
