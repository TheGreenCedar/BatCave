use super::native::{
    capture_elevated_machine_snapshot, open_installed_uninstaller,
    require_allowlisted_elevated_preflight, require_elevated_installed_candidate,
    require_elevated_total_product_absence, ElevatedMachineSnapshot, ExecuteFailure, OwnedFile,
    PipeConnection, ProcessTerminal, ProcessTerminalSnapshot, ProtectedEvidenceRoot,
};
use crate::windows_lifecycle_proof_contract::{
    DesktopPhase, DesktopPhaseDisposition, DesktopPhaseResult, LifecycleStage, ProofPlan,
    SequenceGate, WorkerDisposition, WorkerFailure, WorkerFailureKind, WorkerResult,
};
use serde::Serialize;
use std::path::Path;
use std::time::Duration;

const CONTROLLER_READY: bool = false;
const MUTATION_FAILURE_SCHEMA: &str = "batcave.windows-lifecycle.mutation-failure.v1";
const INSTALLER_TIMEOUT: Duration = Duration::from_secs(10 * 60);
const UNINSTALLER_TIMEOUT: Duration = Duration::from_secs(10 * 60);
const FINAL_REPAIR_ARGUMENTS: &str = "/S /UPDATE";
const BASELINE_INSTALL_ARGUMENTS: &str = "/S";
const DIRECT_UNINSTALL_ARGUMENTS: &str = r"/S _?=C:\Program Files\BatCave Monitor";

pub(super) fn require_controller_ready() -> Result<(), String> {
    if CONTROLLER_READY {
        Ok(())
    } else {
        Err("lifecycle_controller_not_reviewed".to_string())
    }
}

pub(super) fn run_parent_desktop_phase(
    phase: DesktopPhase,
    _repo_root: &Path,
    _plan: &ProofPlan,
) -> Result<DesktopPhaseResult, String> {
    Ok(DesktopPhaseResult {
        phase,
        disposition: DesktopPhaseDisposition::Failed,
        process_tree_settled: true,
    })
}

pub(super) struct WorkerContext<'a> {
    pub(super) plan: &'a ProofPlan,
    pub(super) repo_root: &'a Path,
    pub(super) baseline: &'a OwnedFile,
    pub(super) final_candidate: &'a OwnedFile,
    pub(super) evidence: &'a ProtectedEvidenceRoot,
    pub(super) pipe: &'a mut PipeConnection,
    pub(super) nonce: &'a str,
    pub(super) gate: &'a mut SequenceGate,
}

pub(super) fn execute_worker(context: WorkerContext<'_>) -> WorkerResult {
    let WorkerContext {
        plan,
        repo_root: _repo_root,
        baseline,
        final_candidate,
        evidence,
        pipe: _pipe,
        nonce: _nonce,
        gate: _gate,
    } = context;
    if let Err(failure) = require_controller_ready() {
        return failed(None, controller_failure(failure), true);
    }
    let result = execute_worker_inner(plan, baseline, final_candidate, evidence);
    match result {
        Ok(completed_stage) => failed(
            Some(completed_stage),
            controller_failure("lifecycle_remaining_stages_not_implemented".to_string()),
            true,
        ),
        Err((completed_stage, failure, settled)) => failed(completed_stage, failure, settled),
    }
}

fn execute_worker_inner(
    plan: &ProofPlan,
    baseline: &OwnedFile,
    final_candidate: &OwnedFile,
    evidence: &ProtectedEvidenceRoot,
) -> Result<LifecycleStage, (Option<LifecycleStage>, WorkerFailure, bool)> {
    let initial = capture_elevated_machine_snapshot();
    evidence
        .write_json_new("initial-state.private.json", &initial)
        .map_err(|failure| {
            (
                None,
                evidence_write_failure("lifecycle_initial_state_evidence_incomplete", failure),
                true,
            )
        })?;
    require_allowlisted_elevated_preflight(&initial, plan)
        .map_err(|failure| (None, controller_failure(failure), true))?;

    let final_copy = final_candidate
        .copy_to(
            &evidence.root().join("final-installer.exe"),
            "final_installer_copy",
        )
        .map_err(|failure| {
            (
                Some(LifecycleStage::InitialState),
                controller_failure(failure),
                true,
            )
        })?;
    let final_repair_state = execute_mutation(
        evidence,
        "final-repair-failure.private.json",
        LifecycleStage::FinalRepair,
        &initial,
        &final_copy,
        FINAL_REPAIR_ARGUMENTS,
        INSTALLER_TIMEOUT,
        "final_repair",
        "lifecycle_final_repair_failed",
        |snapshot| {
            require_elevated_installed_candidate(
                snapshot,
                &plan.final_candidate,
                true,
                "final_repair",
            )
        },
    )
    .map_err(|(failure, settled)| (Some(LifecycleStage::InitialState), failure, settled))?;
    evidence
        .write_json_new("final-repair-state.private.json", &final_repair_state)
        .map_err(|failure| {
            (
                Some(LifecycleStage::FinalRepair),
                evidence_write_failure("lifecycle_final_repair_evidence_incomplete", failure),
                true,
            )
        })?;

    let installed_uninstaller =
        open_installed_uninstaller(&plan.final_candidate).map_err(|failure| {
            (
                Some(LifecycleStage::FinalRepair),
                controller_failure(failure),
                true,
            )
        })?;
    let uninstaller_copy = installed_uninstaller
        .copy_to(
            &evidence.root().join("final-uninstaller.exe"),
            "final_uninstaller_copy",
        )
        .map_err(|failure| {
            (
                Some(LifecycleStage::FinalRepair),
                controller_failure(failure),
                true,
            )
        })?;
    let initial_uninstall_state = execute_mutation(
        evidence,
        "initial-uninstall-failure.private.json",
        LifecycleStage::InitialUninstall,
        &final_repair_state,
        &uninstaller_copy,
        DIRECT_UNINSTALL_ARGUMENTS,
        UNINSTALLER_TIMEOUT,
        "initial_uninstall",
        "lifecycle_initial_uninstall_failed",
        |snapshot| require_elevated_total_product_absence(snapshot, "initial_uninstall"),
    )
    .map_err(|(failure, settled)| (Some(LifecycleStage::FinalRepair), failure, settled))?;
    evidence
        .write_json_new(
            "initial-uninstall-state.private.json",
            &initial_uninstall_state,
        )
        .map_err(|failure| {
            (
                Some(LifecycleStage::InitialUninstall),
                evidence_write_failure("lifecycle_initial_uninstall_evidence_incomplete", failure),
                true,
            )
        })?;

    let baseline_copy = baseline
        .copy_to(
            &evidence.root().join("baseline-installer.exe"),
            "baseline_installer_copy",
        )
        .map_err(|failure| {
            (
                Some(LifecycleStage::InitialUninstall),
                controller_failure(failure),
                true,
            )
        })?;
    let baseline_install_state = execute_mutation(
        evidence,
        "baseline-install-failure.private.json",
        LifecycleStage::BaselineInstall,
        &initial_uninstall_state,
        &baseline_copy,
        BASELINE_INSTALL_ARGUMENTS,
        INSTALLER_TIMEOUT,
        "baseline_install",
        "lifecycle_baseline_install_failed",
        |snapshot| {
            require_elevated_installed_candidate(
                snapshot,
                &plan.baseline,
                false,
                "baseline_install",
            )
        },
    )
    .map_err(|(failure, settled)| (Some(LifecycleStage::InitialUninstall), failure, settled))?;
    evidence
        .write_json_new(
            "baseline-install-state.private.json",
            &baseline_install_state,
        )
        .map_err(|failure| {
            (
                Some(LifecycleStage::BaselineInstall),
                evidence_write_failure("lifecycle_baseline_install_evidence_incomplete", failure),
                true,
            )
        })?;
    Ok(LifecycleStage::BaselineInstall)
}

#[derive(Serialize)]
struct MutationPreSettlement<'a> {
    machine_before_mutation: &'a ElevatedMachineSnapshot,
    process_tree_at_failure: &'a ProcessTerminalSnapshot,
}

#[derive(Serialize)]
struct SettledMutationFailurePacket<'a> {
    schema_version: &'static str,
    attempted_stage: LifecycleStage,
    reason: &'a str,
    executable_revalidation_error: Option<&'a str>,
    process_tree_settled: bool,
    pre_settlement: MutationPreSettlement<'a>,
    post_settlement: &'a ElevatedMachineSnapshot,
}

#[derive(Serialize)]
struct UnsettledMutationFailurePacket<'a> {
    schema_version: &'static str,
    attempted_stage: LifecycleStage,
    reason: &'a str,
    process_tree_settled: bool,
    pre_settlement: MutationPreSettlement<'a>,
}

#[allow(clippy::too_many_arguments)]
fn execute_mutation(
    evidence: &ProtectedEvidenceRoot,
    evidence_name: &'static str,
    attempted_stage: LifecycleStage,
    before: &ElevatedMachineSnapshot,
    executable: &OwnedFile,
    arguments: &'static str,
    timeout: Duration,
    label: &'static str,
    unsuccessful_reason: &'static str,
    validate: impl FnOnce(&ElevatedMachineSnapshot) -> Result<(), String>,
) -> Result<ElevatedMachineSnapshot, (WorkerFailure, bool)> {
    let outcome = match executable.execute(evidence, arguments, timeout, label) {
        Ok(outcome) => outcome,
        Err(ExecuteFailure::NotStarted(reason)) => {
            return Err((controller_failure(reason), true));
        }
        Err(ExecuteFailure::SettlementUnproven { reason, terminal }) => {
            let packet = UnsettledMutationFailurePacket {
                schema_version: MUTATION_FAILURE_SCHEMA,
                attempted_stage,
                reason: &reason,
                process_tree_settled: false,
                pre_settlement: MutationPreSettlement {
                    machine_before_mutation: before,
                    process_tree_at_failure: &terminal,
                },
            };
            let (receipt, evidence_error) = match evidence.write_json_new(evidence_name, &packet) {
                Ok(receipt) => (Some(receipt), None),
                Err(error) => (None, Some(error)),
            };
            return Err((
                WorkerFailure {
                    kind: WorkerFailureKind::ProcessSettlement,
                    attempted_stage: Some(attempted_stage),
                    reason,
                    evidence: receipt,
                    evidence_error,
                },
                false,
            ));
        }
    };

    let post_settlement = capture_elevated_machine_snapshot();
    let revalidation_error = executable.revalidate().err();
    let failure_reason = match &outcome.terminal.terminal {
        ProcessTerminal::Exited { exit_code: 0 } if revalidation_error.is_none() => {
            validate(&post_settlement).err()
        }
        ProcessTerminal::Exited { exit_code: 0 } => revalidation_error.clone(),
        terminal => Some(mutation_terminal_failure(unsuccessful_reason, terminal)),
    };
    let Some(reason) = failure_reason else {
        return Ok(post_settlement);
    };

    let packet = SettledMutationFailurePacket {
        schema_version: MUTATION_FAILURE_SCHEMA,
        attempted_stage,
        reason: &reason,
        executable_revalidation_error: revalidation_error.as_deref(),
        process_tree_settled: true,
        pre_settlement: MutationPreSettlement {
            machine_before_mutation: before,
            process_tree_at_failure: &outcome.terminal,
        },
        post_settlement: &post_settlement,
    };
    match evidence.write_json_new(evidence_name, &packet) {
        Ok(receipt) => Err((
            WorkerFailure {
                kind: WorkerFailureKind::Mutation,
                attempted_stage: Some(attempted_stage),
                reason,
                evidence: Some(receipt),
                evidence_error: None,
            },
            true,
        )),
        Err(error) => Err((
            WorkerFailure {
                kind: WorkerFailureKind::EvidenceWrite,
                attempted_stage: Some(attempted_stage),
                reason,
                evidence: None,
                evidence_error: Some(error),
            },
            true,
        )),
    }
}

fn mutation_terminal_failure(unsuccessful_reason: &str, terminal: &ProcessTerminal) -> String {
    match terminal {
        ProcessTerminal::Exited { exit_code } => {
            format!("{unsuccessful_reason}:exit_code_{exit_code}")
        }
        ProcessTerminal::TimedOut => format!("{unsuccessful_reason}:timeout"),
        ProcessTerminal::SupervisionFailed { reason } => {
            format!("{unsuccessful_reason}:{reason}")
        }
    }
}

fn controller_failure(reason: String) -> WorkerFailure {
    WorkerFailure {
        kind: WorkerFailureKind::Controller,
        attempted_stage: None,
        reason,
        evidence: None,
        evidence_error: None,
    }
}

fn evidence_write_failure(reason: &str, evidence_error: String) -> WorkerFailure {
    WorkerFailure {
        kind: WorkerFailureKind::EvidenceWrite,
        attempted_stage: None,
        reason: reason.to_string(),
        evidence: None,
        evidence_error: Some(evidence_error),
    }
}

fn failed(
    completed_stage: Option<LifecycleStage>,
    failure: WorkerFailure,
    process_tree_settled: bool,
) -> WorkerResult {
    WorkerResult {
        disposition: WorkerDisposition::Failed,
        completed_stage,
        failure: Some(failure),
        process_tree_settled,
        private_evidence_complete: false,
        sanitized_export_complete: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn direct_uninstaller_form_is_fixed_last_and_unquoted() {
        assert_eq!(
            DIRECT_UNINSTALL_ARGUMENTS,
            r"/S _?=C:\Program Files\BatCave Monitor"
        );
        assert!(DIRECT_UNINSTALL_ARGUMENTS.ends_with(r"BatCave Monitor"));
        assert!(!DIRECT_UNINSTALL_ARGUMENTS.contains('"'));
    }

    #[test]
    fn controller_remains_fail_closed_until_reviewed() {
        assert_eq!(
            require_controller_ready(),
            Err("lifecycle_controller_not_reviewed".to_string())
        );
    }

    #[test]
    fn terminal_failure_reason_survives_an_evidence_write_failure() {
        assert_eq!(
            mutation_terminal_failure(
                "lifecycle_install_failed",
                &ProcessTerminal::Exited { exit_code: 23 }
            ),
            "lifecycle_install_failed:exit_code_23"
        );
        assert_eq!(
            mutation_terminal_failure("lifecycle_install_failed", &ProcessTerminal::TimedOut),
            "lifecycle_install_failed:timeout"
        );
        assert_eq!(
            mutation_terminal_failure(
                "lifecycle_install_failed",
                &ProcessTerminal::SupervisionFailed {
                    reason: "lifecycle_wait_failed".to_string(),
                }
            ),
            "lifecycle_install_failed:lifecycle_wait_failed"
        );
    }
}
