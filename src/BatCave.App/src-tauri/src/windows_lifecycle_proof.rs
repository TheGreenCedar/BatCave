#[allow(dead_code)] // The fail-closed export contract is wired by the remaining lifecycle stages.
mod evidence;
mod lifecycle;
mod native;
mod private_evidence;

use crate::windows_lifecycle_proof_contract::{
    message_sha256, parse_plan, plan_sha256, validate_desktop_phase_result, validate_envelope,
    validate_locator, validate_nonce, validate_sha256, ClosedRequest, DesktopPhase,
    DesktopPhaseDisposition, DesktopPhaseResult, Envelope, EvidenceReceipt, LifecycleStage,
    ParentMessage, ProofPlan, RestorationOutcome, SequenceGate, WorkerFailureKind, WorkerMessage,
    WorkerResult, PROTOCOL_SCHEMA, SUCCESS_PRIVATE_EVIDENCE_LEAVES,
};
use native::{OwnedFile, PipeConnection, PreflightSnapshot};
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::time::Duration;

const SESSION_TIMEOUT: Duration = Duration::from_secs(45 * 60);
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
    snapshot: PreflightSnapshot,
    source_commit_sha: String,
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
    let snapshot = native::capture_parent_preflight(&plan, &[controller_binding])?;
    Ok(ParentPreflight {
        plan,
        repo_root,
        controller,
        baseline,
        final_candidate,
        incompatible_service_fixture,
        rollback_failing_service_fixture,
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

    let mut gate = SequenceGate::new();
    let request = ParentMessage::Begin(ClosedRequest {
        plan_sha256: plan_sha256(),
        controller_source_commit_sha: preflight.source_commit_sha.clone(),
        controller_sha256: preflight.controller.sha256_hex(),
        parent_process_id: std::process::id(),
        parent_started_at_100ns: native::current_process_started_at()?,
    });
    send_parent_message(&mut pipe, &nonce, &mut gate, request)?;

    let mut evidence_root = None;
    let mut desktop_phase_index = 0;
    let mut desktop_results = Vec::with_capacity(DESKTOP_PHASES.len());
    loop {
        let envelope: Envelope<WorkerMessage> = pipe.read_json(SESSION_TIMEOUT)?;
        validate_envelope(&envelope, &nonce, &mut gate)?;
        match envelope.message {
            WorkerMessage::Accepted(accepted) => {
                if evidence_root.is_some()
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
                    &mut gate,
                    ParentMessage::DesktopPhaseComplete(Box::new(result)),
                )?;
                desktop_phase_index += 1;
            }
            WorkerMessage::ResultReady(result) if result.failure.is_none() => {
                if evidence_root.is_none() || desktop_phase_index != DESKTOP_PHASES.len() {
                    return Err("lifecycle_worker_completion_order_invalid".to_string());
                }
                validate_worker_result(&result, true)?;
                let evidence_root_guard = evidence_root
                    .as_ref()
                    .ok_or_else(|| "lifecycle_evidence_root_missing".to_string())?;
                let mut private_evidence_guards = Vec::with_capacity(result.private_evidence.len());
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
                evidence::validate_sanitized_export_bytes_with_parent_results(
                    &sanitized_evidence_guard.read_all_exact("sanitized_export")?,
                    &preflight.plan,
                    &preflight.source_commit_sha,
                    &preflight.controller.sha256_hex(),
                    &result.private_evidence,
                    &desktop_results,
                )?;
                revalidate_preflight_artifacts(&preflight)?;
                send_parent_message(
                    &mut pipe,
                    &nonce,
                    &mut gate,
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
                    restoration_evidence_verified: None,
                    parent_followup_error: None,
                    private_evidence: Some(result.private_evidence.clone()),
                    sanitized_export: result.sanitized_export.clone(),
                    success_evidence_verified: Some(true),
                    profile: Some(preflight.plan.profile),
                    controller_source_commit_sha: Some(preflight.source_commit_sha),
                    evidence_root: evidence_root_value,
                    preflight: Some(preflight.snapshot),
                });
                return Ok(0);
            }
            WorkerMessage::ResultReady(result) => {
                if evidence_root.is_none() {
                    return Err("lifecycle_worker_failure_before_acceptance".to_string());
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
                            &mut gate,
                            ParentMessage::EvidenceAccepted,
                        )?;
                        let exit_code = worker.wait(Duration::from_secs(30))?;
                        if exit_code != 0 {
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
                let parent_followup_error = followup.err();
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
                    restoration_evidence_verified,
                    parent_followup_error,
                    private_evidence: None,
                    sanitized_export: None,
                    success_evidence_verified: None,
                    profile: Some(preflight.plan.profile),
                    controller_source_commit_sha: Some(preflight.source_commit_sha),
                    evidence_root: evidence_root_value,
                    preflight: Some(preflight.snapshot),
                });
                return Ok(1);
            }
        }
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

fn revalidate_preflight_artifacts(preflight: &ParentPreflight) -> Result<(), String> {
    preflight.controller.revalidate()?;
    preflight.baseline.revalidate()?;
    preflight.final_candidate.revalidate()?;
    preflight.incompatible_service_fixture.revalidate()?;
    preflight.rollback_failing_service_fixture.revalidate()
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

    let mut gate = SequenceGate::new();
    let begin: Envelope<ParentMessage> = pipe.read_json(Duration::from_secs(30))?;
    let nonce = begin.nonce.clone();
    validate_nonce(&nonce)?;
    validate_envelope(&begin, &nonce, &mut gate)?;
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
    send_worker_message(
        &mut pipe,
        &nonce,
        &mut gate,
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
        gate: &mut gate,
        controller_bindings: &[parent, worker_binding],
    });
    send_worker_message(
        &mut pipe,
        &nonce,
        &mut gate,
        WorkerMessage::ResultReady(Box::new(result)),
    )?;
    let accepted: Envelope<ParentMessage> = pipe.read_json(SESSION_TIMEOUT)?;
    validate_envelope(&accepted, &nonce, &mut gate)?;
    if accepted.message != ParentMessage::EvidenceAccepted {
        return Err("lifecycle_parent_evidence_acceptance_required".to_string());
    }
    Ok(0)
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
        if !result.process_tree_settled
            || result.completed_stage != Some(LifecycleStage::FinalUninstall)
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
        WorkerFailureKind::EvidenceWrite | WorkerFailureKind::Controller => {
            restoration_stage(result, failure)
                .is_some_and(|stage| stage != LifecycleStage::InitialState)
        }
    }
}

fn restoration_leaf_for_failure(
    result: &WorkerResult,
    failure: &crate::windows_lifecycle_proof_contract::WorkerFailure,
) -> Option<&'static str> {
    match restoration_stage(result, failure)? {
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
    fn worker_result_requires_matching_disposition_and_settlement() {
        let passed = success_result();
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

    fn restoration_not_reviewed() -> Box<RestorationOutcome> {
        Box::new(RestorationOutcome::BlockedUntrusted {
            reason: "lifecycle_restoration_not_reviewed".to_string(),
        })
    }

    fn success_result() -> WorkerResult {
        WorkerResult {
            disposition: WorkerDisposition::Passed,
            completed_stage: Some(LifecycleStage::FinalUninstall),
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
