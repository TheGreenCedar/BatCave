mod desktop;
#[allow(dead_code)] // The fail-closed export contract is wired by the remaining lifecycle stages.
mod evidence;
mod lifecycle;
mod native;
mod private_evidence;

use crate::windows_lifecycle_proof_contract::{
    message_sha256, parse_plan, plan_sha256, validate_desktop_phase_result, validate_envelope,
    validate_locator, validate_nonce, validate_sha256, AbortReason, ClosedRequest, DesktopPhase,
    DesktopPhaseDisposition, DesktopPhaseResult, Envelope, EvidenceReceipt, LifecycleStage,
    Observation, ParentMessage, ProofPlan, RestorationOutcome, SequenceGate, WorkerAbort,
    WorkerCheckpoint, WorkerFailureKind, WorkerMessage, WorkerResult, PROTOCOL_SCHEMA,
    SUCCESS_PRIVATE_EVIDENCE_LEAVES,
};
use native::{OwnedFile, PipeConnection, PreflightSnapshot};
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

const SESSION_TIMEOUT: Duration = Duration::from_secs(45 * 60);
const ABORT_TIMEOUT: Duration = Duration::from_secs(12 * 60);
const DESKTOP_PHASES: [DesktopPhase; 6] = [
    DesktopPhase::FinalPrimary,
    DesktopPhase::BaselinePrimary,
    DesktopPhase::BaselineSecondInstance,
    DesktopPhase::FinalMissingService,
    DesktopPhase::FinalStoppedService,
    DesktopPhase::FinalIncompatibleService,
];

#[derive(Debug)]
enum Entry {
    Preflight,
    Run,
    Worker { locator: String },
}

#[derive(Serialize)]
#[serde(rename_all = "snake_case")]
struct ControllerOutcome {
    disposition: &'static str,
    reason: Option<String>,
    worker_failure_kind: Option<WorkerFailureKind>,
    attempted_stage: Option<LifecycleStage>,
    failure_evidence: Option<EvidenceReceipt>,
    failure_evidence_verified: Option<bool>,
    evidence_error: Option<String>,
    restoration: Option<RestorationOutcome>,
    abort: Option<WorkerAbort>,
    restoration_evidence_verified: Option<bool>,
    parent_followup_error: Option<String>,
    private_evidence: Option<Vec<EvidenceReceipt>>,
    sanitized_export: Option<EvidenceReceipt>,
    success_evidence_verified: Option<bool>,
    profile: Option<String>,
    controller_source_commit_sha: Option<String>,
    evidence_root: Option<String>,
    preflight: Option<PreflightSnapshot>,
}

struct ParentPreflight {
    plan: ProofPlan,
    repo_root: PathBuf,
    controller: OwnedFile,
    baseline: OwnedFile,
    final_candidate: OwnedFile,
    incompatible_service_fixture: OwnedFile,
    rollback_failing_service_fixture: OwnedFile,
    parent_current_user: native::ParentCurrentUserAuthorityGuard,
    snapshot: PreflightSnapshot,
    source_commit_sha: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum UnverifiedAbortSettlement {
    WorkerExited,
    ForcedSettled,
    ForcedUnsettled,
}

struct ProtocolGates {
    outbound: SequenceGate,
    inbound: SequenceGate,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct CrossedWorkerRequest {
    completed_stage: LifecycleStage,
    attempted_stage: Option<LifecycleStage>,
}

struct PendingFailedDesktopResult {
    result: DesktopPhaseResult,
    completion_write_confirmed: bool,
}

#[derive(Default)]
struct ParentCurrentUserRetentionState {
    before_uninstall: Option<native::ParentCurrentUserObjectsGuard>,
    after_uninstall: Option<native::ParentCurrentUserObjectsGuard>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ParentRetentionCheckpointAction {
    None,
    CaptureBefore,
    CaptureAfter,
}

fn parent_retention_checkpoint_action(
    stage: LifecycleStage,
    has_before: bool,
    has_after: bool,
) -> Result<ParentRetentionCheckpointAction, String> {
    match stage {
        LifecycleStage::FinalFallbackStates if has_before || has_after => {
            Err("lifecycle_parent_user_before_uninstall_replayed".to_string())
        }
        LifecycleStage::FinalFallbackStates => Ok(ParentRetentionCheckpointAction::CaptureBefore),
        LifecycleStage::FinalUninstall if !has_before => {
            Err("lifecycle_parent_user_before_uninstall_missing".to_string())
        }
        LifecycleStage::FinalUninstall if has_after => {
            Err("lifecycle_parent_user_after_uninstall_replayed".to_string())
        }
        LifecycleStage::FinalUninstall => Ok(ParentRetentionCheckpointAction::CaptureAfter),
        _ => Ok(ParentRetentionCheckpointAction::None),
    }
}

impl ParentCurrentUserRetentionState {
    fn capture_checkpoint(
        &mut self,
        stage: LifecycleStage,
        root: &native::ParentCurrentUserAuthorityGuard,
    ) -> Result<(), String> {
        match parent_retention_checkpoint_action(
            stage,
            self.before_uninstall.is_some(),
            self.after_uninstall.is_some(),
        )? {
            ParentRetentionCheckpointAction::CaptureBefore => {
                self.before_uninstall = Some(native::capture_parent_current_user_objects(root)?);
            }
            ParentRetentionCheckpointAction::CaptureAfter => {
                let before = self
                    .before_uninstall
                    .as_ref()
                    .ok_or_else(|| "lifecycle_parent_user_before_uninstall_missing".to_string())?;
                before.revalidate()?;
                let after = native::capture_parent_current_user_objects(root)?;
                native::validate_parent_current_user_objects_preserved(
                    before.authority(),
                    after.authority(),
                )?;
                self.after_uninstall = Some(after);
            }
            ParentRetentionCheckpointAction::None => {}
        }
        Ok(())
    }

    fn complete(
        &self,
        root: &native::ParentCurrentUserAuthorityGuard,
    ) -> Result<
        (
            &native::ParentCurrentUserObjects,
            &native::ParentCurrentUserObjects,
        ),
        String,
    > {
        root.revalidate()?;
        let before = self
            .before_uninstall
            .as_ref()
            .ok_or_else(|| "lifecycle_parent_user_before_uninstall_missing".to_string())?;
        let after = self
            .after_uninstall
            .as_ref()
            .ok_or_else(|| "lifecycle_parent_user_after_uninstall_missing".to_string())?;
        before.revalidate()?;
        after.revalidate()?;
        native::validate_parent_current_user_objects_preserved(
            before.authority(),
            after.authority(),
        )?;
        Ok((before.authority(), after.authority()))
    }

    fn revalidate_terminal(
        &self,
        root: &native::ParentCurrentUserAuthorityGuard,
    ) -> Result<(), String> {
        let (_, after) = self.complete(root)?;
        let current = native::capture_parent_current_user_objects(root)?;
        current.revalidate()?;
        if current.authority() != after {
            return Err("lifecycle_parent_user_retention_terminal_drift".to_string());
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ParentResidueExpectation {
    Clean,
    SeededKnownHelpers,
    SeededHelpersRemoved,
    FinalUninstalled,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ParentResidueCleanupAction {
    Nothing,
    Cleanup,
    BlockedUnsettled,
}

fn parent_residue_cleanup_action(
    has_transaction: bool,
    worker_tree_settled: bool,
) -> ParentResidueCleanupAction {
    match (has_transaction, worker_tree_settled) {
        (false, _) => ParentResidueCleanupAction::Nothing,
        (true, true) => ParentResidueCleanupAction::Cleanup,
        (true, false) => ParentResidueCleanupAction::BlockedUnsettled,
    }
}

fn finalize_parent_residue_terminal<T>(
    transaction: &mut Option<T>,
    worker_tree_settled: bool,
    cleanup: impl FnOnce(&mut T) -> Result<(), String>,
) -> Result<(), String> {
    match parent_residue_cleanup_action(transaction.is_some(), worker_tree_settled) {
        ParentResidueCleanupAction::Nothing => return Ok(()),
        ParentResidueCleanupAction::BlockedUnsettled => {
            return Err("lifecycle_parent_user_cleanup_blocked_unsettled".to_string());
        }
        ParentResidueCleanupAction::Cleanup => {}
    }
    let pending = transaction
        .as_mut()
        .ok_or_else(|| "lifecycle_parent_user_cleanup_transaction_missing".to_string())?;
    cleanup(pending)?;
    *transaction = None;
    Ok(())
}

#[derive(Default)]
struct ParentCurrentUserResidueState {
    timeline: native::ParentCurrentUserResidueTimeline,
    sentinel_anchor: Option<native::ParentHelperFileSnapshot>,
    run_key_anchor: Option<(String, String, String)>,
    helper_root_anchor: Option<(native::FileIdentity, String, String)>,
    seed_transaction: Option<native::ParentCurrentUserResidueTransaction>,
}

impl ParentCurrentUserResidueState {
    fn capture_checkpoint(
        &mut self,
        stage: LifecycleStage,
        root: &native::ParentCurrentUserAuthorityGuard,
    ) -> Result<(), String> {
        let expectation = checkpoint_parent_residue_expectation(stage);
        let snapshot = native::capture_parent_current_user_residue(root)?;
        validate_parent_residue_snapshot(
            &snapshot,
            expectation,
            root.authority(),
            self.sentinel_anchor.as_ref(),
        )?;
        self.validate_authority_anchors(&snapshot)?;
        self.timeline.insert(
            native::ParentCurrentUserCapturePoint::Checkpoint(stage),
            snapshot,
        )?;
        if stage == LifecycleStage::BaselineRollbackRecovery {
            if self.seed_transaction.is_some() {
                return Err("lifecycle_parent_user_seed_transaction_replayed".to_string());
            }
            match native::seed_parent_current_user_legacy_residue(root) {
                Ok(transaction) => self.seed_transaction = Some(transaction),
                Err(mut failure) => {
                    if let Some(transaction) = failure.transaction.take() {
                        self.seed_transaction = Some(*transaction);
                    }
                    return Err(failure.reason);
                }
            }
            let seeded = native::capture_parent_current_user_residue(root)?;
            validate_parent_residue_snapshot(
                &seeded,
                ParentResidueExpectation::SeededKnownHelpers,
                root.authority(),
                None,
            )?;
            self.validate_authority_anchors(&seeded)?;
            self.sentinel_anchor = Some(parent_residue_sentinel(&seeded)?.clone());
            self.timeline.insert(
                native::ParentCurrentUserCapturePoint::BaselineRollbackRecoverySeeded,
                seeded,
            )?;
        }
        Ok(())
    }

    fn capture_before_desktop(
        &mut self,
        phase: DesktopPhase,
        root: &native::ParentCurrentUserAuthorityGuard,
    ) -> Result<(), String> {
        if phase != DesktopPhase::FinalMissingService {
            return Ok(());
        }
        let snapshot = native::capture_parent_current_user_residue(root)?;
        validate_parent_residue_snapshot(
            &snapshot,
            ParentResidueExpectation::SeededKnownHelpers,
            root.authority(),
            self.sentinel_anchor.as_ref(),
        )?;
        self.validate_authority_anchors(&snapshot)?;
        self.timeline.insert(
            native::ParentCurrentUserCapturePoint::FinalMissingServiceBeforeDesktop,
            snapshot,
        )
    }

    fn capture_desktop_complete(
        &mut self,
        phase: DesktopPhase,
        root: &native::ParentCurrentUserAuthorityGuard,
    ) -> Result<(), String> {
        let expectation = desktop_parent_residue_expectation(phase);
        let snapshot = native::capture_parent_current_user_residue(root)?;
        validate_parent_residue_snapshot(
            &snapshot,
            expectation,
            root.authority(),
            self.sentinel_anchor.as_ref(),
        )?;
        self.validate_authority_anchors(&snapshot)?;
        self.timeline.insert(
            native::ParentCurrentUserCapturePoint::DesktopComplete(phase),
            snapshot,
        )
    }

    fn complete(
        &self,
        root: &native::ParentCurrentUserAuthorityGuard,
    ) -> Result<&native::ParentCurrentUserResidueTimeline, String> {
        root.revalidate()?;
        for stage in [
            LifecycleStage::InitialState,
            LifecycleStage::FinalRepair,
            LifecycleStage::InitialUninstall,
            LifecycleStage::BaselineInstall,
            LifecycleStage::BaselineRestart,
            LifecycleStage::BaselineCrashRecovery,
            LifecycleStage::BaselineRollbackRecovery,
            LifecycleStage::LegacyResidueSeeded,
            LifecycleStage::FinalUpgrade,
            LifecycleStage::FinalRestart,
            LifecycleStage::FinalCrashRecovery,
            LifecycleStage::FinalFallbackStates,
            LifecycleStage::FinalUninstall,
        ] {
            self.timeline
                .get(native::ParentCurrentUserCapturePoint::Checkpoint(stage))?;
        }
        self.timeline
            .get(native::ParentCurrentUserCapturePoint::BaselineRollbackRecoverySeeded)?;
        self.timeline
            .get(native::ParentCurrentUserCapturePoint::FinalMissingServiceBeforeDesktop)?;
        for phase in DESKTOP_PHASES {
            self.timeline
                .get(native::ParentCurrentUserCapturePoint::DesktopComplete(
                    phase,
                ))?;
        }
        Ok(&self.timeline)
    }

    fn revalidate_final_uninstall(
        &mut self,
        root: &native::ParentCurrentUserAuthorityGuard,
    ) -> Result<(), String> {
        root.revalidate()?;
        let expected = self
            .timeline
            .get(native::ParentCurrentUserCapturePoint::Checkpoint(
                LifecycleStage::FinalUninstall,
            ))?
            .clone();
        let current = native::capture_parent_current_user_residue(root)?;
        validate_parent_residue_snapshot(
            &current,
            ParentResidueExpectation::FinalUninstalled,
            root.authority(),
            self.sentinel_anchor.as_ref(),
        )?;
        self.validate_authority_anchors(&current)?;
        if current != expected {
            return Err("lifecycle_parent_user_final_uninstall_drift".to_string());
        }
        Ok(())
    }

    fn cleanup_after_worker_settlement(
        &mut self,
        root: &native::ParentCurrentUserAuthorityGuard,
        worker_tree_settled: bool,
    ) -> Result<(), String> {
        finalize_parent_residue_terminal(
            &mut self.seed_transaction,
            worker_tree_settled,
            |transaction| native::cleanup_parent_current_user_legacy_residue(root, transaction),
        )
    }

    fn validate_authority_anchors(
        &mut self,
        snapshot: &native::ParentCurrentUserResidueSnapshot,
    ) -> Result<(), String> {
        let run_key = (
            snapshot.hkcu_run.final_key_path.clone(),
            snapshot.hkcu_run.owner_sid.clone(),
            snapshot.hkcu_run.dacl_sha256.clone(),
        );
        if self
            .run_key_anchor
            .as_ref()
            .is_some_and(|anchor| anchor != &run_key)
        {
            return Err("lifecycle_parent_user_run_key_authority_changed".to_string());
        }
        self.run_key_anchor.get_or_insert(run_key);

        if let Observation::Present(helper) = &snapshot.helper {
            let root = (
                helper.root.identity,
                helper.root_owner_sid.clone(),
                helper.root_dacl_sha256.clone(),
            );
            if self
                .helper_root_anchor
                .as_ref()
                .is_some_and(|anchor| anchor != &root)
            {
                return Err("lifecycle_parent_user_helper_root_authority_changed".to_string());
            }
            self.helper_root_anchor.get_or_insert(root);
        }
        Ok(())
    }
}

fn checkpoint_parent_residue_expectation(stage: LifecycleStage) -> ParentResidueExpectation {
    match stage {
        LifecycleStage::InitialState
        | LifecycleStage::FinalRepair
        | LifecycleStage::InitialUninstall
        | LifecycleStage::BaselineInstall
        | LifecycleStage::BaselineRestart
        | LifecycleStage::BaselineCrashRecovery
        | LifecycleStage::BaselineRollbackRecovery => ParentResidueExpectation::Clean,
        LifecycleStage::LegacyResidueSeeded
        | LifecycleStage::FinalUpgrade
        | LifecycleStage::FinalRestart
        | LifecycleStage::FinalCrashRecovery => ParentResidueExpectation::SeededKnownHelpers,
        LifecycleStage::FinalFallbackStates => ParentResidueExpectation::SeededHelpersRemoved,
        LifecycleStage::FinalUninstall => ParentResidueExpectation::FinalUninstalled,
    }
}

fn desktop_parent_residue_expectation(phase: DesktopPhase) -> ParentResidueExpectation {
    match phase {
        DesktopPhase::FinalPrimary
        | DesktopPhase::BaselinePrimary
        | DesktopPhase::BaselineSecondInstance => ParentResidueExpectation::Clean,
        DesktopPhase::FinalMissingService
        | DesktopPhase::FinalStoppedService
        | DesktopPhase::FinalIncompatibleService => ParentResidueExpectation::SeededHelpersRemoved,
    }
}

fn crossed_desktop_request(phase: DesktopPhase) -> CrossedWorkerRequest {
    let (completed_stage, attempted_stage) = match phase {
        DesktopPhase::FinalPrimary => (LifecycleStage::FinalRepair, None),
        DesktopPhase::BaselinePrimary | DesktopPhase::BaselineSecondInstance => {
            (LifecycleStage::BaselineInstall, None)
        }
        DesktopPhase::FinalMissingService
        | DesktopPhase::FinalStoppedService
        | DesktopPhase::FinalIncompatibleService => (
            LifecycleStage::FinalCrashRecovery,
            Some(LifecycleStage::FinalFallbackStates),
        ),
    };
    CrossedWorkerRequest {
        completed_stage,
        attempted_stage,
    }
}

fn crossed_worker_result(result: &WorkerResult) -> Option<CrossedWorkerRequest> {
    Some(CrossedWorkerRequest {
        completed_stage: result.completed_stage?,
        attempted_stage: result
            .failure
            .as_ref()
            .and_then(|failure| failure.attempted_stage),
    })
}

trait ParentResidueTerminalCleanup {
    type Authority;

    fn cleanup_parent_residue(
        &mut self,
        authority: &Self::Authority,
        worker_tree_settled: bool,
    ) -> Result<(), String>;

    fn cleanup_success(&mut self, authority: &Self::Authority) -> Result<(), String> {
        self.cleanup_parent_residue(authority, true)
    }

    fn cleanup_worker_failure(
        &mut self,
        authority: &Self::Authority,
        process_trees_settled: bool,
    ) -> Option<String> {
        self.cleanup_parent_residue(authority, process_trees_settled)
            .err()
    }

    fn cleanup_post_release_failure(
        &mut self,
        authority: &Self::Authority,
        process_trees_settled: bool,
    ) -> Option<String> {
        self.cleanup_parent_residue(authority, process_trees_settled)
            .err()
    }

    fn cleanup_authenticated_abort(
        &mut self,
        authority: &Self::Authority,
        process_trees_settled: bool,
    ) -> Option<String> {
        self.cleanup_parent_residue(authority, process_trees_settled)
            .err()
    }

    fn cleanup_last_resort_abort(
        &mut self,
        authority: &Self::Authority,
        worker_tree_settled: bool,
        followup_error: &mut String,
    ) {
        if let Err(error) = self.cleanup_parent_residue(authority, worker_tree_settled) {
            *followup_error = append_parent_followup_error(followup_error, &error);
        }
    }
}

impl ParentResidueTerminalCleanup for ParentCurrentUserResidueState {
    type Authority = native::ParentCurrentUserAuthorityGuard;

    fn cleanup_parent_residue(
        &mut self,
        authority: &Self::Authority,
        worker_tree_settled: bool,
    ) -> Result<(), String> {
        self.cleanup_after_worker_settlement(authority, worker_tree_settled)
    }
}

fn parent_residue_sentinel(
    snapshot: &native::ParentCurrentUserResidueSnapshot,
) -> Result<&native::ParentHelperFileSnapshot, String> {
    match &snapshot.helper {
        Observation::Present(helper) => match &helper.sentinel {
            Observation::Present(sentinel) => Ok(sentinel),
            Observation::Absent | Observation::Unknown(_) => {
                Err("lifecycle_parent_user_helper_sentinel_missing".to_string())
            }
        },
        Observation::Absent | Observation::Unknown(_) => {
            Err("lifecycle_parent_user_helper_manifest_missing".to_string())
        }
    }
}

fn validate_parent_residue_snapshot(
    snapshot: &native::ParentCurrentUserResidueSnapshot,
    expectation: ParentResidueExpectation,
    authority: &native::ParentCurrentUserAuthority,
    sentinel_anchor: Option<&native::ParentHelperFileSnapshot>,
) -> Result<(), String> {
    if !native::valid_parent_run_key_owner(&snapshot.hkcu_run.owner_sid, &authority.user_sid)
        || snapshot.hkcu_run.final_key_path.is_empty()
        || validate_sha256(&snapshot.hkcu_run.dacl_sha256, "parent_user_run_dacl").is_err()
        || validate_sha256(
            &snapshot.hkcu_run.manifest_sha256,
            "parent_user_run_manifest",
        )
        .is_err()
        || snapshot.hkcu_run.last_write_time_100ns == 0
    {
        return Err("lifecycle_parent_user_run_key_authority_invalid".to_string());
    }
    let expects_run = !matches!(
        expectation,
        ParentResidueExpectation::Clean | ParentResidueExpectation::FinalUninstalled
    );
    match (&snapshot.hkcu_run.batcave_monitor, expects_run) {
        (Observation::Absent, false) => {}
        (Observation::Present(value), true)
            if snapshot.hkcu_run.value_count != 0
                && value.value_type == 1
                && value.value == native::exact_parent_run_value() => {}
        (Observation::Unknown(_), _) => {
            return Err("lifecycle_parent_user_run_value_unknown".to_string());
        }
        _ => return Err("lifecycle_parent_user_run_value_timeline_invalid".to_string()),
    }
    let expected_known = matches!(expectation, ParentResidueExpectation::SeededKnownHelpers);
    let expects_sentinel = !matches!(expectation, ParentResidueExpectation::Clean);
    match &snapshot.helper {
        Observation::Unknown(_) => {
            return Err("lifecycle_parent_user_helper_observation_unknown".to_string());
        }
        Observation::Absent if !expected_known && !expects_sentinel => return Ok(()),
        Observation::Absent => {
            return Err("lifecycle_parent_user_helper_manifest_missing".to_string());
        }
        Observation::Present(helper) => {
            if helper.root_owner_sid != authority.user_sid
                || helper.root.identity.volume_serial == 0
                || helper.root.identity.file_index == 0
                || validate_sha256(&helper.root_dacl_sha256, "parent_user_helper_root_dacl")
                    .is_err()
                || validate_sha256(&helper.manifest_sha256, "parent_user_helper_manifest").is_err()
                || helper.unexpected_entry_count != 0
            {
                return Err("lifecycle_parent_user_helper_manifest_invalid".to_string());
            }
            let expected_leaves = native::expected_helper_leaves();
            if expected_known {
                if helper.known_files.len() != expected_leaves.len() {
                    return Err("lifecycle_parent_user_helper_known_set_invalid".to_string());
                }
                for expected_leaf in expected_leaves {
                    let file = helper
                        .known_files
                        .iter()
                        .find(|file| file.relative_leaf == expected_leaf)
                        .ok_or_else(|| {
                            "lifecycle_parent_user_helper_known_set_invalid".to_string()
                        })?;
                    let expected = native::expected_parent_helper_fixture_snapshot(&expected_leaf)?;
                    if file.file.size != expected.0
                        || file.file.sha256 != expected.1
                        || file.owner_sid != authority.user_sid
                        || validate_sha256(&file.dacl_sha256, "parent_user_helper_fixture_dacl")
                            .is_err()
                    {
                        return Err("lifecycle_parent_user_helper_fixture_invalid".to_string());
                    }
                }
            } else if !helper.known_files.is_empty() {
                return Err("lifecycle_parent_user_helper_cleanup_incomplete".to_string());
            }
            match (&helper.sentinel, expects_sentinel) {
                (Observation::Absent, false) => {}
                (Observation::Present(sentinel), true) => {
                    let expected = native::expected_parent_helper_sentinel_snapshot();
                    if sentinel.relative_leaf != "elevated-helper/unknown-sentinel.bin"
                        || sentinel.file.size != expected.0
                        || sentinel.file.sha256 != expected.1
                        || sentinel.owner_sid != authority.user_sid
                        || validate_sha256(
                            &sentinel.dacl_sha256,
                            "parent_user_helper_sentinel_dacl",
                        )
                        .is_err()
                        || sentinel_anchor.is_some_and(|anchor| anchor != sentinel)
                    {
                        return Err("lifecycle_parent_user_helper_sentinel_changed".to_string());
                    }
                }
                (Observation::Unknown(_), _) => {
                    return Err("lifecycle_parent_user_helper_sentinel_unknown".to_string());
                }
                _ => {
                    return Err(
                        "lifecycle_parent_user_helper_sentinel_timeline_invalid".to_string()
                    );
                }
            }
        }
    }
    Ok(())
}

fn checkpoint_acceptance_after_parent_capture(
    checkpoint: WorkerCheckpoint,
    capture: Result<(), String>,
) -> Result<ParentMessage, String> {
    capture?;
    Ok(ParentMessage::CheckpointAccepted(checkpoint))
}

fn desktop_completion_after_parent_capture(
    result: DesktopPhaseResult,
    capture: Result<(), String>,
) -> Result<ParentMessage, String> {
    capture?;
    Ok(ParentMessage::DesktopPhaseComplete(Box::new(result)))
}

impl ProtocolGates {
    fn new() -> Self {
        Self {
            outbound: SequenceGate::new(),
            inbound: SequenceGate::new(),
        }
    }
}

#[derive(Debug, Eq, PartialEq)]
enum AbortResultAction {
    RepeatAbort,
    Complete(Box<WorkerResult>),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum AbortAcceptanceAction {
    Complete,
    RepeatResult,
}

struct AbortResultTracker {
    original: Option<WorkerResult>,
    awaiting_successor: bool,
    expected_parent_abort: Option<CrossedWorkerRequest>,
}

impl AbortResultTracker {
    fn new(original: Option<WorkerResult>) -> Self {
        let awaiting_successor = original.is_some();
        let expected_parent_abort = original.as_ref().and_then(crossed_worker_result);
        Self {
            original,
            awaiting_successor,
            expected_parent_abort,
        }
    }

    fn original_failure(&self) -> Option<&WorkerResult> {
        self.original
            .as_ref()
            .filter(|result| result.failure.is_some())
    }

    fn observe(
        &mut self,
        result: WorkerResult,
        reason: AbortReason,
        evidence_root_identity: crate::windows_lifecycle_proof_contract::EvidenceRootIdentity,
        last_checkpoint: Option<WorkerCheckpoint>,
    ) -> Result<AbortResultAction, String> {
        if result.abort.is_none() {
            validate_worker_result(&result, result.failure.is_none())?;
            if result.last_authenticated_checkpoint != last_checkpoint {
                return Err("lifecycle_parent_abort_context_invalid".to_string());
            }
            if self.awaiting_successor {
                return Err("lifecycle_parent_abort_result_replayed".to_string());
            }
            let crossed = crossed_worker_result(&result);
            match (self.expected_parent_abort, crossed) {
                (Some(expected), Some(actual)) if expected == actual => {}
                (None, None) => {}
                (None, Some(actual)) => self.expected_parent_abort = Some(actual),
                _ => {
                    return Err("lifecycle_parent_abort_crossed_stage_mismatch".to_string());
                }
            }
            self.original = Some(result);
            self.awaiting_successor = true;
            return Ok(AbortResultAction::RepeatAbort);
        }

        validate_worker_result(&result, false)?;
        let expected_abort = WorkerAbort {
            reason,
            last_authenticated_checkpoint: last_checkpoint,
            evidence_root_identity,
        };
        if result.abort != Some(expected_abort)
            || result.last_authenticated_checkpoint != last_checkpoint
        {
            return Err("lifecycle_parent_abort_context_invalid".to_string());
        }
        if let Some(original) = &self.original {
            if original.failure.is_some() {
                let mut expected = original.clone();
                expected.abort = Some(expected_abort);
                if result != expected {
                    return Err("lifecycle_parent_abort_successor_mismatch".to_string());
                }
            } else {
                self.validate_deterministic_parent_abort(&result, reason)?;
            }
        } else {
            self.validate_deterministic_parent_abort(&result, reason)?;
        }
        Ok(AbortResultAction::Complete(Box::new(result)))
    }

    fn note_crossed_stage(&mut self, stage: LifecycleStage) -> Result<(), String> {
        self.note_crossed_request(CrossedWorkerRequest {
            completed_stage: stage,
            attempted_stage: None,
        })
    }

    fn note_crossed_request(&mut self, request: CrossedWorkerRequest) -> Result<(), String> {
        if self
            .expected_parent_abort
            .is_some_and(|expected| expected != request)
        {
            return Err("lifecycle_parent_abort_crossed_stage_mismatch".to_string());
        }
        self.expected_parent_abort = Some(request);
        Ok(())
    }

    fn validate_deterministic_parent_abort(
        &self,
        result: &WorkerResult,
        reason: AbortReason,
    ) -> Result<(), String> {
        let expected = self
            .expected_parent_abort
            .ok_or_else(|| "lifecycle_parent_abort_crossed_stage_missing".to_string())?;
        let failure = result
            .failure
            .as_ref()
            .ok_or_else(|| "lifecycle_parent_abort_failure_missing".to_string())?;
        let restoration_stage = failure.attempted_stage.unwrap_or(expected.completed_stage);
        let restoration_valid = if !result.process_tree_settled {
            failure.restoration.as_ref() == &RestorationOutcome::BlockedUnsettled
        } else if expected.attempted_stage == Some(LifecycleStage::FinalFallbackStates) {
            valid_settled_fallback_restoration(failure.restoration.as_ref())
        } else if restoration_stage == LifecycleStage::InitialState {
            failure.restoration.as_ref() == &RestorationOutcome::NotRequired
        } else {
            matches!(
                failure.restoration.as_ref(),
                RestorationOutcome::Failed {
                    reason,
                    evidence,
                    evidence_error,
                } if reason == "lifecycle_parent_abort_restoration_not_reviewed"
                    && (evidence.is_some() != evidence_error.is_some())
                    && evidence.as_ref().is_none_or(|receipt| {
                        restoration_leaf_for_stage(restoration_stage)
                            == Some(receipt.name.as_str())
                    })
            )
        };
        if result.completed_stage != Some(expected.completed_stage)
            || failure.kind != WorkerFailureKind::ParentAbort
            || failure.attempted_stage != expected.attempted_stage
            || failure.reason != parent_abort_reason(reason)
            || !restoration_valid
        {
            return Err("lifecycle_parent_abort_successor_mismatch".to_string());
        }
        Ok(())
    }
}

pub(crate) fn run() -> i32 {
    let result = parse_entry(std::env::args().skip(1).collect()).and_then(dispatch);
    match result {
        Ok(code) => code,
        Err(reason) => {
            print_json(&ControllerOutcome {
                disposition: "failed",
                reason: Some(reason),
                worker_failure_kind: None,
                attempted_stage: None,
                failure_evidence: None,
                failure_evidence_verified: None,
                evidence_error: None,
                restoration: None,
                abort: None,
                restoration_evidence_verified: None,
                parent_followup_error: None,
                private_evidence: None,
                sanitized_export: None,
                success_evidence_verified: None,
                profile: None,
                controller_source_commit_sha: None,
                evidence_root: None,
                preflight: None,
            });
            1
        }
    }
}

fn dispatch(entry: Entry) -> Result<i32, String> {
    match entry {
        Entry::Preflight => {
            native::require_standard_token()?;
            let preflight = parent_preflight()?;
            print_json(&ControllerOutcome {
                disposition: "preflight_passed",
                reason: None,
                worker_failure_kind: None,
                attempted_stage: None,
                failure_evidence: None,
                failure_evidence_verified: None,
                evidence_error: None,
                restoration: None,
                abort: None,
                restoration_evidence_verified: None,
                parent_followup_error: None,
                private_evidence: None,
                sanitized_export: None,
                success_evidence_verified: None,
                profile: Some(preflight.plan.profile),
                controller_source_commit_sha: Some(preflight.source_commit_sha),
                evidence_root: None,
                preflight: Some(preflight.snapshot),
            });
            Ok(0)
        }
        Entry::Run => {
            native::require_standard_token()?;
            run_parent()
        }
        Entry::Worker { locator } => {
            native::require_elevated_token()?;
            run_worker(&locator)
        }
    }
}

fn parse_entry(args: Vec<String>) -> Result<Entry, String> {
    match args.as_slice() {
        [action] if action == "preflight" => Ok(Entry::Preflight),
        [action] if action == "run" => Ok(Entry::Run),
        [flag, locator] if flag == "--worker" => {
            validate_locator(locator)?;
            Ok(Entry::Worker {
                locator: locator.clone(),
            })
        }
        _ => Err("lifecycle_arguments_invalid".to_string()),
    }
}

fn parent_preflight() -> Result<ParentPreflight, String> {
    let plan = parse_plan()?;
    let repo_root = compiled_repo_root()?;
    let source_commit_sha = embedded_source_commit()?;
    native::require_clean_exact_head(&repo_root, &source_commit_sha)?;
    let controller = OwnedFile::open_current_executable()?;
    let baseline = open_candidate(&repo_root, &plan.baseline, "baseline_installer")?;
    let final_candidate = open_candidate(&repo_root, &plan.final_candidate, "final_installer")?;
    let incompatible_service_fixture = open_service_fixture(
        &repo_root,
        &plan.incompatible_service_fixture,
        "incompatible_service_fixture",
    )?;
    let rollback_failing_service_fixture = open_service_fixture(
        &repo_root,
        &plan.rollback_failing_service_fixture,
        "rollback_failing_service_fixture",
    )?;
    let controller_binding = native::current_controller_binding(&controller)?;
    let parent_current_user = native::capture_parent_current_user_authority()?;
    let snapshot = native::capture_parent_preflight(&plan, &[controller_binding])?;
    Ok(ParentPreflight {
        plan,
        repo_root,
        controller,
        baseline,
        final_candidate,
        incompatible_service_fixture,
        rollback_failing_service_fixture,
        parent_current_user,
        snapshot,
        source_commit_sha,
    })
}

fn run_parent() -> Result<i32, String> {
    let preflight = parent_preflight()?;
    lifecycle::require_controller_ready()?;
    evidence::require_private_evidence_projection_ready()?;
    // This full-ancestry, no-delete guard is acquired before elevation and retained
    // through terminal publication. A stale destination therefore blocks UAC entirely.
    let export_directory = native::pin_parent_export_directory(&preflight.repo_root)?;
    let locator = native::random_hex(crate::windows_lifecycle_proof_contract::LOCATOR_HEX_LENGTH)?;
    let nonce = native::random_hex(crate::windows_lifecycle_proof_contract::NONCE_HEX_LENGTH)?;
    let mut pipe = native::create_parent_pipe(&locator)?;
    let mut worker = native::launch_elevated_worker(&locator)?;
    pipe.connect(SESSION_TIMEOUT)?;
    native::authenticate_worker_peer(&pipe, &worker, &preflight.controller)?;
    worker.bind_to_parent_job()?;

    let mut gates = ProtocolGates::new();
    let request = ParentMessage::Begin(ClosedRequest {
        plan_sha256: plan_sha256(),
        controller_source_commit_sha: preflight.source_commit_sha.clone(),
        controller_sha256: preflight.controller.sha256_hex(),
        parent_process_id: std::process::id(),
        parent_started_at_100ns: native::current_process_started_at()?,
    });
    send_parent_message(&mut pipe, &nonce, &mut gates.outbound, request)?;

    let mut evidence_root = None;
    let mut last_checkpoint = None;
    let mut desktop_phase_index = 0;
    let mut desktop_results = Vec::with_capacity(DESKTOP_PHASES.len());
    let mut pending_failed_desktop: Option<PendingFailedDesktopResult> = None;
    let mut in_flight_worker_request = None;
    let mut unacknowledged_worker_result = None;
    let mut parent_desktop_trees_settled = true;
    let mut current_user_retention = ParentCurrentUserRetentionState::default();
    let mut current_user_residue = ParentCurrentUserResidueState::default();
    let mut worker_released = false;
    let session = (|| -> Result<i32, String> {
        loop {
            let envelope: Envelope<WorkerMessage> = pipe.read_json(SESSION_TIMEOUT)?;
            validate_envelope(&envelope, &nonce, &mut gates.inbound)?;
            if let Some(expected) = pending_failed_desktop.as_ref() {
                let WorkerMessage::ResultReady(result) = &envelope.message else {
                    return Err("lifecycle_failed_desktop_result_required".to_string());
                };
                validate_failed_desktop_worker_result(&expected.result, result)?;
            }
            match envelope.message {
                WorkerMessage::Accepted(accepted) => {
                    if evidence_root.is_some()
                        || last_checkpoint.is_some()
                        || accepted.worker_process_id != worker.process_id()
                        || accepted.worker_started_at_100ns != worker.started_at_100ns()
                    {
                        return Err("lifecycle_worker_acceptance_invalid".to_string());
                    }
                    evidence_root = Some(native::open_protected_evidence_root(
                        &accepted.evidence_root,
                        &nonce,
                        accepted.evidence_root_identity,
                    )?);
                }
                WorkerMessage::Checkpoint(checkpoint) => {
                    let root = evidence_root
                        .as_ref()
                        .ok_or_else(|| "lifecycle_checkpoint_before_acceptance".to_string())?;
                    if !valid_parent_checkpoint(
                        checkpoint,
                        root.identity(),
                        last_checkpoint,
                        desktop_phase_index,
                    ) {
                        return Err("lifecycle_checkpoint_invalid".to_string());
                    }
                    in_flight_worker_request = Some(CrossedWorkerRequest {
                        completed_stage: checkpoint.completed_stage,
                        attempted_stage: None,
                    });
                    current_user_retention.capture_checkpoint(
                        checkpoint.completed_stage,
                        &preflight.parent_current_user,
                    )?;
                    // This is intentionally the final fallible observation before the ACK.
                    // A failed current-user capture returns without acknowledging the worker.
                    let acceptance = checkpoint_acceptance_after_parent_capture(
                        checkpoint,
                        current_user_residue.capture_checkpoint(
                            checkpoint.completed_stage,
                            &preflight.parent_current_user,
                        ),
                    )?;
                    send_parent_message(&mut pipe, &nonce, &mut gates.outbound, acceptance)?;
                    last_checkpoint = Some(checkpoint);
                    in_flight_worker_request = None;
                }
                WorkerMessage::RunDesktopPhase(phase) => {
                    if evidence_root.is_none() {
                        return Err("lifecycle_desktop_phase_before_acceptance".to_string());
                    }
                    if DESKTOP_PHASES.get(desktop_phase_index) != Some(&phase) {
                        return Err("lifecycle_desktop_phase_order_invalid".to_string());
                    }
                    let crossed = crossed_desktop_request(phase);
                    if last_checkpoint.map(|checkpoint| checkpoint.completed_stage)
                        != Some(crossed.completed_stage)
                    {
                        return Err("lifecycle_desktop_phase_checkpoint_invalid".to_string());
                    }
                    in_flight_worker_request = Some(crossed);
                    current_user_residue
                        .capture_before_desktop(phase, &preflight.parent_current_user)?;
                    let result = lifecycle::run_parent_desktop_phase(
                        phase,
                        &preflight.repo_root,
                        &preflight.plan,
                    )
                    .unwrap_or(DesktopPhaseResult {
                        phase,
                        disposition: DesktopPhaseDisposition::Failed,
                        process_tree_settled: false,
                        observation: None,
                        failure_reason: Some("lifecycle_desktop_phase_runner_failed".to_string()),
                    });
                    parent_desktop_trees_settled &= result.process_tree_settled;
                    validate_requested_desktop_phase_result(phase, &result, &preflight.plan)?;
                    desktop_results.push(result.clone());
                    pending_failed_desktop = (result.disposition
                        == DesktopPhaseDisposition::Failed)
                        .then(|| PendingFailedDesktopResult {
                            result: result.clone(),
                            completion_write_confirmed: false,
                        });
                    // The parent executed this phase even when the completion write or the
                    // immediately preceding current-user capture later fails. Retaining that
                    // progress lets the abort lane validate a completion that raced the error.
                    desktop_phase_index += 1;
                    // Capture after the standard-parent launch and immediately before the
                    // authenticated completion message; no helper file handle survives this call.
                    let completion = desktop_completion_after_parent_capture(
                        result,
                        current_user_residue
                            .capture_desktop_complete(phase, &preflight.parent_current_user),
                    )?;
                    send_parent_message(&mut pipe, &nonce, &mut gates.outbound, completion)?;
                    if let Some(pending) = pending_failed_desktop.as_mut() {
                        pending.completion_write_confirmed = true;
                    }
                    in_flight_worker_request = None;
                }
                WorkerMessage::ResultReady(result) if result.failure.is_none() => {
                    if evidence_root.is_none() {
                        return Err("lifecycle_worker_completion_order_invalid".to_string());
                    }
                    validate_parent_original_worker_result(
                        &result,
                        last_checkpoint,
                        desktop_phase_index,
                    )?;
                    in_flight_worker_request = crossed_worker_result(&result);
                    unacknowledged_worker_result = Some((*result).clone());
                    let evidence_root_guard = evidence_root
                        .as_ref()
                        .ok_or_else(|| "lifecycle_evidence_root_missing".to_string())?;
                    let mut private_evidence_guards =
                        Vec::with_capacity(result.private_evidence.len());
                    for receipt in &result.private_evidence {
                        private_evidence_guards.push(native::verify_evidence_receipt(
                            evidence_root_guard,
                            receipt,
                        )?);
                    }
                    validate_success_desktop_results(
                        &desktop_results,
                        parent_desktop_trees_settled,
                        &preflight.plan,
                    )?;
                    let prepared_export = {
                        let (before_uninstall, after_uninstall) =
                            current_user_retention.complete(&preflight.parent_current_user)?;
                        let residue_timeline =
                            current_user_residue.complete(&preflight.parent_current_user)?;
                        evidence::derive_sanitized_export(
                            &private_evidence_guards,
                            &preflight.plan,
                            &preflight.source_commit_sha,
                            &preflight.controller.sha256_hex(),
                            &desktop_results,
                            evidence::ParentCurrentUserProjection {
                                authority: preflight.parent_current_user.authority(),
                                before_uninstall,
                                after_uninstall,
                                residue_timeline,
                            },
                        )?
                    };
                    revalidate_success_sources(
                        &preflight,
                        evidence_root_guard,
                        &private_evidence_guards,
                        &current_user_retention,
                        &mut current_user_residue,
                    )?;
                    export_directory.require_leaf_absent()?;
                    // EvidenceAccepted releases the worker only. It is not publication success,
                    // and every later error stays on the local post-release finalizer.
                    worker_released = true;
                    send_parent_message(
                        &mut pipe,
                        &nonce,
                        &mut gates.outbound,
                        ParentMessage::EvidenceAccepted,
                    )?;
                    let exit_code = worker.wait(Duration::from_secs(30))?;
                    if exit_code != 0 {
                        return Err("lifecycle_worker_exit_mismatch".to_string());
                    }
                    revalidate_success_sources(
                        &preflight,
                        evidence_root_guard,
                        &private_evidence_guards,
                        &current_user_retention,
                        &mut current_user_residue,
                    )?;
                    export_directory.require_leaf_absent()?;
                    let sanitized_export =
                        native::write_parent_export_new(&export_directory, &prepared_export)?;
                    if private_evidence_guards
                        .iter()
                        .any(|private| private.identity() == sanitized_export.identity())
                    {
                        return Err("lifecycle_parent_export_private_identity_reused".to_string());
                    }
                    revalidate_success_sources(
                        &preflight,
                        evidence_root_guard,
                        &private_evidence_guards,
                        &current_user_retention,
                        &mut current_user_residue,
                    )?;
                    sanitized_export.revalidate(&export_directory)?;
                    current_user_residue.cleanup_success(&preflight.parent_current_user)?;
                    print_json(&ControllerOutcome {
                        disposition: "passed",
                        reason: None,
                        worker_failure_kind: None,
                        attempted_stage: None,
                        failure_evidence: None,
                        failure_evidence_verified: None,
                        evidence_error: None,
                        restoration: None,
                        abort: None,
                        restoration_evidence_verified: None,
                        parent_followup_error: None,
                        private_evidence: Some(result.private_evidence.clone()),
                        sanitized_export: Some(sanitized_export.receipt().clone()),
                        success_evidence_verified: Some(true),
                        profile: Some(preflight.plan.profile.clone()),
                        controller_source_commit_sha: Some(preflight.source_commit_sha.clone()),
                        evidence_root: None,
                        preflight: Some(preflight.snapshot.clone()),
                    });
                    return Ok(0);
                }
                WorkerMessage::ResultReady(result) => {
                    if evidence_root.is_none() {
                        return Err("lifecycle_worker_failure_before_acceptance".to_string());
                    }
                    validate_parent_original_worker_result(
                        &result,
                        last_checkpoint,
                        desktop_phase_index,
                    )?;
                    let failure = result
                        .failure
                        .as_ref()
                        .ok_or_else(|| "lifecycle_worker_failure_missing".to_string())?;
                    in_flight_worker_request = crossed_worker_result(&result);
                    unacknowledged_worker_result = Some((*result).clone());
                    let terminate_without_ack = !result.process_tree_settled
                        && matches!(
                            failure.restoration.as_ref(),
                            RestorationOutcome::BlockedUnsettled
                        );
                    let expected_exit_code = u32::from(result.abort.is_some());
                    let mut evidence_guard = None;
                    let mut restoration_evidence_guard = None;
                    let mut cleanup_error = None;
                    let followup = (|| -> Result<(), String> {
                        let evidence_root_guard = evidence_root
                            .as_ref()
                            .ok_or_else(|| "lifecycle_evidence_root_missing".to_string())?;
                        if terminate_without_ack {
                            worker.terminate_and_settle()?;
                        }
                        if let Some(receipt) = &failure.evidence {
                            evidence_guard = Some(native::verify_evidence_receipt(
                                evidence_root_guard,
                                receipt,
                            )?);
                        }
                        if let Some(receipt) = restoration_evidence(failure.restoration.as_ref()) {
                            restoration_evidence_guard = Some(native::verify_evidence_receipt(
                                evidence_root_guard,
                                receipt,
                            )?);
                        }
                        revalidate_preflight_artifacts(&preflight)?;
                        if !terminate_without_ack {
                            send_parent_message(
                                &mut pipe,
                                &nonce,
                                &mut gates.outbound,
                                ParentMessage::EvidenceAccepted,
                            )?;
                            let exit_code = worker.wait(Duration::from_secs(30))?;
                            if exit_code != expected_exit_code {
                                return Err("lifecycle_worker_failure_exit_mismatch".to_string());
                            }
                            revalidate_preflight_artifacts(&preflight)?;
                        }
                        cleanup_error = current_user_residue.cleanup_worker_failure(
                            &preflight.parent_current_user,
                            parent_desktop_trees_settled,
                        );
                        Ok(())
                    })();
                    let failure_evidence_verified =
                        failure.evidence.as_ref().map(|_| evidence_guard.is_some());
                    let restoration_evidence_verified =
                        restoration_evidence(failure.restoration.as_ref())
                            .map(|_| restoration_evidence_guard.is_some());
                    let parent_followup_error = match followup {
                        Ok(()) => cleanup_error,
                        Err(reason) if !terminate_without_ack => {
                            return abort_parent_session(
                                &mut pipe,
                                &nonce,
                                &mut gates,
                                &mut worker,
                                &preflight,
                                &mut current_user_residue,
                                evidence_root.as_ref(),
                                last_checkpoint,
                                desktop_phase_index,
                                in_flight_worker_request,
                                pending_failed_desktop.as_ref(),
                                parent_desktop_trees_settled,
                                Some((*result).clone()),
                                reason,
                            );
                        }
                        Err(reason) => Some(reason),
                    };
                    let evidence_root_value = evidence_root
                        .as_ref()
                        .map(|root| root.root().to_string_lossy().into_owned());
                    print_json(&ControllerOutcome {
                        disposition: if parent_followup_error.is_some() {
                            "worker_failure_followup_failed"
                        } else {
                            "worker_failed"
                        },
                        reason: Some(failure.reason.clone()),
                        worker_failure_kind: Some(failure.kind),
                        attempted_stage: failure.attempted_stage,
                        failure_evidence: failure.evidence.as_deref().cloned(),
                        failure_evidence_verified,
                        evidence_error: failure.evidence_error.clone(),
                        restoration: Some(failure.restoration.as_ref().clone()),
                        abort: result.abort,
                        restoration_evidence_verified,
                        parent_followup_error,
                        private_evidence: None,
                        sanitized_export: None,
                        success_evidence_verified: None,
                        profile: Some(preflight.plan.profile.clone()),
                        controller_source_commit_sha: Some(preflight.source_commit_sha.clone()),
                        evidence_root: evidence_root_value,
                        preflight: Some(preflight.snapshot.clone()),
                    });
                    return Ok(1);
                }
            }
        }
    })();
    match session {
        Ok(code) => Ok(code),
        Err(reason) if worker_released => finalize_post_release_failure(
            &mut worker,
            &preflight,
            &mut current_user_residue,
            parent_desktop_trees_settled,
            reason,
        ),
        Err(reason) => abort_parent_session(
            &mut pipe,
            &nonce,
            &mut gates,
            &mut worker,
            &preflight,
            &mut current_user_residue,
            evidence_root.as_ref(),
            last_checkpoint,
            desktop_phase_index,
            in_flight_worker_request,
            pending_failed_desktop.as_ref(),
            parent_desktop_trees_settled,
            unacknowledged_worker_result,
            reason,
        ),
    }
}

fn validate_requested_desktop_phase_result(
    requested_phase: DesktopPhase,
    result: &DesktopPhaseResult,
    plan: &ProofPlan,
) -> Result<(), String> {
    if result.phase != requested_phase {
        return Err("lifecycle_desktop_phase_result_mismatch".to_string());
    }
    validate_desktop_phase_result(result, plan)
}

fn validate_failed_desktop_worker_result(
    desktop: &DesktopPhaseResult,
    result: &WorkerResult,
) -> Result<(), String> {
    if desktop.disposition != DesktopPhaseDisposition::Failed {
        return Err("lifecycle_failed_desktop_expectation_invalid".to_string());
    }
    validate_worker_result(result, false)?;
    let failure = result
        .failure
        .as_ref()
        .ok_or_else(|| "lifecycle_failed_desktop_worker_failure_missing".to_string())?;
    let crossed = crossed_desktop_request(desktop.phase);
    let expected_reason = format!(
        "{}:{}",
        lifecycle::desktop_evidence_name(desktop.phase),
        desktop
            .failure_reason
            .as_deref()
            .ok_or_else(|| "lifecycle_failed_desktop_reason_missing".to_string())?
    );
    if result.completed_stage != Some(crossed.completed_stage)
        || result.abort.is_some()
        || failure.kind != WorkerFailureKind::Controller
        || failure.attempted_stage != crossed.attempted_stage
        || failure.reason != expected_reason
        || failure.evidence.is_some()
        || failure.evidence_error.is_some()
    {
        return Err("lifecycle_failed_desktop_worker_result_mismatch".to_string());
    }
    let settlement_matches = if crossed.attempted_stage.is_none() {
        result.process_tree_settled == desktop.process_tree_settled
            && if desktop.process_tree_settled {
                matches!(
                    failure.restoration.as_ref(),
                    RestorationOutcome::BlockedUntrusted { reason }
                        if reason == "lifecycle_restoration_not_reviewed"
                )
            } else {
                failure.restoration.as_ref() == &RestorationOutcome::BlockedUnsettled
            }
    } else if !desktop.process_tree_settled {
        !result.process_tree_settled
            && failure.restoration.as_ref() == &RestorationOutcome::BlockedUnsettled
    } else if result.process_tree_settled {
        valid_settled_fallback_restoration(failure.restoration.as_ref())
    } else {
        failure.restoration.as_ref() == &RestorationOutcome::BlockedUnsettled
    };
    if settlement_matches {
        Ok(())
    } else {
        Err("lifecycle_failed_desktop_settlement_mismatch".to_string())
    }
}

fn validate_queued_failed_desktop_result(
    pending: Option<&PendingFailedDesktopResult>,
    tracker_has_original: bool,
    result: &WorkerResult,
) -> Result<(), String> {
    let Some(expected) = pending else {
        return Ok(());
    };
    if result.abort.is_none() {
        validate_failed_desktop_worker_result(&expected.result, result)
    } else if tracker_has_original || !expected.completion_write_confirmed {
        Ok(())
    } else {
        Err("lifecycle_failed_desktop_original_result_required".to_string())
    }
}

fn validate_parent_original_worker_result(
    result: &WorkerResult,
    last_checkpoint: Option<WorkerCheckpoint>,
    desktop_phase_index: usize,
) -> Result<(), String> {
    if result.abort.is_some() {
        return Err("lifecycle_worker_unsolicited_abort".to_string());
    }
    if result.last_authenticated_checkpoint != last_checkpoint {
        return Err("lifecycle_worker_checkpoint_mismatch".to_string());
    }
    let Some(failure) = result.failure.as_ref() else {
        if desktop_phase_index != DESKTOP_PHASES.len() {
            return Err("lifecycle_worker_completion_order_invalid".to_string());
        }
        return validate_worker_result(result, true);
    };
    validate_worker_result(result, false)?;
    validate_parent_failure_completion(result, failure, desktop_phase_index)?;
    validate_parent_failure_progress(result)
}

fn valid_checkpoint_transition(
    previous: Option<WorkerCheckpoint>,
    checkpoint: WorkerCheckpoint,
) -> bool {
    next_lifecycle_stage(previous.map(|checkpoint| checkpoint.completed_stage))
        == Some(checkpoint.completed_stage)
}

fn valid_parent_checkpoint(
    checkpoint: WorkerCheckpoint,
    evidence_root_identity: crate::windows_lifecycle_proof_contract::EvidenceRootIdentity,
    previous: Option<WorkerCheckpoint>,
    desktop_phase_index: usize,
) -> bool {
    checkpoint.evidence_root_identity == evidence_root_identity
        && valid_checkpoint_transition(previous, checkpoint)
        && desktop_phase_index
            == expected_desktop_phase_index_at_checkpoint(checkpoint.completed_stage)
}

fn expected_desktop_phase_index_at_checkpoint(stage: LifecycleStage) -> usize {
    match stage {
        LifecycleStage::InitialState | LifecycleStage::FinalRepair => 0,
        LifecycleStage::InitialUninstall | LifecycleStage::BaselineInstall => 1,
        LifecycleStage::BaselineRestart
        | LifecycleStage::BaselineCrashRecovery
        | LifecycleStage::BaselineRollbackRecovery
        | LifecycleStage::LegacyResidueSeeded
        | LifecycleStage::FinalUpgrade
        | LifecycleStage::FinalRestart
        | LifecycleStage::FinalCrashRecovery => 3,
        LifecycleStage::FinalFallbackStates | LifecycleStage::FinalUninstall => {
            DESKTOP_PHASES.len()
        }
    }
}

fn minimum_desktop_phase_index_before_stage(stage: LifecycleStage) -> usize {
    match stage {
        LifecycleStage::InitialState | LifecycleStage::FinalRepair => 0,
        LifecycleStage::InitialUninstall | LifecycleStage::BaselineInstall => 1,
        LifecycleStage::BaselineRestart
        | LifecycleStage::BaselineCrashRecovery
        | LifecycleStage::BaselineRollbackRecovery
        | LifecycleStage::LegacyResidueSeeded
        | LifecycleStage::FinalUpgrade
        | LifecycleStage::FinalRestart
        | LifecycleStage::FinalCrashRecovery
        | LifecycleStage::FinalFallbackStates => 3,
        LifecycleStage::FinalUninstall => DESKTOP_PHASES.len(),
    }
}

fn next_lifecycle_stage(previous: Option<LifecycleStage>) -> Option<LifecycleStage> {
    match previous {
        None => Some(LifecycleStage::InitialState),
        Some(LifecycleStage::InitialState) => Some(LifecycleStage::FinalRepair),
        Some(LifecycleStage::FinalRepair) => Some(LifecycleStage::InitialUninstall),
        Some(LifecycleStage::InitialUninstall) => Some(LifecycleStage::BaselineInstall),
        Some(LifecycleStage::BaselineInstall) => Some(LifecycleStage::BaselineRestart),
        Some(LifecycleStage::BaselineRestart) => Some(LifecycleStage::BaselineCrashRecovery),
        Some(LifecycleStage::BaselineCrashRecovery) => {
            Some(LifecycleStage::BaselineRollbackRecovery)
        }
        Some(LifecycleStage::BaselineRollbackRecovery) => Some(LifecycleStage::LegacyResidueSeeded),
        Some(LifecycleStage::LegacyResidueSeeded) => Some(LifecycleStage::FinalUpgrade),
        Some(LifecycleStage::FinalUpgrade) => Some(LifecycleStage::FinalRestart),
        Some(LifecycleStage::FinalRestart) => Some(LifecycleStage::FinalCrashRecovery),
        Some(LifecycleStage::FinalCrashRecovery) => Some(LifecycleStage::FinalFallbackStates),
        Some(LifecycleStage::FinalFallbackStates) => Some(LifecycleStage::FinalUninstall),
        Some(LifecycleStage::FinalUninstall) => None,
    }
}

#[allow(clippy::too_many_arguments)]
fn abort_parent_session(
    pipe: &mut PipeConnection,
    nonce: &str,
    gates: &mut ProtocolGates,
    worker: &mut native::ElevatedProcess,
    preflight: &ParentPreflight,
    current_user_residue: &mut ParentCurrentUserResidueState,
    evidence_root: Option<&native::ProtectedEvidenceRoot>,
    last_checkpoint: Option<WorkerCheckpoint>,
    desktop_phase_index: usize,
    crossed_request: Option<CrossedWorkerRequest>,
    pending_failed_desktop: Option<&PendingFailedDesktopResult>,
    parent_desktop_trees_settled: bool,
    original_result: Option<WorkerResult>,
    parent_error: String,
) -> Result<i32, String> {
    let reason = abort_reason_for_parent_error(&parent_error);
    let mut tracker = AbortResultTracker::new(original_result);
    if let Some(crossed) = crossed_request {
        tracker.note_crossed_request(crossed)?;
    }
    let mut failure_evidence_guard = None;
    let mut restoration_evidence_guard = None;
    let mut cleanup_error = None;
    let abort_sent = send_parent_message(
        pipe,
        nonce,
        &mut gates.outbound,
        ParentMessage::Abort(reason),
    )
    .is_ok();
    let mut abort_followup_error = None;
    if abort_sent {
        let followup = (|| -> Result<WorkerResult, String> {
            let mut result = None;
            let deadline = Instant::now() + ABORT_TIMEOUT;
            for _ in 0..8 {
                let remaining = deadline.saturating_duration_since(Instant::now());
                if remaining.is_zero() {
                    return Err("lifecycle_parent_abort_timeout".to_string());
                }
                let envelope: Envelope<WorkerMessage> = pipe.read_json(remaining)?;
                validate_envelope(&envelope, nonce, &mut gates.inbound)?;
                match envelope.message {
                    WorkerMessage::ResultReady(value) => {
                        validate_queued_failed_desktop_result(
                            pending_failed_desktop,
                            tracker.original.is_some(),
                            &value,
                        )?;
                        if value.abort.is_none() {
                            validate_parent_original_worker_result(
                                &value,
                                last_checkpoint,
                                desktop_phase_index,
                            )?;
                        }
                        let root = evidence_root.ok_or_else(|| {
                            "lifecycle_parent_abort_evidence_root_missing".to_string()
                        })?;
                        match tracker.observe(*value, reason, root.identity(), last_checkpoint)? {
                            AbortResultAction::RepeatAbort => {
                                send_parent_message(
                                    pipe,
                                    nonce,
                                    &mut gates.outbound,
                                    ParentMessage::Abort(reason),
                                )?;
                            }
                            AbortResultAction::Complete(value) => {
                                result = Some(*value);
                                break;
                            }
                        }
                    }
                    message
                    @ (WorkerMessage::Checkpoint(_) | WorkerMessage::RunDesktopPhase(_)) => {
                        if let WorkerMessage::Checkpoint(checkpoint) = &message {
                            if evidence_root.is_none_or(|root| {
                                !valid_parent_checkpoint(
                                    *checkpoint,
                                    root.identity(),
                                    last_checkpoint,
                                    desktop_phase_index,
                                )
                            }) {
                                return Err("lifecycle_parent_abort_result_required".to_string());
                            }
                            tracker.note_crossed_stage(checkpoint.completed_stage)?;
                        } else {
                            let precursor = last_checkpoint
                                .ok_or_else(|| {
                                    "lifecycle_parent_abort_desktop_precursor_missing".to_string()
                                })?
                                .completed_stage;
                            let WorkerMessage::RunDesktopPhase(phase) = &message else {
                                unreachable!()
                            };
                            if DESKTOP_PHASES.get(desktop_phase_index) != Some(phase) {
                                return Err("lifecycle_parent_abort_desktop_phase_order_invalid"
                                    .to_string());
                            }
                            let crossed = crossed_desktop_request(*phase);
                            if crossed.completed_stage != precursor {
                                return Err(
                                    "lifecycle_parent_abort_desktop_precursor_mismatch".to_string()
                                );
                            }
                            tracker.note_crossed_request(crossed)?;
                        }
                        if worker_message_requires_abort_repeat(&message) {
                            send_parent_message(
                                pipe,
                                nonce,
                                &mut gates.outbound,
                                ParentMessage::Abort(reason),
                            )?;
                        }
                    }
                    WorkerMessage::Accepted(_) => {
                        return Err("lifecycle_parent_abort_result_required".to_string());
                    }
                }
            }
            let result =
                result.ok_or_else(|| "lifecycle_parent_abort_result_required".to_string())?;
            let root = evidence_root
                .ok_or_else(|| "lifecycle_parent_abort_evidence_root_missing".to_string())?;
            let failure = result
                .failure
                .as_ref()
                .ok_or_else(|| "lifecycle_parent_abort_failure_missing".to_string())?;
            let unsettled_abort = !result.process_tree_settled
                && failure.restoration.as_ref() == &RestorationOutcome::BlockedUnsettled;
            if unsettled_abort {
                revalidate_preflight_artifacts(preflight)?;
                worker.terminate_and_settle()?;
            }
            if let Some(receipt) = &failure.evidence {
                failure_evidence_guard = Some(native::verify_evidence_receipt(root, receipt)?);
            }
            if let Some(receipt) = restoration_evidence(failure.restoration.as_ref()) {
                restoration_evidence_guard = Some(native::verify_evidence_receipt(root, receipt)?);
            }
            revalidate_preflight_artifacts(preflight)?;
            if !unsettled_abort {
                send_parent_message(
                    pipe,
                    nonce,
                    &mut gates.outbound,
                    ParentMessage::EvidenceAccepted,
                )?;
                if worker.wait(Duration::from_secs(30))? != 1 {
                    return Err("lifecycle_parent_abort_exit_mismatch".to_string());
                }
                revalidate_preflight_artifacts(preflight)?;
            }
            cleanup_error = current_user_residue.cleanup_authenticated_abort(
                &preflight.parent_current_user,
                parent_desktop_trees_settled,
            );
            Ok(result)
        })();
        match followup {
            Ok(result) => {
                let preserved_failure = tracker.original_failure().is_some();
                let authoritative = tracker.original_failure().unwrap_or(&result);
                let failure = authoritative
                    .failure
                    .as_ref()
                    .ok_or_else(|| "lifecycle_parent_abort_failure_missing".to_string())?;
                let parent_followup_error = match (preserved_failure, cleanup_error.as_deref()) {
                    (true, Some(cleanup)) => {
                        Some(append_parent_followup_error(&parent_error, cleanup))
                    }
                    (true, None) => Some(parent_error.clone()),
                    (false, Some(cleanup)) => Some(cleanup.to_string()),
                    (false, None) => None,
                };
                print_json(&ControllerOutcome {
                    disposition: if preserved_failure {
                        "worker_failure_followup_failed"
                    } else if cleanup_error.is_some() {
                        "parent_abort_cleanup_blocked"
                    } else {
                        "parent_aborted"
                    },
                    reason: Some(if preserved_failure {
                        failure.reason.clone()
                    } else {
                        parent_error.clone()
                    }),
                    worker_failure_kind: Some(failure.kind),
                    attempted_stage: failure.attempted_stage.or(authoritative.completed_stage),
                    failure_evidence: failure.evidence.as_deref().cloned(),
                    failure_evidence_verified: failure
                        .evidence
                        .as_ref()
                        .map(|_| failure_evidence_guard.is_some()),
                    evidence_error: failure.evidence_error.clone(),
                    restoration: Some(failure.restoration.as_ref().clone()),
                    abort: result.abort,
                    restoration_evidence_verified: restoration_evidence(
                        failure.restoration.as_ref(),
                    )
                    .map(|_| restoration_evidence_guard.is_some()),
                    parent_followup_error,
                    private_evidence: None,
                    sanitized_export: None,
                    success_evidence_verified: None,
                    profile: Some(preflight.plan.profile.clone()),
                    controller_source_commit_sha: Some(preflight.source_commit_sha.clone()),
                    evidence_root: evidence_root
                        .map(|root| root.root().to_string_lossy().into_owned()),
                    preflight: Some(preflight.snapshot.clone()),
                });
                return Ok(1);
            }
            Err(error) => abort_followup_error = Some(error),
        }
    }

    let mut followup_error = if let Some(error) = abort_followup_error {
        error
    } else {
        "lifecycle_parent_abort_send_failed".to_string()
    };
    let (restoration, worker_settled) = match worker
        .wait_without_termination(Duration::from_secs(30))
    {
        Ok(Some(_)) => match worker.terminate_and_settle() {
            Ok(()) => (
                unverified_abort_restoration(UnverifiedAbortSettlement::WorkerExited),
                true,
            ),
            Err(error) => {
                followup_error = append_parent_followup_error(&followup_error, &error);
                (
                    unverified_abort_restoration(UnverifiedAbortSettlement::ForcedUnsettled),
                    false,
                )
            }
        },
        Ok(None) => match worker.terminate_and_settle() {
            Ok(()) => (
                unverified_abort_restoration(UnverifiedAbortSettlement::ForcedSettled),
                true,
            ),
            Err(error) => {
                followup_error = append_parent_followup_error(&followup_error, &error);
                (
                    unverified_abort_restoration(UnverifiedAbortSettlement::ForcedUnsettled),
                    false,
                )
            }
        },
        Err(error) => {
            followup_error = append_parent_followup_error(&followup_error, &error);
            match worker.terminate_and_settle() {
                Ok(()) => (
                    unverified_abort_restoration(UnverifiedAbortSettlement::ForcedSettled),
                    true,
                ),
                Err(error) => {
                    followup_error = append_parent_followup_error(&followup_error, &error);
                    (
                        unverified_abort_restoration(UnverifiedAbortSettlement::ForcedUnsettled),
                        false,
                    )
                }
            }
        }
    };
    current_user_residue.cleanup_last_resort_abort(
        &preflight.parent_current_user,
        worker_settled && parent_desktop_trees_settled,
        &mut followup_error,
    );
    if let Err(error) = revalidate_preflight_artifacts(preflight) {
        followup_error = append_parent_followup_error(&followup_error, &error);
    }
    if let Some(authoritative) = tracker.original_failure() {
        let failure = authoritative
            .failure
            .as_ref()
            .ok_or_else(|| "lifecycle_parent_abort_failure_missing".to_string())?;
        print_json(&ControllerOutcome {
            disposition: "worker_failure_followup_failed",
            reason: Some(failure.reason.clone()),
            worker_failure_kind: Some(failure.kind),
            attempted_stage: failure.attempted_stage.or(authoritative.completed_stage),
            failure_evidence: failure.evidence.as_deref().cloned(),
            failure_evidence_verified: failure
                .evidence
                .as_ref()
                .map(|_| failure_evidence_guard.is_some()),
            evidence_error: failure.evidence_error.clone(),
            restoration: Some(failure.restoration.as_ref().clone()),
            abort: evidence_root.map(|root| WorkerAbort {
                reason,
                last_authenticated_checkpoint: last_checkpoint,
                evidence_root_identity: root.identity(),
            }),
            restoration_evidence_verified: restoration_evidence(failure.restoration.as_ref())
                .map(|_| restoration_evidence_guard.is_some()),
            parent_followup_error: Some(append_parent_followup_error(
                &parent_error,
                &followup_error,
            )),
            private_evidence: None,
            sanitized_export: None,
            success_evidence_verified: None,
            profile: Some(preflight.plan.profile.clone()),
            controller_source_commit_sha: Some(preflight.source_commit_sha.clone()),
            evidence_root: evidence_root.map(|root| root.root().to_string_lossy().into_owned()),
            preflight: Some(preflight.snapshot.clone()),
        });
        return Ok(1);
    }
    print_json(&ControllerOutcome {
        disposition: "parent_abort_untrusted",
        reason: Some(parent_error),
        worker_failure_kind: Some(WorkerFailureKind::ParentAbort),
        attempted_stage: last_checkpoint.map(|checkpoint| checkpoint.completed_stage),
        failure_evidence: None,
        failure_evidence_verified: None,
        evidence_error: None,
        restoration: Some(restoration),
        abort: evidence_root.map(|root| WorkerAbort {
            reason,
            last_authenticated_checkpoint: last_checkpoint,
            evidence_root_identity: root.identity(),
        }),
        restoration_evidence_verified: None,
        parent_followup_error: Some(followup_error),
        private_evidence: None,
        sanitized_export: None,
        success_evidence_verified: None,
        profile: Some(preflight.plan.profile.clone()),
        controller_source_commit_sha: Some(preflight.source_commit_sha.clone()),
        evidence_root: evidence_root.map(|root| root.root().to_string_lossy().into_owned()),
        preflight: Some(preflight.snapshot.clone()),
    });
    Ok(1)
}

fn unverified_abort_restoration(settlement: UnverifiedAbortSettlement) -> RestorationOutcome {
    match settlement {
        UnverifiedAbortSettlement::WorkerExited => RestorationOutcome::BlockedUntrusted {
            reason: "lifecycle_parent_abort_result_unavailable".to_string(),
        },
        UnverifiedAbortSettlement::ForcedSettled => RestorationOutcome::BlockedUntrusted {
            reason: "lifecycle_parent_abort_forced_worker_termination".to_string(),
        },
        UnverifiedAbortSettlement::ForcedUnsettled => RestorationOutcome::BlockedUnsettled,
    }
}

fn append_parent_followup_error(primary: &str, secondary: &str) -> String {
    format!("{primary}|abort_followup:{secondary}")
}

fn abort_reason_for_parent_error(reason: &str) -> AbortReason {
    if reason.contains("timeout") {
        AbortReason::Timeout
    } else if reason.contains("pipe_closed") || reason.contains("disconnect") {
        AbortReason::Disconnected
    } else if reason.contains("desktop") {
        AbortReason::DesktopFailure
    } else if reason.contains("receipt") {
        AbortReason::ReceiptValidation
    } else if reason.contains("artifact") || reason.contains("revalidat") {
        AbortReason::ArtifactValidation
    } else if reason.contains("evidence") || reason.contains("sanitized") {
        AbortReason::EvidenceValidation
    } else {
        AbortReason::ProtocolViolation
    }
}

fn worker_message_requires_abort_repeat(message: &WorkerMessage) -> bool {
    matches!(
        message,
        WorkerMessage::Checkpoint(_) | WorkerMessage::RunDesktopPhase(_)
    )
}

fn revalidate_preflight_artifacts(preflight: &ParentPreflight) -> Result<(), String> {
    preflight.controller.revalidate()?;
    preflight.baseline.revalidate()?;
    preflight.final_candidate.revalidate()?;
    preflight.incompatible_service_fixture.revalidate()?;
    preflight.rollback_failing_service_fixture.revalidate()?;
    preflight.parent_current_user.revalidate()
}

fn validate_success_desktop_results(
    results: &[DesktopPhaseResult],
    process_trees_settled: bool,
    plan: &ProofPlan,
) -> Result<(), String> {
    if !process_trees_settled || results.len() != DESKTOP_PHASES.len() {
        return Err("lifecycle_parent_desktop_results_incomplete".to_string());
    }
    for (result, expected_phase) in results.iter().zip(DESKTOP_PHASES) {
        if result.phase != expected_phase
            || result.disposition != DesktopPhaseDisposition::Passed
            || !result.process_tree_settled
        {
            return Err("lifecycle_parent_desktop_result_invalid".to_string());
        }
        validate_desktop_phase_result(result, plan)?;
    }
    Ok(())
}

fn revalidate_success_sources(
    preflight: &ParentPreflight,
    evidence_root: &native::ProtectedEvidenceRoot,
    private_evidence: &[native::VerifiedEvidenceFile],
    current_user_retention: &ParentCurrentUserRetentionState,
    current_user_residue: &mut ParentCurrentUserResidueState,
) -> Result<(), String> {
    revalidate_preflight_artifacts(preflight)?;
    evidence_root.revalidate()?;
    if private_evidence.len() != SUCCESS_PRIVATE_EVIDENCE_LEAVES.len() {
        return Err("lifecycle_success_private_evidence_manifest_invalid".to_string());
    }
    let mut identities = std::collections::BTreeSet::new();
    for (file, expected_name) in private_evidence.iter().zip(SUCCESS_PRIVATE_EVIDENCE_LEAVES) {
        if file.receipt().name != expected_name
            || !identities.insert((file.identity().volume_serial, file.identity().file_index))
        {
            return Err("lifecycle_success_private_evidence_manifest_invalid".to_string());
        }
        file.revalidate()?;
    }
    current_user_retention.revalidate_terminal(&preflight.parent_current_user)?;
    current_user_residue.complete(&preflight.parent_current_user)?;
    current_user_residue.revalidate_final_uninstall(&preflight.parent_current_user)
}

fn finalize_post_release_failure(
    worker: &mut native::ElevatedProcess,
    preflight: &ParentPreflight,
    current_user_residue: &mut ParentCurrentUserResidueState,
    parent_desktop_trees_settled: bool,
    reason: String,
) -> Result<i32, String> {
    let mut followup = None;
    let worker_tree_settled = match worker.terminate_and_settle() {
        Ok(()) => true,
        Err(error) => {
            followup = Some(error);
            false
        }
    };
    if let Some(error) = current_user_residue.cleanup_post_release_failure(
        &preflight.parent_current_user,
        worker_tree_settled && parent_desktop_trees_settled,
    ) {
        followup = Some(followup.map_or(error.clone(), |existing| {
            append_parent_followup_error(&existing, &error)
        }));
    }
    print_json(&ControllerOutcome {
        disposition: "parent_post_release_failed",
        reason: Some(reason),
        worker_failure_kind: None,
        attempted_stage: None,
        failure_evidence: None,
        failure_evidence_verified: None,
        evidence_error: None,
        restoration: None,
        abort: None,
        restoration_evidence_verified: None,
        parent_followup_error: followup,
        private_evidence: None,
        sanitized_export: None,
        success_evidence_verified: Some(false),
        profile: Some(preflight.plan.profile.clone()),
        controller_source_commit_sha: Some(preflight.source_commit_sha.clone()),
        evidence_root: None,
        preflight: Some(preflight.snapshot.clone()),
    });
    Ok(1)
}

fn restoration_evidence(restoration: &RestorationOutcome) -> Option<&EvidenceReceipt> {
    match restoration {
        RestorationOutcome::Restored { evidence }
        | RestorationOutcome::Failed {
            evidence: Some(evidence),
            ..
        } => Some(evidence),
        RestorationOutcome::NotRequired
        | RestorationOutcome::BlockedUnsettled
        | RestorationOutcome::BlockedUntrusted { .. }
        | RestorationOutcome::Failed { evidence: None, .. } => None,
    }
}

fn run_worker(locator: &str) -> Result<i32, String> {
    lifecycle::require_controller_ready()?;
    let plan = parse_plan()?;
    let repo_root = compiled_repo_root()?;
    let source_commit_sha = embedded_source_commit()?;
    let controller = OwnedFile::open_current_executable()?;
    let baseline = open_candidate(&repo_root, &plan.baseline, "baseline_installer")?;
    let final_candidate = open_candidate(&repo_root, &plan.final_candidate, "final_installer")?;
    let incompatible_service_fixture = open_service_fixture(
        &repo_root,
        &plan.incompatible_service_fixture,
        "incompatible_service_fixture",
    )?;
    let rollback_failing_service_fixture = open_service_fixture(
        &repo_root,
        &plan.rollback_failing_service_fixture,
        "rollback_failing_service_fixture",
    )?;
    let mut pipe = native::connect_worker_pipe(locator, SESSION_TIMEOUT)?;
    let parent = native::authenticate_parent_peer(&pipe, &controller)?;

    let mut gates = ProtocolGates::new();
    let begin: Envelope<ParentMessage> = pipe.read_json(Duration::from_secs(30))?;
    let nonce = begin.nonce.clone();
    validate_nonce(&nonce)?;
    validate_envelope(&begin, &nonce, &mut gates.inbound)?;
    let ParentMessage::Begin(request) = begin.message else {
        return Err("lifecycle_worker_begin_required".to_string());
    };
    validate_sha256(&request.plan_sha256, "plan")?;
    if request.plan_sha256 != plan_sha256()
        || request.controller_source_commit_sha != source_commit_sha
        || request.controller_sha256 != controller.sha256_hex()
        || request.parent_process_id != parent.process_id
        || request.parent_started_at_100ns != parent.started_at_100ns
    {
        return Err("lifecycle_worker_plan_binding_invalid".to_string());
    }
    controller.revalidate()?;
    baseline.revalidate()?;
    final_candidate.revalidate()?;
    incompatible_service_fixture.revalidate()?;
    rollback_failing_service_fixture.revalidate()?;

    let evidence = native::create_protected_evidence_root(&nonce, &pipe)?;
    let worker_binding = native::current_controller_binding(&controller)?;
    let controller_bindings = [parent, worker_binding];
    send_worker_message(
        &mut pipe,
        &nonce,
        &mut gates.outbound,
        WorkerMessage::Accepted(crate::windows_lifecycle_proof_contract::WorkerAccepted {
            evidence_root: evidence.root().to_string_lossy().into_owned(),
            evidence_root_identity: evidence.identity(),
            worker_process_id: std::process::id(),
            worker_started_at_100ns: native::current_process_started_at()?,
        }),
    )?;

    let result = lifecycle::execute_worker(lifecycle::WorkerContext {
        plan: &plan,
        repo_root: &repo_root,
        baseline: &baseline,
        final_candidate: &final_candidate,
        incompatible_service_fixture: &incompatible_service_fixture,
        rollback_failing_service_fixture: &rollback_failing_service_fixture,
        evidence: &evidence,
        pipe: &mut pipe,
        nonce: &nonce,
        outbound_gate: &mut gates.outbound,
        inbound_gate: &mut gates.inbound,
        controller_bindings: &controller_bindings,
    });
    let aborted = result.abort.is_some();
    send_worker_message(
        &mut pipe,
        &nonce,
        &mut gates.outbound,
        WorkerMessage::ResultReady(Box::new(result.clone())),
    )?;
    let followup: Envelope<ParentMessage> = match pipe.read_json(SESSION_TIMEOUT) {
        Ok(followup) => followup,
        Err(reason) => {
            let abort = lifecycle::abort_after_result(
                &result,
                abort_reason_for_parent_error(&reason),
                &evidence,
                &controller_bindings,
            );
            let _ = send_worker_message(
                &mut pipe,
                &nonce,
                &mut gates.outbound,
                WorkerMessage::ResultReady(Box::new(abort)),
            );
            return Ok(1);
        }
    };
    let message = match validate_envelope(&followup, &nonce, &mut gates.inbound) {
        Ok(()) => followup.message,
        Err(_) => {
            let abort = lifecycle::abort_after_result(
                &result,
                AbortReason::ProtocolViolation,
                &evidence,
                &controller_bindings,
            );
            send_abort_result_and_wait_for_acceptance(&mut pipe, &nonce, &mut gates, abort)?;
            return Ok(1);
        }
    };
    if let ParentMessage::Abort(reason) = message {
        let abort = if result.abort.is_some_and(|abort| abort.reason == reason) {
            result
        } else {
            lifecycle::abort_after_result(&result, reason, &evidence, &controller_bindings)
        };
        send_abort_result_and_wait_for_acceptance(&mut pipe, &nonce, &mut gates, abort)?;
        return Ok(1);
    }
    if message != ParentMessage::EvidenceAccepted {
        return Err("lifecycle_parent_evidence_acceptance_required".to_string());
    }
    Ok(i32::from(aborted))
}

fn send_abort_result_and_wait_for_acceptance(
    pipe: &mut PipeConnection,
    nonce: &str,
    gates: &mut ProtocolGates,
    result: WorkerResult,
) -> Result<(), String> {
    let expected_reason = result
        .abort
        .map(|abort| abort.reason)
        .ok_or_else(|| "lifecycle_parent_abort_context_missing".to_string())?;
    send_worker_message(
        pipe,
        nonce,
        &mut gates.outbound,
        WorkerMessage::ResultReady(Box::new(result.clone())),
    )?;
    for _ in 0..4 {
        let followup: Envelope<ParentMessage> = pipe.read_json(Duration::from_secs(30))?;
        validate_envelope(&followup, nonce, &mut gates.inbound)?;
        match abort_acceptance_action(&followup.message, expected_reason)? {
            AbortAcceptanceAction::Complete => return Ok(()),
            AbortAcceptanceAction::RepeatResult => {
                send_worker_message(
                    pipe,
                    nonce,
                    &mut gates.outbound,
                    WorkerMessage::ResultReady(Box::new(result.clone())),
                )?;
            }
        }
    }
    Err("lifecycle_parent_abort_acceptance_timeout".to_string())
}

fn abort_acceptance_action(
    message: &ParentMessage,
    expected_reason: AbortReason,
) -> Result<AbortAcceptanceAction, String> {
    match message {
        ParentMessage::EvidenceAccepted => Ok(AbortAcceptanceAction::Complete),
        ParentMessage::Abort(reason) if *reason == expected_reason => {
            Ok(AbortAcceptanceAction::RepeatResult)
        }
        ParentMessage::Begin(_)
        | ParentMessage::CheckpointAccepted(_)
        | ParentMessage::DesktopPhaseComplete(_)
        | ParentMessage::Abort(_) => Err("lifecycle_parent_abort_acceptance_required".to_string()),
    }
}

fn open_candidate(
    repo_root: &Path,
    candidate: &crate::windows_lifecycle_proof_contract::Candidate,
    label: &str,
) -> Result<OwnedFile, String> {
    let path = repo_root.join(&candidate.installer_relative_path);
    let parent = path
        .parent()
        .ok_or_else(|| format!("lifecycle_{label}_parent_missing"))?;
    let canonical_parent = native::canonical_real_directory(parent, label)?;
    if !canonical_parent.starts_with(repo_root) {
        return Err(format!("lifecycle_{label}_parent_outside_repo"));
    }
    let file = OwnedFile::open(
        &path,
        candidate.installer_size,
        &candidate.installer_sha256,
        label,
    )?;
    file.require_under(repo_root, label)?;
    Ok(file)
}

fn open_service_fixture(
    repo_root: &Path,
    fixture: &crate::windows_lifecycle_proof_contract::ServiceFixture,
    label: &str,
) -> Result<OwnedFile, String> {
    let path = repo_root.join(&fixture.relative_path);
    let parent = path
        .parent()
        .ok_or_else(|| format!("lifecycle_{label}_parent_missing"))?;
    let canonical_parent = native::canonical_real_directory(parent, label)?;
    if !canonical_parent.starts_with(repo_root) {
        return Err(format!("lifecycle_{label}_parent_outside_repo"));
    }
    let file = OwnedFile::open(&path, fixture.size, &fixture.sha256, label)?;
    file.require_under(repo_root, label)?;
    Ok(file)
}

fn send_parent_message(
    pipe: &mut PipeConnection,
    nonce: &str,
    gate: &mut SequenceGate,
    message: ParentMessage,
) -> Result<(), String> {
    let message_sha256 = message_sha256(&message)?;
    let envelope = Envelope {
        schema_version: PROTOCOL_SCHEMA.to_string(),
        nonce: nonce.to_string(),
        sequence: gate.next()?,
        message_sha256,
        message,
    };
    pipe.write_json(&envelope)
}

fn send_worker_message(
    pipe: &mut PipeConnection,
    nonce: &str,
    gate: &mut SequenceGate,
    message: WorkerMessage,
) -> Result<(), String> {
    let message_sha256 = message_sha256(&message)?;
    let envelope = Envelope {
        schema_version: PROTOCOL_SCHEMA.to_string(),
        nonce: nonce.to_string(),
        sequence: gate.next()?,
        message_sha256,
        message,
    };
    pipe.write_json(&envelope)
}

fn validate_worker_result(result: &WorkerResult, expected_success: bool) -> Result<(), String> {
    if result.failure.is_none() != expected_success {
        return Err("lifecycle_worker_result_invalid".to_string());
    }
    let disposition_matches = matches!(
        (result.disposition, expected_success),
        (
            crate::windows_lifecycle_proof_contract::WorkerDisposition::Passed,
            true
        ) | (
            crate::windows_lifecycle_proof_contract::WorkerDisposition::Failed,
            false
        )
    );
    if !disposition_matches {
        return Err("lifecycle_worker_disposition_invalid".to_string());
    }
    if expected_success {
        if result.abort.is_some()
            || !result.process_tree_settled
            || result.completed_stage != Some(LifecycleStage::FinalUninstall)
            || result
                .last_authenticated_checkpoint
                .is_none_or(|checkpoint| {
                    checkpoint.completed_stage != LifecycleStage::FinalUninstall
                })
            || !valid_success_evidence_receipts(result)
        {
            return Err("lifecycle_worker_result_invalid".to_string());
        }
        return Ok(());
    }
    if !result.private_evidence.is_empty() {
        return Err("lifecycle_worker_failure_evidence_state_invalid".to_string());
    }
    let failure = result
        .failure
        .as_ref()
        .ok_or_else(|| "lifecycle_worker_failure_missing".to_string())?;
    if result
        .last_authenticated_checkpoint
        .is_some_and(|checkpoint| {
            checkpoint.evidence_root_identity.volume_serial == 0
                || checkpoint.evidence_root_identity.file_index == 0
                || result
                    .completed_stage
                    .is_none_or(|completed| checkpoint.completed_stage > completed)
        })
    {
        return Err("lifecycle_worker_checkpoint_invalid".to_string());
    }
    if let Some(abort) = result.abort {
        if abort.last_authenticated_checkpoint != result.last_authenticated_checkpoint
            || abort.evidence_root_identity.volume_serial == 0
            || abort.evidence_root_identity.file_index == 0
            || abort
                .last_authenticated_checkpoint
                .is_some_and(|checkpoint| {
                    checkpoint.evidence_root_identity != abort.evidence_root_identity
                        || result
                            .completed_stage
                            .is_none_or(|completed| checkpoint.completed_stage > completed)
                })
        {
            return Err("lifecycle_worker_abort_state_invalid".to_string());
        }
    }
    if failure.reason.is_empty() {
        return Err("lifecycle_worker_failure_reason_invalid".to_string());
    }
    if result.process_tree_settled
        == matches!(
            failure.restoration.as_ref(),
            RestorationOutcome::BlockedUnsettled
        )
    {
        return Err("lifecycle_worker_settlement_restoration_invalid".to_string());
    }
    let failure_shape_valid = match failure.kind {
        WorkerFailureKind::Mutation => {
            failure.evidence.is_some()
                && failure.evidence_error.is_none()
                && mutation_failure_binding(result, failure).is_some_and(|expected_name| {
                    failure
                        .evidence
                        .as_ref()
                        .is_some_and(|receipt| receipt.name == expected_name)
                })
        }
        WorkerFailureKind::EvidenceWrite => {
            failure.evidence.is_none()
                && failure.evidence_error.is_some()
                && (failure.attempted_stage.is_none()
                    || mutation_failure_binding(result, failure).is_some())
        }
        WorkerFailureKind::ProcessSettlement => {
            !result.process_tree_settled
                && failure.restoration.as_ref() == &RestorationOutcome::BlockedUnsettled
                && (failure.evidence.is_some() != failure.evidence_error.is_some())
                && mutation_failure_binding(result, failure).is_some_and(|expected_name| {
                    failure
                        .evidence
                        .as_ref()
                        .is_none_or(|receipt| receipt.name == expected_name)
                })
        }
        WorkerFailureKind::Controller => {
            failure.evidence.is_none()
                && failure.evidence_error.is_none()
                && controller_attempt_binding_valid(result, failure)
        }
        WorkerFailureKind::ParentAbort => {
            (failure.evidence.is_some() != failure.evidence_error.is_some())
                && fallback_attempt_binding_valid(result, failure)
                && result.abort.is_some_and(|abort| {
                    failure.reason == parent_abort_reason(abort.reason)
                        && abort.last_authenticated_checkpoint
                            == result.last_authenticated_checkpoint
                        && abort.evidence_root_identity.volume_serial != 0
                        && abort.evidence_root_identity.file_index != 0
                        && abort
                            .last_authenticated_checkpoint
                            .is_none_or(|checkpoint| {
                                checkpoint.evidence_root_identity == abort.evidence_root_identity
                                    && result.completed_stage.is_some_and(|completed| {
                                        checkpoint.completed_stage <= completed
                                    })
                            })
                })
                && result.completed_stage.is_some_and(|stage| {
                    failure.evidence.as_ref().is_none_or(|receipt| {
                        receipt.name == parent_abort_leaf_for_stage(stage)
                            && receipt.size > 0
                            && receipt.size <= 8 * 1024 * 1024
                            && validate_sha256(&receipt.sha256, "parent_abort_evidence").is_ok()
                    })
                })
        }
    };
    if !failure_shape_valid {
        return Err("lifecycle_worker_failure_shape_invalid".to_string());
    }
    validate_restoration_for_failure(result, failure)?;
    Ok(())
}

fn validate_restoration_for_failure(
    result: &WorkerResult,
    failure: &crate::windows_lifecycle_proof_contract::WorkerFailure,
) -> Result<(), String> {
    if !result.process_tree_settled {
        return Ok(());
    }
    match failure.kind {
        WorkerFailureKind::ProcessSettlement => {
            Err("lifecycle_worker_restoration_disposition_invalid".to_string())
        }
        WorkerFailureKind::Mutation => validate_required_restoration(result, failure),
        WorkerFailureKind::ParentAbort => {
            if result.completed_stage == Some(LifecycleStage::InitialState) {
                if failure.restoration.as_ref() == &RestorationOutcome::NotRequired {
                    Ok(())
                } else {
                    Err("lifecycle_worker_restoration_disposition_invalid".to_string())
                }
            } else {
                validate_required_restoration(result, failure)
            }
        }
        WorkerFailureKind::EvidenceWrite | WorkerFailureKind::Controller => {
            if failure_requires_restoration(result, failure) {
                validate_required_restoration(result, failure)
            } else if failure.restoration.as_ref() == &RestorationOutcome::NotRequired {
                Ok(())
            } else {
                Err("lifecycle_worker_restoration_disposition_invalid".to_string())
            }
        }
    }
}

fn validate_required_restoration(
    result: &WorkerResult,
    failure: &crate::windows_lifecycle_proof_contract::WorkerFailure,
) -> Result<(), String> {
    let expected_name = restoration_leaf_for_failure(result, failure)
        .ok_or_else(|| "lifecycle_worker_restoration_stage_invalid".to_string())?;
    match failure.restoration.as_ref() {
        RestorationOutcome::Restored { evidence } => {
            validate_restoration_receipt(evidence, expected_name, failure.evidence.as_deref())
        }
        RestorationOutcome::BlockedUntrusted { reason } => {
            validate_restoration_reason(reason, "blocked_untrusted")
        }
        RestorationOutcome::Failed {
            reason,
            evidence,
            evidence_error,
        } => {
            validate_restoration_reason(reason, "failed")?;
            if evidence.is_some() == evidence_error.is_some() {
                return Err("lifecycle_worker_restoration_failure_shape_invalid".to_string());
            }
            if let Some(receipt) = evidence {
                validate_restoration_receipt(receipt, expected_name, failure.evidence.as_deref())?;
            }
            if evidence_error.as_deref().is_some_and(str::is_empty) {
                return Err("lifecycle_worker_restoration_failure_shape_invalid".to_string());
            }
            Ok(())
        }
        RestorationOutcome::NotRequired | RestorationOutcome::BlockedUnsettled => {
            Err("lifecycle_worker_restoration_disposition_invalid".to_string())
        }
    }
}

fn failure_requires_restoration(
    result: &WorkerResult,
    failure: &crate::windows_lifecycle_proof_contract::WorkerFailure,
) -> bool {
    match failure.kind {
        WorkerFailureKind::Mutation => true,
        WorkerFailureKind::ProcessSettlement => false,
        WorkerFailureKind::ParentAbort => {
            result.completed_stage != Some(LifecycleStage::InitialState)
        }
        WorkerFailureKind::EvidenceWrite | WorkerFailureKind::Controller => {
            restoration_stage(result, failure)
                .is_some_and(|stage| stage != LifecycleStage::InitialState)
        }
    }
}

fn validate_parent_failure_completion(
    result: &WorkerResult,
    failure: &crate::windows_lifecycle_proof_contract::WorkerFailure,
    desktop_phase_index: usize,
) -> Result<(), String> {
    if failure
        .attempted_stage
        .is_some_and(|stage| desktop_phase_index < minimum_desktop_phase_index_before_stage(stage))
    {
        return Err("lifecycle_worker_failure_desktop_phases_incomplete".to_string());
    }
    let authenticated_stage = result
        .last_authenticated_checkpoint
        .map(|checkpoint| checkpoint.completed_stage);
    if failure.kind == WorkerFailureKind::EvidenceWrite
        && failure.attempted_stage.is_none()
        && result.completed_stage != authenticated_stage
        && result.completed_stage.is_some_and(|stage| {
            desktop_phase_index != expected_desktop_phase_index_at_checkpoint(stage)
        })
    {
        return Err("lifecycle_worker_failure_desktop_phases_incomplete".to_string());
    }
    Ok(())
}

fn validate_parent_failure_progress(result: &WorkerResult) -> Result<(), String> {
    let authenticated_stage = result
        .last_authenticated_checkpoint
        .map(|checkpoint| checkpoint.completed_stage);
    if result.completed_stage == authenticated_stage {
        return Ok(());
    }
    let failure = result
        .failure
        .as_ref()
        .ok_or_else(|| "lifecycle_worker_failure_missing".to_string())?;
    if failure.kind == WorkerFailureKind::EvidenceWrite
        && failure.attempted_stage.is_none()
        && authenticated_stage.and_then(|stage| next_lifecycle_stage(Some(stage)))
            == result.completed_stage
    {
        Ok(())
    } else {
        Err("lifecycle_worker_failure_progress_invalid".to_string())
    }
}

fn parent_abort_leaf_for_stage(stage: LifecycleStage) -> &'static str {
    match stage {
        LifecycleStage::InitialState => "initial-state-parent-abort.private.json",
        LifecycleStage::FinalRepair => "final-repair-parent-abort.private.json",
        LifecycleStage::InitialUninstall => "initial-uninstall-parent-abort.private.json",
        LifecycleStage::BaselineInstall => "baseline-install-parent-abort.private.json",
        LifecycleStage::BaselineRestart => "baseline-restart-parent-abort.private.json",
        LifecycleStage::BaselineCrashRecovery => {
            "baseline-crash-recovery-parent-abort.private.json"
        }
        LifecycleStage::BaselineRollbackRecovery => {
            "baseline-rollback-recovery-parent-abort.private.json"
        }
        LifecycleStage::LegacyResidueSeeded => "legacy-residue-seeded-parent-abort.private.json",
        LifecycleStage::FinalUpgrade => "final-upgrade-parent-abort.private.json",
        LifecycleStage::FinalRestart => "final-restart-parent-abort.private.json",
        LifecycleStage::FinalCrashRecovery => "final-crash-recovery-parent-abort.private.json",
        LifecycleStage::FinalFallbackStates => "final-fallback-states-parent-abort.private.json",
        LifecycleStage::FinalUninstall => "final-uninstall-parent-abort.private.json",
    }
}

fn parent_abort_reason(reason: AbortReason) -> &'static str {
    match reason {
        AbortReason::ArtifactValidation => "lifecycle_parent_abort_artifact_validation",
        AbortReason::DesktopFailure => "lifecycle_parent_abort_desktop_failure",
        AbortReason::Disconnected => "lifecycle_parent_abort_disconnected",
        AbortReason::EvidenceValidation => "lifecycle_parent_abort_evidence_validation",
        AbortReason::ProtocolViolation => "lifecycle_parent_abort_protocol_violation",
        AbortReason::ReceiptValidation => "lifecycle_parent_abort_receipt_validation",
        AbortReason::Timeout => "lifecycle_parent_abort_timeout",
    }
}

fn restoration_leaf_for_failure(
    result: &WorkerResult,
    failure: &crate::windows_lifecycle_proof_contract::WorkerFailure,
) -> Option<&'static str> {
    restoration_leaf_for_stage(restoration_stage(result, failure)?)
}

fn restoration_leaf_for_stage(stage: LifecycleStage) -> Option<&'static str> {
    match stage {
        LifecycleStage::InitialState => None,
        LifecycleStage::FinalRepair => Some("final-repair-restoration.private.json"),
        LifecycleStage::InitialUninstall => Some("initial-uninstall-restoration.private.json"),
        LifecycleStage::BaselineInstall => Some("baseline-install-restoration.private.json"),
        LifecycleStage::BaselineRestart => Some("baseline-restart-restoration.private.json"),
        LifecycleStage::BaselineCrashRecovery => {
            Some("baseline-crash-recovery-restoration.private.json")
        }
        LifecycleStage::BaselineRollbackRecovery => {
            Some("baseline-rollback-recovery-restoration.private.json")
        }
        LifecycleStage::LegacyResidueSeeded => {
            Some("legacy-residue-seeded-restoration.private.json")
        }
        LifecycleStage::FinalUpgrade => Some("final-upgrade-restoration.private.json"),
        LifecycleStage::FinalRestart => Some("final-restart-restoration.private.json"),
        LifecycleStage::FinalCrashRecovery => Some("final-crash-recovery-restoration.private.json"),
        LifecycleStage::FinalFallbackStates => {
            Some("final-fallback-states-restoration.private.json")
        }
        LifecycleStage::FinalUninstall => Some("final-uninstall-restoration.private.json"),
    }
}

fn restoration_stage(
    result: &WorkerResult,
    failure: &crate::windows_lifecycle_proof_contract::WorkerFailure,
) -> Option<LifecycleStage> {
    failure.attempted_stage.or(result.completed_stage)
}

fn validate_restoration_reason(reason: &str, label: &str) -> Result<(), String> {
    if reason.is_empty() || reason.len() > 192 {
        Err(format!(
            "lifecycle_worker_restoration_{label}_reason_invalid"
        ))
    } else {
        Ok(())
    }
}

fn validate_restoration_receipt(
    receipt: &EvidenceReceipt,
    expected_name: &str,
    failure_receipt: Option<&EvidenceReceipt>,
) -> Result<(), String> {
    if receipt.size == 0
        || receipt.size > 8 * 1024 * 1024
        || receipt.name != expected_name
        || failure_receipt.is_some_and(|failure| failure.name == receipt.name)
    {
        return Err("lifecycle_worker_restoration_receipt_invalid".to_string());
    }
    validate_sha256(&receipt.sha256, "restoration_evidence")
}

fn valid_success_evidence_receipts(result: &WorkerResult) -> bool {
    let mut names = std::collections::BTreeSet::new();
    if result.private_evidence.len() != SUCCESS_PRIVATE_EVIDENCE_LEAVES.len()
        || result
            .private_evidence
            .iter()
            .zip(SUCCESS_PRIVATE_EVIDENCE_LEAVES)
            .any(|(receipt, expected_name)| {
                receipt.size == 0
                    || receipt.size > 8 * 1024 * 1024
                    || !receipt.name.ends_with(".private.json")
                    || receipt.name != expected_name
                    || !valid_evidence_leaf(&receipt.name)
                    || validate_sha256(&receipt.sha256, "private_evidence").is_err()
                    || !names.insert(receipt.name.as_str())
            })
    {
        return false;
    }
    names
        == SUCCESS_PRIVATE_EVIDENCE_LEAVES
            .into_iter()
            .collect::<std::collections::BTreeSet<_>>()
}

fn valid_evidence_leaf(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
        && !value.starts_with('.')
        && !value.contains("..")
}

fn mutation_failure_binding(
    result: &WorkerResult,
    failure: &crate::windows_lifecycle_proof_contract::WorkerFailure,
) -> Option<&'static str> {
    match (failure.attempted_stage?, result.completed_stage) {
        (LifecycleStage::FinalRepair, Some(LifecycleStage::InitialState)) => {
            Some("final-repair-failure.private.json")
        }
        (LifecycleStage::InitialUninstall, Some(LifecycleStage::FinalRepair)) => {
            Some("initial-uninstall-failure.private.json")
        }
        (LifecycleStage::BaselineInstall, Some(LifecycleStage::InitialUninstall)) => {
            Some("baseline-install-failure.private.json")
        }
        (LifecycleStage::BaselineRestart, Some(LifecycleStage::BaselineInstall)) => {
            Some("baseline-restart-failure.private.json")
        }
        (LifecycleStage::BaselineCrashRecovery, Some(LifecycleStage::BaselineRestart)) => {
            Some("baseline-crash-recovery-failure.private.json")
        }
        (LifecycleStage::BaselineRollbackRecovery, Some(LifecycleStage::BaselineCrashRecovery)) => {
            Some("baseline-rollback-recovery-failure.private.json")
        }
        (LifecycleStage::FinalUpgrade, Some(LifecycleStage::LegacyResidueSeeded)) => {
            Some("final-upgrade-failure.private.json")
        }
        (LifecycleStage::FinalRestart, Some(LifecycleStage::FinalUpgrade)) => {
            Some("final-restart-failure.private.json")
        }
        (LifecycleStage::FinalCrashRecovery, Some(LifecycleStage::FinalRestart)) => {
            Some("final-crash-recovery-failure.private.json")
        }
        (LifecycleStage::FinalFallbackStates, Some(LifecycleStage::FinalCrashRecovery)) => {
            Some("final-fallback-states-failure.private.json")
        }
        (LifecycleStage::FinalUninstall, Some(LifecycleStage::FinalFallbackStates)) => {
            Some("final-uninstall-failure.private.json")
        }
        _ => None,
    }
}

fn fallback_attempt_binding_valid(
    result: &WorkerResult,
    failure: &crate::windows_lifecycle_proof_contract::WorkerFailure,
) -> bool {
    failure.attempted_stage.is_none()
        || (failure.attempted_stage == Some(LifecycleStage::FinalFallbackStates)
            && result.completed_stage == Some(LifecycleStage::FinalCrashRecovery))
}

fn controller_attempt_binding_valid(
    result: &WorkerResult,
    failure: &crate::windows_lifecycle_proof_contract::WorkerFailure,
) -> bool {
    let fallback_desktop_failure = [
        DesktopPhase::FinalMissingService,
        DesktopPhase::FinalStoppedService,
        DesktopPhase::FinalIncompatibleService,
    ]
    .into_iter()
    .any(|phase| {
        failure
            .reason
            .starts_with(&format!("{}:", lifecycle::desktop_evidence_name(phase)))
    });
    fallback_attempt_binding_valid(result, failure)
        && (!fallback_desktop_failure
            || failure.attempted_stage == Some(LifecycleStage::FinalFallbackStates))
}

fn valid_settled_fallback_restoration(restoration: &RestorationOutcome) -> bool {
    const LEAF: &str = "final-fallback-states-restoration.private.json";
    match restoration {
        RestorationOutcome::Restored { evidence } => evidence.name == LEAF,
        RestorationOutcome::Failed {
            reason,
            evidence,
            evidence_error,
        } => match reason.as_str() {
            "lifecycle_fallback_restoration_failed" => {
                (evidence.is_some() != evidence_error.is_some())
                    && evidence.as_ref().is_none_or(|receipt| receipt.name == LEAF)
            }
            "lifecycle_fallback_restoration_evidence_failed" => {
                evidence.is_none() && evidence_error.is_some()
            }
            _ => false,
        },
        RestorationOutcome::NotRequired
        | RestorationOutcome::BlockedUnsettled
        | RestorationOutcome::BlockedUntrusted { .. } => false,
    }
}

fn compiled_repo_root() -> Result<PathBuf, String> {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let root = manifest
        .parent()
        .and_then(Path::parent)
        .and_then(Path::parent)
        .ok_or_else(|| "lifecycle_compiled_repo_root_invalid".to_string())?;
    native::canonical_real_directory(root, "compiled_repo_root")
}

fn embedded_source_commit() -> Result<String, String> {
    let value = option_env!("BATCAVE_SOURCE_COMMIT_SHA").unwrap_or("");
    if value.len() != 40 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err("lifecycle_controller_source_commit_missing".to_string());
    }
    Ok(value.to_ascii_lowercase())
}

fn print_json<T: Serialize>(value: &T) {
    println!(
        "{}",
        serde_json::to_string(value).unwrap_or_else(|_| {
            r#"{"disposition":"failed","reason":"lifecycle_output_serialize_failed"}"#.to_string()
        })
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::windows_lifecycle_proof_contract::{
        EvidenceReceipt, WorkerDisposition, WorkerFailure, WorkerFailureKind, WorkerResult,
    };

    #[test]
    fn entry_is_closed_and_worker_accepts_only_a_locator() {
        assert!(matches!(
            parse_entry(vec!["preflight".to_string()]),
            Ok(Entry::Preflight)
        ));
        assert!(matches!(
            parse_entry(vec!["run".to_string()]),
            Ok(Entry::Run)
        ));
        assert!(parse_entry(vec![
            "--worker".to_string(),
            "a".repeat(crate::windows_lifecycle_proof_contract::LOCATOR_HEX_LENGTH),
        ])
        .is_ok());
        assert!(parse_entry(vec![
            "run".to_string(),
            "--installer".to_string(),
            "hostile.exe".to_string(),
        ])
        .is_err());
    }

    #[test]
    fn retention_capture_is_bound_only_to_the_two_authenticated_checkpoints() {
        assert_eq!(
            parent_retention_checkpoint_action(LifecycleStage::FinalRestart, false, false),
            Ok(ParentRetentionCheckpointAction::None)
        );
        assert_eq!(
            parent_retention_checkpoint_action(LifecycleStage::FinalFallbackStates, false, false,),
            Ok(ParentRetentionCheckpointAction::CaptureBefore)
        );
        assert_eq!(
            parent_retention_checkpoint_action(LifecycleStage::FinalUninstall, true, false),
            Ok(ParentRetentionCheckpointAction::CaptureAfter)
        );
        assert!(parent_retention_checkpoint_action(
            LifecycleStage::FinalFallbackStates,
            true,
            false,
        )
        .is_err());
        assert!(
            parent_retention_checkpoint_action(LifecycleStage::FinalUninstall, false, false,)
                .is_err()
        );
        assert!(
            parent_retention_checkpoint_action(LifecycleStage::FinalUninstall, true, true,)
                .is_err()
        );
    }

    #[test]
    fn stage_engine_order_preserves_parent_current_user_timing() {
        assert_eq!(
            DESKTOP_PHASES,
            [
                DesktopPhase::FinalPrimary,
                DesktopPhase::BaselinePrimary,
                DesktopPhase::BaselineSecondInstance,
                DesktopPhase::FinalMissingService,
                DesktopPhase::FinalStoppedService,
                DesktopPhase::FinalIncompatibleService,
            ]
        );
        assert_eq!(
            checkpoint_parent_residue_expectation(LifecycleStage::BaselineRollbackRecovery),
            ParentResidueExpectation::Clean
        );
        for stage in [
            LifecycleStage::LegacyResidueSeeded,
            LifecycleStage::FinalUpgrade,
            LifecycleStage::FinalRestart,
            LifecycleStage::FinalCrashRecovery,
        ] {
            assert_eq!(
                checkpoint_parent_residue_expectation(stage),
                ParentResidueExpectation::SeededKnownHelpers
            );
        }
        assert_eq!(
            desktop_parent_residue_expectation(DesktopPhase::FinalMissingService),
            ParentResidueExpectation::SeededHelpersRemoved
        );
        assert_eq!(
            checkpoint_parent_residue_expectation(LifecycleStage::FinalFallbackStates),
            ParentResidueExpectation::SeededHelpersRemoved
        );
        assert_eq!(
            checkpoint_parent_residue_expectation(LifecycleStage::FinalUninstall),
            ParentResidueExpectation::FinalUninstalled
        );
    }

    #[test]
    fn parent_current_user_capture_failure_withholds_authenticated_acknowledgement() {
        let checkpoint = WorkerCheckpoint {
            completed_stage: LifecycleStage::FinalRepair,
            evidence_root_identity: crate::windows_lifecycle_proof_contract::EvidenceRootIdentity {
                volume_serial: 1,
                file_index: 2,
            },
        };
        assert_eq!(
            checkpoint_acceptance_after_parent_capture(
                checkpoint,
                Err("hostile_parent_capture_failed".to_string()),
            ),
            Err("hostile_parent_capture_failed".to_string())
        );
        assert_eq!(
            checkpoint_acceptance_after_parent_capture(checkpoint, Ok(())),
            Ok(ParentMessage::CheckpointAccepted(checkpoint))
        );

        let desktop = DesktopPhaseResult {
            phase: DesktopPhase::FinalMissingService,
            disposition: DesktopPhaseDisposition::Passed,
            process_tree_settled: true,
            observation: None,
            failure_reason: None,
        };
        assert_eq!(
            desktop_completion_after_parent_capture(
                desktop.clone(),
                Err("hostile_parent_capture_failed".to_string()),
            ),
            Err("hostile_parent_capture_failed".to_string())
        );
        assert_eq!(
            desktop_completion_after_parent_capture(desktop.clone(), Ok(())),
            Ok(ParentMessage::DesktopPhaseComplete(Box::new(desktop)))
        );
    }

    #[test]
    fn production_terminal_routes_are_wired_once_in_their_bounded_contexts() {
        let source = include_str!("windows_lifecycle_proof.rs").replace("\r\n", "\n");
        let (production, _) = source
            .split_once("\n#[cfg(test)]\nmod tests {")
            .expect("production/test source boundary");

        fn bounded<'a>(source: &'a str, start: &str, end: &str) -> &'a str {
            assert_eq!(
                source.matches(start).count(),
                1,
                "bounded context start must be unique: {start}"
            );
            assert_eq!(
                source.matches(end).count(),
                1,
                "bounded context end must be unique: {end}"
            );
            let tail = source.split_once(start).expect("bounded context start").1;
            tail.split_once(end).expect("bounded context end").0
        }

        let success_arm = bounded(
            production,
            "                WorkerMessage::ResultReady(result) if result.failure.is_none() => {",
            "                WorkerMessage::ResultReady(result) => {",
        );
        let success_terminal = bounded(
            success_arm,
            "                    let exit_code = worker.wait(Duration::from_secs(30))?;",
            "                    print_json(&ControllerOutcome {",
        );

        let worker_failure_arm = bounded(
            production,
            "                WorkerMessage::ResultReady(result) => {",
            "    match session {",
        );
        let worker_failure_terminal = bounded(
            worker_failure_arm,
            "                        if !terminate_without_ack {",
            "                        Ok(())\n                    })();",
        );

        let abort_function = bounded(
            production,
            "fn abort_parent_session(",
            "fn abort_reason_for_parent_error(",
        );
        let authenticated_abort_terminal = bounded(
            abort_function,
            "            if !unsettled_abort {",
            "            Ok(result)\n        })();\n        match followup {",
        );
        let last_resort_abort_terminal = bounded(
            abort_function,
            "    let (restoration, worker_settled) = match worker",
            "    if let Err(error) = revalidate_preflight_artifacts(preflight) {",
        );
        let post_release_terminal = bounded(
            production,
            "fn finalize_post_release_failure(",
            "fn restoration_evidence(",
        );

        let routes = [
            (".cleanup_success(", success_terminal),
            (".cleanup_worker_failure(", worker_failure_terminal),
            (".cleanup_post_release_failure(", post_release_terminal),
            (
                ".cleanup_authenticated_abort(",
                authenticated_abort_terminal,
            ),
            (".cleanup_last_resort_abort(", last_resort_abort_terminal),
        ];
        for (expected_index, (invocation, context)) in routes.iter().enumerate() {
            assert_eq!(
                production.matches(invocation).count(),
                1,
                "production route must have one invocation: {invocation}"
            );
            for (actual_index, (candidate, _)) in routes.iter().enumerate() {
                assert_eq!(
                    context.matches(candidate).count(),
                    usize::from(actual_index == expected_index),
                    "terminal route invocation is missing, duplicated, or swapped: {candidate}"
                );
            }
        }
    }

    #[test]
    fn parent_publication_order_crosses_ack_only_after_preparation_and_writes_after_job_zero() {
        let source = include_str!("windows_lifecycle_proof.rs").replace("\r\n", "\n");
        let (production, _) = source
            .split_once("\n#[cfg(test)]\nmod tests {")
            .expect("production/test source boundary");
        let run_parent = production
            .split_once("fn run_parent()")
            .expect("run parent")
            .1
            .split_once("fn validate_requested_desktop_phase_result(")
            .expect("run parent end")
            .0;
        let pin = run_parent
            .find("native::pin_parent_export_directory")
            .expect("pre-UAC export pin");
        let launch = run_parent
            .find("native::launch_elevated_worker")
            .expect("worker launch");
        assert!(pin < launch, "destination authority must precede UAC");

        let success = run_parent
            .split_once(
                "                WorkerMessage::ResultReady(result) if result.failure.is_none() => {",
            )
            .expect("success result arm")
            .1
            .split_once("                WorkerMessage::ResultReady(result) => {")
            .expect("success result arm end")
            .0;
        let ordered = [
            "evidence::derive_sanitized_export(",
            "revalidate_success_sources(",
            "worker_released = true;",
            "ParentMessage::EvidenceAccepted,",
            "let exit_code = worker.wait(Duration::from_secs(30))?;",
            "native::write_parent_export_new",
            "sanitized_export.revalidate(&export_directory)?;",
            ".cleanup_success(&preflight.parent_current_user)?;",
            "disposition: \"passed\"",
        ];
        let positions = ordered
            .iter()
            .map(|needle| {
                success
                    .find(needle)
                    .unwrap_or_else(|| panic!("missing ordered publication step: {needle}"))
            })
            .collect::<Vec<_>>();
        assert!(positions.windows(2).all(|pair| pair[0] < pair[1]));

        assert!(run_parent.contains("Err(reason) if worker_released =>"));
        let post_release = production
            .split_once("fn finalize_post_release_failure(")
            .expect("post-release finalizer")
            .1
            .split_once("fn restoration_evidence(")
            .expect("post-release finalizer end")
            .0;
        assert!(!post_release.contains("abort_parent_session("));
        assert!(!post_release.contains("send_parent_message("));
        assert!(post_release.contains(".terminate_and_settle()"));
        assert!(post_release.contains("sanitized_export: None"));
    }

    struct IsolatedParentResidueTerminal {
        transaction: Option<()>,
    }

    impl IsolatedParentResidueTerminal {
        fn pending() -> Self {
            Self {
                transaction: Some(()),
            }
        }

        fn assert_restored(&self) {
            assert!(
                self.transaction.is_none(),
                "terminal retained residue authority"
            );
        }

        fn assert_retained(&self) {
            assert!(
                self.transaction.is_some(),
                "unsettled terminal route deleted retained residue authority"
            );
        }
    }

    impl ParentResidueTerminalCleanup for IsolatedParentResidueTerminal {
        type Authority = ();

        fn cleanup_parent_residue(
            &mut self,
            _authority: &Self::Authority,
            worker_tree_settled: bool,
        ) -> Result<(), String> {
            finalize_parent_residue_terminal(&mut self.transaction, worker_tree_settled, |_| {
                native::exercise_isolated_parent_current_user_residue_cleanup()
            })
        }
    }

    #[test]
    fn success_terminal_route_restores_filesystem_and_isolated_run_key() {
        let mut residue = IsolatedParentResidueTerminal::pending();
        residue.cleanup_success(&()).expect("success cleanup");
        residue.assert_restored();
    }

    #[test]
    fn worker_failure_terminal_route_restores_filesystem_and_isolated_run_key() {
        let mut residue = IsolatedParentResidueTerminal::pending();
        assert_eq!(residue.cleanup_worker_failure(&(), true), None);
        residue.assert_restored();
    }

    #[test]
    fn authenticated_abort_terminal_route_restores_filesystem_and_isolated_run_key() {
        let mut residue = IsolatedParentResidueTerminal::pending();
        assert_eq!(residue.cleanup_authenticated_abort(&(), true), None);
        residue.assert_restored();
    }

    #[test]
    fn unsettled_terminal_routes_retain_parent_user_residue() {
        let expected = Some("lifecycle_parent_user_cleanup_blocked_unsettled".to_string());

        let mut worker_failure = IsolatedParentResidueTerminal::pending();
        assert_eq!(worker_failure.cleanup_worker_failure(&(), false), expected);
        worker_failure.assert_retained();

        let mut authenticated_abort = IsolatedParentResidueTerminal::pending();
        assert_eq!(
            authenticated_abort.cleanup_authenticated_abort(&(), false),
            expected
        );
        authenticated_abort.assert_retained();

        let mut last_resort = IsolatedParentResidueTerminal::pending();
        let mut followup_error = "authenticated_abort_failed".to_string();
        last_resort.cleanup_last_resort_abort(&(), false, &mut followup_error);
        assert_eq!(
            followup_error,
            "authenticated_abort_failed|abort_followup:lifecycle_parent_user_cleanup_blocked_unsettled"
        );
        last_resort.assert_retained();
    }

    #[test]
    fn last_resort_abort_terminal_route_restores_filesystem_and_isolated_run_key() {
        let mut residue = IsolatedParentResidueTerminal::pending();
        let mut followup_error = "authenticated_abort_failed".to_string();
        residue.cleanup_last_resort_abort(&(), true, &mut followup_error);
        assert_eq!(followup_error, "authenticated_abort_failed");
        residue.assert_restored();
    }

    #[test]
    fn parent_residue_terminal_cleanup_is_absence_and_settlement_gated() {
        let mut absent = None::<()>;
        finalize_parent_residue_terminal(&mut absent, false, |_| {
            panic!("an absent transaction must not invoke cleanup")
        })
        .expect("absent transaction is already restored");

        let mut unsettled = Some(());
        assert_eq!(
            finalize_parent_residue_terminal(&mut unsettled, false, |_| {
                panic!("an unsettled worker must not invoke cleanup")
            }),
            Err("lifecycle_parent_user_cleanup_blocked_unsettled".to_string())
        );
        assert!(unsettled.is_some(), "unsettled authority must be retained");
    }

    #[test]
    fn parent_current_user_residue_timeline_rejects_unknown_and_hostile_drift() {
        let authority = parent_current_user_authority_for_test();
        let clean = parent_residue_snapshot_for_test(false, false, false);
        validate_parent_residue_snapshot(&clean, ParentResidueExpectation::Clean, &authority, None)
            .expect("clean baseline");
        for allowed_owner in [&authority.user_sid, "S-1-5-18", "S-1-5-32-544"] {
            let mut owned = clean.clone();
            owned.hkcu_run.owner_sid = allowed_owner.to_string();
            validate_parent_residue_snapshot(
                &owned,
                ParentResidueExpectation::Clean,
                &authority,
                None,
            )
            .expect("allowed Run-key owner");
        }
        for rejected_owner in ["S-1-3-4", "S-1-5-11", "S-1-5-32-545", "S-1-5-21-2"] {
            let mut owned = clean.clone();
            owned.hkcu_run.owner_sid = rejected_owner.to_string();
            assert!(validate_parent_residue_snapshot(
                &owned,
                ParentResidueExpectation::Clean,
                &authority,
                None,
            )
            .is_err());
        }

        let seeded = parent_residue_snapshot_for_test(true, true, true);
        validate_parent_residue_snapshot(
            &seeded,
            ParentResidueExpectation::SeededKnownHelpers,
            &authority,
            None,
        )
        .expect("seeded fixtures");
        let anchor = parent_residue_sentinel(&seeded).expect("sentinel").clone();

        let cleaned = parent_residue_snapshot_for_test(true, false, true);
        validate_parent_residue_snapshot(
            &cleaned,
            ParentResidueExpectation::SeededHelpersRemoved,
            &authority,
            Some(&anchor),
        )
        .expect("post-launch cleanup");

        let final_uninstalled = parent_residue_snapshot_for_test(false, false, true);
        validate_parent_residue_snapshot(
            &final_uninstalled,
            ParentResidueExpectation::FinalUninstalled,
            &authority,
            Some(&anchor),
        )
        .expect("final uninstall");

        let mut authority_state = ParentCurrentUserResidueState::default();
        authority_state
            .validate_authority_anchors(&clean)
            .expect("initial Run authority");
        authority_state
            .validate_authority_anchors(&seeded)
            .expect("seeded helper authority");
        let mut run_owner_drift = seeded.clone();
        run_owner_drift.hkcu_run.owner_sid = "S-1-5-18".to_string();
        assert!(authority_state
            .validate_authority_anchors(&run_owner_drift)
            .is_err());
        let mut run_acl_drift = seeded.clone();
        run_acl_drift.hkcu_run.dacl_sha256 = "1".repeat(64);
        assert!(authority_state
            .validate_authority_anchors(&run_acl_drift)
            .is_err());
        let mut helper_acl_drift = seeded.clone();
        let Observation::Present(helper) = &mut helper_acl_drift.helper else {
            panic!("helper manifest");
        };
        helper.root_dacl_sha256 = "2".repeat(64);
        assert!(authority_state
            .validate_authority_anchors(&helper_acl_drift)
            .is_err());

        let mut wrong_type = seeded.clone();
        let Observation::Present(value) = &mut wrong_type.hkcu_run.batcave_monitor else {
            panic!("seeded run value");
        };
        value.value_type = 2;
        assert!(validate_parent_residue_snapshot(
            &wrong_type,
            ParentResidueExpectation::SeededKnownHelpers,
            &authority,
            None,
        )
        .is_err());

        let mut wrong_value = seeded.clone();
        let Observation::Present(value) = &mut wrong_value.hkcu_run.batcave_monitor else {
            panic!("seeded run value");
        };
        value.value.push_str(" --hostile");
        assert!(validate_parent_residue_snapshot(
            &wrong_value,
            ParentResidueExpectation::SeededKnownHelpers,
            &authority,
            None,
        )
        .is_err());

        let mut hostile_acl = seeded.clone();
        hostile_acl.hkcu_run.dacl_sha256 = "not-a-digest".to_string();
        assert!(validate_parent_residue_snapshot(
            &hostile_acl,
            ParentResidueExpectation::SeededKnownHelpers,
            &authority,
            None,
        )
        .is_err());

        let mut enumeration_drift = seeded.clone();
        enumeration_drift.hkcu_run.value_count = 0;
        assert!(validate_parent_residue_snapshot(
            &enumeration_drift,
            ParentResidueExpectation::SeededKnownHelpers,
            &authority,
            None,
        )
        .is_err());

        let mut unknown = seeded.clone();
        unknown.hkcu_run.batcave_monitor = Observation::Unknown("hostile".to_string());
        assert!(validate_parent_residue_snapshot(
            &unknown,
            ParentResidueExpectation::SeededKnownHelpers,
            &authority,
            None,
        )
        .is_err());

        let mut unexpected = seeded.clone();
        let Observation::Present(helper) = &mut unexpected.helper else {
            panic!("helper manifest");
        };
        helper.unexpected_entry_count = 1;
        assert!(validate_parent_residue_snapshot(
            &unexpected,
            ParentResidueExpectation::SeededKnownHelpers,
            &authority,
            None,
        )
        .is_err());

        let mut sentinel_drift = cleaned.clone();
        let Observation::Present(helper) = &mut sentinel_drift.helper else {
            panic!("helper manifest");
        };
        let Observation::Present(sentinel) = &mut helper.sentinel else {
            panic!("sentinel");
        };
        sentinel.file.identity.file_index += 1;
        assert!(validate_parent_residue_snapshot(
            &sentinel_drift,
            ParentResidueExpectation::SeededHelpersRemoved,
            &authority,
            Some(&anchor),
        )
        .is_err());
    }

    #[test]
    fn worker_messages_exclude_parent_current_user_raw_authority() {
        let worker = WorkerMessage::Checkpoint(WorkerCheckpoint {
            completed_stage: LifecycleStage::FinalRepair,
            evidence_root_identity: crate::windows_lifecycle_proof_contract::EvidenceRootIdentity {
                volume_serial: 1,
                file_index: 2,
            },
        });
        let json = serde_json::to_string(&worker).expect("worker json");
        for forbidden in [
            "user_sid",
            "hkcu_run",
            "local_app_data",
            "unknown-sentinel.bin",
            r"C:\Users",
        ] {
            assert!(!json.contains(forbidden), "worker leaked {forbidden}");
        }
    }

    #[test]
    fn parent_rejects_a_valid_result_for_the_wrong_requested_desktop_phase() {
        let plan = parse_plan().expect("plan");
        let result = DesktopPhaseResult {
            phase: DesktopPhase::BaselinePrimary,
            disposition: DesktopPhaseDisposition::Failed,
            process_tree_settled: false,
            observation: None,
            failure_reason: Some("lifecycle_desktop_phase_runner_failed".to_string()),
        };

        assert!(validate_requested_desktop_phase_result(
            DesktopPhase::BaselinePrimary,
            &result,
            &plan
        )
        .is_ok());
        assert_eq!(
            validate_requested_desktop_phase_result(DesktopPhase::FinalPrimary, &result, &plan),
            Err("lifecycle_desktop_phase_result_mismatch".to_string())
        );
    }

    #[test]
    fn failed_desktop_result_requires_exact_worker_binding() {
        let mut primary_desktop = DesktopPhaseResult {
            phase: DesktopPhase::FinalPrimary,
            disposition: DesktopPhaseDisposition::Failed,
            process_tree_settled: true,
            observation: None,
            failure_reason: Some("lifecycle_desktop_window_failed".to_string()),
        };
        let mut primary_result = failed_result_at(
            LifecycleStage::FinalRepair,
            WorkerFailure {
                kind: WorkerFailureKind::Controller,
                attempted_stage: None,
                reason: "final-primary-desktop.private.json:lifecycle_desktop_window_failed"
                    .to_string(),
                evidence: None,
                evidence_error: None,
                restoration: restoration_not_reviewed(),
            },
        );
        assert!(validate_failed_desktop_worker_result(&primary_desktop, &primary_result).is_ok());

        primary_desktop.process_tree_settled = false;
        primary_result.process_tree_settled = false;
        *primary_result
            .failure
            .as_mut()
            .expect("failure")
            .restoration = RestorationOutcome::BlockedUnsettled;
        assert!(validate_failed_desktop_worker_result(&primary_desktop, &primary_result).is_ok());

        let mut fallback_desktop = DesktopPhaseResult {
            phase: DesktopPhase::FinalMissingService,
            disposition: DesktopPhaseDisposition::Failed,
            process_tree_settled: true,
            observation: None,
            failure_reason: Some("lifecycle_desktop_fallback_failed".to_string()),
        };
        let fallback_result = failed_result_at(
            LifecycleStage::FinalCrashRecovery,
            WorkerFailure {
                kind: WorkerFailureKind::Controller,
                attempted_stage: Some(LifecycleStage::FinalFallbackStates),
                reason:
                    "final-missing-service-desktop.private.json:lifecycle_desktop_fallback_failed"
                        .to_string(),
                evidence: None,
                evidence_error: None,
                restoration: Box::new(RestorationOutcome::Restored {
                    evidence: evidence_receipt("final-fallback-states-restoration.private.json"),
                }),
            },
        );
        assert!(validate_failed_desktop_worker_result(&fallback_desktop, &fallback_result).is_ok());
        let confirmed_pending = PendingFailedDesktopResult {
            result: fallback_desktop.clone(),
            completion_write_confirmed: true,
        };
        assert!(validate_queued_failed_desktop_result(
            Some(&confirmed_pending),
            false,
            &fallback_result,
        )
        .is_ok());

        let mut queued_mismatch = fallback_result.clone();
        queued_mismatch.failure.as_mut().expect("failure").reason =
            "final-missing-service-desktop.private.json:hostile_drift".to_string();
        assert!(validate_queued_failed_desktop_result(
            Some(&confirmed_pending),
            false,
            &queued_mismatch,
        )
        .is_err());

        let mut abort_successor = fallback_result.clone();
        abort_successor.abort = Some(WorkerAbort {
            reason: AbortReason::Timeout,
            last_authenticated_checkpoint: abort_successor.last_authenticated_checkpoint,
            evidence_root_identity: crate::windows_lifecycle_proof_contract::EvidenceRootIdentity {
                volume_serial: 1,
                file_index: 1,
            },
        });
        assert_eq!(
            validate_queued_failed_desktop_result(
                Some(&confirmed_pending),
                false,
                &abort_successor,
            ),
            Err("lifecycle_failed_desktop_original_result_required".to_string())
        );
        assert!(validate_queued_failed_desktop_result(
            Some(&confirmed_pending),
            true,
            &abort_successor,
        )
        .is_ok());
        let ambiguous_pending = PendingFailedDesktopResult {
            result: fallback_desktop.clone(),
            completion_write_confirmed: false,
        };
        assert!(validate_queued_failed_desktop_result(
            Some(&ambiguous_pending),
            false,
            &abort_successor,
        )
        .is_ok());

        let mut forged_settlement = fallback_result.clone();
        fallback_desktop.process_tree_settled = false;
        assert_eq!(
            validate_failed_desktop_worker_result(&fallback_desktop, &forged_settlement),
            Err("lifecycle_failed_desktop_settlement_mismatch".to_string())
        );

        forged_settlement
            .failure
            .as_mut()
            .expect("failure")
            .attempted_stage = None;
        assert_eq!(
            validate_worker_result(&forged_settlement, false),
            Err("lifecycle_worker_failure_shape_invalid".to_string())
        );

        let mut unsettled_fallback = fallback_result;
        unsettled_fallback.process_tree_settled = false;
        *unsettled_fallback
            .failure
            .as_mut()
            .expect("failure")
            .restoration = RestorationOutcome::BlockedUnsettled;
        assert!(
            validate_failed_desktop_worker_result(&fallback_desktop, &unsettled_fallback).is_ok()
        );
    }

    #[test]
    fn parent_classifies_disconnect_and_timeout_before_aborting() {
        assert_eq!(
            abort_reason_for_parent_error("lifecycle_pipe_closed"),
            AbortReason::Disconnected
        );
        assert_eq!(
            abort_reason_for_parent_error("lifecycle_pipe_read_timeout"),
            AbortReason::Timeout
        );
        assert_eq!(
            abort_reason_for_parent_error("lifecycle_desktop_phase_runner_failed"),
            AbortReason::DesktopFailure
        );
    }

    #[test]
    fn checkpoints_require_exact_successors_without_skips_or_replays() {
        let root = crate::windows_lifecycle_proof_contract::EvidenceRootIdentity {
            volume_serial: 7,
            file_index: 11,
        };
        let initial = WorkerCheckpoint {
            completed_stage: LifecycleStage::InitialState,
            evidence_root_identity: root,
        };
        let repair = WorkerCheckpoint {
            completed_stage: LifecycleStage::FinalRepair,
            evidence_root_identity: root,
        };
        let uninstall = WorkerCheckpoint {
            completed_stage: LifecycleStage::InitialUninstall,
            evidence_root_identity: root,
        };
        assert!(valid_checkpoint_transition(None, initial));
        assert!(valid_checkpoint_transition(Some(initial), repair));
        assert!(valid_checkpoint_transition(Some(repair), uninstall));
        assert!(!valid_checkpoint_transition(None, repair));
        assert!(!valid_checkpoint_transition(Some(initial), uninstall));
        assert!(!valid_checkpoint_transition(Some(repair), repair));
        assert!(!valid_checkpoint_transition(Some(repair), initial));
        assert!(valid_parent_checkpoint(initial, root, None, 0));
        assert!(!valid_parent_checkpoint(uninstall, root, Some(repair), 0));
        assert!(valid_parent_checkpoint(uninstall, root, Some(repair), 1));
        assert!(!valid_parent_checkpoint(
            uninstall,
            crate::windows_lifecycle_proof_contract::EvidenceRootIdentity {
                volume_serial: 9,
                file_index: 9,
            },
            Some(repair),
            1,
        ));
        assert_eq!(
            next_lifecycle_stage(Some(LifecycleStage::FinalUninstall)),
            None
        );

        let legal = [
            LifecycleStage::InitialState,
            LifecycleStage::FinalRepair,
            LifecycleStage::InitialUninstall,
            LifecycleStage::BaselineInstall,
            LifecycleStage::BaselineRestart,
            LifecycleStage::BaselineCrashRecovery,
            LifecycleStage::BaselineRollbackRecovery,
            LifecycleStage::LegacyResidueSeeded,
            LifecycleStage::FinalUpgrade,
            LifecycleStage::FinalRestart,
            LifecycleStage::FinalCrashRecovery,
            LifecycleStage::FinalFallbackStates,
            LifecycleStage::FinalUninstall,
        ];
        let mut previous = None;
        for stage in legal {
            assert_eq!(next_lifecycle_stage(previous), Some(stage));
            let checkpoint = WorkerCheckpoint {
                completed_stage: stage,
                evidence_root_identity: root,
            };
            assert!(valid_checkpoint_transition(
                previous.map(|completed_stage| WorkerCheckpoint {
                    completed_stage,
                    evidence_root_identity: root,
                }),
                checkpoint,
            ));
            previous = Some(stage);
        }
        assert_eq!(next_lifecycle_stage(previous), None);

        for (stage, expected_index) in [
            (LifecycleStage::InitialState, 0),
            (LifecycleStage::FinalRepair, 0),
            (LifecycleStage::InitialUninstall, 1),
            (LifecycleStage::BaselineInstall, 1),
            (LifecycleStage::BaselineRestart, 3),
            (LifecycleStage::FinalCrashRecovery, 3),
            (LifecycleStage::FinalFallbackStates, DESKTOP_PHASES.len()),
            (LifecycleStage::FinalUninstall, DESKTOP_PHASES.len()),
        ] {
            assert_eq!(
                expected_desktop_phase_index_at_checkpoint(stage),
                expected_index
            );
        }
    }

    #[test]
    fn independent_direction_sequences_allow_abort_to_cross_a_checkpoint() {
        let mut parent = ProtocolGates::new();
        let mut worker = ProtocolGates::new();

        let begin = parent.outbound.next().expect("parent begin");
        worker.inbound.accept(begin).expect("worker accepts begin");
        let checkpoint = worker.outbound.next().expect("worker checkpoint");
        let abort = parent.outbound.next().expect("parent abort");

        worker.inbound.accept(abort).expect("worker accepts abort");
        parent
            .inbound
            .accept(checkpoint)
            .expect("parent accepts crossed checkpoint");
        let result = worker.outbound.next().expect("worker abort result");
        parent
            .inbound
            .accept(result)
            .expect("parent accepts abort result");
    }

    #[test]
    fn parent_repeats_abort_for_crossed_checkpoint_and_desktop_requests() {
        let root = crate::windows_lifecycle_proof_contract::EvidenceRootIdentity {
            volume_serial: 7,
            file_index: 11,
        };
        assert!(worker_message_requires_abort_repeat(
            &WorkerMessage::Checkpoint(WorkerCheckpoint {
                completed_stage: LifecycleStage::InitialState,
                evidence_root_identity: root,
            })
        ));
        assert!(worker_message_requires_abort_repeat(
            &WorkerMessage::RunDesktopPhase(DesktopPhase::FinalPrimary)
        ));
        assert!(!worker_message_requires_abort_repeat(
            &WorkerMessage::ResultReady(Box::new(success_result()))
        ));

        let result = failed_result_at(
            LifecycleStage::FinalCrashRecovery,
            WorkerFailure {
                kind: WorkerFailureKind::Controller,
                attempted_stage: Some(LifecycleStage::FinalFallbackStates),
                reason: "final-missing-service-desktop.private.json:desktop_failed".to_string(),
                evidence: None,
                evidence_error: None,
                restoration: Box::new(RestorationOutcome::BlockedUnsettled),
            },
        );
        assert_eq!(
            crossed_worker_result(&result),
            Some(CrossedWorkerRequest {
                completed_stage: LifecycleStage::FinalCrashRecovery,
                attempted_stage: Some(LifecycleStage::FinalFallbackStates),
            })
        );
    }

    #[test]
    fn queued_abort_and_result_ready_require_an_abort_bound_successor() {
        let root = crate::windows_lifecycle_proof_contract::EvidenceRootIdentity {
            volume_serial: 1,
            file_index: 1,
        };
        let original = failed_result(WorkerFailure {
            kind: WorkerFailureKind::Mutation,
            attempted_stage: Some(LifecycleStage::FinalRepair),
            reason: "mutation_failed".to_string(),
            evidence: Some(Box::new(evidence_receipt(
                "final-repair-failure.private.json",
            ))),
            evidence_error: None,
            restoration: restoration_not_reviewed(),
        });
        let expected_abort = WorkerAbort {
            reason: AbortReason::ReceiptValidation,
            last_authenticated_checkpoint: original.last_authenticated_checkpoint,
            evidence_root_identity: root,
        };
        let mut successor = original.clone();
        successor.abort = Some(expected_abort);
        let mut tracker = AbortResultTracker::new(None);
        let mut parent = ProtocolGates::new();
        let mut worker = ProtocolGates::new();

        let queued_abort = parent.outbound.next().expect("queued abort");
        let original_result = worker.outbound.next().expect("original result");
        parent
            .inbound
            .accept(original_result)
            .expect("parent accepts original result");
        assert_eq!(
            tracker
                .observe(
                    original.clone(),
                    AbortReason::ReceiptValidation,
                    root,
                    original.last_authenticated_checkpoint,
                )
                .expect("crossed original"),
            AbortResultAction::RepeatAbort
        );
        worker
            .inbound
            .accept(queued_abort)
            .expect("worker accepts queued abort");
        let repeated_abort = parent.outbound.next().expect("repeated abort");
        let successor_result = worker.outbound.next().expect("abort successor");
        worker
            .inbound
            .accept(repeated_abort)
            .expect("worker accepts repeated abort");
        assert_eq!(
            abort_acceptance_action(
                &ParentMessage::Abort(AbortReason::ReceiptValidation),
                AbortReason::ReceiptValidation,
            ),
            Ok(AbortAcceptanceAction::RepeatResult)
        );
        parent
            .inbound
            .accept(successor_result)
            .expect("parent accepts abort successor");
        assert_eq!(
            tracker
                .observe(
                    successor.clone(),
                    AbortReason::ReceiptValidation,
                    root,
                    original.last_authenticated_checkpoint,
                )
                .expect("abort successor"),
            AbortResultAction::Complete(Box::new(successor))
        );
        assert_eq!(
            abort_acceptance_action(
                &ParentMessage::EvidenceAccepted,
                AbortReason::ReceiptValidation,
            ),
            Ok(AbortAcceptanceAction::Complete)
        );
        assert_eq!(
            abort_acceptance_action(
                &ParentMessage::Abort(AbortReason::Timeout),
                AbortReason::ReceiptValidation,
            ),
            Err("lifecycle_parent_abort_acceptance_required".to_string())
        );
    }

    #[test]
    fn abort_result_tracker_rejects_original_replay_and_hostile_successor_drift() {
        let root = crate::windows_lifecycle_proof_contract::EvidenceRootIdentity {
            volume_serial: 1,
            file_index: 1,
        };
        let original = failed_result(WorkerFailure {
            kind: WorkerFailureKind::Mutation,
            attempted_stage: Some(LifecycleStage::FinalRepair),
            reason: "mutation_failed".to_string(),
            evidence: Some(Box::new(evidence_receipt(
                "final-repair-failure.private.json",
            ))),
            evidence_error: None,
            restoration: restoration_not_reviewed(),
        });
        let mut replay = AbortResultTracker::new(None);
        assert_eq!(
            replay
                .observe(
                    original.clone(),
                    AbortReason::ReceiptValidation,
                    root,
                    original.last_authenticated_checkpoint,
                )
                .expect("initial result"),
            AbortResultAction::RepeatAbort
        );
        assert_eq!(
            replay.observe(
                original.clone(),
                AbortReason::ReceiptValidation,
                root,
                original.last_authenticated_checkpoint,
            ),
            Err("lifecycle_parent_abort_result_replayed".to_string())
        );

        let mut mismatch = AbortResultTracker::new(Some(original.clone()));
        let mut successor = original.clone();
        successor.abort = Some(WorkerAbort {
            reason: AbortReason::ReceiptValidation,
            last_authenticated_checkpoint: original.last_authenticated_checkpoint,
            evidence_root_identity: root,
        });
        successor.failure.as_mut().expect("failure").reason =
            "hostile_reason_replacement".to_string();
        assert_eq!(
            mismatch.observe(
                successor,
                AbortReason::ReceiptValidation,
                root,
                original.last_authenticated_checkpoint,
            ),
            Err("lifecycle_parent_abort_successor_mismatch".to_string())
        );
    }

    #[test]
    fn abort_result_tracker_preserves_pre_initial_failure_and_binds_parent_context() {
        let root = crate::windows_lifecycle_proof_contract::EvidenceRootIdentity {
            volume_serial: 1,
            file_index: 1,
        };
        let original = WorkerResult {
            disposition: WorkerDisposition::Failed,
            completed_stage: None,
            last_authenticated_checkpoint: None,
            abort: None,
            failure: Some(WorkerFailure {
                kind: WorkerFailureKind::Controller,
                attempted_stage: None,
                reason: "lifecycle_controller_not_ready".to_string(),
                evidence: None,
                evidence_error: None,
                restoration: Box::new(RestorationOutcome::NotRequired),
            }),
            process_tree_settled: true,
            private_evidence: Vec::new(),
        };
        assert!(validate_worker_result(&original, false).is_ok());

        let mut tracker = AbortResultTracker::new(None);
        assert_eq!(
            tracker
                .observe(original.clone(), AbortReason::ProtocolViolation, root, None,)
                .expect("crossed pre-initial failure"),
            AbortResultAction::RepeatAbort
        );
        let mut successor = original.clone();
        successor.abort = Some(WorkerAbort {
            reason: AbortReason::ProtocolViolation,
            last_authenticated_checkpoint: None,
            evidence_root_identity: root,
        });
        assert_eq!(
            tracker
                .observe(
                    successor.clone(),
                    AbortReason::ProtocolViolation,
                    root,
                    None,
                )
                .expect("bound pre-initial successor"),
            AbortResultAction::Complete(Box::new(successor))
        );

        let forged_parent_checkpoint = WorkerCheckpoint {
            completed_stage: LifecycleStage::InitialState,
            evidence_root_identity: root,
        };
        let mut context_mismatch = AbortResultTracker::new(None);
        assert_eq!(
            context_mismatch.observe(
                original,
                AbortReason::ProtocolViolation,
                root,
                Some(forged_parent_checkpoint),
            ),
            Err("lifecycle_parent_abort_context_invalid".to_string())
        );
        assert!(context_mismatch.original.is_none());
    }

    #[test]
    fn settled_worker_failures_remain_authoritative_after_parent_followup_errors() {
        let root = crate::windows_lifecycle_proof_contract::EvidenceRootIdentity {
            volume_serial: 1,
            file_index: 1,
        };
        let failures = [
            WorkerFailure {
                kind: WorkerFailureKind::Mutation,
                attempted_stage: Some(LifecycleStage::FinalRepair),
                reason: "mutation_failed".to_string(),
                evidence: Some(Box::new(evidence_receipt(
                    "final-repair-failure.private.json",
                ))),
                evidence_error: None,
                restoration: restoration_not_reviewed(),
            },
            WorkerFailure {
                kind: WorkerFailureKind::EvidenceWrite,
                attempted_stage: None,
                reason: "evidence_incomplete".to_string(),
                evidence: None,
                evidence_error: Some("write_failed".to_string()),
                restoration: Box::new(RestorationOutcome::NotRequired),
            },
            WorkerFailure {
                kind: WorkerFailureKind::Controller,
                attempted_stage: None,
                reason: "controller_failed".to_string(),
                evidence: None,
                evidence_error: None,
                restoration: Box::new(RestorationOutcome::NotRequired),
            },
        ];

        for failure in failures {
            let original = failed_result(failure);
            let mut successor = original.clone();
            successor.abort = Some(WorkerAbort {
                reason: AbortReason::ArtifactValidation,
                last_authenticated_checkpoint: original.last_authenticated_checkpoint,
                evidence_root_identity: root,
            });
            let mut tracker = AbortResultTracker::new(Some(original.clone()));
            assert_eq!(
                tracker
                    .observe(
                        successor.clone(),
                        AbortReason::ArtifactValidation,
                        root,
                        original.last_authenticated_checkpoint,
                    )
                    .expect("preserved successor"),
                AbortResultAction::Complete(Box::new(successor.clone()))
            );
            assert_eq!(successor.failure, original.failure);
            assert!(successor.process_tree_settled);
        }
    }

    #[test]
    fn successful_original_requires_exact_parent_abort_restoration_truth() {
        let original = success_result();
        let valid = deterministic_parent_abort_result(
            LifecycleStage::FinalUninstall,
            original.last_authenticated_checkpoint,
            AbortReason::ReceiptValidation,
        );
        let mut tracker = AbortResultTracker::new(Some(original.clone()));
        assert_eq!(
            tracker
                .observe(
                    valid.clone(),
                    AbortReason::ReceiptValidation,
                    crate::windows_lifecycle_proof_contract::EvidenceRootIdentity {
                        volume_serial: 1,
                        file_index: 1,
                    },
                    original.last_authenticated_checkpoint,
                )
                .expect("deterministic success abort"),
            AbortResultAction::Complete(Box::new(valid))
        );

        let mut restoration_drift = deterministic_parent_abort_result(
            LifecycleStage::FinalUninstall,
            original.last_authenticated_checkpoint,
            AbortReason::ReceiptValidation,
        );
        *restoration_drift
            .failure
            .as_mut()
            .expect("failure")
            .restoration = RestorationOutcome::BlockedUntrusted {
            reason: "hostile_restoration_drift".to_string(),
        };
        assert!(validate_worker_result(&restoration_drift, false).is_ok());
        let mut tracker = AbortResultTracker::new(Some(original.clone()));
        assert_eq!(
            tracker.observe(
                restoration_drift,
                AbortReason::ReceiptValidation,
                crate::windows_lifecycle_proof_contract::EvidenceRootIdentity {
                    volume_serial: 1,
                    file_index: 1,
                },
                original.last_authenticated_checkpoint,
            ),
            Err("lifecycle_parent_abort_successor_mismatch".to_string())
        );
    }

    #[test]
    fn no_original_checkpoint_binds_exact_abort_stage_and_restoration() {
        let root = crate::windows_lifecycle_proof_contract::EvidenceRootIdentity {
            volume_serial: 1,
            file_index: 1,
        };
        let prior = WorkerCheckpoint {
            completed_stage: LifecycleStage::InitialState,
            evidence_root_identity: root,
        };
        let valid = deterministic_parent_abort_result(
            LifecycleStage::FinalRepair,
            Some(prior),
            AbortReason::ArtifactValidation,
        );
        let mut tracker = AbortResultTracker::new(None);
        tracker
            .note_crossed_stage(LifecycleStage::FinalRepair)
            .expect("crossed stage");
        assert_eq!(
            tracker
                .observe(
                    valid.clone(),
                    AbortReason::ArtifactValidation,
                    root,
                    Some(prior),
                )
                .expect("exact crossed stage"),
            AbortResultAction::Complete(Box::new(valid))
        );

        let stage_drift = deterministic_parent_abort_result(
            LifecycleStage::InitialUninstall,
            Some(prior),
            AbortReason::ArtifactValidation,
        );
        assert!(validate_worker_result(&stage_drift, false).is_ok());
        let mut tracker = AbortResultTracker::new(None);
        tracker
            .note_crossed_stage(LifecycleStage::FinalRepair)
            .expect("crossed stage");
        assert_eq!(
            tracker.observe(
                stage_drift,
                AbortReason::ArtifactValidation,
                root,
                Some(prior),
            ),
            Err("lifecycle_parent_abort_successor_mismatch".to_string())
        );

        let mut restoration_drift = deterministic_parent_abort_result(
            LifecycleStage::FinalRepair,
            Some(prior),
            AbortReason::ArtifactValidation,
        );
        *restoration_drift
            .failure
            .as_mut()
            .expect("failure")
            .restoration = RestorationOutcome::BlockedUntrusted {
            reason: "hostile_restoration_drift".to_string(),
        };
        assert!(validate_worker_result(&restoration_drift, false).is_ok());
        let mut tracker = AbortResultTracker::new(None);
        tracker
            .note_crossed_stage(LifecycleStage::FinalRepair)
            .expect("crossed stage");
        assert_eq!(
            tracker.observe(
                restoration_drift,
                AbortReason::ArtifactValidation,
                root,
                Some(prior),
            ),
            Err("lifecycle_parent_abort_successor_mismatch".to_string())
        );
    }

    #[test]
    fn fallback_parent_abort_binds_restoration_to_the_attempted_stage() {
        let root = crate::windows_lifecycle_proof_contract::EvidenceRootIdentity {
            volume_serial: 1,
            file_index: 1,
        };
        let checkpoint = WorkerCheckpoint {
            completed_stage: LifecycleStage::FinalCrashRecovery,
            evidence_root_identity: root,
        };

        for process_tree_settled in [true, false] {
            let mut result = deterministic_parent_abort_result(
                LifecycleStage::FinalCrashRecovery,
                Some(checkpoint),
                AbortReason::DesktopFailure,
            );
            let failure = result.failure.as_mut().expect("failure");
            failure.attempted_stage = Some(LifecycleStage::FinalFallbackStates);
            *failure.restoration = if process_tree_settled {
                RestorationOutcome::Restored {
                    evidence: evidence_receipt("final-fallback-states-restoration.private.json"),
                }
            } else {
                RestorationOutcome::BlockedUnsettled
            };
            result.process_tree_settled = process_tree_settled;

            assert!(validate_worker_result(&result, false).is_ok());
            let mut tracker = AbortResultTracker::new(None);
            tracker
                .note_crossed_request(crossed_desktop_request(DesktopPhase::FinalMissingService))
                .expect("crossed stage");
            assert_eq!(
                tracker
                    .observe(
                        result.clone(),
                        AbortReason::DesktopFailure,
                        root,
                        Some(checkpoint),
                    )
                    .expect("fallback parent abort"),
                AbortResultAction::Complete(Box::new(result))
            );
        }

        let omitted_attempt = deterministic_parent_abort_result(
            LifecycleStage::FinalCrashRecovery,
            Some(checkpoint),
            AbortReason::DesktopFailure,
        );
        assert!(validate_worker_result(&omitted_attempt, false).is_ok());
        let mut tracker = AbortResultTracker::new(None);
        tracker
            .note_crossed_request(crossed_desktop_request(DesktopPhase::FinalMissingService))
            .expect("crossed fallback desktop");
        assert_eq!(
            tracker.observe(
                omitted_attempt,
                AbortReason::DesktopFailure,
                root,
                Some(checkpoint),
            ),
            Err("lifecycle_parent_abort_successor_mismatch".to_string())
        );

        let mut legacy_restoration = deterministic_parent_abort_result(
            LifecycleStage::FinalCrashRecovery,
            Some(checkpoint),
            AbortReason::DesktopFailure,
        );
        let failure = legacy_restoration.failure.as_mut().expect("failure");
        failure.attempted_stage = Some(LifecycleStage::FinalFallbackStates);
        *failure.restoration = RestorationOutcome::Failed {
            reason: "lifecycle_parent_abort_restoration_not_reviewed".to_string(),
            evidence: Some(evidence_receipt(
                "final-fallback-states-restoration.private.json",
            )),
            evidence_error: None,
        };
        assert!(validate_worker_result(&legacy_restoration, false).is_ok());
        let mut tracker = AbortResultTracker::new(None);
        tracker
            .note_crossed_request(crossed_desktop_request(DesktopPhase::FinalMissingService))
            .expect("crossed fallback desktop");
        assert_eq!(
            tracker.observe(
                legacy_restoration,
                AbortReason::DesktopFailure,
                root,
                Some(checkpoint),
            ),
            Err("lifecycle_parent_abort_successor_mismatch".to_string())
        );

        let mut wrong_pair = deterministic_parent_abort_result(
            LifecycleStage::FinalRepair,
            None,
            AbortReason::DesktopFailure,
        );
        let failure = wrong_pair.failure.as_mut().expect("failure");
        failure.attempted_stage = Some(LifecycleStage::FinalFallbackStates);
        *failure.restoration = RestorationOutcome::Failed {
            reason: "lifecycle_parent_abort_restoration_not_reviewed".to_string(),
            evidence: Some(evidence_receipt(
                "final-fallback-states-restoration.private.json",
            )),
            evidence_error: None,
        };
        assert_eq!(
            validate_worker_result(&wrong_pair, false),
            Err("lifecycle_worker_failure_shape_invalid".to_string())
        );
    }

    #[test]
    fn parent_abort_result_reports_receipt_and_restoration_failures() {
        let root = crate::windows_lifecycle_proof_contract::EvidenceRootIdentity {
            volume_serial: 7,
            file_index: 11,
        };
        let checkpoint = WorkerCheckpoint {
            completed_stage: LifecycleStage::FinalRepair,
            evidence_root_identity: root,
        };
        let mut result = failed_result_at(
            LifecycleStage::FinalRepair,
            WorkerFailure {
                kind: WorkerFailureKind::ParentAbort,
                attempted_stage: None,
                reason: parent_abort_reason(AbortReason::ReceiptValidation).to_string(),
                evidence: Some(Box::new(evidence_receipt(
                    "final-repair-parent-abort.private.json",
                ))),
                evidence_error: None,
                restoration: Box::new(RestorationOutcome::Failed {
                    reason: "lifecycle_parent_abort_restoration_not_reviewed".to_string(),
                    evidence: Some(evidence_receipt("final-repair-restoration.private.json")),
                    evidence_error: None,
                }),
            },
        );
        result.last_authenticated_checkpoint = Some(checkpoint);
        result.abort = Some(WorkerAbort {
            reason: AbortReason::ReceiptValidation,
            last_authenticated_checkpoint: Some(checkpoint),
            evidence_root_identity: root,
        });
        assert!(validate_worker_result(&result, false).is_ok());

        result.last_authenticated_checkpoint = Some(WorkerCheckpoint {
            completed_stage: LifecycleStage::InitialState,
            evidence_root_identity: root,
        });
        assert_eq!(
            validate_worker_result(&result, false),
            Err("lifecycle_worker_abort_state_invalid".to_string())
        );
        result.last_authenticated_checkpoint = Some(checkpoint);

        let failure = result.failure.as_mut().expect("failure");
        failure.evidence = None;
        failure.evidence_error = Some("lifecycle_evidence_create_failed".to_string());
        assert!(validate_worker_result(&result, false).is_ok());

        let failure = result.failure.as_mut().expect("failure");
        *failure.restoration = RestorationOutcome::Failed {
            reason: "lifecycle_parent_abort_restoration_not_reviewed".to_string(),
            evidence: None,
            evidence_error: Some("lifecycle_evidence_create_failed".to_string()),
        };
        assert!(validate_worker_result(&result, false).is_ok());
    }

    #[test]
    fn forced_termination_never_claims_restoration() {
        assert!(matches!(
            unverified_abort_restoration(UnverifiedAbortSettlement::ForcedSettled),
            RestorationOutcome::BlockedUntrusted { .. }
        ));
        assert_eq!(
            unverified_abort_restoration(UnverifiedAbortSettlement::ForcedUnsettled),
            RestorationOutcome::BlockedUnsettled
        );
    }

    #[test]
    fn worker_result_requires_matching_disposition_and_settlement() {
        let passed = success_result();
        assert!(passed.abort.is_none());
        assert!(validate_worker_result(&passed, true).is_ok());

        let mut forged = passed;
        forged.disposition = WorkerDisposition::Failed;
        assert_eq!(
            validate_worker_result(&forged, true),
            Err("lifecycle_worker_disposition_invalid".to_string())
        );
    }

    #[test]
    fn worker_success_requires_exact_ordered_private_receipts() {
        let passed = success_result();
        assert!(validate_worker_result(&passed, true).is_ok());

        let mut skipped_checkpoint = passed.clone();
        skipped_checkpoint.last_authenticated_checkpoint = Some(WorkerCheckpoint {
            completed_stage: LifecycleStage::FinalFallbackStates,
            evidence_root_identity: crate::windows_lifecycle_proof_contract::EvidenceRootIdentity {
                volume_serial: 1,
                file_index: 1,
            },
        });
        assert_eq!(
            validate_worker_result(&skipped_checkpoint, true),
            Err("lifecycle_worker_result_invalid".to_string())
        );

        let mut incomplete = passed.clone();
        incomplete.private_evidence.pop();
        assert_eq!(
            validate_worker_result(&incomplete, true),
            Err("lifecycle_worker_result_invalid".to_string())
        );

        let mut duplicate = passed.clone();
        duplicate
            .private_evidence
            .push(duplicate.private_evidence[0].clone());
        assert_eq!(
            validate_worker_result(&duplicate, true),
            Err("lifecycle_worker_result_invalid".to_string())
        );

        let mut injected = passed.clone();
        injected.private_evidence[0].name = "../initial-state.private.json".to_string();
        assert_eq!(
            validate_worker_result(&injected, true),
            Err("lifecycle_worker_result_invalid".to_string())
        );

        let mut reordered = passed;
        reordered.private_evidence.swap(0, 1);
        assert_eq!(
            validate_worker_result(&reordered, true),
            Err("lifecycle_worker_result_invalid".to_string())
        );
    }

    #[test]
    fn worker_failure_kinds_require_their_exact_evidence_shape() {
        let mutation = failed_result(WorkerFailure {
            kind: WorkerFailureKind::Mutation,
            attempted_stage: Some(LifecycleStage::FinalRepair),
            reason: "mutation_failed".to_string(),
            evidence: Some(Box::new(evidence_receipt(
                "final-repair-failure.private.json",
            ))),
            evidence_error: None,
            restoration: restoration_not_reviewed(),
        });
        assert!(validate_worker_result(&mutation, false).is_ok());

        let evidence_write = failed_result(WorkerFailure {
            kind: WorkerFailureKind::EvidenceWrite,
            attempted_stage: None,
            reason: "evidence_incomplete".to_string(),
            evidence: None,
            evidence_error: Some("write_failed".to_string()),
            restoration: Box::new(RestorationOutcome::NotRequired),
        });
        assert!(validate_worker_result(&evidence_write, false).is_ok());

        let mut settlement = failed_result(WorkerFailure {
            kind: WorkerFailureKind::ProcessSettlement,
            attempted_stage: Some(LifecycleStage::FinalRepair),
            reason: "settlement_unproven".to_string(),
            evidence: Some(Box::new(evidence_receipt(
                "final-repair-failure.private.json",
            ))),
            evidence_error: None,
            restoration: Box::new(RestorationOutcome::BlockedUnsettled),
        });
        settlement.process_tree_settled = false;
        assert!(validate_worker_result(&settlement, false).is_ok());
        settlement.abort = Some(WorkerAbort {
            reason: AbortReason::Timeout,
            last_authenticated_checkpoint: settlement.last_authenticated_checkpoint,
            evidence_root_identity: crate::windows_lifecycle_proof_contract::EvidenceRootIdentity {
                volume_serial: 1,
                file_index: 1,
            },
        });
        assert!(validate_worker_result(&settlement, false).is_ok());

        let controller = failed_result(WorkerFailure {
            kind: WorkerFailureKind::Controller,
            attempted_stage: None,
            reason: "controller_failed".to_string(),
            evidence: None,
            evidence_error: None,
            restoration: Box::new(RestorationOutcome::NotRequired),
        });
        assert!(validate_worker_result(&controller, false).is_ok());

        let fallback_controller = failed_result_at(
            LifecycleStage::FinalCrashRecovery,
            WorkerFailure {
                kind: WorkerFailureKind::Controller,
                attempted_stage: Some(LifecycleStage::FinalFallbackStates),
                reason: "final-missing-service-desktop.private.json:desktop_failed".to_string(),
                evidence: None,
                evidence_error: None,
                restoration: Box::new(RestorationOutcome::Restored {
                    evidence: evidence_receipt("final-fallback-states-restoration.private.json"),
                }),
            },
        );
        assert!(validate_worker_result(&fallback_controller, false).is_ok());

        let mut unsettled_fallback_controller = failed_result_at(
            LifecycleStage::FinalCrashRecovery,
            WorkerFailure {
                kind: WorkerFailureKind::Controller,
                attempted_stage: Some(LifecycleStage::FinalFallbackStates),
                reason: "final-missing-service-desktop.private.json:desktop_unsettled".to_string(),
                evidence: None,
                evidence_error: None,
                restoration: Box::new(RestorationOutcome::BlockedUnsettled),
            },
        );
        unsettled_fallback_controller.process_tree_settled = false;
        assert!(validate_worker_result(&unsettled_fallback_controller, false).is_ok());

        let restored = failed_result(WorkerFailure {
            kind: WorkerFailureKind::Mutation,
            attempted_stage: Some(LifecycleStage::FinalRepair),
            reason: "mutation_failed".to_string(),
            evidence: Some(Box::new(evidence_receipt(
                "final-repair-failure.private.json",
            ))),
            evidence_error: None,
            restoration: Box::new(RestorationOutcome::Restored {
                evidence: evidence_receipt("final-repair-restoration.private.json"),
            }),
        });
        assert!(validate_worker_result(&restored, false).is_ok());
    }

    #[test]
    fn worker_failure_rejects_forged_settlement_and_receipt_combinations() {
        let forged_mutation = failed_result(WorkerFailure {
            kind: WorkerFailureKind::Mutation,
            attempted_stage: Some(LifecycleStage::FinalRepair),
            reason: "mutation_failed".to_string(),
            evidence: None,
            evidence_error: None,
            restoration: restoration_not_reviewed(),
        });
        assert_eq!(
            validate_worker_result(&forged_mutation, false),
            Err("lifecycle_worker_failure_shape_invalid".to_string())
        );

        let mut forged_settlement = failed_result(WorkerFailure {
            kind: WorkerFailureKind::ProcessSettlement,
            attempted_stage: Some(LifecycleStage::FinalRepair),
            reason: "settlement_unproven".to_string(),
            evidence: None,
            evidence_error: Some("write_failed".to_string()),
            restoration: Box::new(RestorationOutcome::BlockedUnsettled),
        });
        forged_settlement.process_tree_settled = true;
        assert_eq!(
            validate_worker_result(&forged_settlement, false),
            Err("lifecycle_worker_settlement_restoration_invalid".to_string())
        );

        let mut forged_unsettled_controller = failed_result_at(
            LifecycleStage::FinalCrashRecovery,
            WorkerFailure {
                kind: WorkerFailureKind::Controller,
                attempted_stage: Some(LifecycleStage::FinalFallbackStates),
                reason: "desktop_unsettled".to_string(),
                evidence: None,
                evidence_error: None,
                restoration: Box::new(RestorationOutcome::NotRequired),
            },
        );
        forged_unsettled_controller.process_tree_settled = false;
        assert_eq!(
            validate_worker_result(&forged_unsettled_controller, false),
            Err("lifecycle_worker_settlement_restoration_invalid".to_string())
        );

        let wrong_fallback_pair = failed_result_at(
            LifecycleStage::FinalRepair,
            WorkerFailure {
                kind: WorkerFailureKind::Controller,
                attempted_stage: Some(LifecycleStage::FinalFallbackStates),
                reason: "wrong_fallback_pair".to_string(),
                evidence: None,
                evidence_error: None,
                restoration: Box::new(RestorationOutcome::Restored {
                    evidence: evidence_receipt("final-fallback-states-restoration.private.json"),
                }),
            },
        );
        assert_eq!(
            validate_worker_result(&wrong_fallback_pair, false),
            Err("lifecycle_worker_failure_shape_invalid".to_string())
        );

        let forged_leaf = failed_result(WorkerFailure {
            kind: WorkerFailureKind::Mutation,
            attempted_stage: Some(LifecycleStage::FinalRepair),
            reason: "mutation_failed".to_string(),
            evidence: Some(Box::new(evidence_receipt(
                "baseline-install-failure.private.json",
            ))),
            evidence_error: None,
            restoration: restoration_not_reviewed(),
        });
        assert_eq!(
            validate_worker_result(&forged_leaf, false),
            Err("lifecycle_worker_failure_shape_invalid".to_string())
        );

        for restoration in [
            RestorationOutcome::NotRequired,
            RestorationOutcome::BlockedUnsettled,
            RestorationOutcome::Restored {
                evidence: evidence_receipt("final-repair-failure.private.json"),
            },
            RestorationOutcome::Restored {
                evidence: evidence_receipt("baseline-install-restoration.private.json"),
            },
        ] {
            let invalid_restoration = failed_result(WorkerFailure {
                kind: WorkerFailureKind::Mutation,
                attempted_stage: Some(LifecycleStage::FinalRepair),
                reason: "mutation_failed".to_string(),
                evidence: Some(Box::new(evidence_receipt(
                    "final-repair-failure.private.json",
                ))),
                evidence_error: None,
                restoration: Box::new(restoration),
            });
            assert!(validate_worker_result(&invalid_restoration, false).is_err());
        }

        for kind in [
            WorkerFailureKind::EvidenceWrite,
            WorkerFailureKind::Controller,
        ] {
            let after_mutation = failed_result_at(
                LifecycleStage::FinalRepair,
                WorkerFailure {
                    kind,
                    attempted_stage: None,
                    reason: "post_mutation_failed".to_string(),
                    evidence: None,
                    evidence_error: (kind == WorkerFailureKind::EvidenceWrite)
                        .then(|| "write_failed".to_string()),
                    restoration: Box::new(RestorationOutcome::NotRequired),
                },
            );
            assert!(validate_worker_result(&after_mutation, false).is_err());

            let valid_after_mutation = failed_result_at(
                LifecycleStage::FinalRepair,
                WorkerFailure {
                    kind,
                    attempted_stage: None,
                    reason: "post_mutation_failed".to_string(),
                    evidence: None,
                    evidence_error: (kind == WorkerFailureKind::EvidenceWrite)
                        .then(|| "write_failed".to_string()),
                    restoration: restoration_not_reviewed(),
                },
            );
            assert!(validate_worker_result(&valid_after_mutation, false).is_ok());
        }

        let mut empty_restoration_reason = failed_result(WorkerFailure {
            kind: WorkerFailureKind::Mutation,
            attempted_stage: Some(LifecycleStage::FinalRepair),
            reason: "mutation_failed".to_string(),
            evidence: Some(Box::new(evidence_receipt(
                "final-repair-failure.private.json",
            ))),
            evidence_error: None,
            restoration: Box::new(RestorationOutcome::BlockedUntrusted {
                reason: String::new(),
            }),
        });
        assert_eq!(
            validate_worker_result(&empty_restoration_reason, false),
            Err("lifecycle_worker_restoration_blocked_untrusted_reason_invalid".to_string())
        );

        *empty_restoration_reason
            .failure
            .as_mut()
            .expect("failure")
            .restoration = RestorationOutcome::Failed {
            reason: "restoration_failed".to_string(),
            evidence: Some(evidence_receipt("restoration.private.json")),
            evidence_error: Some("write_failed".to_string()),
        };
        assert_eq!(
            validate_worker_result(&empty_restoration_reason, false),
            Err("lifecycle_worker_restoration_failure_shape_invalid".to_string())
        );
    }

    #[test]
    fn lifecycle_mutation_failures_bind_their_exact_stage_and_leaf() {
        for (attempted, completed, leaf) in [
            (
                LifecycleStage::FinalRepair,
                LifecycleStage::InitialState,
                "final-repair-failure.private.json",
            ),
            (
                LifecycleStage::InitialUninstall,
                LifecycleStage::FinalRepair,
                "initial-uninstall-failure.private.json",
            ),
            (
                LifecycleStage::BaselineInstall,
                LifecycleStage::InitialUninstall,
                "baseline-install-failure.private.json",
            ),
            (
                LifecycleStage::BaselineRestart,
                LifecycleStage::BaselineInstall,
                "baseline-restart-failure.private.json",
            ),
            (
                LifecycleStage::BaselineCrashRecovery,
                LifecycleStage::BaselineRestart,
                "baseline-crash-recovery-failure.private.json",
            ),
            (
                LifecycleStage::BaselineRollbackRecovery,
                LifecycleStage::BaselineCrashRecovery,
                "baseline-rollback-recovery-failure.private.json",
            ),
            (
                LifecycleStage::FinalUpgrade,
                LifecycleStage::LegacyResidueSeeded,
                "final-upgrade-failure.private.json",
            ),
            (
                LifecycleStage::FinalRestart,
                LifecycleStage::FinalUpgrade,
                "final-restart-failure.private.json",
            ),
            (
                LifecycleStage::FinalCrashRecovery,
                LifecycleStage::FinalRestart,
                "final-crash-recovery-failure.private.json",
            ),
            (
                LifecycleStage::FinalFallbackStates,
                LifecycleStage::FinalCrashRecovery,
                "final-fallback-states-failure.private.json",
            ),
            (
                LifecycleStage::FinalUninstall,
                LifecycleStage::FinalFallbackStates,
                "final-uninstall-failure.private.json",
            ),
        ] {
            let result = failed_result_at(
                completed,
                WorkerFailure {
                    kind: WorkerFailureKind::Mutation,
                    attempted_stage: Some(attempted),
                    reason: "mutation_failed".to_string(),
                    evidence: Some(Box::new(evidence_receipt(leaf))),
                    evidence_error: None,
                    restoration: restoration_not_reviewed(),
                },
            );
            assert!(validate_worker_result(&result, false).is_ok(), "{leaf}");
        }
    }

    #[test]
    fn ordinary_failure_progress_allows_only_the_next_stage_evidence_write() {
        let initial_checkpoint = WorkerCheckpoint {
            completed_stage: LifecycleStage::InitialState,
            evidence_root_identity: crate::windows_lifecycle_proof_contract::EvidenceRootIdentity {
                volume_serial: 1,
                file_index: 1,
            },
        };
        let mut evidence_write = failed_result_at(
            LifecycleStage::FinalRepair,
            WorkerFailure {
                kind: WorkerFailureKind::EvidenceWrite,
                attempted_stage: None,
                reason: "lifecycle_final_repair_evidence_incomplete".to_string(),
                evidence: None,
                evidence_error: Some("lifecycle_evidence_create_failed".to_string()),
                restoration: restoration_not_reviewed(),
            },
        );
        evidence_write.last_authenticated_checkpoint = Some(initial_checkpoint);
        assert!(validate_worker_result(&evidence_write, false).is_ok());
        assert!(validate_parent_failure_progress(&evidence_write).is_ok());

        let mut forged_controller = failed_result_at(
            LifecycleStage::FinalRepair,
            WorkerFailure {
                kind: WorkerFailureKind::Controller,
                attempted_stage: None,
                reason: "lifecycle_controller_forward_drift".to_string(),
                evidence: None,
                evidence_error: None,
                restoration: restoration_not_reviewed(),
            },
        );
        forged_controller.last_authenticated_checkpoint = Some(initial_checkpoint);
        assert!(validate_worker_result(&forged_controller, false).is_ok());
        assert_eq!(
            validate_parent_failure_progress(&forged_controller),
            Err("lifecycle_worker_failure_progress_invalid".to_string())
        );

        let final_repair_checkpoint = WorkerCheckpoint {
            completed_stage: LifecycleStage::FinalRepair,
            evidence_root_identity: initial_checkpoint.evidence_root_identity,
        };
        let mut post_desktop_evidence_write = failed_result_at(
            LifecycleStage::InitialUninstall,
            WorkerFailure {
                kind: WorkerFailureKind::EvidenceWrite,
                attempted_stage: None,
                reason: "lifecycle_initial_uninstall_evidence_incomplete".to_string(),
                evidence: None,
                evidence_error: Some("lifecycle_evidence_create_failed".to_string()),
                restoration: restoration_not_reviewed(),
            },
        );
        post_desktop_evidence_write.last_authenticated_checkpoint = Some(final_repair_checkpoint);
        assert!(validate_worker_result(&post_desktop_evidence_write, false).is_ok());
        assert_eq!(
            validate_parent_original_worker_result(
                &post_desktop_evidence_write,
                Some(final_repair_checkpoint),
                0,
            ),
            Err("lifecycle_worker_failure_desktop_phases_incomplete".to_string())
        );
        assert!(validate_parent_original_worker_result(
            &post_desktop_evidence_write,
            Some(final_repair_checkpoint),
            1,
        )
        .is_ok());
    }

    #[test]
    fn attempted_stage_failures_require_all_prior_desktop_phases() {
        for (completed, attempted, leaf, required_index) in [
            (
                LifecycleStage::FinalRepair,
                LifecycleStage::InitialUninstall,
                "initial-uninstall-failure.private.json",
                1,
            ),
            (
                LifecycleStage::BaselineInstall,
                LifecycleStage::BaselineRestart,
                "baseline-restart-failure.private.json",
                3,
            ),
            (
                LifecycleStage::FinalCrashRecovery,
                LifecycleStage::FinalFallbackStates,
                "final-fallback-states-failure.private.json",
                3,
            ),
            (
                LifecycleStage::FinalFallbackStates,
                LifecycleStage::FinalUninstall,
                "final-uninstall-failure.private.json",
                DESKTOP_PHASES.len(),
            ),
        ] {
            let result = failed_result_at(
                completed,
                WorkerFailure {
                    kind: WorkerFailureKind::Mutation,
                    attempted_stage: Some(attempted),
                    reason: "lifecycle_mutation_failed".to_string(),
                    evidence: Some(Box::new(evidence_receipt(leaf))),
                    evidence_error: None,
                    restoration: restoration_not_reviewed(),
                },
            );
            assert!(
                validate_worker_result(&result, false).is_ok(),
                "{attempted:?}"
            );
            assert_eq!(
                validate_parent_original_worker_result(
                    &result,
                    result.last_authenticated_checkpoint,
                    required_index - 1,
                ),
                Err("lifecycle_worker_failure_desktop_phases_incomplete".to_string()),
                "{attempted:?}"
            );
            assert!(
                validate_parent_original_worker_result(
                    &result,
                    result.last_authenticated_checkpoint,
                    required_index,
                )
                .is_ok(),
                "{attempted:?}"
            );
        }
    }

    fn failed_result(failure: WorkerFailure) -> WorkerResult {
        failed_result_at(LifecycleStage::InitialState, failure)
    }

    fn deterministic_parent_abort_result(
        completed_stage: LifecycleStage,
        last_authenticated_checkpoint: Option<WorkerCheckpoint>,
        reason: AbortReason,
    ) -> WorkerResult {
        let evidence_root_identity = last_authenticated_checkpoint
            .map(|checkpoint| checkpoint.evidence_root_identity)
            .unwrap_or(
                crate::windows_lifecycle_proof_contract::EvidenceRootIdentity {
                    volume_serial: 1,
                    file_index: 1,
                },
            );
        let restoration = if completed_stage == LifecycleStage::InitialState {
            RestorationOutcome::NotRequired
        } else {
            RestorationOutcome::Failed {
                reason: "lifecycle_parent_abort_restoration_not_reviewed".to_string(),
                evidence: Some(evidence_receipt(
                    restoration_leaf_for_stage(completed_stage).expect("restoration leaf"),
                )),
                evidence_error: None,
            }
        };
        WorkerResult {
            disposition: WorkerDisposition::Failed,
            completed_stage: Some(completed_stage),
            last_authenticated_checkpoint,
            abort: Some(WorkerAbort {
                reason,
                last_authenticated_checkpoint,
                evidence_root_identity,
            }),
            failure: Some(WorkerFailure {
                kind: WorkerFailureKind::ParentAbort,
                attempted_stage: None,
                reason: parent_abort_reason(reason).to_string(),
                evidence: Some(Box::new(evidence_receipt(parent_abort_leaf_for_stage(
                    completed_stage,
                )))),
                evidence_error: None,
                restoration: Box::new(restoration),
            }),
            process_tree_settled: true,
            private_evidence: Vec::new(),
        }
    }

    fn restoration_not_reviewed() -> Box<RestorationOutcome> {
        Box::new(RestorationOutcome::BlockedUntrusted {
            reason: "lifecycle_restoration_not_reviewed".to_string(),
        })
    }

    fn success_result() -> WorkerResult {
        WorkerResult {
            disposition: WorkerDisposition::Passed,
            completed_stage: Some(LifecycleStage::FinalUninstall),
            last_authenticated_checkpoint: Some(WorkerCheckpoint {
                completed_stage: LifecycleStage::FinalUninstall,
                evidence_root_identity:
                    crate::windows_lifecycle_proof_contract::EvidenceRootIdentity {
                        volume_serial: 1,
                        file_index: 1,
                    },
            }),
            abort: None,
            failure: None,
            process_tree_settled: true,
            private_evidence: SUCCESS_PRIVATE_EVIDENCE_LEAVES
                .iter()
                .map(|name| evidence_receipt(name))
                .collect(),
        }
    }

    fn failed_result_at(completed_stage: LifecycleStage, failure: WorkerFailure) -> WorkerResult {
        WorkerResult {
            disposition: WorkerDisposition::Failed,
            completed_stage: Some(completed_stage),
            last_authenticated_checkpoint: Some(WorkerCheckpoint {
                completed_stage,
                evidence_root_identity:
                    crate::windows_lifecycle_proof_contract::EvidenceRootIdentity {
                        volume_serial: 1,
                        file_index: 1,
                    },
            }),
            abort: None,
            failure: Some(failure),
            process_tree_settled: true,
            private_evidence: Vec::new(),
        }
    }

    fn parent_current_user_authority_for_test() -> native::ParentCurrentUserAuthority {
        native::ParentCurrentUserAuthority {
            user_sid: "S-1-5-21-1".to_string(),
            session_id: 1,
            logon_luid: native::LogonLuid {
                low_part: 1,
                high_part: 0,
            },
            profile: native::DirectorySnapshot {
                identity: native::FileIdentity {
                    volume_serial: 1,
                    file_index: 1,
                },
                final_path: r"C:\Users\proof".to_string(),
            },
            local_app_data: native::DirectorySnapshot {
                identity: native::FileIdentity {
                    volume_serial: 1,
                    file_index: 2,
                },
                final_path: r"C:\Users\proof\AppData\Local".to_string(),
            },
            resolved_data_root: r"C:\Users\proof\AppData\Local\BatCaveMonitor".to_string(),
            data_root: Observation::Present(native::DirectorySnapshot {
                identity: native::FileIdentity {
                    volume_serial: 1,
                    file_index: 3,
                },
                final_path: r"C:\Users\proof\AppData\Local\BatCaveMonitor".to_string(),
            }),
        }
    }

    fn parent_residue_snapshot_for_test(
        run_present: bool,
        known_helpers: bool,
        sentinel_present: bool,
    ) -> native::ParentCurrentUserResidueSnapshot {
        let authority = parent_current_user_authority_for_test();
        let known_files = if known_helpers {
            native::expected_helper_leaves()
                .into_iter()
                .enumerate()
                .map(|(index, relative_leaf)| {
                    let expected = native::expected_parent_helper_fixture_snapshot(&relative_leaf)
                        .expect("fixture");
                    native::ParentHelperFileSnapshot {
                        relative_leaf,
                        file: native::FileSnapshot {
                            size: expected.0,
                            sha256: expected.1,
                            identity: native::FileIdentity {
                                volume_serial: 1,
                                file_index: 100 + index as u64,
                            },
                        },
                        owner_sid: authority.user_sid.clone(),
                        dacl_sha256: "a".repeat(64),
                    }
                })
                .collect()
        } else {
            Vec::new()
        };
        let sentinel = if sentinel_present {
            let expected = native::expected_parent_helper_sentinel_snapshot();
            Observation::Present(native::ParentHelperFileSnapshot {
                relative_leaf: "elevated-helper/unknown-sentinel.bin".to_string(),
                file: native::FileSnapshot {
                    size: expected.0,
                    sha256: expected.1,
                    identity: native::FileIdentity {
                        volume_serial: 1,
                        file_index: 200,
                    },
                },
                owner_sid: authority.user_sid.clone(),
                dacl_sha256: "b".repeat(64),
            })
        } else {
            Observation::Absent
        };
        let helper = if known_helpers || sentinel_present {
            Observation::Present(native::ParentHelperManifestSnapshot {
                root: native::DirectorySnapshot {
                    identity: native::FileIdentity {
                        volume_serial: 1,
                        file_index: 50,
                    },
                    final_path: r"C:\Users\proof\AppData\Local\BatCaveMonitor\elevated-helper"
                        .to_string(),
                },
                root_owner_sid: authority.user_sid.clone(),
                root_dacl_sha256: "c".repeat(64),
                known_files,
                sentinel,
                unexpected_entry_count: 0,
                manifest_sha256: "d".repeat(64),
            })
        } else {
            Observation::Absent
        };
        native::ParentCurrentUserResidueSnapshot {
            hkcu_run: native::ParentRunKeySnapshot {
                final_key_path: format!(
                    r"\REGISTRY\USER\{}\Software\Microsoft\Windows\CurrentVersion\Run",
                    authority.user_sid
                ),
                owner_sid: authority.user_sid,
                dacl_sha256: "e".repeat(64),
                last_write_time_100ns: 1,
                value_count: u32::from(run_present),
                manifest_sha256: "f".repeat(64),
                batcave_monitor: if run_present {
                    Observation::Present(native::ParentRunValueSnapshot {
                        value_type: 1,
                        value: native::exact_parent_run_value().to_string(),
                    })
                } else {
                    Observation::Absent
                },
            },
            helper,
        }
    }

    fn evidence_receipt(name: &str) -> EvidenceReceipt {
        EvidenceReceipt {
            name: name.to_string(),
            size: 1,
            sha256: "a".repeat(64),
        }
    }
}
