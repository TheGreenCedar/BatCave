use super::native::{
    capture_elevated_machine_snapshot, open_installed_uninstaller,
    require_allowlisted_elevated_preflight, require_elevated_installed_candidate,
    require_elevated_total_product_absence, OwnedFile, PipeConnection, ProtectedEvidenceRoot,
};
use crate::windows_lifecycle_proof_contract::{
    DesktopPhase, DesktopPhaseDisposition, DesktopPhaseResult, LifecycleStage, ProofPlan,
    SequenceGate, WorkerDisposition, WorkerResult,
};
use std::path::Path;
use std::time::Duration;

const CONTROLLER_READY: bool = false;
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
        return failed(None, failure, true);
    }
    let result = execute_worker_inner(plan, baseline, final_candidate, evidence);
    match result {
        Ok(completed_stage) => failed(
            Some(completed_stage),
            "lifecycle_remaining_stages_not_implemented".to_string(),
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
) -> Result<LifecycleStage, (Option<LifecycleStage>, String, bool)> {
    let initial = capture_elevated_machine_snapshot();
    evidence
        .write_json_new("initial-state.private.json", &initial)
        .map_err(|failure| (None, failure, true))?;
    require_allowlisted_elevated_preflight(&initial, plan)
        .map_err(|failure| (None, failure, true))?;

    let final_copy = final_candidate
        .copy_to(
            &evidence.root().join("final-installer.exe"),
            "final_installer_copy",
        )
        .map_err(|failure| (Some(LifecycleStage::InitialState), failure, true))?;
    let final_repair = final_copy
        .execute(
            evidence,
            FINAL_REPAIR_ARGUMENTS,
            INSTALLER_TIMEOUT,
            "final_repair",
        )
        .map_err(|failure| (Some(LifecycleStage::InitialState), failure, false))?;
    if final_repair.timed_out || !final_repair.process_tree_settled || final_repair.exit_code != 0 {
        return Err((
            Some(LifecycleStage::InitialState),
            "lifecycle_final_repair_failed".to_string(),
            final_repair.process_tree_settled,
        ));
    }
    let final_repair_state = capture_elevated_machine_snapshot();
    require_elevated_installed_candidate(
        &final_repair_state,
        &plan.final_candidate,
        true,
        "final_repair",
    )
    .map_err(|failure| (Some(LifecycleStage::FinalRepair), failure, true))?;
    evidence
        .write_json_new("final-repair-state.private.json", &final_repair_state)
        .map_err(|failure| (Some(LifecycleStage::FinalRepair), failure, true))?;

    let installed_uninstaller = open_installed_uninstaller(&plan.final_candidate)
        .map_err(|failure| (Some(LifecycleStage::FinalRepair), failure, true))?;
    let uninstaller_copy = installed_uninstaller
        .copy_to(
            &evidence.root().join("final-uninstaller.exe"),
            "final_uninstaller_copy",
        )
        .map_err(|failure| (Some(LifecycleStage::FinalRepair), failure, true))?;
    let uninstall = uninstaller_copy
        .execute(
            evidence,
            DIRECT_UNINSTALL_ARGUMENTS,
            UNINSTALLER_TIMEOUT,
            "initial_uninstall",
        )
        .map_err(|failure| (Some(LifecycleStage::FinalRepair), failure, false))?;
    if uninstall.timed_out || !uninstall.process_tree_settled || uninstall.exit_code != 0 {
        return Err((
            Some(LifecycleStage::FinalRepair),
            "lifecycle_initial_uninstall_failed".to_string(),
            uninstall.process_tree_settled,
        ));
    }
    let initial_uninstall_state = capture_elevated_machine_snapshot();
    require_elevated_total_product_absence(&initial_uninstall_state, "initial_uninstall")
        .map_err(|failure| (Some(LifecycleStage::InitialUninstall), failure, true))?;
    evidence
        .write_json_new(
            "initial-uninstall-state.private.json",
            &initial_uninstall_state,
        )
        .map_err(|failure| (Some(LifecycleStage::InitialUninstall), failure, true))?;

    let baseline_copy = baseline
        .copy_to(
            &evidence.root().join("baseline-installer.exe"),
            "baseline_installer_copy",
        )
        .map_err(|failure| (Some(LifecycleStage::InitialUninstall), failure, true))?;
    let baseline_install = baseline_copy
        .execute(
            evidence,
            BASELINE_INSTALL_ARGUMENTS,
            INSTALLER_TIMEOUT,
            "baseline_install",
        )
        .map_err(|failure| (Some(LifecycleStage::InitialUninstall), failure, false))?;
    if baseline_install.timed_out
        || !baseline_install.process_tree_settled
        || baseline_install.exit_code != 0
    {
        return Err((
            Some(LifecycleStage::InitialUninstall),
            "lifecycle_baseline_install_failed".to_string(),
            baseline_install.process_tree_settled,
        ));
    }
    let baseline_install_state = capture_elevated_machine_snapshot();
    require_elevated_installed_candidate(
        &baseline_install_state,
        &plan.baseline,
        false,
        "baseline_install",
    )
    .map_err(|failure| (Some(LifecycleStage::BaselineInstall), failure, true))?;
    evidence
        .write_json_new(
            "baseline-install-state.private.json",
            &baseline_install_state,
        )
        .map_err(|failure| (Some(LifecycleStage::BaselineInstall), failure, true))?;
    Ok(LifecycleStage::BaselineInstall)
}

fn failed(
    completed_stage: Option<LifecycleStage>,
    failure: String,
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
}
