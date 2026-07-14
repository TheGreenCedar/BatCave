use serde::{Deserialize, Serialize};

pub(crate) const ETW_LEASE_SCHEMA_VERSION: u16 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum EtwLeasePhase {
    Intent,
    Active,
    Stopping,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct EtwControllerIdentityV1 {
    pub process_id: u32,
    pub process_started_at: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct EtwSessionIdentityV1 {
    pub name: String,
    pub provider_id: [u8; 16],
    pub session_flags: u64,
    pub configuration_digest: [u8; 32],
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct EtwLeaseV1 {
    pub schema_version: u16,
    pub phase: EtwLeasePhase,
    pub install_id: [u8; 16],
    pub service_generation: [u8; 16],
    pub service_instance_id: [u8; 16],
    pub boot_identity: [u8; 16],
    pub controller: EtwControllerIdentityV1,
    pub session: EtwSessionIdentityV1,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct EtwExpectedOwnerV1 {
    pub install_id: [u8; 16],
    pub service_generation: [u8; 16],
    pub boot_identity: [u8; 16],
    pub session: EtwSessionIdentityV1,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum EtwLeaseObservation {
    Absent,
    Corrupt,
    Untrusted,
    Trusted(EtwLeaseV1),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum EtwSessionObservation {
    Absent,
    Present(EtwSessionIdentityV1),
    QueryUnavailable,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum EtwControllerObservation {
    Absent,
    Present(EtwControllerIdentityV1),
    QueryUnavailable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum EtwReclaimAttempt {
    NotAttempted,
    StopFailed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum EtwLeaseConflict {
    CorruptLease,
    UntrustedLease,
    SchemaVersionMismatch,
    MalformedLease,
    InstallIdMismatch,
    ServiceGenerationMismatch,
    BootIdentityMismatch,
    LeaseSessionMismatch,
    SessionWithoutTrustedLease,
    ObservedSessionMismatch,
    ControllerStillActive,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum EtwRecoveryHold {
    SessionQueryUnavailable,
    ControllerQueryUnavailable,
    StopFailed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum EtwRecoveryDecision {
    StartFresh { discard_stale_lease: bool },
    ReclaimExact { phase: EtwLeasePhase },
    Conflict(EtwLeaseConflict),
    Retain(EtwRecoveryHold),
}

pub(crate) fn decide_etw_recovery(
    expected: &EtwExpectedOwnerV1,
    lease_observation: &EtwLeaseObservation,
    session_observation: &EtwSessionObservation,
    controller_observation: &EtwControllerObservation,
    reclaim_attempt: EtwReclaimAttempt,
) -> EtwRecoveryDecision {
    let lease = match lease_observation {
        EtwLeaseObservation::Absent => {
            return match session_observation {
                EtwSessionObservation::Absent => EtwRecoveryDecision::StartFresh {
                    discard_stale_lease: false,
                },
                EtwSessionObservation::Present(_) => {
                    EtwRecoveryDecision::Conflict(EtwLeaseConflict::SessionWithoutTrustedLease)
                }
                EtwSessionObservation::QueryUnavailable => {
                    EtwRecoveryDecision::Retain(EtwRecoveryHold::SessionQueryUnavailable)
                }
            };
        }
        EtwLeaseObservation::Corrupt => {
            return EtwRecoveryDecision::Conflict(EtwLeaseConflict::CorruptLease);
        }
        EtwLeaseObservation::Untrusted => {
            return EtwRecoveryDecision::Conflict(EtwLeaseConflict::UntrustedLease);
        }
        EtwLeaseObservation::Trusted(lease) => lease,
    };

    if lease.schema_version != ETW_LEASE_SCHEMA_VERSION {
        return EtwRecoveryDecision::Conflict(EtwLeaseConflict::SchemaVersionMismatch);
    }
    if !lease_is_well_formed(lease) {
        return EtwRecoveryDecision::Conflict(EtwLeaseConflict::MalformedLease);
    }
    if lease.install_id != expected.install_id {
        return EtwRecoveryDecision::Conflict(EtwLeaseConflict::InstallIdMismatch);
    }
    if lease.service_generation != expected.service_generation {
        return EtwRecoveryDecision::Conflict(EtwLeaseConflict::ServiceGenerationMismatch);
    }
    if lease.session != expected.session {
        return EtwRecoveryDecision::Conflict(EtwLeaseConflict::LeaseSessionMismatch);
    }
    if lease.boot_identity != expected.boot_identity {
        return match session_observation {
            EtwSessionObservation::Absent => EtwRecoveryDecision::StartFresh {
                discard_stale_lease: true,
            },
            EtwSessionObservation::Present(_) => {
                EtwRecoveryDecision::Conflict(EtwLeaseConflict::BootIdentityMismatch)
            }
            EtwSessionObservation::QueryUnavailable => {
                EtwRecoveryDecision::Retain(EtwRecoveryHold::SessionQueryUnavailable)
            }
        };
    }
    if matches!(session_observation, EtwSessionObservation::QueryUnavailable) {
        return EtwRecoveryDecision::Retain(EtwRecoveryHold::SessionQueryUnavailable);
    }

    match controller_observation {
        EtwControllerObservation::QueryUnavailable => {
            return EtwRecoveryDecision::Retain(EtwRecoveryHold::ControllerQueryUnavailable);
        }
        EtwControllerObservation::Present(controller) if controller == &lease.controller => {
            return EtwRecoveryDecision::Conflict(EtwLeaseConflict::ControllerStillActive);
        }
        EtwControllerObservation::Absent | EtwControllerObservation::Present(_) => {}
    }

    if reclaim_attempt == EtwReclaimAttempt::StopFailed {
        return EtwRecoveryDecision::Retain(EtwRecoveryHold::StopFailed);
    }

    match session_observation {
        EtwSessionObservation::Absent => EtwRecoveryDecision::StartFresh {
            discard_stale_lease: true,
        },
        EtwSessionObservation::Present(session) if session == &lease.session => {
            EtwRecoveryDecision::ReclaimExact { phase: lease.phase }
        }
        EtwSessionObservation::Present(_) => {
            EtwRecoveryDecision::Conflict(EtwLeaseConflict::ObservedSessionMismatch)
        }
        EtwSessionObservation::QueryUnavailable => unreachable!("handled above"),
    }
}

fn lease_is_well_formed(lease: &EtwLeaseV1) -> bool {
    lease.install_id != [0; 16]
        && lease.service_generation != [0; 16]
        && lease.service_instance_id != [0; 16]
        && lease.boot_identity != [0; 16]
        && lease.controller.process_id != 0
        && lease.controller.process_started_at != 0
        && !lease.session.name.is_empty()
        && lease.session.name.len() <= 128
        && !lease.session.name.contains('\0')
        && lease.session.provider_id != [0; 16]
        && lease.session.session_flags != 0
        && lease.session.configuration_digest != [0; 32]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fresh_start_requires_both_lease_and_session_to_be_absent() {
        assert_eq!(
            decide(
                EtwLeaseObservation::Absent,
                EtwSessionObservation::Absent,
                EtwControllerObservation::QueryUnavailable,
                EtwReclaimAttempt::NotAttempted,
            ),
            EtwRecoveryDecision::StartFresh {
                discard_stale_lease: false,
            }
        );
        assert_eq!(
            decide(
                EtwLeaseObservation::Absent,
                EtwSessionObservation::Present(session()),
                EtwControllerObservation::Absent,
                EtwReclaimAttempt::NotAttempted,
            ),
            EtwRecoveryDecision::Conflict(EtwLeaseConflict::SessionWithoutTrustedLease)
        );
    }

    #[test]
    fn every_crash_phase_reclaims_only_the_exact_session_for_a_dead_controller() {
        for phase in [
            EtwLeasePhase::Intent,
            EtwLeasePhase::Active,
            EtwLeasePhase::Stopping,
        ] {
            let lease = lease(phase);
            assert_eq!(
                decide(
                    EtwLeaseObservation::Trusted(lease.clone()),
                    EtwSessionObservation::Present(lease.session),
                    EtwControllerObservation::Absent,
                    EtwReclaimAttempt::NotAttempted,
                ),
                EtwRecoveryDecision::ReclaimExact { phase }
            );
        }
    }

    #[test]
    fn every_crash_phase_discards_a_stale_lease_only_after_session_absence_is_proven() {
        for phase in [
            EtwLeasePhase::Intent,
            EtwLeasePhase::Active,
            EtwLeasePhase::Stopping,
        ] {
            assert_eq!(
                decide(
                    EtwLeaseObservation::Trusted(lease(phase)),
                    EtwSessionObservation::Absent,
                    EtwControllerObservation::Absent,
                    EtwReclaimAttempt::NotAttempted,
                ),
                EtwRecoveryDecision::StartFresh {
                    discard_stale_lease: true,
                }
            );
        }
    }

    #[test]
    fn live_exact_controller_blocks_reclaim_and_replacement() {
        let lease = lease(EtwLeasePhase::Active);
        assert_eq!(
            decide(
                EtwLeaseObservation::Trusted(lease.clone()),
                EtwSessionObservation::Present(lease.session.clone()),
                EtwControllerObservation::Present(lease.controller),
                EtwReclaimAttempt::NotAttempted,
            ),
            EtwRecoveryDecision::Conflict(EtwLeaseConflict::ControllerStillActive)
        );
    }

    #[test]
    fn reused_pid_with_a_different_creation_time_proves_the_old_controller_is_dead() {
        let lease = lease(EtwLeasePhase::Active);
        let reused = EtwControllerIdentityV1 {
            process_id: lease.controller.process_id,
            process_started_at: lease.controller.process_started_at + 1,
        };
        assert_eq!(
            decide(
                EtwLeaseObservation::Trusted(lease.clone()),
                EtwSessionObservation::Present(lease.session),
                EtwControllerObservation::Present(reused),
                EtwReclaimAttempt::NotAttempted,
            ),
            EtwRecoveryDecision::ReclaimExact {
                phase: EtwLeasePhase::Active,
            }
        );
    }

    #[test]
    fn corrupt_and_untrusted_leases_are_never_overwritten() {
        for (observation, conflict) in [
            (EtwLeaseObservation::Corrupt, EtwLeaseConflict::CorruptLease),
            (
                EtwLeaseObservation::Untrusted,
                EtwLeaseConflict::UntrustedLease,
            ),
        ] {
            for session in [
                EtwSessionObservation::Absent,
                EtwSessionObservation::QueryUnavailable,
            ] {
                assert_eq!(
                    decide(
                        observation.clone(),
                        session,
                        EtwControllerObservation::Absent,
                        EtwReclaimAttempt::NotAttempted,
                    ),
                    EtwRecoveryDecision::Conflict(conflict)
                );
            }
        }
    }

    #[test]
    fn malformed_and_schema_mismatched_leases_fail_closed() {
        let mut wrong_schema = lease(EtwLeasePhase::Intent);
        wrong_schema.schema_version += 1;
        assert_conflict(wrong_schema, EtwLeaseConflict::SchemaVersionMismatch);

        for mutate in [
            |lease: &mut EtwLeaseV1| lease.service_instance_id = [0; 16],
            |lease: &mut EtwLeaseV1| lease.controller.process_id = 0,
            |lease: &mut EtwLeaseV1| lease.session.configuration_digest = [0; 32],
        ] {
            let mut malformed = lease(EtwLeasePhase::Intent);
            mutate(&mut malformed);
            assert_conflict(malformed, EtwLeaseConflict::MalformedLease);
        }
    }

    #[test]
    fn install_generation_and_session_mismatches_never_reclaim() {
        type LeaseMutation = fn(&mut EtwLeaseV1);
        let cases: [(LeaseMutation, EtwLeaseConflict); 3] = [
            (
                |lease| lease.install_id[0] ^= 1,
                EtwLeaseConflict::InstallIdMismatch,
            ),
            (
                |lease| lease.service_generation[0] ^= 1,
                EtwLeaseConflict::ServiceGenerationMismatch,
            ),
            (
                |lease| lease.session.configuration_digest[0] ^= 1,
                EtwLeaseConflict::LeaseSessionMismatch,
            ),
        ];

        for (mutate, expected_conflict) in cases {
            let mut candidate = lease(EtwLeasePhase::Active);
            mutate(&mut candidate);
            assert_conflict(candidate, expected_conflict);
        }
    }

    #[test]
    fn prior_boot_lease_is_discarded_only_after_current_boot_session_absence_is_proven() {
        let mut prior_boot = lease(EtwLeasePhase::Active);
        prior_boot.boot_identity[0] ^= 1;
        assert_eq!(
            decide(
                EtwLeaseObservation::Trusted(prior_boot.clone()),
                EtwSessionObservation::Absent,
                EtwControllerObservation::QueryUnavailable,
                EtwReclaimAttempt::NotAttempted,
            ),
            EtwRecoveryDecision::StartFresh {
                discard_stale_lease: true,
            }
        );
        assert_eq!(
            decide(
                EtwLeaseObservation::Trusted(prior_boot.clone()),
                EtwSessionObservation::Present(prior_boot.session.clone()),
                EtwControllerObservation::Absent,
                EtwReclaimAttempt::NotAttempted,
            ),
            EtwRecoveryDecision::Conflict(EtwLeaseConflict::BootIdentityMismatch)
        );
        assert_eq!(
            decide(
                EtwLeaseObservation::Trusted(prior_boot),
                EtwSessionObservation::QueryUnavailable,
                EtwControllerObservation::Absent,
                EtwReclaimAttempt::NotAttempted,
            ),
            EtwRecoveryDecision::Retain(EtwRecoveryHold::SessionQueryUnavailable)
        );

        let mut mismatched = lease(EtwLeasePhase::Active);
        mismatched.boot_identity[0] ^= 1;
        mismatched.session.session_flags ^= 1;
        assert_eq!(
            decide(
                EtwLeaseObservation::Trusted(mismatched),
                EtwSessionObservation::Absent,
                EtwControllerObservation::Absent,
                EtwReclaimAttempt::NotAttempted,
            ),
            EtwRecoveryDecision::Conflict(EtwLeaseConflict::LeaseSessionMismatch)
        );
    }

    #[test]
    fn exact_name_prefix_or_provider_alone_never_authorizes_reclaim() {
        let lease = lease(EtwLeasePhase::Active);
        let mut prefix_only = lease.session.clone();
        prefix_only.name.push_str("-foreign");
        assert_eq!(
            decide(
                EtwLeaseObservation::Trusted(lease.clone()),
                EtwSessionObservation::Present(prefix_only),
                EtwControllerObservation::Absent,
                EtwReclaimAttempt::NotAttempted,
            ),
            EtwRecoveryDecision::Conflict(EtwLeaseConflict::ObservedSessionMismatch)
        );

        let mut provider_only = lease.session.clone();
        provider_only.configuration_digest[0] ^= 1;
        assert_eq!(
            decide(
                EtwLeaseObservation::Trusted(lease),
                EtwSessionObservation::Present(provider_only),
                EtwControllerObservation::Absent,
                EtwReclaimAttempt::NotAttempted,
            ),
            EtwRecoveryDecision::Conflict(EtwLeaseConflict::ObservedSessionMismatch)
        );
    }

    #[test]
    fn unavailable_session_or_controller_truth_retains_ownership_state() {
        let lease = lease(EtwLeasePhase::Stopping);
        assert_eq!(
            decide(
                EtwLeaseObservation::Trusted(lease.clone()),
                EtwSessionObservation::QueryUnavailable,
                EtwControllerObservation::Absent,
                EtwReclaimAttempt::NotAttempted,
            ),
            EtwRecoveryDecision::Retain(EtwRecoveryHold::SessionQueryUnavailable)
        );
        assert_eq!(
            decide(
                EtwLeaseObservation::Trusted(lease.clone()),
                EtwSessionObservation::Present(lease.session),
                EtwControllerObservation::QueryUnavailable,
                EtwReclaimAttempt::NotAttempted,
            ),
            EtwRecoveryDecision::Retain(EtwRecoveryHold::ControllerQueryUnavailable)
        );
    }

    #[test]
    fn stop_failure_preserves_the_lease_and_blocks_replacement() {
        let lease = lease(EtwLeasePhase::Stopping);
        for session_observation in [
            EtwSessionObservation::Absent,
            EtwSessionObservation::Present(lease.session.clone()),
        ] {
            assert_eq!(
                decide(
                    EtwLeaseObservation::Trusted(lease.clone()),
                    session_observation,
                    EtwControllerObservation::Absent,
                    EtwReclaimAttempt::StopFailed,
                ),
                EtwRecoveryDecision::Retain(EtwRecoveryHold::StopFailed)
            );
        }
    }

    #[test]
    fn lease_json_is_versioned_closed_and_uses_named_phases() {
        let lease = lease(EtwLeasePhase::Stopping);
        let json = serde_json::to_string(&lease).expect("serialize lease");
        assert!(json.contains("\"schema_version\":1"));
        assert!(json.contains("\"phase\":\"stopping\""));

        let mut value = serde_json::to_value(&lease).expect("lease value");
        value
            .as_object_mut()
            .expect("object")
            .insert("unexpected".to_string(), serde_json::Value::Bool(true));
        assert!(serde_json::from_value::<EtwLeaseV1>(value).is_err());
    }

    fn decide(
        lease: EtwLeaseObservation,
        session: EtwSessionObservation,
        controller: EtwControllerObservation,
        reclaim_attempt: EtwReclaimAttempt,
    ) -> EtwRecoveryDecision {
        decide_etw_recovery(&expected(), &lease, &session, &controller, reclaim_attempt)
    }

    fn assert_conflict(lease: EtwLeaseV1, expected_conflict: EtwLeaseConflict) {
        let session = lease.session.clone();
        assert_eq!(
            decide(
                EtwLeaseObservation::Trusted(lease),
                EtwSessionObservation::Present(session),
                EtwControllerObservation::Absent,
                EtwReclaimAttempt::NotAttempted,
            ),
            EtwRecoveryDecision::Conflict(expected_conflict)
        );
    }

    fn expected() -> EtwExpectedOwnerV1 {
        EtwExpectedOwnerV1 {
            install_id: [1; 16],
            service_generation: [2; 16],
            boot_identity: [4; 16],
            session: session(),
        }
    }

    fn lease(phase: EtwLeasePhase) -> EtwLeaseV1 {
        let expected = expected();
        EtwLeaseV1 {
            schema_version: ETW_LEASE_SCHEMA_VERSION,
            phase,
            install_id: expected.install_id,
            service_generation: expected.service_generation,
            service_instance_id: [3; 16],
            boot_identity: expected.boot_identity,
            controller: EtwControllerIdentityV1 {
                process_id: 41,
                process_started_at: 99,
            },
            session: expected.session,
        }
    }

    fn session() -> EtwSessionIdentityV1 {
        EtwSessionIdentityV1 {
            name: "BatCave Process Network v1".to_string(),
            provider_id: [5; 16],
            session_flags: 0x1000_0000,
            configuration_digest: [6; 32],
        }
    }
}
