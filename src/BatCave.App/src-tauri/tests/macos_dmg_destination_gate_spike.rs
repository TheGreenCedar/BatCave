//! Test-only issue #114 probe for the DMG destination boundary.
//!
//! The macOS fixture exercises fixed mount, copy, destination revalidation,
//! process settlement, and cleanup operations. It deliberately cannot prove
//! the DMG transport: ADR 0006 records that `hdiutil` cannot consume the owned
//! descriptor and that a filesystem path retains a same-user TOCTOU window.
//! No production entrypoint, native receipt, or release evidence is added.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Disposition {
    Unsupported,
    #[cfg(target_os = "macos")]
    Rejected,
    #[cfg(target_os = "macos")]
    RetainedProcessUnsettled,
    #[cfg(target_os = "macos")]
    RetainedCleanupFailed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FailureBoundary {
    Transport,
    #[cfg(target_os = "macos")]
    Mount,
    #[cfg(target_os = "macos")]
    Copy,
    #[cfg(target_os = "macos")]
    BundleIdentity,
    #[cfg(target_os = "macos")]
    Architectures,
    #[cfg(target_os = "macos")]
    Signature,
    #[cfg(target_os = "macos")]
    DeveloperId,
    #[cfg(target_os = "macos")]
    Notarization,
    #[cfg(target_os = "macos")]
    Staple,
    #[cfg(target_os = "macos")]
    Timeout,
    #[cfg(target_os = "macos")]
    Supervision,
    #[cfg(target_os = "macos")]
    Cleanup,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct DestinationGates {
    bundle_id: bool,
    version: bool,
    universal_architectures: bool,
    signature_integrity: bool,
    developer_id_authority: bool,
    notarization: bool,
    staple: bool,
}

impl DestinationGates {
    #[cfg(target_os = "macos")]
    fn all_required(&self) -> bool {
        self.bundle_id
            && self.version
            && self.universal_architectures
            && self.signature_integrity
            && self.developer_id_authority
            && self.notarization
            && self.staple
    }
}

#[derive(Debug, PartialEq, Eq)]
struct ProbeOutcome {
    disposition: Disposition,
    primary_boundary: FailureBoundary,
    retained_boundary: Option<FailureBoundary>,
    process_started: bool,
    process_settled: bool,
    fixture_dmg_mounted: bool,
    fixture_app_copied: bool,
    image_binding_checks_passed: bool,
    copied_tree_digest_matched: bool,
    destination_revalidation_completed: bool,
    destination_binding_proven: bool,
    gates: DestinationGates,
    mount_residue: bool,
    temporary_residue: bool,
    exact_transport_proven: bool,
    public_artifact_verified: bool,
    app_installed: bool,
    app_launched: bool,
    native_proven: bool,
    receipt_emitted: bool,
    evidence_emitted: bool,
}

impl ProbeOutcome {
    fn unsupported_host() -> Self {
        Self {
            disposition: Disposition::Unsupported,
            primary_boundary: FailureBoundary::Transport,
            retained_boundary: None,
            process_started: false,
            process_settled: true,
            fixture_dmg_mounted: false,
            fixture_app_copied: false,
            image_binding_checks_passed: false,
            copied_tree_digest_matched: false,
            destination_revalidation_completed: false,
            destination_binding_proven: false,
            gates: DestinationGates::default(),
            mount_residue: false,
            temporary_residue: false,
            exact_transport_proven: false,
            public_artifact_verified: false,
            app_installed: false,
            app_launched: false,
            native_proven: false,
            receipt_emitted: false,
            evidence_emitted: false,
        }
    }
}

fn assert_non_claims(outcome: &ProbeOutcome) {
    assert!(!outcome.exact_transport_proven);
    assert!(!outcome.destination_binding_proven);
    assert!(!outcome.public_artifact_verified);
    assert!(!outcome.app_installed);
    assert!(!outcome.app_launched);
    assert!(!outcome.native_proven);
    assert!(!outcome.receipt_emitted);
    assert!(!outcome.evidence_emitted);
}

#[cfg(not(target_os = "macos"))]
#[test]
fn unsupported_hosts_report_the_boundary_without_mutation() {
    let outcome = ProbeOutcome::unsupported_host();
    assert_eq!(outcome.disposition, Disposition::Unsupported);
    assert_eq!(outcome.primary_boundary, FailureBoundary::Transport);
    assert!(outcome.process_settled);
    assert!(!outcome.process_started);
    assert!(!outcome.mount_residue);
    assert!(!outcome.temporary_residue);
    assert_non_claims(&outcome);
}

#[test]
fn probe_has_no_production_or_javascript_entrypoint() {
    let manifest_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let production_lib =
        std::fs::read_to_string(manifest_root.join("src/lib.rs")).expect("read production library");
    assert!(!production_lib.contains("macos_dmg_destination_gate_spike"));

    let repository_root = manifest_root
        .ancestors()
        .nth(3)
        .expect("manifest is nested below repository root");
    for prohibited in [
        "scripts/macos-dmg-destination-gate.mjs",
        "scripts/macos-dmg-destination-gate.test.mjs",
    ] {
        assert!(!repository_root.join(prohibited).exists());
    }
}

#[cfg(target_os = "macos")]
mod macos {
    use super::{assert_non_claims, DestinationGates, Disposition, FailureBoundary, ProbeOutcome};
    use sha2::{Digest, Sha256};
    use std::fs::{self, DirBuilder, File, OpenOptions};
    use std::io::{self, Read, Seek, SeekFrom, Write};
    use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt};
    use std::os::unix::process::CommandExt;
    use std::path::{Path, PathBuf};
    use std::process::{Child, Command, ExitStatus, Stdio};
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::{Mutex, OnceLock};
    use std::thread;
    use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

    const HDIUTIL: &str = "/usr/bin/hdiutil";
    const DITTO: &str = "/usr/bin/ditto";
    const CODESIGN: &str = "/usr/bin/codesign";
    const LIPO: &str = "/usr/bin/lipo";
    const PLIST_BUDDY: &str = "/usr/libexec/PlistBuddy";
    const SPCTL: &str = "/usr/sbin/spctl";
    const STAPLER: &str = "/usr/bin/stapler";
    const RUSTC: &str = "rustc";
    const APP_NAME: &str = "BatCave Monitor.app";
    const EXECUTABLE_NAME: &str = "BatCaveMonitor";
    const EXPECTED_BUNDLE_ID: &str = "dev.batcave.monitor";
    const EXPECTED_VERSION: &str = env!("CARGO_PKG_VERSION");
    const OUTPUT_LIMIT: usize = 32 * 1024;
    const NORMAL_TIMEOUT: Duration = Duration::from_secs(30);
    const TERMINATION_GRACE: Duration = Duration::from_secs(2);
    static ROOT_SEQUENCE: AtomicU64 = AtomicU64::new(0);
    static OUTPUT_SEQUENCE: AtomicU64 = AtomicU64::new(0);
    static TEST_LOCK: Mutex<()> = Mutex::new(());
    static FIXTURE: OnceLock<FixtureTemplate> = OnceLock::new();
    static RETAINED_RECOVERIES: Mutex<Vec<RetainedRecovery>> = Mutex::new(Vec::new());

    #[derive(Clone)]
    struct FixtureTemplate {
        image_bytes: Vec<u8>,
        replacement_image_bytes: Vec<u8>,
    }

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum Fault {
        None,
        ReplaceImageBeforePreflight,
        ReplaceImageAfterPreflight,
        ReplaceDestinationWithSymlink,
        DriftBundleIdentity,
        DriftExecutableTrust,
        ReplaceDestinationDuringRevalidation,
        MountTimeout,
        CopyTimeout,
        SupervisionSettlementFailure,
        CleanupFailure,
    }

    struct ProcessOutput {
        status: ExitStatus,
        timed_out: bool,
        stdout: Vec<u8>,
        stderr: Vec<u8>,
    }

    struct UnsettledProcess {
        child: Child,
        process_group: i32,
        fail_settlement_once: bool,
    }

    enum SupervisionFailure {
        BeforeSpawn(io::Error),
        Settled(io::Error),
        AfterSpawn(UnsettledProcess),
    }

    struct DestinationAuthority {
        root: Option<PathBuf>,
        image_path: PathBuf,
        mount_point: PathBuf,
        destination: PathBuf,
        source: Option<File>,
        expected_digest: [u8; 32],
        replacement_image_bytes: Vec<u8>,
        mounted: bool,
        force_cleanup_failure: bool,
        unsettled_process: Option<UnsettledProcess>,
    }

    struct RetainedRecovery {
        process: Option<UnsettledProcess>,
        source: Option<File>,
        root: PathBuf,
        mount_point: PathBuf,
    }

    impl DestinationAuthority {
        fn valid() -> io::Result<Self> {
            let template = fixture();
            Self::acquire(
                &template.image_bytes,
                template.replacement_image_bytes.clone(),
            )
        }

        fn invalid() -> io::Result<Self> {
            Self::acquire(
                b"not a disk image\n",
                fixture().replacement_image_bytes.clone(),
            )
        }

        fn acquire(image_bytes: &[u8], replacement_image_bytes: Vec<u8>) -> io::Result<Self> {
            let root = unique_private_root("destination")?;
            let image_path = root.join("owned-fixture.dmg");
            let mount_point = root.join("mount");
            let destination_parent = root.join("destination");
            let destination = destination_parent.join(APP_NAME);
            DirBuilder::new().mode(0o700).create(&mount_point)?;
            DirBuilder::new().mode(0o700).create(&destination_parent)?;
            write_private(&image_path, image_bytes)?;
            let mut source = OpenOptions::new().read(true).open(&image_path)?;
            let expected_digest = digest_file(&mut source)?;
            source.seek(SeekFrom::Start(0))?;
            Ok(Self {
                root: Some(root),
                image_path,
                mount_point,
                destination,
                source: Some(source),
                expected_digest,
                replacement_image_bytes,
                mounted: false,
                force_cleanup_failure: false,
                unsettled_process: None,
            })
        }

        fn execute(&mut self, fault: Fault) -> ProbeOutcome {
            if fault == Fault::ReplaceImageBeforePreflight {
                self.replace_image_path();
            }
            if fault == Fault::CleanupFailure {
                self.force_cleanup_failure = true;
            }

            if !self.image_binding_matches() {
                return self.finish(
                    Disposition::Rejected,
                    FailureBoundary::Transport,
                    false,
                    false,
                    false,
                    false,
                    DestinationGates::default(),
                    false,
                );
            }

            if fault == Fault::ReplaceImageAfterPreflight {
                self.replace_image_path();
            }

            let mount_timeout = if fault == Fault::MountTimeout {
                Duration::ZERO
            } else {
                NORMAL_TIMEOUT
            };
            let mut mount = Command::new(HDIUTIL);
            mount
                .args(["attach", "-nobrowse", "-readonly", "-mountpoint"])
                .arg(&self.mount_point)
                .arg(&self.image_path);
            let mount_result = self.run_owned_process(
                &mut mount,
                "attach",
                mount_timeout,
                fault == Fault::SupervisionSettlementFailure,
            );
            let mount_result = match mount_result {
                Ok(result) => result,
                Err(_) => {
                    return self.finish(
                        Disposition::Rejected,
                        if self.unsettled_process.is_some() {
                            FailureBoundary::Supervision
                        } else {
                            FailureBoundary::Mount
                        },
                        self.unsettled_process.is_some(),
                        false,
                        false,
                        false,
                        DestinationGates::default(),
                        false,
                    );
                }
            };
            self.mounted = mount_is_active(&self.mount_point);
            if mount_result.timed_out {
                return self.finish(
                    Disposition::Rejected,
                    FailureBoundary::Timeout,
                    true,
                    false,
                    false,
                    false,
                    DestinationGates::default(),
                    false,
                );
            }
            if !mount_result.status.success() || !self.mounted {
                return self.finish(
                    Disposition::Rejected,
                    FailureBoundary::Mount,
                    true,
                    false,
                    false,
                    false,
                    DestinationGates::default(),
                    false,
                );
            }

            if !self.image_binding_matches() {
                return self.finish(
                    Disposition::Rejected,
                    FailureBoundary::Transport,
                    true,
                    true,
                    false,
                    false,
                    DestinationGates::default(),
                    false,
                );
            }

            let mounted_app = self.mount_point.join(APP_NAME);
            let mounted_digest = match tree_digest_no_links(&mounted_app) {
                Ok(digest) => digest,
                Err(_) => {
                    return self.finish(
                        Disposition::Rejected,
                        FailureBoundary::Copy,
                        true,
                        true,
                        false,
                        false,
                        DestinationGates::default(),
                        false,
                    );
                }
            };

            let copy_timeout = if fault == Fault::CopyTimeout {
                Duration::ZERO
            } else {
                NORMAL_TIMEOUT
            };
            let mut copy = Command::new(DITTO);
            copy.args(["--rsrc", "--extattr", "--acl"])
                .arg(&mounted_app)
                .arg(&self.destination);
            let copy_result = self.run_owned_process(&mut copy, "copy", copy_timeout, false);
            let copy_result = match copy_result {
                Ok(result) => result,
                Err(_) => {
                    return self.finish(
                        Disposition::Rejected,
                        FailureBoundary::Copy,
                        true,
                        true,
                        false,
                        false,
                        DestinationGates::default(),
                        false,
                    );
                }
            };
            if copy_result.timed_out {
                return self.finish(
                    Disposition::Rejected,
                    FailureBoundary::Timeout,
                    true,
                    true,
                    false,
                    false,
                    DestinationGates::default(),
                    false,
                );
            }
            if !copy_result.status.success() {
                return self.finish(
                    Disposition::Rejected,
                    FailureBoundary::Copy,
                    true,
                    true,
                    false,
                    false,
                    DestinationGates::default(),
                    false,
                );
            }

            if fault == Fault::ReplaceDestinationWithSymlink {
                let _ = fs::remove_dir_all(&self.destination);
                let attacker = self
                    .root
                    .as_ref()
                    .expect("authority root retained")
                    .join("attacker.app");
                let _ = DirBuilder::new().mode(0o700).create(&attacker);
                let _ = std::os::unix::fs::symlink(&attacker, &self.destination);
            }

            let copied_tree_digest_matched = tree_digest_no_links(&self.destination)
                .is_ok_and(|digest| digest == mounted_digest);
            if !copied_tree_digest_matched {
                return self.finish(
                    Disposition::Rejected,
                    FailureBoundary::Copy,
                    true,
                    true,
                    true,
                    false,
                    DestinationGates::default(),
                    false,
                );
            }

            match fault {
                Fault::DriftBundleIdentity => {
                    let info_plist = self.destination.join("Contents/Info.plist");
                    let _ = fs::write(info_plist, drifted_info_plist());
                }
                Fault::DriftExecutableTrust => {
                    let executable = self
                        .destination
                        .join("Contents/MacOS")
                        .join(EXECUTABLE_NAME);
                    if let Ok(mut file) = OpenOptions::new().append(true).open(executable) {
                        let _ = file.write_all(b"trust drift");
                    }
                }
                _ => {}
            }

            let (gates, destination_revalidation_completed) =
                self.revalidate_destination(fault == Fault::ReplaceDestinationDuringRevalidation);
            let primary_boundary = first_failed_gate(&gates).unwrap_or(FailureBoundary::Transport);
            self.finish(
                Disposition::Rejected,
                primary_boundary,
                true,
                true,
                true,
                copied_tree_digest_matched,
                gates,
                destination_revalidation_completed,
            )
        }

        fn replace_image_path(&self) {
            let _ = fs::remove_file(&self.image_path);
            let _ = write_private(&self.image_path, &self.replacement_image_bytes);
        }

        fn image_binding_matches(&mut self) -> bool {
            let Some(source) = self.source.as_mut() else {
                return false;
            };
            let Ok(open_metadata) = source.metadata() else {
                return false;
            };
            let Ok(path_metadata) = fs::metadata(&self.image_path) else {
                return false;
            };
            if open_metadata.dev() != path_metadata.dev()
                || open_metadata.ino() != path_metadata.ino()
                || open_metadata.len() != path_metadata.len()
            {
                return false;
            }
            let open_digest = digest_file(source);
            let path_digest =
                File::open(&self.image_path).and_then(|mut file| digest_file(&mut file));
            source.seek(SeekFrom::Start(0)).is_ok()
                && open_digest.is_ok_and(|digest| digest == self.expected_digest)
                && path_digest.is_ok_and(|digest| digest == self.expected_digest)
        }

        #[allow(clippy::too_many_arguments)]
        fn finish(
            &mut self,
            mut disposition: Disposition,
            primary_boundary: FailureBoundary,
            process_started: bool,
            fixture_dmg_mounted: bool,
            fixture_app_copied: bool,
            copied_tree_digest_matched: bool,
            gates: DestinationGates,
            destination_revalidation_completed: bool,
        ) -> ProbeOutcome {
            let image_binding_checks_passed = self.image_binding_matches();
            let mut retained_boundary = None;
            let process_settled = self.unsettled_process.is_none();

            if !process_settled {
                disposition = Disposition::RetainedProcessUnsettled;
                retained_boundary = Some(FailureBoundary::Supervision);
                return ProbeOutcome {
                    disposition,
                    primary_boundary,
                    retained_boundary,
                    process_started,
                    process_settled,
                    fixture_dmg_mounted,
                    fixture_app_copied,
                    image_binding_checks_passed,
                    copied_tree_digest_matched,
                    destination_revalidation_completed,
                    destination_binding_proven: false,
                    gates,
                    mount_residue: mount_is_active(&self.mount_point),
                    temporary_residue: self.root.as_ref().is_some_and(|root| root.exists()),
                    exact_transport_proven: false,
                    public_artifact_verified: false,
                    app_installed: false,
                    app_launched: false,
                    native_proven: false,
                    receipt_emitted: false,
                    evidence_emitted: false,
                };
            }

            if self.mounted || mount_is_active(&self.mount_point) {
                self.mounted = true;
                if self.detach().is_err() {
                    disposition = Disposition::RetainedCleanupFailed;
                    retained_boundary = Some(FailureBoundary::Cleanup);
                }
            }
            self.source.take();
            if retained_boundary.is_none() && self.cleanup_root().is_err() {
                disposition = Disposition::RetainedCleanupFailed;
                retained_boundary = Some(FailureBoundary::Cleanup);
            }

            ProbeOutcome {
                disposition,
                primary_boundary,
                retained_boundary,
                process_started,
                process_settled,
                fixture_dmg_mounted,
                fixture_app_copied,
                image_binding_checks_passed,
                copied_tree_digest_matched,
                destination_revalidation_completed,
                destination_binding_proven: false,
                gates,
                mount_residue: mount_is_active(&self.mount_point),
                temporary_residue: self.root.as_ref().is_some_and(|root| root.exists()),
                exact_transport_proven: false,
                public_artifact_verified: false,
                app_installed: false,
                app_launched: false,
                native_proven: false,
                receipt_emitted: false,
                evidence_emitted: false,
            }
        }

        fn detach(&mut self) -> io::Result<()> {
            let mut detach = Command::new(HDIUTIL);
            detach
                .arg("detach")
                .arg(&self.mount_point)
                .args(["-force", "-quiet"]);
            let result = self
                .run_owned_process(&mut detach, "detach", NORMAL_TIMEOUT, false)
                .map_err(|_| io::Error::other("owned fixture detach supervision failed"))?;
            if result.status.success() && !result.timed_out && !mount_is_active(&self.mount_point) {
                self.mounted = false;
                Ok(())
            } else {
                Err(io::Error::other("owned fixture mount did not detach"))
            }
        }

        fn cleanup_root(&mut self) -> io::Result<()> {
            let Some(root) = self.root.as_ref() else {
                return Ok(());
            };
            if self.force_cleanup_failure {
                return Err(io::Error::other("injected cleanup failure"));
            }
            fs::remove_dir_all(root)?;
            self.root.take();
            Ok(())
        }

        fn retry_cleanup(&mut self) -> io::Result<()> {
            if let Some(mut process) = self.unsettled_process.take() {
                match process.settle() {
                    Ok(true) => {}
                    Ok(false) | Err(_) => {
                        self.unsettled_process = Some(process);
                        return Err(io::Error::other("owned fixture process remains unsettled"));
                    }
                }
            }
            if mount_is_active(&self.mount_point) {
                self.detach()?;
            }
            self.force_cleanup_failure = false;
            self.source.take();
            self.cleanup_root()
        }

        fn revalidate_destination(
            &mut self,
            replace_during_revalidation: bool,
        ) -> (DestinationGates, bool) {
            let app = self.destination.clone();
            let Ok(app_metadata) = fs::symlink_metadata(&app) else {
                return (DestinationGates::default(), false);
            };
            if app_metadata.file_type().is_symlink() || tree_digest_no_links(&app).is_err() {
                return (DestinationGates::default(), false);
            }

            let mut gates = DestinationGates::default();
            let info_plist = app.join("Contents/Info.plist");
            let mut bundle = Command::new(PLIST_BUDDY);
            bundle
                .args(["-c", "Print :CFBundleIdentifier"])
                .arg(&info_plist);
            gates.bundle_id = self
                .run_output_owned(&mut bundle, "bundle-id")
                .is_some_and(|output| output == EXPECTED_BUNDLE_ID);
            if self.unsettled_process.is_some() {
                return (gates, false);
            }

            let mut version = Command::new(PLIST_BUDDY);
            version
                .args(["-c", "Print :CFBundleShortVersionString"])
                .arg(&info_plist);
            gates.version = self
                .run_output_owned(&mut version, "version")
                .is_some_and(|output| output == EXPECTED_VERSION);
            if self.unsettled_process.is_some() {
                return (gates, false);
            }

            let executable = app.join("Contents/MacOS").join(EXECUTABLE_NAME);
            let mut architectures = Command::new(LIPO);
            architectures.arg("-archs").arg(&executable);
            gates.universal_architectures = self
                .run_output_owned(&mut architectures, "architectures")
                .is_some_and(|output| {
                    let mut observed = output.split_whitespace().collect::<Vec<_>>();
                    observed.sort_unstable();
                    observed == ["arm64", "x86_64"]
                });
            if self.unsettled_process.is_some() {
                return (gates, false);
            }

            if replace_during_revalidation {
                if let Ok(mut file) = OpenOptions::new().append(true).open(&executable) {
                    let _ = file.write_all(b"mid-revalidation substitution");
                }
            }

            let mut verify = Command::new(CODESIGN);
            verify
                .args(["--verify", "--deep", "--strict", "--verbose=2"])
                .arg(&app);
            gates.signature_integrity = self
                .run_owned_process(&mut verify, "signature", NORMAL_TIMEOUT, false)
                .is_ok_and(|result| result.status.success() && !result.timed_out);
            if self.unsettled_process.is_some() {
                return (gates, false);
            }

            let mut authority = Command::new(CODESIGN);
            authority.args(["-d", "--verbose=4"]).arg(&app);
            gates.developer_id_authority = self
                .run_owned_process(&mut authority, "developer-id", NORMAL_TIMEOUT, false)
                .is_ok_and(|result| {
                    result.status.success()
                        && contains_text(&result.stderr, "Authority=Developer ID Application:")
                });
            if self.unsettled_process.is_some() {
                return (gates, false);
            }

            let mut assess = Command::new(SPCTL);
            assess
                .args(["--assess", "--type", "execute", "--verbose=4"])
                .arg(&app);
            gates.notarization = self
                .run_owned_process(&mut assess, "notarization", NORMAL_TIMEOUT, false)
                .is_ok_and(|result| {
                    result.status.success()
                        && contains_text(&result.stderr, "source=Notarized Developer ID")
                });
            if self.unsettled_process.is_some() {
                return (gates, false);
            }

            let mut staple = Command::new(STAPLER);
            staple.arg("validate").arg(&app);
            gates.staple = self
                .run_owned_process(&mut staple, "staple", NORMAL_TIMEOUT, false)
                .is_ok_and(|result| result.status.success() && !result.timed_out);
            (gates, self.unsettled_process.is_none())
        }

        fn run_output_owned(&mut self, command: &mut Command, label: &str) -> Option<String> {
            self.run_owned_process(command, label, NORMAL_TIMEOUT, false)
                .ok()
                .filter(|result| result.status.success() && !result.timed_out)
                .and_then(|result| String::from_utf8(result.stdout).ok())
                .map(|value| value.trim().to_owned())
        }

        fn run_owned_process(
            &mut self,
            command: &mut Command,
            label: &str,
            timeout: Duration,
            fail_after_spawn: bool,
        ) -> Result<ProcessOutput, ()> {
            let root = self.root.as_ref().ok_or(())?.clone();
            match run_process(command, &root, label, timeout, fail_after_spawn) {
                Ok(output) => Ok(output),
                Err(SupervisionFailure::AfterSpawn(process)) => {
                    self.unsettled_process = Some(process);
                    Err(())
                }
                Err(
                    SupervisionFailure::BeforeSpawn(error) | SupervisionFailure::Settled(error),
                ) => {
                    let _ = error.kind();
                    Err(())
                }
            }
        }

        fn transfer_recovery(&mut self, process: Option<UnsettledProcess>) {
            if let Some(root) = self.root.take() {
                RETAINED_RECOVERIES
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .push(RetainedRecovery {
                        process,
                        source: self.source.take(),
                        root,
                        mount_point: self.mount_point.clone(),
                    });
            }
        }
    }

    impl Drop for DestinationAuthority {
        fn drop(&mut self) {
            self.force_cleanup_failure = false;
            if let Some(mut process) = self.unsettled_process.take() {
                if !process.settle().unwrap_or(false) {
                    self.transfer_recovery(Some(process));
                    return;
                }
            }
            if mount_is_active(&self.mount_point) && self.detach().is_err() {
                let process = self.unsettled_process.take();
                self.transfer_recovery(process);
                return;
            }
            self.source.take();
            if self.cleanup_root().is_err() {
                self.transfer_recovery(None);
            }
        }
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

    impl RetainedRecovery {
        fn recover(&mut self) -> bool {
            if let Some(mut process) = self.process.take() {
                if !process.settle().unwrap_or(false) {
                    self.process = Some(process);
                    return false;
                }
            }
            if mount_is_active(&self.mount_point) {
                let mut detach = Command::new(HDIUTIL);
                detach
                    .arg("detach")
                    .arg(&self.mount_point)
                    .args(["-force", "-quiet"]);
                match run_process(
                    &mut detach,
                    &self.root,
                    "recovery-detach",
                    NORMAL_TIMEOUT,
                    false,
                ) {
                    Ok(result)
                        if result.status.success()
                            && !result.timed_out
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
        let count = unresolved.len();
        RETAINED_RECOVERIES
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .extend(unresolved);
        if count == 0 {
            Ok(())
        } else {
            Err(io::Error::other("destination recovery remains unresolved"))
        }
    }

    fn first_failed_gate(gates: &DestinationGates) -> Option<FailureBoundary> {
        if !gates.bundle_id || !gates.version {
            Some(FailureBoundary::BundleIdentity)
        } else if !gates.universal_architectures {
            Some(FailureBoundary::Architectures)
        } else if !gates.signature_integrity {
            Some(FailureBoundary::Signature)
        } else if !gates.developer_id_authority {
            Some(FailureBoundary::DeveloperId)
        } else if !gates.notarization {
            Some(FailureBoundary::Notarization)
        } else if !gates.staple {
            Some(FailureBoundary::Staple)
        } else {
            None
        }
    }

    fn contains_text(bytes: &[u8], expected: &str) -> bool {
        String::from_utf8_lossy(bytes).contains(expected)
    }

    fn fixture() -> &'static FixtureTemplate {
        FIXTURE.get_or_init(|| create_fixture().expect("create destination-gate fixture"))
    }

    fn create_fixture() -> io::Result<FixtureTemplate> {
        let root = unique_private_root("template")?;
        let result = create_fixture_in(&root);
        if !retained_root(&root) {
            let _ = fs::remove_dir_all(&root);
        }
        result
    }

    fn create_fixture_in(root: &Path) -> io::Result<FixtureTemplate> {
        let image_source = root.join("image-source");
        let app = image_source.join(APP_NAME);
        let contents = app.join("Contents");
        let macos = contents.join("MacOS");
        DirBuilder::new().mode(0o700).create(&image_source)?;
        fs::create_dir_all(&macos)?;
        fs::write(contents.join("Info.plist"), fixture_info_plist())?;
        let source = root.join("fixture-main.rs");
        fs::write(&source, "fn main() {}\n")?;
        let arm64 = root.join("fixture-arm64");
        let x86_64 = root.join("fixture-x86_64");
        compile_fixture(root, &source, &arm64, "aarch64-apple-darwin")?;
        compile_fixture(root, &source, &x86_64, "x86_64-apple-darwin")?;
        let executable = macos.join(EXECUTABLE_NAME);
        let mut lipo = Command::new(LIPO);
        lipo.args(["-create"])
            .arg(&arm64)
            .arg(&x86_64)
            .arg("-output")
            .arg(&executable);
        require_success(run_fixture_process(
            &mut lipo,
            root,
            "fixture-lipo",
            NORMAL_TIMEOUT,
        )?)?;
        let mut sign = Command::new(CODESIGN);
        sign.args(["--force", "--deep", "--sign", "-"]).arg(&app);
        require_success(run_fixture_process(
            &mut sign,
            root,
            "fixture-sign",
            NORMAL_TIMEOUT,
        )?)?;

        let image = root.join("fixture.dmg");
        let mut create = Command::new(HDIUTIL);
        create
            .args(["create", "-quiet", "-ov", "-format", "UDRO", "-srcfolder"])
            .arg(&image_source)
            .arg(&image);
        require_success(run_fixture_process(
            &mut create,
            root,
            "fixture-image",
            NORMAL_TIMEOUT,
        )?)?;

        let replacement_source = root.join("replacement-source");
        DirBuilder::new().mode(0o700).create(&replacement_source)?;
        fs::write(
            replacement_source.join("replacement-marker.txt"),
            "replacement fixture\n",
        )?;
        let replacement = root.join("replacement.dmg");
        let mut create_replacement = Command::new(HDIUTIL);
        create_replacement
            .args(["create", "-quiet", "-ov", "-format", "UDRO", "-srcfolder"])
            .arg(&replacement_source)
            .arg(&replacement);
        require_success(run_fixture_process(
            &mut create_replacement,
            root,
            "replacement-image",
            NORMAL_TIMEOUT,
        )?)?;

        Ok(FixtureTemplate {
            image_bytes: fs::read(image)?,
            replacement_image_bytes: fs::read(replacement)?,
        })
    }

    fn compile_fixture(root: &Path, source: &Path, output: &Path, target: &str) -> io::Result<()> {
        let mut compile = Command::new(RUSTC);
        compile
            .arg(source)
            .args([
                "--crate-name",
                "batcave_destination_fixture",
                "--target",
                target,
            ])
            .arg("-o")
            .arg(output);
        require_success(run_fixture_process(
            &mut compile,
            root,
            &format!("fixture-{target}"),
            NORMAL_TIMEOUT,
        )?)
    }

    fn require_success(result: ProcessOutput) -> io::Result<()> {
        if result.status.success() && !result.timed_out {
            Ok(())
        } else {
            Err(io::Error::other("fixed fixture command failed"))
        }
    }

    fn run_fixture_process(
        command: &mut Command,
        root: &Path,
        label: &str,
        timeout: Duration,
    ) -> io::Result<ProcessOutput> {
        match run_process(command, root, label, timeout, false) {
            Ok(output) => Ok(output),
            Err(SupervisionFailure::BeforeSpawn(error) | SupervisionFailure::Settled(error)) => {
                Err(error)
            }
            Err(SupervisionFailure::AfterSpawn(mut process)) => {
                if process.settle().unwrap_or(false) {
                    return Err(io::Error::other("fixture command supervision failed"));
                }
                RETAINED_RECOVERIES
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .push(RetainedRecovery {
                        process: Some(process),
                        source: None,
                        root: root.to_path_buf(),
                        mount_point: root.join("unused-fixture-mount"),
                    });
                Err(io::Error::other(
                    "fixture command unresolved; private root retained",
                ))
            }
        }
    }

    fn retained_root(root: &Path) -> bool {
        RETAINED_RECOVERIES
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .iter()
            .any(|recovery| recovery.root == root)
    }

    fn fixture_info_plist() -> &'static str {
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>CFBundleExecutable</key><string>BatCaveMonitor</string>
<key>CFBundleIdentifier</key><string>dev.batcave.monitor</string>
<key>CFBundleName</key><string>BatCave Monitor</string>
<key>CFBundlePackageType</key><string>APPL</string>
<key>CFBundleShortVersionString</key><string>0.2.0-rc.2</string>
<key>CFBundleVersion</key><string>2</string>
</dict></plist>
"#
    }

    fn drifted_info_plist() -> &'static str {
        r#"<?xml version="1.0" encoding="UTF-8"?>
<plist version="1.0"><dict>
<key>CFBundleExecutable</key><string>BatCaveMonitor</string>
<key>CFBundleIdentifier</key><string>com.attacker.replacement</string>
<key>CFBundleShortVersionString</key><string>99.0.0</string>
</dict></plist>
"#
    }

    fn unique_private_root(label: &str) -> io::Result<PathBuf> {
        for _ in 0..32 {
            let sequence = ROOT_SEQUENCE.fetch_add(1, Ordering::Relaxed);
            let nanos = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos();
            let root = std::env::temp_dir().join(format!(
                "batcave-dmg-{label}-{}-{nanos}-{sequence}",
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
            "could not create private fixture root",
        ))
    }

    fn write_private(path: &Path, bytes: &[u8]) -> io::Result<()> {
        OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(path)?
            .write_all(bytes)
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

    fn tree_digest_no_links(root: &Path) -> io::Result<[u8; 32]> {
        let metadata = fs::symlink_metadata(root)?;
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err(io::Error::other("destination app must be a real directory"));
        }
        let mut entries = Vec::new();
        collect_tree(root, root, &mut entries)?;
        entries.sort_by(|left, right| left.0.cmp(&right.0));
        let mut hasher = Sha256::new();
        for (relative, kind, bytes) in entries {
            hasher.update((relative.len() as u64).to_be_bytes());
            hasher.update(relative.as_bytes());
            hasher.update([kind]);
            hasher.update((bytes.len() as u64).to_be_bytes());
            hasher.update(bytes);
        }
        Ok(hasher.finalize().into())
    }

    fn collect_tree(
        root: &Path,
        current: &Path,
        entries: &mut Vec<(String, u8, Vec<u8>)>,
    ) -> io::Result<()> {
        let mut children = fs::read_dir(current)?.collect::<Result<Vec<_>, _>>()?;
        children.sort_by_key(|entry| entry.file_name());
        for child in children {
            let path = child.path();
            let metadata = fs::symlink_metadata(&path)?;
            if metadata.file_type().is_symlink() {
                return Err(io::Error::other(
                    "symlinks are rejected at the fixture destination",
                ));
            }
            let relative = path
                .strip_prefix(root)
                .map_err(|_| io::Error::other("tree entry escaped root"))?
                .to_string_lossy()
                .into_owned();
            if metadata.is_dir() {
                entries.push((relative, b'd', Vec::new()));
                collect_tree(root, &path, entries)?;
            } else if metadata.is_file() {
                entries.push((relative, b'f', fs::read(path)?));
            } else {
                return Err(io::Error::other("non-file destination entry rejected"));
            }
        }
        Ok(())
    }

    fn run_process(
        command: &mut Command,
        root: &Path,
        label: &str,
        timeout: Duration,
        fail_after_spawn: bool,
    ) -> Result<ProcessOutput, SupervisionFailure> {
        let sequence = OUTPUT_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let stdout_path = root.join(format!("{label}-{sequence}.stdout"));
        let stderr_path = root.join(format!("{label}-{sequence}.stderr"));
        let stdout = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&stdout_path)
            .map_err(SupervisionFailure::BeforeSpawn)?;
        let stderr = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&stderr_path)
            .map_err(SupervisionFailure::BeforeSpawn)?;
        command
            .stdin(Stdio::null())
            .stdout(Stdio::from(stdout))
            .stderr(Stdio::from(stderr));
        unsafe {
            command.pre_exec(|| {
                if libc::setpgid(0, 0) == 0 {
                    Ok(())
                } else {
                    Err(io::Error::last_os_error())
                }
            });
        }
        let mut child = command.spawn().map_err(SupervisionFailure::BeforeSpawn)?;
        let process_group = child.id() as i32;
        if fail_after_spawn {
            return Err(SupervisionFailure::AfterSpawn(UnsettledProcess {
                child,
                process_group,
                fail_settlement_once: true,
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
                break match child.wait() {
                    Ok(status) => status,
                    Err(_) => {
                        return Err(SupervisionFailure::AfterSpawn(UnsettledProcess {
                            child,
                            process_group,
                            fail_settlement_once: false,
                        }));
                    }
                };
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
        Ok(ProcessOutput {
            status,
            timed_out,
            stdout: read_bounded(&stdout_path).map_err(SupervisionFailure::Settled)?,
            stderr: read_bounded(&stderr_path).map_err(SupervisionFailure::Settled)?,
        })
    }

    fn read_bounded(path: &Path) -> io::Result<Vec<u8>> {
        let file = File::open(path)?;
        let mut output = Vec::new();
        file.take((OUTPUT_LIMIT + 1) as u64)
            .read_to_end(&mut output)?;
        if output.len() > OUTPUT_LIMIT {
            Err(io::Error::other("fixed command output exceeded limit"))
        } else {
            Ok(output)
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
        result != 0 && io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH)
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
    fn inert_fixture_reaches_every_destination_gate_then_fails_closed_on_public_trust() {
        let _guard = TEST_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let mut authority = DestinationAuthority::valid().expect("acquire fixture authority");
        let outcome = authority.execute(Fault::None);

        assert_eq!(outcome.disposition, Disposition::Rejected);
        assert_eq!(outcome.primary_boundary, FailureBoundary::DeveloperId);
        assert!(outcome.process_started);
        assert!(outcome.process_settled);
        assert!(outcome.fixture_dmg_mounted);
        assert!(outcome.fixture_app_copied);
        assert!(outcome.image_binding_checks_passed);
        assert!(outcome.copied_tree_digest_matched);
        assert!(outcome.destination_revalidation_completed);
        assert!(outcome.gates.bundle_id);
        assert!(outcome.gates.version);
        assert!(outcome.gates.universal_architectures);
        assert!(outcome.gates.signature_integrity);
        assert!(!outcome.gates.developer_id_authority);
        assert!(!outcome.gates.notarization);
        assert!(!outcome.gates.staple);
        assert!(!outcome.gates.all_required());
        assert!(!outcome.mount_residue);
        assert!(!outcome.temporary_residue);
        assert_non_claims(&outcome);
    }

    #[test]
    fn owned_image_path_replacement_fails_before_mount() {
        let _guard = TEST_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let mut authority = DestinationAuthority::valid().expect("acquire fixture authority");
        let outcome = authority.execute(Fault::ReplaceImageBeforePreflight);
        assert_eq!(outcome.primary_boundary, FailureBoundary::Transport);
        assert!(!outcome.process_started);
        assert!(!outcome.fixture_dmg_mounted);
        assert!(!outcome.fixture_app_copied);
        assert!(!outcome.image_binding_checks_passed);
        assert!(!outcome.mount_residue);
        assert!(!outcome.temporary_residue);
        assert_non_claims(&outcome);
    }

    #[test]
    fn post_preflight_image_substitution_is_detected_after_mount_and_detached() {
        let _guard = TEST_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let mut authority = DestinationAuthority::valid().expect("acquire fixture authority");
        let outcome = authority.execute(Fault::ReplaceImageAfterPreflight);
        assert_eq!(outcome.primary_boundary, FailureBoundary::Transport);
        assert!(outcome.process_started);
        assert!(outcome.fixture_dmg_mounted);
        assert!(!outcome.fixture_app_copied);
        assert!(!outcome.image_binding_checks_passed);
        assert!(!outcome.mount_residue);
        assert!(!outcome.temporary_residue);
        assert_non_claims(&outcome);
    }

    #[test]
    fn invalid_owned_image_fails_at_mount_and_cleans() {
        let _guard = TEST_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let mut authority = DestinationAuthority::invalid().expect("acquire invalid authority");
        let outcome = authority.execute(Fault::None);
        assert_eq!(outcome.primary_boundary, FailureBoundary::Mount);
        assert!(outcome.process_started);
        assert!(!outcome.fixture_dmg_mounted);
        assert!(!outcome.fixture_app_copied);
        assert!(!outcome.mount_residue);
        assert!(!outcome.temporary_residue);
        assert_non_claims(&outcome);
    }

    #[test]
    fn destination_symlink_substitution_fails_before_revalidation() {
        let _guard = TEST_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let mut authority = DestinationAuthority::valid().expect("acquire fixture authority");
        let outcome = authority.execute(Fault::ReplaceDestinationWithSymlink);
        assert_eq!(outcome.primary_boundary, FailureBoundary::Copy);
        assert!(outcome.fixture_dmg_mounted);
        assert!(outcome.fixture_app_copied);
        assert!(!outcome.copied_tree_digest_matched);
        assert!(!outcome.destination_revalidation_completed);
        assert_eq!(outcome.gates, DestinationGates::default());
        assert!(!outcome.mount_residue);
        assert!(!outcome.temporary_residue);
        assert_non_claims(&outcome);
    }

    #[test]
    fn bundle_identity_drift_fails_the_consumed_destination_hook() {
        let _guard = TEST_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let mut authority = DestinationAuthority::valid().expect("acquire fixture authority");
        let outcome = authority.execute(Fault::DriftBundleIdentity);
        assert_eq!(outcome.primary_boundary, FailureBoundary::BundleIdentity);
        assert!(!outcome.gates.bundle_id);
        assert!(!outcome.gates.version);
        assert!(!outcome.gates.signature_integrity);
        assert!(!outcome.gates.all_required());
        assert!(!outcome.mount_residue);
        assert!(!outcome.temporary_residue);
        assert_non_claims(&outcome);
    }

    #[test]
    fn executable_trust_drift_invalidates_the_destination_signature() {
        let _guard = TEST_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let mut authority = DestinationAuthority::valid().expect("acquire fixture authority");
        let outcome = authority.execute(Fault::DriftExecutableTrust);
        assert_eq!(outcome.primary_boundary, FailureBoundary::Signature);
        assert!(outcome.gates.bundle_id);
        assert!(outcome.gates.version);
        assert!(outcome.gates.universal_architectures);
        assert!(!outcome.gates.signature_integrity);
        assert!(!outcome.gates.all_required());
        assert!(!outcome.mount_residue);
        assert!(!outcome.temporary_residue);
        assert_non_claims(&outcome);
    }

    #[test]
    fn mid_revalidation_substitution_exposes_unbound_destination_observations() {
        let _guard = TEST_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let mut authority = DestinationAuthority::valid().expect("acquire fixture authority");
        let outcome = authority.execute(Fault::ReplaceDestinationDuringRevalidation);

        assert_eq!(outcome.primary_boundary, FailureBoundary::Signature);
        assert!(outcome.destination_revalidation_completed);
        assert!(!outcome.destination_binding_proven);
        assert!(outcome.gates.bundle_id);
        assert!(outcome.gates.version);
        assert!(outcome.gates.universal_architectures);
        assert!(!outcome.gates.signature_integrity);
        assert!(!outcome.gates.all_required());
        assert!(!outcome.mount_residue);
        assert!(!outcome.temporary_residue);
        assert_non_claims(&outcome);
    }

    #[test]
    fn mount_and_copy_timeouts_settle_and_leave_zero_residue() {
        let _guard = TEST_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        for fault in [Fault::MountTimeout, Fault::CopyTimeout] {
            let mut authority = DestinationAuthority::valid().expect("acquire timeout authority");
            let outcome = authority.execute(fault);
            assert_eq!(outcome.primary_boundary, FailureBoundary::Timeout);
            assert!(outcome.process_started);
            assert!(outcome.process_settled);
            assert!(!outcome.mount_residue);
            assert!(!outcome.temporary_residue);
            assert_non_claims(&outcome);
        }
    }

    #[test]
    fn cleanup_failure_is_retained_and_bounded_retry_removes_residue() {
        let _guard = TEST_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let mut authority = DestinationAuthority::valid().expect("acquire cleanup authority");
        let outcome = authority.execute(Fault::CleanupFailure);
        assert_eq!(outcome.disposition, Disposition::RetainedCleanupFailed);
        assert_eq!(outcome.retained_boundary, Some(FailureBoundary::Cleanup));
        assert!(!outcome.mount_residue);
        assert!(outcome.temporary_residue);
        assert_non_claims(&outcome);

        authority.retry_cleanup().expect("retry retained cleanup");
        assert!(!authority.root.as_ref().is_some_and(|root| root.exists()));
        assert!(!mount_is_active(&authority.mount_point));
    }

    #[test]
    fn post_spawn_settlement_failure_retains_authority_until_retry_succeeds() {
        let _guard = TEST_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let mut authority = DestinationAuthority::valid().expect("acquire fixture authority");
        let outcome = authority.execute(Fault::SupervisionSettlementFailure);

        assert_eq!(outcome.disposition, Disposition::RetainedProcessUnsettled);
        assert_eq!(outcome.primary_boundary, FailureBoundary::Supervision);
        assert_eq!(
            outcome.retained_boundary,
            Some(FailureBoundary::Supervision)
        );
        assert!(outcome.process_started);
        assert!(!outcome.process_settled);
        assert!(outcome.temporary_residue);
        assert_non_claims(&outcome);

        assert!(authority.retry_cleanup().is_err());
        assert!(authority.unsettled_process.is_some());
        assert!(authority.root.as_ref().is_some_and(|root| root.exists()));

        authority
            .retry_cleanup()
            .expect("second retry settles and cleans retained authority");
        assert!(authority.unsettled_process.is_none());
        assert!(!authority.root.as_ref().is_some_and(|root| root.exists()));
        assert!(!mount_is_active(&authority.mount_point));
    }

    #[test]
    fn drop_transfers_unsettled_authority_to_recoverable_retention() {
        let _guard = TEST_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        recover_retained().expect("start without retained fixture authority");
        assert_eq!(retained_recovery_count(), 0);

        let root = {
            let mut authority = DestinationAuthority::valid().expect("acquire fixture authority");
            let root = authority.root.clone().expect("authority root");
            let outcome = authority.execute(Fault::SupervisionSettlementFailure);
            assert_eq!(outcome.disposition, Disposition::RetainedProcessUnsettled);
            root
        };

        assert_eq!(retained_recovery_count(), 1);
        assert!(root.exists());
        recover_retained().expect("recover transferred fixture authority");
        assert_eq!(retained_recovery_count(), 0);
        assert!(!root.exists());
    }

    #[test]
    fn every_destination_gate_is_mandatory_and_cannot_mint_native_proof() {
        let all = DestinationGates {
            bundle_id: true,
            version: true,
            universal_architectures: true,
            signature_integrity: true,
            developer_id_authority: true,
            notarization: true,
            staple: true,
        };
        assert!(all.all_required());
        for boundary in 0..7 {
            let mut missing = all.clone();
            match boundary {
                0 => missing.bundle_id = false,
                1 => missing.version = false,
                2 => missing.universal_architectures = false,
                3 => missing.signature_integrity = false,
                4 => missing.developer_id_authority = false,
                5 => missing.notarization = false,
                6 => missing.staple = false,
                _ => unreachable!(),
            }
            assert!(!missing.all_required());
        }
        let rendered = format!("{:?}", ProbeOutcome::unsupported_host());
        for forbidden in [
            "/private/",
            "owned-fixture.dmg",
            "receipt_id",
            "evidence_packet",
            "native_proven: true",
        ] {
            assert!(!rendered.contains(forbidden));
        }
    }
}
