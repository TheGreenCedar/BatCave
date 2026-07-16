mod lifecycle;
mod native;

use crate::windows_lifecycle_proof_contract::{
    message_sha256, parse_plan, plan_sha256, validate_envelope, validate_locator, validate_nonce,
    validate_sha256, ClosedRequest, DesktopPhaseDisposition, DesktopPhaseResult, Envelope,
    EvidenceReceipt, LifecycleStage, ParentMessage, ProofPlan, SequenceGate, WorkerFailureKind,
    WorkerMessage, WorkerResult, PROTOCOL_SCHEMA,
};
use native::{OwnedFile, PipeConnection, PreflightSnapshot};
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::time::Duration;

const SESSION_TIMEOUT: Duration = Duration::from_secs(45 * 60);
const DESKTOP_PHASES: [crate::windows_lifecycle_proof_contract::DesktopPhase; 6] = [
    crate::windows_lifecycle_proof_contract::DesktopPhase::FinalPrimary,
    crate::windows_lifecycle_proof_contract::DesktopPhase::BaselinePrimary,
    crate::windows_lifecycle_proof_contract::DesktopPhase::BaselineSecondInstance,
    crate::windows_lifecycle_proof_contract::DesktopPhase::FinalMissingService,
    crate::windows_lifecycle_proof_contract::DesktopPhase::FinalStoppedService,
    crate::windows_lifecycle_proof_contract::DesktopPhase::FinalIncompatibleService,
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
    parent_followup_error: Option<String>,
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
                parent_followup_error: None,
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
                parent_followup_error: None,
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
    let snapshot = native::capture_parent_preflight(&plan)?;
    Ok(ParentPreflight {
        plan,
        repo_root,
        controller,
        baseline,
        final_candidate,
        snapshot,
        source_commit_sha,
    })
}

fn run_parent() -> Result<i32, String> {
    let preflight = parent_preflight()?;
    lifecycle::require_controller_ready()?;
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
                native::validate_evidence_root(&accepted.evidence_root, &nonce)?;
                evidence_root = Some(accepted.evidence_root);
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
                });
                send_parent_message(
                    &mut pipe,
                    &nonce,
                    &mut gate,
                    ParentMessage::DesktopPhaseComplete(result),
                )?;
                desktop_phase_index += 1;
            }
            WorkerMessage::Complete(result) => {
                if evidence_root.is_none() || desktop_phase_index != DESKTOP_PHASES.len() {
                    return Err("lifecycle_worker_completion_order_invalid".to_string());
                }
                validate_worker_result(&result, true)?;
                let exit_code = worker.wait(Duration::from_secs(30))?;
                preflight.controller.revalidate()?;
                preflight.baseline.revalidate()?;
                preflight.final_candidate.revalidate()?;
                if exit_code != 0 {
                    return Err("lifecycle_worker_exit_mismatch".to_string());
                }
                print_json(&ControllerOutcome {
                    disposition: "passed",
                    reason: None,
                    worker_failure_kind: None,
                    attempted_stage: None,
                    failure_evidence: None,
                    failure_evidence_verified: None,
                    evidence_error: None,
                    parent_followup_error: None,
                    profile: Some(preflight.plan.profile),
                    controller_source_commit_sha: Some(preflight.source_commit_sha),
                    evidence_root,
                    preflight: Some(preflight.snapshot),
                });
                return Ok(0);
            }
            WorkerMessage::Failed(result) => {
                if evidence_root.is_none() {
                    return Err("lifecycle_worker_failure_before_acceptance".to_string());
                }
                validate_worker_result(&result, false)?;
                let failure = result
                    .failure
                    .as_ref()
                    .ok_or_else(|| "lifecycle_worker_failure_missing".to_string())?;
                let mut evidence_guard = None;
                let followup = (|| -> Result<(), String> {
                    settle_reported_worker_failure(&mut worker, failure.kind)?;
                    if let Some(receipt) = &failure.evidence {
                        evidence_guard = Some(native::verify_evidence_receipt(
                            evidence_root
                                .as_deref()
                                .ok_or_else(|| "lifecycle_evidence_root_missing".to_string())?,
                            receipt,
                        )?);
                    }
                    preflight.controller.revalidate()?;
                    preflight.baseline.revalidate()?;
                    preflight.final_candidate.revalidate()?;
                    Ok(())
                })();
                let failure_evidence_verified =
                    failure.evidence.as_ref().map(|_| evidence_guard.is_some());
                let parent_followup_error = followup.err();
                print_json(&ControllerOutcome {
                    disposition: if parent_followup_error.is_some() {
                        "worker_failure_followup_failed"
                    } else {
                        "worker_failed"
                    },
                    reason: Some(failure.reason.clone()),
                    worker_failure_kind: Some(failure.kind),
                    attempted_stage: failure.attempted_stage,
                    failure_evidence: failure.evidence.clone(),
                    failure_evidence_verified,
                    evidence_error: failure.evidence_error.clone(),
                    parent_followup_error,
                    profile: Some(preflight.plan.profile),
                    controller_source_commit_sha: Some(preflight.source_commit_sha),
                    evidence_root,
                    preflight: Some(preflight.snapshot),
                });
                return Ok(1);
            }
        }
    }
}

fn settle_reported_worker_failure(
    worker: &mut native::ElevatedProcess,
    kind: WorkerFailureKind,
) -> Result<(), String> {
    if kind == WorkerFailureKind::ProcessSettlement {
        return worker.terminate_and_settle();
    }
    match worker.wait(Duration::from_secs(30)) {
        Ok(0) => Ok(()),
        Ok(_) => Err("lifecycle_worker_failure_exit_mismatch".to_string()),
        Err(primary) if worker.is_settled() => Err(primary),
        Err(primary) => match worker.terminate_and_settle() {
            Ok(()) => Err(primary),
            Err(settlement) => Err(format!("{primary}|{settlement}")),
        },
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

    let evidence = native::create_protected_evidence_root(&nonce, &pipe)?;
    send_worker_message(
        &mut pipe,
        &nonce,
        &mut gate,
        WorkerMessage::Accepted(crate::windows_lifecycle_proof_contract::WorkerAccepted {
            evidence_root: evidence.root().to_string_lossy().into_owned(),
            worker_process_id: std::process::id(),
            worker_started_at_100ns: native::current_process_started_at()?,
        }),
    )?;

    let result = lifecycle::execute_worker(lifecycle::WorkerContext {
        plan: &plan,
        repo_root: &repo_root,
        baseline: &baseline,
        final_candidate: &final_candidate,
        evidence: &evidence,
        pipe: &mut pipe,
        nonce: &nonce,
        gate: &mut gate,
    });
    let message = if result.failure.is_none() {
        WorkerMessage::Complete(result)
    } else {
        WorkerMessage::Failed(result)
    };
    send_worker_message(&mut pipe, &nonce, &mut gate, message)?;
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
    if expected_success && (!result.private_evidence_complete || !result.sanitized_export_complete)
    {
        return Err("lifecycle_worker_evidence_incomplete".to_string());
    }
    if expected_success {
        if !result.process_tree_settled {
            return Err("lifecycle_worker_result_invalid".to_string());
        }
        return Ok(());
    }
    if result.private_evidence_complete || result.sanitized_export_complete {
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
    Ok(())
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
    fn worker_result_requires_matching_disposition_and_settlement() {
        let passed = WorkerResult {
            disposition: WorkerDisposition::Passed,
            completed_stage: None,
            failure: None,
            process_tree_settled: true,
            private_evidence_complete: true,
            sanitized_export_complete: true,
        };
        assert!(validate_worker_result(&passed, true).is_ok());

        let mut forged = passed;
        forged.disposition = WorkerDisposition::Failed;
        assert_eq!(
            validate_worker_result(&forged, true),
            Err("lifecycle_worker_disposition_invalid".to_string())
        );
    }

    #[test]
    fn worker_failure_kinds_require_their_exact_evidence_shape() {
        let mutation = failed_result(WorkerFailure {
            kind: WorkerFailureKind::Mutation,
            attempted_stage: Some(LifecycleStage::FinalRepair),
            reason: "mutation_failed".to_string(),
            evidence: Some(evidence_receipt("final-repair-failure.private.json")),
            evidence_error: None,
        });
        assert!(validate_worker_result(&mutation, false).is_ok());

        let evidence_write = failed_result(WorkerFailure {
            kind: WorkerFailureKind::EvidenceWrite,
            attempted_stage: None,
            reason: "evidence_incomplete".to_string(),
            evidence: None,
            evidence_error: Some("write_failed".to_string()),
        });
        assert!(validate_worker_result(&evidence_write, false).is_ok());

        let mut settlement = failed_result(WorkerFailure {
            kind: WorkerFailureKind::ProcessSettlement,
            attempted_stage: Some(LifecycleStage::FinalRepair),
            reason: "settlement_unproven".to_string(),
            evidence: Some(evidence_receipt("final-repair-failure.private.json")),
            evidence_error: None,
        });
        settlement.process_tree_settled = false;
        assert!(validate_worker_result(&settlement, false).is_ok());

        let controller = failed_result(WorkerFailure {
            kind: WorkerFailureKind::Controller,
            attempted_stage: None,
            reason: "controller_failed".to_string(),
            evidence: None,
            evidence_error: None,
        });
        assert!(validate_worker_result(&controller, false).is_ok());
    }

    #[test]
    fn worker_failure_rejects_forged_settlement_and_receipt_combinations() {
        let forged_mutation = failed_result(WorkerFailure {
            kind: WorkerFailureKind::Mutation,
            attempted_stage: Some(LifecycleStage::FinalRepair),
            reason: "mutation_failed".to_string(),
            evidence: None,
            evidence_error: None,
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
            evidence: Some(evidence_receipt("baseline-install-failure.private.json")),
            evidence_error: None,
        });
        assert_eq!(
            validate_worker_result(&forged_leaf, false),
            Err("lifecycle_worker_failure_shape_invalid".to_string())
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
        ] {
            let result = failed_result_at(
                completed,
                WorkerFailure {
                    kind: WorkerFailureKind::Mutation,
                    attempted_stage: Some(attempted),
                    reason: "mutation_failed".to_string(),
                    evidence: Some(evidence_receipt(leaf)),
                    evidence_error: None,
                },
            );
            assert!(validate_worker_result(&result, false).is_ok(), "{leaf}");
        }
    }

    fn failed_result(failure: WorkerFailure) -> WorkerResult {
        failed_result_at(LifecycleStage::InitialState, failure)
    }

    fn failed_result_at(completed_stage: LifecycleStage, failure: WorkerFailure) -> WorkerResult {
        WorkerResult {
            disposition: WorkerDisposition::Failed,
            completed_stage: Some(completed_stage),
            failure: Some(failure),
            process_tree_settled: true,
            private_evidence_complete: false,
            sanitized_export_complete: false,
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
