use serde::Serialize;
use sha2::{Digest, Sha256};
use std::marker::PhantomData;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

mod pipeline {
    use super::*;

    const DEB_FIXTURE: &[u8] = b"inert deb operation fixture\n";
    const APPIMAGE_FIXTURE: &[u8] = b"inert AppImage operation fixture\n";
    static CAPABILITY_SEQUENCE: AtomicU64 = AtomicU64::new(1);

    #[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
    #[serde(rename_all = "snake_case")]
    pub(super) enum LinuxProfile {
        Deb,
        AppImage,
    }

    impl LinuxProfile {
        fn id(self) -> &'static str {
            match self {
                Self::Deb => "linux:deb",
                Self::AppImage => "linux:appimage",
            }
        }

        fn fixture(self) -> &'static [u8] {
            match self {
                Self::Deb => DEB_FIXTURE,
                Self::AppImage => APPIMAGE_FIXTURE,
            }
        }

        fn operation(self) -> &'static str {
            match self {
                Self::Deb => "install",
                Self::AppImage => "stage",
            }
        }

        fn limitations(self) -> Vec<Limitation> {
            let package = match self {
                Self::Deb => Limitation::DebChecksumAttestationOnly,
                Self::AppImage => Limitation::AppImageUpdaterTrustNotExercised,
            };
            vec![
                Limitation::InertFixtureOnly,
                Limitation::PackageCommandNotRun,
                package,
            ]
        }
    }

    #[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
    #[serde(rename_all = "snake_case")]
    pub(super) enum GateId {
        PackageInstall,
        Launch,
        ReleaseIdentity,
        Settings,
        Degradation,
        Telemetry,
        ApplicationRemoved,
        OwnedRuntimeCleanup,
        UserStatePolicy,
    }

    impl GateId {
        pub(super) fn id(self) -> &'static str {
            match self {
                Self::PackageInstall => "install.package_install",
                Self::Launch => "runtime.launch",
                Self::ReleaseIdentity => "runtime.release_identity",
                Self::Settings => "runtime.settings",
                Self::Degradation => "runtime.degradation",
                Self::Telemetry => "runtime.telemetry",
                Self::ApplicationRemoved => "cleanup.application_removed",
                Self::OwnedRuntimeCleanup => "cleanup.owned_runtime_cleanup",
                Self::UserStatePolicy => "cleanup.user_state_policy",
            }
        }

        fn is_runtime(self) -> bool {
            matches!(
                self,
                Self::PackageInstall
                    | Self::Launch
                    | Self::ReleaseIdentity
                    | Self::Settings
                    | Self::Degradation
                    | Self::Telemetry
            )
        }
    }

    pub(super) const ORDERED_GATES: [GateId; 9] = [
        GateId::PackageInstall,
        GateId::Launch,
        GateId::ReleaseIdentity,
        GateId::Settings,
        GateId::Degradation,
        GateId::Telemetry,
        GateId::ApplicationRemoved,
        GateId::OwnedRuntimeCleanup,
        GateId::UserStatePolicy,
    ];

    #[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
    #[serde(rename_all = "snake_case")]
    pub(super) enum GateStatus {
        Passed,
        Failed,
        Skipped,
        Partial,
        Blocked,
    }

    #[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
    #[serde(rename_all = "snake_case")]
    enum OutcomeCode {
        InertFixtureObserved,
        FixedFixtureSkipped,
        FixedFixtureFailed,
        BlockedByEarlierGate,
        ResidueDetected,
        CleanupFailed,
        SettlementUnconfirmed,
    }

    #[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
    #[serde(rename_all = "snake_case")]
    pub(super) enum Limitation {
        InertFixtureOnly,
        PackageCommandNotRun,
        DebChecksumAttestationOnly,
        AppImageUpdaterTrustNotExercised,
    }

    #[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
    #[serde(rename_all = "snake_case")]
    pub(super) enum Disposition {
        ContractComplete,
        Skipped,
        Failed,
    }

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    pub(super) enum HostileFixture {
        SkipLaunch,
        FailReleaseIdentity,
        ResidueAfterRemoval,
        CleanupFailure,
        UnsettledProcess,
    }

    #[derive(Clone, Copy)]
    enum Scenario {
        Clean,
        Hostile(HostileFixture),
    }

    #[derive(Debug, Serialize)]
    struct GateResult {
        id: GateId,
        status: GateStatus,
        outcome: OutcomeCode,
    }

    #[derive(Debug, Serialize)]
    pub(super) struct PipelineOutcome {
        proof_scope: &'static str,
        profile_id: &'static str,
        package_operation: &'static str,
        disposition: Disposition,
        consumed_process_local_capability: bool,
        process_tree_settled: bool,
        residue_detected: bool,
        gates: Vec<GateResult>,
        limitations: Vec<Limitation>,
        public_artifact_verified: bool,
        native_proven: bool,
        release_evidence: Option<serde_json::Value>,
    }

    impl PipelineOutcome {
        pub(super) fn profile_id(&self) -> &'static str {
            self.profile_id
        }

        pub(super) fn package_operation(&self) -> &'static str {
            self.package_operation
        }

        pub(super) fn disposition(&self) -> Disposition {
            self.disposition
        }

        pub(super) fn gate_ids(&self) -> Vec<&'static str> {
            self.gates.iter().map(|gate| gate.id.id()).collect()
        }

        pub(super) fn status(&self, id: GateId) -> GateStatus {
            self.gates
                .iter()
                .find(|gate| gate.id == id)
                .expect("fixed pipeline contains every gate")
                .status
        }

        pub(super) fn limitations(&self) -> &[Limitation] {
            &self.limitations
        }

        pub(super) fn process_tree_settled(&self) -> bool {
            self.process_tree_settled
        }

        pub(super) fn residue_detected(&self) -> bool {
            self.residue_detected
        }

        pub(super) fn consumed_process_local_capability(&self) -> bool {
            self.consumed_process_local_capability
        }

        pub(super) fn claims_native_proof(&self) -> bool {
            self.public_artifact_verified || self.native_proven || self.release_evidence.is_some()
        }

        pub(super) fn sanitized_json(&self) -> String {
            serde_json::to_string(self).expect("fixed pipeline outcome serializes")
        }
    }

    struct ConsumptionSeal {
        sequence: u64,
    }

    pub(super) struct OwnedFixtureCapability {
        profile: LinuxProfile,
        expected_size: usize,
        expected_sha256: [u8; 32],
        bytes: Arc<[u8]>,
        seal: Arc<ConsumptionSeal>,
    }

    pub(super) struct ConsumedArtifact {
        profile: LinuxProfile,
        seal: Arc<ConsumptionSeal>,
    }

    pub(super) fn owned_fixture_capability(profile: LinuxProfile) -> OwnedFixtureCapability {
        let fixture = profile.fixture();
        OwnedFixtureCapability {
            profile,
            expected_size: fixture.len(),
            expected_sha256: Sha256::digest(fixture).into(),
            bytes: Arc::from(fixture),
            seal: Arc::new(ConsumptionSeal {
                sequence: CAPABILITY_SEQUENCE.fetch_add(1, Ordering::Relaxed),
            }),
        }
    }

    impl OwnedFixtureCapability {
        pub(super) fn consume(self) -> ConsumedArtifact {
            let observed_size = self.bytes.len();
            let observed_sha256: [u8; 32] = Sha256::digest(&self.bytes).into();
            assert_eq!(
                observed_size, self.expected_size,
                "fixture size is capability-bound"
            );
            assert_eq!(
                observed_sha256, self.expected_sha256,
                "fixture digest is capability-bound"
            );
            ConsumedArtifact {
                profile: self.profile,
                seal: self.seal,
            }
        }
    }

    struct PackageOperation;
    struct Launch;
    struct ReleaseIdentity;
    struct Settings;
    struct Degradation;
    struct Telemetry;
    struct ApplicationRemoved;
    struct OwnedRuntimeCleanup;
    struct UserStatePolicy;
    struct Complete;

    struct Pipeline<Stage> {
        artifact: ConsumedArtifact,
        scenario: Scenario,
        gates: Vec<GateResult>,
        runtime_blocked: bool,
        process_tree_settled: bool,
        residue_detected: bool,
        _stage: PhantomData<Stage>,
    }

    impl Pipeline<PackageOperation> {
        fn begin(artifact: ConsumedArtifact, scenario: Scenario) -> Self {
            Self {
                artifact,
                scenario,
                gates: Vec::with_capacity(ORDERED_GATES.len()),
                runtime_blocked: false,
                process_tree_settled: true,
                residue_detected: false,
                _stage: PhantomData,
            }
        }
    }

    impl<Stage> Pipeline<Stage> {
        fn transition<Next>(mut self, gate: GateId) -> Pipeline<Next> {
            let (status, outcome) = self.observe(gate);
            if gate.is_runtime() && !matches!(status, GateStatus::Passed) {
                self.runtime_blocked = true;
            }
            self.gates.push(GateResult {
                id: gate,
                status,
                outcome,
            });
            Pipeline {
                artifact: self.artifact,
                scenario: self.scenario,
                gates: self.gates,
                runtime_blocked: self.runtime_blocked,
                process_tree_settled: self.process_tree_settled,
                residue_detected: self.residue_detected,
                _stage: PhantomData,
            }
        }

        fn observe(&mut self, gate: GateId) -> (GateStatus, OutcomeCode) {
            if gate.is_runtime() && self.runtime_blocked {
                return (GateStatus::Blocked, OutcomeCode::BlockedByEarlierGate);
            }

            match (self.scenario, gate) {
                (Scenario::Hostile(HostileFixture::SkipLaunch), GateId::Launch) => {
                    (GateStatus::Skipped, OutcomeCode::FixedFixtureSkipped)
                }
                (
                    Scenario::Hostile(HostileFixture::FailReleaseIdentity),
                    GateId::ReleaseIdentity,
                ) => (GateStatus::Failed, OutcomeCode::FixedFixtureFailed),
                (
                    Scenario::Hostile(HostileFixture::ResidueAfterRemoval),
                    GateId::ApplicationRemoved,
                ) => {
                    self.residue_detected = true;
                    (GateStatus::Failed, OutcomeCode::ResidueDetected)
                }
                (
                    Scenario::Hostile(HostileFixture::CleanupFailure),
                    GateId::OwnedRuntimeCleanup,
                ) => (GateStatus::Failed, OutcomeCode::CleanupFailed),
                (
                    Scenario::Hostile(HostileFixture::UnsettledProcess),
                    GateId::OwnedRuntimeCleanup,
                ) => {
                    self.process_tree_settled = false;
                    (GateStatus::Partial, OutcomeCode::SettlementUnconfirmed)
                }
                (_, GateId::UserStatePolicy) if !self.process_tree_settled => {
                    (GateStatus::Blocked, OutcomeCode::BlockedByEarlierGate)
                }
                _ => (GateStatus::Passed, OutcomeCode::InertFixtureObserved),
            }
        }
    }

    macro_rules! stage {
        ($current:ty, $next:ty, $method:ident, $gate:expr) => {
            impl Pipeline<$current> {
                fn $method(self) -> Pipeline<$next> {
                    self.transition($gate)
                }
            }
        };
    }

    stage!(
        PackageOperation,
        Launch,
        package_operation,
        GateId::PackageInstall
    );
    stage!(Launch, ReleaseIdentity, launch, GateId::Launch);
    stage!(
        ReleaseIdentity,
        Settings,
        release_identity,
        GateId::ReleaseIdentity
    );
    stage!(Settings, Degradation, settings_restart, GateId::Settings);
    stage!(
        Degradation,
        Telemetry,
        permission_limited_degradation,
        GateId::Degradation
    );
    stage!(Telemetry, ApplicationRemoved, telemetry, GateId::Telemetry);
    stage!(
        ApplicationRemoved,
        OwnedRuntimeCleanup,
        removal,
        GateId::ApplicationRemoved
    );
    stage!(
        OwnedRuntimeCleanup,
        UserStatePolicy,
        process_cleanup,
        GateId::OwnedRuntimeCleanup
    );
    stage!(
        UserStatePolicy,
        Complete,
        user_state_policy,
        GateId::UserStatePolicy
    );

    impl Pipeline<Complete> {
        fn finish(self) -> PipelineOutcome {
            assert_eq!(self.gates.len(), ORDERED_GATES.len());
            assert!(
                self.gates
                    .iter()
                    .zip(ORDERED_GATES)
                    .all(|(result, expected)| result.id == expected),
                "typestate pipeline preserves the fixed gate order"
            );
            let has_failure = self
                .gates
                .iter()
                .any(|gate| matches!(gate.status, GateStatus::Failed | GateStatus::Partial))
                || self.residue_detected
                || !self.process_tree_settled;
            let has_skip = self
                .gates
                .iter()
                .any(|gate| matches!(gate.status, GateStatus::Skipped | GateStatus::Blocked));
            let disposition = if has_failure {
                Disposition::Failed
            } else if has_skip {
                Disposition::Skipped
            } else {
                Disposition::ContractComplete
            };
            let process_local =
                self.artifact.seal.sequence > 0 && Arc::strong_count(&self.artifact.seal) == 1;
            PipelineOutcome {
                proof_scope: "inert_linux_gate_pipeline_contract_only",
                profile_id: self.artifact.profile.id(),
                package_operation: self.artifact.profile.operation(),
                disposition,
                consumed_process_local_capability: process_local,
                process_tree_settled: self.process_tree_settled,
                residue_detected: self.residue_detected,
                gates: self.gates,
                limitations: self.artifact.profile.limitations(),
                public_artifact_verified: false,
                native_proven: false,
                release_evidence: None,
            }
        }
    }

    fn run(artifact: ConsumedArtifact, scenario: Scenario) -> PipelineOutcome {
        Pipeline::begin(artifact, scenario)
            .package_operation()
            .launch()
            .release_identity()
            .settings_restart()
            .permission_limited_degradation()
            .telemetry()
            .removal()
            .process_cleanup()
            .user_state_policy()
            .finish()
    }

    pub(super) fn run_inert(artifact: ConsumedArtifact) -> PipelineOutcome {
        run(artifact, Scenario::Clean)
    }

    pub(super) fn run_hostile(
        artifact: ConsumedArtifact,
        fixture: HostileFixture,
    ) -> PipelineOutcome {
        run(artifact, Scenario::Hostile(fixture))
    }
}

use pipeline::{
    owned_fixture_capability, run_hostile, run_inert, Disposition, GateId, GateStatus,
    HostileFixture, Limitation, LinuxProfile, ORDERED_GATES,
};

fn consumed(profile: LinuxProfile) -> pipeline::ConsumedArtifact {
    owned_fixture_capability(profile).consume()
}

#[test]
fn deb_and_appimage_follow_the_exact_closed_gate_order() {
    for (profile, profile_id, operation, package_limitation) in [
        (
            LinuxProfile::Deb,
            "linux:deb",
            "install",
            Limitation::DebChecksumAttestationOnly,
        ),
        (
            LinuxProfile::AppImage,
            "linux:appimage",
            "stage",
            Limitation::AppImageUpdaterTrustNotExercised,
        ),
    ] {
        let outcome = run_inert(consumed(profile));
        assert_eq!(outcome.profile_id(), profile_id);
        assert_eq!(outcome.package_operation(), operation);
        assert_eq!(outcome.disposition(), Disposition::ContractComplete);
        assert_eq!(
            outcome.gate_ids(),
            ORDERED_GATES.map(GateId::id),
            "launch through user-state policy cannot be reordered or omitted"
        );
        assert!(ORDERED_GATES
            .iter()
            .all(|gate| outcome.status(*gate) == GateStatus::Passed));
        assert!(outcome.consumed_process_local_capability());
        assert!(outcome.process_tree_settled());
        assert!(!outcome.residue_detected());
        assert!(outcome
            .limitations()
            .contains(&Limitation::InertFixtureOnly));
        assert!(outcome
            .limitations()
            .contains(&Limitation::PackageCommandNotRun));
        assert!(outcome.limitations().contains(&package_limitation));
        assert!(!outcome.claims_native_proof());
    }
}

#[test]
fn skipped_launch_blocks_runtime_observations_but_still_runs_cleanup() {
    let outcome = run_hostile(consumed(LinuxProfile::AppImage), HostileFixture::SkipLaunch);
    assert_eq!(outcome.disposition(), Disposition::Skipped);
    assert_eq!(outcome.status(GateId::Launch), GateStatus::Skipped);
    for gate in [
        GateId::ReleaseIdentity,
        GateId::Settings,
        GateId::Degradation,
        GateId::Telemetry,
    ] {
        assert_eq!(outcome.status(gate), GateStatus::Blocked);
    }
    for gate in [
        GateId::ApplicationRemoved,
        GateId::OwnedRuntimeCleanup,
        GateId::UserStatePolicy,
    ] {
        assert_eq!(outcome.status(gate), GateStatus::Passed);
    }
    assert!(!outcome.claims_native_proof());
}

#[test]
fn a_runtime_failure_cannot_skip_removal_or_cleanup() {
    let outcome = run_hostile(
        consumed(LinuxProfile::Deb),
        HostileFixture::FailReleaseIdentity,
    );
    assert_eq!(outcome.disposition(), Disposition::Failed);
    assert_eq!(outcome.status(GateId::ReleaseIdentity), GateStatus::Failed);
    for gate in [GateId::Settings, GateId::Degradation, GateId::Telemetry] {
        assert_eq!(outcome.status(gate), GateStatus::Blocked);
    }
    for gate in [
        GateId::ApplicationRemoved,
        GateId::OwnedRuntimeCleanup,
        GateId::UserStatePolicy,
    ] {
        assert_eq!(outcome.status(gate), GateStatus::Passed);
    }
    assert!(outcome.process_tree_settled());
    assert!(!outcome.claims_native_proof());
}

#[test]
fn removal_residue_is_distinct_and_blocks_proof_after_cleanup_settles() {
    let outcome = run_hostile(
        consumed(LinuxProfile::Deb),
        HostileFixture::ResidueAfterRemoval,
    );
    assert_eq!(outcome.disposition(), Disposition::Failed);
    assert_eq!(
        outcome.status(GateId::ApplicationRemoved),
        GateStatus::Failed
    );
    assert!(outcome.residue_detected());
    assert_eq!(
        outcome.status(GateId::OwnedRuntimeCleanup),
        GateStatus::Passed
    );
    assert_eq!(outcome.status(GateId::UserStatePolicy), GateStatus::Passed);
    assert!(!outcome.claims_native_proof());
}

#[test]
fn cleanup_failure_and_unsettled_process_remain_distinct() {
    let cleanup_failed = run_hostile(
        consumed(LinuxProfile::AppImage),
        HostileFixture::CleanupFailure,
    );
    assert_eq!(cleanup_failed.disposition(), Disposition::Failed);
    assert_eq!(
        cleanup_failed.status(GateId::OwnedRuntimeCleanup),
        GateStatus::Failed
    );
    assert!(cleanup_failed.process_tree_settled());
    assert_eq!(
        cleanup_failed.status(GateId::UserStatePolicy),
        GateStatus::Passed
    );

    let unsettled = run_hostile(
        consumed(LinuxProfile::AppImage),
        HostileFixture::UnsettledProcess,
    );
    assert_eq!(unsettled.disposition(), Disposition::Failed);
    assert_eq!(
        unsettled.status(GateId::OwnedRuntimeCleanup),
        GateStatus::Partial
    );
    assert!(!unsettled.process_tree_settled());
    assert_eq!(
        unsettled.status(GateId::UserStatePolicy),
        GateStatus::Blocked
    );
    assert!(!cleanup_failed.claims_native_proof());
    assert!(!unsettled.claims_native_proof());
}

#[test]
fn outcome_is_sanitized_and_cannot_carry_caller_evidence() {
    let outcome = run_hostile(
        consumed(LinuxProfile::Deb),
        HostileFixture::FailReleaseIdentity,
    );
    let rendered = outcome.sanitized_json();
    for forbidden in [
        "/Users/",
        "/home/",
        "/tmp/",
        "C:\\\\",
        "PATH=",
        "TOKEN=",
        "raw_output",
        "\"command\":",
        "\"environment\":",
    ] {
        assert!(
            !rendered.contains(forbidden),
            "leaked forbidden surface: {forbidden}"
        );
    }
    assert!(rendered.contains("\"release_evidence\":null"));
    assert!(rendered.contains("deb_checksum_attestation_only"));
    assert!(!outcome.claims_native_proof());
}

#[test]
fn entry_accepts_only_a_consumed_process_local_capability() {
    let entry: fn(pipeline::ConsumedArtifact) -> pipeline::PipelineOutcome = run_inert;
    let outcome = entry(consumed(LinuxProfile::Deb));
    assert_eq!(outcome.disposition(), Disposition::ContractComplete);
    assert!(outcome.consumed_process_local_capability());
    assert!(!outcome.claims_native_proof());
}

#[test]
fn prototype_has_no_command_path_environment_or_production_entry() {
    let source = include_str!("linux_native_gate_pipeline.rs");
    for forbidden in [
        concat!("std::process::", "Command"),
        concat!("std::process::", "Stdio"),
        concat!("std::env::", "vars"),
        concat!("std::path::", "Path"),
        concat!("std::", "fs::"),
    ] {
        assert!(
            !source.contains(forbidden),
            "unexpected authority surface: {forbidden}"
        );
    }

    let production = include_str!("../src/lib.rs");
    assert!(!production.contains("linux_native_gate_pipeline"));
}
