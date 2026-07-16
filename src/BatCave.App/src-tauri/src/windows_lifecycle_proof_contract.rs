use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::{Component, Path};

pub(crate) const EMBEDDED_PLAN: &str = include_str!("windows_lifecycle_proof_plan.v1.json");
pub(crate) const PLAN_SCHEMA: &str = "batcave_windows_lifecycle_proof_plan_v1";
pub(crate) const PROTOCOL_SCHEMA: &str = "batcave_windows_lifecycle_proof_protocol_v1";
pub(crate) const NONCE_HEX_LENGTH: usize = 64;
pub(crate) const LOCATOR_HEX_LENGTH: usize = 32;
pub(crate) const FIRST_SEQUENCE: u64 = 1;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ProofPlan {
    pub schema_version: String,
    pub profile: String,
    pub sequence: u64,
    pub baseline: Candidate,
    pub final_candidate: Candidate,
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
    DesktopPhaseComplete(DesktopPhaseResult),
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
    RunDesktopPhase(DesktopPhase),
    Complete(WorkerResult),
    Failed(WorkerResult),
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
    pub worker_process_id: u32,
    pub worker_started_at_100ns: u64,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum DesktopPhase {
    BaselinePrimary,
    BaselineSecondInstance,
    FinalPrimary,
    FinalMissingService,
    FinalStoppedService,
    FinalIncompatibleService,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct DesktopPhaseResult {
    pub phase: DesktopPhase,
    pub disposition: DesktopPhaseDisposition,
    pub process_tree_settled: bool,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum DesktopPhaseDisposition {
    Passed,
    Failed,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct WorkerResult {
    pub disposition: WorkerDisposition,
    pub completed_stage: Option<LifecycleStage>,
    pub failure: Option<String>,
    pub process_tree_settled: bool,
    pub private_evidence_complete: bool,
    pub sanitized_export_complete: bool,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum WorkerDisposition {
    Passed,
    Failed,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum LifecycleStage {
    InitialState,
    FinalRepair,
    InitialUninstall,
    BaselineInstall,
    BaselineRestart,
    BaselineCrashRecovery,
    LegacyResidueSeeded,
    FinalUpgrade,
    FinalRestart,
    FinalCrashRecovery,
    FinalFallbackStates,
    FinalUninstall,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case", tag = "state", content = "value")]
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
}
