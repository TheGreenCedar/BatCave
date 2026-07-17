use super::desktop::DesktopWindow;
use super::native::{
    capture_elevated_machine_snapshot, open_allowlisted_legacy_cli, open_installed_service,
    open_installed_uninstaller, parse_sha256, require_allowlisted_elevated_preflight,
    require_elevated_crashed_candidate, require_elevated_installed_candidate,
    require_elevated_stopped_candidate, require_elevated_total_product_absence,
    restore_allowlisted_legacy_cli, DesktopProcess, ElevatedMachineSnapshot, ExecuteFailure,
    OwnedFile, PeerBinding, PipeConnection, ProcessTerminal, ProcessTerminalSnapshot,
    ProtectedEvidenceRoot,
};
use super::private_evidence::{
    write_desktop_packet, write_machine_packet, write_service_crash_packet,
    write_upgrade_rollback_packet,
};
use crate::windows_lifecycle_proof_contract::{
    validate_envelope, AbortReason, DesktopPhase, DesktopPhaseDisposition, DesktopPhaseObservation,
    DesktopPhaseResult, DesktopSecondInstanceObservation, Envelope, LifecycleStage, ParentMessage,
    ProofPlan, RestorationOutcome, SequenceGate, WorkerAbort, WorkerCheckpoint, WorkerDisposition,
    WorkerFailure, WorkerFailureKind, WorkerMessage, WorkerResult,
};
use serde::Serialize;
use std::path::Path;
use std::time::Duration;

const CONTROLLER_READY: bool = false;
const MUTATION_FAILURE_SCHEMA: &str = "batcave.windows-lifecycle.mutation-failure.v1";
const INSTALLER_TIMEOUT: Duration = Duration::from_secs(10 * 60);
const UNINSTALLER_TIMEOUT: Duration = Duration::from_secs(10 * 60);
const SERVICE_OPERATION_TIMEOUT: Duration = Duration::from_secs(2 * 60);
const DESKTOP_PHASE_TIMEOUT: Duration = Duration::from_secs(8 * 60);
const FINAL_REPAIR_ARGUMENTS: &str = "/S /UPDATE";
const BASELINE_INSTALL_ARGUMENTS: &str = "/S";
const DIRECT_UNINSTALL_ARGUMENTS: &str = r"/S _?=C:\Program Files\BatCave Monitor";
const SERVICE_STOP_ARGUMENTS: &str = "--provision prepare-upgrade";
const SERVICE_START_ARGUMENTS: &str = "--provision install";

struct WorkerExecutionFailure {
    completed_stage: Option<LifecycleStage>,
    failure: WorkerFailure,
    process_tree_settled: bool,
    abort_reason: Option<AbortReason>,
}

impl From<(Option<LifecycleStage>, WorkerFailure, bool)> for WorkerExecutionFailure {
    fn from(
        (completed_stage, failure, process_tree_settled): (
            Option<LifecycleStage>,
            WorkerFailure,
            bool,
        ),
    ) -> Self {
        Self {
            completed_stage,
            failure,
            process_tree_settled,
            abort_reason: None,
        }
    }
}

impl
    From<(
        Option<LifecycleStage>,
        WorkerFailure,
        bool,
        Option<AbortReason>,
    )> for WorkerExecutionFailure
{
    fn from(
        (completed_stage, failure, process_tree_settled, abort_reason): (
            Option<LifecycleStage>,
            WorkerFailure,
            bool,
            Option<AbortReason>,
        ),
    ) -> Self {
        Self {
            completed_stage,
            failure,
            process_tree_settled,
            abort_reason,
        }
    }
}

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
    plan: &ProofPlan,
) -> Result<DesktopPhaseResult, String> {
    let expected_monitor_sha256 = match phase {
        DesktopPhase::BaselinePrimary | DesktopPhase::BaselineSecondInstance => {
            plan.baseline.monitor_sha256.as_str()
        }
        DesktopPhase::FinalPrimary
        | DesktopPhase::FinalMissingService
        | DesktopPhase::FinalStoppedService
        | DesktopPhase::FinalIncompatibleService => plan.final_candidate.monitor_sha256.as_str(),
    };
    let mut primary = match DesktopProcess::launch(expected_monitor_sha256, "desktop_primary") {
        Ok(primary) => primary,
        Err(failure) => {
            return Ok(failed_desktop_phase(
                phase,
                failure.reason,
                failure.process_tree_settled,
            ))
        }
    };

    let outcome = (|| -> Result<DesktopPhaseObservation, DesktopPhaseFailure> {
        let window =
            DesktopWindow::open(&primary.observation()).map_err(settled_desktop_failure)?;
        let mut allowed_process_ids = primary
            .current_job_process_ids()
            .map_err(settled_desktop_failure)?
            .into_iter()
            .collect();
        let mut visible = window
            .read_visible(phase, &allowed_process_ids)
            .map_err(settled_desktop_failure)?;
        let second_instance = if phase == DesktopPhase::BaselineSecondInstance {
            let service_instance_id_before =
                visible.service_instance_id.clone().ok_or_else(|| {
                    settled_desktop_failure(
                        "lifecycle_desktop_service_instance_before_missing".to_string(),
                    )
                })?;
            let mut attempted =
                DesktopProcess::launch(expected_monitor_sha256, "desktop_second_instance")
                    .map_err(|failure| DesktopPhaseFailure {
                        reason: failure.reason,
                        process_tree_settled: failure.process_tree_settled,
                    })?;
            let attempted_process = attempted.observation();
            let terminal_exit_code = match attempted.wait_for_clean_exit(None) {
                Ok(exit_code) => exit_code,
                Err(reason) => {
                    let (reason, process_tree_settled) = combine_desktop_settlement(
                        reason,
                        attempted.terminate_and_settle("desktop_second_instance_failure"),
                    );
                    return Err(DesktopPhaseFailure {
                        reason,
                        process_tree_settled,
                    });
                }
            };
            if terminal_exit_code != 0 {
                return Err(settled_desktop_failure(format!(
                    "lifecycle_desktop_second_instance_exit_code_{terminal_exit_code}"
                )));
            }
            let primary_observation = primary.observation();
            window
                .wait_for_primary_focus(&primary_observation)
                .map_err(settled_desktop_failure)?;
            allowed_process_ids = primary
                .current_job_process_ids()
                .map_err(settled_desktop_failure)?
                .into_iter()
                .collect();
            visible = window
                .read_visible(phase, &allowed_process_ids)
                .map_err(settled_desktop_failure)?;
            let service_instance_id_after =
                visible.service_instance_id.clone().ok_or_else(|| {
                    settled_desktop_failure(
                        "lifecycle_desktop_service_instance_after_missing".to_string(),
                    )
                })?;
            Some(DesktopSecondInstanceObservation {
                attempted_process,
                terminal_exit_code,
                process_tree_settled: true,
                focused_primary_process_id: primary_observation.process_id,
                focused_primary_started_at_100ns: primary_observation.started_at_100ns,
                service_instance_id_before,
                service_instance_id_after,
            })
        } else {
            None
        };
        let process_tree = primary.process_tree().map_err(settled_desktop_failure)?;
        let collector_runtime = super::native::observe_desktop_collector_runtime(&primary)
            .map_err(settled_desktop_failure)?;
        let desktop = primary.observation();
        window.close().map_err(settled_desktop_failure)?;
        let exit_code = primary
            .wait_for_clean_exit(Some(&process_tree))
            .map_err(settled_desktop_failure)?;
        if exit_code != 0 {
            return Err(settled_desktop_failure(format!(
                "lifecycle_desktop_primary_exit_code_{exit_code}"
            )));
        }
        Ok(DesktopPhaseObservation {
            desktop,
            process_tree: process_tree.observations(),
            webview_process_ids: process_tree.webview_process_ids(),
            second_instance,
            collector_runtime,
            visible,
        })
    })();

    match outcome {
        Ok(observation) => Ok(DesktopPhaseResult {
            phase,
            disposition: DesktopPhaseDisposition::Passed,
            process_tree_settled: true,
            observation: Some(observation),
            failure_reason: None,
        }),
        Err(failure) => {
            let (reason, primary_settled) = combine_desktop_settlement(
                failure.reason,
                primary.terminate_and_settle("desktop_primary_failure"),
            );
            Ok(failed_desktop_phase(
                phase,
                reason,
                failure.process_tree_settled && primary_settled,
            ))
        }
    }
}

struct DesktopPhaseFailure {
    reason: String,
    process_tree_settled: bool,
}

fn settled_desktop_failure(reason: String) -> DesktopPhaseFailure {
    DesktopPhaseFailure {
        reason,
        process_tree_settled: true,
    }
}

fn combine_desktop_settlement(
    primary: String,
    settlement: Result<(), super::native::DesktopSettlementFailure>,
) -> (String, bool) {
    match settlement {
        Ok(()) => (primary, true),
        Err(settlement) => (
            format!("{primary}|{}", settlement.reason),
            settlement.process_tree_settled,
        ),
    }
}

fn failed_desktop_phase(
    phase: DesktopPhase,
    reason: String,
    process_tree_settled: bool,
) -> DesktopPhaseResult {
    DesktopPhaseResult {
        phase,
        disposition: DesktopPhaseDisposition::Failed,
        process_tree_settled,
        observation: None,
        failure_reason: Some(bounded_desktop_reason(&reason)),
    }
}

fn bounded_desktop_reason(reason: &str) -> String {
    let mut bounded = reason
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, ':' | '_' | '-' | '.') {
                character
            } else {
                '_'
            }
        })
        .take(192)
        .collect::<String>();
    if bounded.is_empty() {
        bounded.push_str("lifecycle_desktop_phase_failed");
    }
    bounded
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
    pub(super) outbound_gate: &'a mut SequenceGate,
    pub(super) inbound_gate: &'a mut SequenceGate,
    pub(super) controller_bindings: &'a [PeerBinding],
}

struct AuthenticatedWorkerTransport<'a> {
    pipe: &'a mut PipeConnection,
    nonce: &'a str,
    outbound_gate: &'a mut SequenceGate,
    inbound_gate: &'a mut SequenceGate,
}

pub(super) fn execute_worker(context: WorkerContext<'_>) -> WorkerResult {
    if let Err(failure) = require_controller_ready() {
        return failed(None, None, None, controller_failure(failure), true);
    }
    let evidence_identity = context.evidence.identity();
    let mut last_authenticated_checkpoint = None;
    let result = execute_worker_inner(context, &mut last_authenticated_checkpoint);
    match result {
        Ok(completed_stage) => failed(
            Some(completed_stage),
            last_authenticated_checkpoint,
            None,
            controller_failure("lifecycle_parent_export_pending".to_string()),
            true,
        ),
        Err(failure) => failed(
            failure.completed_stage,
            last_authenticated_checkpoint,
            failure.abort_reason.map(|reason| WorkerAbort {
                reason,
                last_authenticated_checkpoint,
                evidence_root_identity: evidence_identity,
            }),
            failure.failure,
            failure.process_tree_settled,
        ),
    }
}

pub(super) fn abort_after_result(
    original: &WorkerResult,
    reason: AbortReason,
    evidence: &ProtectedEvidenceRoot,
    controller_bindings: &[PeerBinding],
) -> WorkerResult {
    if let Some(result) = preserve_failed_abort_result(original, reason, evidence.identity()) {
        return result;
    }
    let stage = original
        .completed_stage
        .unwrap_or(LifecycleStage::InitialState);
    let last_authenticated_checkpoint = original.last_authenticated_checkpoint;
    let failure = parent_abort_execution_failure(
        stage,
        reason,
        last_authenticated_checkpoint,
        evidence,
        controller_bindings,
    );
    failed(
        failure.completed_stage,
        last_authenticated_checkpoint,
        failure.abort_reason.map(|reason| WorkerAbort {
            reason,
            last_authenticated_checkpoint,
            evidence_root_identity: evidence.identity(),
        }),
        failure.failure,
        failure.process_tree_settled,
    )
}

fn preserve_failed_abort_result(
    original: &WorkerResult,
    reason: AbortReason,
    evidence_root_identity: crate::windows_lifecycle_proof_contract::EvidenceRootIdentity,
) -> Option<WorkerResult> {
    original.failure.as_ref()?;
    let mut result = original.clone();
    result.abort = Some(WorkerAbort {
        reason,
        last_authenticated_checkpoint: original.last_authenticated_checkpoint,
        evidence_root_identity,
    });
    Some(result)
}

fn execute_worker_inner(
    context: WorkerContext<'_>,
    last_authenticated_checkpoint: &mut Option<WorkerCheckpoint>,
) -> Result<LifecycleStage, WorkerExecutionFailure> {
    let WorkerContext {
        plan,
        repo_root: _repo_root,
        baseline,
        final_candidate,
        incompatible_service_fixture,
        rollback_failing_service_fixture,
        evidence,
        pipe,
        nonce,
        outbound_gate,
        inbound_gate,
        controller_bindings,
    } = context;
    let mut transport = AuthenticatedWorkerTransport {
        pipe,
        nonce,
        outbound_gate,
        inbound_gate,
    };
    let initial = capture_elevated_machine_snapshot(controller_bindings);
    write_machine_packet(evidence, "initial-state.private.json", &initial).map_err(|failure| {
        (
            None,
            evidence_write_failure("lifecycle_initial_state_evidence_incomplete", failure),
            true,
            None,
        )
    })?;
    require_allowlisted_elevated_preflight(&initial, plan)
        .map_err(|failure| (None, controller_failure(failure), true, None))?;
    authenticated_checkpoint(
        LifecycleStage::InitialState,
        evidence,
        &mut transport,
        last_authenticated_checkpoint,
        controller_bindings,
    )?;

    let historical_cli = open_allowlisted_legacy_cli(plan).map_err(|failure| {
        (
            Some(LifecycleStage::InitialState),
            controller_failure(failure),
            true,
            None,
        )
    })?;
    let historical_cli_copy = historical_cli
        .copy_to(
            &evidence.root().join("historical-cli.exe"),
            "historical_cli_copy",
        )
        .map_err(|failure| {
            (
                Some(LifecycleStage::InitialState),
                controller_failure(failure),
                true,
                None,
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
                None,
            )
        })?;
    let final_repair_state = execute_mutation(
        evidence,
        controller_bindings,
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
    .map_err(|(failure, settled)| (Some(LifecycleStage::InitialState), failure, settled, None))?;
    write_machine_packet(
        evidence,
        "final-repair-state.private.json",
        &final_repair_state,
    )
    .map_err(|failure| {
        (
            Some(LifecycleStage::FinalRepair),
            evidence_write_failure("lifecycle_final_repair_evidence_incomplete", failure),
            true,
            None,
        )
    })?;
    authenticated_checkpoint(
        LifecycleStage::FinalRepair,
        evidence,
        &mut transport,
        last_authenticated_checkpoint,
        controller_bindings,
    )?;
    authenticated_desktop_phase(
        LifecycleStage::FinalRepair,
        DesktopPhase::FinalPrimary,
        plan,
        evidence,
        &mut transport,
        last_authenticated_checkpoint,
        controller_bindings,
        false,
        |snapshot| {
            require_elevated_installed_candidate(
                snapshot,
                &plan.final_candidate,
                true,
                "final_primary_desktop",
            )
        },
    )?;

    let installed_uninstaller =
        open_installed_uninstaller(&plan.final_candidate).map_err(|failure| {
            (
                Some(LifecycleStage::FinalRepair),
                controller_failure(failure),
                true,
                None,
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
                None,
            )
        })?;
    drop(installed_uninstaller);
    let initial_uninstall_state = execute_mutation(
        evidence,
        controller_bindings,
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
    .map_err(|(failure, settled)| (Some(LifecycleStage::FinalRepair), failure, settled, None))?;
    write_machine_packet(
        evidence,
        "initial-uninstall-state.private.json",
        &initial_uninstall_state,
    )
    .map_err(|failure| {
        (
            Some(LifecycleStage::InitialUninstall),
            evidence_write_failure("lifecycle_initial_uninstall_evidence_incomplete", failure),
            true,
            None,
        )
    })?;
    authenticated_checkpoint(
        LifecycleStage::InitialUninstall,
        evidence,
        &mut transport,
        last_authenticated_checkpoint,
        controller_bindings,
    )?;

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
                None,
            )
        })?;
    let baseline_install_state = execute_mutation(
        evidence,
        controller_bindings,
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
    .map_err(|(failure, settled)| {
        (
            Some(LifecycleStage::InitialUninstall),
            failure,
            settled,
            None,
        )
    })?;
    write_machine_packet(
        evidence,
        "baseline-install-state.private.json",
        &baseline_install_state,
    )
    .map_err(|failure| {
        (
            Some(LifecycleStage::BaselineInstall),
            evidence_write_failure("lifecycle_baseline_install_evidence_incomplete", failure),
            true,
            None,
        )
    })?;
    authenticated_checkpoint(
        LifecycleStage::BaselineInstall,
        evidence,
        &mut transport,
        last_authenticated_checkpoint,
        controller_bindings,
    )?;
    for phase in [
        DesktopPhase::BaselinePrimary,
        DesktopPhase::BaselineSecondInstance,
    ] {
        authenticated_desktop_phase(
            LifecycleStage::BaselineInstall,
            phase,
            plan,
            evidence,
            &mut transport,
            last_authenticated_checkpoint,
            controller_bindings,
            false,
            |snapshot| {
                require_elevated_installed_candidate(
                    snapshot,
                    &plan.baseline,
                    false,
                    "baseline_desktop",
                )
            },
        )?;
    }

    let baseline_stop_service = open_installed_service(&plan.baseline).map_err(|failure| {
        (
            Some(LifecycleStage::BaselineInstall),
            controller_failure(failure),
            true,
        )
    })?;
    let baseline_restart_stopped = execute_mutation(
        evidence,
        controller_bindings,
        "baseline-restart-failure.private.json",
        LifecycleStage::BaselineRestart,
        &baseline_install_state,
        &baseline_stop_service,
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
    drop(baseline_stop_service);
    write_machine_packet(
        evidence,
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
    let baseline_start_service = open_installed_service(&plan.baseline).map_err(|failure| {
        (
            Some(LifecycleStage::BaselineInstall),
            controller_failure(failure),
            true,
        )
    })?;
    let baseline_restart_state = execute_mutation(
        evidence,
        controller_bindings,
        "baseline-restart-failure.private.json",
        LifecycleStage::BaselineRestart,
        &baseline_restart_stopped,
        &baseline_start_service,
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
    drop(baseline_start_service);
    write_machine_packet(
        evidence,
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
    authenticated_checkpoint(
        LifecycleStage::BaselineRestart,
        evidence,
        &mut transport,
        last_authenticated_checkpoint,
        controller_bindings,
    )?;

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
                    controller_bindings,
                    LifecycleStage::BaselineCrashRecovery,
                    "baseline-crash-recovery-failure.private.json",
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
                )
                    .into());
            }
        };
    let baseline_crashed_state = capture_elevated_machine_snapshot(controller_bindings);
    if let Err(reason) = require_elevated_crashed_candidate(
        &baseline_crashed_state,
        &plan.baseline,
        false,
        "baseline_crash",
    ) {
        let failure = write_service_crash_failure(
            evidence,
            controller_bindings,
            LifecycleStage::BaselineCrashRecovery,
            "baseline-crash-recovery-failure.private.json",
            &baseline_restart_state,
            &reason,
            true,
            ServiceCrashMutationObservation::Terminated(&termination),
        );
        return Err((Some(LifecycleStage::BaselineRestart), failure, true).into());
    }
    write_service_crash_packet(
        evidence,
        "baseline-crashed-state.private.json",
        &baseline_crashed_state,
        &termination,
    )
    .map_err(|failure| {
        (
            Some(LifecycleStage::BaselineRestart),
            evidence_write_failure("lifecycle_baseline_crashed_evidence_incomplete", failure),
            true,
        )
    })?;
    let baseline_recovery_service = open_installed_service(&plan.baseline).map_err(|failure| {
        (
            Some(LifecycleStage::BaselineRestart),
            controller_failure(failure),
            true,
        )
    })?;
    let baseline_crash_recovery_state = execute_mutation(
        evidence,
        controller_bindings,
        "baseline-crash-recovery-failure.private.json",
        LifecycleStage::BaselineCrashRecovery,
        &baseline_crashed_state,
        &baseline_recovery_service,
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
    drop(baseline_recovery_service);
    write_machine_packet(
        evidence,
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
    authenticated_checkpoint(
        LifecycleStage::BaselineCrashRecovery,
        evidence,
        &mut transport,
        last_authenticated_checkpoint,
        controller_bindings,
    )?;

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
                controller_bindings,
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
            )
                .into());
        }
    };
    if let Err(reason) = rollback_failing_service_fixture
        .revalidate()
        .and_then(|_| rollback_fixture_copy.revalidate())
    {
        let failure = write_upgrade_rollback_failure(
            evidence,
            controller_bindings,
            "baseline-rollback-recovery-failure.private.json",
            &baseline_crash_recovery_state,
            &reason,
            true,
        );
        return Err((Some(LifecycleStage::BaselineCrashRecovery), failure, true).into());
    }
    let baseline_rollback_recovery_state = capture_elevated_machine_snapshot(controller_bindings);
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
            controller_bindings,
            "baseline-rollback-recovery-failure.private.json",
            &baseline_crash_recovery_state,
            &reason,
            true,
        );
        (Some(LifecycleStage::BaselineCrashRecovery), failure, true)
    })?;
    write_upgrade_rollback_packet(
        evidence,
        "baseline-rollback-recovery-state.private.json",
        &baseline_rollback_recovery_state,
        &rollback,
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
    authenticated_checkpoint(
        LifecycleStage::BaselineRollbackRecovery,
        evidence,
        &mut transport,
        last_authenticated_checkpoint,
        controller_bindings,
    )?;

    let restored_legacy_cli =
        restore_allowlisted_legacy_cli(&historical_cli_copy).map_err(|reason| {
            (
                Some(LifecycleStage::BaselineRollbackRecovery),
                controller_failure(reason),
                true,
            )
        })?;
    let legacy_residue_seeded_state = capture_elevated_machine_snapshot(controller_bindings);
    require_elevated_installed_candidate(
        &legacy_residue_seeded_state,
        &plan.baseline,
        false,
        "legacy_residue_seeded",
    )
    .and_then(|_| {
        require_legacy_cli_hash(
            &legacy_residue_seeded_state,
            &plan.allowlisted_start.legacy_cli_sha256,
            "legacy_residue_seeded",
        )
    })
    .map_err(|reason| {
        (
            Some(LifecycleStage::BaselineRollbackRecovery),
            controller_failure(reason),
            true,
        )
    })?;
    write_machine_packet(
        evidence,
        "legacy-residue-seeded-state.private.json",
        &legacy_residue_seeded_state,
    )
    .map_err(|error| {
        (
            Some(LifecycleStage::LegacyResidueSeeded),
            evidence_write_failure("lifecycle_legacy_residue_seeded_evidence_incomplete", error),
            true,
        )
    })?;
    drop(restored_legacy_cli);
    authenticated_checkpoint(
        LifecycleStage::LegacyResidueSeeded,
        evidence,
        &mut transport,
        last_authenticated_checkpoint,
        controller_bindings,
    )?;

    let final_upgrade_state = execute_mutation(
        evidence,
        controller_bindings,
        "final-upgrade-failure.private.json",
        LifecycleStage::FinalUpgrade,
        &legacy_residue_seeded_state,
        &final_copy,
        FINAL_REPAIR_ARGUMENTS,
        INSTALLER_TIMEOUT,
        "final_upgrade",
        "lifecycle_final_upgrade_failed",
        |snapshot| {
            require_elevated_installed_candidate(
                snapshot,
                &plan.final_candidate,
                true,
                "final_upgrade",
            )
        },
    )
    .map_err(|(failure, settled)| (Some(LifecycleStage::LegacyResidueSeeded), failure, settled))?;
    write_machine_packet(
        evidence,
        "final-upgrade-state.private.json",
        &final_upgrade_state,
    )
    .map_err(|error| {
        (
            Some(LifecycleStage::FinalUpgrade),
            evidence_write_failure("lifecycle_final_upgrade_evidence_incomplete", error),
            true,
        )
    })?;
    authenticated_checkpoint(
        LifecycleStage::FinalUpgrade,
        evidence,
        &mut transport,
        last_authenticated_checkpoint,
        controller_bindings,
    )?;

    let final_stop_service = open_installed_service(&plan.final_candidate).map_err(|reason| {
        (
            Some(LifecycleStage::FinalUpgrade),
            controller_failure(reason),
            true,
        )
    })?;
    let final_restart_stopped = execute_mutation(
        evidence,
        controller_bindings,
        "final-restart-failure.private.json",
        LifecycleStage::FinalRestart,
        &final_upgrade_state,
        &final_stop_service,
        SERVICE_STOP_ARGUMENTS,
        SERVICE_OPERATION_TIMEOUT,
        "final_restart_stop",
        "lifecycle_final_restart_stop_failed",
        |snapshot| {
            require_elevated_stopped_candidate(
                snapshot,
                &plan.final_candidate,
                true,
                "final_restart_stop",
            )
        },
    )
    .map_err(|(failure, settled)| (Some(LifecycleStage::FinalUpgrade), failure, settled))?;
    drop(final_stop_service);
    write_machine_packet(
        evidence,
        "final-restart-stopped-state.private.json",
        &final_restart_stopped,
    )
    .map_err(|error| {
        (
            Some(LifecycleStage::FinalUpgrade),
            evidence_write_failure("lifecycle_final_restart_stopped_evidence_incomplete", error),
            true,
        )
    })?;
    let final_start_service = open_installed_service(&plan.final_candidate).map_err(|reason| {
        (
            Some(LifecycleStage::FinalUpgrade),
            controller_failure(reason),
            true,
        )
    })?;
    let final_restart_state = execute_mutation(
        evidence,
        controller_bindings,
        "final-restart-failure.private.json",
        LifecycleStage::FinalRestart,
        &final_restart_stopped,
        &final_start_service,
        SERVICE_START_ARGUMENTS,
        SERVICE_OPERATION_TIMEOUT,
        "final_restart_start",
        "lifecycle_final_restart_start_failed",
        |snapshot| {
            require_elevated_installed_candidate(
                snapshot,
                &plan.final_candidate,
                true,
                "final_restart_start",
            )
        },
    )
    .map_err(|(failure, settled)| (Some(LifecycleStage::FinalUpgrade), failure, settled))?;
    drop(final_start_service);
    write_machine_packet(
        evidence,
        "final-restart-state.private.json",
        &final_restart_state,
    )
    .map_err(|error| {
        (
            Some(LifecycleStage::FinalRestart),
            evidence_write_failure("lifecycle_final_restart_evidence_incomplete", error),
            true,
        )
    })?;
    authenticated_checkpoint(
        LifecycleStage::FinalRestart,
        evidence,
        &mut transport,
        last_authenticated_checkpoint,
        controller_bindings,
    )?;

    require_running_service(&final_restart_state, "final_crash_before").map_err(|reason| {
        (
            Some(LifecycleStage::FinalRestart),
            controller_failure(reason),
            true,
        )
    })?;
    let expected_final_service_sha256 =
        parse_sha256(&plan.final_candidate.service_sha256, "final_service").map_err(|reason| {
            (
                Some(LifecycleStage::FinalRestart),
                controller_failure(reason),
                true,
            )
        })?;
    let final_termination =
        match crate::collector_service::windows_provisioner::terminate_running_service_for_proof(
            expected_final_service_sha256,
        ) {
            Ok(termination) => termination,
            Err(failure) => {
                let failure = write_service_crash_failure(
                    evidence,
                    controller_bindings,
                    LifecycleStage::FinalCrashRecovery,
                    "final-crash-recovery-failure.private.json",
                    &final_restart_state,
                    &failure.reason,
                    failure.service_settled,
                    ServiceCrashMutationObservation::Failed(&failure),
                );
                let process_tree_settled = failure.kind != WorkerFailureKind::ProcessSettlement;
                return Err((
                    Some(LifecycleStage::FinalRestart),
                    failure,
                    process_tree_settled,
                )
                    .into());
            }
        };
    let final_crashed_state = capture_elevated_machine_snapshot(controller_bindings);
    if let Err(reason) = require_elevated_crashed_candidate(
        &final_crashed_state,
        &plan.final_candidate,
        true,
        "final_crash",
    ) {
        let failure = write_service_crash_failure(
            evidence,
            controller_bindings,
            LifecycleStage::FinalCrashRecovery,
            "final-crash-recovery-failure.private.json",
            &final_restart_state,
            &reason,
            true,
            ServiceCrashMutationObservation::Terminated(&final_termination),
        );
        return Err((Some(LifecycleStage::FinalRestart), failure, true).into());
    }
    write_service_crash_packet(
        evidence,
        "final-crashed-state.private.json",
        &final_crashed_state,
        &final_termination,
    )
    .map_err(|error| {
        (
            Some(LifecycleStage::FinalRestart),
            evidence_write_failure("lifecycle_final_crashed_evidence_incomplete", error),
            true,
        )
    })?;
    let final_recovery_service =
        open_installed_service(&plan.final_candidate).map_err(|reason| {
            (
                Some(LifecycleStage::FinalRestart),
                controller_failure(reason),
                true,
            )
        })?;
    let final_crash_recovery_state = execute_mutation(
        evidence,
        controller_bindings,
        "final-crash-recovery-failure.private.json",
        LifecycleStage::FinalCrashRecovery,
        &final_crashed_state,
        &final_recovery_service,
        SERVICE_START_ARGUMENTS,
        SERVICE_OPERATION_TIMEOUT,
        "final_crash_recovery",
        "lifecycle_final_crash_recovery_failed",
        |snapshot| {
            require_elevated_installed_candidate(
                snapshot,
                &plan.final_candidate,
                true,
                "final_crash_recovery",
            )?;
            require_running_service(snapshot, "final_crash_recovery").map(|_| ())
        },
    )
    .map_err(|(failure, settled)| (Some(LifecycleStage::FinalRestart), failure, settled))?;
    drop(final_recovery_service);
    write_machine_packet(
        evidence,
        "final-crash-recovery-state.private.json",
        &final_crash_recovery_state,
    )
    .map_err(|error| {
        (
            Some(LifecycleStage::FinalCrashRecovery),
            evidence_write_failure("lifecycle_final_crash_recovery_evidence_incomplete", error),
            true,
        )
    })?;
    authenticated_checkpoint(
        LifecycleStage::FinalCrashRecovery,
        evidence,
        &mut transport,
        last_authenticated_checkpoint,
        controller_bindings,
    )?;

    let installed_final_service =
        open_installed_service(&plan.final_candidate).map_err(|reason| {
            (
                Some(LifecycleStage::FinalCrashRecovery),
                controller_failure(reason),
                true,
            )
        })?;
    let retained_final_service = installed_final_service
        .copy_to(
            &evidence.root().join("final-service.exe"),
            "final_service_copy",
        )
        .map_err(|reason| {
            (
                Some(LifecycleStage::FinalCrashRecovery),
                controller_failure(reason),
                true,
            )
        })?;
    drop(installed_final_service);
    let final_service_bytes = retained_final_service
        .read_all_exact("final_service")
        .map_err(|reason| {
            (
                Some(LifecycleStage::FinalCrashRecovery),
                controller_failure(reason),
                true,
            )
        })?;

    let missing = crate::collector_service::windows_provisioner::with_missing_service_for_proof(
        &final_service_bytes,
        expected_final_service_sha256,
        || {
            let state = capture_elevated_machine_snapshot(controller_bindings);
            super::native::require_elevated_desktop_only_candidate(
                &state,
                &plan.final_candidate,
                "final_missing_service",
            )
            .map_err(|reason| {
                WorkerExecutionFailure::from((
                    Some(LifecycleStage::FinalCrashRecovery),
                    controller_failure(reason),
                    true,
                ))
            })?;
            write_machine_packet(evidence, "final-missing-service-state.private.json", &state)
                .map_err(|error| {
                    WorkerExecutionFailure::from((
                        Some(LifecycleStage::FinalCrashRecovery),
                        evidence_write_failure(
                            "lifecycle_final_missing_service_evidence_incomplete",
                            error,
                        ),
                        true,
                    ))
                })?;
            authenticated_desktop_phase(
                LifecycleStage::FinalCrashRecovery,
                DesktopPhase::FinalMissingService,
                plan,
                evidence,
                &mut transport,
                last_authenticated_checkpoint,
                controller_bindings,
                true,
                |snapshot| {
                    super::native::require_elevated_desktop_only_candidate(
                        snapshot,
                        &plan.final_candidate,
                        "final_missing_service_desktop",
                    )
                },
            )
        },
        |failure| failure.process_tree_settled,
    )
    .map_err(|failure| {
        service_state_execution_failure(
            &final_crash_recovery_state,
            &failure.reason,
            failure.service_settled,
            plan,
            evidence,
            controller_bindings,
        )
    })?;
    let final_missing_service_restored = finish_fallback_transaction(
        missing.body,
        missing.restoration,
        &final_crash_recovery_state,
        plan,
        evidence,
        controller_bindings,
    )?;
    write_machine_packet(
        evidence,
        "final-missing-service-restored-state.private.json",
        &final_missing_service_restored,
    )
    .map_err(|error| {
        let mut failure = WorkerExecutionFailure::from((
            Some(LifecycleStage::FinalCrashRecovery),
            evidence_write_failure(
                "lifecycle_final_missing_service_restored_evidence_incomplete",
                error,
            ),
            true,
        ));
        record_fallback_restoration(
            &mut failure,
            None,
            &final_missing_service_restored,
            evidence,
        );
        failure
    })?;

    let stopped = crate::collector_service::windows_provisioner::with_stopped_service_for_proof(
        expected_final_service_sha256,
        || {
            let state = capture_elevated_machine_snapshot(controller_bindings);
            require_elevated_stopped_candidate(
                &state,
                &plan.final_candidate,
                true,
                "final_stopped_service",
            )
            .map_err(|reason| {
                WorkerExecutionFailure::from((
                    Some(LifecycleStage::FinalCrashRecovery),
                    controller_failure(reason),
                    true,
                ))
            })?;
            write_machine_packet(evidence, "final-stopped-service-state.private.json", &state)
                .map_err(|error| {
                    WorkerExecutionFailure::from((
                        Some(LifecycleStage::FinalCrashRecovery),
                        evidence_write_failure(
                            "lifecycle_final_stopped_service_evidence_incomplete",
                            error,
                        ),
                        true,
                    ))
                })?;
            authenticated_desktop_phase(
                LifecycleStage::FinalCrashRecovery,
                DesktopPhase::FinalStoppedService,
                plan,
                evidence,
                &mut transport,
                last_authenticated_checkpoint,
                controller_bindings,
                true,
                |snapshot| {
                    require_elevated_stopped_candidate(
                        snapshot,
                        &plan.final_candidate,
                        true,
                        "final_stopped_service_desktop",
                    )
                },
            )
        },
        |failure| failure.process_tree_settled,
    )
    .map_err(|failure| {
        service_state_execution_failure(
            &final_missing_service_restored,
            &failure.reason,
            failure.service_settled,
            plan,
            evidence,
            controller_bindings,
        )
    })?;
    let final_stopped_service_restored = finish_fallback_transaction(
        stopped.body,
        stopped.restoration,
        &final_missing_service_restored,
        plan,
        evidence,
        controller_bindings,
    )?;
    write_machine_packet(
        evidence,
        "final-stopped-service-restored-state.private.json",
        &final_stopped_service_restored,
    )
    .map_err(|error| {
        let mut failure = WorkerExecutionFailure::from((
            Some(LifecycleStage::FinalCrashRecovery),
            evidence_write_failure(
                "lifecycle_final_stopped_service_restored_evidence_incomplete",
                error,
            ),
            true,
        ));
        record_fallback_restoration(
            &mut failure,
            None,
            &final_stopped_service_restored,
            evidence,
        );
        failure
    })?;

    let incompatible_service_bytes = incompatible_service_fixture
        .read_all_exact("incompatible_service")
        .map_err(|reason| {
            (
                Some(LifecycleStage::FinalCrashRecovery),
                controller_failure(reason),
                true,
            )
        })?;
    let expected_incompatible_service_sha256 = parse_sha256(
        &plan.incompatible_service_fixture.sha256,
        "incompatible_service",
    )
    .map_err(|reason| {
        (
            Some(LifecycleStage::FinalCrashRecovery),
            controller_failure(reason),
            true,
        )
    })?;
    let incompatible =
        crate::collector_service::windows_provisioner::with_incompatible_service_for_proof(
            &final_service_bytes,
            expected_final_service_sha256,
            &incompatible_service_bytes,
            expected_incompatible_service_sha256,
            || {
                let state = capture_elevated_machine_snapshot(controller_bindings);
                require_incompatible_candidate(&state, plan, "final_incompatible_service")
                    .map_err(|reason| {
                        WorkerExecutionFailure::from((
                            Some(LifecycleStage::FinalCrashRecovery),
                            controller_failure(reason),
                            true,
                        ))
                    })?;
                write_machine_packet(
                    evidence,
                    "final-incompatible-service-state.private.json",
                    &state,
                )
                .map_err(|error| {
                    WorkerExecutionFailure::from((
                        Some(LifecycleStage::FinalCrashRecovery),
                        evidence_write_failure(
                            "lifecycle_final_incompatible_service_evidence_incomplete",
                            error,
                        ),
                        true,
                    ))
                })?;
                authenticated_desktop_phase(
                    LifecycleStage::FinalCrashRecovery,
                    DesktopPhase::FinalIncompatibleService,
                    plan,
                    evidence,
                    &mut transport,
                    last_authenticated_checkpoint,
                    controller_bindings,
                    true,
                    |snapshot| {
                        require_incompatible_candidate(
                            snapshot,
                            plan,
                            "final_incompatible_service_desktop",
                        )
                    },
                )
            },
            |failure| failure.process_tree_settled,
        )
        .map_err(|failure| {
            service_state_execution_failure(
                &final_stopped_service_restored,
                &failure.reason,
                failure.service_settled,
                plan,
                evidence,
                controller_bindings,
            )
        })?;
    let final_incompatible_service_restored = finish_fallback_transaction(
        incompatible.body,
        incompatible.restoration,
        &final_stopped_service_restored,
        plan,
        evidence,
        controller_bindings,
    )?;
    write_machine_packet(
        evidence,
        "final-incompatible-service-restored-state.private.json",
        &final_incompatible_service_restored,
    )
    .map_err(|error| {
        let mut failure = WorkerExecutionFailure::from((
            Some(LifecycleStage::FinalCrashRecovery),
            evidence_write_failure(
                "lifecycle_final_incompatible_service_restored_evidence_incomplete",
                error,
            ),
            true,
        ));
        record_fallback_restoration(
            &mut failure,
            None,
            &final_incompatible_service_restored,
            evidence,
        );
        failure
    })?;
    retained_final_service.revalidate().map_err(|reason| {
        (
            Some(LifecycleStage::FinalCrashRecovery),
            controller_failure(reason),
            true,
        )
    })?;
    authenticated_checkpoint(
        LifecycleStage::FinalFallbackStates,
        evidence,
        &mut transport,
        last_authenticated_checkpoint,
        controller_bindings,
    )?;

    let final_uninstall_state = execute_mutation(
        evidence,
        controller_bindings,
        "final-uninstall-failure.private.json",
        LifecycleStage::FinalUninstall,
        &final_incompatible_service_restored,
        &uninstaller_copy,
        DIRECT_UNINSTALL_ARGUMENTS,
        UNINSTALLER_TIMEOUT,
        "final_uninstall",
        "lifecycle_final_uninstall_failed",
        |snapshot| require_elevated_total_product_absence(snapshot, "final_uninstall"),
    )
    .map_err(|(failure, settled)| (Some(LifecycleStage::FinalFallbackStates), failure, settled))?;
    write_machine_packet(
        evidence,
        "final-uninstall-state.private.json",
        &final_uninstall_state,
    )
    .map_err(|error| {
        (
            Some(LifecycleStage::FinalUninstall),
            evidence_write_failure("lifecycle_final_uninstall_evidence_incomplete", error),
            true,
        )
    })?;
    authenticated_checkpoint(
        LifecycleStage::FinalUninstall,
        evidence,
        &mut transport,
        last_authenticated_checkpoint,
        controller_bindings,
    )?;

    Ok(LifecycleStage::FinalUninstall)
}

fn authenticated_checkpoint(
    completed_stage: LifecycleStage,
    evidence: &ProtectedEvidenceRoot,
    transport: &mut AuthenticatedWorkerTransport<'_>,
    last_authenticated_checkpoint: &mut Option<WorkerCheckpoint>,
    controller_bindings: &[PeerBinding],
) -> Result<(), WorkerExecutionFailure> {
    let checkpoint = WorkerCheckpoint {
        completed_stage,
        evidence_root_identity: evidence.identity(),
    };
    if super::send_worker_message(
        transport.pipe,
        transport.nonce,
        transport.outbound_gate,
        WorkerMessage::Checkpoint(checkpoint),
    )
    .is_err()
    {
        return Err(parent_abort_execution_failure(
            completed_stage,
            AbortReason::Disconnected,
            *last_authenticated_checkpoint,
            evidence,
            controller_bindings,
        ));
    }
    let response: Envelope<ParentMessage> = match transport.pipe.read_json(Duration::from_secs(30))
    {
        Ok(response) => response,
        Err(reason) => {
            return Err(parent_abort_execution_failure(
                completed_stage,
                abort_reason_for_transport_error(&reason),
                *last_authenticated_checkpoint,
                evidence,
                controller_bindings,
            ));
        }
    };
    if validate_envelope(&response, transport.nonce, transport.inbound_gate).is_err() {
        return Err(parent_abort_execution_failure(
            completed_stage,
            AbortReason::ProtocolViolation,
            *last_authenticated_checkpoint,
            evidence,
            controller_bindings,
        ));
    }
    match response.message {
        ParentMessage::CheckpointAccepted(accepted) if accepted == checkpoint => {
            *last_authenticated_checkpoint = Some(checkpoint);
            Ok(())
        }
        ParentMessage::Abort(reason) => Err(parent_abort_execution_failure(
            completed_stage,
            reason,
            *last_authenticated_checkpoint,
            evidence,
            controller_bindings,
        )),
        ParentMessage::Begin(_)
        | ParentMessage::CheckpointAccepted(_)
        | ParentMessage::DesktopPhaseComplete(_)
        | ParentMessage::EvidenceAccepted => Err(parent_abort_execution_failure(
            completed_stage,
            AbortReason::ProtocolViolation,
            *last_authenticated_checkpoint,
            evidence,
            controller_bindings,
        )),
    }
}

#[allow(clippy::too_many_arguments)]
fn authenticated_desktop_phase(
    completed_stage: LifecycleStage,
    phase: DesktopPhase,
    plan: &ProofPlan,
    evidence: &ProtectedEvidenceRoot,
    transport: &mut AuthenticatedWorkerTransport<'_>,
    last_authenticated_checkpoint: &mut Option<WorkerCheckpoint>,
    controller_bindings: &[PeerBinding],
    defer_restoration: bool,
    validate_machine: impl FnOnce(&ElevatedMachineSnapshot) -> Result<(), String>,
) -> Result<(), WorkerExecutionFailure> {
    if super::send_worker_message(
        transport.pipe,
        transport.nonce,
        transport.outbound_gate,
        WorkerMessage::RunDesktopPhase(phase),
    )
    .is_err()
    {
        return Err(parent_abort_execution_failure_for_desktop(
            completed_stage,
            AbortReason::Disconnected,
            *last_authenticated_checkpoint,
            evidence,
            controller_bindings,
            defer_restoration,
        ));
    }
    let response: Envelope<ParentMessage> = match transport.pipe.read_json(DESKTOP_PHASE_TIMEOUT) {
        Ok(response) => response,
        Err(reason) => {
            return Err(parent_abort_execution_failure_for_desktop(
                completed_stage,
                abort_reason_for_transport_error(&reason),
                *last_authenticated_checkpoint,
                evidence,
                controller_bindings,
                defer_restoration,
            ));
        }
    };
    if validate_envelope(&response, transport.nonce, transport.inbound_gate).is_err() {
        return Err(parent_abort_execution_failure_for_desktop(
            completed_stage,
            AbortReason::ProtocolViolation,
            *last_authenticated_checkpoint,
            evidence,
            controller_bindings,
            defer_restoration,
        ));
    }
    match response.message {
        ParentMessage::DesktopPhaseComplete(result) => {
            super::validate_requested_desktop_phase_result(phase, &result, plan).map_err(
                |reason| {
                    WorkerExecutionFailure::from((
                        Some(completed_stage),
                        controller_failure(reason),
                        true,
                    ))
                },
            )?;
            if let Some(failure) =
                desktop_execution_failure(completed_stage, &result, defer_restoration)
            {
                return Err(failure);
            }
            let machine = capture_elevated_machine_snapshot(controller_bindings);
            validate_machine(&machine).map_err(|reason| {
                WorkerExecutionFailure::from((
                    Some(completed_stage),
                    controller_failure(reason),
                    true,
                ))
            })?;
            write_desktop_packet(evidence, desktop_evidence_name(phase), &machine, &result)
                .map(|_| ())
                .map_err(|error| {
                    WorkerExecutionFailure::from((
                        Some(completed_stage),
                        evidence_write_failure("lifecycle_desktop_evidence_incomplete", error),
                        true,
                    ))
                })
        }
        ParentMessage::Abort(reason) => Err(parent_abort_execution_failure_for_desktop(
            completed_stage,
            reason,
            *last_authenticated_checkpoint,
            evidence,
            controller_bindings,
            defer_restoration,
        )),
        ParentMessage::Begin(_)
        | ParentMessage::CheckpointAccepted(_)
        | ParentMessage::EvidenceAccepted => Err(parent_abort_execution_failure_for_desktop(
            completed_stage,
            AbortReason::ProtocolViolation,
            *last_authenticated_checkpoint,
            evidence,
            controller_bindings,
            defer_restoration,
        )),
    }
}

fn desktop_execution_failure(
    completed_stage: LifecycleStage,
    result: &DesktopPhaseResult,
    fallback_transaction: bool,
) -> Option<WorkerExecutionFailure> {
    if result.disposition == DesktopPhaseDisposition::Passed {
        return None;
    }
    Some(WorkerExecutionFailure {
        completed_stage: Some(completed_stage),
        failure: WorkerFailure {
            kind: WorkerFailureKind::Controller,
            attempted_stage: fallback_transaction.then_some(LifecycleStage::FinalFallbackStates),
            reason: format!(
                "{}:{}",
                desktop_evidence_name(result.phase),
                result
                    .failure_reason
                    .as_deref()
                    .unwrap_or("lifecycle_desktop_phase_failed")
            ),
            evidence: None,
            evidence_error: None,
            restoration: Box::new(if result.process_tree_settled {
                RestorationOutcome::NotRequired
            } else {
                RestorationOutcome::BlockedUnsettled
            }),
        },
        process_tree_settled: result.process_tree_settled,
        abort_reason: None,
    })
}

pub(super) fn desktop_evidence_name(phase: DesktopPhase) -> &'static str {
    match phase {
        DesktopPhase::FinalPrimary => "final-primary-desktop.private.json",
        DesktopPhase::BaselinePrimary => "baseline-primary-desktop.private.json",
        DesktopPhase::BaselineSecondInstance => "baseline-second-instance-desktop.private.json",
        DesktopPhase::FinalMissingService => "final-missing-service-desktop.private.json",
        DesktopPhase::FinalStoppedService => "final-stopped-service-desktop.private.json",
        DesktopPhase::FinalIncompatibleService => "final-incompatible-service-desktop.private.json",
    }
}

fn abort_reason_for_transport_error(reason: &str) -> AbortReason {
    if reason.contains("timeout") {
        AbortReason::Timeout
    } else {
        AbortReason::Disconnected
    }
}

fn parent_abort_execution_failure(
    completed_stage: LifecycleStage,
    reason: AbortReason,
    last_authenticated_checkpoint: Option<WorkerCheckpoint>,
    evidence: &ProtectedEvidenceRoot,
    controller_bindings: &[PeerBinding],
) -> WorkerExecutionFailure {
    let packet = ParentAbortFailurePacket {
        schema_version: MUTATION_FAILURE_SCHEMA,
        reason,
        completed_stage,
        last_authenticated_checkpoint,
        evidence_root_identity: evidence.identity(),
        process_tree_settled: true,
        machine_after_attempt: capture_elevated_machine_snapshot(controller_bindings),
    };
    let (failure_evidence, evidence_error) = match evidence
        .write_json_new(super::parent_abort_leaf_for_stage(completed_stage), &packet)
    {
        Ok(receipt) => (Some(Box::new(receipt)), None),
        Err(error) => (None, Some(error)),
    };
    WorkerExecutionFailure {
        completed_stage: Some(completed_stage),
        failure: WorkerFailure {
            kind: WorkerFailureKind::ParentAbort,
            attempted_stage: None,
            reason: super::parent_abort_reason(reason).to_string(),
            evidence: failure_evidence,
            evidence_error,
            restoration: Box::new(parent_abort_restoration(
                completed_stage,
                reason,
                evidence,
                controller_bindings,
            )),
        },
        process_tree_settled: true,
        abort_reason: Some(reason),
    }
}

fn parent_abort_execution_failure_for_desktop(
    completed_stage: LifecycleStage,
    reason: AbortReason,
    last_authenticated_checkpoint: Option<WorkerCheckpoint>,
    evidence: &ProtectedEvidenceRoot,
    controller_bindings: &[PeerBinding],
    defer_restoration: bool,
) -> WorkerExecutionFailure {
    let packet = ParentAbortFailurePacket {
        schema_version: MUTATION_FAILURE_SCHEMA,
        reason,
        completed_stage,
        last_authenticated_checkpoint,
        evidence_root_identity: evidence.identity(),
        process_tree_settled: false,
        machine_after_attempt: capture_elevated_machine_snapshot(controller_bindings),
    };
    let (failure_evidence, evidence_error) = match evidence
        .write_json_new(super::parent_abort_leaf_for_stage(completed_stage), &packet)
    {
        Ok(receipt) => (Some(Box::new(receipt)), None),
        Err(error) => (None, Some(error)),
    };
    WorkerExecutionFailure {
        completed_stage: Some(completed_stage),
        failure: WorkerFailure {
            kind: WorkerFailureKind::ParentAbort,
            attempted_stage: defer_restoration.then_some(LifecycleStage::FinalFallbackStates),
            reason: super::parent_abort_reason(reason).to_string(),
            evidence: failure_evidence,
            evidence_error,
            restoration: Box::new(RestorationOutcome::BlockedUnsettled),
        },
        process_tree_settled: false,
        abort_reason: Some(reason),
    }
}

fn parent_abort_restoration(
    completed_stage: LifecycleStage,
    reason: AbortReason,
    evidence: &ProtectedEvidenceRoot,
    controller_bindings: &[PeerBinding],
) -> RestorationOutcome {
    if completed_stage == LifecycleStage::InitialState {
        return RestorationOutcome::NotRequired;
    }
    let restoration_reason = "lifecycle_parent_abort_restoration_not_reviewed".to_string();
    let packet = ParentAbortRestorationPacket {
        schema_version: MUTATION_FAILURE_SCHEMA,
        reason,
        completed_stage,
        restoration_reason: &restoration_reason,
        machine_after_attempt: capture_elevated_machine_snapshot(controller_bindings),
    };
    let Some(restoration_leaf) = super::restoration_leaf_for_stage(completed_stage) else {
        return RestorationOutcome::Failed {
            reason: restoration_reason,
            evidence: None,
            evidence_error: Some("lifecycle_parent_abort_restoration_leaf_missing".to_string()),
        };
    };
    match evidence.write_json_new(restoration_leaf, &packet) {
        Ok(receipt) => RestorationOutcome::Failed {
            reason: restoration_reason,
            evidence: Some(receipt),
            evidence_error: None,
        },
        Err(error) => RestorationOutcome::Failed {
            reason: restoration_reason,
            evidence: None,
            evidence_error: Some(error),
        },
    }
}

#[derive(Serialize)]
struct ParentAbortFailurePacket {
    schema_version: &'static str,
    reason: AbortReason,
    completed_stage: LifecycleStage,
    last_authenticated_checkpoint: Option<WorkerCheckpoint>,
    evidence_root_identity: crate::windows_lifecycle_proof_contract::EvidenceRootIdentity,
    process_tree_settled: bool,
    machine_after_attempt: ElevatedMachineSnapshot,
}

#[derive(Serialize)]
struct ParentAbortRestorationPacket<'a> {
    schema_version: &'static str,
    reason: AbortReason,
    completed_stage: LifecycleStage,
    restoration_reason: &'a str,
    machine_after_attempt: ElevatedMachineSnapshot,
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
struct ServiceStateFailurePacket<'a> {
    schema_version: &'static str,
    attempted_stage: LifecycleStage,
    reason: &'a str,
    service_settled: bool,
    machine_before_mutation: &'a ElevatedMachineSnapshot,
    machine_after_attempt: ElevatedMachineSnapshot,
}

#[derive(Serialize)]
struct StageRestorationPacket<'a> {
    schema_version: &'static str,
    stage: LifecycleStage,
    restored: bool,
    reason: Option<&'a str>,
    machine_after_attempt: &'a ElevatedMachineSnapshot,
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
    controller_bindings: &[PeerBinding],
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

    let post_settlement = capture_elevated_machine_snapshot(controller_bindings);
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

fn require_legacy_cli_hash(
    snapshot: &ElevatedMachineSnapshot,
    expected_sha256: &str,
    label: &str,
) -> Result<(), String> {
    match &snapshot.machine.legacy_cli {
        crate::windows_lifecycle_proof_contract::Observation::Present(file)
            if file.sha256 == expected_sha256 =>
        {
            Ok(())
        }
        _ => Err(format!("lifecycle_{label}_legacy_cli_identity_invalid")),
    }
}

fn require_incompatible_candidate(
    snapshot: &ElevatedMachineSnapshot,
    plan: &ProofPlan,
    label: &str,
) -> Result<(), String> {
    let mut candidate = plan.final_candidate.clone();
    candidate.service_sha256 = plan.incompatible_service_fixture.sha256.clone();
    require_elevated_installed_candidate(snapshot, &candidate, true, label)?;
    match &snapshot.machine.service_binary {
        crate::windows_lifecycle_proof_contract::Observation::Present(file)
            if file.size == plan.incompatible_service_fixture.size =>
        {
            Ok(())
        }
        _ => Err(format!("lifecycle_{label}_service_fixture_size_invalid")),
    }
}

#[allow(clippy::too_many_arguments)]
fn write_service_crash_failure(
    evidence: &ProtectedEvidenceRoot,
    controller_bindings: &[PeerBinding],
    attempted_stage: LifecycleStage,
    evidence_name: &'static str,
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
        machine_after_attempt: capture_elevated_machine_snapshot(controller_bindings),
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
    controller_bindings: &[PeerBinding],
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
        machine_after_attempt: capture_elevated_machine_snapshot(controller_bindings),
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

fn finish_fallback_transaction(
    body: Result<(), WorkerExecutionFailure>,
    restoration: crate::collector_service::windows_provisioner::ServiceStateRestorationOutcome,
    before: &ElevatedMachineSnapshot,
    plan: &ProofPlan,
    evidence: &ProtectedEvidenceRoot,
    controller_bindings: &[PeerBinding],
) -> Result<ElevatedMachineSnapshot, WorkerExecutionFailure> {
    use crate::collector_service::windows_provisioner::ServiceStateRestorationOutcome;

    let restored = capture_elevated_machine_snapshot(controller_bindings);
    if matches!(
        restoration,
        ServiceStateRestorationOutcome::BlockedUnsettled
    ) {
        return match body {
            Ok(()) => Err(service_state_execution_failure(
                before,
                "lifecycle_fallback_restoration_blocked_unsettled",
                false,
                plan,
                evidence,
                controller_bindings,
            )),
            Err(mut failure) => {
                failure.failure.attempted_stage = Some(LifecycleStage::FinalFallbackStates);
                failure.process_tree_settled = false;
                failure.failure.restoration = Box::new(RestorationOutcome::BlockedUnsettled);
                Err(failure)
            }
        };
    }
    let restoration_error = match restoration {
        ServiceStateRestorationOutcome::Restored => require_elevated_installed_candidate(
            &restored,
            &plan.final_candidate,
            true,
            "fallback_restoration",
        )
        .err()
        .map(|reason| (reason, true)),
        ServiceStateRestorationOutcome::Failed(failure) => {
            Some((failure.reason, failure.service_settled))
        }
        ServiceStateRestorationOutcome::BlockedUnsettled => unreachable!(),
    };
    match (body, restoration_error) {
        (Ok(()), None) => Ok(restored),
        (Err(mut failure), None) => {
            failure.failure.attempted_stage = Some(LifecycleStage::FinalFallbackStates);
            if failure.process_tree_settled {
                record_fallback_restoration(&mut failure, None, &restored, evidence);
            }
            Err(failure)
        }
        (Err(mut failure), Some((reason, service_settled))) => {
            failure.failure.attempted_stage = Some(LifecycleStage::FinalFallbackStates);
            if failure.process_tree_settled && service_settled {
                record_fallback_restoration(&mut failure, Some(&reason), &restored, evidence);
            } else {
                failure.process_tree_settled = false;
                failure.failure.restoration = Box::new(RestorationOutcome::BlockedUnsettled);
            }
            Err(failure)
        }
        (Ok(()), Some((reason, service_settled))) => Err(service_state_execution_failure(
            before,
            &reason,
            service_settled,
            plan,
            evidence,
            controller_bindings,
        )),
    }
}

fn record_fallback_restoration(
    failure: &mut WorkerExecutionFailure,
    reason: Option<&str>,
    machine: &ElevatedMachineSnapshot,
    evidence: &ProtectedEvidenceRoot,
) {
    let stage = LifecycleStage::FinalFallbackStates;
    let name = "final-fallback-states-restoration.private.json";
    failure.failure.attempted_stage = Some(stage);
    let packet = StageRestorationPacket {
        schema_version: MUTATION_FAILURE_SCHEMA,
        stage,
        restored: reason.is_none(),
        reason,
        machine_after_attempt: machine,
    };
    match evidence.write_json_new(name, &packet) {
        Ok(receipt) if reason.is_none() => {
            *failure.failure.restoration = RestorationOutcome::Restored { evidence: receipt }
        }
        Ok(receipt) => {
            *failure.failure.restoration = RestorationOutcome::Failed {
                reason: "lifecycle_fallback_restoration_failed".to_string(),
                evidence: Some(receipt),
                evidence_error: None,
            }
        }
        Err(error) => {
            *failure.failure.restoration = RestorationOutcome::Failed {
                reason: if reason.is_some() {
                    "lifecycle_fallback_restoration_failed"
                } else {
                    "lifecycle_fallback_restoration_evidence_failed"
                }
                .to_string(),
                evidence: None,
                evidence_error: Some(error),
            }
        }
    }
}

fn service_state_execution_failure(
    before: &ElevatedMachineSnapshot,
    reason: &str,
    service_settled: bool,
    plan: &ProofPlan,
    evidence: &ProtectedEvidenceRoot,
    controller_bindings: &[PeerBinding],
) -> WorkerExecutionFailure {
    let after = capture_elevated_machine_snapshot(controller_bindings);
    let packet = ServiceStateFailurePacket {
        schema_version: MUTATION_FAILURE_SCHEMA,
        attempted_stage: LifecycleStage::FinalFallbackStates,
        reason,
        service_settled,
        machine_before_mutation: before,
        machine_after_attempt: after.clone(),
    };
    let (receipt, evidence_error) =
        match evidence.write_json_new("final-fallback-states-failure.private.json", &packet) {
            Ok(receipt) => (Some(receipt), None),
            Err(error) => (None, Some(error)),
        };
    let mut failure = WorkerExecutionFailure {
        completed_stage: Some(LifecycleStage::FinalCrashRecovery),
        failure: WorkerFailure {
            kind: if service_settled {
                if receipt.is_some() {
                    WorkerFailureKind::Mutation
                } else {
                    WorkerFailureKind::EvidenceWrite
                }
            } else {
                WorkerFailureKind::ProcessSettlement
            },
            attempted_stage: Some(LifecycleStage::FinalFallbackStates),
            reason: reason.to_string(),
            evidence: receipt.map(Box::new),
            evidence_error,
            restoration: Box::new(if service_settled {
                RestorationOutcome::BlockedUntrusted {
                    reason: "lifecycle_fallback_restoration_unproven".to_string(),
                }
            } else {
                RestorationOutcome::BlockedUnsettled
            }),
        },
        process_tree_settled: service_settled,
        abort_reason: None,
    };
    if service_settled {
        let restoration_error = require_elevated_installed_candidate(
            &after,
            &plan.final_candidate,
            true,
            "fallback_failure_restoration",
        )
        .err();
        record_fallback_restoration(&mut failure, restoration_error.as_deref(), &after, evidence);
    }
    failure
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
    last_authenticated_checkpoint: Option<WorkerCheckpoint>,
    abort: Option<WorkerAbort>,
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
        let parent_export_pending = completed_stage == Some(LifecycleStage::FinalUninstall)
            && last_authenticated_checkpoint.is_some_and(|checkpoint| {
                checkpoint.completed_stage == LifecycleStage::FinalUninstall
            })
            && failure.kind == WorkerFailureKind::Controller
            && failure.reason == "lifecycle_parent_export_pending";
        if restoration_required
            && !parent_export_pending
            && failure.restoration.as_ref() == &RestorationOutcome::NotRequired
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
        last_authenticated_checkpoint,
        abort,
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
    fn desktop_failure_preserves_phase_reason_and_settlement_truth() {
        for process_tree_settled in [true, false] {
            let result = DesktopPhaseResult {
                phase: DesktopPhase::FinalMissingService,
                disposition: DesktopPhaseDisposition::Failed,
                process_tree_settled,
                observation: None,
                failure_reason: Some("lifecycle_desktop_launch_failed".to_string()),
            };
            let failure =
                desktop_execution_failure(LifecycleStage::FinalCrashRecovery, &result, true)
                    .expect("failed desktop result");

            assert_eq!(
                failure.completed_stage,
                Some(LifecycleStage::FinalCrashRecovery)
            );
            assert_eq!(
                failure.failure.attempted_stage,
                Some(LifecycleStage::FinalFallbackStates)
            );
            assert_eq!(failure.failure.kind, WorkerFailureKind::Controller);
            assert_eq!(
                failure.failure.reason,
                "final-missing-service-desktop.private.json:lifecycle_desktop_launch_failed"
            );
            assert_eq!(failure.process_tree_settled, process_tree_settled);
            assert_eq!(
                failure.failure.restoration.as_ref(),
                if process_tree_settled {
                    &RestorationOutcome::NotRequired
                } else {
                    &RestorationOutcome::BlockedUnsettled
                }
            );
        }
    }

    #[test]
    fn post_result_abort_preserves_unsettled_failure_authority() {
        let evidence_root_identity =
            crate::windows_lifecycle_proof_contract::EvidenceRootIdentity {
                volume_serial: 7,
                file_index: 11,
            };
        let checkpoint = WorkerCheckpoint {
            completed_stage: LifecycleStage::InitialState,
            evidence_root_identity,
        };
        let original = WorkerResult {
            disposition: WorkerDisposition::Failed,
            completed_stage: Some(LifecycleStage::InitialState),
            last_authenticated_checkpoint: Some(checkpoint),
            abort: None,
            failure: Some(WorkerFailure {
                kind: WorkerFailureKind::ProcessSettlement,
                attempted_stage: Some(LifecycleStage::FinalRepair),
                reason: "lifecycle_final_repair_process_tree_unsettled".to_string(),
                evidence: Some(Box::new(
                    crate::windows_lifecycle_proof_contract::EvidenceReceipt {
                        name: "final-repair-failure.private.json".to_string(),
                        size: 128,
                        sha256: "a".repeat(64),
                    },
                )),
                evidence_error: None,
                restoration: Box::new(RestorationOutcome::BlockedUnsettled),
            }),
            process_tree_settled: false,
            private_evidence: Vec::new(),
            sanitized_export: None,
        };

        let aborted = preserve_failed_abort_result(
            &original,
            AbortReason::Disconnected,
            evidence_root_identity,
        )
        .expect("unsettled result");

        assert_eq!(aborted.failure, original.failure);
        assert!(!aborted.process_tree_settled);
        assert_eq!(
            aborted
                .failure
                .as_ref()
                .expect("failure")
                .restoration
                .as_ref(),
            &RestorationOutcome::BlockedUnsettled
        );
        assert_eq!(
            aborted.abort,
            Some(WorkerAbort {
                reason: AbortReason::Disconnected,
                last_authenticated_checkpoint: Some(checkpoint),
                evidence_root_identity,
            })
        );
    }

    #[test]
    fn post_result_abort_preserves_settled_failure_and_restoration_truth() {
        let evidence_root_identity =
            crate::windows_lifecycle_proof_contract::EvidenceRootIdentity {
                volume_serial: 7,
                file_index: 11,
            };
        let checkpoint = WorkerCheckpoint {
            completed_stage: LifecycleStage::InitialState,
            evidence_root_identity,
        };
        let failures = [
            WorkerFailure {
                kind: WorkerFailureKind::Mutation,
                attempted_stage: Some(LifecycleStage::FinalRepair),
                reason: "lifecycle_final_repair_failed".to_string(),
                evidence: Some(Box::new(
                    crate::windows_lifecycle_proof_contract::EvidenceReceipt {
                        name: "final-repair-failure.private.json".to_string(),
                        size: 128,
                        sha256: "a".repeat(64),
                    },
                )),
                evidence_error: None,
                restoration: Box::new(RestorationOutcome::Failed {
                    reason: "lifecycle_restoration_failed".to_string(),
                    evidence: Some(crate::windows_lifecycle_proof_contract::EvidenceReceipt {
                        name: "final-repair-restoration.private.json".to_string(),
                        size: 128,
                        sha256: "b".repeat(64),
                    }),
                    evidence_error: None,
                }),
            },
            WorkerFailure {
                kind: WorkerFailureKind::EvidenceWrite,
                attempted_stage: None,
                reason: "lifecycle_initial_state_evidence_incomplete".to_string(),
                evidence: None,
                evidence_error: Some("lifecycle_evidence_create_failed".to_string()),
                restoration: Box::new(RestorationOutcome::NotRequired),
            },
            WorkerFailure {
                kind: WorkerFailureKind::Controller,
                attempted_stage: None,
                reason: "lifecycle_controller_not_reviewed".to_string(),
                evidence: None,
                evidence_error: None,
                restoration: Box::new(RestorationOutcome::NotRequired),
            },
        ];

        for failure in failures {
            let original = WorkerResult {
                disposition: WorkerDisposition::Failed,
                completed_stage: Some(LifecycleStage::InitialState),
                last_authenticated_checkpoint: Some(checkpoint),
                abort: None,
                failure: Some(failure),
                process_tree_settled: true,
                private_evidence: Vec::new(),
                sanitized_export: None,
            };
            let aborted = preserve_failed_abort_result(
                &original,
                AbortReason::ReceiptValidation,
                evidence_root_identity,
            )
            .expect("failed result");

            assert_eq!(aborted.failure, original.failure);
            assert!(aborted.process_tree_settled);
            assert_eq!(
                aborted.abort,
                Some(WorkerAbort {
                    reason: AbortReason::ReceiptValidation,
                    last_authenticated_checkpoint: Some(checkpoint),
                    evidence_root_identity,
                })
            );
        }
    }

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
        assert_eq!(DESKTOP_PHASE_TIMEOUT, Duration::from_secs(8 * 60));
    }

    #[test]
    fn controller_remains_fail_closed_until_reviewed() {
        assert_eq!(
            require_controller_ready(),
            Err("lifecycle_controller_not_reviewed".to_string())
        );
    }

    #[test]
    fn desktop_settlement_preserves_exact_process_tree_state() {
        assert_eq!(
            combine_desktop_settlement("primary".to_string(), Ok(())),
            ("primary".to_string(), true)
        );
        assert_eq!(
            combine_desktop_settlement(
                "primary".to_string(),
                Err(super::super::native::DesktopSettlementFailure {
                    reason: "identity_revalidation_failed".to_string(),
                    process_tree_settled: true,
                }),
            ),
            ("primary|identity_revalidation_failed".to_string(), true)
        );
        assert_eq!(
            combine_desktop_settlement(
                "primary".to_string(),
                Err(super::super::native::DesktopSettlementFailure {
                    reason: "job_settlement_unproven".to_string(),
                    process_tree_settled: false,
                }),
            ),
            ("primary|job_settlement_unproven".to_string(), false)
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
            None,
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
            None,
            None,
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

        let root = crate::windows_lifecycle_proof_contract::EvidenceRootIdentity {
            volume_serial: 1,
            file_index: 1,
        };
        let final_checkpoint = WorkerCheckpoint {
            completed_stage: LifecycleStage::FinalUninstall,
            evidence_root_identity: root,
        };
        let exact_export_boundary = failed(
            Some(LifecycleStage::FinalUninstall),
            Some(final_checkpoint),
            None,
            controller_failure("lifecycle_parent_export_pending".to_string()),
            true,
        );
        assert_eq!(
            exact_export_boundary
                .failure
                .expect("failure")
                .restoration
                .as_ref(),
            &RestorationOutcome::NotRequired
        );

        for stale_checkpoint in [
            None,
            Some(WorkerCheckpoint {
                completed_stage: LifecycleStage::FinalFallbackStates,
                evidence_root_identity: root,
            }),
        ] {
            let forged_export_boundary = failed(
                Some(LifecycleStage::FinalUninstall),
                stale_checkpoint,
                None,
                controller_failure("lifecycle_parent_export_pending".to_string()),
                true,
            );
            assert!(matches!(
                forged_export_boundary
                    .failure
                    .expect("failure")
                    .restoration
                    .as_ref(),
                RestorationOutcome::BlockedUntrusted { reason }
                    if reason == "lifecycle_restoration_not_reviewed"
            ));
        }
    }
}
