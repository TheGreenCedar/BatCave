use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::{Component, Path};

pub(crate) const EMBEDDED_PLAN: &str = include_str!("windows_lifecycle_proof_plan.v1.json");
pub(crate) const PLAN_SCHEMA: &str = "batcave_windows_lifecycle_proof_plan_v1";
pub(crate) const PROTOCOL_SCHEMA: &str = "batcave_windows_lifecycle_proof_protocol_v3";
pub(crate) const NONCE_HEX_LENGTH: usize = 64;
pub(crate) const LOCATOR_HEX_LENGTH: usize = 32;
pub(crate) const FIRST_SEQUENCE: u64 = 1;
pub(crate) const SUCCESS_PRIVATE_EVIDENCE_LEAVES: [&str; 28] = [
    "initial-state.private.json",
    "final-repair-state.private.json",
    "final-primary-desktop.private.json",
    "initial-uninstall-state.private.json",
    "baseline-install-state.private.json",
    "baseline-primary-desktop.private.json",
    "baseline-second-instance-desktop.private.json",
    "baseline-restart-stopped-state.private.json",
    "baseline-restart-state.private.json",
    "baseline-crashed-state.private.json",
    "baseline-crash-recovery-state.private.json",
    "baseline-rollback-recovery-state.private.json",
    "legacy-residue-seeded-state.private.json",
    "final-upgrade-state.private.json",
    "final-restart-stopped-state.private.json",
    "final-restart-state.private.json",
    "final-crashed-state.private.json",
    "final-crash-recovery-state.private.json",
    "final-missing-service-state.private.json",
    "final-missing-service-desktop.private.json",
    "final-missing-service-restored-state.private.json",
    "final-stopped-service-state.private.json",
    "final-stopped-service-desktop.private.json",
    "final-stopped-service-restored-state.private.json",
    "final-incompatible-service-state.private.json",
    "final-incompatible-service-desktop.private.json",
    "final-incompatible-service-restored-state.private.json",
    "final-uninstall-state.private.json",
];

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ProofPlan {
    pub schema_version: String,
    pub profile: String,
    pub sequence: u64,
    pub baseline: Candidate,
    pub final_candidate: Candidate,
    pub incompatible_service_fixture: ServiceFixture,
    pub rollback_failing_service_fixture: ServiceFixture,
    pub allowlisted_start: AllowlistedStart,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Candidate {
    pub source_commit_sha: String,
    pub installer_relative_path: String,
    pub installer_size: u64,
    pub installer_sha256: String,
    pub monitor_sha256: String,
    pub service_sha256: String,
    pub uninstaller_size: u64,
    pub uninstaller_sha256: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ServiceFixture {
    pub build_source_commit_sha: String,
    pub relative_path: String,
    pub size: u64,
    pub sha256: String,
    pub product_version: String,
    pub behavior: ServiceFixtureBehavior,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ServiceFixtureBehavior {
    IncompatibleRelease,
    FailOnScmStart,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct AllowlistedStart {
    pub state: StartState,
    pub monitor_sha256: String,
    pub service_sha256: String,
    pub uninstaller_sha256: String,
    pub legacy_cli_sha256: String,
    pub win32_exit_code: u32,
    pub service_specific_exit_code: u32,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum StartState {
    #[serde(rename = "legacy_stopped_1066_1")]
    LegacyStopped1066_1,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Envelope<T> {
    pub schema_version: String,
    pub nonce: String,
    pub sequence: u64,
    pub message_sha256: String,
    pub message: T,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(
    deny_unknown_fields,
    rename_all = "snake_case",
    tag = "kind",
    content = "value"
)]
pub(crate) enum ParentMessage {
    Begin(ClosedRequest),
    CheckpointAccepted(WorkerCheckpoint),
    DesktopPhaseComplete(Box<DesktopPhaseResult>),
    Abort(AbortReason),
    EvidenceAccepted,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(
    deny_unknown_fields,
    rename_all = "snake_case",
    tag = "kind",
    content = "value"
)]
pub(crate) enum WorkerMessage {
    Accepted(WorkerAccepted),
    Checkpoint(WorkerCheckpoint),
    RunDesktopPhase(DesktopPhase),
    ResultReady(Box<WorkerResult>),
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum AbortReason {
    ArtifactValidation,
    DesktopFailure,
    Disconnected,
    EvidenceValidation,
    ProtocolViolation,
    ReceiptValidation,
    Timeout,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct WorkerCheckpoint {
    pub completed_stage: LifecycleStage,
    pub evidence_root_identity: EvidenceRootIdentity,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct WorkerAbort {
    pub reason: AbortReason,
    pub last_authenticated_checkpoint: Option<WorkerCheckpoint>,
    pub evidence_root_identity: EvidenceRootIdentity,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ClosedRequest {
    pub plan_sha256: String,
    pub controller_source_commit_sha: String,
    pub controller_sha256: String,
    pub parent_process_id: u32,
    pub parent_started_at_100ns: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct WorkerAccepted {
    pub evidence_root: String,
    pub evidence_root_identity: EvidenceRootIdentity,
    pub worker_process_id: u32,
    pub worker_started_at_100ns: u64,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct EvidenceRootIdentity {
    pub volume_serial: u32,
    pub file_index: u64,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum DesktopPhase {
    BaselinePrimary,
    BaselineSecondInstance,
    FinalPrimary,
    FinalMissingService,
    FinalStoppedService,
    FinalIncompatibleService,
}

impl DesktopPhase {
    pub(crate) fn expected_collector_state(self) -> DesktopCollectorState {
        match self {
            Self::BaselinePrimary | Self::BaselineSecondInstance | Self::FinalPrimary => {
                DesktopCollectorState::Active
            }
            Self::FinalMissingService => DesktopCollectorState::NotInstalled,
            Self::FinalStoppedService => DesktopCollectorState::Stopped,
            Self::FinalIncompatibleService => DesktopCollectorState::Incompatible,
        }
    }

    pub(crate) fn expects_existing_primary_focus(self) -> bool {
        self == Self::BaselineSecondInstance
    }

    fn expected_monitor_sha256(self, plan: &ProofPlan) -> &str {
        match self {
            Self::BaselinePrimary | Self::BaselineSecondInstance => &plan.baseline.monitor_sha256,
            Self::FinalPrimary
            | Self::FinalMissingService
            | Self::FinalStoppedService
            | Self::FinalIncompatibleService => &plan.final_candidate.monitor_sha256,
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum DesktopCollectorState {
    Active,
    NotInstalled,
    Stopped,
    Incompatible,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum DesktopPrivilegedSource {
    InstalledCollectorService,
    None,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct DesktopProcessObservation {
    pub process_id: u32,
    pub parent_process_id: Option<u32>,
    pub started_at_100ns: u64,
    pub session_id: u32,
    pub elevated: bool,
    pub executable_path: String,
    pub executable_size: u64,
    pub executable_sha256: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct DesktopSecondInstanceObservation {
    pub attempted_process: DesktopProcessObservation,
    pub terminal_exit_code: u32,
    pub process_tree_settled: bool,
    pub focused_primary_process_id: u32,
    pub focused_primary_started_at_100ns: u64,
    pub service_instance_id_before: String,
    pub service_instance_id_after: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct DesktopFileObservation {
    pub executable_path: String,
    pub executable_size: u64,
    pub executable_sha256: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct DesktopServiceProcessObservation {
    pub process_id: u32,
    pub started_at_100ns: u64,
    pub local_system: bool,
    pub executable_path: String,
    pub executable_size: u64,
    pub executable_sha256: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct DesktopCollectorRuntimeObservation {
    pub installed_service: Option<DesktopFileObservation>,
    pub service_process: Option<DesktopServiceProcessObservation>,
    pub pipe_server_process_id: Option<u32>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct DesktopVisibleObservation {
    pub current_process_standard: bool,
    pub collector_state: DesktopCollectorState,
    pub privileged_source: DesktopPrivilegedSource,
    pub standard_monitoring_current: bool,
    pub protected_sample_current: bool,
    pub fallback_etw_disabled: bool,
    pub service_version: Option<String>,
    pub service_release_version: Option<String>,
    pub negotiated_protocol_version: Option<u16>,
    pub minimum_desktop_version: Option<String>,
    pub service_instance_id: Option<String>,
    pub service_detail: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct DesktopPhaseObservation {
    pub desktop: DesktopProcessObservation,
    pub process_tree: Vec<DesktopProcessObservation>,
    pub webview_process_ids: Vec<u32>,
    pub second_instance: Option<DesktopSecondInstanceObservation>,
    pub collector_runtime: DesktopCollectorRuntimeObservation,
    pub visible: DesktopVisibleObservation,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct DesktopPhaseResult {
    pub phase: DesktopPhase,
    pub disposition: DesktopPhaseDisposition,
    pub process_tree_settled: bool,
    pub observation: Option<DesktopPhaseObservation>,
    pub failure_reason: Option<String>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum DesktopPhaseDisposition {
    Passed,
    Failed,
}

pub(crate) fn validate_desktop_phase_result(
    result: &DesktopPhaseResult,
    plan: &ProofPlan,
) -> Result<(), String> {
    match result.disposition {
        DesktopPhaseDisposition::Passed => {
            if !result.process_tree_settled || result.failure_reason.is_some() {
                return Err("lifecycle_desktop_phase_pass_shape_invalid".to_string());
            }
            let observation = result
                .observation
                .as_ref()
                .ok_or_else(|| "lifecycle_desktop_phase_observation_missing".to_string())?;
            validate_desktop_process(&observation.desktop, "desktop")?;
            if observation.desktop.session_id == 0
                || observation.desktop.parent_process_id.is_some()
                || !observation
                    .desktop
                    .executable_path
                    .eq_ignore_ascii_case(r"C:\Program Files\BatCave Monitor\batcave-monitor.exe")
                || observation.desktop.executable_sha256
                    != result.phase.expected_monitor_sha256(plan)
            {
                return Err("lifecycle_desktop_exact_identity_invalid".to_string());
            }
            validate_desktop_process_tree(observation)?;
            validate_collector_runtime(result.phase, &observation.collector_runtime, plan)?;
            validate_desktop_process_roles(observation)?;
            validate_second_instance(result.phase, observation)?;
            validate_desktop_visible(result.phase, &observation.visible)
        }
        DesktopPhaseDisposition::Failed => {
            if result
                .failure_reason
                .as_deref()
                .is_none_or(|reason| !valid_bounded_reason(reason))
            {
                return Err("lifecycle_desktop_phase_failure_reason_invalid".to_string());
            }
            if let Some(observation) = &result.observation {
                validate_desktop_process(&observation.desktop, "desktop")?;
                if observation.process_tree.len() > 128
                    || observation.webview_process_ids.len() > 32
                {
                    return Err("lifecycle_desktop_phase_process_tree_invalid".to_string());
                }
                for process in &observation.process_tree {
                    validate_desktop_process(process, "child")?;
                }
                if let Some(second) = &observation.second_instance {
                    validate_desktop_process(&second.attempted_process, "second_instance")?;
                }
            }
            Ok(())
        }
    }
}

fn validate_desktop_process(
    observation: &DesktopProcessObservation,
    label: &str,
) -> Result<(), String> {
    if observation.process_id == 0
        || observation.started_at_100ns == 0
        || observation.executable_size == 0
        || observation.executable_path.is_empty()
        || observation.executable_path.len() > 32_768
        || observation.executable_path.contains('\0')
        || observation.elevated
    {
        return Err(format!("lifecycle_desktop_{label}_identity_invalid"));
    }
    validate_sha256(
        &observation.executable_sha256,
        &format!("desktop_{label}_executable"),
    )
}

fn validate_desktop_process_tree(observation: &DesktopPhaseObservation) -> Result<(), String> {
    if observation.process_tree.is_empty()
        || observation.process_tree.len() > 128
        || observation.webview_process_ids.is_empty()
        || observation.webview_process_ids.len() > 32
    {
        return Err("lifecycle_desktop_phase_process_tree_invalid".to_string());
    }

    let mut processes = std::collections::BTreeMap::new();
    processes.insert(observation.desktop.process_id, &observation.desktop);
    for process in &observation.process_tree {
        validate_desktop_process(process, "child")?;
        if process.session_id != observation.desktop.session_id
            || processes.insert(process.process_id, process).is_some()
        {
            return Err("lifecycle_desktop_phase_process_tree_invalid".to_string());
        }
    }
    for process in &observation.process_tree {
        let Some(parent_process_id) = process.parent_process_id else {
            return Err("lifecycle_desktop_phase_process_tree_invalid".to_string());
        };
        if !processes.contains_key(&parent_process_id) {
            return Err("lifecycle_desktop_phase_process_tree_invalid".to_string());
        }
        let mut next = process.parent_process_id;
        let mut seen = std::collections::BTreeSet::new();
        loop {
            let Some(parent_process_id) = next else {
                return Err("lifecycle_desktop_phase_process_tree_invalid".to_string());
            };
            if parent_process_id == observation.desktop.process_id {
                break;
            }
            if !seen.insert(parent_process_id) {
                return Err("lifecycle_desktop_phase_process_tree_invalid".to_string());
            }
            next = processes
                .get(&parent_process_id)
                .and_then(|parent| parent.parent_process_id);
        }
    }

    let mut webview_process_ids = std::collections::BTreeSet::new();
    let mut webview_hashes = std::collections::BTreeSet::new();
    let mut webview_sizes = std::collections::BTreeSet::new();
    let mut webview_paths = std::collections::BTreeSet::new();
    for process_id in &observation.webview_process_ids {
        let Some(webview) = processes.get(process_id) else {
            return Err("lifecycle_desktop_phase_webview_identity_invalid".to_string());
        };
        if !webview_process_ids.insert(*process_id)
            || !valid_webview_runtime_path(&webview.executable_path)
        {
            return Err("lifecycle_desktop_phase_webview_identity_invalid".to_string());
        }
        webview_hashes.insert(webview.executable_sha256.as_str());
        webview_sizes.insert(webview.executable_size);
        webview_paths.insert(webview.executable_path.to_ascii_lowercase());
    }
    if webview_hashes.len() != 1 || webview_sizes.len() != 1 || webview_paths.len() != 1 {
        return Err("lifecycle_desktop_phase_webview_identity_invalid".to_string());
    }
    Ok(())
}

fn valid_webview_runtime_path(value: &str) -> bool {
    const PREFIX: &str = r"C:\Program Files (x86)\Microsoft\EdgeWebView\Application\";
    let Some(relative) = value.get(PREFIX.len()..) else {
        return false;
    };
    value
        .get(..PREFIX.len())
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case(PREFIX))
        && relative.rsplit_once('\\').is_some_and(|(version, leaf)| {
            valid_webview_version(version) && leaf.eq_ignore_ascii_case("msedgewebview2.exe")
        })
}

fn valid_webview_version(version: &str) -> bool {
    !version.is_empty()
        && version
            .split('.')
            .all(|segment| !segment.is_empty() && segment.bytes().all(|byte| byte.is_ascii_digit()))
}

fn validate_collector_runtime(
    phase: DesktopPhase,
    runtime: &DesktopCollectorRuntimeObservation,
    plan: &ProofPlan,
) -> Result<(), String> {
    let expected_service_sha256 = match phase {
        DesktopPhase::BaselinePrimary | DesktopPhase::BaselineSecondInstance => {
            &plan.baseline.service_sha256
        }
        DesktopPhase::FinalPrimary
        | DesktopPhase::FinalMissingService
        | DesktopPhase::FinalStoppedService => &plan.final_candidate.service_sha256,
        DesktopPhase::FinalIncompatibleService => &plan.incompatible_service_fixture.sha256,
    };
    let service_expected = phase != DesktopPhase::FinalMissingService;
    let running_expected = !matches!(
        phase,
        DesktopPhase::FinalMissingService | DesktopPhase::FinalStoppedService
    );

    if service_expected {
        let service = runtime
            .installed_service
            .as_ref()
            .ok_or_else(|| "lifecycle_desktop_installed_service_missing".to_string())?;
        validate_desktop_file(service, "installed_service")?;
        if !service
            .executable_path
            .eq_ignore_ascii_case(r"C:\Program Files\BatCave Monitor\batcave-collector-service.exe")
            || service.executable_sha256.as_str() != expected_service_sha256
            || (phase == DesktopPhase::FinalIncompatibleService
                && service.executable_size != plan.incompatible_service_fixture.size)
        {
            return Err("lifecycle_desktop_installed_service_identity_invalid".to_string());
        }
    } else if runtime.installed_service.is_some() {
        return Err("lifecycle_desktop_installed_service_unexpected".to_string());
    }

    if running_expected {
        let service = runtime
            .installed_service
            .as_ref()
            .ok_or_else(|| "lifecycle_desktop_installed_service_missing".to_string())?;
        let process = runtime
            .service_process
            .as_ref()
            .ok_or_else(|| "lifecycle_desktop_service_process_missing".to_string())?;
        validate_desktop_service_process(process)?;
        if !process
            .executable_path
            .eq_ignore_ascii_case(&service.executable_path)
            || process.executable_size != service.executable_size
            || process.executable_sha256 != service.executable_sha256
            || runtime.pipe_server_process_id != Some(process.process_id)
        {
            return Err("lifecycle_desktop_service_runtime_invalid".to_string());
        }
    } else if runtime.service_process.is_some() || runtime.pipe_server_process_id.is_some() {
        return Err("lifecycle_desktop_service_runtime_unexpected".to_string());
    }
    Ok(())
}

fn validate_desktop_file(observation: &DesktopFileObservation, label: &str) -> Result<(), String> {
    if observation.executable_size == 0
        || observation.executable_path.is_empty()
        || observation.executable_path.len() > 32_768
        || observation.executable_path.contains('\0')
    {
        return Err(format!("lifecycle_desktop_{label}_identity_invalid"));
    }
    validate_sha256(
        &observation.executable_sha256,
        &format!("desktop_{label}_executable"),
    )
}

fn validate_desktop_service_process(
    observation: &DesktopServiceProcessObservation,
) -> Result<(), String> {
    if observation.process_id == 0 || observation.started_at_100ns == 0 || !observation.local_system
    {
        return Err("lifecycle_desktop_service_process_identity_invalid".to_string());
    }
    validate_desktop_file(
        &DesktopFileObservation {
            executable_path: observation.executable_path.clone(),
            executable_size: observation.executable_size,
            executable_sha256: observation.executable_sha256.clone(),
        },
        "service_process",
    )
}

fn validate_desktop_process_roles(observation: &DesktopPhaseObservation) -> Result<(), String> {
    let mut desktop_process_ids = observation
        .process_tree
        .iter()
        .map(|process| process.process_id)
        .collect::<std::collections::BTreeSet<_>>();
    desktop_process_ids.insert(observation.desktop.process_id);

    let service_process_id = observation
        .collector_runtime
        .service_process
        .as_ref()
        .map(|process| process.process_id);
    if service_process_id.is_some_and(|process_id| desktop_process_ids.contains(&process_id)) {
        return Err("lifecycle_desktop_process_role_collision".to_string());
    }
    if let Some(second) = &observation.second_instance {
        let attempted_process_id = second.attempted_process.process_id;
        if desktop_process_ids.contains(&attempted_process_id)
            || service_process_id == Some(attempted_process_id)
        {
            return Err("lifecycle_desktop_process_role_collision".to_string());
        }
    }
    Ok(())
}

fn validate_second_instance(
    phase: DesktopPhase,
    observation: &DesktopPhaseObservation,
) -> Result<(), String> {
    if phase.expects_existing_primary_focus() {
        let second = observation
            .second_instance
            .as_ref()
            .ok_or_else(|| "lifecycle_desktop_second_instance_missing".to_string())?;
        validate_desktop_process(&second.attempted_process, "second_instance")?;
        if second.attempted_process.parent_process_id.is_some()
            || second.attempted_process.process_id == observation.desktop.process_id
            || second.attempted_process.session_id != observation.desktop.session_id
            || !second
                .attempted_process
                .executable_path
                .eq_ignore_ascii_case(&observation.desktop.executable_path)
            || second.attempted_process.executable_size != observation.desktop.executable_size
            || second.attempted_process.executable_sha256 != observation.desktop.executable_sha256
            || !second.process_tree_settled
            || second.terminal_exit_code != 0
            || second.focused_primary_process_id != observation.desktop.process_id
            || second.focused_primary_started_at_100ns != observation.desktop.started_at_100ns
            || second.service_instance_id_before != second.service_instance_id_after
            || !valid_service_instance_id(&second.service_instance_id_before)
            || observation.visible.service_instance_id.as_deref()
                != Some(second.service_instance_id_before.as_str())
        {
            return Err("lifecycle_desktop_second_instance_invalid".to_string());
        }
    } else if observation.second_instance.is_some() {
        return Err("lifecycle_desktop_second_instance_unexpected".to_string());
    }
    Ok(())
}

pub(crate) fn validate_desktop_visible(
    phase: DesktopPhase,
    visible: &DesktopVisibleObservation,
) -> Result<(), String> {
    if !visible.current_process_standard
        || visible.collector_state != phase.expected_collector_state()
    {
        return Err("lifecycle_desktop_visible_state_invalid".to_string());
    }
    let active = visible.collector_state == DesktopCollectorState::Active;
    let expected_source = if active {
        DesktopPrivilegedSource::InstalledCollectorService
    } else {
        DesktopPrivilegedSource::None
    };
    if visible.privileged_source != expected_source
        || visible.standard_monitoring_current == active
        || visible.protected_sample_current != active
        || visible.fallback_etw_disabled == active
    {
        return Err("lifecycle_desktop_visible_source_invalid".to_string());
    }

    match visible.collector_state {
        DesktopCollectorState::Active => {
            if visible.service_version.as_deref() != Some(env!("CARGO_PKG_VERSION"))
                || visible.service_release_version.as_deref() != Some(env!("CARGO_PKG_VERSION"))
                || visible.negotiated_protocol_version != Some(1)
                || visible.minimum_desktop_version.as_deref() != Some(env!("CARGO_PKG_VERSION"))
                || visible
                    .service_instance_id
                    .as_deref()
                    .is_none_or(|value| !valid_service_instance_id(value))
                || visible.service_detail.is_some()
            {
                return Err("lifecycle_desktop_active_identity_invalid".to_string());
            }
        }
        DesktopCollectorState::Incompatible => {
            if visible.service_version.as_deref() != Some("0.2.0-rc.3")
                || visible.service_release_version.as_deref() != Some("0.2.0-rc.3")
                || visible.negotiated_protocol_version.is_some()
                || visible.minimum_desktop_version.is_some()
                || visible.service_instance_id.is_some()
                || visible.service_detail.as_deref()
                    != Some("collector_service_desktop_release_incompatible")
            {
                return Err("lifecycle_desktop_incompatible_identity_invalid".to_string());
            }
        }
        DesktopCollectorState::NotInstalled | DesktopCollectorState::Stopped => {
            let expected_detail = match visible.collector_state {
                DesktopCollectorState::NotInstalled => "collector_service_open_failed:1060",
                DesktopCollectorState::Stopped => "collector_service_stopped",
                _ => unreachable!("matched fallback state"),
            };
            if visible.service_version.is_some()
                || visible.service_release_version.is_some()
                || visible.negotiated_protocol_version.is_some()
                || visible.minimum_desktop_version.is_some()
                || visible.service_instance_id.is_some()
                || visible.service_detail.as_deref() != Some(expected_detail)
            {
                return Err("lifecycle_desktop_fallback_identity_invalid".to_string());
            }
        }
    }
    Ok(())
}

pub(crate) fn valid_service_instance_id(value: &str) -> bool {
    value.len() == 41
        && value.as_bytes().get(8) == Some(&b'-')
        && value.bytes().enumerate().all(|(index, byte)| {
            index == 8 || byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)
        })
}

fn valid_bounded_reason(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 192
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b':' | b'_' | b'-' | b'.'))
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct WorkerResult {
    pub disposition: WorkerDisposition,
    pub completed_stage: Option<LifecycleStage>,
    pub last_authenticated_checkpoint: Option<WorkerCheckpoint>,
    pub abort: Option<WorkerAbort>,
    pub failure: Option<WorkerFailure>,
    pub process_tree_settled: bool,
    pub private_evidence: Vec<EvidenceReceipt>,
    pub sanitized_export: Option<EvidenceReceipt>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum WorkerFailureKind {
    Mutation,
    EvidenceWrite,
    ProcessSettlement,
    Controller,
    ParentAbort,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct EvidenceReceipt {
    pub name: String,
    pub size: u64,
    pub sha256: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct WorkerFailure {
    pub kind: WorkerFailureKind,
    pub attempted_stage: Option<LifecycleStage>,
    pub reason: String,
    pub evidence: Option<Box<EvidenceReceipt>>,
    pub evidence_error: Option<String>,
    pub restoration: Box<RestorationOutcome>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(
    deny_unknown_fields,
    rename_all = "snake_case",
    tag = "disposition",
    content = "value"
)]
pub(crate) enum RestorationOutcome {
    NotRequired,
    Restored {
        evidence: EvidenceReceipt,
    },
    BlockedUnsettled,
    BlockedUntrusted {
        reason: String,
    },
    Failed {
        reason: String,
        evidence: Option<EvidenceReceipt>,
        evidence_error: Option<String>,
    },
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum WorkerDisposition {
    Passed,
    Failed,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum LifecycleStage {
    InitialState,
    FinalRepair,
    InitialUninstall,
    BaselineInstall,
    BaselineRestart,
    BaselineCrashRecovery,
    BaselineRollbackRecovery,
    LegacyResidueSeeded,
    FinalUpgrade,
    FinalRestart,
    FinalCrashRecovery,
    FinalFallbackStates,
    FinalUninstall,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(
    deny_unknown_fields,
    rename_all = "snake_case",
    tag = "state",
    content = "value"
)]
pub(crate) enum Observation<T> {
    Present(T),
    Absent,
    Unknown(String),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct SequenceGate {
    expected: u64,
}

impl SequenceGate {
    pub(crate) fn new() -> Self {
        Self {
            expected: FIRST_SEQUENCE,
        }
    }

    pub(crate) fn accept(&mut self, sequence: u64) -> Result<(), String> {
        if sequence != self.expected {
            return Err("lifecycle_protocol_sequence_invalid".to_string());
        }
        self.expected = self
            .expected
            .checked_add(1)
            .ok_or_else(|| "lifecycle_protocol_sequence_overflow".to_string())?;
        Ok(())
    }

    pub(crate) fn next(&mut self) -> Result<u64, String> {
        let sequence = self.expected;
        self.accept(sequence)?;
        Ok(sequence)
    }
}

pub(crate) fn parse_plan() -> Result<ProofPlan, String> {
    let plan: ProofPlan = serde_json::from_str(EMBEDDED_PLAN)
        .map_err(|error| format!("lifecycle_plan_json_invalid:{error}"))?;
    validate_plan(&plan)?;
    Ok(plan)
}

pub(crate) fn plan_sha256() -> String {
    hex_digest(EMBEDDED_PLAN.as_bytes())
}

pub(crate) fn validate_plan(plan: &ProofPlan) -> Result<(), String> {
    if plan.schema_version != PLAN_SCHEMA {
        return Err("lifecycle_plan_schema_invalid".to_string());
    }
    if plan.profile.is_empty() || plan.profile.len() > 96 {
        return Err("lifecycle_plan_profile_invalid".to_string());
    }
    if plan.sequence != FIRST_SEQUENCE {
        return Err("lifecycle_plan_sequence_invalid".to_string());
    }
    validate_candidate(&plan.baseline, "baseline")?;
    validate_candidate(&plan.final_candidate, "final")?;
    validate_incompatible_service_fixture(
        &plan.incompatible_service_fixture,
        &plan.baseline,
        &plan.final_candidate,
    )?;
    validate_rollback_service_fixture(
        &plan.rollback_failing_service_fixture,
        &plan.incompatible_service_fixture,
        &plan.baseline,
        &plan.final_candidate,
    )?;
    if plan.baseline.installer_relative_path == plan.final_candidate.installer_relative_path {
        return Err("lifecycle_plan_installer_paths_collide".to_string());
    }
    for (value, field) in [
        (&plan.allowlisted_start.monitor_sha256, "start_monitor"),
        (&plan.allowlisted_start.service_sha256, "start_service"),
        (
            &plan.allowlisted_start.uninstaller_sha256,
            "start_uninstaller",
        ),
        (&plan.allowlisted_start.legacy_cli_sha256, "start_cli"),
    ] {
        validate_sha256(value, field)?;
    }
    if plan.allowlisted_start.win32_exit_code != 1066
        || plan.allowlisted_start.service_specific_exit_code != 1
    {
        return Err("lifecycle_plan_start_exit_codes_invalid".to_string());
    }
    Ok(())
}

fn validate_incompatible_service_fixture(
    fixture: &ServiceFixture,
    baseline: &Candidate,
    final_candidate: &Candidate,
) -> Result<(), String> {
    validate_commit_sha(&fixture.build_source_commit_sha, "incompatible_fixture")?;
    if fixture.build_source_commit_sha != final_candidate.source_commit_sha {
        return Err("lifecycle_plan_incompatible_fixture_source_invalid".to_string());
    }
    validate_relative_artifact_path(&fixture.relative_path, "incompatible_fixture")?;
    if fixture.relative_path == baseline.installer_relative_path
        || fixture.relative_path == final_candidate.installer_relative_path
        || fixture.size == 0
        || fixture.size > 64 * 1024 * 1024
        || fixture.sha256 == baseline.service_sha256
        || fixture.sha256 == final_candidate.service_sha256
    {
        return Err("lifecycle_plan_incompatible_fixture_identity_invalid".to_string());
    }
    validate_sha256(&fixture.sha256, "incompatible_fixture")?;
    if fixture.product_version != "0.2.0-rc.3"
        || fixture.product_version == env!("CARGO_PKG_VERSION")
        || fixture.behavior != ServiceFixtureBehavior::IncompatibleRelease
    {
        return Err("lifecycle_plan_incompatible_fixture_behavior_invalid".to_string());
    }
    Ok(())
}

fn validate_rollback_service_fixture(
    fixture: &ServiceFixture,
    incompatible_fixture: &ServiceFixture,
    baseline: &Candidate,
    final_candidate: &Candidate,
) -> Result<(), String> {
    validate_commit_sha(&fixture.build_source_commit_sha, "rollback_fixture")?;
    if fixture.build_source_commit_sha != "c95fffc870226f0852048055d79fa4a18a14471c" {
        return Err("lifecycle_plan_rollback_fixture_source_invalid".to_string());
    }
    validate_relative_artifact_path(&fixture.relative_path, "rollback_fixture")?;
    if fixture.relative_path == baseline.installer_relative_path
        || fixture.relative_path == final_candidate.installer_relative_path
        || fixture.relative_path == incompatible_fixture.relative_path
        || fixture.size == 0
        || fixture.size > 64 * 1024 * 1024
        || fixture.sha256 == baseline.service_sha256
        || fixture.sha256 == final_candidate.service_sha256
        || fixture.sha256 == incompatible_fixture.sha256
    {
        return Err("lifecycle_plan_rollback_fixture_identity_invalid".to_string());
    }
    validate_sha256(&fixture.sha256, "rollback_fixture")?;
    if fixture.product_version != env!("CARGO_PKG_VERSION")
        || fixture.behavior != ServiceFixtureBehavior::FailOnScmStart
    {
        return Err("lifecycle_plan_rollback_fixture_behavior_invalid".to_string());
    }
    Ok(())
}

fn validate_candidate(candidate: &Candidate, prefix: &str) -> Result<(), String> {
    validate_commit_sha(&candidate.source_commit_sha, prefix)?;
    validate_relative_artifact_path(&candidate.installer_relative_path, prefix)?;
    if candidate.installer_size == 0 || candidate.installer_size > 512 * 1024 * 1024 {
        return Err(format!("lifecycle_plan_{prefix}_size_invalid"));
    }
    if candidate.uninstaller_size == 0 || candidate.uninstaller_size > 16 * 1024 * 1024 {
        return Err(format!("lifecycle_plan_{prefix}_uninstaller_size_invalid"));
    }
    for (value, field) in [
        (&candidate.installer_sha256, "installer"),
        (&candidate.monitor_sha256, "monitor"),
        (&candidate.service_sha256, "service"),
        (&candidate.uninstaller_sha256, "uninstaller"),
    ] {
        validate_sha256(value, &format!("{prefix}_{field}"))?;
    }
    Ok(())
}

fn validate_relative_artifact_path(value: &str, prefix: &str) -> Result<(), String> {
    let path = Path::new(value);
    if value.is_empty()
        || value.contains('\\')
        || path.is_absolute()
        || path
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
        || path.extension().and_then(|value| value.to_str()) != Some("exe")
    {
        return Err(format!("lifecycle_plan_{prefix}_path_invalid"));
    }
    Ok(())
}

pub(crate) fn validate_nonce(value: &str) -> Result<(), String> {
    validate_lower_hex(value, NONCE_HEX_LENGTH, "lifecycle_nonce_invalid")
}

pub(crate) fn validate_locator(value: &str) -> Result<(), String> {
    validate_lower_hex(value, LOCATOR_HEX_LENGTH, "lifecycle_pipe_locator_invalid")
}

pub(crate) fn validate_sha256(value: &str, field: &str) -> Result<(), String> {
    validate_lower_hex(value, 64, &format!("lifecycle_{field}_sha256_invalid"))
}

pub(crate) fn validate_envelope<T: Serialize>(
    envelope: &Envelope<T>,
    nonce: &str,
    gate: &mut SequenceGate,
) -> Result<(), String> {
    if envelope.schema_version != PROTOCOL_SCHEMA {
        return Err("lifecycle_protocol_schema_invalid".to_string());
    }
    if envelope.nonce != nonce {
        return Err("lifecycle_protocol_nonce_invalid".to_string());
    }
    validate_sha256(&envelope.message_sha256, "protocol_message")?;
    if envelope.message_sha256 != message_sha256(&envelope.message)? {
        return Err("lifecycle_protocol_message_digest_invalid".to_string());
    }
    gate.accept(envelope.sequence)
}

pub(crate) fn message_sha256<T: Serialize>(message: &T) -> Result<String, String> {
    serde_json::to_vec(message)
        .map(|bytes| hex_digest(&bytes))
        .map_err(|_| "lifecycle_protocol_message_serialize_failed".to_string())
}

fn validate_commit_sha(value: &str, field: &str) -> Result<(), String> {
    validate_lower_hex(value, 40, &format!("lifecycle_{field}_commit_invalid"))
}

fn validate_lower_hex(value: &str, length: usize, reason: &str) -> Result<(), String> {
    if value.len() != length
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(reason.to_string());
    }
    Ok(())
}

fn hex_digest(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_plan_is_strict_fixed_and_valid() {
        let plan = parse_plan().expect("embedded plan");
        assert_eq!(plan.schema_version, PLAN_SCHEMA);
        assert_eq!(plan.sequence, FIRST_SEQUENCE);
        assert_eq!(
            plan.allowlisted_start.state,
            StartState::LegacyStopped1066_1
        );
        assert_eq!(
            plan.incompatible_service_fixture.behavior,
            ServiceFixtureBehavior::IncompatibleRelease
        );
        assert_ne!(
            plan.incompatible_service_fixture.product_version,
            env!("CARGO_PKG_VERSION")
        );
        assert_eq!(
            plan.rollback_failing_service_fixture.behavior,
            ServiceFixtureBehavior::FailOnScmStart
        );
        assert_eq!(
            plan.rollback_failing_service_fixture.product_version,
            env!("CARGO_PKG_VERSION")
        );
        assert_eq!(plan_sha256().len(), 64);
    }

    #[test]
    fn plan_rejects_unknown_fields_noncanonical_hashes_and_runtime_paths() {
        let unknown = EMBEDDED_PLAN.replacen(
            "\"profile\":",
            "\"command\": \"powershell.exe\", \"profile\":",
            1,
        );
        assert!(serde_json::from_str::<ProofPlan>(&unknown).is_err());

        let mut plan = parse_plan().expect("plan");
        plan.final_candidate.installer_sha256 =
            plan.final_candidate.installer_sha256.to_uppercase();
        assert_eq!(
            validate_plan(&plan),
            Err("lifecycle_final_installer_sha256_invalid".to_string())
        );

        let mut plan = parse_plan().expect("plan");
        plan.final_candidate.installer_relative_path = "../hostile.exe".to_string();
        assert_eq!(
            validate_plan(&plan),
            Err("lifecycle_plan_final_path_invalid".to_string())
        );

        let mut plan = parse_plan().expect("plan");
        plan.incompatible_service_fixture.product_version = env!("CARGO_PKG_VERSION").to_string();
        assert_eq!(
            validate_plan(&plan),
            Err("lifecycle_plan_incompatible_fixture_behavior_invalid".to_string())
        );

        let mut plan = parse_plan().expect("plan");
        plan.incompatible_service_fixture.relative_path = "../hostile.exe".to_string();
        assert_eq!(
            validate_plan(&plan),
            Err("lifecycle_plan_incompatible_fixture_path_invalid".to_string())
        );

        let mut plan = parse_plan().expect("plan");
        plan.incompatible_service_fixture.build_source_commit_sha =
            plan.baseline.source_commit_sha.clone();
        assert_eq!(
            validate_plan(&plan),
            Err("lifecycle_plan_incompatible_fixture_source_invalid".to_string())
        );

        let mut plan = parse_plan().expect("plan");
        plan.incompatible_service_fixture.sha256 = plan.final_candidate.service_sha256.clone();
        assert_eq!(
            validate_plan(&plan),
            Err("lifecycle_plan_incompatible_fixture_identity_invalid".to_string())
        );

        let mut plan = parse_plan().expect("plan");
        plan.rollback_failing_service_fixture
            .build_source_commit_sha = plan.final_candidate.source_commit_sha.clone();
        assert_eq!(
            validate_plan(&plan),
            Err("lifecycle_plan_rollback_fixture_source_invalid".to_string())
        );

        let mut plan = parse_plan().expect("plan");
        plan.rollback_failing_service_fixture.relative_path =
            plan.incompatible_service_fixture.relative_path.clone();
        assert_eq!(
            validate_plan(&plan),
            Err("lifecycle_plan_rollback_fixture_identity_invalid".to_string())
        );

        let mut plan = parse_plan().expect("plan");
        plan.rollback_failing_service_fixture.behavior =
            ServiceFixtureBehavior::IncompatibleRelease;
        assert_eq!(
            validate_plan(&plan),
            Err("lifecycle_plan_rollback_fixture_behavior_invalid".to_string())
        );
    }

    #[test]
    fn nonce_and_locator_require_canonical_entropy() {
        assert!(validate_nonce(&"a".repeat(NONCE_HEX_LENGTH)).is_ok());
        assert!(validate_locator(&"b".repeat(LOCATOR_HEX_LENGTH)).is_ok());
        assert!(validate_nonce(&"A".repeat(NONCE_HEX_LENGTH)).is_err());
        assert!(validate_locator(&"b".repeat(LOCATOR_HEX_LENGTH - 1)).is_err());
    }

    #[test]
    fn sequence_gate_rejects_replay_gap_and_overflow() {
        let mut gate = SequenceGate::new();
        assert_eq!(gate.next(), Ok(1));
        assert_eq!(
            gate.accept(1),
            Err("lifecycle_protocol_sequence_invalid".to_string())
        );
        assert_eq!(
            gate.accept(3),
            Err("lifecycle_protocol_sequence_invalid".to_string())
        );
        assert_eq!(gate.accept(2), Ok(()));

        let mut gate = SequenceGate { expected: u64::MAX };
        assert_eq!(
            gate.accept(u64::MAX),
            Err("lifecycle_protocol_sequence_overflow".to_string())
        );
    }

    #[test]
    fn observation_never_conflates_unknown_with_absent() {
        let absent: Observation<u32> = Observation::Absent;
        let unknown: Observation<u32> = Observation::Unknown("access_denied".to_string());
        assert_ne!(absent, unknown);
    }

    #[test]
    fn desktop_phase_contract_accepts_active_and_fallback_truth() {
        for phase in [
            DesktopPhase::FinalPrimary,
            DesktopPhase::BaselinePrimary,
            DesktopPhase::BaselineSecondInstance,
            DesktopPhase::FinalMissingService,
            DesktopPhase::FinalStoppedService,
            DesktopPhase::FinalIncompatibleService,
        ] {
            assert_eq!(
                validate_desktop_phase_result(&passed_desktop_phase(phase), &parse_plan().unwrap()),
                Ok(()),
                "{phase:?}"
            );
        }
    }

    #[test]
    fn desktop_phase_contract_rejects_elevation_source_and_settlement_drift() {
        let mut elevated = passed_desktop_phase(DesktopPhase::FinalPrimary);
        elevated
            .observation
            .as_mut()
            .expect("observation")
            .desktop
            .elevated = true;
        assert_eq!(
            validate_desktop_phase_result(&elevated, &parse_plan().unwrap()),
            Err("lifecycle_desktop_desktop_identity_invalid".to_string())
        );

        let mut wrong_source = passed_desktop_phase(DesktopPhase::FinalStoppedService);
        wrong_source
            .observation
            .as_mut()
            .expect("observation")
            .visible
            .privileged_source = DesktopPrivilegedSource::InstalledCollectorService;
        assert_eq!(
            validate_desktop_phase_result(&wrong_source, &parse_plan().unwrap()),
            Err("lifecycle_desktop_visible_source_invalid".to_string())
        );

        let mut invalid_instance = passed_desktop_phase(DesktopPhase::FinalPrimary);
        invalid_instance
            .observation
            .as_mut()
            .expect("observation")
            .visible
            .service_instance_id = Some(".".to_string());
        assert_eq!(
            validate_desktop_phase_result(&invalid_instance, &parse_plan().unwrap()),
            Err("lifecycle_desktop_active_identity_invalid".to_string())
        );

        let mut missing_detail = passed_desktop_phase(DesktopPhase::FinalMissingService);
        missing_detail
            .observation
            .as_mut()
            .expect("observation")
            .visible
            .service_detail = None;
        assert_eq!(
            validate_desktop_phase_result(&missing_detail, &parse_plan().unwrap()),
            Err("lifecycle_desktop_fallback_identity_invalid".to_string())
        );

        let mut wrong_stopped_detail = passed_desktop_phase(DesktopPhase::FinalStoppedService);
        wrong_stopped_detail
            .observation
            .as_mut()
            .expect("observation")
            .visible
            .service_detail = Some("collector_service_open_failed:1060".to_string());
        assert_eq!(
            validate_desktop_phase_result(&wrong_stopped_detail, &parse_plan().unwrap()),
            Err("lifecycle_desktop_fallback_identity_invalid".to_string())
        );

        let mut unsettled = passed_desktop_phase(DesktopPhase::FinalPrimary);
        unsettled.process_tree_settled = false;
        assert_eq!(
            validate_desktop_phase_result(&unsettled, &parse_plan().unwrap()),
            Err("lifecycle_desktop_phase_pass_shape_invalid".to_string())
        );
    }

    #[test]
    fn desktop_phase_contract_binds_webviews_and_the_attempted_second_process() {
        let plan = parse_plan().unwrap();
        let mut detached_webview = passed_desktop_phase(DesktopPhase::FinalPrimary);
        detached_webview
            .observation
            .as_mut()
            .expect("observation")
            .process_tree[0]
            .parent_process_id = Some(999);
        assert_eq!(
            validate_desktop_phase_result(&detached_webview, &plan),
            Err("lifecycle_desktop_phase_process_tree_invalid".to_string())
        );

        let mut unpinned_webview = passed_desktop_phase(DesktopPhase::FinalPrimary);
        unpinned_webview
            .observation
            .as_mut()
            .expect("observation")
            .process_tree[0]
            .executable_path = r"C:\Temp\msedgewebview2.exe".to_string();
        assert_eq!(
            validate_desktop_phase_result(&unpinned_webview, &plan),
            Err("lifecycle_desktop_phase_webview_identity_invalid".to_string())
        );

        let mut malformed_webview_version = passed_desktop_phase(DesktopPhase::FinalPrimary);
        malformed_webview_version
            .observation
            .as_mut()
            .expect("observation")
            .process_tree[0]
            .executable_path =
            r"C:\Program Files (x86)\Microsoft\EdgeWebView\Application\.\msedgewebview2.exe"
                .to_string();
        assert_eq!(
            validate_desktop_phase_result(&malformed_webview_version, &plan),
            Err("lifecycle_desktop_phase_webview_identity_invalid".to_string())
        );

        let mut forged_second = passed_desktop_phase(DesktopPhase::BaselineSecondInstance);
        forged_second
            .observation
            .as_mut()
            .expect("observation")
            .second_instance
            .as_mut()
            .expect("second instance")
            .service_instance_id_after = "00000065-00000000000000000000000000000002".to_string();
        assert_eq!(
            validate_desktop_phase_result(&forged_second, &plan),
            Err("lifecycle_desktop_second_instance_invalid".to_string())
        );

        let mut colliding_second = passed_desktop_phase(DesktopPhase::BaselineSecondInstance);
        colliding_second
            .observation
            .as_mut()
            .expect("observation")
            .second_instance
            .as_mut()
            .expect("second instance")
            .attempted_process
            .process_id = 102;
        assert_eq!(
            validate_desktop_phase_result(&colliding_second, &plan),
            Err("lifecycle_desktop_process_role_collision".to_string())
        );

        let mut colliding_service = passed_desktop_phase(DesktopPhase::FinalPrimary);
        let observation = colliding_service.observation.as_mut().expect("observation");
        let service = observation
            .collector_runtime
            .service_process
            .as_mut()
            .expect("service");
        service.process_id = 102;
        service.started_at_100ns = 10_200;
        observation.collector_runtime.pipe_server_process_id = Some(102);
        assert_eq!(
            validate_desktop_phase_result(&colliding_service, &plan),
            Err("lifecycle_desktop_process_role_collision".to_string())
        );

        let mut forged_pipe = passed_desktop_phase(DesktopPhase::FinalPrimary);
        forged_pipe
            .observation
            .as_mut()
            .expect("observation")
            .collector_runtime
            .pipe_server_process_id = Some(999);
        assert_eq!(
            validate_desktop_phase_result(&forged_pipe, &plan),
            Err("lifecycle_desktop_service_runtime_invalid".to_string())
        );
    }

    #[test]
    fn failed_desktop_phase_requires_a_bounded_code_and_allows_unsettled_truth() {
        let failed = DesktopPhaseResult {
            phase: DesktopPhase::FinalPrimary,
            disposition: DesktopPhaseDisposition::Failed,
            process_tree_settled: false,
            observation: None,
            failure_reason: Some("lifecycle_desktop_process_settlement_unproven".to_string()),
        };
        assert_eq!(
            validate_desktop_phase_result(&failed, &parse_plan().unwrap()),
            Ok(())
        );

        let mut hostile = failed;
        hostile.failure_reason = Some(r"C:\Users\albert\private evidence".to_string());
        assert_eq!(
            validate_desktop_phase_result(&hostile, &parse_plan().unwrap()),
            Err("lifecycle_desktop_phase_failure_reason_invalid".to_string())
        );
    }

    #[test]
    fn envelopes_reject_nonce_schema_and_sequence_drift() {
        let nonce = "a".repeat(NONCE_HEX_LENGTH);
        let mut gate = SequenceGate::new();
        let valid = Envelope {
            schema_version: PROTOCOL_SCHEMA.to_string(),
            nonce: nonce.clone(),
            sequence: 1,
            message_sha256: String::new(),
            message: ParentMessage::Begin(ClosedRequest {
                plan_sha256: "b".repeat(64),
                controller_source_commit_sha: "c".repeat(40),
                controller_sha256: "d".repeat(64),
                parent_process_id: 123,
                parent_started_at_100ns: 456,
            }),
        };
        let mut valid = valid;
        valid.message_sha256 = message_sha256(&valid.message).expect("message digest");
        assert!(validate_envelope(&valid, &nonce, &mut gate).is_ok());

        let mut wrong = valid;
        wrong.nonce = "e".repeat(NONCE_HEX_LENGTH);
        assert_eq!(
            validate_envelope(&wrong, &nonce, &mut gate),
            Err("lifecycle_protocol_nonce_invalid".to_string())
        );
    }

    #[test]
    fn envelopes_reject_message_tampering_and_injected_authority() {
        let nonce = "a".repeat(NONCE_HEX_LENGTH);
        let message = ParentMessage::Begin(ClosedRequest {
            plan_sha256: "b".repeat(64),
            controller_source_commit_sha: "c".repeat(40),
            controller_sha256: "d".repeat(64),
            parent_process_id: 123,
            parent_started_at_100ns: 456,
        });
        let envelope = Envelope {
            schema_version: PROTOCOL_SCHEMA.to_string(),
            nonce: nonce.clone(),
            sequence: 1,
            message_sha256: message_sha256(&message).expect("message digest"),
            message,
        };
        let mut tampered = envelope.clone();
        let ParentMessage::Begin(request) = &mut tampered.message else {
            unreachable!("fixed begin message");
        };
        request.parent_process_id += 1;
        assert_eq!(
            validate_envelope(&tampered, &nonce, &mut SequenceGate::new()),
            Err("lifecycle_protocol_message_digest_invalid".to_string())
        );

        let mut value = serde_json::to_value(envelope).expect("envelope value");
        value
            .as_object_mut()
            .expect("envelope object")
            .insert("command".to_string(), serde_json::json!("powershell.exe"));
        assert!(serde_json::from_value::<Envelope<ParentMessage>>(value).is_err());
    }

    #[test]
    fn abort_messages_reject_unauthenticated_and_malformed_authority() {
        let nonce = "a".repeat(NONCE_HEX_LENGTH);
        let message = ParentMessage::Abort(AbortReason::Timeout);
        let envelope = Envelope {
            schema_version: PROTOCOL_SCHEMA.to_string(),
            nonce: nonce.clone(),
            sequence: FIRST_SEQUENCE,
            message_sha256: message_sha256(&message).expect("abort digest"),
            message,
        };
        let mut unauthenticated = envelope.clone();
        unauthenticated.nonce = "b".repeat(NONCE_HEX_LENGTH);
        assert_eq!(
            validate_envelope(&unauthenticated, &nonce, &mut SequenceGate::new()),
            Err("lifecycle_protocol_nonce_invalid".to_string())
        );

        let mut malformed = serde_json::to_value(envelope).expect("abort envelope");
        malformed["message"]["value"] = serde_json::json!("run_arbitrary_command");
        assert!(serde_json::from_value::<Envelope<ParentMessage>>(malformed).is_err());
    }

    #[test]
    fn protocol_binds_the_evidence_root_and_requires_result_acceptance() {
        let accepted = WorkerMessage::Accepted(WorkerAccepted {
            evidence_root:
                r"C:\ProgramData\BatCaveMonitor\lifecycle-proof\aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
                    .to_string(),
            evidence_root_identity: EvidenceRootIdentity {
                volume_serial: 17,
                file_index: 29,
            },
            worker_process_id: 41,
            worker_started_at_100ns: 43,
        });
        let accepted_value = serde_json::to_value(&accepted).expect("accepted message");
        assert_eq!(accepted_value["kind"], "accepted");
        assert_eq!(
            accepted_value["value"]["evidence_root_identity"],
            serde_json::json!({
                "volume_serial": 17,
                "file_index": 29,
            })
        );

        let result_ready = WorkerMessage::ResultReady(Box::new(WorkerResult {
            disposition: WorkerDisposition::Failed,
            completed_stage: None,
            last_authenticated_checkpoint: None,
            abort: None,
            failure: Some(WorkerFailure {
                kind: WorkerFailureKind::Controller,
                attempted_stage: None,
                reason: "controller_failed".to_string(),
                evidence: None,
                evidence_error: None,
                restoration: Box::new(RestorationOutcome::NotRequired),
            }),
            process_tree_settled: true,
            private_evidence: Vec::new(),
            sanitized_export: None,
        }));
        assert_eq!(
            serde_json::to_value(&result_ready).expect("result ready")["kind"],
            "result_ready"
        );
        assert_eq!(
            serde_json::to_value(ParentMessage::EvidenceAccepted).expect("evidence acceptance"),
            serde_json::json!({"kind": "evidence_accepted"})
        );

        let mut injected_identity = accepted_value;
        injected_identity["value"]["evidence_root_identity"]["path"] =
            serde_json::json!(r"C:\other");
        assert!(serde_json::from_value::<WorkerMessage>(injected_identity).is_err());
        assert!(serde_json::from_value::<WorkerMessage>(serde_json::json!({
            "kind": "complete",
            "value": {}
        }))
        .is_err());
    }

    #[test]
    fn desktop_wire_contract_rejects_privileged_synthetic_fields() {
        let result = passed_desktop_phase(DesktopPhase::BaselineSecondInstance);
        let value = serde_json::to_value(&result).expect("desktop result");
        let observation = &value["observation"];
        assert!(observation["collector_runtime"]
            .get("service_etw")
            .is_none());
        assert!(observation["collector_runtime"]
            .get("failure_marker_present")
            .is_none());
        assert!(observation["second_instance"]
            .get("collector_generation_before")
            .is_none());
        assert!(observation["second_instance"]
            .get("collector_generation_after")
            .is_none());

        let mut injected_etw = value.clone();
        injected_etw["observation"]["collector_runtime"]["service_etw"] =
            serde_json::json!({"lease_generation": 7});
        assert!(serde_json::from_value::<DesktopPhaseResult>(injected_etw).is_err());

        let mut injected_generation = value;
        injected_generation["observation"]["second_instance"]["collector_generation_before"] =
            serde_json::json!(7);
        assert!(serde_json::from_value::<DesktopPhaseResult>(injected_generation).is_err());
    }

    #[test]
    fn observation_wire_contract_rejects_unknown_sibling_fields() {
        let present = serde_json::json!({
            "state": "present",
            "value": 7,
            "unexpected": true
        });
        assert!(serde_json::from_value::<Observation<u32>>(present).is_err());

        let absent = serde_json::json!({
            "state": "absent",
            "unexpected": true
        });
        assert!(serde_json::from_value::<Observation<u32>>(absent).is_err());

        let unknown = serde_json::json!({
            "state": "unknown",
            "value": "probe_failed",
            "unexpected": true
        });
        assert!(serde_json::from_value::<Observation<u32>>(unknown).is_err());
    }

    fn passed_desktop_phase(phase: DesktopPhase) -> DesktopPhaseResult {
        let plan = parse_plan().expect("plan");
        let state = phase.expected_collector_state();
        let active = state == DesktopCollectorState::Active;
        let incompatible = state == DesktopCollectorState::Incompatible;
        DesktopPhaseResult {
            phase,
            disposition: DesktopPhaseDisposition::Passed,
            process_tree_settled: true,
            observation: Some(DesktopPhaseObservation {
                desktop: desktop_process(
                    101,
                    None,
                    r"C:\Program Files\BatCave Monitor\batcave-monitor.exe",
                    phase.expected_monitor_sha256(&plan),
                ),
                process_tree: vec![desktop_process(
                    102,
                    Some(101),
                    r"C:\Program Files (x86)\Microsoft\EdgeWebView\Application\1\msedgewebview2.exe",
                    &"b".repeat(64),
                )],
                webview_process_ids: vec![102],
                second_instance: phase.expects_existing_primary_focus().then(|| {
                    DesktopSecondInstanceObservation {
                        attempted_process: desktop_process(
                            103,
                            None,
                            r"C:\Program Files\BatCave Monitor\batcave-monitor.exe",
                            phase.expected_monitor_sha256(&plan),
                        ),
                        terminal_exit_code: 0,
                        process_tree_settled: true,
                        focused_primary_process_id: 101,
                        focused_primary_started_at_100ns: 10_100,
                        service_instance_id_before: "00000065-00000000000000000000000000000001"
                            .to_string(),
                        service_instance_id_after: "00000065-00000000000000000000000000000001"
                            .to_string(),
                    }
                }),
                collector_runtime: collector_runtime(phase, &plan),
                visible: DesktopVisibleObservation {
                    current_process_standard: true,
                    collector_state: state,
                    privileged_source: if active {
                        DesktopPrivilegedSource::InstalledCollectorService
                    } else {
                        DesktopPrivilegedSource::None
                    },
                    standard_monitoring_current: !active,
                    protected_sample_current: active,
                    fallback_etw_disabled: !active,
                    service_version: active
                        .then(|| env!("CARGO_PKG_VERSION").to_string())
                        .or_else(|| incompatible.then(|| "0.2.0-rc.3".to_string())),
                    service_release_version: active
                        .then(|| env!("CARGO_PKG_VERSION").to_string())
                        .or_else(|| incompatible.then(|| "0.2.0-rc.3".to_string())),
                    negotiated_protocol_version: active.then_some(1),
                    minimum_desktop_version: active.then(|| env!("CARGO_PKG_VERSION").to_string()),
                    service_instance_id: active
                        .then(|| "00000065-00000000000000000000000000000001".to_string()),
                    service_detail: if incompatible {
                        Some("collector_service_desktop_release_incompatible".to_string())
                    } else {
                        match phase {
                            DesktopPhase::FinalMissingService => {
                                Some("collector_service_open_failed:1060".to_string())
                            }
                            DesktopPhase::FinalStoppedService => {
                                Some("collector_service_stopped".to_string())
                            }
                            _ => None,
                        }
                    },
                },
            }),
            failure_reason: None,
        }
    }

    fn collector_runtime(
        phase: DesktopPhase,
        plan: &ProofPlan,
    ) -> DesktopCollectorRuntimeObservation {
        if phase == DesktopPhase::FinalMissingService {
            return DesktopCollectorRuntimeObservation {
                installed_service: None,
                service_process: None,
                pipe_server_process_id: None,
            };
        }
        let (sha256, size) = if phase == DesktopPhase::FinalIncompatibleService {
            (
                plan.incompatible_service_fixture.sha256.clone(),
                plan.incompatible_service_fixture.size,
            )
        } else if matches!(
            phase,
            DesktopPhase::BaselinePrimary | DesktopPhase::BaselineSecondInstance
        ) {
            (plan.baseline.service_sha256.clone(), 8192)
        } else {
            (plan.final_candidate.service_sha256.clone(), 8192)
        };
        let installed_service = DesktopFileObservation {
            executable_path: r"C:\Program Files\BatCave Monitor\batcave-collector-service.exe"
                .to_string(),
            executable_size: size,
            executable_sha256: sha256.clone(),
        };
        if phase == DesktopPhase::FinalStoppedService {
            return DesktopCollectorRuntimeObservation {
                installed_service: Some(installed_service),
                service_process: None,
                pipe_server_process_id: None,
            };
        }
        DesktopCollectorRuntimeObservation {
            installed_service: Some(installed_service.clone()),
            service_process: Some(DesktopServiceProcessObservation {
                process_id: 55,
                started_at_100ns: 5_500,
                local_system: true,
                executable_path: installed_service.executable_path,
                executable_size: installed_service.executable_size,
                executable_sha256: installed_service.executable_sha256,
            }),
            pipe_server_process_id: Some(55),
        }
    }

    fn desktop_process(
        process_id: u32,
        parent_process_id: Option<u32>,
        executable_path: &str,
        executable_sha256: &str,
    ) -> DesktopProcessObservation {
        DesktopProcessObservation {
            process_id,
            parent_process_id,
            started_at_100ns: u64::from(process_id) * 100,
            session_id: 1,
            elevated: false,
            executable_path: executable_path.to_string(),
            executable_size: 4096,
            executable_sha256: executable_sha256.to_string(),
        }
    }
}
