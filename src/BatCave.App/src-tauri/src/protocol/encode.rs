use std::collections::HashMap;

use super::{
    catalog::{
        metric_limitation_code, metric_quality_code, metric_source, CatalogBuilder,
        MetricDefinition, QUALITY_CODES,
    },
    types::*,
    validate::validate_envelope,
    RUNTIME_PROTOCOL_VERSION,
};
use crate::contracts::{
    AccessState, GroupDetail, KernelPoolKind, MetricLimitationCode, MetricQuality,
    MetricQualityInfo, MetricSource, ProcessContributorIdentity, ProcessDetail, ProcessFocusMode,
    ProcessSample, ProcessViewRow, RuntimeAdminModeState, RuntimeCollectorState,
    RuntimeEngineState, RuntimeInstallKind, RuntimePersistence, RuntimePersistenceDurability,
    RuntimePersistenceKind, RuntimePersistenceOperation, RuntimePersistenceOwner,
    RuntimePersistencePermissionState, RuntimePersistenceState, RuntimePlatform,
    RuntimePrivilegedSource, RuntimeProcessElevation, RuntimeSnapshot, SortColumn, SortDirection,
};

pub fn encode_snapshot(snapshot: RuntimeSnapshot) -> Result<ProtocolEnvelope, String> {
    // The runtime owns freshness. A second wall-clock read would turn clock
    // corrections into permanently stale samples while runtime health stays live.
    let evaluated_at_ms = snapshot
        .health
        .updated_at_ms
        .max(snapshot.published_at_ms)
        .max(snapshot.health.last_heartbeat_at_ms.unwrap_or_default());
    encode_snapshot_with_identity(
        snapshot,
        evaluated_at_ms,
        target_architecture(),
        super::release_identity(),
    )
}

#[cfg(test)]
pub(super) fn encode_snapshot_at(
    snapshot: RuntimeSnapshot,
    evaluated_at_ms: u64,
    architecture: RuntimeArchitectureV4,
) -> Result<ProtocolEnvelope, String> {
    encode_snapshot_with_identity(
        snapshot,
        evaluated_at_ms,
        architecture,
        RuntimeReleaseIdentityV4 {
            app_version: "development".to_string(),
            source_commit_sha: None,
        },
    )
}

fn encode_snapshot_with_identity(
    mut snapshot: RuntimeSnapshot,
    evaluated_at_ms: u64,
    architecture: RuntimeArchitectureV4,
    release_identity: RuntimeReleaseIdentityV4,
) -> Result<ProtocolEnvelope, String> {
    ensure_js_safe(snapshot.publication_seq)?;
    ensure_js_safe(snapshot.published_at_ms)?;
    ensure_js_safe(snapshot.sample_seq)?;
    ensure_js_safe(evaluated_at_ms)?;
    if evaluated_at_ms < snapshot.published_at_ms {
        return Err("protocol_health_evaluation_before_publication".to_string());
    }
    if let Some(value) = snapshot.sampled_at_ms {
        ensure_js_safe(value)?;
        if value > snapshot.published_at_ms {
            return Err("protocol_sample_time_after_publication".to_string());
        }
    }

    crate::runtime_health::evaluate_snapshot_health(&mut snapshot, evaluated_at_ms);
    let mut catalog = CatalogBuilder::new(snapshot.settings.sample_interval_ms)?;
    let system = encode_system(&snapshot, &mut catalog)?;
    let workloads = encode_workloads(
        &snapshot.process_view_rows,
        snapshot.sample_seq,
        snapshot.sampled_at_ms,
        snapshot.environment.platform,
        &mut catalog,
    )?;
    let overview_workloads = encode_workloads(
        &snapshot.overview_rows,
        snapshot.sample_seq,
        snapshot.sampled_at_ms,
        snapshot.environment.platform,
        &mut catalog,
    )?;
    let contributors = encode_contributors(&snapshot, &mut catalog)?;
    let visible_process_count = workloads
        .iter()
        .filter(|workload| matches!(workload, WorkloadDetailV4::Process(_)))
        .count();
    let payload = RuntimeSnapshotPayloadV4 {
        publication_seq: snapshot.publication_seq,
        published_at_ms: snapshot.published_at_ms,
        sample_seq: snapshot.sample_seq,
        sampled_at_ms: snapshot.sampled_at_ms,
        source: snapshot.source,
        environment: RuntimeEnvironmentV4 {
            platform: platform(snapshot.environment.platform),
            architecture,
            process_elevation: process_elevation(snapshot.environment.process_elevation),
            install_kind: install_kind(snapshot.environment.install_kind),
            data_directory: snapshot.environment.data_directory,
            release_identity,
        },
        privileged_collection: RuntimePrivilegedCollectionV4 {
            state: admin_state(snapshot.admin_mode.state),
            source: privileged_source(snapshot.admin_mode.source),
            preference: if snapshot.environment.admin_mode_available {
                PrivilegedCollectionPreferenceV4::BestAvailable
            } else {
                PrivilegedCollectionPreferenceV4::StandardOnly
            },
            standard_fallback_process_etw_disabled: snapshot.standard_fallback_process_etw_disabled,
            detail: snapshot.admin_mode.detail,
            last_success_at_ms: snapshot.admin_mode.last_success_at_ms,
            collector_service: snapshot.admin_mode.collector_service.map(|service| {
                CollectorServiceStatusV4 {
                    state: match service.state {
                        crate::contracts::RuntimeCollectorServiceState::NotInstalled => {
                            CollectorServiceStateV4::NotInstalled
                        }
                        crate::contracts::RuntimeCollectorServiceState::Stopped => {
                            CollectorServiceStateV4::Stopped
                        }
                        crate::contracts::RuntimeCollectorServiceState::Connecting => {
                            CollectorServiceStateV4::Connecting
                        }
                        crate::contracts::RuntimeCollectorServiceState::Recovering => {
                            CollectorServiceStateV4::Recovering
                        }
                        crate::contracts::RuntimeCollectorServiceState::Active => {
                            CollectorServiceStateV4::Active
                        }
                        crate::contracts::RuntimeCollectorServiceState::Incompatible => {
                            CollectorServiceStateV4::Incompatible
                        }
                        crate::contracts::RuntimeCollectorServiceState::Unauthorized => {
                            CollectorServiceStateV4::Unauthorized
                        }
                        crate::contracts::RuntimeCollectorServiceState::Failed => {
                            CollectorServiceStateV4::Failed
                        }
                    },
                    release_identity: service.release_identity.map(|identity| {
                        RuntimeReleaseIdentityV4 {
                            app_version: identity.app_version,
                            source_commit_sha: identity.source_commit_sha,
                        }
                    }),
                    service_version: service.service_version,
                    negotiated_protocol_version: service.negotiated_protocol_version,
                    minimum_desktop_version: service.minimum_desktop_version,
                    instance_id: service.instance_id,
                    last_connected_at_ms: service.last_connected_at_ms,
                    detail: service.detail,
                }
            }),
        },
        settings: RuntimeSettingsV4 {
            query: RuntimeQueryV4 {
                filter_text: snapshot.settings.query.filter_text,
                focus_mode: focus_mode(snapshot.settings.query.focus_mode),
                sort_column: sort_column(snapshot.settings.query.sort_column),
                sort_direction: sort_direction(snapshot.settings.query.sort_direction),
                limit: to_u32(
                    snapshot.settings.query.limit,
                    "protocol_query_limit_out_of_range",
                )?,
            },
            metric_window_seconds: snapshot.settings.metric_window_seconds,
            effective_sample_interval_ms: snapshot.settings.sample_interval_ms,
            collection_paused: snapshot.settings.paused,
            ui_preferences: snapshot.settings.ui_preferences.map(|preferences| {
                RuntimeUiPreferencesV4 {
                    theme: preferences.theme,
                    history_point_limit: preferences.history_point_limit,
                }
            }),
        },
        health: RuntimeHealthV4 {
            freshness: snapshot.health.freshness,
            reason_codes: snapshot.health.reason_codes,
            engine_state: snapshot.health.engine_state.map(engine_state),
            collector_state: snapshot.health.collector_state.map(collector_state),
            degraded: snapshot.health.degraded,
            status_summary: snapshot.health.status_summary,
            evaluated_at_ms,
            last_heartbeat_at_ms: snapshot.health.last_heartbeat_at_ms,
            heartbeat_age_ms: snapshot
                .health
                .last_heartbeat_at_ms
                .map(|heartbeat| evaluated_at_ms.saturating_sub(heartbeat)),
            publication_age_ms: evaluated_at_ms - snapshot.published_at_ms,
            sample_age_ms: snapshot
                .sampled_at_ms
                .map(|sampled_at_ms| evaluated_at_ms.saturating_sub(sampled_at_ms)),
            deadline_misses: snapshot.health.deadline_misses,
            deadline_lateness_p95_ms: snapshot.health.deadline_lateness_p95_ms,
            collection_latency_ms: snapshot.health.collection_latency_ms,
            collection_p95_ms: snapshot.health.collection_p95_ms,
            publication_latency_ms: snapshot.health.publication_latency_ms,
            publication_p95_ms: snapshot.health.publication_p95_ms,
            collector_warning_count: to_u32(
                snapshot.health.collector_warnings,
                "protocol_warning_count_out_of_range",
            )?,
            app_cpu_percent: snapshot.health.app_cpu_percent,
            app_rss_bytes: snapshot.health.app_rss_bytes,
            last_warning: snapshot.health.last_warning,
            fatal_error: snapshot
                .health
                .fatal_error
                .map(|error| RuntimeFatalErrorV4 {
                    code: error.code,
                    message: error.message,
                    occurred_at_ms: error.occurred_at_ms,
                }),
        },
        persistence: snapshot.persistence.map(encode_persistence),
        descriptors: catalog.descriptors,
        quality_codes: QUALITY_CODES.to_vec(),
        limitations: catalog.limitations,
        system,
        workloads,
        overview_workloads,
        contributors,
        total_process_count: to_u32(
            snapshot.total_process_count,
            "protocol_process_count_out_of_range",
        )?,
        visible_process_count: to_u32(
            visible_process_count,
            "protocol_visible_process_count_out_of_range",
        )?,
        warnings: snapshot
            .warnings
            .into_iter()
            .map(|warning| RuntimeWarningV4 {
                key: warning.key,
                publication_seq: warning.publication_seq,
                occurred_at_ms: warning.occurred_at_ms,
                category: warning.category,
                message: warning.message,
            })
            .collect(),
    };
    let envelope = ProtocolEnvelope {
        protocol_version: RUNTIME_PROTOCOL_VERSION,
        compatibility: Compatibility {
            minimum_reader_version: RUNTIME_PROTOCOL_VERSION,
            breaking: true,
        },
        event: ProtocolEvent::RuntimeSnapshot(Box::new(payload)),
    };
    validate_envelope(&envelope)?;
    Ok(envelope)
}

fn encode_persistence(persistence: RuntimePersistence) -> RuntimePersistenceV4 {
    RuntimePersistenceV4 {
        state: persistence_state(persistence.state),
        roots: persistence
            .roots
            .into_iter()
            .map(|root| RuntimePersistenceRootV4 {
                owner: persistence_owner(root.owner),
                directory: root.directory,
                permission_state: match root.permission_state {
                    RuntimePersistencePermissionState::Verified => {
                        RuntimePersistencePermissionStateV4::Verified
                    }
                    RuntimePersistencePermissionState::Invalid => {
                        RuntimePersistencePermissionStateV4::Invalid
                    }
                    RuntimePersistencePermissionState::Unavailable => {
                        RuntimePersistencePermissionStateV4::Unavailable
                    }
                },
            })
            .collect(),
        components: persistence
            .components
            .into_iter()
            .map(|component| RuntimePersistenceComponentV4 {
                owner: persistence_owner(component.owner),
                kind: match component.kind {
                    RuntimePersistenceKind::Settings => RuntimePersistenceKindV4::Settings,
                    RuntimePersistenceKind::WarmCache => RuntimePersistenceKindV4::WarmCache,
                    RuntimePersistenceKind::Diagnostics => RuntimePersistenceKindV4::Diagnostics,
                    RuntimePersistenceKind::ServiceState => RuntimePersistenceKindV4::ServiceState,
                },
                state: persistence_state(component.state),
                durability: match component.durability {
                    RuntimePersistenceDurability::Durable => {
                        RuntimePersistenceDurabilityV4::Durable
                    }
                    RuntimePersistenceDurability::NotWritten => {
                        RuntimePersistenceDurabilityV4::NotWritten
                    }
                    RuntimePersistenceDurability::SessionOnly => {
                        RuntimePersistenceDurabilityV4::SessionOnly
                    }
                    RuntimePersistenceDurability::NotApplicable => {
                        RuntimePersistenceDurabilityV4::NotApplicable
                    }
                },
                last_success_at_ms: component.last_success_at_ms,
                active_failure: component.active_failure.map(|failure| {
                    RuntimePersistenceFailureV4 {
                        code: failure.code,
                        operation: match failure.operation {
                            RuntimePersistenceOperation::ResolveRoot => {
                                RuntimePersistenceOperationV4::ResolveRoot
                            }
                            RuntimePersistenceOperation::Create => {
                                RuntimePersistenceOperationV4::Create
                            }
                            RuntimePersistenceOperation::Load => {
                                RuntimePersistenceOperationV4::Load
                            }
                            RuntimePersistenceOperation::Parse => {
                                RuntimePersistenceOperationV4::Parse
                            }
                            RuntimePersistenceOperation::Migrate => {
                                RuntimePersistenceOperationV4::Migrate
                            }
                            RuntimePersistenceOperation::Serialize => {
                                RuntimePersistenceOperationV4::Serialize
                            }
                            RuntimePersistenceOperation::Write => {
                                RuntimePersistenceOperationV4::Write
                            }
                            RuntimePersistenceOperation::Sync => {
                                RuntimePersistenceOperationV4::Sync
                            }
                            RuntimePersistenceOperation::Replace => {
                                RuntimePersistenceOperationV4::Replace
                            }
                            RuntimePersistenceOperation::Rotate => {
                                RuntimePersistenceOperationV4::Rotate
                            }
                            RuntimePersistenceOperation::Remove => {
                                RuntimePersistenceOperationV4::Remove
                            }
                            RuntimePersistenceOperation::Permissions => {
                                RuntimePersistenceOperationV4::Permissions
                            }
                        },
                        occurred_at_ms: failure.occurred_at_ms,
                        retryable: failure.retryable,
                        summary: failure.summary,
                    }
                }),
            })
            .collect(),
        suppressed_diagnostic_events: persistence.suppressed_diagnostic_events,
    }
}

fn persistence_state(state: RuntimePersistenceState) -> RuntimePersistenceStateV4 {
    match state {
        RuntimePersistenceState::Healthy => RuntimePersistenceStateV4::Healthy,
        RuntimePersistenceState::Degraded => RuntimePersistenceStateV4::Degraded,
        RuntimePersistenceState::Unavailable => RuntimePersistenceStateV4::Unavailable,
    }
}

fn persistence_owner(owner: RuntimePersistenceOwner) -> RuntimePersistenceOwnerV4 {
    match owner {
        RuntimePersistenceOwner::CurrentUser => RuntimePersistenceOwnerV4::CurrentUser,
        RuntimePersistenceOwner::CollectorService => RuntimePersistenceOwnerV4::CollectorService,
    }
}

fn encode_system(
    snapshot: &RuntimeSnapshot,
    catalog: &mut CatalogBuilder,
) -> Result<SystemDetailV4, String> {
    let system = &snapshot.system;
    let quality = system.quality.as_ref();
    let sampled = snapshot.sampled_at_ms;
    let mut metrics = vec![
        observation(
            catalog,
            MetricSemantic::CpuUsage,
            MetricUnit::PercentSystem,
            system.cpu_percent.into(),
            quality.and_then(|value| value.cpu.as_ref()),
            sampled,
        )?,
        observation(
            catalog,
            MetricSemantic::KernelCpuUsage,
            MetricUnit::PercentSystem,
            system.kernel_cpu_percent.into(),
            quality.and_then(|value| value.kernel_cpu.as_ref()),
            sampled,
        )?,
        observation(
            catalog,
            MetricSemantic::MemoryUsed,
            MetricUnit::Bytes,
            Some(system.memory_used_bytes as f64),
            quality.and_then(|value| value.memory.as_ref()),
            sampled,
        )?,
        observation(
            catalog,
            MetricSemantic::MemoryCapacity,
            MetricUnit::Bytes,
            Some(system.memory_total_bytes as f64),
            quality.and_then(|value| value.memory.as_ref()),
            sampled,
        )?,
        observation(
            catalog,
            MetricSemantic::MemoryAvailable,
            MetricUnit::Bytes,
            system.memory_available_bytes.map(|value| value as f64),
            quality.and_then(|value| value.memory.as_ref()),
            sampled,
        )?,
        observation(
            catalog,
            MetricSemantic::SwapUsed,
            MetricUnit::Bytes,
            system.swap_used_bytes.map(|value| value as f64),
            quality.and_then(|value| value.swap.as_ref()),
            sampled,
        )?,
        observation(
            catalog,
            MetricSemantic::SwapCapacity,
            MetricUnit::Bytes,
            system.swap_total_bytes.map(|value| value as f64),
            quality.and_then(|value| value.swap.as_ref()),
            sampled,
        )?,
        observation(
            catalog,
            MetricSemantic::ProcessCount,
            MetricUnit::Count,
            Some(system.process_count as f64),
            Some(&runtime_native_quality()),
            sampled,
        )?,
        observation(
            catalog,
            MetricSemantic::PhysicalDiskReadTotal,
            MetricUnit::Bytes,
            Some(system.disk_read_total_bytes as f64),
            quality.and_then(|value| value.disk.as_ref()),
            sampled,
        )?,
        observation(
            catalog,
            MetricSemantic::PhysicalDiskWriteTotal,
            MetricUnit::Bytes,
            Some(system.disk_write_total_bytes as f64),
            quality.and_then(|value| value.disk.as_ref()),
            sampled,
        )?,
        observation(
            catalog,
            MetricSemantic::PhysicalDiskReadRate,
            MetricUnit::BytesPerSecond,
            Some(system.disk_read_bps as f64),
            quality.and_then(|value| value.disk.as_ref()),
            sampled,
        )?,
        observation(
            catalog,
            MetricSemantic::PhysicalDiskWriteRate,
            MetricUnit::BytesPerSecond,
            Some(system.disk_write_bps as f64),
            quality.and_then(|value| value.disk.as_ref()),
            sampled,
        )?,
        observation(
            catalog,
            MetricSemantic::NetworkReceiveTotal,
            MetricUnit::Bytes,
            Some(system.network_received_total_bytes as f64),
            quality.and_then(|value| value.network.as_ref()),
            sampled,
        )?,
        observation(
            catalog,
            MetricSemantic::NetworkTransmitTotal,
            MetricUnit::Bytes,
            Some(system.network_transmitted_total_bytes as f64),
            quality.and_then(|value| value.network.as_ref()),
            sampled,
        )?,
        observation(
            catalog,
            MetricSemantic::NetworkReceiveRate,
            MetricUnit::BytesPerSecond,
            Some(system.network_received_bps as f64),
            quality.and_then(|value| value.network.as_ref()),
            sampled,
        )?,
        observation(
            catalog,
            MetricSemantic::NetworkTransmitRate,
            MetricUnit::BytesPerSecond,
            Some(system.network_transmitted_bps as f64),
            quality.and_then(|value| value.network.as_ref()),
            sampled,
        )?,
    ];

    if let Some(accounting) = &system.memory_accounting {
        let memory_quality = quality.and_then(|value| value.memory.as_ref());
        for (semantic, value) in [
            (
                MetricSemantic::ProcessWorkingSetMemory,
                Some(accounting.process_working_set_bytes as f64),
            ),
            (
                MetricSemantic::ProcessPrivateMemory,
                Some(accounting.process_private_bytes as f64),
            ),
            (
                MetricSemantic::DeniedProcessCount,
                Some(accounting.denied_process_count as f64),
            ),
            (
                MetricSemantic::PartialProcessCount,
                Some(accounting.partial_process_count as f64),
            ),
            (
                MetricSemantic::CommitUsed,
                accounting.commit_used_bytes.map(|value| value as f64),
            ),
            (
                MetricSemantic::CommitLimit,
                accounting.commit_limit_bytes.map(|value| value as f64),
            ),
            (
                MetricSemantic::SystemCache,
                accounting.system_cache_bytes.map(|value| value as f64),
            ),
            (
                MetricSemantic::KernelMemory,
                accounting.kernel_total_bytes.map(|value| value as f64),
            ),
            (
                MetricSemantic::KernelPagedPool,
                accounting.kernel_paged_pool_bytes.map(|value| value as f64),
            ),
            (
                MetricSemantic::KernelNonpagedPool,
                accounting
                    .kernel_nonpaged_pool_bytes
                    .map(|value| value as f64),
            ),
        ] {
            metrics.push(observation(
                catalog,
                semantic,
                if matches!(
                    semantic,
                    MetricSemantic::DeniedProcessCount | MetricSemantic::PartialProcessCount
                ) {
                    MetricUnit::Count
                } else {
                    MetricUnit::Bytes
                },
                value,
                memory_quality,
                sampled,
            )?);
        }
    }

    let logical_quality = quality.and_then(|value| value.logical_cpu.as_ref());
    let logical_cpus = system
        .logical_cpu_percent
        .iter()
        .enumerate()
        .map(|(index, value)| {
            Ok(LogicalCpuDetailV4 {
                stable_id: format!("system:local:cpu:{index}"),
                index: u16::try_from(index)
                    .map_err(|_| "protocol_logical_cpu_count_out_of_range")?,
                metrics: vec![observation(
                    catalog,
                    MetricSemantic::LogicalCpuUsage,
                    MetricUnit::PercentSystem,
                    Some(*value),
                    logical_quality,
                    sampled,
                )?],
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    let kernel_pool_tags = system
        .memory_accounting
        .as_ref()
        .map(|accounting| {
            accounting
                .kernel_pool_tags
                .iter()
                .map(|tag| {
                    Ok(KernelPoolTagDetailV4 {
                        stable_id: format!("system:local:pool:{}:{:?}", tag.tag, tag.kind)
                            .to_ascii_lowercase(),
                        tag: tag.tag.clone(),
                        kind: match tag.kind {
                            KernelPoolKind::Paged => KernelPoolKindV4::Paged,
                            KernelPoolKind::Nonpaged => KernelPoolKindV4::Nonpaged,
                        },
                        driver_candidates: tag.driver_candidates.clone(),
                        driver_candidates_pending: tag.driver_candidates_pending,
                        metrics: vec![
                            observation(
                                catalog,
                                MetricSemantic::KernelPoolBytes,
                                MetricUnit::Bytes,
                                Some(tag.bytes as f64),
                                quality.and_then(|value| value.memory.as_ref()),
                                sampled,
                            )?,
                            observation(
                                catalog,
                                MetricSemantic::KernelPoolAllocations,
                                MetricUnit::Count,
                                Some(tag.allocations as f64),
                                quality.and_then(|value| value.memory.as_ref()),
                                sampled,
                            )?,
                            observation(
                                catalog,
                                MetricSemantic::KernelPoolFrees,
                                MetricUnit::Count,
                                Some(tag.frees as f64),
                                quality.and_then(|value| value.memory.as_ref()),
                                sampled,
                            )?,
                        ],
                    })
                })
                .collect::<Result<Vec<_>, String>>()
        })
        .transpose()?
        .unwrap_or_default();

    Ok(SystemDetailV4 {
        stable_id: "system:local".to_string(),
        metrics,
        logical_cpus,
        kernel_pool_tags,
    })
}

pub(crate) fn encode_workloads(
    rows: &[ProcessViewRow],
    sample_seq: u64,
    sampled: Option<u64>,
    platform: RuntimePlatform,
    catalog: &mut CatalogBuilder,
) -> Result<Vec<WorkloadDetailV4>, String> {
    let process_details = rows
        .iter()
        .filter_map(|row| match row {
            ProcessViewRow::Process { detail, .. } => Some(detail),
            _ => None,
        })
        .collect::<Vec<_>>();
    let samples = process_details
        .iter()
        .map(|detail| detail.process.clone())
        .collect::<Vec<_>>();
    let parents = crate::workload_identity::verified_parent_indices(&samples);
    let parent_ids = parents
        .iter()
        .enumerate()
        .filter_map(|(index, parent)| {
            parent.map(|parent| {
                (
                    stable_process_id(process_details[index], sample_seq),
                    stable_process_id(process_details[parent], sample_seq),
                )
            })
        })
        .collect::<HashMap<_, _>>();
    let mut members_by_group = HashMap::<String, Vec<String>>::new();
    for row in rows {
        if let ProcessViewRow::Process {
            detail,
            group_key,
            is_grouped,
            ..
        } = row
        {
            let id = stable_process_id(detail, sample_seq);
            if *is_grouped {
                members_by_group
                    .entry(group_key.clone())
                    .or_default()
                    .push(id);
            }
        }
    }

    rows.iter()
        .map(|row| match row {
            ProcessViewRow::Process {
                detail,
                group_key,
                group_label,
                group_category,
                group_count,
                icon_kind,
                is_child,
                is_grouped,
                ..
            } => {
                let process = &detail.process;
                let id = stable_process_id(detail, sample_seq);
                let parent_process_id = parent_ids.get(&id).cloned();
                Ok(WorkloadDetailV4::Process(ProcessDetailV4 {
                    stable_id: id,
                    identity_stability: if process.start_time_ms == 0 {
                        ProcessIdentityStabilityV4::Publication
                    } else {
                        ProcessIdentityStabilityV4::Stable
                    },
                    pid: process.pid.clone(),
                    parent_pid: process.parent_pid.clone(),
                    parent_process_id,
                    start_time_ms: (process.start_time_ms != 0).then_some(process.start_time_ms),
                    display_name: process.name.clone(),
                    executable: process.exe.clone(),
                    status: process.status.clone(),
                    access_state: access_state(process.access_state),
                    presentation: ProcessPresentationV4 {
                        group_id: is_grouped.then(|| format!("group:{group_key}")),
                        group_key: group_key.clone(),
                        group_label: group_label.clone(),
                        group_category: group_category.clone(),
                        group_count: to_u32(*group_count, "protocol_group_count_out_of_range")?,
                        icon_kind: icon_kind.clone(),
                        is_child: *is_child,
                        is_grouped: *is_grouped,
                    },
                    metrics: encode_process_metrics(process, platform, sampled, catalog)?,
                }))
            }
            ProcessViewRow::Group {
                detail,
                icon_kind,
                icon_source,
                example_label,
                ..
            } => {
                let member_ids = members_by_group
                    .get(&detail.group_key)
                    .cloned()
                    .unwrap_or_default();
                Ok(WorkloadDetailV4::Group(encode_group(
                    detail,
                    member_ids,
                    icon_kind,
                    icon_source,
                    example_label,
                    sampled,
                    catalog,
                )?))
            }
        })
        .collect()
}

fn encode_process_metrics(
    process: &ProcessSample,
    platform: RuntimePlatform,
    sampled: Option<u64>,
    catalog: &mut CatalogBuilder,
) -> Result<Vec<MetricObservation>, String> {
    let quality = process.quality.as_ref();
    let cpu = quality.and_then(|value| value.cpu.as_ref());
    let memory = quality.and_then(|value| value.memory.as_ref());
    let io_rate = quality.and_then(|value| value.io.as_ref());
    let io_total_owned = io_total_quality(io_rate);
    let other_rate = quality.and_then(|value| value.other_io.as_ref());
    let other_total_owned = io_total_quality(other_rate);
    let network = quality.and_then(|value| value.network.as_ref());
    let threads = quality.and_then(|value| value.threads.as_ref());
    let handles = quality.and_then(|value| value.handles.as_ref());
    let private_value = if platform == RuntimePlatform::Macos
        && process.private_bytes == 0
        && !memory.is_some_and(|quality| {
            quality.quality == MetricQuality::Native
                && quality.source == Some(MetricSource::DirectApi)
        }) {
        None
    } else {
        Some(process.private_bytes as f64)
    };
    Ok(vec![
        process_observation(
            catalog,
            MetricSemantic::CpuUsage,
            MetricUnit::PercentOneCore,
            Some(process.cpu_percent),
            cpu,
            sampled,
        )?,
        process_observation(
            catalog,
            MetricSemantic::KernelCpuUsage,
            MetricUnit::PercentOneCore,
            process.kernel_cpu_percent,
            cpu,
            sampled,
        )?,
        process_observation(
            catalog,
            MetricSemantic::ResidentMemory,
            MetricUnit::Bytes,
            Some(process.memory_bytes as f64),
            memory,
            sampled,
        )?,
        process_observation(
            catalog,
            MetricSemantic::PrivateMemory,
            MetricUnit::Bytes,
            private_value,
            memory,
            sampled,
        )?,
        process_observation(
            catalog,
            MetricSemantic::VirtualMemory,
            MetricUnit::Bytes,
            process.virtual_memory_bytes.map(|value| value as f64),
            memory,
            sampled,
        )?,
        process_observation(
            catalog,
            MetricSemantic::ReadIoTotal,
            MetricUnit::Bytes,
            Some(process.io_read_total_bytes as f64),
            io_total_owned.as_ref().or(io_rate),
            sampled,
        )?,
        process_observation(
            catalog,
            MetricSemantic::WriteIoTotal,
            MetricUnit::Bytes,
            Some(process.io_write_total_bytes as f64),
            io_total_owned.as_ref().or(io_rate),
            sampled,
        )?,
        process_observation(
            catalog,
            MetricSemantic::OtherIoTotal,
            MetricUnit::Bytes,
            process.other_io_total_bytes.map(|value| value as f64),
            other_total_owned.as_ref().or(other_rate),
            sampled,
        )?,
        process_observation(
            catalog,
            MetricSemantic::ReadIoRate,
            MetricUnit::BytesPerSecond,
            Some(process.io_read_bps as f64),
            io_rate,
            sampled,
        )?,
        process_observation(
            catalog,
            MetricSemantic::WriteIoRate,
            MetricUnit::BytesPerSecond,
            Some(process.io_write_bps as f64),
            io_rate,
            sampled,
        )?,
        process_observation(
            catalog,
            MetricSemantic::OtherIoRate,
            MetricUnit::BytesPerSecond,
            process.other_io_bps.map(|value| value as f64),
            other_rate,
            sampled,
        )?,
        process_observation(
            catalog,
            MetricSemantic::NetworkReceiveRate,
            MetricUnit::BytesPerSecond,
            process.network_received_bps.map(|value| value as f64),
            network,
            sampled,
        )?,
        process_observation(
            catalog,
            MetricSemantic::NetworkTransmitRate,
            MetricUnit::BytesPerSecond,
            process.network_transmitted_bps.map(|value| value as f64),
            network,
            sampled,
        )?,
        process_observation(
            catalog,
            MetricSemantic::ThreadCount,
            MetricUnit::Count,
            Some(process.threads as f64),
            threads,
            sampled,
        )?,
        process_observation(
            catalog,
            MetricSemantic::HandleCount,
            MetricUnit::Count,
            Some(process.handles as f64),
            handles,
            sampled,
        )?,
    ])
}

#[allow(clippy::too_many_arguments)]
fn encode_group(
    detail: &GroupDetail,
    member_ids: Vec<String>,
    icon_kind: &str,
    icon_source: &Option<String>,
    example_label: &Option<String>,
    sampled: Option<u64>,
    catalog: &mut CatalogBuilder,
) -> Result<GroupDetailV4, String> {
    let specs = [
        (
            MetricSemantic::CpuUsage,
            MetricUnit::PercentOneCore,
            Some(detail.cpu_percent),
            &detail.quality.cpu,
            detail.coverage.cpu,
        ),
        (
            MetricSemantic::ResidentMemory,
            MetricUnit::Bytes,
            Some(detail.memory_bytes as f64),
            &detail.quality.memory,
            detail.coverage.memory,
        ),
        (
            MetricSemantic::ReadWriteIoRate,
            MetricUnit::BytesPerSecond,
            Some(detail.io_bps as f64),
            &detail.quality.io,
            detail.coverage.io,
        ),
        (
            MetricSemantic::OtherIoRate,
            MetricUnit::BytesPerSecond,
            detail.other_io_bps.map(|value| value as f64),
            &detail.quality.other_io,
            detail.coverage.other_io,
        ),
        (
            MetricSemantic::NetworkRate,
            MetricUnit::BytesPerSecond,
            Some(detail.network_bps as f64),
            &detail.quality.network,
            detail.coverage.network,
        ),
        (
            MetricSemantic::ThreadCount,
            MetricUnit::Count,
            Some(detail.threads as f64),
            &detail.quality.threads,
            detail.coverage.threads,
        ),
    ];
    let mut metrics = Vec::with_capacity(specs.len());
    let mut coverage = Vec::with_capacity(specs.len());
    for (semantic, unit, value, quality, metric_coverage) in specs {
        let observation = catalog.observation(
            MetricDefinition::new(semantic, MetricScope::Group, unit),
            value,
            Some(quality),
            sampled,
        )?;
        coverage.push(GroupMetricCoverageV4 {
            descriptor_index: observation.0,
            available_contributors: to_u32(
                metric_coverage.available,
                "protocol_coverage_out_of_range",
            )?,
            total_contributors: to_u32(metric_coverage.total, "protocol_coverage_out_of_range")?,
            limitation_index: observation.4,
        });
        metrics.push(observation);
    }
    Ok(GroupDetailV4 {
        stable_id: detail.workload_id.clone(),
        group_key: detail.group_key.clone(),
        label: detail.label.clone(),
        category: detail.category.clone(),
        member_ids,
        icon_kind: icon_kind.to_string(),
        icon_source: icon_source.clone(),
        example_label: example_label.clone(),
        metrics,
        coverage,
    })
}

fn encode_contributors(
    snapshot: &RuntimeSnapshot,
    catalog: &mut CatalogBuilder,
) -> Result<Vec<ProcessContributorV4>, String> {
    let summary = &snapshot.process_contributors;
    [
        (
            ContributorMetricV4::Cpu,
            summary.cpu.as_ref(),
            summary.cpu_identity.as_ref(),
            summary.cpu_coverage,
            summary.cpu_quality.as_ref(),
            summary.cpu_name_ambiguous,
        ),
        (
            ContributorMetricV4::Memory,
            summary.memory.as_ref(),
            summary.memory_identity.as_ref(),
            summary.memory_coverage,
            summary.memory_quality.as_ref(),
            summary.memory_name_ambiguous,
        ),
        (
            ContributorMetricV4::Io,
            summary.io.as_ref(),
            summary.io_identity.as_ref(),
            summary.io_coverage,
            summary.io_quality.as_ref(),
            summary.io_name_ambiguous,
        ),
        (
            ContributorMetricV4::Network,
            summary.network.as_ref(),
            summary.network_identity.as_ref(),
            summary.network_coverage,
            summary.network_quality.as_ref(),
            summary.network_name_ambiguous,
        ),
    ]
    .into_iter()
    .map(|(metric, name, identity, coverage, quality, ambiguous)| {
        let process_id =
            identity.map(|identity| stable_contributor_id(identity, snapshot.sample_seq));
        let coverage_missing = coverage.is_none();
        let coverage = coverage.unwrap_or(crate::contracts::MetricCoverage {
            available: 0,
            total: snapshot.total_process_count,
        });
        let quality_has_source = quality.is_some_and(|quality| quality.source.is_some());
        let mut quality_code = quality
            .filter(|_| quality_has_source)
            .map(|quality| metric_quality_code(quality.quality))
            .unwrap_or_else(|| metric_quality_code(MetricQuality::Unavailable));
        let mut limitation_index = if let Some((code, message)) = quality.and_then(|quality| {
            quality.message.as_ref().map(|message| {
                (
                    quality
                        .limitation_code
                        .map(metric_limitation_code)
                        .unwrap_or(match quality.quality {
                            MetricQuality::Held => LimitationCode::HeldValue,
                            MetricQuality::Partial => LimitationCode::PartialCoverage,
                            MetricQuality::Unavailable => LimitationCode::UnsupportedMetric,
                            MetricQuality::Native | MetricQuality::Estimated => {
                                LimitationCode::CollectorFailure
                            }
                        }),
                    message,
                )
            })
        }) {
            Some(catalog.limitation(code, message.clone())?)
        } else if !quality_has_source {
            Some(catalog.limitation(
                LimitationCode::MissingMetadata,
                "Contributor source provenance was not reported by the runtime.".to_string(),
            )?)
        } else if coverage_missing {
            Some(catalog.limitation(
                LimitationCode::MissingMetadata,
                "Contributor coverage was not reported by the runtime.".to_string(),
            )?)
        } else if coverage.available < coverage.total {
            Some(catalog.limitation(
                LimitationCode::PartialCoverage,
                format!(
                    "{} of {} processes provide this contributor metric.",
                    coverage.available, coverage.total
                ),
            )?)
        } else if coverage.total != snapshot.total_process_count {
            Some(catalog.limitation(
                LimitationCode::MissingMetadata,
                "Contributor coverage does not match the runtime process population.".to_string(),
            )?)
        } else {
            None
        };
        let quality_requires_explanation = [
            MetricQuality::Held,
            MetricQuality::Partial,
            MetricQuality::Unavailable,
        ]
        .into_iter()
        .any(|quality| quality_code == metric_quality_code(quality));
        if quality_requires_explanation && limitation_index.is_none() {
            quality_code = metric_quality_code(MetricQuality::Unavailable);
            limitation_index = Some(catalog.limitation(
                LimitationCode::MissingMetadata,
                "Contributor quality is missing a typed explanation.".to_string(),
            )?);
        }
        if coverage.available < coverage.total
            && [MetricQuality::Native, MetricQuality::Estimated]
                .into_iter()
                .any(|quality| quality_code == metric_quality_code(quality))
        {
            quality_code = metric_quality_code(MetricQuality::Partial);
            limitation_index = Some(catalog.limitation(
                LimitationCode::PartialCoverage,
                format!(
                    "{} of {} processes provide this contributor metric.",
                    coverage.available, coverage.total
                ),
            )?);
        }
        let process_id = process_id.filter(|_| quality_has_source);
        let display_name = process_id.as_ref().and(name).cloned();
        Ok(ProcessContributorV4 {
            metric,
            process_id,
            display_name,
            name_ambiguous: ambiguous,
            available_contributors: to_u32(
                coverage.available,
                "protocol_contributor_coverage_out_of_range",
            )?,
            total_contributors: to_u32(
                coverage.total,
                "protocol_contributor_coverage_out_of_range",
            )?,
            quality_code,
            source: quality
                .and_then(|quality| quality.source)
                .map(metric_source)
                .unwrap_or(MetricSourceV4::Unknown),
            limitation_index,
        })
    })
    .collect()
}

fn io_total_quality(rate_quality: Option<&MetricQualityInfo>) -> Option<MetricQualityInfo> {
    let quality = rate_quality?;
    let pending = quality.quality == MetricQuality::Held
        && quality.limitation_code == Some(MetricLimitationCode::PendingBaseline);
    pending.then_some(MetricQualityInfo {
        quality: match quality.source {
            Some(MetricSource::Sysinfo) => MetricQuality::Estimated,
            _ => MetricQuality::Native,
        },
        source: quality.source,
        updated_at_ms: quality.updated_at_ms,
        age_ms: quality.age_ms,
        limitation_code: None,
        message: None,
    })
}

fn observation(
    catalog: &mut CatalogBuilder,
    semantic: MetricSemantic,
    unit: MetricUnit,
    value: Option<f64>,
    quality: Option<&MetricQualityInfo>,
    sampled: Option<u64>,
) -> Result<MetricObservation, String> {
    catalog.observation(
        MetricDefinition::new(semantic, MetricScope::System, unit),
        value,
        quality,
        sampled,
    )
}

fn process_observation(
    catalog: &mut CatalogBuilder,
    semantic: MetricSemantic,
    unit: MetricUnit,
    value: Option<f64>,
    quality: Option<&MetricQualityInfo>,
    sampled: Option<u64>,
) -> Result<MetricObservation, String> {
    catalog.observation(
        MetricDefinition::new(semantic, MetricScope::Process, unit),
        value,
        quality,
        sampled,
    )
}

fn runtime_native_quality() -> MetricQualityInfo {
    MetricQualityInfo::new(MetricQuality::Native, MetricSource::Runtime)
}
fn stable_process_id(detail: &ProcessDetail, sample_seq: u64) -> String {
    stable_process_sample_id(&detail.process, sample_seq)
}
fn stable_process_sample_id(process: &ProcessSample, sample_seq: u64) -> String {
    if process.start_time_ms == 0 {
        format!("process:{}:publication:{sample_seq}", process.pid)
    } else {
        format!("process:{}:{}", process.pid, process.start_time_ms)
    }
}

fn stable_contributor_id(identity: &ProcessContributorIdentity, sample_seq: u64) -> String {
    if identity.start_time_ms == 0 {
        format!("process:{}:publication:{sample_seq}", identity.pid)
    } else {
        format!("process:{}:{}", identity.pid, identity.start_time_ms)
    }
}

fn ensure_js_safe(value: u64) -> Result<(), String> {
    (value <= 9_007_199_254_740_991)
        .then_some(())
        .ok_or_else(|| "protocol_timestamp_out_of_range".to_string())
}
fn to_u32(value: usize, error: &str) -> Result<u32, String> {
    u32::try_from(value).map_err(|_| error.to_string())
}
fn access_state(value: AccessState) -> AccessStateV4 {
    match value {
        AccessState::Full => AccessStateV4::Full,
        AccessState::Partial => AccessStateV4::Partial,
        AccessState::Denied => AccessStateV4::Denied,
    }
}
fn platform(value: RuntimePlatform) -> RuntimePlatformV4 {
    match value {
        RuntimePlatform::Windows => RuntimePlatformV4::Windows,
        RuntimePlatform::Linux => RuntimePlatformV4::Linux,
        RuntimePlatform::Macos => RuntimePlatformV4::Macos,
        RuntimePlatform::Fixture => RuntimePlatformV4::Fixture,
    }
}
fn engine_state(value: RuntimeEngineState) -> RuntimeEngineStateV4 {
    match value {
        RuntimeEngineState::Starting => RuntimeEngineStateV4::Starting,
        RuntimeEngineState::Running => RuntimeEngineStateV4::Running,
        RuntimeEngineState::Paused => RuntimeEngineStateV4::Paused,
        RuntimeEngineState::Fatal => RuntimeEngineStateV4::Fatal,
    }
}
fn collector_state(value: RuntimeCollectorState) -> RuntimeCollectorStateV4 {
    match value {
        RuntimeCollectorState::Healthy => RuntimeCollectorStateV4::Healthy,
        RuntimeCollectorState::Limited => RuntimeCollectorStateV4::Limited,
        RuntimeCollectorState::Unavailable => RuntimeCollectorStateV4::Unavailable,
    }
}

fn target_architecture() -> RuntimeArchitectureV4 {
    match std::env::consts::ARCH {
        "x86_64" => RuntimeArchitectureV4::X86_64,
        "aarch64" => RuntimeArchitectureV4::Aarch64,
        "x86" => RuntimeArchitectureV4::X86,
        _ => RuntimeArchitectureV4::Unknown,
    }
}
fn process_elevation(value: RuntimeProcessElevation) -> RuntimeProcessElevationV4 {
    match value {
        RuntimeProcessElevation::Unknown => RuntimeProcessElevationV4::Unknown,
        RuntimeProcessElevation::Standard => RuntimeProcessElevationV4::Standard,
        RuntimeProcessElevation::Elevated => RuntimeProcessElevationV4::Elevated,
        RuntimeProcessElevation::NotApplicable => RuntimeProcessElevationV4::NotApplicable,
    }
}
fn install_kind(value: RuntimeInstallKind) -> RuntimeInstallKindV4 {
    match value {
        RuntimeInstallKind::Unknown => RuntimeInstallKindV4::Unknown,
        RuntimeInstallKind::Nsis => RuntimeInstallKindV4::Nsis,
        RuntimeInstallKind::Appimage => RuntimeInstallKindV4::Appimage,
        RuntimeInstallKind::Deb => RuntimeInstallKindV4::Deb,
        RuntimeInstallKind::Dmg => RuntimeInstallKindV4::Dmg,
        RuntimeInstallKind::AppBundle => RuntimeInstallKindV4::AppBundle,
        RuntimeInstallKind::Portable => RuntimeInstallKindV4::Portable,
        RuntimeInstallKind::Development => RuntimeInstallKindV4::Development,
    }
}
fn admin_state(value: RuntimeAdminModeState) -> PrivilegedCollectionStateV4 {
    match value {
        RuntimeAdminModeState::Unavailable => PrivilegedCollectionStateV4::Unavailable,
        RuntimeAdminModeState::Off => PrivilegedCollectionStateV4::StandardOnly,
        RuntimeAdminModeState::Requesting => PrivilegedCollectionStateV4::Connecting,
        RuntimeAdminModeState::Active => PrivilegedCollectionStateV4::Active,
        RuntimeAdminModeState::Recovering => PrivilegedCollectionStateV4::Recovering,
        RuntimeAdminModeState::Failed => PrivilegedCollectionStateV4::Failed,
    }
}
fn privileged_source(value: RuntimePrivilegedSource) -> PrivilegedCollectionSourceV4 {
    match value {
        RuntimePrivilegedSource::None => PrivilegedCollectionSourceV4::None,
        RuntimePrivilegedSource::CurrentProcess => PrivilegedCollectionSourceV4::LocalProcess,
        RuntimePrivilegedSource::CollectorService => PrivilegedCollectionSourceV4::CollectorService,
    }
}
fn focus_mode(value: ProcessFocusMode) -> ProcessFocusModeV4 {
    match value {
        ProcessFocusMode::All => ProcessFocusModeV4::All,
        ProcessFocusMode::Attention => ProcessFocusModeV4::Attention,
        ProcessFocusMode::Io => ProcessFocusModeV4::Io,
    }
}
fn sort_column(value: SortColumn) -> SortColumnV4 {
    match value {
        SortColumn::Attention => SortColumnV4::Attention,
        SortColumn::Name => SortColumnV4::Name,
        SortColumn::Pid => SortColumnV4::Pid,
        SortColumn::CpuPct => SortColumnV4::CpuPct,
        SortColumn::MemoryBytes => SortColumnV4::MemoryBytes,
        SortColumn::IoBps => SortColumnV4::IoBps,
        SortColumn::NetworkBps => SortColumnV4::NetworkBps,
        SortColumn::Threads => SortColumnV4::Threads,
        SortColumn::Handles => SortColumnV4::Handles,
        SortColumn::StartTimeMs => SortColumnV4::StartTimeMs,
    }
}
fn sort_direction(value: SortDirection) -> SortDirectionV4 {
    match value {
        SortDirection::Asc => SortDirectionV4::Asc,
        SortDirection::Desc => SortDirectionV4::Desc,
    }
}
