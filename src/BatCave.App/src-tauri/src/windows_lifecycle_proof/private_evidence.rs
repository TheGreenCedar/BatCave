use super::native::{ElevatedMachineSnapshot, ProtectedEvidenceRoot};
use crate::collector_service::windows_provisioner::{
    FailedUpgradeRollbackForProof, TerminatedServiceForProof,
};
use crate::windows_lifecycle_proof_contract::{
    DesktopPhase, DesktopPhaseDisposition, DesktopPhaseResult, EvidenceReceipt,
};
use serde::{Deserialize, Serialize};

const PRIVATE_SUCCESS_SCHEMA: &str = "batcave_windows_lifecycle_private_success_v1";
const MAX_PRIVATE_EVIDENCE_SIZE: usize = 8 * 1024 * 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PrivatePayloadBinding {
    Machine,
    Desktop(DesktopPhase),
    ServiceCrash,
    UpgradeRollback,
}

const PRIVATE_SUCCESS_BINDINGS: [(&str, PrivatePayloadBinding); 28] = [
    ("initial-state.private.json", PrivatePayloadBinding::Machine),
    (
        "final-repair-state.private.json",
        PrivatePayloadBinding::Machine,
    ),
    (
        "final-primary-desktop.private.json",
        PrivatePayloadBinding::Desktop(DesktopPhase::FinalPrimary),
    ),
    (
        "initial-uninstall-state.private.json",
        PrivatePayloadBinding::Machine,
    ),
    (
        "baseline-install-state.private.json",
        PrivatePayloadBinding::Machine,
    ),
    (
        "baseline-primary-desktop.private.json",
        PrivatePayloadBinding::Desktop(DesktopPhase::BaselinePrimary),
    ),
    (
        "baseline-second-instance-desktop.private.json",
        PrivatePayloadBinding::Desktop(DesktopPhase::BaselineSecondInstance),
    ),
    (
        "baseline-restart-stopped-state.private.json",
        PrivatePayloadBinding::Machine,
    ),
    (
        "baseline-restart-state.private.json",
        PrivatePayloadBinding::Machine,
    ),
    (
        "baseline-crashed-state.private.json",
        PrivatePayloadBinding::ServiceCrash,
    ),
    (
        "baseline-crash-recovery-state.private.json",
        PrivatePayloadBinding::Machine,
    ),
    (
        "baseline-rollback-recovery-state.private.json",
        PrivatePayloadBinding::UpgradeRollback,
    ),
    (
        "legacy-residue-seeded-state.private.json",
        PrivatePayloadBinding::Machine,
    ),
    (
        "final-upgrade-state.private.json",
        PrivatePayloadBinding::Machine,
    ),
    (
        "final-restart-stopped-state.private.json",
        PrivatePayloadBinding::Machine,
    ),
    (
        "final-restart-state.private.json",
        PrivatePayloadBinding::Machine,
    ),
    (
        "final-crashed-state.private.json",
        PrivatePayloadBinding::ServiceCrash,
    ),
    (
        "final-crash-recovery-state.private.json",
        PrivatePayloadBinding::Machine,
    ),
    (
        "final-missing-service-state.private.json",
        PrivatePayloadBinding::Machine,
    ),
    (
        "final-missing-service-desktop.private.json",
        PrivatePayloadBinding::Desktop(DesktopPhase::FinalMissingService),
    ),
    (
        "final-missing-service-restored-state.private.json",
        PrivatePayloadBinding::Machine,
    ),
    (
        "final-stopped-service-state.private.json",
        PrivatePayloadBinding::Machine,
    ),
    (
        "final-stopped-service-desktop.private.json",
        PrivatePayloadBinding::Desktop(DesktopPhase::FinalStoppedService),
    ),
    (
        "final-stopped-service-restored-state.private.json",
        PrivatePayloadBinding::Machine,
    ),
    (
        "final-incompatible-service-state.private.json",
        PrivatePayloadBinding::Machine,
    ),
    (
        "final-incompatible-service-desktop.private.json",
        PrivatePayloadBinding::Desktop(DesktopPhase::FinalIncompatibleService),
    ),
    (
        "final-incompatible-service-restored-state.private.json",
        PrivatePayloadBinding::Machine,
    ),
    (
        "final-uninstall-state.private.json",
        PrivatePayloadBinding::Machine,
    ),
];

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct PrivateSuccessPacket {
    schema_version: String,
    payload: PrivateSuccessPayload,
}

impl PrivateSuccessPacket {
    pub(super) fn payload(&self) -> &PrivateSuccessPayload {
        &self.payload
    }
}

#[cfg(test)]
pub(super) fn packet_for_test(payload: PrivateSuccessPayload) -> PrivateSuccessPacket {
    PrivateSuccessPacket {
        schema_version: PRIVATE_SUCCESS_SCHEMA.to_string(),
        payload,
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(
    deny_unknown_fields,
    rename_all = "snake_case",
    tag = "kind",
    content = "value"
)]
pub(super) enum PrivateSuccessPayload {
    Machine(ElevatedMachineSnapshot),
    Desktop(Box<PrivateDesktopPayload>),
    ServiceCrash(PrivateServiceCrashPayload),
    UpgradeRollback(PrivateUpgradeRollbackPayload),
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct PrivateDesktopPayload {
    pub(super) machine: ElevatedMachineSnapshot,
    pub(super) result: DesktopPhaseResult,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct PrivateServiceCrashPayload {
    pub(super) machine: ElevatedMachineSnapshot,
    pub(super) termination: TerminatedServiceForProof,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct PrivateUpgradeRollbackPayload {
    pub(super) machine: ElevatedMachineSnapshot,
    pub(super) rollback: FailedUpgradeRollbackForProof,
}

pub(super) fn write_machine_packet(
    evidence: &ProtectedEvidenceRoot,
    name: &'static str,
    machine: &ElevatedMachineSnapshot,
) -> Result<EvidenceReceipt, String> {
    write_packet(
        evidence,
        name,
        PrivateSuccessPayload::Machine(machine.clone()),
    )
}

pub(super) fn write_desktop_packet(
    evidence: &ProtectedEvidenceRoot,
    name: &'static str,
    machine: &ElevatedMachineSnapshot,
    result: &DesktopPhaseResult,
) -> Result<EvidenceReceipt, String> {
    write_packet(
        evidence,
        name,
        PrivateSuccessPayload::Desktop(Box::new(PrivateDesktopPayload {
            machine: machine.clone(),
            result: result.clone(),
        })),
    )
}

pub(super) fn write_service_crash_packet(
    evidence: &ProtectedEvidenceRoot,
    name: &'static str,
    machine: &ElevatedMachineSnapshot,
    termination: &TerminatedServiceForProof,
) -> Result<EvidenceReceipt, String> {
    write_packet(
        evidence,
        name,
        PrivateSuccessPayload::ServiceCrash(PrivateServiceCrashPayload {
            machine: machine.clone(),
            termination: termination.clone(),
        }),
    )
}

pub(super) fn write_upgrade_rollback_packet(
    evidence: &ProtectedEvidenceRoot,
    name: &'static str,
    machine: &ElevatedMachineSnapshot,
    rollback: &FailedUpgradeRollbackForProof,
) -> Result<EvidenceReceipt, String> {
    write_packet(
        evidence,
        name,
        PrivateSuccessPayload::UpgradeRollback(PrivateUpgradeRollbackPayload {
            machine: machine.clone(),
            rollback: rollback.clone(),
        }),
    )
}

fn write_packet(
    evidence: &ProtectedEvidenceRoot,
    name: &'static str,
    payload: PrivateSuccessPayload,
) -> Result<EvidenceReceipt, String> {
    validate_payload_for_leaf(name, &payload)?;
    let packet = PrivateSuccessPacket {
        schema_version: PRIVATE_SUCCESS_SCHEMA.to_string(),
        payload,
    };
    let bytes = validated_pretty_packet_bytes(name, &packet)?;
    evidence.write_bytes_new(name, &bytes)
}

pub(super) fn parse_private_success_packet(
    name: &str,
    bytes: &[u8],
) -> Result<PrivateSuccessPacket, String> {
    parse_private_success_packet_with_limit(name, bytes, MAX_PRIVATE_EVIDENCE_SIZE)
}

fn validated_pretty_packet_bytes(
    name: &str,
    packet: &PrivateSuccessPacket,
) -> Result<Vec<u8>, String> {
    let bytes = serde_json::to_vec_pretty(packet)
        .map_err(|_| "lifecycle_private_success_serialize_failed".to_string())?;
    parse_private_success_packet(name, &bytes)?;
    Ok(bytes)
}

#[cfg(test)]
fn validated_pretty_packet_bytes_with_limit(
    name: &str,
    packet: &PrivateSuccessPacket,
    maximum_size: usize,
) -> Result<Vec<u8>, String> {
    let bytes = serde_json::to_vec_pretty(packet)
        .map_err(|_| "lifecycle_private_success_serialize_failed".to_string())?;
    parse_private_success_packet_with_limit(name, &bytes, maximum_size)?;
    Ok(bytes)
}

fn parse_private_success_packet_with_limit(
    name: &str,
    bytes: &[u8],
    maximum_size: usize,
) -> Result<PrivateSuccessPacket, String> {
    if bytes.is_empty() || bytes.len() > maximum_size {
        return Err("lifecycle_private_success_size_invalid".to_string());
    }
    let packet: PrivateSuccessPacket = serde_json::from_slice(bytes)
        .map_err(|_| "lifecycle_private_success_json_invalid".to_string())?;
    if packet.schema_version != PRIVATE_SUCCESS_SCHEMA {
        return Err("lifecycle_private_success_schema_invalid".to_string());
    }
    validate_payload_for_leaf(name, &packet.payload)?;
    Ok(packet)
}

fn validate_payload_for_leaf(name: &str, payload: &PrivateSuccessPayload) -> Result<(), String> {
    let binding = PRIVATE_SUCCESS_BINDINGS
        .iter()
        .find_map(|(leaf, binding)| (*leaf == name).then_some(*binding))
        .ok_or_else(|| "lifecycle_private_success_leaf_invalid".to_string())?;
    let valid = match binding {
        PrivatePayloadBinding::Machine => matches!(payload, PrivateSuccessPayload::Machine(_)),
        PrivatePayloadBinding::Desktop(phase) => valid_desktop_payload(payload, phase),
        PrivatePayloadBinding::ServiceCrash => {
            matches!(payload, PrivateSuccessPayload::ServiceCrash(_))
        }
        PrivatePayloadBinding::UpgradeRollback => {
            matches!(payload, PrivateSuccessPayload::UpgradeRollback(_))
        }
    };
    if valid {
        Ok(())
    } else {
        Err("lifecycle_private_success_payload_invalid".to_string())
    }
}

fn valid_desktop_payload(payload: &PrivateSuccessPayload, expected: DesktopPhase) -> bool {
    matches!(
        payload,
        PrivateSuccessPayload::Desktop(desktop)
            if desktop.result.phase == expected
                && desktop.result.disposition == DesktopPhaseDisposition::Passed
                && desktop.result.process_tree_settled
                && desktop.result.observation.is_some()
                && desktop.result.failure_reason.is_none()
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collector_service::windows_provisioner::{
        AcePolicyForProof, InstalledBoundariesForProof, SecurityPolicyForProof,
        SecurityPrincipalForProof, ServiceTerminationTargetForProof, TerminatedServiceForProof,
    };
    use crate::windows_lifecycle_proof::native::{
        DirectorySnapshot, PreflightSnapshot, RegistryView,
    };
    use crate::windows_lifecycle_proof_contract::{
        DesktopCollectorRuntimeObservation, DesktopCollectorState, DesktopPhase,
        DesktopPhaseDisposition, DesktopPhaseObservation, DesktopPrivilegedSource,
        DesktopProcessObservation, DesktopVisibleObservation, Observation,
        SUCCESS_PRIVATE_EVIDENCE_LEAVES,
    };

    #[test]
    fn private_success_packet_round_trips_and_denies_unknown_fields() {
        let packet = PrivateSuccessPacket {
            schema_version: PRIVATE_SUCCESS_SCHEMA.to_string(),
            payload: PrivateSuccessPayload::Machine(machine()),
        };
        let bytes = serde_json::to_vec(&packet).expect("serialize private packet");
        assert_eq!(
            parse_private_success_packet("initial-state.private.json", &bytes),
            Ok(packet)
        );

        let mut value: serde_json::Value =
            serde_json::from_slice(&bytes).expect("parse private packet value");
        value
            .as_object_mut()
            .expect("packet object")
            .insert("unexpected".to_string(), serde_json::Value::Bool(true));
        assert_eq!(
            parse_private_success_packet(
                "initial-state.private.json",
                &serde_json::to_vec(&value).expect("serialize hostile packet")
            ),
            Err("lifecycle_private_success_json_invalid".to_string())
        );

        let mut nested_observation_extra: serde_json::Value =
            serde_json::from_slice(&bytes).expect("parse private packet value");
        nested_observation_extra["payload"]["value"]["machine"]["service"]
            .as_object_mut()
            .expect("machine service observation")
            .insert("unexpected".to_string(), serde_json::Value::Bool(true));
        assert_eq!(
            parse_private_success_packet(
                "initial-state.private.json",
                &serde_json::to_vec(&nested_observation_extra)
                    .expect("serialize hostile nested observation")
            ),
            Err("lifecycle_private_success_json_invalid".to_string())
        );

        let mut payload_extra: serde_json::Value =
            serde_json::from_slice(&bytes).expect("parse private packet value");
        payload_extra["payload"]
            .as_object_mut()
            .expect("payload object")
            .insert("unexpected".to_string(), serde_json::Value::Bool(true));
        assert_eq!(
            parse_private_success_packet(
                "initial-state.private.json",
                &serde_json::to_vec(&payload_extra).expect("serialize hostile payload")
            ),
            Err("lifecycle_private_success_json_invalid".to_string())
        );

        let desktop = PrivateSuccessPacket {
            schema_version: PRIVATE_SUCCESS_SCHEMA.to_string(),
            payload: PrivateSuccessPayload::Desktop(Box::new(PrivateDesktopPayload {
                machine: machine(),
                result: DesktopPhaseResult {
                    phase: DesktopPhase::FinalPrimary,
                    disposition: DesktopPhaseDisposition::Failed,
                    process_tree_settled: true,
                    observation: None,
                    failure_reason: Some("not_implemented".to_string()),
                },
            })),
        };
        let mut desktop_extra =
            serde_json::to_value(desktop).expect("serialize desktop packet value");
        desktop_extra["payload"]["value"]
            .as_object_mut()
            .expect("desktop payload value")
            .insert("unexpected".to_string(), serde_json::Value::Bool(true));
        assert_eq!(
            parse_private_success_packet(
                "final-primary-desktop.private.json",
                &serde_json::to_vec(&desktop_extra).expect("serialize hostile desktop payload")
            ),
            Err("lifecycle_private_success_json_invalid".to_string())
        );
    }

    #[test]
    fn private_success_leaf_selects_the_only_allowed_payload_kind() {
        let machine = machine();
        let packet = PrivateSuccessPacket {
            schema_version: PRIVATE_SUCCESS_SCHEMA.to_string(),
            payload: PrivateSuccessPayload::Machine(machine),
        };
        let bytes = serde_json::to_vec(&packet).expect("serialize private packet");
        assert_eq!(
            parse_private_success_packet("baseline-crashed-state.private.json", &bytes),
            Err("lifecycle_private_success_payload_invalid".to_string())
        );
        assert_eq!(
            parse_private_success_packet("not-in-the-manifest.private.json", &bytes),
            Err("lifecycle_private_success_leaf_invalid".to_string())
        );
    }

    #[test]
    fn private_success_packet_rejects_unknown_schema() {
        let packet = PrivateSuccessPacket {
            schema_version: "future".to_string(),
            payload: PrivateSuccessPayload::Machine(machine()),
        };
        assert_eq!(
            parse_private_success_packet(
                "initial-state.private.json",
                &serde_json::to_vec(&packet).expect("serialize private packet")
            ),
            Err("lifecycle_private_success_schema_invalid".to_string())
        );
    }

    #[test]
    fn private_success_writer_validates_the_exact_pretty_bytes_and_size() {
        let packet = PrivateSuccessPacket {
            schema_version: PRIVATE_SUCCESS_SCHEMA.to_string(),
            payload: PrivateSuccessPayload::Machine(machine()),
        };
        let compact = serde_json::to_vec(&packet).expect("serialize compact packet");
        let pretty = serde_json::to_vec_pretty(&packet).expect("serialize pretty packet");
        assert!(pretty.len() > compact.len());
        assert_eq!(
            validated_pretty_packet_bytes_with_limit(
                "initial-state.private.json",
                &packet,
                compact.len()
            ),
            Err("lifecycle_private_success_size_invalid".to_string())
        );
        assert_eq!(
            validated_pretty_packet_bytes_with_limit(
                "initial-state.private.json",
                &packet,
                pretty.len()
            ),
            Ok(pretty)
        );
    }

    #[test]
    fn private_packet_preserves_observed_acl_digests_exactly() {
        let mut machine = machine();
        machine.installed_boundaries = Observation::Present(InstalledBoundariesForProof {
            service_dacl_sha256: "a".repeat(64),
            service_aces: vec![AcePolicyForProof {
                principal: SecurityPrincipalForProof::InteractiveUsers,
                allow: true,
                inherit_only: false,
                object_inherit: false,
                container_inherit: false,
                mask: 4,
            }],
            service_data_root_dacl_sha256: "b".repeat(64),
            service_data_root: SecurityPolicyForProof {
                owner: SecurityPrincipalForProof::LocalSystem,
                dacl_protected: true,
                reparse: false,
                aces: Vec::new(),
            },
        });
        let packet = PrivateSuccessPacket {
            schema_version: PRIVATE_SUCCESS_SCHEMA.to_string(),
            payload: PrivateSuccessPayload::Machine(machine),
        };
        let bytes = validated_pretty_packet_bytes("initial-state.private.json", &packet)
            .expect("validate packet with ACL digests");
        let parsed =
            parse_private_success_packet("initial-state.private.json", &bytes).expect("parse");
        let PrivateSuccessPayload::Machine(parsed_machine) = parsed.payload else {
            panic!("expected machine packet");
        };
        let Observation::Present(boundaries) = parsed_machine.installed_boundaries else {
            panic!("expected installed boundaries");
        };
        assert_eq!(boundaries.service_dacl_sha256, "a".repeat(64));
        assert_eq!(boundaries.service_data_root_dacl_sha256, "b".repeat(64));
    }

    #[test]
    fn every_success_leaf_has_one_explicit_payload_binding() {
        let bound_leaves = PRIVATE_SUCCESS_BINDINGS
            .iter()
            .map(|(leaf, _)| *leaf)
            .collect::<Vec<_>>();
        assert_eq!(bound_leaves.as_slice(), SUCCESS_PRIVATE_EVIDENCE_LEAVES);

        let machine_count = PRIVATE_SUCCESS_BINDINGS
            .iter()
            .filter(|(_, binding)| *binding == PrivatePayloadBinding::Machine)
            .count();
        let desktop_phases = PRIVATE_SUCCESS_BINDINGS
            .iter()
            .filter_map(|(_, binding)| match binding {
                PrivatePayloadBinding::Desktop(phase) => Some(*phase),
                _ => None,
            })
            .collect::<Vec<_>>();
        let crash_count = PRIVATE_SUCCESS_BINDINGS
            .iter()
            .filter(|(_, binding)| *binding == PrivatePayloadBinding::ServiceCrash)
            .count();
        let rollback_count = PRIVATE_SUCCESS_BINDINGS
            .iter()
            .filter(|(_, binding)| *binding == PrivatePayloadBinding::UpgradeRollback)
            .count();
        assert_eq!(machine_count, 19);
        assert_eq!(
            desktop_phases,
            vec![
                DesktopPhase::FinalPrimary,
                DesktopPhase::BaselinePrimary,
                DesktopPhase::BaselineSecondInstance,
                DesktopPhase::FinalMissingService,
                DesktopPhase::FinalStoppedService,
                DesktopPhase::FinalIncompatibleService,
            ]
        );
        assert_eq!(crash_count, 2);
        assert_eq!(rollback_count, 1);

        for (leaf, binding) in PRIVATE_SUCCESS_BINDINGS {
            assert_eq!(validate_payload_for_leaf(leaf, &payload(binding)), Ok(()));
        }
    }

    fn payload(binding: PrivatePayloadBinding) -> PrivateSuccessPayload {
        match binding {
            PrivatePayloadBinding::Machine => PrivateSuccessPayload::Machine(machine()),
            PrivatePayloadBinding::Desktop(phase) => {
                PrivateSuccessPayload::Desktop(Box::new(PrivateDesktopPayload {
                    machine: machine(),
                    result: DesktopPhaseResult {
                        phase,
                        disposition: DesktopPhaseDisposition::Passed,
                        process_tree_settled: true,
                        observation: Some(DesktopPhaseObservation {
                            desktop: DesktopProcessObservation {
                                process_id: 10,
                                parent_process_id: None,
                                started_at_100ns: 20,
                                session_id: 1,
                                elevated: false,
                                executable_path:
                                    r"C:\Program Files\BatCave Monitor\batcave-monitor.exe"
                                        .to_string(),
                                executable_size: 30,
                                executable_sha256: "c".repeat(64),
                            },
                            process_tree: Vec::new(),
                            webview_process_ids: Vec::new(),
                            second_instance: None,
                            collector_runtime: DesktopCollectorRuntimeObservation {
                                installed_service: None,
                                service_process: None,
                                pipe_server_process_id: None,
                            },
                            visible: DesktopVisibleObservation {
                                current_process_standard: true,
                                collector_state: DesktopCollectorState::NotInstalled,
                                privileged_source: DesktopPrivilegedSource::None,
                                standard_monitoring_current: true,
                                protected_sample_current: false,
                                fallback_etw_disabled: true,
                                service_version: None,
                                service_release_version: None,
                                negotiated_protocol_version: None,
                                minimum_desktop_version: None,
                                service_instance_id: None,
                                service_detail: None,
                            },
                        }),
                        failure_reason: None,
                    },
                }))
            }
            PrivatePayloadBinding::ServiceCrash => {
                PrivateSuccessPayload::ServiceCrash(PrivateServiceCrashPayload {
                    machine: machine(),
                    termination: TerminatedServiceForProof {
                        target: ServiceTerminationTargetForProof {
                            process_id: 10,
                            process_started_at_100ns: 20,
                            image_path:
                                r"C:\Program Files\BatCave Monitor\batcave-collector-service.exe"
                                    .to_string(),
                            image_sha256: "d".repeat(64),
                        },
                        process_exit_code: 70,
                        win32_exit_code: 1066,
                        service_specific_exit_code: 1,
                    },
                })
            }
            PrivatePayloadBinding::UpgradeRollback => {
                PrivateSuccessPayload::UpgradeRollback(PrivateUpgradeRollbackPayload {
                    machine: machine(),
                    rollback: FailedUpgradeRollbackForProof {
                        candidate_sha256: "e".repeat(64),
                        candidate_failure_code: "collector_service_proof_candidate_start_failed"
                            .to_string(),
                        candidate_failure_detail: "candidate_failed".to_string(),
                        execution_marker_sha256: "f".repeat(64),
                        restored_sha256: "a".repeat(64),
                        restored_process_id: 10,
                    },
                })
            }
        }
    }

    fn machine() -> ElevatedMachineSnapshot {
        ElevatedMachineSnapshot {
            machine: PreflightSnapshot {
                service: Observation::Absent,
                install_root: Observation::Present(DirectorySnapshot {
                    identity: super::super::native::FileIdentity {
                        volume_serial: 1,
                        file_index: 2,
                    },
                    final_path: r"C:\Program Files\BatCave Monitor".to_string(),
                }),
                monitor: Observation::Absent,
                service_binary: Observation::Absent,
                uninstaller: Observation::Absent,
                legacy_cli: Observation::Absent,
                uninstall_registry: Observation::Present(super::super::native::RegistrySnapshot {
                    view: RegistryView::Registry64,
                    install_location: r"C:\Program Files\BatCave Monitor".to_string(),
                    display_version: "0.2.0-rc.2".to_string(),
                }),
                product_processes: Observation::Present(Vec::new()),
            },
            product_data_root: Observation::Absent,
            service_data_root: Observation::Absent,
            current_user_data_root: Observation::Absent,
            installed_boundaries: Observation::Absent,
            named_pipe: Observation::Absent,
            etw_lease: Observation::Absent,
            etw_session: Observation::Absent,
            etw_owner_lock:
                crate::collector_service::windows_provisioner::RuntimeLockObservation::Absent {},
            service_lifecycle_lock:
                crate::collector_service::windows_provisioner::RuntimeLockObservation::Absent {},
            service_install_residue:
                crate::collector_service::windows_provisioner::ServiceInstallResidueForProof {
                    service_registry_key: Observation::Absent,
                    service_data: Observation::Absent,
                    install: Observation::Present(
                        crate::collector_service::windows_provisioner::InstallResidueForProof {
                            volume_serial: 1,
                            file_index: 2,
                            staged_service_images: Vec::new(),
                            rollback_service_images: Vec::new(),
                            atomic_temporary_files: Vec::new(),
                            rollback_execution_marker: Observation::Absent,
                        },
                    ),
                },
            machine_registration:
                crate::collector_service::windows_provisioner::MachineRegistrationForProof {
                    product_key_64: Observation::Absent,
                    product_key_32: Observation::Absent,
                    app_path_key: Observation::Absent,
                    public_desktop_shortcut: Observation::Absent,
                    common_start_menu_shortcut: Observation::Absent,
                },
        }
    }
}
