use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

const PAYLOAD: &[u8] = b"BatCave private native consumption authority prototype bytes\n";

mod authority {
    use super::*;
    use std::sync::{mpsc, Arc};
    use std::thread;

    const ASSET_NAME: &str = "selected-artifact.bin";

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    pub(super) enum Profile {
        WindowsNsis,
        LinuxDeb,
        LinuxAppImage,
        MacOsDmg,
        MacOsUpdater,
    }

    impl Profile {
        fn id(self) -> &'static str {
            match self {
                Self::WindowsNsis => "windows:nsis",
                Self::LinuxDeb => "linux:deb",
                Self::LinuxAppImage => "linux:appimage",
                Self::MacOsDmg => "macos:dmg",
                Self::MacOsUpdater => "macos:macos_updater",
            }
        }
    }

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    pub(super) enum FailureBoundary {
        Acquisition,
        Authority,
        Consumption,
        Timeout,
        Settlement,
        Cleanup,
        Residue,
    }

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    pub(super) enum Disposition {
        PrototypeConsumed,
        Failed,
    }

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    pub(super) enum BoundaryState {
        Passed,
        Failed,
        TimedOut,
        NotTriggered,
    }

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    enum Phase {
        Acquired,
        Consuming,
        Settling,
        RetainedCleanupFailed,
        Closed,
    }

    #[derive(Clone, Copy)]
    enum ProbeBehavior {
        Normal,
        Slow,
    }

    pub(super) struct ClosedPlan {
        binding_id: String,
        profile: Profile,
        expected_size: usize,
        expected_sha256: [u8; 32],
        step_timeout: Duration,
        probe_behavior: ProbeBehavior,
        cleanup_fails_once: bool,
    }

    pub(super) fn closed_plan(profile: Profile, expected: &[u8]) -> ClosedPlan {
        plan_with_behavior(profile, expected, ProbeBehavior::Normal, false)
    }

    pub(super) fn timeout_plan(profile: Profile, expected: &[u8]) -> ClosedPlan {
        plan_with_behavior(profile, expected, ProbeBehavior::Slow, false)
    }

    pub(super) fn cleanup_failure_plan(profile: Profile, expected: &[u8]) -> ClosedPlan {
        plan_with_behavior(profile, expected, ProbeBehavior::Normal, true)
    }

    fn plan_with_behavior(
        profile: Profile,
        expected: &[u8],
        behavior: ProbeBehavior,
        cleanup_fails_once: bool,
    ) -> ClosedPlan {
        let step_timeout = match behavior {
            ProbeBehavior::Normal => Duration::from_secs(1),
            ProbeBehavior::Slow => Duration::from_millis(1),
        };
        ClosedPlan {
            binding_id: format!("prototype-{}", profile.id().replace([':', '_'], "-")),
            profile,
            expected_size: expected.len(),
            expected_sha256: digest(expected),
            step_timeout,
            probe_behavior: behavior,
            cleanup_fails_once,
        }
    }

    #[derive(Debug, Eq, PartialEq)]
    pub(super) struct AuthorityError {
        boundary: FailureBoundary,
        message: &'static str,
    }

    impl AuthorityError {
        pub(super) fn boundary(&self) -> FailureBoundary {
            self.boundary
        }

        pub(super) fn message(&self) -> &'static str {
            self.message
        }
    }

    #[derive(Debug)]
    pub(super) struct PublicOutcome {
        disposition: Disposition,
        profile_id: &'static str,
        binding_id: String,
        failures: Vec<FailureBoundary>,
        acquisition: BoundaryState,
        consumption: BoundaryState,
        timeout: BoundaryState,
        settlement: BoundaryState,
        cleanup: BoundaryState,
        residue: BoundaryState,
        observed_size: Option<usize>,
        observed_sha256: Option<String>,
        fixed_probe_completed: bool,
        package_bytes_executed: bool,
        package_installed_or_staged: bool,
        native_proven: bool,
    }

    impl PublicOutcome {
        pub(super) fn disposition(&self) -> Disposition {
            self.disposition
        }

        pub(super) fn profile_id(&self) -> &'static str {
            self.profile_id
        }

        pub(super) fn binding_id(&self) -> &str {
            &self.binding_id
        }

        pub(super) fn failures(&self) -> &[FailureBoundary] {
            &self.failures
        }

        pub(super) fn boundary(&self, boundary: FailureBoundary) -> BoundaryState {
            match boundary {
                FailureBoundary::Acquisition => self.acquisition,
                FailureBoundary::Authority => BoundaryState::NotTriggered,
                FailureBoundary::Consumption => self.consumption,
                FailureBoundary::Timeout => self.timeout,
                FailureBoundary::Settlement => self.settlement,
                FailureBoundary::Cleanup => self.cleanup,
                FailureBoundary::Residue => self.residue,
            }
        }

        pub(super) fn observed_size(&self) -> Option<usize> {
            self.observed_size
        }

        pub(super) fn observed_sha256(&self) -> Option<&str> {
            self.observed_sha256.as_deref()
        }

        pub(super) fn fixed_probe_completed(&self) -> bool {
            self.fixed_probe_completed
        }

        pub(super) fn claims_native_execution(&self) -> bool {
            self.package_bytes_executed || self.package_installed_or_staged || self.native_proven
        }
    }

    struct CompletionSeal;

    pub(super) struct Completion {
        seal: Arc<CompletionSeal>,
        outcome: PublicOutcome,
    }

    impl Completion {
        pub(super) fn outcome(&self) -> &PublicOutcome {
            &self.outcome
        }
    }

    pub(super) struct NativeAuthority {
        plan: ClosedPlan,
        owned_bytes: Option<Arc<[u8]>>,
        phase: Phase,
        seal: Arc<CompletionSeal>,
        cleanup_fails_once: bool,
    }

    pub(super) fn acquire(
        plan: ClosedPlan,
        verified_root: &Path,
    ) -> Result<NativeAuthority, AuthorityError> {
        let root_metadata = fs::symlink_metadata(verified_root).map_err(|_| AuthorityError {
            boundary: FailureBoundary::Acquisition,
            message: "verified root is unavailable",
        })?;
        if !root_metadata.is_dir() || root_metadata.file_type().is_symlink() {
            return Err(AuthorityError {
                boundary: FailureBoundary::Acquisition,
                message: "verified root must be a non-link directory",
            });
        }

        let source_path = verified_root.join(ASSET_NAME);
        let source_metadata = fs::symlink_metadata(&source_path).map_err(|_| AuthorityError {
            boundary: FailureBoundary::Acquisition,
            message: "selected artifact is unavailable",
        })?;
        if !source_metadata.is_file() || source_metadata.file_type().is_symlink() {
            return Err(AuthorityError {
                boundary: FailureBoundary::Acquisition,
                message: "selected artifact must be a regular non-link file",
            });
        }

        let bytes = fs::read(&source_path).map_err(|_| AuthorityError {
            boundary: FailureBoundary::Acquisition,
            message: "selected artifact could not be read",
        })?;
        if bytes.len() != plan.expected_size || digest(&bytes) != plan.expected_sha256 {
            return Err(AuthorityError {
                boundary: FailureBoundary::Acquisition,
                message: "selected artifact does not match the closed binding",
            });
        }

        Ok(NativeAuthority {
            cleanup_fails_once: plan.cleanup_fails_once,
            plan,
            owned_bytes: Some(Arc::from(bytes)),
            phase: Phase::Acquired,
            seal: Arc::new(CompletionSeal),
        })
    }

    impl NativeAuthority {
        pub(super) fn consume(&mut self) -> Result<Completion, AuthorityError> {
            if self.phase != Phase::Acquired {
                return Err(AuthorityError {
                    boundary: FailureBoundary::Authority,
                    message: "authority is not in the acquired phase",
                });
            }
            self.phase = Phase::Consuming;

            let bytes = Arc::clone(
                self.owned_bytes
                    .as_ref()
                    .expect("acquired authority owns exact bytes"),
            );
            let behavior = self.plan.probe_behavior;
            let (sender, receiver) = mpsc::sync_channel(1);
            let (release_sender, release_receiver) = mpsc::sync_channel(0);
            let worker = thread::spawn(move || {
                match behavior {
                    ProbeBehavior::Normal => thread::sleep(Duration::from_millis(2)),
                    ProbeBehavior::Slow => {
                        let _ = release_receiver.recv();
                    }
                }
                let observation = (bytes.len(), digest(&bytes));
                let _ = sender.send(observation);
            });

            let observation = receiver.recv_timeout(self.plan.step_timeout);
            if matches!(behavior, ProbeBehavior::Slow) {
                let _ = release_sender.send(());
            }
            self.phase = Phase::Settling;
            let settled = worker.join().is_ok();

            let timed_out = matches!(observation, Err(mpsc::RecvTimeoutError::Timeout));
            let observed = observation.ok();
            let digest_matches = observed.as_ref().is_some_and(|(size, sha256)| {
                *size == self.plan.expected_size && *sha256 == self.plan.expected_sha256
            });

            let mut failures = Vec::new();
            if timed_out {
                failures.push(FailureBoundary::Timeout);
            }
            if !timed_out && !digest_matches {
                failures.push(FailureBoundary::Consumption);
            }
            if !settled {
                failures.push(FailureBoundary::Settlement);
            }

            let cleanup_failed = self.cleanup_fails_once;
            self.cleanup_fails_once = false;
            if cleanup_failed {
                failures.push(FailureBoundary::Cleanup);
                self.phase = Phase::RetainedCleanupFailed;
            } else {
                self.owned_bytes.take();
                self.phase = Phase::Closed;
            }

            let successful_probe = !timed_out && settled && digest_matches;
            let successful = successful_probe && !cleanup_failed;
            let outcome = PublicOutcome {
                disposition: if successful {
                    Disposition::PrototypeConsumed
                } else {
                    Disposition::Failed
                },
                profile_id: self.plan.profile.id(),
                binding_id: self.plan.binding_id.clone(),
                failures,
                acquisition: BoundaryState::Passed,
                consumption: if timed_out {
                    BoundaryState::TimedOut
                } else if digest_matches {
                    BoundaryState::Passed
                } else {
                    BoundaryState::Failed
                },
                timeout: if timed_out {
                    BoundaryState::TimedOut
                } else {
                    BoundaryState::NotTriggered
                },
                settlement: if settled {
                    BoundaryState::Passed
                } else {
                    BoundaryState::Failed
                },
                cleanup: if cleanup_failed {
                    BoundaryState::Failed
                } else {
                    BoundaryState::Passed
                },
                residue: BoundaryState::Passed,
                observed_size: observed.as_ref().map(|(size, _)| *size),
                observed_sha256: observed.map(|(_, sha256)| hex_digest(sha256)),
                fixed_probe_completed: successful_probe,
                package_bytes_executed: false,
                package_installed_or_staged: false,
                native_proven: false,
            };
            Ok(Completion {
                seal: Arc::clone(&self.seal),
                outcome,
            })
        }

        pub(super) fn close(&mut self) -> Result<(), AuthorityError> {
            match self.phase {
                Phase::Acquired | Phase::RetainedCleanupFailed => {
                    self.owned_bytes.take();
                    self.phase = Phase::Closed;
                    Ok(())
                }
                Phase::Closed => Err(AuthorityError {
                    boundary: FailureBoundary::Authority,
                    message: "authority is already closed",
                }),
                Phase::Consuming | Phase::Settling => Err(AuthorityError {
                    boundary: FailureBoundary::Authority,
                    message: "authority cannot close before settlement",
                }),
            }
        }

        pub(super) fn accepts_completion(&self, completion: &Completion) -> bool {
            Arc::ptr_eq(&self.seal, &completion.seal)
        }

        pub(super) fn retains_owned_bytes(&self) -> bool {
            self.owned_bytes.is_some()
        }
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
}

use authority::{
    acquire, cleanup_failure_plan, closed_plan, timeout_plan, AuthorityError, BoundaryState,
    Completion, Disposition, FailureBoundary, NativeAuthority, Profile,
};

fn scratch_root(name: &str, bytes: &[u8]) -> PathBuf {
    let safe_name = name.replace([':', '_'], "-");
    let root = std::env::temp_dir().join(format!(
        "batcave-native-authority-test-{}-{safe_name}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir(&root).expect("create test root");
    fs::write(root.join("selected-artifact.bin"), bytes).expect("write test artifact");
    root
}

fn remove_root(root: &Path) {
    fs::remove_dir_all(root).expect("remove test root");
}

fn expected_digest() -> String {
    let digest: [u8; 32] = Sha256::digest(PAYLOAD).into();
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[test]
fn every_closed_profile_uses_the_fixed_native_memory_probe() {
    let profiles = [
        Profile::WindowsNsis,
        Profile::LinuxDeb,
        Profile::LinuxAppImage,
        Profile::MacOsDmg,
        Profile::MacOsUpdater,
    ];

    for profile in profiles {
        let root = scratch_root(profile_id(profile), PAYLOAD);
        let mut authority = acquire(closed_plan(profile, PAYLOAD), &root).expect("acquire");
        let completion = authority.consume().expect("consume");
        assert!(authority.accepts_completion(&completion));
        let outcome = completion.outcome();
        assert_eq!(outcome.disposition(), Disposition::PrototypeConsumed);
        assert_eq!(outcome.profile_id(), profile_id(profile));
        assert_eq!(
            outcome.binding_id(),
            format!("prototype-{}", profile_id(profile).replace([':', '_'], "-"))
        );
        assert!(outcome.failures().is_empty());
        assert_eq!(
            outcome.boundary(FailureBoundary::Acquisition),
            BoundaryState::Passed
        );
        assert_eq!(
            outcome.boundary(FailureBoundary::Consumption),
            BoundaryState::Passed
        );
        assert_eq!(outcome.observed_size(), Some(PAYLOAD.len()));
        assert_eq!(outcome.observed_sha256(), Some(expected_digest().as_str()));
        assert!(outcome.fixed_probe_completed());
        assert!(!outcome.claims_native_execution());
        assert!(!authority.retains_owned_bytes());
        remove_root(&root);
    }
}

#[test]
fn original_path_replacement_cannot_change_the_owned_bytes() {
    let root = scratch_root("source-replacement", PAYLOAD);
    let source = root.join("selected-artifact.bin");
    let mut authority =
        acquire(closed_plan(Profile::LinuxAppImage, PAYLOAD), &root).expect("acquire");
    fs::rename(&source, root.join("selected-artifact.original")).expect("move original");
    fs::write(&source, b"hostile replacement").expect("replace source");

    let completion = authority.consume().expect("consume");
    assert_eq!(
        completion.outcome().disposition(),
        Disposition::PrototypeConsumed
    );
    assert_eq!(
        completion.outcome().observed_sha256(),
        Some(expected_digest().as_str())
    );
    remove_root(&root);
}

#[test]
fn linked_or_mismatched_sources_fail_before_authority_exists() {
    let wrong_root = scratch_root("wrong-bytes", b"wrong bytes");
    let error = acquire(closed_plan(Profile::LinuxDeb, PAYLOAD), &wrong_root)
        .err()
        .expect("wrong bytes fail");
    assert_eq!(error.boundary(), FailureBoundary::Acquisition);
    assert_eq!(
        error.message(),
        "selected artifact does not match the closed binding"
    );
    remove_root(&wrong_root);

    #[cfg(unix)]
    {
        use std::os::unix::fs::symlink;
        let real_root = scratch_root("real-root", PAYLOAD);
        let linked_root = real_root.with_extension("linked");
        let _ = fs::remove_file(&linked_root);
        symlink(&real_root, &linked_root).expect("link root");
        let error = acquire(closed_plan(Profile::MacOsDmg, PAYLOAD), &linked_root)
            .err()
            .expect("linked root fails");
        assert_eq!(error.boundary(), FailureBoundary::Acquisition);
        assert_eq!(
            error.message(),
            "verified root must be a non-link directory"
        );
        fs::remove_file(linked_root).expect("remove link");
        remove_root(&real_root);
    }
}

#[test]
fn replay_and_early_close_fail_closed() {
    let root = scratch_root("replay", PAYLOAD);
    let mut authority =
        acquire(closed_plan(Profile::WindowsNsis, PAYLOAD), &root).expect("acquire");
    authority.consume().expect("first consume");
    let replay = authority.consume().err().expect("replay fails");
    assert_eq!(replay.boundary(), FailureBoundary::Authority);

    let mut closed = acquire(closed_plan(Profile::MacOsUpdater, PAYLOAD), &root).expect("acquire");
    closed.close().expect("close before use");
    let early_close = closed.consume().err().expect("closed authority fails");
    assert_eq!(early_close.boundary(), FailureBoundary::Authority);
    remove_root(&root);
}

#[test]
fn timeout_is_derived_only_after_the_fixed_worker_settles() {
    let root = scratch_root("timeout", PAYLOAD);
    let mut authority = acquire(timeout_plan(Profile::LinuxDeb, PAYLOAD), &root).expect("acquire");
    let completion = authority.consume().expect("derive timeout completion");
    let outcome = completion.outcome();
    assert_eq!(outcome.disposition(), Disposition::Failed);
    assert_eq!(outcome.failures(), &[FailureBoundary::Timeout]);
    assert_eq!(
        outcome.boundary(FailureBoundary::Timeout),
        BoundaryState::TimedOut
    );
    assert_eq!(
        outcome.boundary(FailureBoundary::Settlement),
        BoundaryState::Passed
    );
    assert!(!outcome.fixed_probe_completed());
    assert!(!outcome.claims_native_execution());
    assert!(!authority.retains_owned_bytes());
    remove_root(&root);
}

#[test]
fn cleanup_failure_retains_bytes_and_cannot_overwrite_the_probe_result() {
    let root = scratch_root("cleanup", PAYLOAD);
    let mut authority =
        acquire(cleanup_failure_plan(Profile::MacOsDmg, PAYLOAD), &root).expect("acquire");
    let completion = authority.consume().expect("derive cleanup completion");
    let outcome = completion.outcome();
    assert_eq!(outcome.disposition(), Disposition::Failed);
    assert_eq!(outcome.failures(), &[FailureBoundary::Cleanup]);
    assert_eq!(
        outcome.boundary(FailureBoundary::Cleanup),
        BoundaryState::Failed
    );
    assert_eq!(
        outcome.boundary(FailureBoundary::Residue),
        BoundaryState::Passed
    );
    assert!(outcome.fixed_probe_completed());
    assert!(!outcome.claims_native_execution());
    assert!(authority.retains_owned_bytes());
    authority.close().expect("retry retained cleanup");
    assert!(!authority.retains_owned_bytes());
    remove_root(&root);
}

#[test]
fn completion_is_bound_to_the_exact_native_authority() {
    let first_root = scratch_root("first-completion", PAYLOAD);
    let second_root = scratch_root("second-completion", PAYLOAD);
    let mut first =
        acquire(closed_plan(Profile::LinuxDeb, PAYLOAD), &first_root).expect("first acquire");
    let second =
        acquire(closed_plan(Profile::LinuxDeb, PAYLOAD), &second_root).expect("second acquire");
    let completion = first.consume().expect("first completion");
    assert!(first.accepts_completion(&completion));
    assert!(!second.accepts_completion(&completion));
    remove_root(&first_root);
    remove_root(&second_root);
}

#[test]
fn completion_debug_surface_contains_no_path_handle_receipt_or_proof() {
    let root = scratch_root("sanitized", PAYLOAD);
    let mut authority =
        acquire(closed_plan(Profile::MacOsUpdater, PAYLOAD), &root).expect("acquire");
    let completion = authority.consume().expect("completion");
    let rendered = format!("{:?}", completion.outcome());
    assert!(!rendered.contains(root.to_string_lossy().as_ref()));
    assert!(!rendered.contains("native_execution_receipt"));
    assert!(!rendered.contains("evidence_packet"));
    assert!(!completion.outcome().claims_native_execution());
    remove_root(&root);
}

#[test]
fn consume_surface_accepts_no_caller_execution_or_completion_input() {
    let consume: fn(&mut NativeAuthority) -> Result<Completion, AuthorityError> =
        NativeAuthority::consume;
    let root = scratch_root("closed-signature", PAYLOAD);
    let mut authority = acquire(closed_plan(Profile::LinuxDeb, PAYLOAD), &root).expect("acquire");
    let completion = consume(&mut authority).expect("fixed consume");
    assert!(authority.accepts_completion(&completion));
    remove_root(&root);
}

fn profile_id(profile: Profile) -> &'static str {
    match profile {
        Profile::WindowsNsis => "windows:nsis",
        Profile::LinuxDeb => "linux:deb",
        Profile::LinuxAppImage => "linux:appimage",
        Profile::MacOsDmg => "macos:dmg",
        Profile::MacOsUpdater => "macos:macos_updater",
    }
}
