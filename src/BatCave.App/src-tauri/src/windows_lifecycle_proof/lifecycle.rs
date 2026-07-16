use super::native::{
    capture_elevated_machine_snapshot, open_allowlisted_legacy_cli, open_installed_service,
    open_installed_uninstaller, parse_sha256, require_allowlisted_elevated_preflight,
    require_elevated_crashed_candidate, require_elevated_installed_candidate,
    require_elevated_stopped_candidate, require_elevated_total_product_absence,
    ElevatedMachineSnapshot, ExecuteFailure, OwnedFile, PipeConnection, ProcessTerminal,
    ProcessTerminalSnapshot, ProtectedEvidenceRoot,
};
use crate::windows_lifecycle_proof_contract::{
    DesktopPhase, DesktopPhaseDisposition, DesktopPhaseResult, LifecycleStage, ProofPlan,
    RestorationOutcome, SequenceGate, WorkerDisposition, WorkerFailure, WorkerFailureKind,
    WorkerResult,
};
use serde::Serialize;
use std::path::Path;
use std::time::Duration;

const CONTROLLER_READY: bool = false;
const MUTATION_FAILURE_SCHEMA: &str = "batcave.windows-lifecycle.mutation-failure.v1";
const INSTALLER_TIMEOUT: Duration = Duration::from_secs(10 * 60);
const UNINSTALLER_TIMEOUT: Duration = Duration::from_secs(10 * 60);
const SERVICE_OPERATION_TIMEOUT: Duration = Duration::from_secs(2 * 60);
const FINAL_REPAIR_ARGUMENTS: &str = "/S /UPDATE";
const BASELINE_INSTALL_ARGUMENTS: &str = "/S";
const DIRECT_UNINSTALL_ARGUMENTS: &str = r"/S _?=C:\Program Files\BatCave Monitor";
const SERVICE_STOP_ARGUMENTS: &str = "--provision prepare-upgrade";
const SERVICE_START_ARGUMENTS: &str = "--provision install";

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
        observation: None,
        failure_reason: Some("lifecycle_desktop_phase_not_implemented".to_string()),
    })
}

pub(super) struct WorkerContext<'a> {
    pub(super) plan: &'a ProofPlan,
    pub(super) repo_root: &'a Path,
    pub(super) baseline: &'a OwnedFile,
    pub(super) final_candidate: &'a OwnedFile,
    pub(super) incompatible_service_fixture: &'a OwnedFile,
    pub(super) rollback_failing_service_fixture: &'a OwnedFile,
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
        incompatible_service_fixture: _incompatible_service_fixture,
        rollback_failing_service_fixture,
        evidence,
        pipe: _pipe,
        nonce: _nonce,
        gate: _gate,
    } = context;
    if let Err(failure) = require_controller_ready() {
        return failed(None, controller_failure(failure), true);
    }
    let result = execute_worker_inner(
        plan,
        baseline,
        final_candidate,
        rollback_failing_service_fixture,
        evidence,
    );
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
    rollback_failing_service_fixture: &OwnedFile,
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

    let historical_cli = open_allowlisted_legacy_cli(plan).map_err(|failure| {
        (
            Some(LifecycleStage::InitialState),
            controller_failure(failure),
            true,
        )
    })?;
    let _historical_cli_copy = historical_cli
        .copy_to(
            &evidence.root().join("historical-cli.exe"),
            "historical_cli_copy",
        )
        .map_err(|failure| {
            (
                Some(LifecycleStage::InitialState),
                controller_failure(failure),
                true,
            )
        })?;
    drop(historical_cli);

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

    let baseline_service = open_installed_service(&plan.baseline).map_err(|failure| {
        (
            Some(LifecycleStage::BaselineInstall),
            controller_failure(failure),
            true,
        )
    })?;
    let baseline_restart_stopped = execute_mutation(
        evidence,
        "baseline-restart-failure.private.json",
        LifecycleStage::BaselineRestart,
        &baseline_install_state,
        &baseline_service,
        SERVICE_STOP_ARGUMENTS,
        SERVICE_OPERATION_TIMEOUT,
        "baseline_restart_stop",
        "lifecycle_baseline_restart_stop_failed",
        |snapshot| {
            require_elevated_stopped_candidate(
                snapshot,
                &plan.baseline,
                false,
                "baseline_restart_stop",
            )
        },
    )
    .map_err(|(failure, settled)| (Some(LifecycleStage::BaselineInstall), failure, settled))?;
    evidence
        .write_json_new(
            "baseline-restart-stopped-state.private.json",
            &baseline_restart_stopped,
        )
        .map_err(|failure| {
            (
                Some(LifecycleStage::BaselineInstall),
                evidence_write_failure(
                    "lifecycle_baseline_restart_stopped_evidence_incomplete",
                    failure,
                ),
                true,
            )
        })?;
    let baseline_restart_state = execute_mutation(
        evidence,
        "baseline-restart-failure.private.json",
        LifecycleStage::BaselineRestart,
        &baseline_restart_stopped,
        &baseline_service,
        SERVICE_START_ARGUMENTS,
        SERVICE_OPERATION_TIMEOUT,
        "baseline_restart_start",
        "lifecycle_baseline_restart_start_failed",
        |snapshot| {
            require_elevated_installed_candidate(
                snapshot,
                &plan.baseline,
                false,
                "baseline_restart_start",
            )
        },
    )
    .map_err(|(failure, settled)| (Some(LifecycleStage::BaselineInstall), failure, settled))?;
    evidence
        .write_json_new(
            "baseline-restart-state.private.json",
            &baseline_restart_state,
        )
        .map_err(|failure| {
            (
                Some(LifecycleStage::BaselineRestart),
                evidence_write_failure("lifecycle_baseline_restart_evidence_incomplete", failure),
                true,
            )
        })?;

    require_running_service(&baseline_restart_state, "baseline_crash_before").map_err(
        |failure| {
            (
                Some(LifecycleStage::BaselineRestart),
                controller_failure(failure),
                true,
            )
        },
    )?;
    let expected_service_sha256 = parse_sha256(&plan.baseline.service_sha256, "baseline_service")
        .map_err(|failure| {
        (
            Some(LifecycleStage::BaselineRestart),
            controller_failure(failure),
            true,
        )
    })?;
    let termination =
        match crate::collector_service::windows_provisioner::terminate_running_service_for_proof(
            expected_service_sha256,
        ) {
            Ok(termination) => termination,
            Err(failure) => {
                let failure = write_service_crash_failure(
                    evidence,
                    "baseline-crash-recovery-failure.private.json",
                    LifecycleStage::BaselineCrashRecovery,
                    &baseline_restart_state,
                    &failure.reason,
                    failure.service_settled,
                    ServiceCrashMutationObservation::Failed(&failure),
                );
                let process_tree_settled = failure.kind != WorkerFailureKind::ProcessSettlement;
                return Err((
                    Some(LifecycleStage::BaselineRestart),
                    failure,
                    process_tree_settled,
                ));
            }
        };
    let baseline_crashed_state = capture_elevated_machine_snapshot();
    if let Err(reason) = require_elevated_crashed_candidate(
        &baseline_crashed_state,
        &plan.baseline,
        false,
        "baseline_crash",
    ) {
        let failure = write_service_crash_failure(
            evidence,
            "baseline-crash-recovery-failure.private.json",
            LifecycleStage::BaselineCrashRecovery,
            &baseline_restart_state,
            &reason,
            true,
            ServiceCrashMutationObservation::Terminated(&termination),
        );
        return Err((Some(LifecycleStage::BaselineRestart), failure, true));
    }
    let crashed = ServiceCrashState {
        termination,
        machine: &baseline_crashed_state,
    };
    evidence
        .write_json_new("baseline-crashed-state.private.json", &crashed)
        .map_err(|failure| {
            (
                Some(LifecycleStage::BaselineRestart),
                evidence_write_failure("lifecycle_baseline_crashed_evidence_incomplete", failure),
                true,
            )
        })?;
    let baseline_crash_recovery_state = execute_mutation(
        evidence,
        "baseline-crash-recovery-failure.private.json",
        LifecycleStage::BaselineCrashRecovery,
        &baseline_crashed_state,
        &baseline_service,
        SERVICE_START_ARGUMENTS,
        SERVICE_OPERATION_TIMEOUT,
        "baseline_crash_recovery",
        "lifecycle_baseline_crash_recovery_failed",
        |snapshot| {
            require_elevated_installed_candidate(
                snapshot,
                &plan.baseline,
                false,
                "baseline_crash_recovery",
            )?;
            require_running_service(snapshot, "baseline_crash_recovery").map(|_| ())
        },
    )
    .map_err(|(failure, settled)| (Some(LifecycleStage::BaselineRestart), failure, settled))?;
    evidence
        .write_json_new(
            "baseline-crash-recovery-state.private.json",
            &baseline_crash_recovery_state,
        )
        .map_err(|failure| {
            (
                Some(LifecycleStage::BaselineCrashRecovery),
                evidence_write_failure(
                    "lifecycle_baseline_crash_recovery_evidence_incomplete",
                    failure,
                ),
                true,
            )
        })?;

    let rollback_fixture_copy = rollback_failing_service_fixture
        .copy_to(
            &evidence.root().join("rollback-failing-service.exe"),
            "rollback_failing_service_copy",
        )
        .map_err(|failure| {
            (
                Some(LifecycleStage::BaselineCrashRecovery),
                controller_failure(failure),
                true,
            )
        })?;
    let rollback_fixture_bytes = rollback_fixture_copy
        .read_all_exact("rollback_failing_service")
        .map_err(|failure| {
            (
                Some(LifecycleStage::BaselineCrashRecovery),
                controller_failure(failure),
                true,
            )
        })?;
    let rollback_fixture_sha256 = parse_sha256(
        &plan.rollback_failing_service_fixture.sha256,
        "rollback_failing_service",
    )
    .map_err(|failure| {
        (
            Some(LifecycleStage::BaselineCrashRecovery),
            controller_failure(failure),
            true,
        )
    })?;
    let baseline_service_sha256 = parse_sha256(&plan.baseline.service_sha256, "baseline_service")
        .map_err(|failure| {
        (
            Some(LifecycleStage::BaselineCrashRecovery),
            controller_failure(failure),
            true,
        )
    })?;
    let rollback = match crate::collector_service::windows_provisioner::exercise_failed_upgrade_rollback_for_proof(
        &rollback_fixture_bytes,
        rollback_fixture_sha256,
        baseline_service_sha256,
    ) {
        Ok(rollback) => rollback,
        Err(failure) => {
            let failure = write_upgrade_rollback_failure(
                evidence,
                "baseline-rollback-recovery-failure.private.json",
                &baseline_crash_recovery_state,
                &failure.reason,
                failure.service_settled,
            );
            let process_tree_settled = failure.kind != WorkerFailureKind::ProcessSettlement;
            return Err((
                Some(LifecycleStage::BaselineCrashRecovery),
                failure,
                process_tree_settled,
            ));
        }
    };
    if let Err(reason) = rollback_failing_service_fixture
        .revalidate()
        .and_then(|_| rollback_fixture_copy.revalidate())
    {
        let failure = write_upgrade_rollback_failure(
            evidence,
            "baseline-rollback-recovery-failure.private.json",
            &baseline_crash_recovery_state,
            &reason,
            true,
        );
        return Err((Some(LifecycleStage::BaselineCrashRecovery), failure, true));
    }
    let baseline_rollback_recovery_state = capture_elevated_machine_snapshot();
    require_elevated_installed_candidate(
        &baseline_rollback_recovery_state,
        &plan.baseline,
        false,
        "baseline_rollback_recovery",
    )
    .and_then(|_| {
        require_running_service(
            &baseline_rollback_recovery_state,
            "baseline_rollback_recovery",
        )
        .map(|_| ())
    })
    .map_err(|reason| {
        let failure = write_upgrade_rollback_failure(
            evidence,
            "baseline-rollback-recovery-failure.private.json",
            &baseline_crash_recovery_state,
            &reason,
            true,
        );
        (Some(LifecycleStage::BaselineCrashRecovery), failure, true)
    })?;
    let rollback_state = UpgradeRollbackState {
        rollback,
        machine: &baseline_rollback_recovery_state,
    };
    evidence
        .write_json_new(
            "baseline-rollback-recovery-state.private.json",
            &rollback_state,
        )
        .map_err(|failure| {
            (
                Some(LifecycleStage::BaselineRollbackRecovery),
                evidence_write_failure(
                    "lifecycle_baseline_rollback_recovery_evidence_incomplete",
                    failure,
                ),
                true,
            )
        })?;
    Ok(LifecycleStage::BaselineRollbackRecovery)
}

#[derive(Serialize)]
struct UpgradeRollbackState<'a> {
    rollback: crate::collector_service::windows_provisioner::FailedUpgradeRollbackForProof,
    machine: &'a ElevatedMachineSnapshot,
}

#[derive(Serialize)]
struct ServiceCrashState<'a> {
    termination: crate::collector_service::windows_provisioner::TerminatedServiceForProof,
    machine: &'a ElevatedMachineSnapshot,
}

#[derive(Serialize)]
struct ServiceCrashFailurePacket<'a> {
    schema_version: &'static str,
    attempted_stage: LifecycleStage,
    reason: &'a str,
    service_settled: bool,
    mutation: ServiceCrashMutationObservation<'a>,
    machine_before_mutation: &'a ElevatedMachineSnapshot,
    machine_after_attempt: ElevatedMachineSnapshot,
}

#[derive(Serialize)]
#[serde(rename_all = "snake_case", tag = "outcome", content = "observation")]
enum ServiceCrashMutationObservation<'a> {
    Failed(&'a crate::collector_service::windows_provisioner::ServiceTerminationFailure),
    Terminated(&'a crate::collector_service::windows_provisioner::TerminatedServiceForProof),
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
                    evidence: receipt.map(Box::new),
                    evidence_error,
                    restoration: Box::new(RestorationOutcome::BlockedUnsettled),
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
                evidence: Some(Box::new(receipt)),
                evidence_error: None,
                restoration: Box::new(restoration_not_reviewed()),
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
                restoration: Box::new(restoration_not_reviewed()),
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

fn require_running_service<'a>(
    snapshot: &'a ElevatedMachineSnapshot,
    label: &str,
) -> Result<&'a super::native::ServiceSnapshot, String> {
    match &snapshot.machine.service {
        crate::windows_lifecycle_proof_contract::Observation::Present(service)
            if service.state == windows_sys::Win32::System::Services::SERVICE_RUNNING
                && service.process_id != 0 =>
        {
            Ok(service)
        }
        _ => Err(format!("lifecycle_{label}_service_not_running")),
    }
}

fn write_service_crash_failure(
    evidence: &ProtectedEvidenceRoot,
    evidence_name: &'static str,
    attempted_stage: LifecycleStage,
    before: &ElevatedMachineSnapshot,
    reason: &str,
    service_settled: bool,
    mutation: ServiceCrashMutationObservation<'_>,
) -> WorkerFailure {
    let packet = ServiceCrashFailurePacket {
        schema_version: MUTATION_FAILURE_SCHEMA,
        attempted_stage,
        reason,
        service_settled,
        mutation,
        machine_before_mutation: before,
        machine_after_attempt: capture_elevated_machine_snapshot(),
    };
    let (receipt, evidence_error) = match evidence.write_json_new(evidence_name, &packet) {
        Ok(receipt) => (Some(receipt), None),
        Err(error) => (None, Some(error)),
    };
    WorkerFailure {
        kind: if service_settled {
            if receipt.is_some() {
                WorkerFailureKind::Mutation
            } else {
                WorkerFailureKind::EvidenceWrite
            }
        } else {
            WorkerFailureKind::ProcessSettlement
        },
        attempted_stage: Some(attempted_stage),
        reason: reason.to_string(),
        evidence: receipt.map(Box::new),
        evidence_error,
        restoration: Box::new(if service_settled {
            restoration_not_reviewed()
        } else {
            RestorationOutcome::BlockedUnsettled
        }),
    }
}

fn write_upgrade_rollback_failure(
    evidence: &ProtectedEvidenceRoot,
    evidence_name: &'static str,
    before: &ElevatedMachineSnapshot,
    reason: &str,
    service_settled: bool,
) -> WorkerFailure {
    let packet = UpgradeRollbackFailurePacket {
        schema_version: MUTATION_FAILURE_SCHEMA,
        attempted_stage: LifecycleStage::BaselineRollbackRecovery,
        reason,
        service_settled,
        machine_before_mutation: before,
        machine_after_attempt: capture_elevated_machine_snapshot(),
    };
    let (receipt, evidence_error) = match evidence.write_json_new(evidence_name, &packet) {
        Ok(receipt) => (Some(receipt), None),
        Err(error) => (None, Some(error)),
    };
    WorkerFailure {
        kind: if service_settled {
            if receipt.is_some() {
                WorkerFailureKind::Mutation
            } else {
                WorkerFailureKind::EvidenceWrite
            }
        } else {
            WorkerFailureKind::ProcessSettlement
        },
        attempted_stage: Some(LifecycleStage::BaselineRollbackRecovery),
        reason: reason.to_string(),
        evidence: receipt.map(Box::new),
        evidence_error,
        restoration: Box::new(if service_settled {
            restoration_not_reviewed()
        } else {
            RestorationOutcome::BlockedUnsettled
        }),
    }
}

#[derive(Serialize)]
struct UpgradeRollbackFailurePacket<'a> {
    schema_version: &'static str,
    attempted_stage: LifecycleStage,
    reason: &'a str,
    service_settled: bool,
    machine_before_mutation: &'a ElevatedMachineSnapshot,
    machine_after_attempt: ElevatedMachineSnapshot,
}

fn controller_failure(reason: String) -> WorkerFailure {
    WorkerFailure {
        kind: WorkerFailureKind::Controller,
        attempted_stage: None,
        reason,
        evidence: None,
        evidence_error: None,
        restoration: Box::new(RestorationOutcome::NotRequired),
    }
}

fn evidence_write_failure(reason: &str, evidence_error: String) -> WorkerFailure {
    WorkerFailure {
        kind: WorkerFailureKind::EvidenceWrite,
        attempted_stage: None,
        reason: reason.to_string(),
        evidence: None,
        evidence_error: Some(evidence_error),
        restoration: Box::new(restoration_not_reviewed()),
    }
}

fn restoration_not_reviewed() -> RestorationOutcome {
    RestorationOutcome::BlockedUntrusted {
        reason: "lifecycle_restoration_not_reviewed".to_string(),
    }
}

fn failed(
    completed_stage: Option<LifecycleStage>,
    mut failure: WorkerFailure,
    process_tree_settled: bool,
) -> WorkerResult {
    if process_tree_settled
        && matches!(
            failure.kind,
            WorkerFailureKind::EvidenceWrite | WorkerFailureKind::Controller
        )
    {
        let restoration_required = failure
            .attempted_stage
            .or(completed_stage)
            .is_some_and(|stage| stage != LifecycleStage::InitialState);
        if restoration_required && failure.restoration.as_ref() == &RestorationOutcome::NotRequired
        {
            failure.restoration = Box::new(restoration_not_reviewed());
        } else if !restoration_required
            && matches!(
                failure.restoration.as_ref(),
                RestorationOutcome::BlockedUntrusted { reason }
                    if reason == "lifecycle_restoration_not_reviewed"
            )
        {
            failure.restoration = Box::new(RestorationOutcome::NotRequired);
        }
    }
    WorkerResult {
        disposition: WorkerDisposition::Failed,
        completed_stage,
        failure: Some(failure),
        process_tree_settled,
        private_evidence: Vec::new(),
        sanitized_export: None,
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
    fn service_lifecycle_uses_only_production_provisioner_verbs() {
        assert_eq!(SERVICE_STOP_ARGUMENTS, "--provision prepare-upgrade");
        assert_eq!(SERVICE_START_ARGUMENTS, "--provision install");
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

    #[test]
    fn failure_restoration_is_normalized_from_completed_mutation_state() {
        let before_mutation = failed(
            None,
            evidence_write_failure("initial_evidence_failed", "write_failed".to_string()),
            true,
        );
        assert_eq!(
            before_mutation
                .failure
                .expect("failure")
                .restoration
                .as_ref(),
            &RestorationOutcome::NotRequired
        );

        let after_mutation = failed(
            Some(LifecycleStage::FinalRepair),
            controller_failure("controller_failed".to_string()),
            true,
        );
        assert!(matches!(
            after_mutation
                .failure
                .expect("failure")
                .restoration
                .as_ref(),
            RestorationOutcome::BlockedUntrusted { reason }
                if reason == "lifecycle_restoration_not_reviewed"
        ));
    }
}
