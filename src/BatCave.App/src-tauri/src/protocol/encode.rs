use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

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
    ProcessSample, ProcessViewRow, RuntimeAdminModeState, RuntimeInstallKind, RuntimePlatform,
    RuntimePrivilegedSource, RuntimeProcessElevation, RuntimeSnapshot, SortColumn, SortDirection,
};

pub fn encode_snapshot(snapshot: RuntimeSnapshot) -> Result<ProtocolEnvelope, String> {
    let evaluated_at_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| "protocol_system_time_before_epoch".to_string())?
        .as_millis();
    let evaluated_at_ms = u64::try_from(evaluated_at_ms)
        .map_err(|_| "protocol_timestamp_out_of_range".to_string())?;
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
    architecture: RuntimeArchitectureV3,
) -> Result<ProtocolEnvelope, String> {
    encode_snapshot_with_identity(
        snapshot,
        evaluated_at_ms,
        architecture,
        RuntimeReleaseIdentityV3 {
            app_version: "development".to_string(),
            source_commit_sha: None,
        },
    )
}

fn encode_snapshot_with_identity(
    snapshot: RuntimeSnapshot,
    evaluated_at_ms: u64,
    architecture: RuntimeArchitectureV3,
    release_identity: RuntimeReleaseIdentityV3,
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

    let mut catalog = CatalogBuilder::new(snapshot.settings.sample_interval_ms)?;
    let system = encode_system(&snapshot, &mut catalog)?;
    let workloads = encode_workloads(&snapshot, &mut catalog)?;
    let contributors = encode_contributors(&snapshot, &mut catalog)?;
    let visible_process_count = workloads
        .iter()
        .filter(|workload| matches!(workload, WorkloadDetailV3::Process(_)))
        .count();
    let payload = RuntimeSnapshotPayloadV3 {
        publication_seq: snapshot.publication_seq,
        published_at_ms: snapshot.published_at_ms,
        sample_seq: snapshot.sample_seq,
        sampled_at_ms: snapshot.sampled_at_ms,
        source: snapshot.source,
        environment: RuntimeEnvironmentV3 {
            platform: platform(snapshot.environment.platform),
            architecture,
            process_elevation: process_elevation(snapshot.environment.process_elevation),
            install_kind: install_kind(snapshot.environment.install_kind),
            data_directory: snapshot.environment.data_directory,
            release_identity,
        },
        privileged_collection: RuntimePrivilegedCollectionV3 {
            state: admin_state(snapshot.admin_mode.state),
            source: privileged_source(snapshot.admin_mode.source),
            preference: if snapshot.settings.admin_mode_requested {
                PrivilegedCollectionPreferenceV3::BestAvailable
            } else {
                PrivilegedCollectionPreferenceV3::StandardOnly
            },
            detail: snapshot.admin_mode.detail,
            last_success_at_ms: snapshot.admin_mode.last_success_at_ms,
            collector_service: None,
        },
        settings: RuntimeSettingsV3 {
            query: RuntimeQueryV3 {
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
            ui_preferences: None,
        },
        health: RuntimeHealthV3 {
            engine_state: None,
            collector_state: None,
            degraded: snapshot.health.degraded,
            status_summary: snapshot.health.status_summary,
            evaluated_at_ms,
            last_heartbeat_at_ms: None,
            heartbeat_age_ms: None,
            publication_age_ms: evaluated_at_ms - snapshot.published_at_ms,
            sample_age_ms: snapshot
                .sampled_at_ms
                .map(|sampled_at_ms| evaluated_at_ms.saturating_sub(sampled_at_ms)),
            deadline_misses: None,
            deadline_lateness_p95_ms: None,
            collection_latency_ms: None,
            collection_p95_ms: None,
            publication_latency_ms: None,
            publication_p95_ms: None,
            collector_warning_count: to_u32(
                snapshot.health.collector_warnings,
                "protocol_warning_count_out_of_range",
            )?,
            app_cpu_percent: snapshot.health.app_cpu_percent,
            app_rss_bytes: snapshot.health.app_rss_bytes,
            last_warning: snapshot.health.last_warning,
            fatal_error: None,
        },
        persistence: None,
        descriptors: catalog.descriptors,
        quality_codes: QUALITY_CODES.to_vec(),
        limitations: catalog.limitations,
        system,
        workloads,
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
            .map(|warning| RuntimeWarningV3 {
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
        event: ProtocolEvent::RuntimeSnapshot(payload),
    };
    validate_envelope(&envelope)?;
    Ok(envelope)
}

fn encode_system(
    snapshot: &RuntimeSnapshot,
    catalog: &mut CatalogBuilder,
) -> Result<SystemDetailV3, String> {
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
            Ok(LogicalCpuDetailV3 {
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
                    Ok(KernelPoolTagDetailV3 {
                        stable_id: format!("system:local:pool:{}:{:?}", tag.tag, tag.kind)
                            .to_ascii_lowercase(),
                        tag: tag.tag.clone(),
                        kind: match tag.kind {
                            KernelPoolKind::Paged => KernelPoolKindV3::Paged,
                            KernelPoolKind::Nonpaged => KernelPoolKindV3::Nonpaged,
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

    Ok(SystemDetailV3 {
        stable_id: "system:local".to_string(),
        metrics,
        logical_cpus,
        kernel_pool_tags,
    })
}

fn encode_workloads(
    snapshot: &RuntimeSnapshot,
    catalog: &mut CatalogBuilder,
) -> Result<Vec<WorkloadDetailV3>, String> {
    let sampled = snapshot.sampled_at_ms;
    let mut process_ids_by_pid = HashMap::<String, Vec<String>>::new();
    let mut members_by_group = HashMap::<String, Vec<String>>::new();
    for row in &snapshot.process_view_rows {
        if let ProcessViewRow::Process {
            detail,
            group_key,
            is_grouped,
            ..
        } = row
        {
            let id = stable_process_id(detail, snapshot.sample_seq);
            process_ids_by_pid
                .entry(detail.process.pid.clone())
                .or_default()
                .push(id.clone());
            if *is_grouped {
                members_by_group
                    .entry(group_key.clone())
                    .or_default()
                    .push(id);
            }
        }
    }

    snapshot
        .process_view_rows
        .iter()
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
                let id = stable_process_id(detail, snapshot.sample_seq);
                let parent_process_id = process
                    .parent_pid
                    .as_ref()
                    .and_then(|pid| process_ids_by_pid.get(pid))
                    .filter(|ids| ids.len() == 1)
                    .and_then(|ids| ids.first())
                    .cloned();
                Ok(WorkloadDetailV3::Process(ProcessDetailV3 {
                    stable_id: id,
                    identity_stability: if process.start_time_ms == 0 {
                        ProcessIdentityStabilityV3::Publication
                    } else {
                        ProcessIdentityStabilityV3::Stable
                    },
                    pid: process.pid.clone(),
                    parent_pid: process.parent_pid.clone(),
                    parent_process_id,
                    start_time_ms: (process.start_time_ms != 0).then_some(process.start_time_ms),
                    display_name: process.name.clone(),
                    executable: process.exe.clone(),
                    status: process.status.clone(),
                    access_state: access_state(process.access_state),
                    presentation: ProcessPresentationV3 {
                        group_id: is_grouped.then(|| format!("group:{group_key}")),
                        group_key: group_key.clone(),
                        group_label: group_label.clone(),
                        group_category: group_category.clone(),
                        group_count: to_u32(*group_count, "protocol_group_count_out_of_range")?,
                        icon_kind: icon_kind.clone(),
                        is_child: *is_child,
                        is_grouped: *is_grouped,
                    },
                    metrics: encode_process_metrics(
                        process,
                        snapshot.environment.platform,
                        sampled,
                        catalog,
                    )?,
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
                Ok(WorkloadDetailV3::Group(encode_group(
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
) -> Result<GroupDetailV3, String> {
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
        coverage.push(GroupMetricCoverageV3 {
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
    Ok(GroupDetailV3 {
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
) -> Result<Vec<ProcessContributorV3>, String> {
    let summary = &snapshot.process_contributors;
    [
        (
            ContributorMetricV3::Cpu,
            summary.cpu.as_ref(),
            summary.cpu_identity.as_ref(),
            summary.cpu_coverage,
            summary.cpu_quality.as_ref(),
            summary.cpu_name_ambiguous,
        ),
        (
            ContributorMetricV3::Memory,
            summary.memory.as_ref(),
            summary.memory_identity.as_ref(),
            summary.memory_coverage,
            summary.memory_quality.as_ref(),
            summary.memory_name_ambiguous,
        ),
        (
            ContributorMetricV3::Io,
            summary.io.as_ref(),
            summary.io_identity.as_ref(),
            summary.io_coverage,
            summary.io_quality.as_ref(),
            summary.io_name_ambiguous,
        ),
        (
            ContributorMetricV3::Network,
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
        Ok(ProcessContributorV3 {
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
                .unwrap_or(MetricSourceV3::Unknown),
            limitation_index,
        })
    })
    .collect()
}

fn io_total_quality(rate_quality: Option<&MetricQualityInfo>) -> Option<MetricQualityInfo> {
    let quality = rate_quality?;
    let pending = quality.quality == MetricQuality::Held
        && quality.limitation_code == Some(MetricLimitationCode::PendingBaseline);
    pending.then(|| MetricQualityInfo {
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
fn access_state(value: AccessState) -> AccessStateV3 {
    match value {
        AccessState::Full => AccessStateV3::Full,
        AccessState::Partial => AccessStateV3::Partial,
        AccessState::Denied => AccessStateV3::Denied,
    }
}
fn platform(value: RuntimePlatform) -> RuntimePlatformV3 {
    match value {
        RuntimePlatform::Windows => RuntimePlatformV3::Windows,
        RuntimePlatform::Linux => RuntimePlatformV3::Linux,
        RuntimePlatform::Macos => RuntimePlatformV3::Macos,
        RuntimePlatform::Fixture => RuntimePlatformV3::Fixture,
    }
}

fn target_architecture() -> RuntimeArchitectureV3 {
    match std::env::consts::ARCH {
        "x86_64" => RuntimeArchitectureV3::X86_64,
        "aarch64" => RuntimeArchitectureV3::Aarch64,
        "x86" => RuntimeArchitectureV3::X86,
        _ => RuntimeArchitectureV3::Unknown,
    }
}
fn process_elevation(value: RuntimeProcessElevation) -> RuntimeProcessElevationV3 {
    match value {
        RuntimeProcessElevation::Unknown => RuntimeProcessElevationV3::Unknown,
        RuntimeProcessElevation::Standard => RuntimeProcessElevationV3::Standard,
        RuntimeProcessElevation::Elevated => RuntimeProcessElevationV3::Elevated,
        RuntimeProcessElevation::NotApplicable => RuntimeProcessElevationV3::NotApplicable,
    }
}
fn install_kind(value: RuntimeInstallKind) -> RuntimeInstallKindV3 {
    match value {
        RuntimeInstallKind::Unknown => RuntimeInstallKindV3::Unknown,
        RuntimeInstallKind::Nsis => RuntimeInstallKindV3::Nsis,
        RuntimeInstallKind::Appimage => RuntimeInstallKindV3::Appimage,
        RuntimeInstallKind::Deb => RuntimeInstallKindV3::Deb,
        RuntimeInstallKind::Dmg => RuntimeInstallKindV3::Dmg,
        RuntimeInstallKind::AppBundle => RuntimeInstallKindV3::AppBundle,
        RuntimeInstallKind::Portable => RuntimeInstallKindV3::Portable,
        RuntimeInstallKind::Development => RuntimeInstallKindV3::Development,
    }
}
fn admin_state(value: RuntimeAdminModeState) -> PrivilegedCollectionStateV3 {
    match value {
        RuntimeAdminModeState::Unavailable => PrivilegedCollectionStateV3::Unavailable,
        RuntimeAdminModeState::Off => PrivilegedCollectionStateV3::StandardOnly,
        RuntimeAdminModeState::Requesting => PrivilegedCollectionStateV3::Connecting,
        RuntimeAdminModeState::Active => PrivilegedCollectionStateV3::Active,
        RuntimeAdminModeState::Recovering => PrivilegedCollectionStateV3::Recovering,
        RuntimeAdminModeState::Failed => PrivilegedCollectionStateV3::Failed,
    }
}
fn privileged_source(value: RuntimePrivilegedSource) -> PrivilegedCollectionSourceV3 {
    match value {
        RuntimePrivilegedSource::None => PrivilegedCollectionSourceV3::None,
        RuntimePrivilegedSource::CurrentProcess | RuntimePrivilegedSource::ElevatedHelper => {
            PrivilegedCollectionSourceV3::LocalProcess
        }
    }
}
fn focus_mode(value: ProcessFocusMode) -> ProcessFocusModeV3 {
    match value {
        ProcessFocusMode::All => ProcessFocusModeV3::All,
        ProcessFocusMode::Attention => ProcessFocusModeV3::Attention,
        ProcessFocusMode::Io => ProcessFocusModeV3::Io,
    }
}
fn sort_column(value: SortColumn) -> SortColumnV3 {
    match value {
        SortColumn::Attention => SortColumnV3::Attention,
        SortColumn::Name => SortColumnV3::Name,
        SortColumn::Pid => SortColumnV3::Pid,
        SortColumn::CpuPct => SortColumnV3::CpuPct,
        SortColumn::MemoryBytes => SortColumnV3::MemoryBytes,
        SortColumn::IoBps => SortColumnV3::IoBps,
        SortColumn::NetworkBps => SortColumnV3::NetworkBps,
        SortColumn::Threads => SortColumnV3::Threads,
        SortColumn::Handles => SortColumnV3::Handles,
        SortColumn::StartTimeMs => SortColumnV3::StartTimeMs,
    }
}
fn sort_direction(value: SortDirection) -> SortDirectionV3 {
    match value {
        SortDirection::Asc => SortDirectionV3::Asc,
        SortDirection::Desc => SortDirectionV3::Desc,
    }
}
