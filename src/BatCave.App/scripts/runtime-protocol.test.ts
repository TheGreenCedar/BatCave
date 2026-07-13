import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import type {
  MetricSemantic,
  ProtocolEnvelope,
  RuntimeSnapshotPayloadV3,
  WorkloadDetailV3,
} from "../src/lib/generated/runtime-protocol-v3.ts";
import { canonicalKernelPoolStableId } from "../src/lib/protocol/fixtureProtocol.ts";
import { adaptRuntimePayload } from "../src/lib/protocol/runtimeAdapter.ts";
import { decodeProtocolEnvelope } from "../src/lib/protocol/runtimeProtocol.ts";

const windows = fixture("../src-tauri/src/fixtures/runtime-protocol-v3/windows-standard.json");
const elevated = fixture("../src-tauri/src/fixtures/runtime-protocol-v3/windows-elevated.json");
const linux = fixture("../src-tauri/src/fixtures/runtime-protocol-v3/linux-partial.json");
const macos = fixture("../src-tauri/src/fixtures/runtime-protocol-v3/macos-limited.json");
const incompatible = fixture("../src-tauri/src/fixtures/runtime-protocol-v3/incompatible.json");
const transitions = fixtureArray(
  "../src-tauri/src/fixtures/runtime-protocol-v3/quality-transitions.json",
);

test("production fixture validates and preserves workload identity and order", () => {
  const decoded = decodeProtocolEnvelope(windows);
  assert.equal(decoded.kind, "snapshot");
  if (decoded.kind !== "snapshot") return;

  const group = decoded.payload.workloads.find((workload) => workload.kind === "group");
  assert.ok(group && group.kind === "group");
  assert.equal(group.detail.member_ids.length, 2);
  assert.equal(new Set(group.detail.member_ids).size, 2);
  assert.equal(group.detail.coverage.length, group.detail.metrics.length);

  const adapted = adaptRuntimePayload(decoded.payload);
  const wireProcess = firstProcess(decoded.payload);
  assert.equal("attention_label" in wireProcess.detail.presentation, false);
  assert.equal("attention_label" in group.detail, false);
  assert.deepEqual(
    adapted.process_view_rows.map((row) => row.kind),
    decoded.payload.workloads.map((workload) => workload.kind),
  );
  assert.deepEqual(
    adapted.process_view_rows
      .filter((row) => row.kind === "process")
      .map((row) => row.detail.workload_id),
    group.detail.member_ids,
  );
  assert.equal(adapted.process_view_rows[0].attention_label, "CPU activity · telemetry limited");
  assert.equal(adapted.process_view_rows[1].attention_label, "Pending");
  assert.equal(adapted.process_contributors.cpu_process_id, "process:1234:1699999999000");
  assert.deepEqual(adapted.process_contributors.cpu_coverage, { available: 2, total: 2 });
  assert.equal(adapted.process_contributors.network_quality?.quality, "held");
  assert.equal(adapted.process_contributors.network_quality?.limitation_code, "pending_baseline");
  assert.equal(decoded.payload.health.engine_state, null);
  assert.equal(decoded.payload.health.collection_p95_ms, null);
  assert.equal(decoded.payload.persistence, null);
  assert.equal(decoded.payload.settings.ui_preferences, null);
  assert.deepEqual(decoded.payload.environment.release_identity, {
    app_version: "development",
    source_commit_sha: null,
  });
  assert.deepEqual(
    adapted.environment.release_identity,
    decoded.payload.environment.release_identity,
  );
  for (const descriptor of decoded.payload.descriptors) {
    const intervalMetric = ["percent_one_core", "percent_system", "bytes_per_second"].includes(
      descriptor.unit,
    );
    assert.equal(
      descriptor.interval_ms,
      intervalMetric ? decoded.payload.settings.effective_sample_interval_ms : null,
    );
  }
});

test("dynamic fixture pool tags use canonical protocol identities", () => {
  assert.equal(canonicalKernelPoolStableId("Leak", "nonpaged"), "system:local:pool:leak:nonpaged");
});

test("process identifiers require canonical JavaScript-safe decimal PID segments", () => {
  for (const pid of ["0", "9007199254740991"]) {
    const validWorkloadPid = structuredClone(windows);
    rewriteFirstProcessPid(payload(validWorkloadPid), pid);
    assert.equal(decodeProtocolEnvelope(validWorkloadPid).kind, "snapshot");

    for (const suffix of ["1699999999000", `publication:${payload(windows).sample_seq}`]) {
      const validContributorPid = structuredClone(windows);
      payload(validContributorPid).contributors[0].process_id = `process:${pid}:${suffix}`;
      assert.equal(decodeProtocolEnvelope(validContributorPid).kind, "snapshot");
    }
  }

  for (const pid of [
    "01234",
    "9007199254740992",
    "999999999999999999999999999999999999",
    "-1",
    " 1234",
    "1234 ",
    "not-decimal",
  ]) {
    const invalidWorkloadPid = structuredClone(windows);
    rewriteFirstProcessPid(payload(invalidWorkloadPid), pid);
    assertMismatch(invalidWorkloadPid, "process identity");

    for (const suffix of ["1699999999000", `publication:${payload(windows).sample_seq}`]) {
      const invalidContributorPid = structuredClone(windows);
      payload(invalidContributorPid).contributors[0].process_id = `process:${pid}:${suffix}`;
      assertMismatch(invalidContributorPid, "metadata is malformed");
    }
  }
});

test("unsupported process values remain unavailable instead of becoming zero", () => {
  const decoded = decodeProtocolEnvelope(macos);
  assert.equal(decoded.kind, "snapshot");
  if (decoded.kind !== "snapshot") return;

  const process = firstProcess(decoded.payload);
  const privateMemory = observation(decoded.payload, process, "private_memory");
  assert.equal(privateMemory[1], null);
  assert.equal(decoded.payload.quality_codes[privateMemory[2]], "unavailable");

  const adapted = adaptRuntimePayload(decoded.payload);
  assert.ok(Number.isNaN(adapted.processes[0].private_bytes));
  assert.equal(adapted.processes[0].quality?.memory?.quality, "estimated");
});

test("platform fixtures carry their privilege and collection limits", () => {
  const elevatedDecoded = decodeProtocolEnvelope(elevated);
  assert.equal(elevatedDecoded.kind, "snapshot");
  if (elevatedDecoded.kind !== "snapshot") return;
  assert.equal(elevatedDecoded.payload.environment.process_elevation, "elevated");
  assert.equal(elevatedDecoded.payload.privileged_collection.state, "active");
  assert.equal(elevatedDecoded.payload.privileged_collection.source, "local_process");
  assert.equal(elevatedDecoded.payload.privileged_collection.preference, "best_available");

  const linuxDecoded = decodeProtocolEnvelope(linux);
  assert.equal(linuxDecoded.kind, "snapshot");
  if (linuxDecoded.kind !== "snapshot") return;
  const process = firstProcess(linuxDecoded.payload);
  const memory = observation(linuxDecoded.payload, process, "resident_memory");
  assert.equal(linuxDecoded.payload.environment.platform, "linux");
  assert.equal(linuxDecoded.payload.environment.architecture, "aarch64");
  assert.equal(linuxDecoded.payload.quality_codes[memory[2]], "partial");
  assert.equal(linuxDecoded.payload.descriptors[memory[0]].source, "procfs");

  const macosDecoded = decodeProtocolEnvelope(macos);
  assert.equal(macosDecoded.kind, "snapshot");
  if (macosDecoded.kind !== "snapshot") return;
  const macNetwork = macosDecoded.payload.descriptors.find(
    (descriptor) =>
      descriptor.scope === "system" && descriptor.semantic === "network_receive_total",
  );
  assert.equal(macNetwork?.source, "sysinfo");
  assert.equal(macNetwork?.network_scope, "all_interface_aggregate");
});

test("I/O baseline transitions retain totals while holding only rates", () => {
  const states = transitions.map((envelope) => {
    const decoded = decodeProtocolEnvelope(envelope);
    assert.equal(decoded.kind, "snapshot");
    if (decoded.kind !== "snapshot") throw new Error("transition fixture did not decode");
    const process = firstProcess(decoded.payload);
    return {
      payload: decoded.payload,
      total: observation(decoded.payload, process, "read_io_total"),
      rate: observation(decoded.payload, process, "read_io_rate"),
    };
  });

  assert.notEqual(states[0].total[1], null);
  assert.equal(states[0].payload.quality_codes[states[0].total[2]], "native");
  assert.equal(states[0].rate[1], null);
  assert.equal(states[0].payload.quality_codes[states[0].rate[2]], "held");
  assert.notEqual(states[1].rate[1], null);
  assert.equal(states[1].payload.quality_codes[states[1].rate[2]], "native");
  assert.notEqual(states[2].rate[1], null);
  assert.equal(states[2].payload.quality_codes[states[2].rate[2]], "held");
  assert.equal(states[3].rate[1], null);
  assert.equal(states[3].payload.quality_codes[states[3].rate[2]], "unavailable");
});

test("incompatible writers enter the explicit mismatch state", () => {
  const decoded = decodeProtocolEnvelope(incompatible);
  assert.equal(decoded.kind, "protocol_mismatch");
  if (decoded.kind !== "protocol_mismatch") return;
  assert.equal(decoded.mismatch.writerVersion, 4);
  assert.equal(decoded.mismatch.minimumReaderVersion, 4);
  assert.equal(decoded.mismatch.reason, "reader_too_old");
});

test("compatibility metadata has deterministic legacy and additive behavior", () => {
  const legacy = structuredClone(windows);
  legacy.protocol_version = 2;
  legacy.compatibility.minimum_reader_version = 2;
  assert.equal(decodeProtocolEnvelope(legacy).kind, "protocol_mismatch");
  const legacyResult = decodeProtocolEnvelope(legacy);
  assert.equal(
    legacyResult.kind === "protocol_mismatch" ? legacyResult.mismatch.reason : "",
    "legacy_writer",
  );

  const additive = structuredClone(windows);
  additive.protocol_version = 4;
  additive.compatibility.breaking = false;
  assert.equal(decodeProtocolEnvelope(additive).kind, "snapshot");

  const breaking = structuredClone(additive);
  breaking.compatibility.breaking = true;
  const breakingResult = decodeProtocolEnvelope(breaking);
  assert.equal(
    breakingResult.kind === "protocol_mismatch" ? breakingResult.mismatch.reason : "",
    "breaking_writer",
  );
});

test("reader rejects descriptor, value, and membership corruption", () => {
  const wrongScope = structuredClone(windows);
  const wrongScopePayload = payload(wrongScope);
  wrongScopePayload.descriptors[wrongScopePayload.system.metrics[0][0]].scope = "process";
  assertMismatch(wrongScope, "descriptor");

  const wrongUnit = structuredClone(windows);
  const wrongUnitPayload = payload(wrongUnit);
  wrongUnitPayload.descriptors[wrongUnitPayload.system.metrics[0][0]].unit = "bytes";
  assertMismatch(wrongUnit, "descriptor");

  const wrongNetworkScope = structuredClone(windows);
  const wrongNetworkPayload = payload(wrongNetworkScope);
  const networkDescriptor = wrongNetworkPayload.descriptors.find(
    (descriptor) => descriptor.semantic === "network_receive_total",
  );
  assert.ok(networkDescriptor);
  networkDescriptor.network_scope = "ip_socket_payload";
  assertMismatch(wrongNetworkScope, "descriptor");

  const zeroInterval = structuredClone(windows);
  const zeroIntervalPayload = payload(zeroInterval);
  const intervalDescriptor = zeroIntervalPayload.descriptors.find(
    (descriptor) => descriptor.interval_ms !== null,
  );
  assert.ok(intervalDescriptor);
  intervalDescriptor.interval_ms = 0;
  assertMismatch(zeroInterval, "descriptor");

  const unavailableValue = structuredClone(windows);
  const unavailablePayload = payload(unavailableValue);
  unavailablePayload.system.metrics[0][1] = 1;
  unavailablePayload.system.metrics[0][2] = 4;
  assertMismatch(unavailableValue, "unavailable");

  const publishableNull = structuredClone(windows);
  payload(publishableNull).system.metrics[0][1] = null;
  assertMismatch(publishableNull, "null observation");

  const futureObservation = structuredClone(windows);
  const futurePayload = payload(futureObservation);
  futurePayload.system.metrics[0][3] = futurePayload.published_at_ms + 1;
  assertMismatch(futureObservation, "after publication");

  const dangling = structuredClone(windows);
  const danglingGroup = payload(dangling).workloads.find((workload) => workload.kind === "group");
  assert.ok(danglingGroup && danglingGroup.kind === "group");
  danglingGroup.detail.member_ids[0] = "process:missing";
  assertMismatch(dangling, "dangling");

  const forgedIdentity = structuredClone(windows);
  firstProcess(payload(forgedIdentity)).detail.stable_id = "process:1234:1";
  assertMismatch(forgedIdentity, "stable identity");

  const malformedDisplayName = structuredClone(windows);
  (
    firstProcess(payload(malformedDisplayName)).detail as unknown as {
      display_name: unknown;
    }
  ).display_name = 17;
  assertMismatch(malformedDisplayName, "process identity or presentation");

  const malformedAccess = structuredClone(windows);
  (
    firstProcess(payload(malformedAccess)).detail as unknown as {
      access_state: unknown;
    }
  ).access_state = "root";
  assertMismatch(malformedAccess, "process identity or presentation");

  const malformedGroupCount = structuredClone(windows);
  (
    firstProcess(payload(malformedGroupCount)).detail.presentation as unknown as {
      group_count: unknown;
    }
  ).group_count = "many";
  assertMismatch(malformedGroupCount, "process identity or presentation");

  const negativeCoverage = structuredClone(windows);
  firstGroup(payload(negativeCoverage)).detail.coverage[0].available_contributors = -1;
  assertMismatch(negativeCoverage, "group coverage");

  const nullCoverage = structuredClone(windows);
  (firstGroup(payload(nullCoverage)).detail.coverage as unknown[])[0] = null;
  assertMismatch(nullCoverage, "group coverage");

  const malformedSystemIdentity = structuredClone(windows);
  (
    payload(malformedSystemIdentity).system as unknown as {
      stable_id: unknown;
    }
  ).stable_id = null;
  assertMismatch(malformedSystemIdentity, "system identity");

  const malformedPoolKind = structuredClone(windows);
  (
    payload(malformedPoolKind).system.kernel_pool_tags[0] as unknown as {
      kind: unknown;
    }
  ).kind = "invalid";
  assertMismatch(malformedPoolKind, "kernel pool tag");

  const impossibleCount = structuredClone(windows);
  payload(impossibleCount).total_process_count = 1;
  assertMismatch(impossibleCount, "process counts");

  const impossibleFatal = structuredClone(windows);
  payload(impossibleFatal).health.engine_state = "fatal";
  assertMismatch(impossibleFatal, "fatal");

  const nondegradedFatal = structuredClone(windows);
  const nondegradedFatalPayload = payload(nondegradedFatal);
  nondegradedFatalPayload.health.engine_state = "fatal";
  nondegradedFatalPayload.health.fatal_error = {
    code: "runtime_failed",
    message: "The runtime failed.",
    occurred_at_ms: nondegradedFatalPayload.published_at_ms,
  };
  assertMismatch(nondegradedFatal, "must be degraded");

  const nondegradedLimited = structuredClone(windows);
  payload(nondegradedLimited).health.engine_state = "running";
  payload(nondegradedLimited).health.collector_state = "limited";
  assertMismatch(nondegradedLimited, "must be degraded");
  payload(nondegradedLimited).health.degraded = true;
  assert.equal(decodeProtocolEnvelope(nondegradedLimited).kind, "snapshot");

  const nondegradedUnavailable = structuredClone(windows);
  payload(nondegradedUnavailable).health.engine_state = "running";
  payload(nondegradedUnavailable).health.collector_state = "unavailable";
  assertMismatch(nondegradedUnavailable, "must be degraded");

  const forgedRelease = structuredClone(windows);
  payload(forgedRelease).environment.release_identity.source_commit_sha = "not-a-sha";
  assertMismatch(forgedRelease, "release identity");

  const multibyteReleaseBoundary = structuredClone(windows);
  payload(multibyteReleaseBoundary).environment.release_identity.app_version = "é".repeat(32);
  assert.equal(decodeProtocolEnvelope(multibyteReleaseBoundary).kind, "snapshot");

  const multibyteRelease = structuredClone(windows);
  payload(multibyteRelease).environment.release_identity.app_version = "é".repeat(40);
  assertMismatch(multibyteRelease, "release identity");

  const healthyNothing = structuredClone(windows);
  payload(healthyNothing).persistence = {
    state: "healthy",
    roots: [],
    components: [],
    suppressed_diagnostic_events: 0,
  };
  assertMismatch(healthyNothing, "requires roots and components");

  const orphanPersistenceComponent = structuredClone(windows);
  payload(orphanPersistenceComponent).persistence = {
    state: "healthy",
    roots: [{ owner: "current_user", directory: "/tmp/batcave", permission_state: "verified" }],
    components: [
      {
        owner: "collector_service",
        kind: "settings",
        state: "healthy",
        durability: "durable",
        last_success_at_ms: null,
        active_failure: null,
      },
    ],
    suppressed_diagnostic_events: 0,
  };
  assertMismatch(orphanPersistenceComponent, "component");

  const oversizedTheme = structuredClone(windows);
  payload(oversizedTheme).settings.ui_preferences = {
    theme: "x".repeat(65),
    history_point_limit: 72,
  };
  assertMismatch(oversizedTheme, "settings");

  const negativeMetric = structuredClone(windows);
  payload(negativeMetric).system.metrics[0][1] = -1;
  assertMismatch(negativeMetric, "value is invalid");

  const heldWithoutExplanation = structuredClone(windows);
  const heldWithoutExplanationMetric = payload(heldWithoutExplanation).system.metrics[0];
  heldWithoutExplanationMetric[2] = 2;
  heldWithoutExplanationMetric[4] = null;
  assertMismatch(heldWithoutExplanation, "requires a typed explanation");

  const contradictoryQuality = structuredClone(windows);
  const contradictoryMetric = payload(contradictoryQuality).system.metrics[0];
  contradictoryMetric[2] = 2;
  contradictoryMetric[4] = 0;
  assertMismatch(contradictoryQuality, "contradict");

  const missingContributor = structuredClone(windows);
  payload(missingContributor).contributors.pop();
  assertMismatch(missingContributor, "catalog is incomplete");

  const duplicateContributor = structuredClone(windows);
  payload(duplicateContributor).contributors[1].metric = "cpu";
  assertMismatch(duplicateContributor, "metadata is malformed");

  const contributorNameWithoutIdentity = structuredClone(windows);
  payload(contributorNameWithoutIdentity).contributors[3].display_name = "orphan";
  assertMismatch(contributorNameWithoutIdentity, "name lacks stable identity");

  const blankContributorName = structuredClone(windows);
  payload(blankContributorName).contributors[0].display_name = " \t ";
  assertMismatch(blankContributorName, "identity is inconsistent");

  const maxSafeContributorIdentity = structuredClone(windows);
  payload(maxSafeContributorIdentity).contributors[0].process_id = "process:1234:9007199254740991";
  assert.equal(decodeProtocolEnvelope(maxSafeContributorIdentity).kind, "snapshot");

  for (const processId of [
    "process:1234:9007199254740992",
    "process:1234:9999999999999999999999999999999999999999",
  ]) {
    const unsafeContributorIdentity = structuredClone(windows);
    payload(unsafeContributorIdentity).contributors[0].process_id = processId;
    assertMismatch(unsafeContributorIdentity, "metadata is malformed");
  }

  const activeWithoutSource = structuredClone(windows);
  payload(activeWithoutSource).privileged_collection.state = "active";
  assertMismatch(activeWithoutSource, "state and source are inconsistent");

  const futureWarning = structuredClone(windows);
  payload(futureWarning).warnings[0].publication_seq = payload(futureWarning).publication_seq + 1;
  assertMismatch(futureWarning, "warnings are malformed");

  const duplicateWarning = structuredClone(windows);
  payload(duplicateWarning).warnings.push(structuredClone(payload(duplicateWarning).warnings[0]));
  assertMismatch(duplicateWarning, "warnings are malformed");

  const matchingActiveService = structuredClone(windows);
  const matchingPayload = payload(matchingActiveService);
  matchingPayload.privileged_collection = {
    state: "active",
    source: "collector_service",
    preference: "best_available",
    detail: null,
    last_success_at_ms: null,
    collector_service: {
      state: "active",
      release_identity: structuredClone(matchingPayload.environment.release_identity),
      service_version: "1.0.0",
      negotiated_protocol_version: 3,
      minimum_desktop_version: null,
      instance_id: "collector-instance",
      last_connected_at_ms: matchingPayload.health.evaluated_at_ms,
      detail: null,
    },
  };
  assert.equal(decodeProtocolEnvelope(matchingActiveService).kind, "snapshot");

  const mismatchedActiveService = structuredClone(matchingActiveService);
  const collectorIdentity =
    payload(mismatchedActiveService).privileged_collection.collector_service?.release_identity;
  assert.ok(collectorIdentity);
  collectorIdentity.app_version = "different";
  assertMismatch(mismatchedActiveService, "release identity does not match");

  const identityFreeActiveService = structuredClone(windows);
  payload(identityFreeActiveService).privileged_collection = {
    state: "active",
    source: "collector_service",
    preference: "best_available",
    detail: null,
    last_success_at_ms: null,
    collector_service: {
      state: "active",
      release_identity: null,
      service_version: null,
      negotiated_protocol_version: null,
      minimum_desktop_version: null,
      instance_id: null,
      last_connected_at_ms: null,
      detail: null,
    },
  };
  assertMismatch(identityFreeActiveService, "lacks identity");
});

function fixture(relativePath: string): ProtocolEnvelope {
  return JSON.parse(readFileSync(new URL(relativePath, import.meta.url), "utf8"));
}

function fixtureArray(relativePath: string): ProtocolEnvelope[] {
  return JSON.parse(readFileSync(new URL(relativePath, import.meta.url), "utf8"));
}

function payload(envelope: ProtocolEnvelope): RuntimeSnapshotPayloadV3 {
  if (envelope.event.kind !== "runtime_snapshot") throw new Error("expected runtime snapshot");
  return envelope.event.payload;
}

function firstProcess(payload: RuntimeSnapshotPayloadV3) {
  const process = payload.workloads.find((workload) => workload.kind === "process");
  if (!process || process.kind !== "process") throw new Error("expected process workload");
  return process;
}

function rewriteFirstProcessPid(payload: RuntimeSnapshotPayloadV3, pid: string) {
  const process = firstProcess(payload);
  const oldId = process.detail.stable_id;
  const newId =
    process.detail.start_time_ms === null
      ? `process:${pid}:publication:${payload.sample_seq}`
      : `process:${pid}:${process.detail.start_time_ms}`;
  process.detail.pid = pid;
  process.detail.stable_id = newId;
  for (const workload of payload.workloads) {
    if (workload.kind === "group") {
      workload.detail.member_ids = workload.detail.member_ids.map((id) =>
        id === oldId ? newId : id,
      );
    } else if (workload.detail.parent_process_id === oldId) {
      workload.detail.parent_pid = pid;
      workload.detail.parent_process_id = newId;
    }
  }
  for (const contributor of payload.contributors) {
    if (contributor.process_id === oldId) contributor.process_id = newId;
  }
}

function firstGroup(payload: RuntimeSnapshotPayloadV3) {
  const group = payload.workloads.find((workload) => workload.kind === "group");
  if (!group || group.kind !== "group") throw new Error("expected group workload");
  return group;
}

function observation(
  payload: RuntimeSnapshotPayloadV3,
  workload: Extract<WorkloadDetailV3, { kind: "process" }>,
  semantic: MetricSemantic,
) {
  const metric = workload.detail.metrics.find(
    (candidate) => payload.descriptors[candidate[0]].semantic === semantic,
  );
  if (!metric) throw new Error(`missing ${semantic} observation`);
  return metric;
}

function assertMismatch(envelope: ProtocolEnvelope, messageFragment: string) {
  const decoded = decodeProtocolEnvelope(envelope);
  assert.equal(decoded.kind, "protocol_mismatch");
  if (decoded.kind !== "protocol_mismatch") return;
  assert.match(decoded.mismatch.message.toLocaleLowerCase(), new RegExp(messageFragment));
}
