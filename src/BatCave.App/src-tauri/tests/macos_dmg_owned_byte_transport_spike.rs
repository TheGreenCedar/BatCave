//! Non-installing issue #140 probe for a Rust-owned DMG descriptor transport.
//!
//! This integration-test crate is deliberately not linked into the BatCave
//! runtime. It creates only a local fixture image and never installs or launches
//! an application, verifies package trust, or creates release evidence.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Disposition {
    Unsupported,
    #[cfg(target_os = "macos")]
    Failed,
    #[cfg(target_os = "macos")]
    AuthorityRejected,
    #[cfg(target_os = "macos")]
    RetainedProcessUnsettled,
    #[cfg(target_os = "macos")]
    RetainedCleanupFailed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FailureBoundary {
    #[cfg(target_os = "macos")]
    Authority,
    Consumption,
    #[cfg(target_os = "macos")]
    Timeout,
    #[cfg(target_os = "macos")]
    Supervision,
    #[cfg(target_os = "macos")]
    Detach,
    #[cfg(target_os = "macos")]
    Cleanup,
}

#[derive(Debug, PartialEq, Eq)]
struct ProbeOutcome {
    disposition: Disposition,
    primary_boundary: FailureBoundary,
    retained_boundary: Option<FailureBoundary>,
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
            retained_boundary: None,
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
    assert_eq!(outcome.primary_boundary, FailureBoundary::Consumption);
    assert_eq!(outcome.retained_boundary, None);
    assert!(!outcome.hdiutil_started);
    assert!(outcome.process_settled);
    assert!(!outcome.mount_created);
    assert!(!outcome.mount_residue);
    assert!(!outcome.temporary_residue);
    assert!(!outcome.descriptor_bytes_unchanged);
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
    static RETAINED_RECOVERIES: Mutex<Vec<RetainedRecovery>> = Mutex::new(Vec::new());

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum AuthorityState {
        Acquired,
        Consumed,
        Closed,
        RetainedProcessUnsettled,
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

    struct UnsettledProcess {
        child: Child,
        process_group: i32,
        fail_settlement_once: bool,
    }

    enum SupervisionFailure {
        BeforeSpawn,
        AfterSpawn(UnsettledProcess),
    }

    struct DescriptorAttachResult {
        process: OwnedProcessResult,
        bad_descriptor: Option<bool>,
    }

    struct RetainedRecovery {
        process: Option<UnsettledProcess>,
        source: Option<File>,
        root: PathBuf,
        mount_point: PathBuf,
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
        unsettled_process: Option<UnsettledProcess>,
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
                FixtureKind::ValidDmg => match create_fixture_dmg(&source_dir, &source_path) {
                    Ok(result) if result.status.success() && result.settled => {}
                    Ok(_) | Err(SupervisionFailure::BeforeSpawn) => {
                        return Err(io::Error::other("fixed fixture DMG creation failed"));
                    }
                    Err(SupervisionFailure::AfterSpawn(mut process)) => {
                        if process.settle().unwrap_or(false) {
                            return Err(io::Error::other("fixed fixture DMG creation failed"));
                        }
                        retain_recovery(RetainedRecovery {
                            process: Some(process),
                            source: None,
                            root: root.transfer(),
                            mount_point: mount_point.clone(),
                        });
                        return Err(io::Error::other(
                            "fixture process unresolved; private root retained",
                        ));
                    }
                },
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
                unsettled_process: None,
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
            self.consume_with_supervision(timeout, valid_fixture, false)
        }

        fn consume_with_supervision(
            &mut self,
            timeout: Duration,
            valid_fixture: bool,
            fail_after_spawn: bool,
        ) -> ProbeOutcome {
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
                fail_after_spawn,
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
                Err(SupervisionFailure::BeforeSpawn) => (
                    Disposition::Failed,
                    FailureBoundary::Consumption,
                    false,
                    true,
                ),
                Err(SupervisionFailure::AfterSpawn(process)) => {
                    self.unsettled_process = Some(process);
                    (
                        Disposition::Failed,
                        FailureBoundary::Supervision,
                        true,
                        false,
                    )
                }
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
            let (primary_boundary, started, settled) = match result {
                Ok(result) => (FailureBoundary::Detach, true, result.settled),
                Err(SupervisionFailure::BeforeSpawn) => (FailureBoundary::Detach, false, true),
                Err(SupervisionFailure::AfterSpawn(process)) => {
                    self.unsettled_process = Some(process);
                    (FailureBoundary::Supervision, true, false)
                }
            };
            let descriptor_bytes_unchanged = self.descriptor_digest_matches();
            self.finish_outcome(
                Disposition::Failed,
                primary_boundary,
                started,
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
                retained_boundary: None,
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
            if let Some(mut process) = self.unsettled_process.take() {
                match process.settle() {
                    Ok(true) => {}
                    Ok(false) | Err(_) => {
                        self.unsettled_process = Some(process);
                        return Err(io::Error::other("owned hdiutil process remains unsettled"));
                    }
                }
            }
            if mount_is_active(&self.mount_point) {
                match detach_mount(&self.mount_point, true, NORMAL_TIMEOUT) {
                    Ok(result)
                        if result.status.success()
                            && result.settled
                            && !mount_is_active(&self.mount_point) => {}
                    Err(SupervisionFailure::AfterSpawn(process)) => {
                        self.unsettled_process = Some(process);
                        return Err(io::Error::other("detach supervision remains unsettled"));
                    }
                    _ => return Err(io::Error::other("owned mount remains attached")),
                }
            }
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
            primary_boundary: FailureBoundary,
            hdiutil_started: bool,
            process_settled: bool,
            descriptor_bytes_unchanged: bool,
        ) -> ProbeOutcome {
            let mut retained_boundary = None;
            if !process_settled || self.unsettled_process.is_some() {
                self.state = AuthorityState::RetainedProcessUnsettled;
                disposition = Disposition::RetainedProcessUnsettled;
                retained_boundary = Some(FailureBoundary::Supervision);
                return self.outcome(
                    disposition,
                    primary_boundary,
                    retained_boundary,
                    hdiutil_started,
                    false,
                    descriptor_bytes_unchanged,
                );
            }

            if self.mount_created || mount_is_active(&self.mount_point) {
                self.mount_created = true;
                let detached = self.detach_owned(false);
                if !detached {
                    retained_boundary = Some(FailureBoundary::Detach);
                    disposition = Disposition::Failed;
                    if self.unsettled_process.is_some() || !self.detach_owned(true) {
                        self.state = AuthorityState::RetainedCleanupFailed;
                        disposition = Disposition::RetainedCleanupFailed;
                        return self.outcome(
                            disposition,
                            primary_boundary,
                            retained_boundary,
                            hdiutil_started,
                            self.unsettled_process.is_none(),
                            descriptor_bytes_unchanged,
                        );
                    }
                }
            }

            self.source.take();
            let cleanup_failed = self.cleanup_root().is_err();
            if cleanup_failed {
                self.state = AuthorityState::RetainedCleanupFailed;
                disposition = Disposition::RetainedCleanupFailed;
                retained_boundary = Some(FailureBoundary::Cleanup);
            }

            self.outcome(
                disposition,
                primary_boundary,
                retained_boundary,
                hdiutil_started,
                process_settled,
                descriptor_bytes_unchanged,
            )
        }

        fn detach_owned(&mut self, force: bool) -> bool {
            match detach_mount(&self.mount_point, force, NORMAL_TIMEOUT) {
                Ok(result) => {
                    result.status.success() && result.settled && !mount_is_active(&self.mount_point)
                }
                Err(SupervisionFailure::AfterSpawn(process)) => {
                    self.unsettled_process = Some(process);
                    false
                }
                Err(SupervisionFailure::BeforeSpawn) => false,
            }
        }

        fn outcome(
            &self,
            disposition: Disposition,
            primary_boundary: FailureBoundary,
            retained_boundary: Option<FailureBoundary>,
            hdiutil_started: bool,
            process_settled: bool,
            descriptor_bytes_unchanged: bool,
        ) -> ProbeOutcome {
            ProbeOutcome {
                disposition,
                primary_boundary,
                retained_boundary,
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
                retained_boundary: None,
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

        fn retain_for_recovery(&mut self, process: Option<UnsettledProcess>) {
            if let Some(root) = self.root.take() {
                retain_recovery(RetainedRecovery {
                    process,
                    source: self.source.take(),
                    root,
                    mount_point: self.mount_point.clone(),
                });
            }
        }
    }

    impl Drop for DmgTransportAuthority {
        fn drop(&mut self) {
            if let Some(mut process) = self.unsettled_process.take() {
                if !process.settle().unwrap_or(false) {
                    self.retain_for_recovery(Some(process));
                    return;
                }
            }
            if mount_is_active(&self.mount_point) {
                match detach_mount(&self.mount_point, true, Duration::from_secs(5)) {
                    Ok(result)
                        if result.status.success()
                            && result.settled
                            && !mount_is_active(&self.mount_point) => {}
                    Err(SupervisionFailure::AfterSpawn(process)) => {
                        self.retain_for_recovery(Some(process));
                        return;
                    }
                    _ => {
                        self.retain_for_recovery(None);
                        return;
                    }
                }
            }
            self.force_cleanup_failure = false;
            self.source.take();
            if self.cleanup_root().is_err() {
                self.retain_for_recovery(None);
            }
        }
    }

    fn retain_recovery(recovery: RetainedRecovery) {
        RETAINED_RECOVERIES
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push(recovery);
    }

    fn retained_recovery_count() -> usize {
        RETAINED_RECOVERIES
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .len()
    }

    fn recover_retained() -> io::Result<()> {
        let recoveries = {
            let mut retained = RETAINED_RECOVERIES
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            std::mem::take(&mut *retained)
        };
        let mut unresolved = Vec::new();
        for mut recovery in recoveries {
            if !recovery.recover() {
                unresolved.push(recovery);
            }
        }
        let unresolved_count = unresolved.len();
        RETAINED_RECOVERIES
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .extend(unresolved);
        if unresolved_count == 0 {
            Ok(())
        } else {
            Err(io::Error::other("retained DMG recovery remains unresolved"))
        }
    }

    impl RetainedRecovery {
        fn recover(&mut self) -> bool {
            if let Some(mut process) = self.process.take() {
                if !process.settle().unwrap_or(false) {
                    self.process = Some(process);
                    return false;
                }
            }
            if mount_is_active(&self.mount_point) {
                match detach_mount(&self.mount_point, true, Duration::from_secs(5)) {
                    Ok(result)
                        if result.status.success()
                            && result.settled
                            && !mount_is_active(&self.mount_point) => {}
                    Err(SupervisionFailure::AfterSpawn(process)) => {
                        self.process = Some(process);
                        return false;
                    }
                    _ => return false,
                }
            }
            self.source.take();
            fs::remove_dir_all(&self.root).is_ok()
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

    fn create_fixture_dmg(
        source_dir: &Path,
        output: &Path,
    ) -> Result<OwnedProcessResult, SupervisionFailure> {
        let mut command = Command::new(HDIUTIL);
        command
            .args(["create", "-quiet", "-ov", "-format", "UDRO", "-srcfolder"])
            .arg(source_dir)
            .arg(output)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        supervise(command, None, NORMAL_TIMEOUT, false)
    }

    fn attach_descriptor(
        raw_fd: RawFd,
        mount_point: &Path,
        root: &Path,
        timeout: Duration,
        fail_after_spawn: bool,
    ) -> Result<DescriptorAttachResult, SupervisionFailure> {
        let stdout = private_output(root.join("attach.stdout"))
            .map_err(|_| SupervisionFailure::BeforeSpawn)?;
        let stderr = private_output(root.join("attach.stderr"))
            .map_err(|_| SupervisionFailure::BeforeSpawn)?;
        let mut command = Command::new(HDIUTIL);
        command
            .args(["attach", "-nobrowse", "-readonly", "-mountpoint"])
            .arg(mount_point)
            .arg(format!("/dev/fd/{raw_fd}"))
            .stdin(Stdio::null())
            .stdout(Stdio::from(stdout))
            .stderr(Stdio::from(stderr));
        let result = supervise(command, Some(raw_fd), timeout, fail_after_spawn)?;
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
    ) -> Result<OwnedProcessResult, SupervisionFailure> {
        let mut command = Command::new(HDIUTIL);
        command.arg("detach").arg(mount_point).arg("-quiet");
        if force {
            command.arg("-force");
        }
        command
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        supervise(command, None, timeout, false)
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
        fail_after_spawn: bool,
    ) -> Result<OwnedProcessResult, SupervisionFailure> {
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

        let mut child = command
            .spawn()
            .map_err(|_| SupervisionFailure::BeforeSpawn)?;
        let process_group = child.id() as i32;
        if fail_after_spawn {
            return Err(SupervisionFailure::AfterSpawn(UnsettledProcess {
                child,
                process_group,
                fail_settlement_once: false,
            }));
        }
        let deadline = Instant::now() + timeout;
        let mut timed_out = timeout.is_zero();

        let status = loop {
            if timed_out || Instant::now() >= deadline {
                timed_out = true;
                if terminate_process_group(&mut child, process_group).is_err() {
                    return Err(SupervisionFailure::AfterSpawn(UnsettledProcess {
                        child,
                        process_group,
                        fail_settlement_once: false,
                    }));
                }
                match child.wait() {
                    Ok(status) => break status,
                    Err(_) => {
                        return Err(SupervisionFailure::AfterSpawn(UnsettledProcess {
                            child,
                            process_group,
                            fail_settlement_once: false,
                        }));
                    }
                }
            }
            let status = match child.try_wait() {
                Ok(status) => status,
                Err(_) => {
                    return Err(SupervisionFailure::AfterSpawn(UnsettledProcess {
                        child,
                        process_group,
                        fail_settlement_once: false,
                    }));
                }
            };
            if let Some(status) = status {
                if !process_group_settled(process_group)
                    && terminate_process_group(&mut child, process_group).is_err()
                {
                    return Err(SupervisionFailure::AfterSpawn(UnsettledProcess {
                        child,
                        process_group,
                        fail_settlement_once: false,
                    }));
                }
                break status;
            }
            thread::sleep(Duration::from_millis(10));
        };

        if !process_group_settled(process_group) {
            return Err(SupervisionFailure::AfterSpawn(UnsettledProcess {
                child,
                process_group,
                fail_settlement_once: false,
            }));
        }

        Ok(OwnedProcessResult {
            status,
            timed_out,
            settled: true,
        })
    }

    impl UnsettledProcess {
        fn settle(&mut self) -> io::Result<bool> {
            if self.fail_settlement_once {
                self.fail_settlement_once = false;
                return Err(io::Error::other("injected settlement failure"));
            }
            terminate_process_group(&mut self.child, self.process_group)?;
            Ok(process_group_settled(self.process_group))
        }
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
        assert_eq!(outcome.retained_boundary, None);
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
    fn substituted_descriptor_fails_before_hdiutil_and_cleans_without_residue() {
        let mut authority =
            DmgTransportAuthority::invalid().expect("create substitution authority");
        let substitute_path = authority
            .root
            .as_ref()
            .expect("retain private root")
            .join("substitute.dmg");
        write_private(&substitute_path, b"substituted descriptor bytes\n")
            .expect("create substitute bytes");
        authority.source = Some(File::open(substitute_path).expect("open substitute descriptor"));

        let outcome = authority.consume();
        assert_eq!(outcome.disposition, Disposition::Failed);
        assert_eq!(outcome.primary_boundary, FailureBoundary::Authority);
        assert_eq!(outcome.retained_boundary, None);
        assert!(!outcome.hdiutil_started);
        assert!(outcome.process_settled);
        assert!(!outcome.descriptor_bytes_unchanged);
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
        assert_eq!(outcome.retained_boundary, None);
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
        assert_eq!(outcome.primary_boundary, FailureBoundary::Consumption);
        assert_eq!(outcome.retained_boundary, Some(FailureBoundary::Cleanup));
        assert!(outcome.temporary_residue);
        assert!(!outcome.mount_residue);
        assert_non_claims(&outcome);

        authority.retry_cleanup().expect("retry retained cleanup");
        assert!(!authority.has_temporary_residue());
        assert!(!authority.has_mount_residue());
    }

    #[test]
    fn post_spawn_supervision_failure_retains_authority_until_settlement_retry() {
        let mut authority = DmgTransportAuthority::invalid().expect("create retained authority");
        let outcome = authority.consume_with_supervision(NORMAL_TIMEOUT, false, true);

        assert_eq!(outcome.disposition, Disposition::RetainedProcessUnsettled);
        assert_eq!(outcome.primary_boundary, FailureBoundary::Supervision);
        assert_eq!(
            outcome.retained_boundary,
            Some(FailureBoundary::Supervision)
        );
        assert!(outcome.hdiutil_started);
        assert!(!outcome.process_settled);
        assert!(outcome.temporary_residue);
        assert!(authority.source.is_some());
        assert!(!outcome.mount_residue);
        assert_non_claims(&outcome);

        authority
            .unsettled_process
            .as_mut()
            .expect("retain spawned process")
            .fail_settlement_once = true;
        assert!(authority.retry_cleanup().is_err());
        assert!(authority.unsettled_process.is_some());
        assert!(authority.source.is_some());
        assert!(authority.has_temporary_residue());

        authority
            .retry_cleanup()
            .expect("settle retained process and remove authority root");
        assert!(!authority.has_temporary_residue());
        assert!(!authority.has_mount_residue());
    }

    #[test]
    fn drop_transfers_unsettled_authority_to_bounded_recovery_owner() {
        assert_eq!(retained_recovery_count(), 0);
        let retained_root = {
            let mut authority =
                DmgTransportAuthority::invalid().expect("create drop recovery authority");
            let outcome = authority.consume_with_supervision(NORMAL_TIMEOUT, false, true);
            assert_eq!(outcome.disposition, Disposition::RetainedProcessUnsettled);
            authority
                .unsettled_process
                .as_mut()
                .expect("retain spawned process")
                .fail_settlement_once = true;
            authority
                .root
                .as_ref()
                .expect("retain private root")
                .clone()
        };

        assert!(retained_root.exists());
        assert_eq!(retained_recovery_count(), 1);
        recover_retained().expect("settle and clean the retained drop recovery");
        assert_eq!(retained_recovery_count(), 0);
        assert!(!retained_root.exists());
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
