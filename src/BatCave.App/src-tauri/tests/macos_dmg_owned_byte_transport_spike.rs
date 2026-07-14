//! Non-installing issue #140 probe for a Rust-owned DMG descriptor transport.
//!
//! This integration-test crate is deliberately not linked into the BatCave
//! runtime. It creates only a local fixture image and never installs or launches
//! an application, verifies package trust, or creates release evidence.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Disposition {
    Unsupported,
    Failed,
    AuthorityRejected,
    RetainedCleanupFailed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FailureBoundary {
    Authority,
    Consumption,
    Timeout,
    Detach,
    Cleanup,
}

#[derive(Debug, PartialEq, Eq)]
struct ProbeOutcome {
    disposition: Disposition,
    primary_boundary: FailureBoundary,
    hdiutil_started: bool,
    process_settled: bool,
    mount_created: bool,
    mount_residue: bool,
    temporary_residue: bool,
    descriptor_bytes_unchanged: bool,
    package_consumed: bool,
    app_installed: bool,
    app_launched: bool,
    trust_verified: bool,
    native_proven: bool,
    receipt_emitted: bool,
    evidence_emitted: bool,
}

impl ProbeOutcome {
    fn unsupported_host() -> Self {
        Self {
            disposition: Disposition::Unsupported,
            primary_boundary: FailureBoundary::Consumption,
            hdiutil_started: false,
            process_settled: true,
            mount_created: false,
            mount_residue: false,
            temporary_residue: false,
            descriptor_bytes_unchanged: false,
            package_consumed: false,
            app_installed: false,
            app_launched: false,
            trust_verified: false,
            native_proven: false,
            receipt_emitted: false,
            evidence_emitted: false,
        }
    }
}

fn assert_non_claims(outcome: &ProbeOutcome) {
    assert!(!outcome.package_consumed);
    assert!(!outcome.app_installed);
    assert!(!outcome.app_launched);
    assert!(!outcome.trust_verified);
    assert!(!outcome.native_proven);
    assert!(!outcome.receipt_emitted);
    assert!(!outcome.evidence_emitted);
}

#[cfg(not(target_os = "macos"))]
#[test]
fn unsupported_hosts_report_the_boundary_without_mutation() {
    let outcome = ProbeOutcome::unsupported_host();
    assert_eq!(outcome.disposition, Disposition::Unsupported);
    assert!(!outcome.hdiutil_started);
    assert!(outcome.process_settled);
    assert!(!outcome.mount_residue);
    assert!(!outcome.temporary_residue);
    assert_non_claims(&outcome);
}

#[test]
fn probe_has_no_production_or_javascript_entrypoint() {
    let manifest_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let production_lib =
        std::fs::read_to_string(manifest_root.join("src/lib.rs")).expect("read production library");
    assert!(!production_lib.contains("macos_dmg_owned_byte_transport_spike"));

    let repository_root = manifest_root
        .ancestors()
        .nth(3)
        .expect("manifest is nested below repository root");
    for script in [
        "scripts/macos-dmg-owned-byte-transport.mjs",
        "scripts/macos-dmg-owned-byte-transport.test.mjs",
    ] {
        assert!(!repository_root.join(script).exists());
    }
}

#[cfg(target_os = "macos")]
mod macos {
    use super::{assert_non_claims, Disposition, FailureBoundary, ProbeOutcome};
    use sha2::{Digest, Sha256};
    use std::fs::{self, DirBuilder, File, OpenOptions};
    use std::io::{self, Read, Seek, SeekFrom};
    use std::os::fd::{AsRawFd, RawFd};
    use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt};
    use std::os::unix::process::CommandExt;
    use std::path::{Path, PathBuf};
    use std::process::{Child, Command, ExitStatus, Stdio};
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::Mutex;
    use std::thread;
    use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

    const HDIUTIL: &str = "/usr/bin/hdiutil";
    const FIXTURE_MARKER: &str = "authority-marker.txt";
    const FIXTURE_CONTENTS: &[u8] = b"BatCave issue 140 descriptor fixture\n";
    const OUTPUT_LIMIT: usize = 4096;
    const BAD_DESCRIPTOR: &[u8] = b"Bad file descriptor";
    const NORMAL_TIMEOUT: Duration = Duration::from_secs(30);
    const TERMINATION_GRACE: Duration = Duration::from_secs(2);
    static ROOT_SEQUENCE: AtomicU64 = AtomicU64::new(0);
    static HDIUTIL_LOCK: Mutex<()> = Mutex::new(());

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum AuthorityState {
        Acquired,
        Consumed,
        Closed,
        RetainedCleanupFailed,
    }

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum FixtureKind {
        ValidDmg,
        InvalidBytes,
    }

    struct OwnedProcessResult {
        status: ExitStatus,
        timed_out: bool,
        settled: bool,
    }

    struct DescriptorAttachResult {
        process: OwnedProcessResult,
        bad_descriptor: Option<bool>,
    }

    struct AcquisitionRoot(Option<PathBuf>);

    impl AcquisitionRoot {
        fn new(path: PathBuf) -> Self {
            Self(Some(path))
        }

        fn path(&self) -> &Path {
            self.0.as_deref().expect("acquisition root retained")
        }

        fn transfer(mut self) -> PathBuf {
            self.0.take().expect("acquisition root retained")
        }
    }

    impl Drop for AcquisitionRoot {
        fn drop(&mut self) {
            if let Some(root) = self.0.take() {
                let _ = fs::remove_dir_all(root);
            }
        }
    }

    struct DmgTransportAuthority {
        state: AuthorityState,
        root: Option<PathBuf>,
        mount_point: PathBuf,
        source_path: PathBuf,
        source: Option<File>,
        expected_digest: [u8; 32],
        mount_created: bool,
        force_cleanup_failure: bool,
    }

    impl DmgTransportAuthority {
        fn valid() -> io::Result<Self> {
            Self::acquire(FixtureKind::ValidDmg, false)
        }

        fn invalid() -> io::Result<Self> {
            Self::acquire(FixtureKind::InvalidBytes, false)
        }

        fn cleanup_failure() -> io::Result<Self> {
            Self::acquire(FixtureKind::InvalidBytes, true)
        }

        fn acquire(kind: FixtureKind, force_cleanup_failure: bool) -> io::Result<Self> {
            let root = AcquisitionRoot::new(unique_private_root()?);
            let source_dir = root.path().join("fixture-source");
            let mount_point = root.path().join("mount");
            DirBuilder::new().mode(0o700).create(&source_dir)?;
            DirBuilder::new().mode(0o700).create(&mount_point)?;
            fs::write(source_dir.join(FIXTURE_MARKER), FIXTURE_CONTENTS)?;

            let source_path = root.path().join("fixture.dmg");
            match kind {
                FixtureKind::ValidDmg => create_fixture_dmg(&source_dir, &source_path)?,
                FixtureKind::InvalidBytes => write_private(&source_path, b"not a disk image\n")?,
            }

            let mut source = OpenOptions::new().read(true).open(&source_path)?;
            let expected_digest = digest_file(&mut source)?;
            source.seek(SeekFrom::Start(0))?;
            let root = root.transfer();

            Ok(Self {
                state: AuthorityState::Acquired,
                root: Some(root),
                mount_point,
                source_path,
                source: Some(source),
                expected_digest,
                mount_created: false,
                force_cleanup_failure,
            })
        }

        fn replace_source_path(&mut self) -> io::Result<()> {
            fs::remove_file(&self.source_path)?;
            write_private(&self.source_path, b"caller-visible replacement bytes\n")
        }

        fn consume(&mut self) -> ProbeOutcome {
            self.consume_with_timeout(NORMAL_TIMEOUT, true)
        }

        fn consume_with_timeout(&mut self, timeout: Duration, valid_fixture: bool) -> ProbeOutcome {
            if self.state != AuthorityState::Acquired {
                return self.rejected_outcome();
            }
            self.state = AuthorityState::Consumed;

            let descriptor_bytes_unchanged = self.descriptor_digest_matches();
            if !descriptor_bytes_unchanged {
                return self.finish_outcome(
                    Disposition::Failed,
                    FailureBoundary::Authority,
                    false,
                    true,
                    false,
                );
            }

            let raw_fd = self.source.as_ref().expect("acquired source").as_raw_fd();
            let process = attach_descriptor(
                raw_fd,
                &self.mount_point,
                self.root.as_ref().expect("retained root"),
                timeout,
            );

            let (disposition, primary_boundary, started, settled) = match process {
                Ok(result) if result.process.timed_out => (
                    Disposition::Failed,
                    FailureBoundary::Timeout,
                    true,
                    result.process.settled,
                ),
                Ok(result) if result.process.status.success() => {
                    self.mount_created = mount_is_active(&self.mount_point);
                    (
                        Disposition::Failed,
                        FailureBoundary::Consumption,
                        true,
                        result.process.settled,
                    )
                }
                Ok(result) if valid_fixture && result.bad_descriptor == Some(true) => (
                    Disposition::Unsupported,
                    FailureBoundary::Consumption,
                    true,
                    result.process.settled,
                ),
                Ok(result) => (
                    Disposition::Failed,
                    FailureBoundary::Consumption,
                    true,
                    result.process.settled,
                ),
                Err(_) => (
                    Disposition::Failed,
                    FailureBoundary::Consumption,
                    false,
                    true,
                ),
            };

            self.finish_outcome(
                disposition,
                primary_boundary,
                started,
                settled,
                descriptor_bytes_unchanged,
            )
        }

        fn probe_failed_detach(&mut self) -> ProbeOutcome {
            if self.state != AuthorityState::Acquired {
                return self.rejected_outcome();
            }
            self.state = AuthorityState::Consumed;
            let result = detach_mount(&self.mount_point, false, NORMAL_TIMEOUT);
            let settled = result.as_ref().map_or(true, |value| value.settled);
            let descriptor_bytes_unchanged = self.descriptor_digest_matches();
            self.finish_outcome(
                Disposition::Failed,
                FailureBoundary::Detach,
                result.is_ok(),
                settled,
                descriptor_bytes_unchanged,
            )
        }

        fn close(&mut self) -> ProbeOutcome {
            if self.state != AuthorityState::Acquired {
                return self.rejected_outcome();
            }
            self.state = AuthorityState::Closed;
            self.source.take();
            let temporary_residue = self.cleanup_root().is_err();
            ProbeOutcome {
                disposition: Disposition::AuthorityRejected,
                primary_boundary: FailureBoundary::Authority,
                hdiutil_started: false,
                process_settled: true,
                mount_created: false,
                mount_residue: false,
                temporary_residue,
                descriptor_bytes_unchanged: true,
                package_consumed: false,
                app_installed: false,
                app_launched: false,
                trust_verified: false,
                native_proven: false,
                receipt_emitted: false,
                evidence_emitted: false,
            }
        }

        fn retry_cleanup(&mut self) -> io::Result<()> {
            self.force_cleanup_failure = false;
            self.source.take();
            self.cleanup_root()?;
            self.state = AuthorityState::Closed;
            Ok(())
        }

        fn has_temporary_residue(&self) -> bool {
            self.root.as_ref().is_some_and(|root| root.exists())
        }

        fn has_mount_residue(&self) -> bool {
            mount_is_active(&self.mount_point)
        }

        fn descriptor_digest_matches(&mut self) -> bool {
            let Some(source) = self.source.as_mut() else {
                return false;
            };
            digest_file(source).is_ok_and(|digest| digest == self.expected_digest)
                && source.seek(SeekFrom::Start(0)).is_ok()
        }

        fn finish_outcome(
            &mut self,
            mut disposition: Disposition,
            mut primary_boundary: FailureBoundary,
            hdiutil_started: bool,
            process_settled: bool,
            descriptor_bytes_unchanged: bool,
        ) -> ProbeOutcome {
            if self.mount_created || mount_is_active(&self.mount_point) {
                self.mount_created = true;
                let detached = detach_mount(&self.mount_point, false, NORMAL_TIMEOUT)
                    .is_ok_and(|result| result.status.success() && result.settled);
                if !detached {
                    let _ = detach_mount(&self.mount_point, true, NORMAL_TIMEOUT);
                    disposition = Disposition::Failed;
                    primary_boundary = FailureBoundary::Detach;
                }
            }

            self.source.take();
            let cleanup_failed = self.cleanup_root().is_err();
            if cleanup_failed {
                self.state = AuthorityState::RetainedCleanupFailed;
                disposition = Disposition::RetainedCleanupFailed;
                primary_boundary = FailureBoundary::Cleanup;
            }

            ProbeOutcome {
                disposition,
                primary_boundary,
                hdiutil_started,
                process_settled,
                mount_created: self.mount_created,
                mount_residue: self.has_mount_residue(),
                temporary_residue: self.has_temporary_residue(),
                descriptor_bytes_unchanged,
                package_consumed: false,
                app_installed: false,
                app_launched: false,
                trust_verified: false,
                native_proven: false,
                receipt_emitted: false,
                evidence_emitted: false,
            }
        }

        fn rejected_outcome(&self) -> ProbeOutcome {
            ProbeOutcome {
                disposition: Disposition::AuthorityRejected,
                primary_boundary: FailureBoundary::Authority,
                hdiutil_started: false,
                process_settled: true,
                mount_created: false,
                mount_residue: self.has_mount_residue(),
                temporary_residue: self.has_temporary_residue(),
                descriptor_bytes_unchanged: false,
                package_consumed: false,
                app_installed: false,
                app_launched: false,
                trust_verified: false,
                native_proven: false,
                receipt_emitted: false,
                evidence_emitted: false,
            }
        }

        fn cleanup_root(&mut self) -> io::Result<()> {
            let Some(root) = self.root.as_ref() else {
                return Ok(());
            };
            if self.force_cleanup_failure {
                fs::remove_dir(root)?;
            } else {
                fs::remove_dir_all(root)?;
                self.root.take();
            }
            Ok(())
        }
    }

    impl Drop for DmgTransportAuthority {
        fn drop(&mut self) {
            if mount_is_active(&self.mount_point) {
                let _ = detach_mount(&self.mount_point, true, Duration::from_secs(5));
            }
            self.force_cleanup_failure = false;
            self.source.take();
            let _ = self.cleanup_root();
        }
    }

    fn unique_private_root() -> io::Result<PathBuf> {
        for _ in 0..32 {
            let sequence = ROOT_SEQUENCE.fetch_add(1, Ordering::Relaxed);
            let nanos = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos();
            let root = std::env::temp_dir().join(format!(
                "batcave-dmg-transport-{}-{nanos}-{sequence}",
                std::process::id()
            ));
            match DirBuilder::new().mode(0o700).create(&root) {
                Ok(()) => return Ok(root),
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(error),
            }
        }
        Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "could not create a unique private fixture root",
        ))
    }

    fn write_private(path: &Path, bytes: &[u8]) -> io::Result<()> {
        let mut options = OpenOptions::new();
        options.write(true).create_new(true).mode(0o600);
        use std::io::Write as _;
        options.open(path)?.write_all(bytes)
    }

    fn digest_file(file: &mut File) -> io::Result<[u8; 32]> {
        file.seek(SeekFrom::Start(0))?;
        let mut hasher = Sha256::new();
        let mut buffer = [0_u8; 16 * 1024];
        loop {
            let read = file.read(&mut buffer)?;
            if read == 0 {
                break;
            }
            hasher.update(&buffer[..read]);
        }
        Ok(hasher.finalize().into())
    }

    fn create_fixture_dmg(source_dir: &Path, output: &Path) -> io::Result<()> {
        let mut command = Command::new(HDIUTIL);
        command
            .args(["create", "-quiet", "-ov", "-format", "UDRO", "-srcfolder"])
            .arg(source_dir)
            .arg(output)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        let result = supervise(command, None, NORMAL_TIMEOUT)?;
        if result.status.success() && result.settled {
            Ok(())
        } else {
            Err(io::Error::other("fixed fixture DMG creation failed"))
        }
    }

    fn attach_descriptor(
        raw_fd: RawFd,
        mount_point: &Path,
        root: &Path,
        timeout: Duration,
    ) -> io::Result<DescriptorAttachResult> {
        let stdout = private_output(root.join("attach.stdout"))?;
        let stderr = private_output(root.join("attach.stderr"))?;
        let mut command = Command::new(HDIUTIL);
        command
            .args(["attach", "-nobrowse", "-readonly", "-mountpoint"])
            .arg(mount_point)
            .arg(format!("/dev/fd/{raw_fd}"))
            .stdin(Stdio::null())
            .stdout(Stdio::from(stdout))
            .stderr(Stdio::from(stderr));
        let result = supervise(command, Some(raw_fd), timeout)?;
        let stdout = read_bounded(&root.join("attach.stdout"));
        let stderr = read_bounded(&root.join("attach.stderr"));
        let bad_descriptor = match (stdout, stderr) {
            (Ok(_), Ok(stderr)) => Some(contains_bytes(&stderr, BAD_DESCRIPTOR)),
            _ => None,
        };
        Ok(DescriptorAttachResult {
            process: result,
            bad_descriptor,
        })
    }

    fn contains_bytes(haystack: &[u8], needle: &[u8]) -> bool {
        haystack
            .windows(needle.len())
            .any(|window| window == needle)
    }

    fn detach_mount(
        mount_point: &Path,
        force: bool,
        timeout: Duration,
    ) -> io::Result<OwnedProcessResult> {
        let mut command = Command::new(HDIUTIL);
        command.arg("detach").arg(mount_point).arg("-quiet");
        if force {
            command.arg("-force");
        }
        command
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        supervise(command, None, timeout)
    }

    fn private_output(path: PathBuf) -> io::Result<File> {
        OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(path)
    }

    fn read_bounded(path: &Path) -> io::Result<Vec<u8>> {
        let file = File::open(path)?;
        let mut output = Vec::new();
        file.take((OUTPUT_LIMIT + 1) as u64)
            .read_to_end(&mut output)?;
        if output.len() > OUTPUT_LIMIT {
            return Err(io::Error::other("hdiutil output exceeded the spike limit"));
        }
        Ok(output)
    }

    fn supervise(
        mut command: Command,
        inherited_fd: Option<RawFd>,
        timeout: Duration,
    ) -> io::Result<OwnedProcessResult> {
        let _hdiutil = HDIUTIL_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        unsafe {
            command.pre_exec(move || {
                if libc::setpgid(0, 0) != 0 {
                    return Err(io::Error::last_os_error());
                }
                if let Some(raw_fd) = inherited_fd {
                    let flags = libc::fcntl(raw_fd, libc::F_GETFD);
                    if flags < 0
                        || libc::fcntl(raw_fd, libc::F_SETFD, flags & !libc::FD_CLOEXEC) < 0
                    {
                        return Err(io::Error::last_os_error());
                    }
                }
                Ok(())
            });
        }

        let mut child = command.spawn()?;
        let process_group = child.id() as i32;
        let deadline = Instant::now() + timeout;
        let mut timed_out = timeout.is_zero();

        let status = loop {
            if timed_out || Instant::now() >= deadline {
                timed_out = true;
                terminate_process_group(&mut child, process_group)?;
                break child.wait()?;
            }
            if let Some(status) = child.try_wait()? {
                if !process_group_settled(process_group) {
                    terminate_process_group(&mut child, process_group)?;
                }
                break status;
            }
            thread::sleep(Duration::from_millis(10));
        };

        Ok(OwnedProcessResult {
            status,
            timed_out,
            settled: process_group_settled(process_group),
        })
    }

    fn terminate_process_group(child: &mut Child, process_group: i32) -> io::Result<()> {
        signal_process_group(process_group, libc::SIGTERM)?;
        let deadline = Instant::now() + TERMINATION_GRACE;
        while Instant::now() < deadline {
            if child.try_wait()?.is_some() && process_group_settled(process_group) {
                return Ok(());
            }
            thread::sleep(Duration::from_millis(10));
        }
        signal_process_group(process_group, libc::SIGKILL)?;
        let _ = child.wait()?;
        Ok(())
    }

    fn signal_process_group(process_group: i32, signal: i32) -> io::Result<()> {
        let result = unsafe { libc::kill(-process_group, signal) };
        if result == 0 {
            return Ok(());
        }
        let error = io::Error::last_os_error();
        if error.raw_os_error() == Some(libc::ESRCH) {
            Ok(())
        } else {
            Err(error)
        }
    }

    fn process_group_settled(process_group: i32) -> bool {
        let result = unsafe { libc::kill(-process_group, 0) };
        if result == 0 {
            return false;
        }
        io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH)
    }

    fn mount_is_active(mount_point: &Path) -> bool {
        let Some(parent) = mount_point.parent() else {
            return false;
        };
        let Ok(parent_metadata) = fs::metadata(parent) else {
            return false;
        };
        let Ok(mount_metadata) = fs::metadata(mount_point) else {
            return false;
        };
        parent_metadata.dev() != mount_metadata.dev()
    }

    #[test]
    fn valid_fixture_descriptor_transport_is_unsupported_without_residue() {
        let mut authority = DmgTransportAuthority::valid().expect("create valid fixture authority");
        authority
            .replace_source_path()
            .expect("replace caller-visible fixture path");

        let outcome = authority.consume();
        assert_eq!(outcome.disposition, Disposition::Unsupported);
        assert_eq!(outcome.primary_boundary, FailureBoundary::Consumption);
        assert!(outcome.hdiutil_started);
        assert!(outcome.process_settled);
        assert!(outcome.descriptor_bytes_unchanged);
        assert!(!outcome.mount_created);
        assert!(!outcome.mount_residue);
        assert!(!outcome.temporary_residue);
        assert_non_claims(&outcome);

        let replay = authority.consume();
        assert_eq!(replay.disposition, Disposition::AuthorityRejected);
        assert!(!replay.hdiutil_started);
        assert_non_claims(&replay);
    }

    #[test]
    fn early_close_rejects_consumption_without_starting_hdiutil() {
        let mut authority = DmgTransportAuthority::invalid().expect("create authority");
        let close = authority.close();
        assert_eq!(close.disposition, Disposition::AuthorityRejected);
        assert!(!close.temporary_residue);

        let outcome = authority.consume();
        assert_eq!(outcome.disposition, Disposition::AuthorityRejected);
        assert!(!outcome.hdiutil_started);
        assert!(!outcome.mount_residue);
        assert!(!outcome.temporary_residue);
        assert_non_claims(&outcome);
    }

    #[test]
    fn timeout_terminates_and_settles_the_owned_hdiutil_group() {
        let mut authority = DmgTransportAuthority::valid().expect("create timeout authority");
        let outcome = authority.consume_with_timeout(Duration::ZERO, true);
        assert_eq!(outcome.disposition, Disposition::Failed);
        assert_eq!(outcome.primary_boundary, FailureBoundary::Timeout);
        assert!(outcome.hdiutil_started);
        assert!(outcome.process_settled);
        assert!(!outcome.mount_residue);
        assert!(!outcome.temporary_residue);
        assert_non_claims(&outcome);
    }

    #[test]
    fn known_invalid_fixture_attach_and_failed_detach_remain_distinct_and_settled() {
        let mut invalid = DmgTransportAuthority::invalid().expect("create invalid authority");
        let attach = invalid.consume_with_timeout(NORMAL_TIMEOUT, false);
        assert_eq!(attach.disposition, Disposition::Failed);
        assert_eq!(attach.primary_boundary, FailureBoundary::Consumption);
        assert!(attach.hdiutil_started);
        assert!(attach.process_settled);
        assert!(!attach.mount_residue);
        assert!(!attach.temporary_residue);
        assert_non_claims(&attach);

        let mut detach = DmgTransportAuthority::invalid().expect("create detach authority");
        let detached = detach.probe_failed_detach();
        assert_eq!(detached.disposition, Disposition::Failed);
        assert_eq!(detached.primary_boundary, FailureBoundary::Detach);
        assert!(detached.hdiutil_started);
        assert!(detached.process_settled);
        assert!(!detached.mount_residue);
        assert!(!detached.temporary_residue);
        assert_non_claims(&detached);
    }

    #[test]
    fn cleanup_failure_is_retained_and_retry_removes_all_residue() {
        let mut authority =
            DmgTransportAuthority::cleanup_failure().expect("create cleanup authority");
        let outcome = authority.consume_with_timeout(NORMAL_TIMEOUT, false);
        assert_eq!(outcome.disposition, Disposition::RetainedCleanupFailed);
        assert_eq!(outcome.primary_boundary, FailureBoundary::Cleanup);
        assert!(outcome.temporary_residue);
        assert!(!outcome.mount_residue);
        assert_non_claims(&outcome);

        authority.retry_cleanup().expect("retry retained cleanup");
        assert!(!authority.has_temporary_residue());
        assert!(!authority.has_mount_residue());
    }

    #[test]
    fn probe_result_has_no_path_descriptor_receipt_or_evidence_surface() {
        let rendered = format!("{:?}", ProbeOutcome::unsupported_host());
        for forbidden in [
            "/dev/fd",
            "batcave-dmg-transport",
            "RawFd",
            "receipt_id",
            "evidence_packet",
        ] {
            assert!(!rendered.contains(forbidden));
        }
        let consume: fn(&mut DmgTransportAuthority) -> ProbeOutcome =
            DmgTransportAuthority::consume;
        let mut authority = DmgTransportAuthority::invalid().expect("create closed authority");
        let outcome = consume(&mut authority);
        assert_non_claims(&outcome);
    }
}
