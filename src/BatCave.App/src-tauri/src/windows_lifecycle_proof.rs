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
    ParentMessage, ProofPlan, RestorationOutcome, SequenceGate, WorkerAbort, WorkerCheckpoint,
    WorkerFailureKind, WorkerMessage, WorkerResult, PROTOCOL_SCHEMA,
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
    expected_parent_abort_stage: Option<LifecycleStage>,
}

impl AbortResultTracker {
    fn new(original: Option<WorkerResult>) -> Self {
        let awaiting_successor = original.is_some();
        let expected_parent_abort_stage =
            original.as_ref().and_then(|result| result.completed_stage);
        Self {
            original,
            awaiting_successor,
            expected_parent_abort_stage,
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
            if self.awaiting_successor {
                return Err("lifecycle_parent_abort_result_replayed".to_string());
            }
            self.expected_parent_abort_stage = result.completed_stage;
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
        if self
            .expected_parent_abort_stage
            .is_some_and(|expected| expected != stage)
        {
            return Err("lifecycle_parent_abort_crossed_stage_mismatch".to_string());
        }
        self.expected_parent_abort_stage = Some(stage);
        Ok(())
    }

    fn validate_deterministic_parent_abort(
        &self,
        result: &WorkerResult,
        reason: AbortReason,
    ) -> Result<(), String> {
        let expected_stage = self
            .expected_parent_abort_stage
            .ok_or_else(|| "lifecycle_parent_abort_crossed_stage_missing".to_string())?;
        let failure = result
            .failure
            .as_ref()
            .ok_or_else(|| "lifecycle_parent_abort_failure_missing".to_string())?;
        let restoration_valid = if expected_stage == LifecycleStage::InitialState {
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
                        restoration_leaf_for_stage(expected_stage)
                            == Some(receipt.name.as_str())
                    })
            )
        };
        if result.completed_stage != Some(expected_stage)
            || failure.kind != WorkerFailureKind::ParentAbort
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
    let mut current_user_retention = ParentCurrentUserRetentionState::default();
    let session = (|| -> Result<i32, String> {
        loop {
            let envelope: Envelope<WorkerMessage> = pipe.read_json(SESSION_TIMEOUT)?;
            validate_envelope(&envelope, &nonce, &mut gates.inbound)?;
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
                    if checkpoint.evidence_root_identity != root.identity()
                        || !valid_checkpoint_transition(last_checkpoint, checkpoint)
                    {
                        return Err("lifecycle_checkpoint_invalid".to_string());
                    }
                    current_user_retention.capture_checkpoint(
                        checkpoint.completed_stage,
                        &preflight.parent_current_user,
                    )?;
                    send_parent_message(
                        &mut pipe,
                        &nonce,
                        &mut gates.outbound,
                        ParentMessage::CheckpointAccepted(checkpoint),
                    )?;
                    last_checkpoint = Some(checkpoint);
                }
                WorkerMessage::RunDesktopPhase(phase) => {
                    if evidence_root.is_none() {
                        return Err("lifecycle_desktop_phase_before_acceptance".to_string());
                    }
                    if DESKTOP_PHASES.get(desktop_phase_index) != Some(&phase) {
                        return Err("lifecycle_desktop_phase_order_invalid".to_string());
                    }
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
                    validate_requested_desktop_phase_result(phase, &result, &preflight.plan)?;
                    desktop_results.push(result.clone());
                    send_parent_message(
                        &mut pipe,
                        &nonce,
                        &mut gates.outbound,
                        ParentMessage::DesktopPhaseComplete(Box::new(result)),
                    )?;
                    desktop_phase_index += 1;
                }
                WorkerMessage::ResultReady(result) if result.failure.is_none() => {
                    if evidence_root.is_none() || desktop_phase_index != DESKTOP_PHASES.len() {
                        return Err("lifecycle_worker_completion_order_invalid".to_string());
                    }
                    if result.last_authenticated_checkpoint != last_checkpoint {
                        return Err("lifecycle_worker_checkpoint_mismatch".to_string());
                    }
                    validate_worker_result(&result, true)?;
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
                    let sanitized_export = result
                        .sanitized_export
                        .as_ref()
                        .ok_or_else(|| "lifecycle_sanitized_export_missing".to_string())?;
                    let sanitized_evidence_guard =
                        native::verify_evidence_receipt(evidence_root_guard, sanitized_export)?;
                    let (before_uninstall, after_uninstall) =
                        current_user_retention.complete(&preflight.parent_current_user)?;
                    evidence::validate_verified_private_projection(
                        &private_evidence_guards,
                        &sanitized_evidence_guard,
                        &preflight.plan,
                        &preflight.source_commit_sha,
                        &preflight.controller.sha256_hex(),
                        &desktop_results,
                        evidence::ParentCurrentUserProjection {
                            authority: preflight.parent_current_user.authority(),
                            before_uninstall,
                            after_uninstall,
                        },
                    )?;
                    revalidate_preflight_artifacts(&preflight)?;
                    send_parent_message(
                        &mut pipe,
                        &nonce,
                        &mut gates.outbound,
                        ParentMessage::EvidenceAccepted,
                    )?;
                    let exit_code = worker.wait(Duration::from_secs(30))?;
                    revalidate_preflight_artifacts(&preflight)?;
                    if exit_code != 0 {
                        return Err("lifecycle_worker_exit_mismatch".to_string());
                    }
                    let evidence_root_value =
                        Some(evidence_root_guard.root().to_string_lossy().into_owned());
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
                        sanitized_export: result.sanitized_export.clone(),
                        success_evidence_verified: Some(true),
                        profile: Some(preflight.plan.profile.clone()),
                        controller_source_commit_sha: Some(preflight.source_commit_sha.clone()),
                        evidence_root: evidence_root_value,
                        preflight: Some(preflight.snapshot.clone()),
                    });
                    return Ok(0);
                }
                WorkerMessage::ResultReady(result) => {
                    if evidence_root.is_none() {
                        return Err("lifecycle_worker_failure_before_acceptance".to_string());
                    }
                    if result.last_authenticated_checkpoint != last_checkpoint {
                        return Err("lifecycle_worker_checkpoint_mismatch".to_string());
                    }
                    if result.abort.is_some() {
                        return Err("lifecycle_worker_unsolicited_abort".to_string());
                    }
                    validate_worker_result(&result, false)?;
                    let failure = result
                        .failure
                        .as_ref()
                        .ok_or_else(|| "lifecycle_worker_failure_missing".to_string())?;
                    let terminate_without_ack = matches!(
                        failure.restoration.as_ref(),
                        RestorationOutcome::BlockedUnsettled
                    );
                    let expected_exit_code = u32::from(result.abort.is_some());
                    let mut evidence_guard = None;
                    let mut restoration_evidence_guard = None;
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
                        Ok(())
                    })();
                    let failure_evidence_verified =
                        failure.evidence.as_ref().map(|_| evidence_guard.is_some());
                    let restoration_evidence_verified =
                        restoration_evidence(failure.restoration.as_ref())
                            .map(|_| restoration_evidence_guard.is_some());
                    let parent_followup_error = match followup {
                        Ok(()) => None,
                        Err(reason) if !terminate_without_ack => {
                            return abort_parent_session(
                                &mut pipe,
                                &nonce,
                                &mut gates,
                                &mut worker,
                                &preflight,
                                evidence_root.as_ref(),
                                last_checkpoint,
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
        Err(reason) => abort_parent_session(
            &mut pipe,
            &nonce,
            &mut gates,
            &mut worker,
            &preflight,
            evidence_root.as_ref(),
            last_checkpoint,
            None,
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

fn valid_checkpoint_transition(
    previous: Option<WorkerCheckpoint>,
    checkpoint: WorkerCheckpoint,
) -> bool {
    next_lifecycle_stage(previous.map(|checkpoint| checkpoint.completed_stage))
        == Some(checkpoint.completed_stage)
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
    evidence_root: Option<&native::ProtectedEvidenceRoot>,
    last_checkpoint: Option<WorkerCheckpoint>,
    original_result: Option<WorkerResult>,
    parent_error: String,
) -> Result<i32, String> {
    let reason = abort_reason_for_parent_error(&parent_error);
    let mut tracker = AbortResultTracker::new(original_result);
    let mut failure_evidence_guard = None;
    let mut restoration_evidence_guard = None;
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
                                checkpoint.evidence_root_identity != root.identity()
                                    || !valid_checkpoint_transition(last_checkpoint, *checkpoint)
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
                            tracker.note_crossed_stage(precursor)?;
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
            let unsettled_abort = failure.kind == WorkerFailureKind::ProcessSettlement
                && !result.process_tree_settled
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
                print_json(&ControllerOutcome {
                    disposition: if preserved_failure {
                        "worker_failure_followup_failed"
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
                    parent_followup_error: preserved_failure.then_some(parent_error.clone()),
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
    let restoration = match worker.wait_without_termination(Duration::from_secs(30)) {
        Ok(Some(_)) => unverified_abort_restoration(UnverifiedAbortSettlement::WorkerExited),
        Ok(None) => match worker.terminate_and_settle() {
            Ok(()) => unverified_abort_restoration(UnverifiedAbortSettlement::ForcedSettled),
            Err(error) => {
                followup_error = append_parent_followup_error(&followup_error, &error);
                unverified_abort_restoration(UnverifiedAbortSettlement::ForcedUnsettled)
            }
        },
        Err(error) => {
            followup_error = append_parent_followup_error(&followup_error, &error);
            match worker.terminate_and_settle() {
                Ok(()) => unverified_abort_restoration(UnverifiedAbortSettlement::ForcedSettled),
                Err(error) => {
                    followup_error = append_parent_followup_error(&followup_error, &error);
                    unverified_abort_restoration(UnverifiedAbortSettlement::ForcedUnsettled)
                }
            }
        }
    };
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
            || result.private_evidence.is_empty()
            || result.private_evidence.len() > 64
            || result.sanitized_export.is_none()
            || !valid_success_evidence_receipts(result)
        {
            return Err("lifecycle_worker_result_invalid".to_string());
        }
        return Ok(());
    }
    if !result.private_evidence.is_empty() || result.sanitized_export.is_some() {
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
    let failure_shape_valid = match failure.kind {
        WorkerFailureKind::Mutation => {
            result.process_tree_settled
                && failure.evidence.is_some()
                && failure.evidence_error.is_none()
                && mutation_failure_binding(result, failure).is_some_and(|expected_name| {
                    failure
                        .evidence
                        .as_ref()
                        .is_some_and(|receipt| receipt.name == expected_name)
                })
        }
        WorkerFailureKind::EvidenceWrite => {
            result.process_tree_settled
                && failure.evidence.is_none()
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
            result.process_tree_settled
                && failure.attempted_stage.is_none()
                && failure.evidence.is_none()
                && failure.evidence_error.is_none()
        }
        WorkerFailureKind::ParentAbort => {
            result.process_tree_settled
                && failure.attempted_stage.is_none()
                && (failure.evidence.is_some() != failure.evidence_error.is_some())
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
    match failure.kind {
        WorkerFailureKind::ProcessSettlement => {
            if !result.process_tree_settled
                && failure.restoration.as_ref() == &RestorationOutcome::BlockedUnsettled
            {
                Ok(())
            } else {
                Err("lifecycle_worker_restoration_disposition_invalid".to_string())
            }
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
    if result.private_evidence.iter().any(|receipt| {
        receipt.size == 0
            || receipt.size > 8 * 1024 * 1024
            || !receipt.name.ends_with(".private.json")
            || !valid_evidence_leaf(&receipt.name)
            || validate_sha256(&receipt.sha256, "private_evidence").is_err()
            || !names.insert(receipt.name.as_str())
    }) {
        return false;
    }
    if names
        != SUCCESS_PRIVATE_EVIDENCE_LEAVES
            .into_iter()
            .collect::<std::collections::BTreeSet<_>>()
    {
        return false;
    }
    result.sanitized_export.as_ref().is_some_and(|receipt| {
        receipt.name == "windows-lifecycle-proof.sanitized.json"
            && receipt.size > 0
            && receipt.size <= 8 * 1024 * 1024
            && validate_sha256(&receipt.sha256, "sanitized_export").is_ok()
            && names.insert(receipt.name.as_str())
    })
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
        _ => None,
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
        assert_eq!(
            next_lifecycle_stage(Some(LifecycleStage::FinalUninstall)),
            None
        );
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
    fn worker_success_requires_unique_fixed_private_and_sanitized_receipts() {
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

        let mut wrong_export = passed;
        wrong_export
            .sanitized_export
            .as_mut()
            .expect("sanitized")
            .name = "sanitized.json".to_string();
        assert_eq!(
            validate_worker_result(&wrong_export, true),
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
            sanitized_export: None,
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
            sanitized_export: Some(evidence_receipt("windows-lifecycle-proof.sanitized.json")),
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
            sanitized_export: None,
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
