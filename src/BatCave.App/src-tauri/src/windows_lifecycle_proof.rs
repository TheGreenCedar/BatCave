mod lifecycle;
mod native;

use crate::windows_lifecycle_proof_contract::{
    message_sha256, parse_plan, plan_sha256, validate_envelope, validate_locator, validate_nonce,
    validate_sha256, ClosedRequest, DesktopPhaseDisposition, DesktopPhaseResult, Envelope,
    ParentMessage, ProofPlan, SequenceGate, WorkerMessage, WorkerResult, PROTOCOL_SCHEMA,
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
                let exit_code = worker.wait(Duration::from_secs(30))?;
                preflight.controller.revalidate()?;
                preflight.baseline.revalidate()?;
                preflight.final_candidate.revalidate()?;
                if exit_code != 0 {
                    return Err("lifecycle_worker_failure_exit_mismatch".to_string());
                }
                print_json(&ControllerOutcome {
                    disposition: "worker_failed",
                    reason: result.failure,
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
    if result.failure.is_none() != expected_success || !result.process_tree_settled {
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
    Ok(())
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
    use crate::windows_lifecycle_proof_contract::{WorkerDisposition, WorkerResult};

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
}
