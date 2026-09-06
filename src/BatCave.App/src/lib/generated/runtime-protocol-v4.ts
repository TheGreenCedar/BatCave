// Generated from the production Rust protocol; do not edit by hand.
export const RUNTIME_PROTOCOL_VERSION = 4 as const;

export const RUNTIME_PROTOCOL_POLICY = {
  "quality_codes": [
    "native",
    "estimated",
    "held",
    "partial",
    "unavailable"
  ],
  "quality_limitation_policies": [
    {
      "allowed_codes": [],
      "quality": "native",
      "requires_limitation": false
    },
    {
      "allowed_codes": [
        "unsupported_metric",
        "access_denied",
        "authorization_scope",
        "partial_coverage",
        "collector_failure",
        "data_loss",
        "missing_metadata",
        "numeric_range"
      ],
      "quality": "estimated",
      "requires_limitation": false
    },
    {
      "allowed_codes": [
        "pending_baseline",
        "held_value"
      ],
      "quality": "held",
      "requires_limitation": true
    },
    {
      "allowed_codes": [
        "unsupported_metric",
        "access_denied",
        "authorization_scope",
        "partial_coverage",
        "collector_failure",
        "data_loss",
        "missing_metadata",
        "group_partial_coverage"
      ],
      "quality": "partial",
      "requires_limitation": true
    },
    {
      "allowed_codes": [
        "unsupported_metric",
        "access_denied",
        "authorization_scope",
        "partial_coverage",
        "collector_failure",
        "data_loss",
        "missing_metadata",
        "group_partial_coverage",
        "numeric_range"
      ],
      "quality": "unavailable",
      "requires_limitation": true
    }
  ],
  "semantic_definitions": [
    {
      "network_scope": {
        "default": null,
        "sysinfo": null
      },
      "sampled_over_interval": true,
      "scope": "system",
      "semantic": "cpu_usage",
      "unit": "percent_system"
    },
    {
      "network_scope": {
        "default": null,
        "sysinfo": null
      },
      "sampled_over_interval": true,
      "scope": "system",
      "semantic": "kernel_cpu_usage",
      "unit": "percent_system"
    },
    {
      "network_scope": {
        "default": null,
        "sysinfo": null
      },
      "sampled_over_interval": true,
      "scope": "system",
      "semantic": "logical_cpu_usage",
      "unit": "percent_system"
    },
    {
      "network_scope": {
        "default": null,
        "sysinfo": null
      },
      "sampled_over_interval": true,
      "scope": "process",
      "semantic": "cpu_usage",
      "unit": "percent_one_core"
    },
    {
      "network_scope": {
        "default": null,
        "sysinfo": null
      },
      "sampled_over_interval": true,
      "scope": "process",
      "semantic": "kernel_cpu_usage",
      "unit": "percent_one_core"
    },
    {
      "network_scope": {
        "default": null,
        "sysinfo": null
      },
      "sampled_over_interval": true,
      "scope": "group",
      "semantic": "cpu_usage",
      "unit": "percent_one_core"
    },
    {
      "network_scope": {
        "default": null,
        "sysinfo": null
      },
      "sampled_over_interval": false,
      "scope": "system",
      "semantic": "memory_used",
      "unit": "bytes"
    },
    {
      "network_scope": {
        "default": null,
        "sysinfo": null
      },
      "sampled_over_interval": false,
      "scope": "system",
      "semantic": "memory_capacity",
      "unit": "bytes"
    },
    {
      "network_scope": {
        "default": null,
        "sysinfo": null
      },
      "sampled_over_interval": false,
      "scope": "system",
      "semantic": "memory_available",
      "unit": "bytes"
    },
    {
      "network_scope": {
        "default": null,
        "sysinfo": null
      },
      "sampled_over_interval": false,
      "scope": "system",
      "semantic": "swap_used",
      "unit": "bytes"
    },
    {
      "network_scope": {
        "default": null,
        "sysinfo": null
      },
      "sampled_over_interval": false,
      "scope": "system",
      "semantic": "swap_capacity",
      "unit": "bytes"
    },
    {
      "network_scope": {
        "default": null,
        "sysinfo": null
      },
      "sampled_over_interval": false,
      "scope": "system",
      "semantic": "process_working_set_memory",
      "unit": "bytes"
    },
    {
      "network_scope": {
        "default": null,
        "sysinfo": null
      },
      "sampled_over_interval": false,
      "scope": "system",
      "semantic": "process_private_memory",
      "unit": "bytes"
    },
    {
      "network_scope": {
        "default": null,
        "sysinfo": null
      },
      "sampled_over_interval": false,
      "scope": "system",
      "semantic": "commit_used",
      "unit": "bytes"
    },
    {
      "network_scope": {
        "default": null,
        "sysinfo": null
      },
      "sampled_over_interval": false,
      "scope": "system",
      "semantic": "commit_limit",
      "unit": "bytes"
    },
    {
      "network_scope": {
        "default": null,
        "sysinfo": null
      },
      "sampled_over_interval": false,
      "scope": "system",
      "semantic": "system_cache",
      "unit": "bytes"
    },
    {
      "network_scope": {
        "default": null,
        "sysinfo": null
      },
      "sampled_over_interval": false,
      "scope": "system",
      "semantic": "kernel_memory",
      "unit": "bytes"
    },
    {
      "network_scope": {
        "default": null,
        "sysinfo": null
      },
      "sampled_over_interval": false,
      "scope": "system",
      "semantic": "kernel_paged_pool",
      "unit": "bytes"
    },
    {
      "network_scope": {
        "default": null,
        "sysinfo": null
      },
      "sampled_over_interval": false,
      "scope": "system",
      "semantic": "kernel_nonpaged_pool",
      "unit": "bytes"
    },
    {
      "network_scope": {
        "default": null,
        "sysinfo": null
      },
      "sampled_over_interval": false,
      "scope": "system",
      "semantic": "kernel_pool_bytes",
      "unit": "bytes"
    },
    {
      "network_scope": {
        "default": null,
        "sysinfo": null
      },
      "sampled_over_interval": false,
      "scope": "system",
      "semantic": "physical_disk_read_total",
      "unit": "bytes"
    },
    {
      "network_scope": {
        "default": null,
        "sysinfo": null
      },
      "sampled_over_interval": false,
      "scope": "system",
      "semantic": "physical_disk_write_total",
      "unit": "bytes"
    },
    {
      "network_scope": {
        "default": "non_loopback_interface_aggregate",
        "sysinfo": "all_interface_aggregate"
      },
      "sampled_over_interval": false,
      "scope": "system",
      "semantic": "network_receive_total",
      "unit": "bytes"
    },
    {
      "network_scope": {
        "default": "non_loopback_interface_aggregate",
        "sysinfo": "all_interface_aggregate"
      },
      "sampled_over_interval": false,
      "scope": "system",
      "semantic": "network_transmit_total",
      "unit": "bytes"
    },
    {
      "network_scope": {
        "default": null,
        "sysinfo": null
      },
      "sampled_over_interval": false,
      "scope": "process",
      "semantic": "resident_memory",
      "unit": "bytes"
    },
    {
      "network_scope": {
        "default": null,
        "sysinfo": null
      },
      "sampled_over_interval": false,
      "scope": "process",
      "semantic": "private_memory",
      "unit": "bytes"
    },
    {
      "network_scope": {
        "default": null,
        "sysinfo": null
      },
      "sampled_over_interval": false,
      "scope": "process",
      "semantic": "virtual_memory",
      "unit": "bytes"
    },
    {
      "network_scope": {
        "default": null,
        "sysinfo": null
      },
      "sampled_over_interval": false,
      "scope": "process",
      "semantic": "read_io_total",
      "unit": "bytes"
    },
    {
      "network_scope": {
        "default": null,
        "sysinfo": null
      },
      "sampled_over_interval": false,
      "scope": "process",
      "semantic": "write_io_total",
      "unit": "bytes"
    },
    {
      "network_scope": {
        "default": null,
        "sysinfo": null
      },
      "sampled_over_interval": false,
      "scope": "process",
      "semantic": "other_io_total",
      "unit": "bytes"
    },
    {
      "network_scope": {
        "default": null,
        "sysinfo": null
      },
      "sampled_over_interval": false,
      "scope": "group",
      "semantic": "resident_memory",
      "unit": "bytes"
    },
    {
      "network_scope": {
        "default": null,
        "sysinfo": null
      },
      "sampled_over_interval": true,
      "scope": "system",
      "semantic": "physical_disk_read_rate",
      "unit": "bytes_per_second"
    },
    {
      "network_scope": {
        "default": null,
        "sysinfo": null
      },
      "sampled_over_interval": true,
      "scope": "system",
      "semantic": "physical_disk_write_rate",
      "unit": "bytes_per_second"
    },
    {
      "network_scope": {
        "default": "non_loopback_interface_aggregate",
        "sysinfo": "all_interface_aggregate"
      },
      "sampled_over_interval": true,
      "scope": "system",
      "semantic": "network_receive_rate",
      "unit": "bytes_per_second"
    },
    {
      "network_scope": {
        "default": "non_loopback_interface_aggregate",
        "sysinfo": "all_interface_aggregate"
      },
      "sampled_over_interval": true,
      "scope": "system",
      "semantic": "network_transmit_rate",
      "unit": "bytes_per_second"
    },
    {
      "network_scope": {
        "default": null,
        "sysinfo": null
      },
      "sampled_over_interval": true,
      "scope": "process",
      "semantic": "read_io_rate",
      "unit": "bytes_per_second"
    },
    {
      "network_scope": {
        "default": null,
        "sysinfo": null
      },
      "sampled_over_interval": true,
      "scope": "process",
      "semantic": "write_io_rate",
      "unit": "bytes_per_second"
    },
    {
      "network_scope": {
        "default": null,
        "sysinfo": null
      },
      "sampled_over_interval": true,
      "scope": "process",
      "semantic": "other_io_rate",
      "unit": "bytes_per_second"
    },
    {
      "network_scope": {
        "default": "ip_socket_payload",
        "sysinfo": "ip_socket_payload"
      },
      "sampled_over_interval": true,
      "scope": "process",
      "semantic": "network_receive_rate",
      "unit": "bytes_per_second"
    },
    {
      "network_scope": {
        "default": "ip_socket_payload",
        "sysinfo": "ip_socket_payload"
      },
      "sampled_over_interval": true,
      "scope": "process",
      "semantic": "network_transmit_rate",
      "unit": "bytes_per_second"
    },
    {
      "network_scope": {
        "default": null,
        "sysinfo": null
      },
      "sampled_over_interval": true,
      "scope": "group",
      "semantic": "read_write_io_rate",
      "unit": "bytes_per_second"
    },
    {
      "network_scope": {
        "default": null,
        "sysinfo": null
      },
      "sampled_over_interval": true,
      "scope": "group",
      "semantic": "other_io_rate",
      "unit": "bytes_per_second"
    },
    {
      "network_scope": {
        "default": "ip_socket_payload",
        "sysinfo": "ip_socket_payload"
      },
      "sampled_over_interval": true,
      "scope": "group",
      "semantic": "network_rate",
      "unit": "bytes_per_second"
    },
    {
      "network_scope": {
        "default": null,
        "sysinfo": null
      },
      "sampled_over_interval": false,
      "scope": "system",
      "semantic": "process_count",
      "unit": "count"
    },
    {
      "network_scope": {
        "default": null,
        "sysinfo": null
      },
      "sampled_over_interval": false,
      "scope": "system",
      "semantic": "denied_process_count",
      "unit": "count"
    },
    {
      "network_scope": {
        "default": null,
        "sysinfo": null
      },
      "sampled_over_interval": false,
      "scope": "system",
      "semantic": "partial_process_count",
      "unit": "count"
    },
    {
      "network_scope": {
        "default": null,
        "sysinfo": null
      },
      "sampled_over_interval": false,
      "scope": "system",
      "semantic": "kernel_pool_allocations",
      "unit": "count"
    },
    {
      "network_scope": {
        "default": null,
        "sysinfo": null
      },
      "sampled_over_interval": false,
      "scope": "system",
      "semantic": "kernel_pool_frees",
      "unit": "count"
    },
    {
      "network_scope": {
        "default": null,
        "sysinfo": null
      },
      "sampled_over_interval": false,
      "scope": "process",
      "semantic": "thread_count",
      "unit": "count"
    },
    {
      "network_scope": {
        "default": null,
        "sysinfo": null
      },
      "sampled_over_interval": false,
      "scope": "process",
      "semantic": "handle_count",
      "unit": "count"
    },
    {
      "network_scope": {
        "default": null,
        "sysinfo": null
      },
      "sampled_over_interval": false,
      "scope": "group",
      "semantic": "thread_count",
      "unit": "count"
    }
  ]
} as const;

export type Compatibility = { minimum_reader_version: number, breaking: boolean, };

export type ProtocolMismatchReason = "legacy_writer" | "reader_too_old" | "breaking_writer" | "malformed_payload";

export type ProtocolMismatchPayload = { reason: ProtocolMismatchReason, writer_version: number, minimum_reader_version: number, message: string, };

export type RuntimePlatformV4 = "windows" | "linux" | "macos" | "fixture";

export type RuntimeArchitectureV4 = "x86_64" | "aarch64" | "x86" | "unknown";

export type RuntimeProcessElevationV4 = "unknown" | "standard" | "elevated" | "not_applicable";

export type RuntimeInstallKindV4 = "unknown" | "nsis" | "appimage" | "deb" | "dmg" | "app_bundle" | "portable" | "development";

export type RuntimeReleaseIdentityV4 = { app_version: string, source_commit_sha: string | null, };

export type PrivilegedCollectionStateV4 = "unavailable" | "standard_only" | "connecting" | "active" | "recovering" | "failed";

export type PrivilegedCollectionSourceV4 = "none" | "local_process" | "collector_service";

export type PrivilegedCollectionPreferenceV4 = "standard_only" | "best_available";

export type CollectorServiceStatusV4 = { state: CollectorServiceStateV4, release_identity: RuntimeReleaseIdentityV4 | null, service_version: string | null, negotiated_protocol_version: number | null, minimum_desktop_version: string | null, instance_id: string | null, last_connected_at_ms: number | null, detail: string | null, };

export type CollectorServiceStateV4 = "not_installed" | "stopped" | "connecting" | "recovering" | "active" | "incompatible" | "unauthorized" | "failed";

export type RuntimeEnvironmentV4 = { platform: RuntimePlatformV4, architecture: RuntimeArchitectureV4, process_elevation: RuntimeProcessElevationV4, install_kind: RuntimeInstallKindV4, data_directory: string | null, release_identity: RuntimeReleaseIdentityV4, };

export type RuntimePrivilegedCollectionV4 = { state: PrivilegedCollectionStateV4, source: PrivilegedCollectionSourceV4, preference: PrivilegedCollectionPreferenceV4, standard_fallback_process_etw_disabled: boolean, detail: string | null, last_success_at_ms: number | null, collector_service: CollectorServiceStatusV4 | null, };

export type ProcessFocusModeV4 = "all" | "attention" | "io";

export type SortColumnV4 = "attention" | "name" | "pid" | "cpu_pct" | "memory_bytes" | "io_bps" | "network_bps" | "threads" | "handles" | "start_time_ms";

export type SortDirectionV4 = "asc" | "desc";

export type RuntimeQueryInputV4 = { filter_text: string, focus_mode: ProcessFocusModeV4, sort_column: SortColumnV4, sort_direction: SortDirectionV4, limit: number, };

export type RuntimeQueryV4 = { filter_text: string, focus_mode: ProcessFocusModeV4, sort_column: SortColumnV4, sort_direction: SortDirectionV4, limit: number, };

export type RuntimeSettingsV4 = { query: RuntimeQueryV4, metric_window_seconds: number, effective_sample_interval_ms: number, collection_paused: boolean, ui_preferences: RuntimeUiPreferencesV4 | null, };

export type RuntimeUiPreferencesV4 = { theme: string, history_point_limit: number, };

export type RuntimeHealthV4 = { engine_state: RuntimeEngineStateV4 | null, collector_state: RuntimeCollectorStateV4 | null, degraded: boolean, freshness: RuntimeFreshness, reason_codes: Array<RuntimeHealthReason>, status_summary: string, evaluated_at_ms: number, last_heartbeat_at_ms: number | null, heartbeat_age_ms: number | null, publication_age_ms: number, sample_age_ms: number | null, deadline_misses: number | null, deadline_lateness_p95_ms: number | null, collection_latency_ms: number | null, collection_p95_ms: number | null, publication_latency_ms: number | null, publication_p95_ms: number | null, collector_warning_count: number, app_cpu_percent: number, app_rss_bytes: number, last_warning: string | null, fatal_error: RuntimeFatalErrorV4 | null, };

export type RuntimeFreshness = "starting" | "live" | "paused" | "stale";

export type RuntimeHealthReason = "collector_unavailable" | "collector_limited" | "collector_warning" | "persistence_unavailable" | "persistence_degraded" | "cadence_missed" | "runtime_cpu_budget" | "runtime_memory_budget" | "engine_fatal" | "heartbeat_stale" | "publication_stale" | "sample_stale";

export type RuntimeEngineStateV4 = "starting" | "running" | "paused" | "fatal";

export type RuntimeCollectorStateV4 = "healthy" | "limited" | "unavailable";

export type RuntimeFatalErrorV4 = { code: string, message: string, occurred_at_ms: number, };

export type RuntimePersistenceV4 = { state: RuntimePersistenceStateV4, roots: Array<RuntimePersistenceRootV4>, components: Array<RuntimePersistenceComponentV4>, suppressed_diagnostic_events: number, };

export type RuntimePersistenceStateV4 = "healthy" | "degraded" | "unavailable";

export type RuntimePersistenceRootV4 = { owner: RuntimePersistenceOwnerV4, directory: string | null, permission_state: RuntimePersistencePermissionStateV4, };

export type RuntimePersistenceOwnerV4 = "current_user" | "collector_service";

export type RuntimePersistencePermissionStateV4 = "verified" | "invalid" | "unavailable";

export type RuntimePersistenceComponentV4 = { owner: RuntimePersistenceOwnerV4, kind: RuntimePersistenceKindV4, state: RuntimePersistenceStateV4, durability: RuntimePersistenceDurabilityV4, last_success_at_ms: number | null, active_failure: RuntimePersistenceFailureV4 | null, };

export type RuntimePersistenceKindV4 = "settings" | "warm_cache" | "diagnostics" | "service_state";

export type RuntimePersistenceDurabilityV4 = "durable" | "not_written" | "session_only" | "not_applicable";

export type RuntimePersistenceFailureV4 = { code: string, operation: RuntimePersistenceOperationV4, occurred_at_ms: number, retryable: boolean, summary: string, };

export type RuntimePersistenceOperationV4 = "resolve_root" | "create" | "load" | "parse" | "migrate" | "serialize" | "write" | "sync" | "replace" | "rotate" | "remove" | "permissions";

export type MetricSemantic = "cpu_usage" | "kernel_cpu_usage" | "logical_cpu_usage" | "resident_memory" | "private_memory" | "virtual_memory" | "memory_used" | "memory_capacity" | "memory_available" | "swap_used" | "swap_capacity" | "process_working_set_memory" | "process_private_memory" | "denied_process_count" | "partial_process_count" | "commit_used" | "commit_limit" | "system_cache" | "kernel_memory" | "kernel_paged_pool" | "kernel_nonpaged_pool" | "kernel_pool_bytes" | "kernel_pool_allocations" | "kernel_pool_frees" | "physical_disk_read_total" | "physical_disk_write_total" | "physical_disk_read_rate" | "physical_disk_write_rate" | "read_io_total" | "write_io_total" | "other_io_total" | "read_io_rate" | "write_io_rate" | "other_io_rate" | "read_write_io_rate" | "network_receive_total" | "network_transmit_total" | "network_receive_rate" | "network_transmit_rate" | "network_rate" | "process_count" | "thread_count" | "handle_count";

export type MetricScope = "system" | "process" | "group";

export type MetricUnit = "percent_one_core" | "percent_system" | "bytes" | "bytes_per_second" | "count";

export type MetricSourceV4 = "unknown" | "direct_api" | "libproc" | "iokit" | "pdh" | "interface_aggregate" | "process_aggregate" | "sysinfo" | "runtime" | "etw" | "nstat" | "procfs" | "ebpf" | "fixture";

export type MetricQualityV4 = "native" | "estimated" | "held" | "partial" | "unavailable";

export type MeasurementDescriptor = { id: number, semantic: MetricSemantic, scope: MetricScope, unit: MetricUnit, interval_ms: number | null, network_scope: NetworkScopeV4 | null, source: MetricSourceV4, };

export type NetworkScopeV4 = "non_loopback_interface_aggregate" | "all_interface_aggregate" | "ip_socket_payload";

export type MetricObservation = [number, number | null, number, number | null, number | null];

export type LimitationCode = "unsupported_metric" | "access_denied" | "authorization_scope" | "partial_coverage" | "pending_baseline" | "held_value" | "collector_failure" | "data_loss" | "missing_metadata" | "group_partial_coverage" | "numeric_range";

export type LimitationEntry = { code: LimitationCode, message: string, };

export type LogicalCpuDetailV4 = { stable_id: string, index: number, metrics: Array<MetricObservation>, };

export type KernelPoolKindV4 = "paged" | "nonpaged";

export type KernelPoolTagDetailV4 = { stable_id: string, tag: string, kind: KernelPoolKindV4, driver_candidates: Array<string>, driver_candidates_pending: boolean, metrics: Array<MetricObservation>, };

export type SystemDetailV4 = { stable_id: string, metrics: Array<MetricObservation>, logical_cpus: Array<LogicalCpuDetailV4>, kernel_pool_tags: Array<KernelPoolTagDetailV4>, };

export type ProcessIdentityStabilityV4 = "stable" | "publication";

export type AccessStateV4 = "full" | "partial" | "denied";

export type ProcessPresentationV4 = { group_id: string | null, group_key: string, group_label: string, group_category: string, group_count: number, icon_kind: string, is_child: boolean, is_grouped: boolean, };

export type ProcessDetailV4 = { stable_id: string, identity_stability: ProcessIdentityStabilityV4, pid: string, parent_pid: string | null, parent_process_id: string | null, start_time_ms: number | null, display_name: string, executable: string, status: string, access_state: AccessStateV4, presentation: ProcessPresentationV4, metrics: Array<MetricObservation>, };

export type GroupMetricCoverageV4 = { descriptor_index: number, available_contributors: number, total_contributors: number, limitation_index: number | null, };

export type GroupDetailV4 = { stable_id: string, group_key: string, label: string, category: string, member_ids: Array<string>, icon_kind: string, icon_source: string | null, example_label: string | null, metrics: Array<MetricObservation>, coverage: Array<GroupMetricCoverageV4>, };

export type WorkloadDetailV4 = { "kind": "process", "detail": ProcessDetailV4 } | { "kind": "group", "detail": GroupDetailV4 };

export type ContributorMetricV4 = "cpu" | "memory" | "io" | "network";

export type ProcessContributorV4 = { metric: ContributorMetricV4, process_id: string | null, display_name: string | null, name_ambiguous: boolean, available_contributors: number, total_contributors: number, quality_code: number, source: MetricSourceV4, limitation_index: number | null, };

export type RuntimeWarningV4 = { key: string, publication_seq: number, occurred_at_ms: number, category: string, message: string, };

export type RuntimeSnapshotPayloadV4 = { publication_seq: number, published_at_ms: number, sample_seq: number, sampled_at_ms: number | null, source: string, environment: RuntimeEnvironmentV4, privileged_collection: RuntimePrivilegedCollectionV4, settings: RuntimeSettingsV4, health: RuntimeHealthV4, persistence: RuntimePersistenceV4 | null, descriptors: Array<MeasurementDescriptor>, quality_codes: Array<MetricQualityV4>, limitations: Array<LimitationEntry>, system: SystemDetailV4, workloads: Array<WorkloadDetailV4>, overview_workloads: Array<WorkloadDetailV4>, contributors: Array<ProcessContributorV4>, total_process_count: number, visible_process_count: number, warnings: Array<RuntimeWarningV4>, };

export type ProtocolEvent = { "kind": "runtime_snapshot", "payload": RuntimeSnapshotPayloadV4 } | { "kind": "protocol_mismatch", "payload": ProtocolMismatchPayload };

export type ProtocolEnvelope = { protocol_version: number, compatibility: Compatibility, event: ProtocolEvent, };
