use serde::Serialize;
use sha2::{Digest, Sha256};
use std::marker::PhantomData;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Weak};

mod operation {
    use super::*;

    const DEB_FIXTURE: &[u8] = b"inert deb complete-operation fixture\n";
    const APPIMAGE_FIXTURE: &[u8] = b"inert AppImage complete-operation fixture\n";
    static OPERATION_SEQUENCE: AtomicU64 = AtomicU64::new(1);

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

        fn package_operation(self) -> &'static str {
            match self {
                Self::Deb => "install",
                Self::AppImage => "stage",
            }
        }

        fn transport(self) -> ClosedTransport {
            match self {
                Self::Deb => ClosedTransport::DebDescriptor,
                Self::AppImage => ClosedTransport::AppImageDescriptor,
            }
        }

        fn package_limitation(self) -> Limitation {
            match self {
                Self::Deb => Limitation::DebChecksumAttestationOnly,
                Self::AppImage => Limitation::AppImageUpdaterTrustNotExercised,
            }
        }
    }

    #[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
    #[serde(rename_all = "snake_case")]
    enum ClosedTransport {
        DebDescriptor,
        AppImageDescriptor,
    }

    impl ClosedTransport {
        fn id(self) -> &'static str {
            match self {
                Self::DebDescriptor => "sealed_descriptor_to_fixed_deb_consumer",
                Self::AppImageDescriptor => "sealed_descriptor_to_fixed_appimage_consumer",
            }
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
        Partial,
        Blocked,
    }

    #[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
    #[serde(rename_all = "snake_case")]
    enum OutcomeCode {
        InertOwnedBytesBound,
        InertGateObserved,
        FixedFixtureFailed,
        BlockedByEarlierGate,
        ResidueRetained,
        CleanupRetained,
        SettlementRetained,
        RecoverySettled,
    }

    #[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
    #[serde(rename_all = "snake_case")]
    pub(super) enum Limitation {
        InertCapabilityOnly,
        PackageCommandNotRun,
        DebChecksumAttestationOnly,
        AppImageUpdaterTrustNotExercised,
    }

    #[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
    #[serde(rename_all = "snake_case")]
    pub(super) enum Disposition {
        SourceContractComplete,
        Failed,
    }

    #[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
    #[serde(rename_all = "snake_case")]
    pub(super) enum RetainedReason {
        Residue,
        CleanupFailed,
        ProcessUnsettled,
    }

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    pub(super) enum HostileFixture {
        DigestMismatch,
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
    pub(super) struct OperationOutcome {
        proof_scope: &'static str,
        profile_id: &'static str,
        package_operation: &'static str,
        transport: &'static str,
        disposition: Disposition,
        retained_reason: Option<RetainedReason>,
        consumed_process_local_capability: bool,
        package_bytes_executed: bool,
        process_tree_settled: bool,
        private_root_removed: bool,
        residue_detected: bool,
        gates: Vec<GateResult>,
        limitations: Vec<Limitation>,
        public_artifact_verified: bool,
        native_proven: bool,
        release_evidence: Option<serde_json::Value>,
    }

    impl OperationOutcome {
        fn acquisition_failed(profile: LinuxProfile) -> Self {
            Self {
                proof_scope: "linux_owned_complete_operation_source_contract",
                profile_id: profile.id(),
                package_operation: profile.package_operation(),
                transport: profile.transport().id(),
                disposition: Disposition::Failed,
                retained_reason: None,
                consumed_process_local_capability: false,
                package_bytes_executed: false,
                process_tree_settled: true,
                private_root_removed: true,
                residue_detected: false,
                gates: ORDERED_GATES
                    .into_iter()
                    .map(|id| GateResult {
                        id,
                        status: GateStatus::Blocked,
                        outcome: OutcomeCode::BlockedByEarlierGate,
                    })
                    .collect(),
                limitations: limitations(profile),
                public_artifact_verified: false,
                native_proven: false,
                release_evidence: None,
            }
        }

        pub(super) fn profile_id(&self) -> &'static str {
            self.profile_id
        }

        pub(super) fn package_operation(&self) -> &'static str {
            self.package_operation
        }

        pub(super) fn transport(&self) -> &'static str {
            self.transport
        }

        pub(super) fn disposition(&self) -> Disposition {
            self.disposition
        }

        pub(super) fn retained_reason(&self) -> Option<RetainedReason> {
            self.retained_reason
        }

        pub(super) fn consumed_process_local_capability(&self) -> bool {
            self.consumed_process_local_capability
        }

        pub(super) fn gate_ids(&self) -> Vec<&'static str> {
            self.gates.iter().map(|gate| gate.id.id()).collect()
        }

        pub(super) fn status(&self, id: GateId) -> GateStatus {
            self.gates
                .iter()
                .find(|gate| gate.id == id)
                .expect("fixed complete operation contains every gate")
                .status
        }

        pub(super) fn process_tree_settled(&self) -> bool {
            self.process_tree_settled
        }

        pub(super) fn private_root_removed(&self) -> bool {
            self.private_root_removed
        }

        pub(super) fn residue_detected(&self) -> bool {
            self.residue_detected
        }

        pub(super) fn limitations(&self) -> &[Limitation] {
            &self.limitations
        }

        pub(super) fn claims_native_proof(&self) -> bool {
            self.public_artifact_verified || self.native_proven || self.release_evidence.is_some()
        }

        pub(super) fn sanitized_json(&self) -> String {
            serde_json::to_string(self).expect("complete-operation outcome serializes")
        }
    }

    fn limitations(profile: LinuxProfile) -> Vec<Limitation> {
        vec![
            Limitation::InertCapabilityOnly,
            Limitation::PackageCommandNotRun,
            profile.package_limitation(),
        ]
    }

    struct ConsumptionSeal {
        sequence: u64,
    }

    struct OwnershipToken {
        kind: &'static str,
        settled_before_release: Arc<AtomicBool>,
    }

    pub(super) struct OwnershipWitness {
        artifact: Weak<OwnershipToken>,
        process: Weak<OwnershipToken>,
        root: Weak<OwnershipToken>,
        settlement: [Arc<AtomicBool>; 3],
    }

    impl OwnershipWitness {
        pub(super) fn live_kinds(&self) -> Vec<&'static str> {
            [&self.artifact, &self.process, &self.root]
                .into_iter()
                .filter_map(Weak::upgrade)
                .map(|token| token.kind)
                .collect()
        }

        pub(super) fn all_released(&self) -> bool {
            self.live_kinds().is_empty()
        }

        pub(super) fn all_settled_before_release(&self) -> bool {
            self.settlement
                .iter()
                .all(|settled| settled.load(Ordering::Acquire))
        }
    }

    pub(super) struct OwnedFixtureCapability {
        profile: LinuxProfile,
        expected_size: usize,
        expected_sha256: [u8; 32],
        bytes: Arc<[u8]>,
        seal: Arc<ConsumptionSeal>,
        scenario: Scenario,
        artifact_token: Arc<OwnershipToken>,
        process_token: Arc<OwnershipToken>,
        root_token: Arc<OwnershipToken>,
    }

    struct ConsumedArtifact {
        profile: LinuxProfile,
        bytes: Arc<[u8]>,
        sha256: [u8; 32],
        seal: Arc<ConsumptionSeal>,
        scenario: Scenario,
        artifact_token: Arc<OwnershipToken>,
        process_token: Arc<OwnershipToken>,
        root_token: Arc<OwnershipToken>,
    }

    impl OwnedFixtureCapability {
        fn consume(self) -> Result<ConsumedArtifact, LinuxProfile> {
            let observed_sha256: [u8; 32] = Sha256::digest(self.bytes.as_ref()).into();
            if self.bytes.len() != self.expected_size || observed_sha256 != self.expected_sha256 {
                for token in [&self.artifact_token, &self.process_token, &self.root_token] {
                    token.settled_before_release.store(true, Ordering::Release);
                }
                return Err(self.profile);
            }
            Ok(ConsumedArtifact {
                profile: self.profile,
                bytes: self.bytes,
                sha256: observed_sha256,
                seal: self.seal,
                scenario: self.scenario,
                artifact_token: self.artifact_token,
                process_token: self.process_token,
                root_token: self.root_token,
            })
        }
    }

    fn capability(
        profile: LinuxProfile,
        scenario: Scenario,
    ) -> (OwnedFixtureCapability, OwnershipWitness) {
        let bytes: Arc<[u8]> = Arc::from(profile.fixture());
        let artifact_settled = Arc::new(AtomicBool::new(false));
        let process_settled = Arc::new(AtomicBool::new(false));
        let root_settled = Arc::new(AtomicBool::new(false));
        let artifact_token = Arc::new(OwnershipToken {
            kind: "artifact",
            settled_before_release: Arc::clone(&artifact_settled),
        });
        let process_token = Arc::new(OwnershipToken {
            kind: "process",
            settled_before_release: Arc::clone(&process_settled),
        });
        let root_token = Arc::new(OwnershipToken {
            kind: "root",
            settled_before_release: Arc::clone(&root_settled),
        });
        let mut expected_sha256: [u8; 32] = Sha256::digest(bytes.as_ref()).into();
        if matches!(scenario, Scenario::Hostile(HostileFixture::DigestMismatch)) {
            expected_sha256[0] ^= 0xff;
        }
        let witness = OwnershipWitness {
            artifact: Arc::downgrade(&artifact_token),
            process: Arc::downgrade(&process_token),
            root: Arc::downgrade(&root_token),
            settlement: [artifact_settled, process_settled, root_settled],
        };
        (
            OwnedFixtureCapability {
                profile,
                expected_size: bytes.len(),
                expected_sha256,
                bytes,
                seal: Arc::new(ConsumptionSeal {
                    sequence: OPERATION_SEQUENCE.fetch_add(1, Ordering::Relaxed),
                }),
                scenario,
                artifact_token,
                process_token,
                root_token,
            },
            witness,
        )
    }

    pub(super) fn owned_fixture_capability(
        profile: LinuxProfile,
    ) -> (OwnedFixtureCapability, OwnershipWitness) {
        capability(profile, Scenario::Clean)
    }

    pub(super) fn hostile_fixture_capability(
        profile: LinuxProfile,
        fixture: HostileFixture,
    ) -> (OwnedFixtureCapability, OwnershipWitness) {
        capability(profile, Scenario::Hostile(fixture))
    }

    struct OwnedProcessAuthority {
        token: Arc<OwnershipToken>,
        settled: bool,
    }

    struct OwnedRootAuthority {
        token: Arc<OwnershipToken>,
        removed: bool,
    }

    struct OperationAuthority {
        artifact: ConsumedArtifact,
        transport: ClosedTransport,
        process: OwnedProcessAuthority,
        root: OwnedRootAuthority,
        gates: Vec<GateResult>,
        runtime_failed: bool,
        residue_detected: bool,
        retained_reason: Option<RetainedReason>,
    }

    impl OperationAuthority {
        fn begin(artifact: ConsumedArtifact) -> Self {
            let transport = artifact.profile.transport();
            let process_token = Arc::clone(&artifact.process_token);
            let root_token = Arc::clone(&artifact.root_token);
            Self {
                artifact,
                transport,
                process: OwnedProcessAuthority {
                    token: process_token,
                    settled: false,
                },
                root: OwnedRootAuthority {
                    token: root_token,
                    removed: false,
                },
                gates: Vec::with_capacity(ORDERED_GATES.len()),
                runtime_failed: false,
                residue_detected: false,
                retained_reason: None,
            }
        }

        fn scenario(&self) -> Scenario {
            self.artifact.scenario
        }

        fn push(&mut self, id: GateId, status: GateStatus, outcome: OutcomeCode) {
            assert_eq!(
                ORDERED_GATES[self.gates.len()],
                id,
                "complete operation cannot reorder or omit a gate"
            );
            self.gates.push(GateResult {
                id,
                status,
                outcome,
            });
        }

        fn runtime_gate(&mut self, id: GateId, fail_on: Option<HostileFixture>) {
            if self.runtime_failed {
                self.push(
                    id,
                    GateStatus::Blocked,
                    OutcomeCode::BlockedByEarlierGate,
                );
            } else if fail_on.is_some_and(|fixture| {
                matches!(self.scenario(), Scenario::Hostile(observed) if observed == fixture)
            }) {
                self.runtime_failed = true;
                self.push(id, GateStatus::Failed, OutcomeCode::FixedFixtureFailed);
            } else {
                self.push(id, GateStatus::Passed, OutcomeCode::InertGateObserved);
            }
        }

        fn outcome(&self, disposition: Disposition) -> OperationOutcome {
            let observed_sha256: [u8; 32] = Sha256::digest(self.artifact.bytes.as_ref()).into();
            let consumed_process_local_capability = self.artifact.seal.sequence > 0
                && !self.artifact.bytes.is_empty()
                && self.artifact.sha256 == observed_sha256
                && Arc::strong_count(&self.artifact.artifact_token) >= 1
                && Arc::strong_count(&self.process.token) >= 1
                && Arc::strong_count(&self.root.token) >= 1;
            OperationOutcome {
                proof_scope: "linux_owned_complete_operation_source_contract",
                profile_id: self.artifact.profile.id(),
                package_operation: self.artifact.profile.package_operation(),
                transport: self.transport.id(),
                disposition,
                retained_reason: self.retained_reason,
                consumed_process_local_capability,
                package_bytes_executed: false,
                process_tree_settled: self.process.settled,
                private_root_removed: self.root.removed,
                residue_detected: self.residue_detected,
                gates: self
                    .gates
                    .iter()
                    .map(|gate| GateResult {
                        id: gate.id,
                        status: gate.status,
                        outcome: gate.outcome,
                    })
                    .collect(),
                limitations: limitations(self.artifact.profile),
                public_artifact_verified: false,
                native_proven: false,
                release_evidence: None,
            }
        }

        fn replace_gate(&mut self, id: GateId, status: GateStatus, outcome: OutcomeCode) {
            let gate = self
                .gates
                .iter_mut()
                .find(|gate| gate.id == id)
                .expect("recovery updates an already-observed gate");
            gate.status = status;
            gate.outcome = outcome;
        }

        fn mark_authority_settled(&self) {
            assert!(self.process.settled, "process authority is not settled");
            assert!(self.root.removed, "root authority is not settled");
            assert!(!self.residue_detected, "residue authority is not settled");
            for token in [
                &self.artifact.artifact_token,
                &self.process.token,
                &self.root.token,
            ] {
                token.settled_before_release.store(true, Ordering::Release);
            }
        }

        fn force_retained_cleanup(&mut self) {
            self.residue_detected = false;
            self.process.settled = true;
            self.root.removed = true;
            self.retained_reason = None;
            self.mark_authority_settled();
        }
    }

    struct PackageOperation;
    struct Launch;
    struct ReleaseIdentity;
    struct Settings;
    struct Degradation;
    struct Telemetry;
    struct Removal;
    struct RuntimeCleanup;
    struct UserState;
    struct Complete;

    struct Pipeline<State> {
        authority: OperationAuthority,
        state: PhantomData<State>,
    }

    impl Pipeline<PackageOperation> {
        fn begin(artifact: ConsumedArtifact) -> Self {
            Self {
                authority: OperationAuthority::begin(artifact),
                state: PhantomData,
            }
        }

        fn package_operation(mut self) -> Pipeline<Launch> {
            self.authority.push(
                GateId::PackageInstall,
                GateStatus::Passed,
                OutcomeCode::InertOwnedBytesBound,
            );
            Pipeline {
                authority: self.authority,
                state: PhantomData,
            }
        }
    }

    impl Pipeline<Launch> {
        fn launch(mut self) -> Pipeline<ReleaseIdentity> {
            self.authority.runtime_gate(GateId::Launch, None);
            Pipeline {
                authority: self.authority,
                state: PhantomData,
            }
        }
    }

    impl Pipeline<ReleaseIdentity> {
        fn release_identity(mut self) -> Pipeline<Settings> {
            self.authority.runtime_gate(
                GateId::ReleaseIdentity,
                Some(HostileFixture::FailReleaseIdentity),
            );
            Pipeline {
                authority: self.authority,
                state: PhantomData,
            }
        }
    }

    impl Pipeline<Settings> {
        fn settings_restart(mut self) -> Pipeline<Degradation> {
            self.authority.runtime_gate(GateId::Settings, None);
            Pipeline {
                authority: self.authority,
                state: PhantomData,
            }
        }
    }

    impl Pipeline<Degradation> {
        fn permission_limited_degradation(mut self) -> Pipeline<Telemetry> {
            self.authority.runtime_gate(GateId::Degradation, None);
            Pipeline {
                authority: self.authority,
                state: PhantomData,
            }
        }
    }

    impl Pipeline<Telemetry> {
        fn telemetry(mut self) -> Pipeline<Removal> {
            self.authority.runtime_gate(GateId::Telemetry, None);
            Pipeline {
                authority: self.authority,
                state: PhantomData,
            }
        }
    }

    impl Pipeline<Removal> {
        fn removal(mut self) -> Pipeline<RuntimeCleanup> {
            if matches!(
                self.authority.scenario(),
                Scenario::Hostile(HostileFixture::ResidueAfterRemoval)
            ) {
                self.authority.residue_detected = true;
                self.authority.retained_reason = Some(RetainedReason::Residue);
                self.authority.push(
                    GateId::ApplicationRemoved,
                    GateStatus::Failed,
                    OutcomeCode::ResidueRetained,
                );
            } else {
                self.authority.push(
                    GateId::ApplicationRemoved,
                    GateStatus::Passed,
                    OutcomeCode::InertGateObserved,
                );
            }
            Pipeline {
                authority: self.authority,
                state: PhantomData,
            }
        }
    }

    impl Pipeline<RuntimeCleanup> {
        fn process_cleanup(mut self) -> Pipeline<UserState> {
            match self.authority.scenario() {
                Scenario::Hostile(HostileFixture::UnsettledProcess) => {
                    self.authority.retained_reason = Some(RetainedReason::ProcessUnsettled);
                    self.authority.push(
                        GateId::OwnedRuntimeCleanup,
                        GateStatus::Partial,
                        OutcomeCode::SettlementRetained,
                    );
                }
                Scenario::Hostile(HostileFixture::CleanupFailure) => {
                    self.authority.process.settled = true;
                    self.authority.retained_reason = Some(RetainedReason::CleanupFailed);
                    self.authority.push(
                        GateId::OwnedRuntimeCleanup,
                        GateStatus::Failed,
                        OutcomeCode::CleanupRetained,
                    );
                }
                _ => {
                    self.authority.process.settled = true;
                    if self.authority.residue_detected {
                        self.authority.push(
                            GateId::OwnedRuntimeCleanup,
                            GateStatus::Partial,
                            OutcomeCode::ResidueRetained,
                        );
                    } else {
                        self.authority.root.removed = true;
                        self.authority.push(
                            GateId::OwnedRuntimeCleanup,
                            GateStatus::Passed,
                            OutcomeCode::InertGateObserved,
                        );
                    }
                }
            }
            Pipeline {
                authority: self.authority,
                state: PhantomData,
            }
        }
    }

    impl Pipeline<UserState> {
        fn user_state_policy(mut self) -> Pipeline<Complete> {
            if self.authority.process.settled && self.authority.root.removed {
                self.authority.push(
                    GateId::UserStatePolicy,
                    GateStatus::Passed,
                    OutcomeCode::InertGateObserved,
                );
            } else {
                self.authority.push(
                    GateId::UserStatePolicy,
                    GateStatus::Blocked,
                    OutcomeCode::BlockedByEarlierGate,
                );
            }
            Pipeline {
                authority: self.authority,
                state: PhantomData,
            }
        }
    }

    #[must_use = "a retained operation still owns artifact, process, and root recovery authority"]
    pub(super) struct RetainedOperation {
        authority: OperationAuthority,
    }

    impl RetainedOperation {
        pub(super) fn outcome(&self) -> OperationOutcome {
            self.authority.outcome(Disposition::Failed)
        }

        pub(super) fn retry_cleanup(mut self) -> OperationOutcome {
            if self.authority.residue_detected {
                self.authority.residue_detected = false;
                self.authority.replace_gate(
                    GateId::ApplicationRemoved,
                    GateStatus::Passed,
                    OutcomeCode::RecoverySettled,
                );
            }
            self.authority.process.settled = true;
            self.authority.root.removed = true;
            self.authority.replace_gate(
                GateId::OwnedRuntimeCleanup,
                GateStatus::Passed,
                OutcomeCode::RecoverySettled,
            );
            self.authority.replace_gate(
                GateId::UserStatePolicy,
                GateStatus::Passed,
                OutcomeCode::RecoverySettled,
            );
            self.authority.retained_reason = None;
            self.authority.mark_authority_settled();
            self.authority.outcome(Disposition::Failed)
        }
    }

    impl Drop for RetainedOperation {
        fn drop(&mut self) {
            if self.authority.retained_reason.is_some() {
                self.authority.force_retained_cleanup();
            }
        }
    }

    pub(super) enum CompleteOperationResult {
        Settled(OperationOutcome),
        Retained(RetainedOperation),
    }

    impl CompleteOperationResult {
        pub(super) fn settled(self) -> OperationOutcome {
            match self {
                Self::Settled(outcome) => outcome,
                Self::Retained(_) => panic!("operation still retains recovery authority"),
            }
        }

        pub(super) fn retained(self) -> RetainedOperation {
            match self {
                Self::Settled(_) => panic!("operation settled without retained authority"),
                Self::Retained(retained) => retained,
            }
        }
    }

    impl Pipeline<Complete> {
        fn finish(self) -> CompleteOperationResult {
            let clean_contract = !self.authority.runtime_failed
                && !self.authority.residue_detected
                && self.authority.process.settled
                && self.authority.root.removed;
            if self.authority.process.settled
                && self.authority.root.removed
                && self.authority.retained_reason.is_none()
            {
                self.authority.mark_authority_settled();
                CompleteOperationResult::Settled(self.authority.outcome(if clean_contract {
                    Disposition::SourceContractComplete
                } else {
                    Disposition::Failed
                }))
            } else {
                CompleteOperationResult::Retained(RetainedOperation {
                    authority: self.authority,
                })
            }
        }
    }

    pub(super) fn run_complete_operation(
        capability: OwnedFixtureCapability,
    ) -> CompleteOperationResult {
        let artifact = match capability.consume() {
            Ok(artifact) => artifact,
            Err(profile) => {
                return CompleteOperationResult::Settled(OperationOutcome::acquisition_failed(
                    profile,
                ));
            }
        };
        Pipeline::begin(artifact)
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
}

use operation::{
    hostile_fixture_capability, owned_fixture_capability, run_complete_operation, Disposition,
    GateId, GateStatus, HostileFixture, Limitation, LinuxProfile, RetainedReason, ORDERED_GATES,
};

#[test]
fn complete_entry_binds_both_closed_transports_to_the_exact_gate_order() {
    for (profile, profile_id, package_operation, transport, package_limitation) in [
        (
            LinuxProfile::Deb,
            "linux:deb",
            "install",
            "sealed_descriptor_to_fixed_deb_consumer",
            Limitation::DebChecksumAttestationOnly,
        ),
        (
            LinuxProfile::AppImage,
            "linux:appimage",
            "stage",
            "sealed_descriptor_to_fixed_appimage_consumer",
            Limitation::AppImageUpdaterTrustNotExercised,
        ),
    ] {
        let (capability, witness) = owned_fixture_capability(profile);
        let outcome = run_complete_operation(capability).settled();
        assert_eq!(outcome.profile_id(), profile_id);
        assert_eq!(outcome.package_operation(), package_operation);
        assert_eq!(outcome.transport(), transport);
        assert_eq!(outcome.disposition(), Disposition::SourceContractComplete);
        assert_eq!(outcome.gate_ids(), ORDERED_GATES.map(GateId::id));
        assert!(ORDERED_GATES
            .iter()
            .all(|gate| outcome.status(*gate) == GateStatus::Passed));
        assert!(outcome.consumed_process_local_capability());
        assert!(outcome.process_tree_settled());
        assert!(outcome.private_root_removed());
        assert!(!outcome.residue_detected());
        assert!(outcome.limitations().contains(&package_limitation));
        assert!(outcome
            .limitations()
            .contains(&Limitation::InertCapabilityOnly));
        assert!(outcome
            .limitations()
            .contains(&Limitation::PackageCommandNotRun));
        assert!(!outcome.claims_native_proof());
        assert!(witness.all_released());
        assert!(witness.all_settled_before_release());
    }
}

#[test]
fn capability_rehash_failure_stops_before_transport_or_gate_execution() {
    let (capability, witness) =
        hostile_fixture_capability(LinuxProfile::Deb, HostileFixture::DigestMismatch);
    let outcome = run_complete_operation(capability).settled();
    assert_eq!(outcome.disposition(), Disposition::Failed);
    assert!(!outcome.consumed_process_local_capability());
    assert!(ORDERED_GATES
        .iter()
        .all(|gate| outcome.status(*gate) == GateStatus::Blocked));
    assert!(!outcome.claims_native_proof());
    assert!(witness.all_released());
    assert!(witness.all_settled_before_release());
}

#[test]
fn runtime_failure_still_removes_and_settles_every_owned_resource() {
    let (capability, witness) =
        hostile_fixture_capability(LinuxProfile::Deb, HostileFixture::FailReleaseIdentity);
    let outcome = run_complete_operation(capability).settled();
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
    assert!(outcome.private_root_removed());
    assert!(!outcome.claims_native_proof());
    assert!(witness.all_released());
    assert!(witness.all_settled_before_release());
}

#[test]
fn unsettled_process_retains_all_authority_until_explicit_recovery() {
    let (capability, witness) =
        hostile_fixture_capability(LinuxProfile::AppImage, HostileFixture::UnsettledProcess);
    let retained = run_complete_operation(capability).retained();
    let pending = retained.outcome();
    assert_eq!(
        pending.retained_reason(),
        Some(RetainedReason::ProcessUnsettled)
    );
    assert_eq!(
        pending.status(GateId::OwnedRuntimeCleanup),
        GateStatus::Partial
    );
    assert_eq!(pending.status(GateId::UserStatePolicy), GateStatus::Blocked);
    assert!(!pending.process_tree_settled());
    assert!(!pending.private_root_removed());
    assert_eq!(witness.live_kinds(), ["artifact", "process", "root"]);
    assert!(!pending.claims_native_proof());

    let recovered = retained.retry_cleanup();
    assert!(recovered.process_tree_settled());
    assert!(recovered.private_root_removed());
    assert_eq!(recovered.retained_reason(), None);
    assert_eq!(
        recovered.status(GateId::OwnedRuntimeCleanup),
        GateStatus::Passed
    );
    assert_eq!(
        recovered.status(GateId::UserStatePolicy),
        GateStatus::Passed
    );
    assert!(!recovered.claims_native_proof());
    assert!(witness.all_released());
    assert!(witness.all_settled_before_release());
}

#[test]
fn residue_and_cleanup_failure_each_retain_the_same_closed_recovery_owner() {
    for (fixture, reason, failed_gate) in [
        (
            HostileFixture::ResidueAfterRemoval,
            RetainedReason::Residue,
            GateId::ApplicationRemoved,
        ),
        (
            HostileFixture::CleanupFailure,
            RetainedReason::CleanupFailed,
            GateId::OwnedRuntimeCleanup,
        ),
    ] {
        let (capability, witness) = hostile_fixture_capability(LinuxProfile::Deb, fixture);
        let retained = run_complete_operation(capability).retained();
        let pending = retained.outcome();
        assert_eq!(pending.retained_reason(), Some(reason));
        assert!(matches!(
            pending.status(failed_gate),
            GateStatus::Failed | GateStatus::Partial
        ));
        assert_eq!(witness.live_kinds(), ["artifact", "process", "root"]);
        assert!(!pending.claims_native_proof());

        let recovered = retained.retry_cleanup();
        assert!(recovered.process_tree_settled());
        assert!(recovered.private_root_removed());
        assert!(!recovered.residue_detected());
        assert!(!recovered.claims_native_proof());
        assert!(witness.all_released());
        assert!(witness.all_settled_before_release());
    }
}

#[test]
fn dropping_a_retained_result_takes_the_fixed_settlement_path() {
    let (capability, witness) =
        hostile_fixture_capability(LinuxProfile::AppImage, HostileFixture::UnsettledProcess);
    let retained = run_complete_operation(capability).retained();
    assert_eq!(witness.live_kinds(), ["artifact", "process", "root"]);
    assert!(!witness.all_settled_before_release());

    drop(retained);

    assert!(witness.all_released());
    assert!(witness.all_settled_before_release());
}

#[test]
fn result_is_sanitized_and_hard_wires_public_and_native_proof_false() {
    let (capability, _) = owned_fixture_capability(LinuxProfile::AppImage);
    let outcome = run_complete_operation(capability).settled();
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
        "\"artifact_path\":",
        "\"caller_status\":",
        "\"evidence_input\":",
    ] {
        assert!(
            !rendered.contains(forbidden),
            "leaked forbidden surface: {forbidden}"
        );
    }
    assert!(rendered.contains("\"package_bytes_executed\":false"));
    assert!(rendered.contains("\"public_artifact_verified\":false"));
    assert!(rendered.contains("\"native_proven\":false"));
    assert!(rendered.contains("\"release_evidence\":null"));
    assert!(rendered.contains("app_image_updater_trust_not_exercised"));
    assert!(!outcome.claims_native_proof());
}

#[test]
fn entry_has_no_caller_operation_seam_and_remains_outside_production() {
    let entry: fn(operation::OwnedFixtureCapability) -> operation::CompleteOperationResult =
        run_complete_operation;
    let (capability, _) = owned_fixture_capability(LinuxProfile::Deb);
    assert_eq!(
        entry(capability).settled().disposition(),
        Disposition::SourceContractComplete
    );

    let source = include_str!("linux_owned_complete_operation.rs");
    for forbidden in [
        concat!("std::process::", "Command"),
        concat!("std::process::", "Stdio"),
        concat!("std::env::", "vars"),
        concat!("std::path::", "Path"),
        concat!("std::", "fs::"),
    ] {
        assert!(
            !source.contains(forbidden),
            "unexpected caller-replaceable operation seam: {forbidden}"
        );
    }
    let production = include_str!("../src/lib.rs");
    assert!(!production.contains("linux_owned_complete_operation"));
}
