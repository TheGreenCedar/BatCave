mod catalog;
mod encode;
mod types;
mod validate;

pub use encode::encode_snapshot;
pub(crate) use types::RuntimeReleaseIdentityV3;
pub use types::{
    ProcessFocusModeV3, ProtocolEnvelope, RuntimeQueryInputV3, RuntimeUiPreferencesV3,
    SortColumnV3, SortDirectionV3,
};

pub const RUNTIME_PROTOCOL_VERSION: u16 = 3;

pub(crate) fn release_identity() -> RuntimeReleaseIdentityV3 {
    RuntimeReleaseIdentityV3 {
        app_version: env!("CARGO_PKG_VERSION").to_string(),
        source_commit_sha: option_env!("BATCAVE_SOURCE_COMMIT_SHA")
            .filter(|value| !value.is_empty())
            .map(str::to_string),
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::catalog::{CatalogBuilder, MetricDefinition};
    use super::types::*;
    use super::{encode::encode_snapshot_at, encode_snapshot, validate::validate_envelope};
    use crate::contracts::{
        GroupDetail, GroupDetailKind, GroupMetricCoverage, GroupMetricQuality, MetricCoverage,
        MetricLimitationCode, MetricQuality, MetricQualityInfo, MetricSource,
        ProcessContributorIdentity, ProcessContributorSummary, ProcessDetail, ProcessDetailKind,
        ProcessMetricQuality, ProcessSample, ProcessViewRow, RuntimeAdminModeState,
        RuntimeInstallKind, RuntimePersistence as ContractPersistence,
        RuntimePersistenceComponent as ContractPersistenceComponent,
        RuntimePersistenceDurability as ContractPersistenceDurability,
        RuntimePersistenceKind as ContractPersistenceKind,
        RuntimePersistenceOwner as ContractPersistenceOwner,
        RuntimePersistencePermissionState as ContractPersistencePermissionState,
        RuntimePersistenceRoot as ContractPersistenceRoot,
        RuntimePersistenceState as ContractPersistenceState, RuntimePlatform,
        RuntimePrivilegedSource, RuntimeProcessElevation, RuntimeSnapshot, RuntimeUiPreferences,
        SystemMetricQuality,
    };
    use ts_rs::{Config, TS};

    fn generated_typescript() -> String {
        let config = Config::default();
        let declarations = [
            Compatibility::decl(&config),
            ProtocolMismatchReason::decl(&config),
            ProtocolMismatchPayload::decl(&config),
            RuntimePlatformV3::decl(&config),
            RuntimeArchitectureV3::decl(&config),
            RuntimeProcessElevationV3::decl(&config),
            RuntimeInstallKindV3::decl(&config),
            RuntimeReleaseIdentityV3::decl(&config),
            PrivilegedCollectionStateV3::decl(&config),
            PrivilegedCollectionSourceV3::decl(&config),
            PrivilegedCollectionPreferenceV3::decl(&config),
            CollectorServiceStatusV3::decl(&config),
            CollectorServiceStateV3::decl(&config),
            RuntimeEnvironmentV3::decl(&config),
            RuntimePrivilegedCollectionV3::decl(&config),
            ProcessFocusModeV3::decl(&config),
            SortColumnV3::decl(&config),
            SortDirectionV3::decl(&config),
            RuntimeQueryInputV3::decl(&config),
            RuntimeQueryV3::decl(&config),
            RuntimeSettingsV3::decl(&config),
            RuntimeUiPreferencesV3::decl(&config),
            RuntimeHealthV3::decl(&config),
            RuntimeEngineStateV3::decl(&config),
            RuntimeCollectorStateV3::decl(&config),
            RuntimeFatalErrorV3::decl(&config),
            RuntimePersistenceV3::decl(&config),
            RuntimePersistenceStateV3::decl(&config),
            RuntimePersistenceRootV3::decl(&config),
            RuntimePersistenceOwnerV3::decl(&config),
            RuntimePersistencePermissionStateV3::decl(&config),
            RuntimePersistenceComponentV3::decl(&config),
            RuntimePersistenceKindV3::decl(&config),
            RuntimePersistenceDurabilityV3::decl(&config),
            RuntimePersistenceFailureV3::decl(&config),
            RuntimePersistenceOperationV3::decl(&config),
            MetricSemantic::decl(&config),
            MetricScope::decl(&config),
            MetricUnit::decl(&config),
            MetricSourceV3::decl(&config),
            MetricQualityV3::decl(&config),
            MeasurementDescriptor::decl(&config),
            NetworkScopeV3::decl(&config),
            MetricObservation::decl(&config),
            LimitationCode::decl(&config),
            LimitationEntry::decl(&config),
            LogicalCpuDetailV3::decl(&config),
            KernelPoolKindV3::decl(&config),
            KernelPoolTagDetailV3::decl(&config),
            SystemDetailV3::decl(&config),
            ProcessIdentityStabilityV3::decl(&config),
            AccessStateV3::decl(&config),
            ProcessPresentationV3::decl(&config),
            ProcessDetailV3::decl(&config),
            GroupMetricCoverageV3::decl(&config),
            GroupDetailV3::decl(&config),
            WorkloadDetailV3::decl(&config),
            ContributorMetricV3::decl(&config),
            ProcessContributorV3::decl(&config),
            RuntimeWarningV3::decl(&config),
            RuntimeSnapshotPayloadV3::decl(&config),
            ProtocolEvent::decl(&config),
            ProtocolEnvelope::decl(&config),
        ]
        .map(|declaration| declaration.replacen("type ", "export type ", 1));
        format!(
            "// Generated from the production Rust protocol; do not edit by hand.\nexport const RUNTIME_PROTOCOL_VERSION = 3 as const;\n\n{}\n",
            declarations.join("\n\n")
        )
    }

    #[test]
    fn generated_typescript_matches_checked_contract() {
        if std::env::var_os("BATCAVE_UPDATE_PROTOCOL_GOLDENS").is_some() {
            std::fs::write(
                concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/../src/lib/generated/runtime-protocol-v3.ts"
                ),
                generated_typescript(),
            )
            .expect("write generated TypeScript protocol");
            return;
        }
        assert_eq!(
            generated_typescript(),
            include_str!("../../../src/lib/generated/runtime-protocol-v3.ts")
        );
    }

    fn quality(quality: MetricQuality, source: MetricSource) -> MetricQualityInfo {
        MetricQualityInfo::new(quality, source)
    }

    fn rewrite_first_process_pid(payload: &mut RuntimeSnapshotPayloadV3, pid: &str) {
        let (old_id, new_id) = {
            let process = payload
                .workloads
                .iter_mut()
                .find_map(|workload| match workload {
                    WorkloadDetailV3::Process(process) => Some(process),
                    WorkloadDetailV3::Group(_) => None,
                })
                .expect("process fixture");
            let old_id = process.stable_id.clone();
            let new_id = match process.start_time_ms {
                Some(start_time) => format!("process:{pid}:{start_time}"),
                None => format!("process:{pid}:publication:{}", payload.sample_seq),
            };
            process.pid = pid.to_string();
            process.stable_id.clone_from(&new_id);
            (old_id, new_id)
        };
        for workload in &mut payload.workloads {
            match workload {
                WorkloadDetailV3::Process(process)
                    if process.parent_process_id.as_deref() == Some(old_id.as_str()) =>
                {
                    process.parent_pid = Some(pid.to_string());
                    process.parent_process_id = Some(new_id.clone());
                }
                WorkloadDetailV3::Group(group) => {
                    for member_id in &mut group.member_ids {
                        if *member_id == old_id {
                            member_id.clone_from(&new_id);
                        }
                    }
                }
                WorkloadDetailV3::Process(_) => {}
            }
        }
        for contributor in &mut payload.contributors {
            if contributor.process_id.as_deref() == Some(old_id.as_str()) {
                contributor.process_id = Some(new_id.clone());
            }
        }
    }

    fn fixture_snapshot() -> RuntimeSnapshot {
        let mut snapshot: RuntimeSnapshot = serde_json::from_str(include_str!(
            "../../../scripts/fixtures/runtime-snapshot.v2.json"
        ))
        .expect("v2 compatibility fixture remains readable internally");
        snapshot.source = "protocol_fixture".to_string();
        snapshot.system.quality = Some(SystemMetricQuality {
            cpu: Some(quality(MetricQuality::Native, MetricSource::DirectApi)),
            kernel_cpu: Some(quality(MetricQuality::Native, MetricSource::DirectApi)),
            logical_cpu: Some(quality(MetricQuality::Native, MetricSource::DirectApi)),
            memory: Some(quality(MetricQuality::Native, MetricSource::DirectApi)),
            swap: Some(
                quality(MetricQuality::Unavailable, MetricSource::DirectApi).with_limitation(
                    MetricLimitationCode::UnsupportedMetric,
                    "Windows reports commit accounting instead of swap.",
                ),
            ),
            disk: Some(quality(MetricQuality::Native, MetricSource::Pdh)),
            network: Some(quality(
                MetricQuality::Native,
                MetricSource::InterfaceAggregate,
            )),
        });

        let process_quality = ProcessMetricQuality {
            cpu: Some(quality(MetricQuality::Estimated, MetricSource::Sysinfo)),
            memory: Some(quality(MetricQuality::Native, MetricSource::DirectApi)),
            io: Some(quality(MetricQuality::Native, MetricSource::DirectApi)),
            other_io: Some(
                quality(MetricQuality::Unavailable, MetricSource::DirectApi).with_limitation(
                    MetricLimitationCode::UnsupportedMetric,
                    "Other I/O is not reported by this fixture.",
                ),
            ),
            network: Some(
                quality(MetricQuality::Held, MetricSource::Etw).with_limitation(
                    MetricLimitationCode::PendingBaseline,
                    "Waiting for process network attribution.",
                ),
            ),
            threads: Some(quality(MetricQuality::Native, MetricSource::DirectApi)),
            handles: Some(quality(MetricQuality::Native, MetricSource::DirectApi)),
        };
        let mut first = snapshot.processes[0].clone();
        first.quality = Some(process_quality.clone());
        let mut second = first.clone();
        second.pid = "1235".to_string();
        second.start_time_ms += 1;
        second.cpu_percent = 4.25;
        second.memory_bytes = 48_000;
        second.private_bytes = 24_000;
        second.io_read_total_bytes = 100;
        second.io_write_total_bytes = 200;
        second.io_read_bps = 3;
        second.io_write_bps = 4;

        let process_row = |process: ProcessSample, is_child| ProcessViewRow::Process {
            detail: Box::new(ProcessDetail {
                kind: ProcessDetailKind::Process,
                workload_id: format!("process:{}:{}", process.pid, process.start_time_ms),
                io_bps: process.io_read_bps + process.io_write_bps,
                network_bps: 0,
                process,
            }),
            group_key: "batcave.app.exe".to_string(),
            group_label: "BatCave.App.exe".to_string(),
            group_category: "BatCave".to_string(),
            group_count: 2,
            icon_kind: "batcave".to_string(),
            is_child,
            is_grouped: true,
            attention_label: "steady".to_string(),
        };
        let group_quality = GroupMetricQuality {
            cpu: quality(MetricQuality::Estimated, MetricSource::ProcessAggregate),
            memory: quality(MetricQuality::Native, MetricSource::ProcessAggregate),
            io: quality(MetricQuality::Native, MetricSource::ProcessAggregate),
            other_io: quality(MetricQuality::Unavailable, MetricSource::ProcessAggregate)
                .with_limitation(
                    MetricLimitationCode::GroupPartialCoverage,
                    "0 of 2 processes contribute to this aggregate.",
                ),
            network: quality(MetricQuality::Unavailable, MetricSource::ProcessAggregate)
                .with_limitation(
                    MetricLimitationCode::GroupPartialCoverage,
                    "0 of 2 processes contribute to this aggregate.",
                ),
            threads: quality(MetricQuality::Native, MetricSource::ProcessAggregate),
        };
        let coverage = GroupMetricCoverage {
            cpu: MetricCoverage {
                available: 2,
                total: 2,
            },
            memory: MetricCoverage {
                available: 2,
                total: 2,
            },
            io: MetricCoverage {
                available: 2,
                total: 2,
            },
            other_io: MetricCoverage {
                available: 0,
                total: 2,
            },
            network: MetricCoverage {
                available: 0,
                total: 2,
            },
            threads: MetricCoverage {
                available: 2,
                total: 2,
            },
        };
        let group = ProcessViewRow::Group {
            detail: Box::new(GroupDetail {
                kind: GroupDetailKind::Group,
                workload_id: "group:batcave.app.exe".to_string(),
                group_key: "batcave.app.exe".to_string(),
                label: "BatCave.App.exe".to_string(),
                category: "BatCave".to_string(),
                process_count: 2,
                cpu_percent: first.cpu_percent + second.cpu_percent,
                memory_bytes: first.memory_bytes + second.memory_bytes,
                io_bps: first.io_read_bps
                    + first.io_write_bps
                    + second.io_read_bps
                    + second.io_write_bps,
                other_io_bps: None,
                network_bps: 0,
                threads: u64::from(first.threads + second.threads),
                quality: group_quality,
                coverage,
            }),
            icon_kind: "batcave".to_string(),
            icon_source: Some(first.exe.clone()),
            example_label: Some(first.name.clone()),
            attention_label: "CPU activity".to_string(),
        };
        snapshot.process_view_rows = vec![
            group,
            process_row(first.clone(), false),
            process_row(second.clone(), true),
        ];
        snapshot.processes = vec![first.clone(), second];
        snapshot.total_process_count = 2;
        snapshot.process_contributors = ProcessContributorSummary {
            cpu: Some("BatCave.App".to_string()),
            cpu_identity: Some(ProcessContributorIdentity {
                pid: first.pid.clone(),
                start_time_ms: first.start_time_ms,
            }),
            cpu_coverage: Some(MetricCoverage {
                available: 2,
                total: 2,
            }),
            cpu_quality: Some(quality(MetricQuality::Estimated, MetricSource::Sysinfo)),
            cpu_name_ambiguous: true,
            memory: Some("BatCave.App".to_string()),
            memory_identity: Some(ProcessContributorIdentity {
                pid: first.pid.clone(),
                start_time_ms: first.start_time_ms,
            }),
            memory_coverage: Some(MetricCoverage {
                available: 2,
                total: 2,
            }),
            memory_quality: Some(quality(MetricQuality::Native, MetricSource::DirectApi)),
            memory_name_ambiguous: true,
            io: Some("BatCave.App".to_string()),
            io_identity: Some(ProcessContributorIdentity {
                pid: first.pid.clone(),
                start_time_ms: first.start_time_ms,
            }),
            io_coverage: Some(MetricCoverage {
                available: 2,
                total: 2,
            }),
            io_quality: Some(quality(MetricQuality::Native, MetricSource::DirectApi)),
            io_name_ambiguous: true,
            network: None,
            network_identity: None,
            network_coverage: Some(MetricCoverage {
                available: 0,
                total: 2,
            }),
            network_quality: Some(
                quality(MetricQuality::Held, MetricSource::Etw).with_limitation(
                    MetricLimitationCode::PendingBaseline,
                    "Waiting for process network attribution.",
                ),
            ),
            network_name_ambiguous: false,
        };
        snapshot
    }

    fn fixture_for(platform: RuntimePlatform) -> RuntimeSnapshot {
        let mut snapshot = fixture_snapshot();
        snapshot.environment.platform = platform;
        match platform {
            RuntimePlatform::Windows => {}
            RuntimePlatform::Linux => {
                snapshot.environment.process_elevation = RuntimeProcessElevation::NotApplicable;
                snapshot.environment.install_kind = RuntimeInstallKind::Appimage;
                snapshot.environment.data_directory =
                    Some("/home/test/.local/share/BatCaveMonitor".to_string());
                let partial = quality(MetricQuality::Partial, MetricSource::Procfs)
                    .with_limitation(
                        MetricLimitationCode::AccessDenied,
                        "Some process fields were denied by procfs.",
                    );
                for process in &mut snapshot.processes {
                    process.quality.as_mut().expect("fixture quality").memory =
                        Some(partial.clone());
                }
                for row in &mut snapshot.process_view_rows {
                    if let ProcessViewRow::Process { detail, .. } = row {
                        detail
                            .process
                            .quality
                            .as_mut()
                            .expect("fixture quality")
                            .memory = Some(partial.clone());
                    }
                }
                snapshot.process_contributors.memory_quality = Some(partial);
            }
            RuntimePlatform::Macos => {
                snapshot.environment.admin_mode_available = false;
                snapshot.environment.process_elevation = RuntimeProcessElevation::NotApplicable;
                snapshot.environment.install_kind = RuntimeInstallKind::AppBundle;
                snapshot.environment.data_directory =
                    Some("/Users/test/Library/Application Support/BatCaveMonitor".to_string());
                snapshot.admin_mode.state = RuntimeAdminModeState::Unavailable;
                snapshot
                    .system
                    .quality
                    .as_mut()
                    .expect("fixture system quality")
                    .network = Some(quality(MetricQuality::Native, MetricSource::Sysinfo));
                let estimated_memory = quality(MetricQuality::Estimated, MetricSource::Sysinfo);
                for process in &mut snapshot.processes {
                    process.private_bytes = 0;
                    process.quality.as_mut().expect("fixture quality").memory =
                        Some(estimated_memory.clone());
                }
                for row in &mut snapshot.process_view_rows {
                    if let ProcessViewRow::Process { detail, .. } = row {
                        detail.process.private_bytes = 0;
                        detail
                            .process
                            .quality
                            .as_mut()
                            .expect("fixture quality")
                            .memory = Some(estimated_memory.clone());
                    }
                }
                snapshot.process_contributors.memory_quality = Some(estimated_memory);
            }
            RuntimePlatform::Fixture => unreachable!("goldens model real platforms"),
        }
        snapshot
    }

    fn encode_fixture(
        snapshot: RuntimeSnapshot,
        architecture: RuntimeArchitectureV3,
    ) -> ProtocolEnvelope {
        let evaluated_at_ms = snapshot.published_at_ms;
        encode_snapshot_at(snapshot, evaluated_at_ms, architecture).expect("fixture encodes")
    }

    fn json_with_newline(value: &impl serde::Serialize) -> String {
        format!(
            "{}\n",
            serde_json::to_string_pretty(value).expect("serialize protocol fixture")
        )
    }

    fn update_or_assert(path: &Path, expected: String, checked: &str) {
        if std::env::var_os("BATCAVE_UPDATE_PROTOCOL_GOLDENS").is_some() {
            std::fs::write(path, &expected).expect("write protocol fixture");
            return;
        }
        assert_eq!(expected, checked, "{} is stale", path.display());
    }

    #[test]
    fn encoder_publishes_runtime_owned_preferences_and_persistence_health() {
        let mut snapshot = fixture_snapshot();
        snapshot.settings.ui_preferences = Some(RuntimeUiPreferences {
            theme: "aurora".to_string(),
            history_point_limit: 180,
        });
        snapshot.persistence = Some(ContractPersistence {
            state: ContractPersistenceState::Healthy,
            roots: vec![ContractPersistenceRoot {
                owner: ContractPersistenceOwner::CurrentUser,
                directory: Some("/tmp/BatCaveMonitor".to_string()),
                permission_state: ContractPersistencePermissionState::Verified,
            }],
            components: vec![ContractPersistenceComponent {
                owner: ContractPersistenceOwner::CurrentUser,
                kind: ContractPersistenceKind::Settings,
                state: ContractPersistenceState::Healthy,
                durability: ContractPersistenceDurability::Durable,
                last_success_at_ms: Some(snapshot.published_at_ms),
                active_failure: None,
            }],
            suppressed_diagnostic_events: 0,
        });

        let envelope = encode_fixture(snapshot, RuntimeArchitectureV3::Aarch64);
        let ProtocolEvent::RuntimeSnapshot(payload) = envelope.event else {
            unreachable!()
        };
        let preferences = payload
            .settings
            .ui_preferences
            .expect("preferences are published");
        let persistence = payload.persistence.expect("persistence is published");

        assert_eq!(preferences.theme, "aurora");
        assert_eq!(preferences.history_point_limit, 180);
        assert_eq!(persistence.state, RuntimePersistenceStateV3::Healthy);
        assert_eq!(
            persistence.roots[0].owner,
            RuntimePersistenceOwnerV3::CurrentUser
        );
        assert_eq!(
            persistence.components[0].kind,
            RuntimePersistenceKindV3::Settings
        );
    }

    #[test]
    fn production_protocol_fixtures_match_encoder() {
        let fixture_dir =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("src/fixtures/runtime-protocol-v3");
        let windows = encode_fixture(
            fixture_for(RuntimePlatform::Windows),
            RuntimeArchitectureV3::X86_64,
        );
        update_or_assert(
            &fixture_dir.join("windows-standard.json"),
            json_with_newline(&windows),
            include_str!("../fixtures/runtime-protocol-v3/windows-standard.json"),
        );
        update_or_assert(
            &Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../scripts/fixtures/runtime-snapshot.v3.json"),
            json_with_newline(&windows),
            include_str!("../../../scripts/fixtures/runtime-snapshot.v3.json"),
        );

        let mut elevated = fixture_for(RuntimePlatform::Windows);
        elevated.environment.process_elevation = RuntimeProcessElevation::Elevated;
        elevated.admin_mode.state = RuntimeAdminModeState::Active;
        elevated.admin_mode.source = RuntimePrivilegedSource::ElevatedHelper;
        elevated.settings.admin_mode_requested = true;
        elevated.settings.admin_mode_enabled = true;
        update_or_assert(
            &fixture_dir.join("windows-elevated.json"),
            json_with_newline(&encode_fixture(elevated, RuntimeArchitectureV3::X86_64)),
            include_str!("../fixtures/runtime-protocol-v3/windows-elevated.json"),
        );
        update_or_assert(
            &fixture_dir.join("linux-partial.json"),
            json_with_newline(&encode_fixture(
                fixture_for(RuntimePlatform::Linux),
                RuntimeArchitectureV3::Aarch64,
            )),
            include_str!("../fixtures/runtime-protocol-v3/linux-partial.json"),
        );
        update_or_assert(
            &fixture_dir.join("macos-limited.json"),
            json_with_newline(&encode_fixture(
                fixture_for(RuntimePlatform::Macos),
                RuntimeArchitectureV3::Aarch64,
            )),
            include_str!("../fixtures/runtime-protocol-v3/macos-limited.json"),
        );

        let transitions = [
            (
                MetricQuality::Held,
                "Waiting for a compatible I/O baseline.",
            ),
            (MetricQuality::Native, ""),
            (MetricQuality::Held, "Publishing the previous I/O rate."),
            (
                MetricQuality::Unavailable,
                "I/O attribution is unavailable.",
            ),
        ]
        .into_iter()
        .map(|(state, message)| {
            let mut snapshot = fixture_for(RuntimePlatform::Windows);
            for row in &mut snapshot.process_view_rows {
                if let ProcessViewRow::Process { detail, .. } = row {
                    let mut value = quality(state, MetricSource::DirectApi);
                    if !message.is_empty() {
                        value = value.with_limitation(
                            match state {
                                MetricQuality::Held if message.contains("baseline") => {
                                    MetricLimitationCode::PendingBaseline
                                }
                                MetricQuality::Held => MetricLimitationCode::HeldValue,
                                MetricQuality::Unavailable => {
                                    MetricLimitationCode::UnsupportedMetric
                                }
                                _ => MetricLimitationCode::CollectorFailure,
                            },
                            message,
                        );
                    }
                    if state == MetricQuality::Held && !message.contains("baseline") {
                        value.updated_at_ms = snapshot.sampled_at_ms.map(|time| time - 1_000);
                    }
                    detail.process.quality.as_mut().expect("fixture quality").io = Some(value);
                }
            }
            encode_fixture(snapshot, RuntimeArchitectureV3::X86_64)
        })
        .collect::<Vec<_>>();
        update_or_assert(
            &fixture_dir.join("quality-transitions.json"),
            json_with_newline(&transitions),
            include_str!("../fixtures/runtime-protocol-v3/quality-transitions.json"),
        );
    }

    #[test]
    fn incompatible_fixture_is_explicit() {
        let incompatible = serde_json::json!({
            "protocol_version": 4,
            "compatibility": { "minimum_reader_version": 4, "breaking": true },
            "event": {
                "kind": "protocol_mismatch",
                "payload": {
                    "reason": "reader_too_old",
                    "writer_version": 4,
                    "minimum_reader_version": 4,
                    "message": "This fixture requires a newer BatCave reader."
                }
            }
        });
        update_or_assert(
            &Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("src/fixtures/runtime-protocol-v3/incompatible.json"),
            json_with_newline(&incompatible),
            include_str!("../fixtures/runtime-protocol-v3/incompatible.json"),
        );
    }

    #[test]
    fn validator_rejects_scope_value_and_reference_corruption() {
        let envelope = encode_snapshot(fixture_snapshot()).expect("fixture encodes");

        let mut wrong_scope = envelope.clone();
        let ProtocolEvent::RuntimeSnapshot(payload) = &mut wrong_scope.event else {
            unreachable!()
        };
        let descriptor_index = usize::from(payload.system.metrics[0].0);
        payload.descriptors[descriptor_index].scope = MetricScope::Process;
        assert_eq!(
            validate_envelope(&wrong_scope),
            Err("protocol_semantic_unit_invalid".to_string())
        );

        let mut unavailable_value = envelope.clone();
        let ProtocolEvent::RuntimeSnapshot(payload) = &mut unavailable_value.event else {
            unreachable!()
        };
        payload.system.metrics[0].1 = Some(1.0);
        payload.system.metrics[0].2 = 4;
        assert_eq!(
            validate_envelope(&unavailable_value),
            Err("protocol_unavailable_observation_has_value".to_string())
        );

        let mut negative_value = envelope.clone();
        let ProtocolEvent::RuntimeSnapshot(payload) = &mut negative_value.event else {
            unreachable!()
        };
        payload.system.metrics[0].1 = Some(-1.0);
        assert_eq!(
            validate_envelope(&negative_value),
            Err("protocol_observation_not_finite".to_string())
        );

        let mut held_without_explanation = envelope.clone();
        let ProtocolEvent::RuntimeSnapshot(payload) = &mut held_without_explanation.event else {
            unreachable!()
        };
        payload.system.metrics[0].2 = 2;
        payload.system.metrics[0].4 = None;
        assert_eq!(
            validate_envelope(&held_without_explanation),
            Err("protocol_quality_explanation_missing".to_string())
        );

        let mut dangling_member = envelope;
        let ProtocolEvent::RuntimeSnapshot(payload) = &mut dangling_member.event else {
            unreachable!()
        };
        let group = payload
            .workloads
            .iter_mut()
            .find_map(|workload| match workload {
                WorkloadDetailV3::Group(group) => Some(group),
                WorkloadDetailV3::Process(_) => None,
            })
            .expect("group fixture");
        group.member_ids[0] = "process:missing".to_string();
        assert_eq!(
            validate_envelope(&dangling_member),
            Err("protocol_dangling_group_member".to_string())
        );
    }

    #[test]
    fn validator_rejects_health_persistence_network_and_identity_corruption() {
        let envelope = encode_snapshot(fixture_snapshot()).expect("fixture encodes");

        let mut wrong_network_scope = envelope.clone();
        let ProtocolEvent::RuntimeSnapshot(payload) = &mut wrong_network_scope.event else {
            unreachable!()
        };
        let descriptor = payload
            .descriptors
            .iter_mut()
            .find(|descriptor| descriptor.semantic == MetricSemantic::NetworkReceiveTotal)
            .expect("network descriptor");
        descriptor.network_scope = Some(NetworkScopeV3::IpSocketPayload);
        assert_eq!(
            validate_envelope(&wrong_network_scope),
            Err("protocol_semantic_network_scope_invalid".to_string())
        );

        let mut fatal_without_error = envelope.clone();
        let ProtocolEvent::RuntimeSnapshot(payload) = &mut fatal_without_error.event else {
            unreachable!()
        };
        payload.health.engine_state = Some(RuntimeEngineStateV3::Fatal);
        assert_eq!(
            validate_envelope(&fatal_without_error),
            Err("protocol_fatal_state_without_error".to_string())
        );

        let mut nondegraded_fatal = envelope.clone();
        let ProtocolEvent::RuntimeSnapshot(payload) = &mut nondegraded_fatal.event else {
            unreachable!()
        };
        payload.health.engine_state = Some(RuntimeEngineStateV3::Fatal);
        payload.health.fatal_error = Some(RuntimeFatalErrorV3 {
            code: "runtime_failed".to_string(),
            message: "The runtime failed.".to_string(),
            occurred_at_ms: payload.published_at_ms,
        });
        assert_eq!(
            validate_envelope(&nondegraded_fatal),
            Err("protocol_health_degraded_state_invalid".to_string())
        );

        let mut nondegraded_limited = envelope.clone();
        let ProtocolEvent::RuntimeSnapshot(payload) = &mut nondegraded_limited.event else {
            unreachable!()
        };
        payload.health.engine_state = Some(RuntimeEngineStateV3::Running);
        payload.health.collector_state = Some(RuntimeCollectorStateV3::Limited);
        assert_eq!(
            validate_envelope(&nondegraded_limited),
            Err("protocol_health_degraded_state_invalid".to_string())
        );
        let ProtocolEvent::RuntimeSnapshot(payload) = &mut nondegraded_limited.event else {
            unreachable!()
        };
        payload.health.degraded = true;
        assert_eq!(validate_envelope(&nondegraded_limited), Ok(()));

        let mut nondegraded_unavailable = envelope.clone();
        let ProtocolEvent::RuntimeSnapshot(payload) = &mut nondegraded_unavailable.event else {
            unreachable!()
        };
        payload.health.engine_state = Some(RuntimeEngineStateV3::Running);
        payload.health.collector_state = Some(RuntimeCollectorStateV3::Unavailable);
        assert_eq!(
            validate_envelope(&nondegraded_unavailable),
            Err("protocol_health_degraded_state_invalid".to_string())
        );

        let mut healthy_without_persistence = envelope.clone();
        let ProtocolEvent::RuntimeSnapshot(payload) = &mut healthy_without_persistence.event else {
            unreachable!()
        };
        payload.persistence = Some(RuntimePersistenceV3 {
            state: RuntimePersistenceStateV3::Healthy,
            roots: Vec::new(),
            components: Vec::new(),
            suppressed_diagnostic_events: 0,
        });
        assert_eq!(
            validate_envelope(&healthy_without_persistence),
            Err("protocol_persistence_nonempty_state_invalid".to_string())
        );

        let mut service_without_identity = envelope.clone();
        let ProtocolEvent::RuntimeSnapshot(payload) = &mut service_without_identity.event else {
            unreachable!()
        };
        payload.privileged_collection.state = PrivilegedCollectionStateV3::Active;
        payload.privileged_collection.source = PrivilegedCollectionSourceV3::CollectorService;
        payload.privileged_collection.collector_service = Some(CollectorServiceStatusV3 {
            state: CollectorServiceStateV3::Active,
            release_identity: None,
            service_version: None,
            negotiated_protocol_version: None,
            minimum_desktop_version: None,
            instance_id: None,
            last_connected_at_ms: None,
            detail: None,
        });
        assert_eq!(
            validate_envelope(&service_without_identity),
            Err("protocol_collector_service_active_identity_invalid".to_string())
        );

        let mut matching_service = envelope.clone();
        let ProtocolEvent::RuntimeSnapshot(payload) = &mut matching_service.event else {
            unreachable!()
        };
        payload.privileged_collection.state = PrivilegedCollectionStateV3::Active;
        payload.privileged_collection.source = PrivilegedCollectionSourceV3::CollectorService;
        payload.privileged_collection.collector_service = Some(CollectorServiceStatusV3 {
            state: CollectorServiceStateV3::Active,
            release_identity: Some(payload.environment.release_identity.clone()),
            service_version: Some("1.0.0".to_string()),
            negotiated_protocol_version: Some(super::RUNTIME_PROTOCOL_VERSION),
            minimum_desktop_version: None,
            instance_id: Some("collector-instance".to_string()),
            last_connected_at_ms: Some(payload.health.evaluated_at_ms),
            detail: None,
        });
        assert_eq!(validate_envelope(&matching_service), Ok(()));

        let mut mismatched_service = matching_service;
        let ProtocolEvent::RuntimeSnapshot(payload) = &mut mismatched_service.event else {
            unreachable!()
        };
        payload
            .privileged_collection
            .collector_service
            .as_mut()
            .expect("collector service")
            .release_identity
            .as_mut()
            .expect("collector release identity")
            .app_version = "different".to_string();
        assert_eq!(
            validate_envelope(&mismatched_service),
            Err("protocol_collector_service_release_mismatch".to_string())
        );

        let mut invalid_release_identity = envelope.clone();
        let ProtocolEvent::RuntimeSnapshot(payload) = &mut invalid_release_identity.event else {
            unreachable!()
        };
        payload.environment.release_identity.source_commit_sha = Some("not-a-sha".to_string());
        assert_eq!(
            validate_envelope(&invalid_release_identity),
            Err("protocol_release_commit_invalid".to_string())
        );

        let mut multibyte_release_boundary = envelope.clone();
        let ProtocolEvent::RuntimeSnapshot(payload) = &mut multibyte_release_boundary.event else {
            unreachable!()
        };
        payload.environment.release_identity.app_version = "é".repeat(32);
        assert_eq!(validate_envelope(&multibyte_release_boundary), Ok(()));

        let mut multibyte_release_identity = envelope.clone();
        let ProtocolEvent::RuntimeSnapshot(payload) = &mut multibyte_release_identity.event else {
            unreachable!()
        };
        payload.environment.release_identity.app_version = "é".repeat(40);
        assert_eq!(
            validate_envelope(&multibyte_release_identity),
            Err("protocol_release_version_invalid".to_string())
        );

        for pid in ["0", "9007199254740991"] {
            let mut valid_pid = envelope.clone();
            let ProtocolEvent::RuntimeSnapshot(payload) = &mut valid_pid.event else {
                unreachable!()
            };
            rewrite_first_process_pid(payload, pid);
            assert_eq!(validate_envelope(&valid_pid), Ok(()));
        }

        for pid in [
            "01234",
            "9007199254740992",
            "999999999999999999999999999999999999",
            "-1",
            " 1234",
            "1234 ",
            "not-decimal",
        ] {
            let mut invalid_pid = envelope.clone();
            let ProtocolEvent::RuntimeSnapshot(payload) = &mut invalid_pid.event else {
                unreachable!()
            };
            rewrite_first_process_pid(payload, pid);
            assert_eq!(
                validate_envelope(&invalid_pid),
                Err("protocol_process_identity_invalid".to_string())
            );
        }

        let mut forged_identity = envelope;
        let ProtocolEvent::RuntimeSnapshot(payload) = &mut forged_identity.event else {
            unreachable!()
        };
        let process = payload
            .workloads
            .iter_mut()
            .find_map(|workload| match workload {
                WorkloadDetailV3::Process(process) => Some(process),
                WorkloadDetailV3::Group(_) => None,
            })
            .expect("process fixture");
        process.stable_id = format!("process:{}:1", process.pid);
        assert_eq!(
            validate_envelope(&forged_identity),
            Err("protocol_process_identity_invalid".to_string())
        );
    }

    #[test]
    fn descriptor_interval_can_override_the_settings_default() {
        let mut catalog = CatalogBuilder::new(1_000).expect("catalog");
        let quality = quality(MetricQuality::Native, MetricSource::Ebpf);
        let observation = catalog
            .observation(
                MetricDefinition::new(
                    MetricSemantic::NetworkReceiveRate,
                    MetricScope::Process,
                    MetricUnit::BytesPerSecond,
                )
                .with_interval_ms(250),
                Some(10.0),
                Some(&quality),
                Some(1_000),
            )
            .expect("observation");
        let descriptor = &catalog.descriptors[usize::from(observation.0)];
        assert_eq!(descriptor.interval_ms, Some(250));
        assert_eq!(
            descriptor.network_scope,
            Some(NetworkScopeV3::IpSocketPayload)
        );
    }

    #[test]
    fn baseline_holds_rates_without_erasing_cumulative_io() {
        let mut snapshot = fixture_snapshot();
        for row in &mut snapshot.process_view_rows {
            if let ProcessViewRow::Process { detail, .. } = row {
                detail.process.quality.as_mut().expect("fixture quality").io = Some(
                    quality(MetricQuality::Held, MetricSource::DirectApi).with_limitation(
                        MetricLimitationCode::PendingBaseline,
                        "Waiting for a compatible I/O baseline.",
                    ),
                );
            }
        }
        let envelope = encode_snapshot(snapshot).expect("held snapshot encodes");
        let ProtocolEvent::RuntimeSnapshot(payload) = envelope.event else {
            unreachable!()
        };
        let process = payload
            .workloads
            .iter()
            .find_map(|workload| match workload {
                WorkloadDetailV3::Process(process) => Some(process),
                WorkloadDetailV3::Group(_) => None,
            })
            .expect("process fixture");
        for semantic in [MetricSemantic::ReadIoTotal, MetricSemantic::WriteIoTotal] {
            let metric = process
                .metrics
                .iter()
                .find(|metric| payload.descriptors[usize::from(metric.0)].semantic == semantic)
                .expect("total metric");
            assert!(metric.1.is_some());
            assert_eq!(metric.2, 0);
        }
        for semantic in [MetricSemantic::ReadIoRate, MetricSemantic::WriteIoRate] {
            let metric = process
                .metrics
                .iter()
                .find(|metric| payload.descriptors[usize::from(metric.0)].semantic == semantic)
                .expect("rate metric");
            assert_eq!(metric.1, None);
            assert_eq!(metric.2, 2);
        }
    }

    #[test]
    fn contributor_attribution_is_independent_of_visible_query_rows() {
        let full = encode_snapshot(fixture_snapshot()).expect("full fixture encodes");
        let mut filtered_snapshot = fixture_snapshot();
        filtered_snapshot.processes.truncate(1);
        filtered_snapshot
            .process_view_rows
            .retain(|row| matches!(row, ProcessViewRow::Process { detail, .. } if detail.process.pid == "1235"));
        if let ProcessViewRow::Process {
            is_grouped,
            is_child,
            group_count,
            ..
        } = &mut filtered_snapshot.process_view_rows[0]
        {
            *is_grouped = false;
            *is_child = false;
            *group_count = 1;
        }
        let filtered = encode_snapshot(filtered_snapshot).expect("filtered fixture encodes");
        let ProtocolEvent::RuntimeSnapshot(full) = full.event else {
            unreachable!()
        };
        let ProtocolEvent::RuntimeSnapshot(filtered) = filtered.event else {
            unreachable!()
        };

        let cpu = full
            .contributors
            .iter()
            .find(|contributor| matches!(contributor.metric, ContributorMetricV3::Cpu))
            .expect("CPU contributor");
        assert_eq!(
            cpu.process_id.as_deref(),
            Some("process:1234:1699999999000")
        );
        assert!(cpu.name_ambiguous);

        let normalized = |payload: &RuntimeSnapshotPayloadV3| {
            payload
                .contributors
                .iter()
                .map(|contributor| {
                    serde_json::json!({
                        "metric": contributor.metric,
                        "process_id": contributor.process_id,
                        "display_name": contributor.display_name,
                        "name_ambiguous": contributor.name_ambiguous,
                        "available_contributors": contributor.available_contributors,
                        "total_contributors": contributor.total_contributors,
                        "quality_code": contributor.quality_code,
                        "source": contributor.source,
                        "limitation": contributor.limitation_index.map(|index| &payload.limitations[usize::from(index)]),
                    })
                })
                .collect::<Vec<_>>()
        };
        assert_eq!(normalized(&full), normalized(&filtered));
    }

    #[test]
    fn contributor_identity_handles_unknown_start_and_partial_coverage() {
        let mut unknown_start = fixture_snapshot();
        unknown_start
            .process_contributors
            .cpu_identity
            .as_mut()
            .expect("CPU identity")
            .start_time_ms = 0;
        let envelope = encode_snapshot(unknown_start).expect("unknown-start contributor encodes");
        let ProtocolEvent::RuntimeSnapshot(payload) = envelope.event else {
            unreachable!()
        };
        let cpu = payload
            .contributors
            .iter()
            .find(|contributor| matches!(contributor.metric, ContributorMetricV3::Cpu))
            .expect("CPU contributor");
        assert_eq!(
            cpu.process_id.as_deref(),
            Some("process:1234:publication:40")
        );

        let mut partial = fixture_snapshot();
        partial.process_contributors.cpu = None;
        partial.process_contributors.cpu_identity = None;
        partial.process_contributors.cpu_coverage = Some(MetricCoverage {
            available: 1,
            total: 2,
        });
        partial.process_contributors.cpu_quality = Some(
            quality(MetricQuality::Unavailable, MetricSource::DirectApi).with_limitation(
                MetricLimitationCode::PartialCoverage,
                "1 of 2 processes provide CPU telemetry.",
            ),
        );
        let envelope = encode_snapshot(partial).expect("partial contributor encodes");
        let ProtocolEvent::RuntimeSnapshot(payload) = envelope.event else {
            unreachable!()
        };
        let cpu = payload
            .contributors
            .iter()
            .find(|contributor| matches!(contributor.metric, ContributorMetricV3::Cpu))
            .expect("CPU contributor");
        assert_eq!(cpu.process_id, None);
        assert_eq!(cpu.available_contributors, 1);
        assert_eq!(cpu.total_contributors, 2);
        assert!(cpu.limitation_index.is_some());
    }

    #[test]
    fn contributor_encoding_preserves_limitations_and_normalizes_missing_provenance() {
        let envelope = encode_snapshot(fixture_snapshot()).expect("fixture encodes");
        let ProtocolEvent::RuntimeSnapshot(payload) = envelope.event else {
            unreachable!()
        };
        let network = payload
            .contributors
            .iter()
            .find(|contributor| contributor.metric == ContributorMetricV3::Network)
            .expect("network contributor");
        assert_eq!(
            payload.quality_codes[usize::from(network.quality_code)],
            MetricQualityV3::Held
        );
        assert_eq!(
            network
                .limitation_index
                .map(|index| payload.limitations[usize::from(index)].code),
            Some(LimitationCode::PendingBaseline)
        );

        let mut missing = fixture_snapshot();
        missing
            .process_contributors
            .cpu_quality
            .as_mut()
            .expect("CPU quality")
            .source = None;
        missing
            .system
            .quality
            .as_mut()
            .expect("system quality")
            .cpu
            .as_mut()
            .expect("system CPU quality")
            .source = None;
        let envelope = encode_snapshot(missing).expect("missing provenance normalizes");
        let ProtocolEvent::RuntimeSnapshot(payload) = envelope.event else {
            unreachable!()
        };
        let cpu_contributor = payload
            .contributors
            .iter()
            .find(|contributor| contributor.metric == ContributorMetricV3::Cpu)
            .expect("CPU contributor");
        assert_eq!(cpu_contributor.source, MetricSourceV3::Unknown);
        assert_eq!(cpu_contributor.process_id, None);
        assert_eq!(
            payload.quality_codes[usize::from(cpu_contributor.quality_code)],
            MetricQualityV3::Unavailable
        );
        assert_eq!(
            cpu_contributor
                .limitation_index
                .map(|index| payload.limitations[usize::from(index)].code),
            Some(LimitationCode::MissingMetadata)
        );
        let system_cpu = &payload.system.metrics[0];
        assert_eq!(
            payload.descriptors[usize::from(system_cpu.0)].source,
            MetricSourceV3::Unknown
        );
        assert_eq!(
            payload.quality_codes[usize::from(system_cpu.2)],
            MetricQualityV3::Unavailable
        );
        assert_eq!(
            system_cpu
                .4
                .map(|index| payload.limitations[usize::from(index)].code),
            Some(LimitationCode::MissingMetadata)
        );
    }

    #[test]
    fn validator_rejects_contributor_quality_and_coverage_corruption() {
        let envelope = encode_snapshot(fixture_snapshot()).expect("fixture encodes");
        let ProtocolEvent::RuntimeSnapshot(source_payload) = &envelope.event else {
            unreachable!()
        };
        let sample_seq = source_payload.sample_seq;

        let mut missing = envelope.clone();
        let ProtocolEvent::RuntimeSnapshot(payload) = &mut missing.event else {
            unreachable!()
        };
        payload.contributors.pop();
        assert_eq!(
            validate_envelope(&missing),
            Err("protocol_contributor_catalog_incomplete".to_string())
        );

        let mut duplicate = envelope.clone();
        let ProtocolEvent::RuntimeSnapshot(payload) = &mut duplicate.event else {
            unreachable!()
        };
        payload.contributors[1].metric = ContributorMetricV3::Cpu;
        assert_eq!(
            validate_envelope(&duplicate),
            Err("protocol_duplicate_contributor_metric".to_string())
        );

        let mut name_without_identity = envelope.clone();
        let ProtocolEvent::RuntimeSnapshot(payload) = &mut name_without_identity.event else {
            unreachable!()
        };
        payload.contributors[3].display_name = Some("orphan".to_string());
        assert_eq!(
            validate_envelope(&name_without_identity),
            Err("protocol_contributor_name_without_identity".to_string())
        );

        let mut blank_name = envelope.clone();
        let ProtocolEvent::RuntimeSnapshot(payload) = &mut blank_name.event else {
            unreachable!()
        };
        payload.contributors[0].display_name = Some(" \t ".to_string());
        assert_eq!(
            validate_envelope(&blank_name),
            Err("protocol_contributor_identity_invalid".to_string())
        );

        let mut max_safe_identity = envelope.clone();
        let ProtocolEvent::RuntimeSnapshot(payload) = &mut max_safe_identity.event else {
            unreachable!()
        };
        payload.contributors[0].process_id = Some("process:1234:9007199254740991".to_string());
        assert_eq!(validate_envelope(&max_safe_identity), Ok(()));

        for pid in ["0", "9007199254740991"] {
            for suffix in [
                "1699999999000".to_string(),
                format!("publication:{sample_seq}"),
            ] {
                let mut valid_pid = envelope.clone();
                let ProtocolEvent::RuntimeSnapshot(payload) = &mut valid_pid.event else {
                    unreachable!()
                };
                payload.contributors[0].process_id = Some(format!("process:{pid}:{suffix}"));
                assert_eq!(validate_envelope(&valid_pid), Ok(()));
            }
        }

        for pid in [
            "01234",
            "9007199254740992",
            "999999999999999999999999999999999999",
            "-1",
            " 1234",
            "1234 ",
            "not-decimal",
        ] {
            for suffix in [
                "1699999999000".to_string(),
                format!("publication:{sample_seq}"),
            ] {
                let mut invalid_pid = envelope.clone();
                let ProtocolEvent::RuntimeSnapshot(payload) = &mut invalid_pid.event else {
                    unreachable!()
                };
                payload.contributors[0].process_id = Some(format!("process:{pid}:{suffix}"));
                assert_eq!(
                    validate_envelope(&invalid_pid),
                    Err("protocol_contributor_identity_malformed".to_string())
                );
            }
        }

        for process_id in [
            "process:1234:9007199254740992",
            "process:1234:9999999999999999999999999999999999999999",
        ] {
            let mut unsafe_identity = envelope.clone();
            let ProtocolEvent::RuntimeSnapshot(payload) = &mut unsafe_identity.event else {
                unreachable!()
            };
            payload.contributors[0].process_id = Some(process_id.to_string());
            assert_eq!(
                validate_envelope(&unsafe_identity),
                Err("protocol_contributor_identity_malformed".to_string())
            );
        }

        let mut held_without_explanation = envelope.clone();
        let ProtocolEvent::RuntimeSnapshot(payload) = &mut held_without_explanation.event else {
            unreachable!()
        };
        payload.contributors[3].available_contributors = payload.total_process_count;
        payload.contributors[3].limitation_index = None;
        assert_eq!(
            validate_envelope(&held_without_explanation),
            Err("protocol_quality_explanation_missing".to_string())
        );

        let mut contradictory = envelope.clone();
        let ProtocolEvent::RuntimeSnapshot(payload) = &mut contradictory.event else {
            unreachable!()
        };
        payload.contributors[3].limitation_index = Some(0);
        assert_eq!(
            validate_envelope(&contradictory),
            Err("protocol_quality_limitation_contradiction".to_string())
        );

        let mut group_contradiction = envelope;
        let ProtocolEvent::RuntimeSnapshot(payload) = &mut group_contradiction.event else {
            unreachable!()
        };
        let group = payload
            .workloads
            .iter_mut()
            .find_map(|workload| match workload {
                WorkloadDetailV3::Group(group) => Some(group),
                WorkloadDetailV3::Process(_) => None,
            })
            .expect("group fixture");
        group.coverage[0].available_contributors = 1;
        group.coverage[0].limitation_index = Some(0);
        group.metrics[0].4 = Some(0);
        assert_eq!(
            validate_envelope(&group_contradiction),
            Err("protocol_group_quality_coverage_contradiction".to_string())
        );
    }

    #[test]
    fn validator_rejects_warning_range_preferences_and_orphan_persistence() {
        let envelope = encode_snapshot(fixture_snapshot()).expect("fixture encodes");

        let mut oversized_theme = envelope.clone();
        let ProtocolEvent::RuntimeSnapshot(payload) = &mut oversized_theme.event else {
            unreachable!()
        };
        payload.settings.ui_preferences = Some(RuntimeUiPreferencesV3 {
            theme: "x".repeat(65),
            history_point_limit: 72,
        });
        assert_eq!(
            validate_envelope(&oversized_theme),
            Err("protocol_ui_preferences_invalid".to_string())
        );

        let mut future_warning = envelope.clone();
        let ProtocolEvent::RuntimeSnapshot(payload) = &mut future_warning.event else {
            unreachable!()
        };
        payload.warnings[0].publication_seq = payload.publication_seq + 1;
        assert_eq!(
            validate_envelope(&future_warning),
            Err("protocol_warning_invalid".to_string())
        );

        let mut future_warning_time = envelope.clone();
        let ProtocolEvent::RuntimeSnapshot(payload) = &mut future_warning_time.event else {
            unreachable!()
        };
        payload.warnings[0].occurred_at_ms = payload.published_at_ms + 1;
        assert_eq!(
            validate_envelope(&future_warning_time),
            Err("protocol_warning_invalid".to_string())
        );

        let mut unsafe_warning = envelope.clone();
        let ProtocolEvent::RuntimeSnapshot(payload) = &mut unsafe_warning.event else {
            unreachable!()
        };
        payload.warnings[0].publication_seq = 9_007_199_254_740_992;
        assert_eq!(
            validate_envelope(&unsafe_warning),
            Err("protocol_warning_invalid".to_string())
        );

        let mut duplicate_warning = envelope.clone();
        let ProtocolEvent::RuntimeSnapshot(payload) = &mut duplicate_warning.event else {
            unreachable!()
        };
        payload.warnings.push(payload.warnings[0].clone());
        assert_eq!(
            validate_envelope(&duplicate_warning),
            Err("protocol_warning_invalid".to_string())
        );

        let mut unsafe_publication = envelope.clone();
        let ProtocolEvent::RuntimeSnapshot(payload) = &mut unsafe_publication.event else {
            unreachable!()
        };
        payload.publication_seq = 9_007_199_254_740_992;
        assert_eq!(
            validate_envelope(&unsafe_publication),
            Err("protocol_publication_metadata_invalid".to_string())
        );

        let mut unsafe_sample = envelope.clone();
        let ProtocolEvent::RuntimeSnapshot(payload) = &mut unsafe_sample.event else {
            unreachable!()
        };
        payload.sample_seq = 9_007_199_254_740_992;
        assert_eq!(
            validate_envelope(&unsafe_sample),
            Err("protocol_publication_metadata_invalid".to_string())
        );

        let mut unsafe_start = envelope.clone();
        let ProtocolEvent::RuntimeSnapshot(payload) = &mut unsafe_start.event else {
            unreachable!()
        };
        let process = payload
            .workloads
            .iter_mut()
            .find_map(|workload| match workload {
                WorkloadDetailV3::Process(process) => Some(process),
                WorkloadDetailV3::Group(_) => None,
            })
            .expect("process fixture");
        process.start_time_ms = Some(9_007_199_254_740_992);
        process.stable_id = format!("process:{}:9007199254740992", process.pid);
        assert_eq!(
            validate_envelope(&unsafe_start),
            Err("protocol_process_identity_stability_invalid".to_string())
        );

        let mut orphan_component = envelope;
        let ProtocolEvent::RuntimeSnapshot(payload) = &mut orphan_component.event else {
            unreachable!()
        };
        payload.persistence = Some(RuntimePersistenceV3 {
            state: RuntimePersistenceStateV3::Healthy,
            roots: vec![RuntimePersistenceRootV3 {
                owner: RuntimePersistenceOwnerV3::CurrentUser,
                directory: Some("/tmp/batcave".to_string()),
                permission_state: RuntimePersistencePermissionStateV3::Verified,
            }],
            components: vec![RuntimePersistenceComponentV3 {
                owner: RuntimePersistenceOwnerV3::CollectorService,
                kind: RuntimePersistenceKindV3::Settings,
                state: RuntimePersistenceStateV3::Healthy,
                durability: RuntimePersistenceDurabilityV3::Durable,
                last_success_at_ms: None,
                active_failure: None,
            }],
            suppressed_diagnostic_events: 0,
        });
        assert_eq!(
            validate_envelope(&orphan_component),
            Err("protocol_persistence_component_root_missing".to_string())
        );
    }
}
