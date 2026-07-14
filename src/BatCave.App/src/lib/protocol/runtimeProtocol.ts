import {
  RUNTIME_PROTOCOL_VERSION,
  type MetricQualityV3,
  type MetricScope,
  type MetricSemantic,
  type MetricUnit,
  type NetworkScopeV3,
  type RuntimeSnapshotPayloadV3,
} from "../generated/runtime-protocol-v3.ts";

export interface ProtocolMismatchView {
  writerVersion: number | null;
  minimumReaderVersion: number | null;
  reason: string;
  message: string;
}

export type ProtocolDecodeResult =
  | { kind: "snapshot"; payload: RuntimeSnapshotPayloadV3 }
  | { kind: "protocol_mismatch"; mismatch: ProtocolMismatchView };

const qualityCodes: MetricQualityV3[] = ["native", "estimated", "held", "partial", "unavailable"];
const scopes = new Set<MetricScope>(["system", "process", "group"]);
const semantics = new Set<MetricSemantic>([
  "cpu_usage",
  "kernel_cpu_usage",
  "logical_cpu_usage",
  "resident_memory",
  "private_memory",
  "virtual_memory",
  "memory_used",
  "memory_capacity",
  "memory_available",
  "swap_used",
  "swap_capacity",
  "process_working_set_memory",
  "process_private_memory",
  "denied_process_count",
  "partial_process_count",
  "commit_used",
  "commit_limit",
  "system_cache",
  "kernel_memory",
  "kernel_paged_pool",
  "kernel_nonpaged_pool",
  "kernel_pool_bytes",
  "kernel_pool_allocations",
  "kernel_pool_frees",
  "physical_disk_read_total",
  "physical_disk_write_total",
  "physical_disk_read_rate",
  "physical_disk_write_rate",
  "read_io_total",
  "write_io_total",
  "other_io_total",
  "read_io_rate",
  "write_io_rate",
  "other_io_rate",
  "read_write_io_rate",
  "network_receive_total",
  "network_transmit_total",
  "network_receive_rate",
  "network_transmit_rate",
  "network_rate",
  "process_count",
  "thread_count",
  "handle_count",
]);
const units = new Set(["percent_one_core", "percent_system", "bytes", "bytes_per_second", "count"]);
const sources = new Set([
  "unknown",
  "direct_api",
  "libproc",
  "iokit",
  "pdh",
  "interface_aggregate",
  "process_aggregate",
  "sysinfo",
  "runtime",
  "etw",
  "procfs",
  "ebpf",
  "fixture",
]);
const platforms = new Set(["windows", "linux", "macos", "fixture"]);
const architectures = new Set(["x86_64", "aarch64", "x86", "unknown"]);
const elevations = new Set(["unknown", "standard", "elevated", "not_applicable"]);
const installKinds = new Set([
  "unknown",
  "nsis",
  "appimage",
  "deb",
  "dmg",
  "app_bundle",
  "portable",
  "development",
]);
const privilegedStates = new Set([
  "unavailable",
  "standard_only",
  "connecting",
  "active",
  "recovering",
  "failed",
]);
const privilegedSources = new Set(["none", "local_process", "collector_service"]);
const privilegedPreferences = new Set(["standard_only", "best_available"]);
const collectorServiceStates = new Set([
  "not_installed",
  "stopped",
  "connecting",
  "recovering",
  "active",
  "incompatible",
  "unauthorized",
  "failed",
]);
const limitationCodes = new Set([
  "unsupported_metric",
  "access_denied",
  "authorization_scope",
  "partial_coverage",
  "pending_baseline",
  "held_value",
  "collector_failure",
  "data_loss",
  "missing_metadata",
  "group_partial_coverage",
  "numeric_range",
]);
const focusModes = new Set(["all", "attention", "io"]);
const sortColumns = new Set([
  "attention",
  "name",
  "pid",
  "cpu_pct",
  "memory_bytes",
  "io_bps",
  "network_bps",
  "threads",
  "handles",
  "start_time_ms",
]);
const sortDirections = new Set(["asc", "desc"]);
const engineStates = new Set(["starting", "running", "paused", "fatal"]);
const collectorStates = new Set(["healthy", "limited", "unavailable"]);
const processIdentityStabilities = new Set(["stable", "publication"]);
const accessStates = new Set(["full", "partial", "denied"]);
const kernelPoolKinds = new Set(["paged", "nonpaged"]);
const contributorMetrics = new Set(["cpu", "memory", "io", "network"]);
const persistenceStates = new Set(["healthy", "degraded", "unavailable"]);
const persistenceOwners = new Set(["current_user", "collector_service"]);
const persistencePermissions = new Set(["verified", "invalid", "unavailable"]);
const persistenceKinds = new Set(["settings", "warm_cache", "diagnostics", "service_state"]);
const persistenceDurability = new Set(["durable", "not_written", "session_only", "not_applicable"]);
const persistenceOperations = new Set([
  "resolve_root",
  "create",
  "load",
  "parse",
  "migrate",
  "serialize",
  "write",
  "sync",
  "replace",
  "rotate",
  "remove",
  "permissions",
]);
const semanticDefinitions = new Map<
  string,
  { unit: MetricUnit; sampledOverInterval: boolean; networkScope: NetworkScopeV3 | null }
>();

function defineSemantics(
  scope: MetricScope,
  unit: MetricUnit,
  semantics: MetricSemantic[],
  networkScope: NetworkScopeV3 | null = null,
): void {
  const sampledOverInterval = ["percent_one_core", "percent_system", "bytes_per_second"].includes(
    unit,
  );
  for (const semantic of semantics) {
    semanticDefinitions.set(`${scope}:${semantic}`, {
      unit,
      sampledOverInterval,
      networkScope,
    });
  }
}

defineSemantics("system", "percent_system", ["cpu_usage", "kernel_cpu_usage", "logical_cpu_usage"]);
defineSemantics("process", "percent_one_core", ["cpu_usage", "kernel_cpu_usage"]);
defineSemantics("group", "percent_one_core", ["cpu_usage"]);
defineSemantics("system", "bytes", [
  "memory_used",
  "memory_capacity",
  "memory_available",
  "swap_used",
  "swap_capacity",
  "process_working_set_memory",
  "process_private_memory",
  "commit_used",
  "commit_limit",
  "system_cache",
  "kernel_memory",
  "kernel_paged_pool",
  "kernel_nonpaged_pool",
  "kernel_pool_bytes",
  "physical_disk_read_total",
  "physical_disk_write_total",
  "network_receive_total",
  "network_transmit_total",
]);
defineSemantics("process", "bytes", [
  "resident_memory",
  "private_memory",
  "virtual_memory",
  "read_io_total",
  "write_io_total",
  "other_io_total",
]);
defineSemantics("group", "bytes", ["resident_memory"]);
defineSemantics("system", "bytes_per_second", [
  "physical_disk_read_rate",
  "physical_disk_write_rate",
  "network_receive_rate",
  "network_transmit_rate",
]);
defineSemantics("process", "bytes_per_second", [
  "read_io_rate",
  "write_io_rate",
  "other_io_rate",
  "network_receive_rate",
  "network_transmit_rate",
]);
defineSemantics("group", "bytes_per_second", [
  "read_write_io_rate",
  "other_io_rate",
  "network_rate",
]);
defineSemantics("system", "count", [
  "process_count",
  "denied_process_count",
  "partial_process_count",
  "kernel_pool_allocations",
  "kernel_pool_frees",
]);
defineSemantics("process", "count", ["thread_count", "handle_count"]);
defineSemantics("group", "count", ["thread_count"]);
defineSemantics(
  "system",
  "bytes",
  ["network_receive_total", "network_transmit_total"],
  "non_loopback_interface_aggregate",
);
defineSemantics(
  "system",
  "bytes_per_second",
  ["network_receive_rate", "network_transmit_rate"],
  "non_loopback_interface_aggregate",
);
defineSemantics(
  "process",
  "bytes_per_second",
  ["network_receive_rate", "network_transmit_rate"],
  "ip_socket_payload",
);
defineSemantics("group", "bytes_per_second", ["network_rate"], "ip_socket_payload");

export function decodeProtocolEnvelope(input: unknown): ProtocolDecodeResult {
  if (!isRecord(input))
    return mismatch(null, null, "malformed_payload", "Protocol envelope is not an object.");
  const writerVersion = safeInteger(input.protocol_version) ? input.protocol_version : null;
  const compatibility = isRecord(input.compatibility) ? input.compatibility : null;
  const minimumReaderVersion =
    compatibility && safeInteger(compatibility.minimum_reader_version)
      ? compatibility.minimum_reader_version
      : null;
  if (
    writerVersion === null ||
    minimumReaderVersion === null ||
    typeof compatibility?.breaking !== "boolean"
  ) {
    return mismatch(
      writerVersion,
      minimumReaderVersion,
      "malformed_payload",
      "Protocol version metadata is malformed.",
    );
  }
  if (writerVersion < RUNTIME_PROTOCOL_VERSION) {
    return mismatch(
      writerVersion,
      minimumReaderVersion,
      "legacy_writer",
      "This build does not retain the legacy runtime reader.",
    );
  }
  if (minimumReaderVersion > RUNTIME_PROTOCOL_VERSION) {
    return mismatch(
      writerVersion,
      minimumReaderVersion,
      "reader_too_old",
      "The runtime requires a newer telemetry reader.",
    );
  }
  if (writerVersion > RUNTIME_PROTOCOL_VERSION && compatibility.breaking) {
    return mismatch(
      writerVersion,
      minimumReaderVersion,
      "breaking_writer",
      "The runtime reports a breaking protocol revision.",
    );
  }
  if (
    !isRecord(input.event) ||
    typeof input.event.kind !== "string" ||
    !isRecord(input.event.payload)
  ) {
    return mismatch(
      writerVersion,
      minimumReaderVersion,
      "malformed_payload",
      "Protocol event is malformed.",
    );
  }
  if (input.event.kind === "protocol_mismatch") {
    const payload = input.event.payload;
    return mismatch(
      safeInteger(payload.writer_version) ? payload.writer_version : writerVersion,
      safeInteger(payload.minimum_reader_version)
        ? payload.minimum_reader_version
        : minimumReaderVersion,
      typeof payload.reason === "string" ? payload.reason : "malformed_payload",
      typeof payload.message === "string"
        ? payload.message
        : "The runtime rejected this protocol reader.",
    );
  }
  if (input.event.kind !== "runtime_snapshot") {
    return mismatch(
      writerVersion,
      minimumReaderVersion,
      "malformed_payload",
      "Protocol event kind is unknown.",
    );
  }
  const rawPayload = input.event.payload;
  const error = validatePayload(rawPayload);
  return error
    ? mismatch(writerVersion, minimumReaderVersion, "malformed_payload", error)
    : { kind: "snapshot", payload: rawPayload as unknown as RuntimeSnapshotPayloadV3 };
}

function validatePayload(input: unknown): string | null {
  if (!isRecord(input)) return "Runtime payload is not an object.";
  const payload = input;
  for (const value of [payload.publication_seq, payload.published_at_ms, payload.sample_seq]) {
    if (!safeInteger(value)) return "Publication metadata is outside the safe integer range.";
  }
  if (payload.sampled_at_ms !== null && !safeInteger(payload.sampled_at_ms))
    return "Sample time is invalid.";
  if (payload.sampled_at_ms !== null && payload.sampled_at_ms > payload.published_at_ms)
    return "Sample time is after publication.";
  if (
    !isRecord(payload.environment) ||
    !isRecord(payload.privileged_collection) ||
    !isRecord(payload.settings) ||
    !isRecord(payload.settings.query) ||
    !isRecord(payload.health)
  )
    return "Runtime context is malformed.";
  if (!validReleaseIdentity(payload.environment.release_identity))
    return "Runtime release identity is malformed.";
  if (
    !nonEmptyString(payload.source) ||
    !platforms.has(payload.environment.platform) ||
    !architectures.has(payload.environment.architecture) ||
    !elevations.has(payload.environment.process_elevation) ||
    !installKinds.has(payload.environment.install_kind) ||
    (payload.environment.data_directory !== null &&
      typeof payload.environment.data_directory !== "string")
  ) {
    return "Runtime environment is malformed.";
  }
  if (
    !privilegedStates.has(payload.privileged_collection.state) ||
    !privilegedSources.has(payload.privileged_collection.source) ||
    !privilegedPreferences.has(payload.privileged_collection.preference) ||
    (payload.privileged_collection.detail !== null &&
      typeof payload.privileged_collection.detail !== "string") ||
    (payload.privileged_collection.last_success_at_ms !== null &&
      !safeInteger(payload.privileged_collection.last_success_at_ms))
  ) {
    return "Privileged collection context is malformed.";
  }
  if (
    (payload.privileged_collection.state === "active") !==
    (payload.privileged_collection.source !== "none")
  )
    return "Privileged collection state and source are inconsistent.";
  const collectorServiceError = validateCollectorService(
    payload.privileged_collection,
    payload.health,
    payload.environment.release_identity,
  );
  if (collectorServiceError !== null) return collectorServiceError;
  const query = payload.settings.query;
  if (
    typeof query.filter_text !== "string" ||
    !focusModes.has(query.focus_mode) ||
    !sortColumns.has(query.sort_column) ||
    !sortDirections.has(query.sort_direction) ||
    !safeInteger(query.limit) ||
    typeof payload.settings.collection_paused !== "boolean" ||
    !safeInteger(payload.settings.metric_window_seconds) ||
    !safeInteger(payload.settings.effective_sample_interval_ms) ||
    payload.settings.metric_window_seconds === 0 ||
    payload.settings.effective_sample_interval_ms === 0 ||
    (payload.settings.ui_preferences !== null &&
      (!isRecord(payload.settings.ui_preferences) ||
        typeof payload.settings.ui_preferences.theme !== "string" ||
        payload.settings.ui_preferences.theme.trim().length === 0 ||
        [...payload.settings.ui_preferences.theme].length > 64 ||
        !safeInteger(payload.settings.ui_preferences.history_point_limit) ||
        payload.settings.ui_preferences.history_point_limit === 0))
  )
    return "Runtime settings are malformed.";
  const health = payload.health;
  if (
    ![
      health.evaluated_at_ms,
      health.publication_age_ms,
      health.collector_warning_count,
      health.app_rss_bytes,
    ].every(safeInteger) ||
    !nonNegativeFiniteNumber(health.app_cpu_percent) ||
    (health.sample_age_ms !== null && !safeInteger(health.sample_age_ms)) ||
    typeof health.degraded !== "boolean" ||
    typeof health.status_summary !== "string" ||
    (health.last_warning !== null && typeof health.last_warning !== "string")
  ) {
    return "Runtime health is malformed.";
  }
  const engineIntegerFacts = [
    health.last_heartbeat_at_ms,
    health.heartbeat_age_ms,
    health.deadline_misses,
  ];
  const engineNumericFacts = [
    health.deadline_lateness_p95_ms,
    health.collection_latency_ms,
    health.collection_p95_ms,
    health.publication_latency_ms,
    health.publication_p95_ms,
  ];
  if (
    (health.engine_state !== null && !engineStates.has(health.engine_state)) ||
    engineIntegerFacts.some((value) => value !== null && !safeInteger(value)) ||
    engineNumericFacts.some((value) => value !== null && !nonNegativeFiniteNumber(value)) ||
    (health.collector_state !== null && !collectorStates.has(health.collector_state)) ||
    (health.fatal_error !== null &&
      (!isRecord(health.fatal_error) ||
        typeof health.fatal_error.code !== "string" ||
        health.fatal_error.code.trim().length === 0 ||
        typeof health.fatal_error.message !== "string" ||
        health.fatal_error.message.trim().length === 0 ||
        !safeInteger(health.fatal_error.occurred_at_ms)))
  ) {
    return "Runtime engine health is malformed.";
  }
  if (
    health.evaluated_at_ms < payload.published_at_ms ||
    health.publication_age_ms !== health.evaluated_at_ms - payload.published_at_ms
  )
    return "Runtime publication age is inconsistent.";
  if (
    payload.sampled_at_ms === null
      ? health.sample_age_ms !== null
      : health.sample_age_ms === null ||
        health.evaluated_at_ms < payload.sampled_at_ms ||
        health.sample_age_ms !== health.evaluated_at_ms - payload.sampled_at_ms
  )
    return "Runtime sample age is inconsistent.";
  if (
    health.last_heartbeat_at_ms === null || health.heartbeat_age_ms === null
      ? health.last_heartbeat_at_ms !== health.heartbeat_age_ms
      : health.evaluated_at_ms < health.last_heartbeat_at_ms ||
        health.heartbeat_age_ms !== health.evaluated_at_ms - health.last_heartbeat_at_ms
  )
    return "Runtime heartbeat age is inconsistent.";
  const hasEngineOwnedFact =
    engineIntegerFacts.some((value) => value !== null) ||
    engineNumericFacts.some((value) => value !== null) ||
    health.collector_state !== null ||
    health.fatal_error !== null;
  if (health.engine_state === null && hasEngineOwnedFact)
    return "Runtime engine facts require an engine state.";
  if (health.deadline_lateness_p95_ms !== null && health.deadline_misses === null)
    return "Runtime deadline lateness requires deadline ownership.";
  if (health.engine_state === "fatal" && health.fatal_error === null)
    return "Fatal runtime engine state requires a fatal error.";
  if (
    health.engine_state !== null &&
    health.engine_state !== "fatal" &&
    health.fatal_error !== null
  )
    return "Nonfatal runtime engine state cannot carry a fatal error.";
  if (
    !health.degraded &&
    (health.engine_state === "fatal" ||
      health.collector_state === "limited" ||
      health.collector_state === "unavailable")
  )
    return "Runtime failure or limited collection state must be degraded.";
  if (
    health.engine_state !== null &&
    health.engine_state !== "fatal" &&
    payload.settings.collection_paused !== (health.engine_state === "paused")
  )
    return "Runtime pause state is inconsistent with settings.";
  if (health.fatal_error !== null && health.fatal_error.occurred_at_ms > health.evaluated_at_ms)
    return "Runtime fatal error occurs after health evaluation.";
  const persistenceError = validatePersistence(payload.persistence, health.evaluated_at_ms);
  if (persistenceError) return persistenceError;
  if (
    !Array.isArray(payload.descriptors) ||
    !Array.isArray(payload.quality_codes) ||
    !Array.isArray(payload.limitations) ||
    !isRecord(payload.system) ||
    !Array.isArray(payload.workloads) ||
    !Array.isArray(payload.contributors) ||
    !Array.isArray(payload.warnings)
  )
    return "Runtime collections are malformed.";
  if (
    payload.quality_codes.length !== qualityCodes.length ||
    payload.quality_codes.some((quality, index) => quality !== qualityCodes[index])
  ) {
    return "Quality catalog is unknown or reordered.";
  }
  for (let index = 0; index < payload.descriptors.length; index += 1) {
    const descriptor = payload.descriptors[index];
    const definition = isRecord(descriptor)
      ? semanticDefinitions.get(`${descriptor.scope}:${descriptor.semantic}`)
      : undefined;
    const requiresInterval = definition?.sampledOverInterval ?? false;
    if (
      !isRecord(descriptor) ||
      descriptor.id !== index ||
      !semantics.has(descriptor.semantic) ||
      !scopes.has(descriptor.scope) ||
      !units.has(descriptor.unit) ||
      !sources.has(descriptor.source) ||
      !definition ||
      descriptor.unit !== definition.unit ||
      descriptor.network_scope !==
        networkScopeDefinition(descriptor.semantic, descriptor.scope, descriptor.source) ||
      (requiresInterval
        ? !safeInteger(descriptor.interval_ms) || descriptor.interval_ms === 0
        : descriptor.interval_ms !== null)
    ) {
      return `Descriptor ${index} is invalid.`;
    }
  }
  if (
    payload.limitations.some(
      (entry) =>
        !isRecord(entry) ||
        !limitationCodes.has(entry.code) ||
        typeof entry.message !== "string" ||
        entry.message.trim().length === 0,
    )
  ) {
    return "Limitation catalog is malformed.";
  }
  const systemError = validateSystem(payload.system, payload);
  if (systemError) return systemError;
  const workloadError = validateWorkloads(payload.workloads, payload);
  if (workloadError) return workloadError;
  const warningKeys = new Set<string>();
  for (const warning of payload.warnings) {
    if (
      !isRecord(warning) ||
      !nonEmptyString(warning.key) ||
      warningKeys.has(warning.key) ||
      !safeInteger(warning.publication_seq) ||
      warning.publication_seq > payload.publication_seq ||
      !safeInteger(warning.occurred_at_ms) ||
      warning.occurred_at_ms > payload.published_at_ms ||
      !nonEmptyString(warning.category) ||
      !nonEmptyString(warning.message)
    ) {
      return "Runtime warnings are malformed.";
    }
    warningKeys.add(warning.key);
  }
  return null;
}

function validateSystem(input: unknown, payload: Record<string, any>): string | null {
  if (
    !isRecord(input) ||
    input.stable_id !== "system:local" ||
    !Array.isArray(input.logical_cpus) ||
    !Array.isArray(input.kernel_pool_tags)
  )
    return "System identity or facets are malformed.";
  const systemMetricError = validateObservations(input.metrics, "system", payload);
  if (systemMetricError) return systemMetricError;

  const logicalIds = new Set<string>();
  const logicalIndexes = new Set<number>();
  for (const logical of input.logical_cpus) {
    if (
      !isRecord(logical) ||
      !safeInteger(logical.index) ||
      logical.stable_id !== `system:local:cpu:${logical.index}` ||
      logicalIds.has(logical.stable_id) ||
      logicalIndexes.has(logical.index)
    )
      return "Logical CPU identity is malformed or duplicated.";
    logicalIds.add(logical.stable_id);
    logicalIndexes.add(logical.index);
    const error = validateObservations(logical.metrics, "system", payload);
    if (error) return error;
  }

  const poolIds = new Set<string>();
  for (const tag of input.kernel_pool_tags) {
    if (
      !isRecord(tag) ||
      !nonEmptyString(tag.tag) ||
      !kernelPoolKinds.has(tag.kind) ||
      tag.stable_id !== `system:local:pool:${tag.tag}:${tag.kind}`.toLocaleLowerCase() ||
      poolIds.has(tag.stable_id) ||
      !Array.isArray(tag.driver_candidates) ||
      tag.driver_candidates.some((candidate) => !nonEmptyString(candidate)) ||
      typeof tag.driver_candidates_pending !== "boolean"
    )
      return "Kernel pool tag is malformed or duplicated.";
    poolIds.add(tag.stable_id);
    const error = validateObservations(tag.metrics, "system", payload);
    if (error) return error;
  }
  return null;
}

function validatePersistence(input: unknown, evaluatedAtMs: number): string | null {
  if (input === null) return null;
  if (
    !isRecord(input) ||
    !persistenceStates.has(input.state) ||
    !Array.isArray(input.roots) ||
    !Array.isArray(input.components) ||
    !safeInteger(input.suppressed_diagnostic_events)
  )
    return "Runtime persistence health is malformed.";

  const rootOwners = new Set<string>();
  let worstState: "healthy" | "degraded" | "unavailable" | null = null;
  const recordState = (state: "healthy" | "degraded" | "unavailable") => {
    const rank = { healthy: 0, degraded: 1, unavailable: 2 } as const;
    if (worstState === null || rank[state] > rank[worstState]) worstState = state;
  };
  for (const root of input.roots) {
    if (
      !isRecord(root) ||
      !persistenceOwners.has(root.owner) ||
      rootOwners.has(root.owner) ||
      (root.directory !== null && typeof root.directory !== "string") ||
      !persistencePermissions.has(root.permission_state) ||
      (root.permission_state === "verified" &&
        (typeof root.directory !== "string" || root.directory.trim().length === 0)) ||
      (root.permission_state === "unavailable" && root.directory !== null)
    )
      return "Runtime persistence root is malformed or duplicated.";
    rootOwners.add(root.owner);
    recordState(
      root.permission_state === "verified"
        ? "healthy"
        : root.permission_state === "invalid"
          ? "degraded"
          : "unavailable",
    );
  }

  const componentKeys = new Set<string>();
  for (const component of input.components) {
    if (!isRecord(component)) return "Runtime persistence component is malformed.";
    const key = `${component.owner}:${component.kind}`;
    const failure = component.active_failure;
    if (
      !persistenceOwners.has(component.owner) ||
      !rootOwners.has(component.owner) ||
      !persistenceKinds.has(component.kind) ||
      componentKeys.has(key) ||
      !persistenceStates.has(component.state) ||
      !persistenceDurability.has(component.durability) ||
      (component.last_success_at_ms !== null &&
        (!safeInteger(component.last_success_at_ms) ||
          component.last_success_at_ms > evaluatedAtMs)) ||
      (failure !== null &&
        (!isRecord(failure) ||
          typeof failure.code !== "string" ||
          failure.code.trim().length === 0 ||
          !persistenceOperations.has(failure.operation) ||
          !safeInteger(failure.occurred_at_ms) ||
          failure.occurred_at_ms > evaluatedAtMs ||
          typeof failure.retryable !== "boolean" ||
          typeof failure.summary !== "string" ||
          failure.summary.trim().length === 0))
    )
      return "Runtime persistence component is malformed or duplicated.";
    const requiresFailure = component.state === "degraded" || component.state === "unavailable";
    if (requiresFailure !== (failure !== null))
      return "Runtime persistence component failure state is inconsistent.";
    if (
      component.state === "healthy" &&
      (component.durability === "session_only" || component.durability === "not_written")
    )
      return "Runtime persistence component durability is inconsistent.";
    if (
      component.durability === "not_applicable" &&
      (component.last_success_at_ms !== null || failure !== null)
    )
      return "Runtime persistence not-applicable component carries state.";
    componentKeys.add(key);
    if (component.durability !== "not_applicable")
      recordState(component.state as "healthy" | "degraded" | "unavailable");
  }
  if (input.state !== "unavailable" && (input.roots.length === 0 || input.components.length === 0))
    return "Runtime persistence active state requires roots and components.";
  if (input.state !== (worstState ?? "unavailable"))
    return "Runtime persistence overall state is inconsistent.";
  return null;
}

function validateCollectorService(
  privileged: Record<string, any>,
  health: Record<string, any>,
  desktopReleaseIdentity: unknown,
): string | null {
  if (
    privileged.last_success_at_ms !== null &&
    privileged.last_success_at_ms > health.evaluated_at_ms
  )
    return "Privileged collection success occurs after health evaluation.";
  const service = privileged.collector_service;
  if (service === null) {
    return privileged.source === "collector_service"
      ? "Collector-service source has no service status."
      : null;
  }
  if (
    !isRecord(service) ||
    !collectorServiceStates.has(service.state) ||
    (service.release_identity !== null && !validReleaseIdentity(service.release_identity)) ||
    (service.service_version !== null && typeof service.service_version !== "string") ||
    (service.negotiated_protocol_version !== null &&
      !safeInteger(service.negotiated_protocol_version)) ||
    (service.minimum_desktop_version !== null &&
      typeof service.minimum_desktop_version !== "string") ||
    (service.instance_id !== null && typeof service.instance_id !== "string") ||
    (service.last_connected_at_ms !== null &&
      (!safeInteger(service.last_connected_at_ms) ||
        service.last_connected_at_ms > health.evaluated_at_ms)) ||
    (service.detail !== null && typeof service.detail !== "string")
  )
    return "Collector-service status is malformed.";
  if (
    service.state === "active" &&
    (!validReleaseIdentity(service.release_identity) ||
      typeof service.service_version !== "string" ||
      service.service_version.trim().length === 0 ||
      service.negotiated_protocol_version === null ||
      typeof service.instance_id !== "string" ||
      service.instance_id.trim().length === 0 ||
      service.last_connected_at_ms === null)
  )
    return "Active collector-service status lacks identity, version, or time.";
  if (
    service.state === "active" &&
    !serviceReleaseMatchesDesktop(service.release_identity, desktopReleaseIdentity)
  )
    return "Active collector-service release identity does not match desktop.";
  if (
    service.state === "incompatible" &&
    (typeof service.service_version !== "string" ||
      service.service_version.trim().length === 0 ||
      typeof service.minimum_desktop_version !== "string" ||
      service.minimum_desktop_version.trim().length === 0)
  )
    return "Incompatible collector-service status lacks version detail.";
  if (
    privileged.source === "collector_service" &&
    (privileged.state !== "active" || service.state !== "active")
  )
    return "Collector-service source is not active.";
  if (privileged.source === "local_process" && service.state === "active")
    return "Local and collector-service collection cannot both be active.";
  return null;
}

function serviceReleaseMatchesDesktop(
  service: unknown,
  desktop: unknown,
): boolean {
  if (!validReleaseIdentity(service) || !validReleaseIdentity(desktop)) return false;
  if (!isRecord(service) || !isRecord(desktop)) return false;
  return (
    service.app_version === desktop.app_version &&
    (service.source_commit_sha === null || service.source_commit_sha === desktop.source_commit_sha)
  );
}

function validReleaseIdentity(value: unknown): boolean {
  if (!isRecord(value)) return false;
  if (
    typeof value.app_version !== "string" ||
    value.app_version.trim().length === 0 ||
    new TextEncoder().encode(value.app_version).length > 64
  )
    return false;
  return (
    value.source_commit_sha === null ||
    (typeof value.source_commit_sha === "string" &&
      /^[0-9a-fA-F]{40}$/.test(value.source_commit_sha))
  );
}

function validateWorkloads(input: unknown, payload: Record<string, any>): string | null {
  if (!Array.isArray(input)) return "Workload catalog is not an array.";
  const workloads = input;
  const processIds = new Set<string>();
  const processes = new Map<string, Record<string, any>>();
  for (const workload of workloads) {
    if (!isRecord(workload) || !isRecord(workload.detail)) return "Workload is malformed.";
    if (workload.kind === "process") {
      const detail = workload.detail;
      const presentation = detail.presentation;
      if (
        !nonEmptyString(detail.stable_id) ||
        processIds.has(detail.stable_id) ||
        parseJsSafeDecimal(detail.pid, true) === null ||
        !processIdentityStabilities.has(detail.identity_stability) ||
        (detail.parent_pid !== null && typeof detail.parent_pid !== "string") ||
        (detail.parent_process_id !== null && typeof detail.parent_process_id !== "string") ||
        !nonEmptyString(detail.display_name) ||
        typeof detail.executable !== "string" ||
        !nonEmptyString(detail.status) ||
        !accessStates.has(detail.access_state) ||
        !isRecord(presentation) ||
        (presentation.group_id !== null && typeof presentation.group_id !== "string") ||
        !nonEmptyString(presentation.group_key) ||
        !nonEmptyString(presentation.group_label) ||
        !nonEmptyString(presentation.group_category) ||
        !safeInteger(presentation.group_count) ||
        presentation.group_count === 0 ||
        !nonEmptyString(presentation.icon_kind) ||
        typeof presentation.is_child !== "boolean" ||
        typeof presentation.is_grouped !== "boolean"
      )
        return "Process identity or presentation is malformed or duplicated.";
      const startTime = detail.start_time_ms;
      const expectedId =
        startTime === null
          ? `process:${detail.pid}:publication:${payload.sample_seq}`
          : `process:${detail.pid}:${startTime}`;
      if (
        (startTime !== null && (!safeInteger(startTime) || startTime === 0)) ||
        (startTime === null
          ? detail.identity_stability !== "publication"
          : detail.identity_stability !== "stable") ||
        detail.stable_id !== expectedId
      )
        return "Process stable identity does not match its PID and start time.";
      processIds.add(detail.stable_id);
      processes.set(detail.stable_id, detail);
      const error = validateObservations(detail.metrics, "process", payload);
      if (error) return error;
    } else if (workload.kind !== "group") return "Workload kind is unknown.";
  }
  const claimedMembers = new Set<string>();
  const groupIds = new Set<string>();
  for (const process of processes.values()) {
    if (process.parent_process_id !== null) {
      const parent = processes.get(process.parent_process_id);
      if (!parent || process.parent_pid === null || parent.pid !== process.parent_pid)
        return "Parent process identity is inconsistent.";
    }
  }
  for (const workload of workloads) {
    if (workload.kind !== "group") continue;
    const detail = workload.detail;
    if (
      !isRecord(detail) ||
      !nonEmptyString(detail.stable_id) ||
      !nonEmptyString(detail.group_key) ||
      !nonEmptyString(detail.label) ||
      !nonEmptyString(detail.category) ||
      !nonEmptyString(detail.icon_kind) ||
      (detail.icon_source !== null && !nonEmptyString(detail.icon_source)) ||
      (detail.example_label !== null && !nonEmptyString(detail.example_label)) ||
      groupIds.has(detail.stable_id) ||
      !Array.isArray(detail.member_ids) ||
      detail.member_ids.length < 2 ||
      detail.member_ids.some((member) => !nonEmptyString(member)) ||
      !Array.isArray(detail.coverage)
    )
      return "Group identity, presentation, or coverage is malformed.";
    if (detail.stable_id !== `group:${detail.group_key}`)
      return "Group stable identity does not match its group key.";
    groupIds.add(detail.stable_id);
    if (new Set(detail.member_ids).size !== detail.member_ids.length)
      return "Group member identity is duplicated.";
    for (const member of detail.member_ids) {
      const process = processes.get(member);
      if (!process || claimedMembers.has(member))
        return "Group member identity is dangling or reused.";
      if (
        !process.presentation.is_grouped ||
        process.presentation.group_id !== detail.stable_id ||
        process.presentation.group_key !== detail.group_key ||
        process.presentation.group_label !== detail.label ||
        process.presentation.group_category !== detail.category ||
        process.presentation.group_count !== detail.member_ids.length ||
        process.presentation.icon_kind !== detail.icon_kind
      )
        return "Group presentation does not match its membership.";
      claimedMembers.add(member);
    }
    const error = validateObservations(detail.metrics, "group", payload);
    if (error) return error;
    if (detail.coverage.length !== detail.metrics.length)
      return "Group coverage count does not match observations.";
    const coverageByDescriptor = new Map<number, Record<string, any>>();
    for (const coverage of detail.coverage) {
      if (
        !isRecord(coverage) ||
        !safeInteger(coverage.descriptor_index) ||
        coverageByDescriptor.has(coverage.descriptor_index) ||
        !safeInteger(coverage.available_contributors) ||
        !safeInteger(coverage.total_contributors) ||
        coverage.available_contributors > coverage.total_contributors ||
        coverage.total_contributors !== detail.member_ids.length ||
        !validIndex(coverage.limitation_index, payload.limitations.length)
      )
        return "Group coverage is malformed or inconsistent with membership.";
      coverageByDescriptor.set(coverage.descriptor_index, coverage);
    }
    for (const observation of detail.metrics) {
      const coverage = coverageByDescriptor.get(observation[0]);
      if (!coverage) return "Group coverage is inconsistent with membership.";
      if (coverage.limitation_index !== observation[4])
        return "Group coverage limitation does not match its observation.";
      if (
        coverage.available_contributors < coverage.total_contributors &&
        coverage.limitation_index === null
      )
        return "Group coverage loss is unexplained.";
      const quality = payload.quality_codes[observation[2]];
      if (
        coverage.available_contributors < coverage.total_contributors &&
        (quality === "native" || quality === "estimated")
      )
        return "Group quality contradicts contributor coverage.";
    }
  }
  for (const [id, process] of processes) {
    if (
      process.presentation.is_grouped !== claimedMembers.has(id) ||
      process.presentation.is_grouped !== (process.presentation.group_id !== null) ||
      (!process.presentation.is_grouped &&
        (process.presentation.is_child || process.presentation.group_count !== 1))
    ) {
      return "Process grouping state is inconsistent.";
    }
  }
  const visible = processes.size;
  if (
    !safeInteger(payload.visible_process_count) ||
    !safeInteger(payload.total_process_count) ||
    payload.visible_process_count !== visible ||
    payload.total_process_count < visible
  )
    return "Process counts are inconsistent.";
  const seenContributorMetrics = new Set<string>();
  for (const contributor of payload.contributors) {
    if (
      !isRecord(contributor) ||
      !contributorMetrics.has(contributor.metric) ||
      seenContributorMetrics.has(contributor.metric) ||
      !safeInteger(contributor.quality_code) ||
      !payload.quality_codes[contributor.quality_code] ||
      !safeInteger(contributor.available_contributors) ||
      !safeInteger(contributor.total_contributors) ||
      contributor.available_contributors > contributor.total_contributors ||
      contributor.total_contributors !== payload.total_process_count ||
      !sources.has(contributor.source) ||
      (contributor.process_id !== null &&
        (typeof contributor.process_id !== "string" ||
          !validProcessId(contributor.process_id, payload.sample_seq))) ||
      (contributor.display_name !== null && typeof contributor.display_name !== "string") ||
      typeof contributor.name_ambiguous !== "boolean" ||
      !validIndex(contributor.limitation_index, payload.limitations.length)
    ) {
      return "Process contributor metadata is malformed.";
    }
    const quality = payload.quality_codes[contributor.quality_code];
    if (
      contributor.process_id !== null &&
      (!nonEmptyString(contributor.display_name) ||
        contributor.available_contributors !== contributor.total_contributors ||
        contributor.total_contributors === 0 ||
        quality === "held" ||
        quality === "unavailable")
    )
      return "Process contributor identity is inconsistent.";
    if (contributor.process_id === null && contributor.display_name !== null)
      return "Process contributor name lacks stable identity.";
    if (
      contributor.available_contributors < contributor.total_contributors &&
      contributor.limitation_index === null
    )
      return "Process contributor coverage is unexplained.";
    const qualityError = validateQualityLimitation(quality, contributor.limitation_index, payload);
    if (qualityError) return qualityError;
    if (
      contributor.source === "unknown" &&
      (quality !== "unavailable" ||
        (contributor.limitation_index === null
          ? null
          : payload.limitations[contributor.limitation_index]?.code) !== "missing_metadata")
    )
      return "Process contributor source contradicts its quality.";
    if (
      contributor.available_contributors < contributor.total_contributors &&
      (quality === "native" || quality === "estimated")
    )
      return "Process contributor quality contradicts coverage.";
    seenContributorMetrics.add(contributor.metric);
  }
  if (seenContributorMetrics.size !== contributorMetrics.size || payload.contributors.length !== 4)
    return "Process contributor catalog is incomplete.";
  return null;
}

function validateObservations(
  input: unknown,
  scope: MetricScope,
  payload: Record<string, any>,
): string | null {
  if (!Array.isArray(input)) return `${scope} observations are not an array.`;
  const observed = new Set<string>();
  for (const candidate of input) {
    if (!Array.isArray(candidate) || candidate.length !== 5)
      return `${scope} observation tuple is malformed.`;
    const observation = candidate;
    if (!safeInteger(observation[0]) || !safeInteger(observation[2]))
      return `${scope} observation indexes are malformed.`;
    const descriptor = payload.descriptors[observation[0]];
    const quality = payload.quality_codes[observation[2]];
    if (!descriptor || descriptor.scope !== scope || !quality)
      return `${scope} observation descriptor or quality is invalid.`;
    if (observed.has(descriptor.semantic))
      return `${scope} subject repeats ${descriptor.semantic}.`;
    observed.add(descriptor.semantic);
    if (observation[1] !== null && !nonNegativeFiniteNumber(observation[1]))
      return `${scope} observation value is invalid.`;
    if (quality === "unavailable" && observation[1] !== null)
      return `${scope} unavailable observation carries a value.`;
    if (
      observation[1] === null &&
      !(quality === "unavailable" || (quality === "held" && observation[4] !== null))
    )
      return `${scope} null observation carries publishable quality.`;
    if ((observation[1] === null) !== (observation[3] === null))
      return `${scope} observation value/time state is inconsistent.`;
    if (observation[3] !== null && !safeInteger(observation[3]))
      return `${scope} observation time is invalid.`;
    if (observation[3] !== null && observation[3] > payload.published_at_ms)
      return `${scope} observation time is after publication.`;
    if (
      quality === "held" &&
      observation[3] !== null &&
      payload.sampled_at_ms !== null &&
      observation[3] > payload.sampled_at_ms
    )
      return `${scope} held observation time is after the sample.`;
    if (!validIndex(observation[4], payload.limitations.length))
      return `${scope} limitation index is invalid.`;
    const qualityError = validateQualityLimitation(quality, observation[4], payload);
    if (qualityError) return `${scope} ${qualityError}`;
    if (
      descriptor.source === "unknown" &&
      (quality !== "unavailable" ||
        (observation[4] === null ? null : payload.limitations[observation[4]]?.code) !==
          "missing_metadata")
    )
      return `${scope} observation source contradicts its quality.`;
  }
  return null;
}

function validateQualityLimitation(
  quality: unknown,
  limitationIndex: unknown,
  payload: Record<string, any>,
): string | null {
  if (!qualityCodes.some((candidate) => candidate === quality)) return "quality code is unknown.";
  if (!validIndex(limitationIndex, payload.limitations.length))
    return "quality limitation index is invalid.";
  const code =
    limitationIndex === null ? null : (payload.limitations[limitationIndex]?.code ?? null);
  if ((quality === "held" || quality === "partial" || quality === "unavailable") && code === null)
    return "quality requires a typed explanation.";
  const valid =
    quality === "native"
      ? code === null
      : quality === "estimated"
        ? code !== "pending_baseline" && code !== "held_value" && code !== "group_partial_coverage"
        : quality === "held"
          ? code === "pending_baseline" || code === "held_value"
          : quality === "partial"
            ? code !== null &&
              code !== "pending_baseline" &&
              code !== "held_value" &&
              code !== "numeric_range"
            : code !== null && code !== "pending_baseline" && code !== "held_value";
  return valid ? null : "quality and limitation code contradict each other.";
}

function mismatch(
  writerVersion: number | null,
  minimumReaderVersion: number | null,
  reason: string,
  message: string,
): ProtocolDecodeResult {
  return {
    kind: "protocol_mismatch",
    mismatch: { writerVersion, minimumReaderVersion, reason, message },
  };
}

function safeInteger(value: unknown): value is number {
  return typeof value === "number" && Number.isSafeInteger(value) && value >= 0;
}

function finiteNumber(value: unknown): value is number {
  return typeof value === "number" && Number.isFinite(value);
}

function nonNegativeFiniteNumber(value: unknown): value is number {
  return finiteNumber(value) && value >= 0;
}

function nonEmptyString(value: unknown): value is string {
  return typeof value === "string" && value.trim().length > 0;
}

function networkScopeDefinition(
  semantic: MetricSemantic,
  scope: MetricScope,
  source: string,
): NetworkScopeV3 | null {
  if (source === "unknown") return null;
  const systemNetwork = [
    "network_receive_total",
    "network_transmit_total",
    "network_receive_rate",
    "network_transmit_rate",
  ].includes(semantic);
  if (scope === "system" && systemNetwork)
    return source === "sysinfo" ? "all_interface_aggregate" : "non_loopback_interface_aggregate";
  if (
    (scope === "process" && ["network_receive_rate", "network_transmit_rate"].includes(semantic)) ||
    (scope === "group" && semantic === "network_rate")
  )
    return "ip_socket_payload";
  return null;
}

function validProcessId(value: string, sampleSeq: number): boolean {
  const parts = value.split(":");
  if (parts[0] !== "process" || parseJsSafeDecimal(parts[1], true) === null) return false;
  if (parts.length === 3) return parseJsSafeDecimal(parts[2], false) !== null;
  if (parts.length !== 4 || parts[2] !== "publication") return false;
  return parseJsSafeDecimal(parts[3], true) === sampleSeq;
}

function parseJsSafeDecimal(value: unknown, allowZero: boolean): number | null {
  if (
    typeof value !== "string" ||
    value.length === 0 ||
    (value.length > 1 && value.startsWith("0")) ||
    /[^0-9]/.test(value)
  )
    return null;
  const parsed = Number(value);
  return safeInteger(parsed) && (allowZero || parsed > 0) ? parsed : null;
}

function validIndex(index: unknown, length: number): index is number | null {
  return index === null || (safeInteger(index) && index < length);
}

function isRecord(value: unknown): value is Record<string, any> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}
