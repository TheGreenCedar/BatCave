use std::collections::{HashMap, HashSet};

use super::{
    catalog::{network_scope_definition, semantic_definition, QUALITY_CODES},
    types::*,
    RUNTIME_PROTOCOL_VERSION,
};

const JS_MAX_SAFE_INTEGER: u64 = 9_007_199_254_740_991;

pub fn validate_envelope(envelope: &ProtocolEnvelope) -> Result<(), String> {
    if envelope.protocol_version != RUNTIME_PROTOCOL_VERSION {
        return Err("protocol_writer_version_mismatch".to_string());
    }
    if envelope.compatibility.minimum_reader_version > RUNTIME_PROTOCOL_VERSION {
        return Err("protocol_minimum_reader_unsupported".to_string());
    }
    let ProtocolEvent::RuntimeSnapshot(payload) = &envelope.event else {
        return Ok(());
    };
    if payload.publication_seq > JS_MAX_SAFE_INTEGER
        || payload.published_at_ms > JS_MAX_SAFE_INTEGER
        || payload.sample_seq > JS_MAX_SAFE_INTEGER
        || payload
            .sampled_at_ms
            .is_some_and(|sampled| sampled > JS_MAX_SAFE_INTEGER)
    {
        return Err("protocol_publication_metadata_invalid".to_string());
    }
    if payload
        .sampled_at_ms
        .is_some_and(|sampled| sampled > payload.published_at_ms)
    {
        return Err("protocol_sample_time_after_publication".to_string());
    }
    validate_release_identity(&payload.environment.release_identity)?;
    validate_settings(&payload.settings)?;
    validate_health(payload)?;
    validate_privileged_collection(payload)?;
    validate_persistence(payload.persistence.as_ref(), payload.health.evaluated_at_ms)?;
    if payload.quality_codes != QUALITY_CODES {
        return Err("protocol_quality_catalog_mismatch".to_string());
    }
    if payload
        .limitations
        .iter()
        .any(|limitation| limitation.message.trim().is_empty())
    {
        return Err("protocol_limitation_message_empty".to_string());
    }
    for (index, descriptor) in payload.descriptors.iter().enumerate() {
        if usize::from(descriptor.id) != index {
            return Err("protocol_descriptor_ids_not_contiguous".to_string());
        }
        let definition = semantic_definition(descriptor.semantic, descriptor.scope)
            .ok_or_else(|| "protocol_semantic_scope_invalid".to_string())?;
        if descriptor.unit != definition.unit {
            return Err("protocol_semantic_unit_invalid".to_string());
        }
        if descriptor.network_scope
            != network_scope_definition(descriptor.semantic, descriptor.scope, descriptor.source)
        {
            return Err("protocol_semantic_network_scope_invalid".to_string());
        }
        if definition.sampled_over_interval != descriptor.interval_ms.is_some()
            || descriptor.interval_ms == Some(0)
        {
            return Err("protocol_descriptor_interval_invalid".to_string());
        }
    }
    validate_system(&payload.system, payload)?;
    let process_details = payload
        .workloads
        .iter()
        .filter_map(|workload| match workload {
            WorkloadDetailV3::Process(detail) => Some((detail.stable_id.as_str(), detail)),
            WorkloadDetailV3::Group(_) => None,
        })
        .collect::<Vec<_>>();
    if process_details
        .iter()
        .map(|(id, _)| *id)
        .collect::<HashSet<_>>()
        .len()
        != process_details.len()
    {
        return Err("protocol_duplicate_process_id".to_string());
    }
    if usize::try_from(payload.visible_process_count).ok() != Some(process_details.len()) {
        return Err("protocol_visible_process_count_mismatch".to_string());
    }
    if payload.total_process_count < payload.visible_process_count {
        return Err("protocol_total_process_count_invalid".to_string());
    }
    let known_processes = process_details.into_iter().collect::<HashMap<_, _>>();
    for (id, process) in &known_processes {
        let expected_id = if let Some(start_time) = process.start_time_ms {
            if start_time == 0
                || start_time > JS_MAX_SAFE_INTEGER
                || !matches!(
                    process.identity_stability,
                    ProcessIdentityStabilityV3::Stable
                )
            {
                return Err("protocol_process_identity_stability_invalid".to_string());
            }
            format!("process:{}:{start_time}", process.pid)
        } else {
            if !matches!(
                process.identity_stability,
                ProcessIdentityStabilityV3::Publication
            ) {
                return Err("protocol_process_identity_stability_invalid".to_string());
            }
            format!("process:{}:publication:{}", process.pid, payload.sample_seq)
        };
        if process.pid.is_empty() || process.pid.contains(':') || *id != expected_id {
            return Err("protocol_process_identity_invalid".to_string());
        }
    }
    for process in known_processes.values() {
        if process.display_name.trim().is_empty()
            || process.status.trim().is_empty()
            || process.presentation.group_key.trim().is_empty()
            || process.presentation.group_label.trim().is_empty()
            || process.presentation.group_category.trim().is_empty()
            || process.presentation.icon_kind.trim().is_empty()
            || process.presentation.group_count == 0
        {
            return Err("protocol_process_presentation_invalid".to_string());
        }
        if let Some(parent_id) = &process.parent_process_id {
            let parent_pid = process
                .parent_pid
                .as_deref()
                .ok_or_else(|| "protocol_parent_identity_invalid".to_string())?;
            let parent = known_processes
                .get(parent_id.as_str())
                .ok_or_else(|| "protocol_parent_identity_invalid".to_string())?;
            if parent.pid != parent_pid {
                return Err("protocol_parent_identity_invalid".to_string());
            }
        }
    }
    let mut process_group = HashMap::<&str, &str>::new();
    let mut group_ids = HashSet::<&str>::new();

    validate_observations(&payload.system.metrics, MetricScope::System, payload)?;
    for logical in &payload.system.logical_cpus {
        validate_observations(&logical.metrics, MetricScope::System, payload)?;
    }
    for tag in &payload.system.kernel_pool_tags {
        validate_observations(&tag.metrics, MetricScope::System, payload)?;
    }
    for workload in &payload.workloads {
        match workload {
            WorkloadDetailV3::Process(detail) => {
                validate_observations(&detail.metrics, MetricScope::Process, payload)?;
            }
            WorkloadDetailV3::Group(detail) => {
                if detail.stable_id != format!("group:{}", detail.group_key) {
                    return Err("protocol_group_identity_invalid".to_string());
                }
                if !group_ids.insert(&detail.stable_id) {
                    return Err("protocol_duplicate_group_id".to_string());
                }
                if detail.member_ids.len() < 2 {
                    return Err("protocol_group_requires_multiple_members".to_string());
                }
                if detail.group_key.trim().is_empty()
                    || detail.label.trim().is_empty()
                    || detail.category.trim().is_empty()
                    || detail.icon_kind.trim().is_empty()
                    || detail
                        .icon_source
                        .as_deref()
                        .is_some_and(|value| value.trim().is_empty())
                    || detail
                        .example_label
                        .as_deref()
                        .is_some_and(|value| value.trim().is_empty())
                {
                    return Err("protocol_group_presentation_invalid".to_string());
                }
                if detail.member_ids.iter().collect::<HashSet<_>>().len() != detail.member_ids.len()
                {
                    return Err("protocol_duplicate_group_member".to_string());
                }
                for member in &detail.member_ids {
                    let process = known_processes
                        .get(member.as_str())
                        .ok_or_else(|| "protocol_dangling_group_member".to_string())?;
                    if process_group.insert(member, &detail.stable_id).is_some() {
                        return Err("protocol_process_in_multiple_groups".to_string());
                    }
                    if !process.presentation.is_grouped
                        || process.presentation.group_id.as_deref()
                            != Some(detail.stable_id.as_str())
                        || process.presentation.group_key != detail.group_key
                        || process.presentation.group_label != detail.label
                        || process.presentation.group_category != detail.category
                        || usize::try_from(process.presentation.group_count).ok()
                            != Some(detail.member_ids.len())
                        || process.presentation.icon_kind != detail.icon_kind
                    {
                        return Err("protocol_group_presentation_mismatch".to_string());
                    }
                }
                validate_observations(&detail.metrics, MetricScope::Group, payload)?;
                if detail.coverage.len() != detail.metrics.len() {
                    return Err("protocol_group_coverage_count_mismatch".to_string());
                }
                for observation in &detail.metrics {
                    let matches = detail
                        .coverage
                        .iter()
                        .filter(|coverage| coverage.descriptor_index == observation.0)
                        .collect::<Vec<_>>();
                    if matches.len() != 1 {
                        return Err("protocol_group_coverage_descriptor_mismatch".to_string());
                    }
                    let coverage = matches[0];
                    if usize::try_from(coverage.total_contributors).ok()
                        != Some(detail.member_ids.len())
                        || coverage.available_contributors > coverage.total_contributors
                    {
                        return Err("protocol_group_coverage_invalid".to_string());
                    }
                    validate_limitation_index(coverage.limitation_index, payload)?;
                    if coverage.limitation_index != observation.4 {
                        return Err("protocol_group_coverage_limitation_mismatch".to_string());
                    }
                    if coverage.available_contributors < coverage.total_contributors
                        && coverage.limitation_index.is_none()
                    {
                        return Err("protocol_group_coverage_unexplained".to_string());
                    }
                    if coverage.available_contributors < coverage.total_contributors
                        && matches!(
                            payload.quality_codes[usize::from(observation.2)],
                            MetricQualityV3::Native | MetricQualityV3::Estimated
                        )
                    {
                        return Err("protocol_group_quality_coverage_contradiction".to_string());
                    }
                }
            }
        }
    }
    for (id, process) in &known_processes {
        if process.presentation.is_grouped != process_group.contains_key(id)
            || process.presentation.is_grouped != process.presentation.group_id.is_some()
            || (!process.presentation.is_grouped
                && (process.presentation.is_child || process.presentation.group_count != 1))
        {
            return Err("protocol_process_group_state_mismatch".to_string());
        }
    }
    let mut contributor_metrics = HashSet::new();
    for contributor in &payload.contributors {
        if usize::from(contributor.quality_code) >= payload.quality_codes.len()
            || contributor.available_contributors > contributor.total_contributors
            || contributor.total_contributors != payload.total_process_count
        {
            return Err("protocol_contributor_invalid".to_string());
        }
        if !contributor_metrics.insert(contributor.metric) {
            return Err("protocol_duplicate_contributor_metric".to_string());
        }
        let quality = payload.quality_codes[usize::from(contributor.quality_code)];
        if contributor.process_id.is_some()
            && (contributor.display_name.is_none()
                || contributor.available_contributors != contributor.total_contributors
                || contributor.total_contributors == 0
                || matches!(
                    quality,
                    MetricQualityV3::Held | MetricQualityV3::Unavailable
                ))
        {
            return Err("protocol_contributor_identity_invalid".to_string());
        }
        if contributor
            .process_id
            .as_deref()
            .is_some_and(|id| !valid_process_id(id, payload.sample_seq))
        {
            return Err("protocol_contributor_identity_malformed".to_string());
        }
        if contributor.process_id.is_none() && contributor.display_name.is_some() {
            return Err("protocol_contributor_name_without_identity".to_string());
        }
        if contributor.available_contributors < contributor.total_contributors
            && contributor.limitation_index.is_none()
        {
            return Err("protocol_contributor_coverage_unexplained".to_string());
        }
        validate_quality_limitation(quality, contributor.limitation_index, payload)?;
        if matches!(contributor.source, MetricSourceV3::Unknown)
            && (quality != MetricQualityV3::Unavailable
                || contributor
                    .limitation_index
                    .map(|index| payload.limitations[usize::from(index)].code)
                    != Some(LimitationCode::MissingMetadata))
        {
            return Err("protocol_contributor_source_quality_contradiction".to_string());
        }
        if contributor.available_contributors < contributor.total_contributors
            && matches!(
                quality,
                MetricQualityV3::Native | MetricQualityV3::Estimated
            )
        {
            return Err("protocol_contributor_quality_coverage_contradiction".to_string());
        }
    }
    if contributor_metrics
        != HashSet::from([
            ContributorMetricV3::Cpu,
            ContributorMetricV3::Memory,
            ContributorMetricV3::Io,
            ContributorMetricV3::Network,
        ])
    {
        return Err("protocol_contributor_catalog_incomplete".to_string());
    }
    validate_warnings(payload)?;
    Ok(())
}

fn validate_release_identity(identity: &RuntimeReleaseIdentityV3) -> Result<(), String> {
    let app_version = identity.app_version.trim();
    if app_version.is_empty() || app_version.len() > 64 {
        return Err("protocol_release_version_invalid".to_string());
    }
    if identity
        .source_commit_sha
        .as_deref()
        .is_some_and(|sha| sha.len() != 40 || !sha.bytes().all(|byte| byte.is_ascii_hexdigit()))
    {
        return Err("protocol_release_commit_invalid".to_string());
    }
    Ok(())
}

fn validate_settings(settings: &RuntimeSettingsV3) -> Result<(), String> {
    if settings.metric_window_seconds == 0 || settings.effective_sample_interval_ms == 0 {
        return Err("protocol_runtime_settings_invalid".to_string());
    }
    if settings.ui_preferences.as_ref().is_some_and(|preferences| {
        preferences.theme.trim().is_empty()
            || preferences.theme.chars().count() > 64
            || preferences.history_point_limit == 0
    }) {
        return Err("protocol_ui_preferences_invalid".to_string());
    }
    Ok(())
}

fn validate_system(
    system: &SystemDetailV3,
    _payload: &RuntimeSnapshotPayloadV3,
) -> Result<(), String> {
    if system.stable_id != "system:local" {
        return Err("protocol_system_identity_invalid".to_string());
    }
    let mut logical_ids = HashSet::new();
    let mut logical_indexes = HashSet::new();
    for logical in &system.logical_cpus {
        if logical.stable_id != format!("system:local:cpu:{}", logical.index)
            || !logical_ids.insert(logical.stable_id.as_str())
            || !logical_indexes.insert(logical.index)
        {
            return Err("protocol_logical_cpu_identity_invalid".to_string());
        }
    }
    let mut pool_ids = HashSet::new();
    for tag in &system.kernel_pool_tags {
        let expected_id =
            format!("system:local:pool:{}:{:?}", tag.tag, tag.kind).to_ascii_lowercase();
        if tag.stable_id != expected_id
            || tag.tag.trim().is_empty()
            || !pool_ids.insert(tag.stable_id.as_str())
            || tag
                .driver_candidates
                .iter()
                .any(|candidate| candidate.trim().is_empty())
        {
            return Err("protocol_kernel_pool_identity_invalid".to_string());
        }
    }
    Ok(())
}

fn validate_warnings(payload: &RuntimeSnapshotPayloadV3) -> Result<(), String> {
    let mut keys = HashSet::new();
    for warning in &payload.warnings {
        if warning.publication_seq > JS_MAX_SAFE_INTEGER
            || warning.occurred_at_ms > JS_MAX_SAFE_INTEGER
            || warning.publication_seq > payload.publication_seq
            || warning.occurred_at_ms > payload.published_at_ms
            || warning.key.trim().is_empty()
            || warning.category.trim().is_empty()
            || warning.message.trim().is_empty()
            || !keys.insert(warning.key.as_str())
        {
            return Err("protocol_warning_invalid".to_string());
        }
    }
    Ok(())
}

fn validate_quality_limitation(
    quality: MetricQualityV3,
    limitation_index: Option<u16>,
    payload: &RuntimeSnapshotPayloadV3,
) -> Result<(), String> {
    validate_limitation_index(limitation_index, payload)?;
    let code = limitation_index.map(|index| payload.limitations[usize::from(index)].code);
    if matches!(
        quality,
        MetricQualityV3::Held | MetricQualityV3::Partial | MetricQualityV3::Unavailable
    ) && code.is_none()
    {
        return Err("protocol_quality_explanation_missing".to_string());
    }
    let valid = match quality {
        MetricQualityV3::Native => code.is_none(),
        MetricQualityV3::Estimated => !matches!(
            code,
            Some(
                LimitationCode::PendingBaseline
                    | LimitationCode::HeldValue
                    | LimitationCode::GroupPartialCoverage
            )
        ),
        MetricQualityV3::Held => matches!(
            code,
            Some(LimitationCode::PendingBaseline | LimitationCode::HeldValue)
        ),
        MetricQualityV3::Partial => !matches!(
            code,
            Some(
                LimitationCode::PendingBaseline
                    | LimitationCode::HeldValue
                    | LimitationCode::NumericRange
            ) | None
        ),
        MetricQualityV3::Unavailable => !matches!(
            code,
            Some(LimitationCode::PendingBaseline | LimitationCode::HeldValue) | None
        ),
    };
    valid
        .then_some(())
        .ok_or_else(|| "protocol_quality_limitation_contradiction".to_string())
}

fn validate_persistence(
    persistence: Option<&RuntimePersistenceV3>,
    evaluated_at_ms: u64,
) -> Result<(), String> {
    let Some(persistence) = persistence else {
        return Ok(());
    };
    if persistence.suppressed_diagnostic_events > JS_MAX_SAFE_INTEGER {
        return Err("protocol_persistence_counter_invalid".to_string());
    }
    let mut root_owners = HashSet::new();
    let mut worst_state = None;
    for root in &persistence.roots {
        if !root_owners.insert(root.owner) {
            return Err("protocol_persistence_root_duplicate".to_string());
        }
        match root.permission_state {
            RuntimePersistencePermissionStateV3::Verified
                if root
                    .directory
                    .as_deref()
                    .is_none_or(|directory| directory.trim().is_empty()) =>
            {
                return Err("protocol_persistence_root_state_invalid".to_string());
            }
            RuntimePersistencePermissionStateV3::Unavailable if root.directory.is_some() => {
                return Err("protocol_persistence_root_state_invalid".to_string());
            }
            RuntimePersistencePermissionStateV3::Verified => {
                record_persistence_state(&mut worst_state, RuntimePersistenceStateV3::Healthy)
            }
            RuntimePersistencePermissionStateV3::Invalid => {
                record_persistence_state(&mut worst_state, RuntimePersistenceStateV3::Degraded)
            }
            RuntimePersistencePermissionStateV3::Unavailable => {
                record_persistence_state(&mut worst_state, RuntimePersistenceStateV3::Unavailable)
            }
        }
    }
    let mut component_keys = HashSet::new();
    for component in &persistence.components {
        if !root_owners.contains(&component.owner) {
            return Err("protocol_persistence_component_root_missing".to_string());
        }
        if !component_keys.insert((component.owner, component.kind)) {
            return Err("protocol_persistence_component_duplicate".to_string());
        }
        if component
            .last_success_at_ms
            .is_some_and(|value| value > JS_MAX_SAFE_INTEGER || value > evaluated_at_ms)
        {
            return Err("protocol_persistence_timestamp_invalid".to_string());
        }
        let requires_failure = matches!(
            component.state,
            RuntimePersistenceStateV3::Degraded | RuntimePersistenceStateV3::Unavailable
        );
        if requires_failure != component.active_failure.is_some() {
            return Err("protocol_persistence_failure_state_invalid".to_string());
        }
        if matches!(component.state, RuntimePersistenceStateV3::Healthy)
            && matches!(
                component.durability,
                RuntimePersistenceDurabilityV3::SessionOnly
                    | RuntimePersistenceDurabilityV3::NotWritten
            )
        {
            return Err("protocol_persistence_durability_state_invalid".to_string());
        }
        if matches!(
            component.durability,
            RuntimePersistenceDurabilityV3::NotApplicable
        ) && (component.last_success_at_ms.is_some() || component.active_failure.is_some())
        {
            return Err("protocol_persistence_not_applicable_invalid".to_string());
        }
        if let Some(failure) = &component.active_failure {
            if failure.occurred_at_ms > JS_MAX_SAFE_INTEGER
                || failure.occurred_at_ms > evaluated_at_ms
                || failure.code.trim().is_empty()
                || failure.summary.trim().is_empty()
            {
                return Err("protocol_persistence_failure_invalid".to_string());
            }
        }
        if !matches!(
            component.durability,
            RuntimePersistenceDurabilityV3::NotApplicable
        ) {
            record_persistence_state(&mut worst_state, component.state);
        }
    }
    let expected_state = worst_state.unwrap_or(RuntimePersistenceStateV3::Unavailable);
    if !matches!(persistence.state, RuntimePersistenceStateV3::Unavailable)
        && (persistence.roots.is_empty() || persistence.components.is_empty())
    {
        return Err("protocol_persistence_nonempty_state_invalid".to_string());
    }
    if persistence.state != expected_state {
        return Err("protocol_persistence_overall_state_invalid".to_string());
    }
    Ok(())
}

fn record_persistence_state(
    worst: &mut Option<RuntimePersistenceStateV3>,
    candidate: RuntimePersistenceStateV3,
) {
    let rank = |state| match state {
        RuntimePersistenceStateV3::Healthy => 0,
        RuntimePersistenceStateV3::Degraded => 1,
        RuntimePersistenceStateV3::Unavailable => 2,
    };
    if worst.is_none_or(|current| rank(candidate) > rank(current)) {
        *worst = Some(candidate);
    }
}

fn validate_health(payload: &RuntimeSnapshotPayloadV3) -> Result<(), String> {
    let health = &payload.health;
    let engine_integer_facts = [
        health.last_heartbeat_at_ms,
        health.heartbeat_age_ms,
        health.deadline_misses,
    ];
    let engine_numeric_facts = [
        health.deadline_lateness_p95_ms,
        health.collection_latency_ms,
        health.collection_p95_ms,
        health.publication_latency_ms,
        health.publication_p95_ms,
    ];
    if engine_integer_facts
        .into_iter()
        .flatten()
        .any(|value| value > JS_MAX_SAFE_INTEGER)
        || engine_numeric_facts
            .into_iter()
            .flatten()
            .any(|value| !value.is_finite() || value < 0.0)
    {
        return Err("protocol_engine_health_fact_invalid".to_string());
    }

    if health.evaluated_at_ms > JS_MAX_SAFE_INTEGER
        || health.publication_age_ms > JS_MAX_SAFE_INTEGER
        || health
            .sample_age_ms
            .is_some_and(|value| value > JS_MAX_SAFE_INTEGER)
        || health.app_rss_bytes > JS_MAX_SAFE_INTEGER
        || !health.app_cpu_percent.is_finite()
        || health.app_cpu_percent < 0.0
        || health.evaluated_at_ms < payload.published_at_ms
        || health.publication_age_ms != health.evaluated_at_ms - payload.published_at_ms
    {
        return Err("protocol_health_fact_invalid".to_string());
    }
    match (payload.sampled_at_ms, health.sample_age_ms) {
        (Some(sampled_at_ms), Some(sample_age_ms))
            if sampled_at_ms <= health.evaluated_at_ms
                && sample_age_ms == health.evaluated_at_ms - sampled_at_ms => {}
        (None, None) => {}
        _ => return Err("protocol_sample_age_invalid".to_string()),
    }
    match (health.last_heartbeat_at_ms, health.heartbeat_age_ms) {
        (Some(heartbeat_at_ms), Some(heartbeat_age_ms))
            if heartbeat_at_ms <= health.evaluated_at_ms
                && heartbeat_age_ms == health.evaluated_at_ms - heartbeat_at_ms => {}
        (None, None) => {}
        _ => return Err("protocol_heartbeat_age_invalid".to_string()),
    }

    let has_engine_owned_fact = engine_integer_facts
        .into_iter()
        .any(|value| value.is_some())
        || engine_numeric_facts
            .into_iter()
            .any(|value| value.is_some())
        || health.collector_state.is_some()
        || health.fatal_error.is_some();
    if health.engine_state.is_none() && has_engine_owned_fact {
        return Err("protocol_engine_health_without_state".to_string());
    }
    if health.deadline_lateness_p95_ms.is_some() && health.deadline_misses.is_none() {
        return Err("protocol_deadline_lateness_without_misses".to_string());
    }
    match health.engine_state {
        Some(RuntimeEngineStateV3::Fatal) if health.fatal_error.is_none() => {
            return Err("protocol_fatal_state_without_error".to_string());
        }
        Some(
            RuntimeEngineStateV3::Starting
            | RuntimeEngineStateV3::Running
            | RuntimeEngineStateV3::Paused,
        ) if health.fatal_error.is_some() => {
            return Err("protocol_nonfatal_state_has_error".to_string());
        }
        _ => {}
    }
    if !matches!(health.engine_state, Some(RuntimeEngineStateV3::Fatal)) {
        if payload.settings.collection_paused
            && !matches!(
                health.engine_state,
                None | Some(RuntimeEngineStateV3::Paused)
            )
        {
            return Err("protocol_engine_pause_state_invalid".to_string());
        }
        if !payload.settings.collection_paused
            && matches!(health.engine_state, Some(RuntimeEngineStateV3::Paused))
        {
            return Err("protocol_engine_pause_state_invalid".to_string());
        }
    }
    if let Some(error) = &health.fatal_error {
        if error.occurred_at_ms > JS_MAX_SAFE_INTEGER
            || error.occurred_at_ms > health.evaluated_at_ms
            || error.code.trim().is_empty()
            || error.message.trim().is_empty()
        {
            return Err("protocol_fatal_error_invalid".to_string());
        }
    }
    Ok(())
}

fn validate_privileged_collection(payload: &RuntimeSnapshotPayloadV3) -> Result<(), String> {
    let privileged = &payload.privileged_collection;
    if matches!(privileged.state, PrivilegedCollectionStateV3::Active)
        != !matches!(privileged.source, PrivilegedCollectionSourceV3::None)
    {
        return Err("protocol_privileged_collection_state_source_invalid".to_string());
    }
    if privileged
        .last_success_at_ms
        .is_some_and(|value| value > JS_MAX_SAFE_INTEGER || value > payload.health.evaluated_at_ms)
    {
        return Err("protocol_privileged_collection_timestamp_invalid".to_string());
    }
    if let Some(service) = &privileged.collector_service {
        if let Some(identity) = &service.release_identity {
            validate_release_identity(identity)?;
        }
        if service.last_connected_at_ms.is_some_and(|value| {
            value > JS_MAX_SAFE_INTEGER || value > payload.health.evaluated_at_ms
        }) {
            return Err("protocol_collector_service_timestamp_invalid".to_string());
        }
        if matches!(service.state, CollectorServiceStateV3::Active)
            && (service.release_identity.is_none()
                || service
                    .service_version
                    .as_deref()
                    .is_none_or(|value| value.trim().is_empty())
                || service.negotiated_protocol_version.is_none()
                || service
                    .instance_id
                    .as_deref()
                    .is_none_or(|value| value.trim().is_empty())
                || service.last_connected_at_ms.is_none())
        {
            return Err("protocol_collector_service_active_identity_invalid".to_string());
        }
        if matches!(service.state, CollectorServiceStateV3::Active)
            && service.release_identity.as_ref() != Some(&payload.environment.release_identity)
        {
            return Err("protocol_collector_service_release_mismatch".to_string());
        }
        if matches!(service.state, CollectorServiceStateV3::Incompatible)
            && (service
                .service_version
                .as_deref()
                .is_none_or(|value| value.trim().is_empty())
                || service
                    .minimum_desktop_version
                    .as_deref()
                    .is_none_or(|value| value.trim().is_empty()))
        {
            return Err("protocol_collector_service_incompatible_version_invalid".to_string());
        }
    }
    if matches!(
        privileged.source,
        PrivilegedCollectionSourceV3::CollectorService
    ) && (!matches!(privileged.state, PrivilegedCollectionStateV3::Active)
        || !privileged
            .collector_service
            .as_ref()
            .is_some_and(|service| matches!(service.state, CollectorServiceStateV3::Active)))
    {
        return Err("protocol_collector_service_source_invalid".to_string());
    }
    if matches!(
        privileged.source,
        PrivilegedCollectionSourceV3::LocalProcess
    ) && privileged
        .collector_service
        .as_ref()
        .is_some_and(|service| matches!(service.state, CollectorServiceStateV3::Active))
    {
        return Err("protocol_local_and_service_collection_conflict".to_string());
    }
    Ok(())
}

fn valid_process_id(id: &str, sample_seq: u64) -> bool {
    let mut parts = id.split(':');
    if parts.next() != Some("process") || parts.next().is_none_or(str::is_empty) {
        return false;
    }
    match (parts.next(), parts.next(), parts.next()) {
        (Some("publication"), Some(sequence), None) => sequence
            .parse::<u64>()
            .is_ok_and(|value| value == sample_seq),
        (Some(start_time), None, None) => start_time.parse::<u64>().is_ok_and(|value| value > 0),
        _ => false,
    }
}

fn validate_observations(
    observations: &[MetricObservation],
    expected_scope: MetricScope,
    payload: &RuntimeSnapshotPayloadV3,
) -> Result<(), String> {
    let mut semantics = HashSet::new();
    for observation in observations {
        let descriptor = payload
            .descriptors
            .get(usize::from(observation.0))
            .ok_or_else(|| "protocol_observation_descriptor_out_of_range".to_string())?;
        if descriptor.scope != expected_scope {
            return Err("protocol_observation_scope_mismatch".to_string());
        }
        if !semantics.insert(descriptor.semantic) {
            return Err("protocol_duplicate_subject_semantic".to_string());
        }
        let quality = payload
            .quality_codes
            .get(usize::from(observation.2))
            .ok_or_else(|| "protocol_observation_quality_out_of_range".to_string())?;
        if *quality == MetricQualityV3::Unavailable && observation.1.is_some() {
            return Err("protocol_unavailable_observation_has_value".to_string());
        }
        if observation.1.is_none()
            && !(*quality == MetricQualityV3::Unavailable
                || (*quality == MetricQualityV3::Held && observation.4.is_some()))
        {
            return Err("protocol_null_observation_quality_invalid".to_string());
        }
        if observation
            .1
            .is_some_and(|value| !value.is_finite() || value < 0.0)
        {
            return Err("protocol_observation_not_finite".to_string());
        }
        if observation.1.is_some() && observation.3.is_none() {
            return Err("protocol_observation_time_missing".to_string());
        }
        if observation.1.is_none() && observation.3.is_some() {
            return Err("protocol_null_observation_has_time".to_string());
        }
        if observation
            .3
            .is_some_and(|observed| observed > payload.published_at_ms)
        {
            return Err("protocol_observation_after_publication".to_string());
        }
        if *quality == MetricQualityV3::Held
            && observation.3.is_some_and(|observed| {
                payload
                    .sampled_at_ms
                    .is_some_and(|sampled| observed > sampled)
            })
        {
            return Err("protocol_held_observation_after_sample".to_string());
        }
        validate_quality_limitation(*quality, observation.4, payload)?;
        if matches!(descriptor.source, MetricSourceV3::Unknown)
            && (*quality != MetricQualityV3::Unavailable
                || observation
                    .4
                    .map(|index| payload.limitations[usize::from(index)].code)
                    != Some(LimitationCode::MissingMetadata))
        {
            return Err("protocol_observation_source_quality_contradiction".to_string());
        }
    }
    Ok(())
}

fn validate_limitation_index(
    index: Option<u16>,
    payload: &RuntimeSnapshotPayloadV3,
) -> Result<(), String> {
    if index.is_some_and(|index| usize::from(index) >= payload.limitations.len()) {
        return Err("protocol_limitation_out_of_range".to_string());
    }
    Ok(())
}
