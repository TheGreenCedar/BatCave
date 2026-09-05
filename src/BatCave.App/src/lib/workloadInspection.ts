import {
  RUNTIME_PROTOCOL_POLICY,
  RUNTIME_PROTOCOL_VERSION,
  type MetricQualityV4,
  type MetricSourceV4,
  type NetworkScopeV4,
  type MetricSemantic,
  type WorkloadDetailV4,
} from "./generated/runtime-protocol-v4.ts";
import { adaptWorkloadRows } from "./protocol/runtimeAdapter.ts";
import { decodeWorkloadCatalog, type WorkloadCatalog } from "./protocol/runtimeProtocol.ts";
import { encodeFixtureSnapshot } from "./protocol/fixtureProtocol.ts";
import type { ProcessViewRow, RuntimeSnapshot } from "./types.ts";
export type HistoryPointLimit = 30 | 72 | 180 | 360;

export interface HistoryObservation {
  value: number | null;
  quality: MetricQualityV4;
  source: MetricSourceV4;
  network_scope: NetworkScopeV4 | null;
  available: number;
  total: number;
}
export interface WorkloadHistoryPoint {
  sample_seq: number;
  sampled_at_ms: number;
  interval_ms: number;
  gap_before: boolean;
  cpu: HistoryObservation;
  memory: HistoryObservation;
  io: HistoryObservation;
  network: HistoryObservation;
}
export type HistoryMetric = "cpu" | "memory" | "io" | "network";
interface InspectionBase {
  response_token: string;
  stable_id: string;
  publication_seq: number;
  sample_seq: number;
  retained_points: number;
  history_truncated: boolean;
  history: WorkloadHistoryPoint[];
}
export type WorkloadInspection = InspectionBase &
  (
    | { status: "current" | "exited"; row: ProcessViewRow; catalog: WorkloadCatalog }
    | { status: "evicted" | "unknown"; row: null; catalog: null }
  );

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
  "nstat",
  "procfs",
  "ebpf",
  "fixture",
]);
const qualities = new Set<string>(RUNTIME_PROTOCOL_POLICY.quality_codes);
function record(input: unknown): input is Record<string, unknown> {
  return typeof input === "object" && input !== null && !Array.isArray(input);
}
function integer(input: unknown): input is number {
  return typeof input === "number" && Number.isSafeInteger(input) && input >= 0;
}
function observation(input: unknown): input is HistoryObservation {
  if (
    !record(input) ||
    (input.value !== null &&
      (typeof input.value !== "number" || !Number.isFinite(input.value) || input.value < 0)) ||
    typeof input.quality !== "string" ||
    !qualities.has(input.quality) ||
    typeof input.source !== "string" ||
    !sources.has(input.source) ||
    !(
      input.network_scope === null ||
      input.network_scope === "non_loopback_interface_aggregate" ||
      input.network_scope === "all_interface_aggregate" ||
      input.network_scope === "ip_socket_payload"
    ) ||
    !integer(input.available) ||
    !integer(input.total) ||
    input.total < 1 ||
    input.available > input.total
  )
    return false;
  const gap =
    input.quality === "held" ||
    input.quality === "unavailable" ||
    input.source === "unknown" ||
    input.available === 0;
  if (gap !== (input.value === null)) return false;
  return !(
    input.available < input.total &&
    (input.quality === "native" || input.quality === "estimated")
  );
}
function historyPoint(input: unknown): input is WorkloadHistoryPoint {
  return (
    record(input) &&
    integer(input.sample_seq) &&
    integer(input.sampled_at_ms) &&
    integer(input.interval_ms) &&
    input.interval_ms >= 500 &&
    input.interval_ms <= 5000 &&
    typeof input.gap_before === "boolean" &&
    observation(input.cpu) &&
    observation(input.memory) &&
    observation(input.io) &&
    observation(input.network)
  );
}
export function decodeWorkloadInspection(input: unknown): WorkloadInspection {
  if (
    !record(input) ||
    input.inspection_version !== 1 ||
    typeof input.response_token !== "string" ||
    !/^inspection:[1-9][0-9]*$/.test(input.response_token) ||
    input.runtime_protocol_version !== RUNTIME_PROTOCOL_VERSION ||
    typeof input.stable_id !== "string" ||
    !input.stable_id ||
    input.stable_id.length > 1024 ||
    !integer(input.publication_seq) ||
    !integer(input.sample_seq) ||
    !integer(input.retained_points) ||
    input.retained_points > 360 ||
    typeof input.history_truncated !== "boolean" ||
    !Array.isArray(input.history) ||
    input.history.length > 360 ||
    !input.history.every(historyPoint)
  )
    throw new Error("Workload inspection response is malformed.");
  const history = input.history;
  if (
    history.length > input.retained_points ||
    (history.length < input.retained_points && !input.history_truncated)
  )
    throw new Error("History retention metadata is inconsistent.");
  for (let index = 0; index < history.length; index++) {
    const current = history[index];
    const previous = history[index - 1];
    if (
      current.sample_seq > input.sample_seq ||
      (previous &&
        (current.sample_seq <= previous.sample_seq ||
          current.sampled_at_ms < previous.sampled_at_ms ||
          (!current.gap_before &&
            (current.sample_seq !== previous.sample_seq + 1 ||
              current.sampled_at_ms === previous.sampled_at_ms ||
              current.interval_ms !== previous.interval_ms ||
              historyScopeChanged(previous, current) ||
              current.sampled_at_ms - previous.sampled_at_ms >= previous.interval_ms * 2))))
    )
      throw new Error("Workload history timeline is inconsistent.");
  }
  const base = {
    response_token: input.response_token,
    stable_id: input.stable_id,
    publication_seq: input.publication_seq,
    sample_seq: input.sample_seq,
    retained_points: input.retained_points,
    history_truncated: input.history_truncated,
    history,
  };
  if (input.status === "evicted" || input.status === "unknown") {
    if (
      input.catalog !== null ||
      history.length ||
      input.retained_points !== 0 ||
      input.history_truncated
    )
      throw new Error("Unavailable inspection carries retained measurements.");
    return { ...base, status: input.status, row: null, catalog: null };
  }
  if (input.status !== "current" && input.status !== "exited")
    throw new Error("Workload inspection state is unknown.");
  const catalog = decodeWorkloadCatalog(input.catalog);
  const row = adaptWorkloadRows(catalog.workloads, catalog).process_view_rows.find(
    (row) => row.detail.workload_id === input.stable_id,
  );
  const latest = history.at(-1);
  if (
    !row ||
    !latest ||
    catalog.sample_seq !== latest.sample_seq ||
    catalog.sampled_at_ms !== latest.sampled_at_ms ||
    (input.status === "current"
      ? catalog.sample_seq !== input.sample_seq
      : catalog.sample_seq >= input.sample_seq)
  )
    throw new Error("Workload inspection identity or sample is inconsistent.");
  return { ...base, status: input.status, row, catalog };
}

export async function getWorkloadInspection(
  invoke: <T>(command: string, args?: Record<string, unknown>) => Promise<T>,
  stableId: string,
  pointLimit: HistoryPointLimit,
): Promise<WorkloadInspection> {
  const response = await invoke<unknown>("get_workload_inspection", { stableId, pointLimit });
  const responseToken =
    record(response) &&
    typeof response.response_token === "string" &&
    /^inspection:[1-9][0-9]*$/.test(response.response_token)
      ? response.response_token
      : null;
  try {
    return decodeWorkloadInspection(response);
  } finally {
    // Credits cover transport-owned serialized bodies until delivery and decoding complete,
    // including malformed replies and responses that selection sequencing later discards.
    if (responseToken) await invoke<void>("acknowledge_workload_inspection", { responseToken });
  }
}
interface RequestTicket {
  generation: number;
  stableId: string;
  pointLimit: HistoryPointLimit;
  publication: number;
}
export class InspectionRequestGate {
  private generation = 0;
  private current: RequestTicket | null = null;
  begin(stableId: string, pointLimit: HistoryPointLimit, publication: number): RequestTicket {
    const ticket = { generation: ++this.generation, stableId, pointLimit, publication };
    this.current = ticket;
    return ticket;
  }
  clear(): void {
    this.current = null;
    this.generation++;
  }
  accept(
    ticket: RequestTicket,
    response: Pick<WorkloadInspection, "stable_id" | "publication_seq">,
  ): boolean {
    return (
      this.current?.generation === ticket.generation &&
      response.stable_id === ticket.stableId &&
      response.publication_seq >= ticket.publication
    );
  }
}

export function inspectionSeries(
  history: WorkloadHistoryPoint[],
  metric: HistoryMetric,
): [number[], (number | null)[]] {
  const times: number[] = [],
    values: (number | null)[] = [];
  for (const [index, sample] of history.entries()) {
    const previous = history[index - 1];
    if (previous && sample.gap_before) {
      times.push((previous.sampled_at_ms + sample.sampled_at_ms) / 2000);
      values.push(null);
    }
    times.push(sample.sampled_at_ms / 1000);
    values.push(sample[metric].value);
  }
  return [times, values];
}

// Browser fixture mode is a deterministic layout harness, never a native collection substitute.
export class FixtureInspectionArchive {
  private records = new Map<
    string,
    { catalog: WorkloadCatalog; history: WorkloadHistoryPoint[]; truncated: boolean }
  >();
  private publication = 0;
  private sample = -1;
  observe(snapshot: RuntimeSnapshot): void {
    this.publication = snapshot.publication_seq;
    if (snapshot.sampled_at_ms === null || snapshot.sample_seq <= this.sample) return;
    this.sample = snapshot.sample_seq;
    const envelope = encodeFixtureSnapshot(snapshot);
    if (envelope.event.kind !== "runtime_snapshot") return;
    const payload = envelope.event.payload;
    const catalog = decodeWorkloadCatalog({ ...payload, workloads: payload.workloads });
    for (const workload of catalog.workloads) {
      const id = workload.detail.stable_id;
      const old = this.records.get(id);
      const last = old?.history.at(-1);
      const point: WorkloadHistoryPoint = {
        sample_seq: catalog.sample_seq,
        sampled_at_ms: snapshot.sampled_at_ms,
        interval_ms: snapshot.settings.sample_interval_ms,
        gap_before:
          !last ||
          last.sample_seq + 1 !== catalog.sample_seq ||
          last.interval_ms !== snapshot.settings.sample_interval_ms ||
          snapshot.sampled_at_ms <= last.sampled_at_ms ||
          snapshot.sampled_at_ms - last.sampled_at_ms >= last.interval_ms * 2,
        cpu: fixtureObservation(workload, catalog, "cpu"),
        memory: fixtureObservation(workload, catalog, "memory"),
        io: fixtureObservation(workload, catalog, "io"),
        network: fixtureObservation(workload, catalog, "network"),
      };
      if (last && historyScopeChanged(last, point)) point.gap_before = true;
      this.records.set(id, {
        catalog,
        truncated: !!old?.truncated || (old?.history.length ?? 0) >= 360,
        history: [...(old?.history ?? []).slice(-359), point],
      });
    }
  }
  read(stableId: string, pointLimit: HistoryPointLimit): WorkloadInspection {
    const record = this.records.get(stableId);
    return decodeWorkloadInspection({
      response_token: `inspection:${this.publication + 1}`,
      inspection_version: 1,
      runtime_protocol_version: RUNTIME_PROTOCOL_VERSION,
      stable_id: stableId,
      publication_seq: this.publication,
      sample_seq: this.sample,
      status: record
        ? record.catalog.sample_seq === this.sample
          ? "current"
          : "exited"
        : "unknown",
      catalog: record?.catalog ?? null,
      retained_points: record?.history.length ?? 0,
      history_truncated: !!record && (record.truncated || record.history.length > pointLimit),
      history: record?.history.slice(-pointLimit) ?? [],
    });
  }
}
function fixtureObservation(
  workload: WorkloadDetailV4,
  catalog: WorkloadCatalog,
  metric: HistoryMetric,
): HistoryObservation {
  const group = workload.kind === "group";
  const semantics: MetricSemantic[] =
    metric === "cpu"
      ? ["cpu_usage"]
      : metric === "memory"
        ? ["resident_memory"]
        : metric === "io"
          ? group
            ? ["read_write_io_rate"]
            : ["read_io_rate", "write_io_rate"]
          : group
            ? ["network_rate"]
            : ["network_receive_rate", "network_transmit_rate"];
  const total = group ? workload.detail.member_ids.length : 1;
  const result: HistoryObservation = {
    value: 0,
    quality: "native",
    source: "unknown",
    network_scope: null,
    available: total,
    total,
  };
  const rank = { native: 0, estimated: 1, partial: 2, held: 3, unavailable: 4 };
  for (const semantic of semantics) {
    const measurement = workload.detail.metrics.find(
      (item) => catalog.descriptors[item[0]]?.semantic === semantic,
    );
    if (!measurement) {
      result.value = null;
      result.quality = "unavailable";
      result.available = 0;
      continue;
    }
    const quality = catalog.quality_codes[measurement[2]];
    const source = catalog.descriptors[measurement[0]].source;
    if (rank[quality] > rank[result.quality]) result.quality = quality;
    result.source = source;
    result.network_scope = catalog.descriptors[measurement[0]].network_scope;
    const available = group
      ? (workload.detail.coverage.find((item) => item.descriptor_index === measurement[0])
          ?.available_contributors ?? 0)
      : measurement[1] === null
        ? 0
        : 1;
    result.available = Math.min(result.available, available);
    result.value =
      result.value !== null &&
      measurement[1] !== null &&
      quality !== "held" &&
      quality !== "unavailable" &&
      source !== "unknown" &&
      available > 0
        ? result.value + measurement[1]
        : null;
  }
  return result;
}

function historyScopeChanged(left: WorkloadHistoryPoint, right: WorkloadHistoryPoint): boolean {
  const metrics: HistoryMetric[] = ["cpu", "memory", "io", "network"];
  return metrics.some(
    (metric) =>
      left[metric].source !== right[metric].source ||
      left[metric].network_scope !== right[metric].network_scope ||
      left[metric].available !== right[metric].available ||
      left[metric].total !== right[metric].total,
  );
}
