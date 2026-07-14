use serde::Serialize;

use crate::{
    cli_args,
    contracts::{
        RuntimeInstallKind, RuntimePersistenceComponent, RuntimePersistenceDurability,
        RuntimePersistenceKind, RuntimePersistenceOperation, RuntimePersistenceOwner,
        RuntimePersistencePermissionState, RuntimePersistenceState, RuntimePlatform,
        RuntimeSnapshot, RuntimeUiPreferences,
    },
    protocol::{release_identity, RuntimeReleaseIdentityV3},
    runtime_store::RuntimeState,
};

const PROOF_FLAG: &str = "--current-user-persistence-proof";
const PHASE_ARG: &str = "--phase";
const PROOF_ENV: &str = "BATCAVE_CURRENT_USER_PERSISTENCE_PROOF";
const FORMAT_VERSION: u32 = 1;
const INITIAL_THEME: &str = "ember";
const INITIAL_HISTORY_POINT_LIMIT: u32 = 180;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum ProofPhase {
    Initialize,
    Restart,
    Degraded,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
struct CurrentUserPersistenceReceipt {
    format_version: u32,
    evidence_scope: &'static str,
    phase: ProofPhase,
    release_identity: RuntimeReleaseIdentityV3,
    platform: RuntimePlatform,
    architecture: &'static str,
    install_kind: RuntimeInstallKind,
    settings: Option<RuntimeUiPreferences>,
    health_degraded: bool,
    persistence_warning_present: bool,
    persistence: Option<SanitizedPersistence>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
struct SanitizedPersistence {
    state: RuntimePersistenceState,
    current_user_root: Option<SanitizedRoot>,
    components: Vec<SanitizedComponent>,
    suppressed_diagnostic_events: u64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
struct SanitizedRoot {
    directory_reported: bool,
    permission_state: RuntimePersistencePermissionState,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
struct SanitizedComponent {
    kind: RuntimePersistenceKind,
    state: RuntimePersistenceState,
    durability: RuntimePersistenceDurability,
    active_failure: Option<SanitizedFailure>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
struct SanitizedFailure {
    code: String,
    operation: RuntimePersistenceOperation,
    retryable: bool,
}

pub fn run_cli(args: &[String]) -> Option<i32> {
    if !args.iter().any(|arg| arg == PROOF_FLAG) {
        return None;
    }

    let proof_enabled = std::env::var(PROOF_ENV).is_ok_and(|value| value == "1");
    match run_from_args(args, proof_enabled) {
        Ok(receipt) => match serde_json::to_string(&receipt) {
            Ok(payload) => {
                println!("{payload}");
                Some(0)
            }
            Err(error) => {
                eprintln!("persistence_proof_serialize_failed:{error}");
                Some(1)
            }
        },
        Err(error) => {
            eprintln!("{error}");
            Some(2)
        }
    }
}

fn run_from_args(
    args: &[String],
    proof_enabled: bool,
) -> Result<CurrentUserPersistenceReceipt, String> {
    require_one(args, PROOF_FLAG)?;
    require_one(args, PHASE_ARG)?;
    let phase = parse_phase(args)?;
    if !proof_enabled {
        return Err(format!("persistence_proof_not_enabled:set_{PROOF_ENV}=1"));
    }
    cli_args::reject_unknown_args(args, &[PHASE_ARG], &[PROOF_FLAG])?;

    let state = RuntimeState::new()?;
    run_with_state(&state, phase)
}

fn require_one(args: &[String], argument: &str) -> Result<(), String> {
    let count = args
        .iter()
        .filter(|value| value.as_str() == argument)
        .count();
    if count == 1 {
        Ok(())
    } else {
        Err(format!("invalid_argument_count:{argument}:{count}"))
    }
}

fn run_with_state(
    state: &RuntimeState,
    phase: ProofPhase,
) -> Result<CurrentUserPersistenceReceipt, String> {
    let observation = match phase {
        ProofPhase::Initialize => state.set_ui_preferences(RuntimeUiPreferences {
            theme: INITIAL_THEME.to_string(),
            history_point_limit: INITIAL_HISTORY_POINT_LIMIT,
        }),
        ProofPhase::Restart | ProofPhase::Degraded => state.snapshot(),
    };
    let shutdown = state.shutdown();
    let snapshot = match (observation, shutdown) {
        (Ok(snapshot), Ok(())) => snapshot,
        (Err(error), Ok(())) => return Err(error),
        (Err(error), Err(shutdown_error)) => {
            return Err(format!(
                "{error}; persistence_proof_shutdown_failed:{shutdown_error}"
            ));
        }
        (Ok(_), Err(error)) => return Err(format!("persistence_proof_shutdown_failed:{error}")),
    };

    Ok(receipt_from_snapshot(phase, snapshot))
}

fn parse_phase(args: &[String]) -> Result<ProofPhase, String> {
    let value = args
        .windows(2)
        .find(|pair| pair[0] == PHASE_ARG)
        .map(|pair| pair[1].as_str())
        .ok_or_else(|| format!("missing_value_for_argument:{PHASE_ARG}"))?;
    match value {
        "initialize" => Ok(ProofPhase::Initialize),
        "restart" => Ok(ProofPhase::Restart),
        "degraded" => Ok(ProofPhase::Degraded),
        _ => Err(format!("invalid_argument:{PHASE_ARG}:{value}")),
    }
}

fn receipt_from_snapshot(
    phase: ProofPhase,
    snapshot: RuntimeSnapshot,
) -> CurrentUserPersistenceReceipt {
    let persistence = snapshot.persistence.map(|persistence| {
        let current_user_root = persistence
            .roots
            .into_iter()
            .find(|root| root.owner == RuntimePersistenceOwner::CurrentUser)
            .map(|root| SanitizedRoot {
                directory_reported: root.directory.is_some(),
                permission_state: root.permission_state,
            });
        let mut components = persistence
            .components
            .into_iter()
            .filter(|component| component.owner == RuntimePersistenceOwner::CurrentUser)
            .map(sanitize_component)
            .collect::<Vec<_>>();
        components.sort_by_key(|component| component_kind_order(component.kind));
        SanitizedPersistence {
            state: persistence.state,
            current_user_root,
            components,
            suppressed_diagnostic_events: persistence.suppressed_diagnostic_events,
        }
    });

    CurrentUserPersistenceReceipt {
        format_version: FORMAT_VERSION,
        evidence_scope: "packaged_current_user_persistence_observation",
        phase,
        release_identity: release_identity(),
        platform: snapshot.environment.platform,
        architecture: canonical_architecture(std::env::consts::ARCH),
        install_kind: snapshot.environment.install_kind,
        settings: snapshot.settings.ui_preferences,
        health_degraded: snapshot.health.degraded,
        persistence_warning_present: snapshot
            .warnings
            .iter()
            .any(|warning| warning.category == "persistence"),
        persistence,
    }
}

fn sanitize_component(component: RuntimePersistenceComponent) -> SanitizedComponent {
    SanitizedComponent {
        kind: component.kind,
        state: component.state,
        durability: component.durability,
        active_failure: component.active_failure.map(|failure| SanitizedFailure {
            code: failure.code,
            operation: failure.operation,
            retryable: failure.retryable,
        }),
    }
}

fn component_kind_order(kind: RuntimePersistenceKind) -> u8 {
    match kind {
        RuntimePersistenceKind::Diagnostics => 0,
        RuntimePersistenceKind::Settings => 1,
        RuntimePersistenceKind::WarmCache => 2,
        RuntimePersistenceKind::ServiceState => 3,
    }
}

fn canonical_architecture(architecture: &str) -> &'static str {
    match architecture {
        "x86_64" | "amd64" | "x64" => "x86_64",
        "aarch64" | "arm64" => "aarch64",
        "x86" | "i686" | "i586" => "x86",
        _ => "unknown",
    }
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        path::PathBuf,
        time::{SystemTime, UNIX_EPOCH},
    };

    use super::*;

    fn args(phase: &str) -> Vec<String> {
        vec![
            PROOF_FLAG.to_string(),
            PHASE_ARG.to_string(),
            phase.to_string(),
        ]
    }

    fn temporary_dir(test_name: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock")
            .as_nanos();
        std::env::temp_dir().join(format!(
            "batcave-persistence-proof-{test_name}-{}-{nonce}",
            std::process::id()
        ))
    }

    #[test]
    fn ignores_unrelated_cli_modes() {
        assert_eq!(run_cli(&["--benchmark".to_string()]), None);
    }

    #[test]
    fn requires_the_explicit_proof_sentinel() {
        assert_eq!(
            run_from_args(&args("restart"), false).unwrap_err(),
            format!("persistence_proof_not_enabled:set_{PROOF_ENV}=1")
        );
    }

    #[test]
    fn rejects_unknown_phases_and_arguments() {
        assert_eq!(
            parse_phase(&args("remove")).unwrap_err(),
            "invalid_argument:--phase:remove"
        );
        let mut unexpected = args("restart");
        unexpected.push("--output".to_string());
        assert_eq!(
            cli_args::reject_unknown_args(&unexpected, &[PHASE_ARG], &[PROOF_FLAG]).unwrap_err(),
            "unknown_argument:--output"
        );
        let mut duplicate = args("restart");
        duplicate.extend([PHASE_ARG.to_string(), "degraded".to_string()]);
        assert_eq!(
            run_from_args(&duplicate, true).unwrap_err(),
            "invalid_argument_count:--phase:2"
        );
    }

    #[test]
    fn initialize_and_restart_emit_sanitized_persistence_observations() {
        let base_dir = temporary_dir("restart");
        let initialize_state = RuntimeState::from_base_dir_manual(base_dir.clone()).unwrap();
        let initialized = run_with_state(&initialize_state, ProofPhase::Initialize).unwrap();
        assert_eq!(
            initialized.settings,
            Some(RuntimeUiPreferences {
                theme: INITIAL_THEME.to_string(),
                history_point_limit: INITIAL_HISTORY_POINT_LIMIT,
            })
        );

        let restart_state = RuntimeState::from_base_dir_manual(base_dir.clone()).unwrap();
        let restarted = run_with_state(&restart_state, ProofPhase::Restart).unwrap();
        assert_eq!(restarted.settings, initialized.settings);
        let payload = serde_json::to_string(&restarted).unwrap();
        assert!(!payload.contains(&base_dir.display().to_string()));
        assert!(payload.contains("\"directory_reported\":true"));
        assert!(payload.contains("\"permission_state\":\"verified\""));

        let _ = fs::remove_dir_all(base_dir);
    }

    #[test]
    fn corrupt_settings_are_reported_without_exposing_contents_or_paths() {
        let base_dir = temporary_dir("corrupt");
        fs::create_dir_all(&base_dir).unwrap();
        let corrupt = br#"{\"schema_version\":1,\"secret\":\"do-not-leak\""#;
        fs::write(base_dir.join("settings.json"), corrupt).unwrap();

        let state = RuntimeState::from_base_dir_manual(base_dir.clone()).unwrap();
        let receipt = run_with_state(&state, ProofPhase::Degraded).unwrap();
        let payload = serde_json::to_string(&receipt).unwrap();
        assert!(receipt.persistence_warning_present);
        assert!(!payload.contains("do-not-leak"));
        assert!(!payload.contains(&base_dir.display().to_string()));
        assert_eq!(fs::read(base_dir.join("settings.json")).unwrap(), corrupt);

        let _ = fs::remove_dir_all(base_dir);
    }
}
