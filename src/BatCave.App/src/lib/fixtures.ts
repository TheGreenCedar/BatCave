import browserLinux from "../../src-tauri/src/fixtures/runtime-protocol-v3/browser-linux.json" with { type: "json" };
import browserMacos from "../../src-tauri/src/fixtures/runtime-protocol-v3/browser-macos.json" with { type: "json" };
import browserMacosDense from "../../src-tauri/src/fixtures/runtime-protocol-v3/browser-macos-dense.json" with { type: "json" };
import browserWindows from "../../src-tauri/src/fixtures/runtime-protocol-v3/browser-windows.json" with { type: "json" };
import type {
  MeasurementDescriptor,
  MetricObservation,
  ProtocolEnvelope,
  RuntimeSnapshotPayloadV3,
} from "./generated/runtime-protocol-v3.ts";
import { adaptRuntimePayload } from "./protocol/runtimeAdapter.ts";
import { decodeProtocolEnvelope } from "./protocol/runtimeProtocol.ts";
import { makeDefaultRuntimeQuery } from "./runtimeSnapshot.ts";
import type { RuntimePlatform, RuntimeQuery, RuntimeSnapshot } from "./types.ts";

const fixtureEpochMs = Date.now();
const canonicalWorkloadPrefixLength = 3;

export type BrowserFixtureDensity = "dense" | "compact";

const fixtureRuntimeEnabled = import.meta.env?.DEV ?? true;
const browserFixtures: Record<Exclude<RuntimePlatform, "fixture">, ProtocolEnvelope> | null =
  fixtureRuntimeEnabled
    ? {
        windows: browserWindows as unknown as ProtocolEnvelope,
        linux: browserLinux as unknown as ProtocolEnvelope,
        macos: browserMacos as unknown as ProtocolEnvelope,
      }
    : null;
const denseMacosFixture = fixtureRuntimeEnabled
  ? (browserMacosDense as unknown as ProtocolEnvelope)
  : null;

export function makeFixtureSnapshot(
  tick: number,
  query: RuntimeQuery = makeDefaultRuntimeQuery(),
  platform: RuntimePlatform = "fixture",
  density: BrowserFixtureDensity = "dense",
): RuntimeSnapshot {
  if (!browserFixtures) throw new Error("Browser fixtures are only available in development.");
  const selectedPlatform = platform === "fixture" ? "windows" : platform;
  const selectedFixture =
    selectedPlatform === "macos" && density === "dense"
      ? denseMacosFixture
      : browserFixtures[selectedPlatform];
  if (!selectedFixture) throw new Error("Dense macOS fixture is unavailable.");
  const envelope = structuredClone(selectedFixture);
  const payload = runtimeSnapshotPayload(envelope);
  applyFixtureState(payload, tick, query, platform);

  const decoded = decodeProtocolEnvelope(envelope);
  if (decoded.kind !== "snapshot") throw new Error(decoded.mismatch.message);
  return adaptRuntimePayload(decoded.payload);
}

function runtimeSnapshotPayload(envelope: ProtocolEnvelope): RuntimeSnapshotPayloadV3 {
  if (envelope.event.kind !== "runtime_snapshot") {
    throw new Error("Browser fixture must contain a runtime snapshot.");
  }
  return envelope.event.payload;
}

function applyFixtureState(
  payload: RuntimeSnapshotPayloadV3,
  tick: number,
  query: RuntimeQuery,
  platform: RuntimePlatform,
): void {
  const sampledAt = fixtureEpochMs + tick * payload.settings.effective_sample_interval_ms;
  payload.publication_seq = tick;
  payload.published_at_ms = sampledAt;
  payload.sample_seq = tick;
  payload.sampled_at_ms = sampledAt;
  payload.source = "fixture";
  payload.settings.query = { ...query };
  payload.health.status_summary = "Fixture telemetry is running.";
  payload.health.evaluated_at_ms = sampledAt;
  payload.health.publication_age_ms = 0;
  payload.health.sample_age_ms = 0;
  payload.health.app_cpu_percent = animatedValue(payload.health.app_cpu_percent, tick, false);
  payload.health.collector_warning_count = 0;
  payload.health.last_warning = null;
  payload.warnings = [];
  payload.environment.release_identity = {
    app_version: "development",
    source_commit_sha: null,
  };

  if (platform === "fixture") {
    payload.environment.platform = "fixture";
    payload.environment.process_elevation = "not_applicable";
    payload.environment.install_kind = "portable";
    payload.environment.data_directory = null;
    payload.privileged_collection = {
      state: "unavailable",
      source: "none",
      preference: "standard_only",
      detail: null,
      last_success_at_ms: null,
      collector_service: null,
    };
  }

  animateObservations(payload.system.metrics, payload.descriptors, tick, sampledAt);
  for (const logicalCpu of payload.system.logical_cpus) {
    animateObservations(logicalCpu.metrics, payload.descriptors, tick, sampledAt);
  }
  for (const poolTag of payload.system.kernel_pool_tags) {
    animateObservations(poolTag.metrics, payload.descriptors, tick, sampledAt);
  }
  for (const workload of payload.workloads) {
    animateObservations(workload.detail.metrics, payload.descriptors, tick, sampledAt);
  }
  rotateFixturePublication(payload, tick);
}

function rotateFixturePublication(payload: RuntimeSnapshotPayloadV3, tick: number): void {
  const prefix = payload.workloads.slice(0, canonicalWorkloadPrefixLength);
  const suffix = payload.workloads.slice(canonicalWorkloadPrefixLength);
  if (suffix.length < 2) return;

  const offset = tick % suffix.length;
  payload.workloads = [...prefix, ...suffix.slice(offset), ...suffix.slice(0, offset)];
}

function animateObservations(
  observations: MetricObservation[],
  descriptors: MeasurementDescriptor[],
  tick: number,
  sampledAt: number,
): void {
  for (const observation of observations) {
    const descriptor = descriptors[observation[0]];
    if (!descriptor || observation[1] === null) continue;

    if (
      descriptor.semantic === "cpu_usage" ||
      descriptor.semantic === "kernel_cpu_usage" ||
      descriptor.semantic === "logical_cpu_usage"
    ) {
      observation[1] = animatedValue(observation[1], tick, false);
    } else if (
      descriptor.semantic === "physical_disk_read_rate" ||
      descriptor.semantic === "physical_disk_write_rate" ||
      descriptor.semantic === "read_io_rate" ||
      descriptor.semantic === "write_io_rate" ||
      descriptor.semantic === "other_io_rate" ||
      descriptor.semantic === "read_write_io_rate" ||
      descriptor.semantic === "network_receive_rate" ||
      descriptor.semantic === "network_transmit_rate" ||
      descriptor.semantic === "network_rate"
    ) {
      observation[1] = animatedValue(observation[1], tick, true);
    }

    if (observation[3] !== null) observation[3] = sampledAt;
  }
}

function animatedValue(value: number, tick: number, integral: boolean): number {
  const factor = 0.8 + (tick % 12) * 0.08;
  const animated = value * factor;
  return integral ? Math.round(animated) : Math.round(animated * 10) / 10;
}
