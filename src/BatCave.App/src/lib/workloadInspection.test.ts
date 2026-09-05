import test from "node:test";
import { readFileSync } from "node:fs";
import assert from "node:assert/strict";
import {
  FixtureInspectionArchive,
  InspectionRequestGate,
  decodeWorkloadInspection,
  getWorkloadInspection,
  inspectionSeries,
} from "./workloadInspection.ts";
import { makeFixtureSnapshot } from "./fixtures.ts";

test("independent inspection retains A through filter, B, and back to A", () => {
  const archive = new FixtureInspectionArchive();
  const snapshot = makeFixtureSnapshot(0);
  archive.observe(snapshot);
  const a = snapshot.process_view_rows.find((row) => row.kind === "process")!.detail.workload_id;
  const initial = archive.read(a, 72);
  assert.equal(initial.status, "current");
  const next = structuredClone(snapshot);
  next.publication_seq += 1;
  next.sample_seq += 1;
  next.sampled_at_ms = (next.sampled_at_ms ?? 0) + 1000;
  next.published_at_ms = next.sampled_at_ms;
  archive.observe(next);
  archive.read("process:unrelated:1", 72);
  assert.equal(archive.read(a, 72).history.length, 2);
  next.process_view_rows = [];
  next.overview_rows = [];
  next.sample_seq += 1;
  next.publication_seq += 1;
  next.sampled_at_ms += 1000;
  next.published_at_ms = next.sampled_at_ms;
  archive.observe(next);
  const exited = archive.read(a, 72);
  assert.equal(exited.status, "exited");
  assert.equal(exited.row?.detail.workload_id, a);
  assert.equal(exited.history.length, 2);
});

test("late responses cannot cross A to B to A or overwrite newer publications", () => {
  const gate = new InspectionRequestGate();
  const oldA = gate.begin("A", 72, 1);
  gate.begin("B", 72, 1);
  const newA = gate.begin("A", 72, 2);
  assert.equal(gate.accept(oldA, { stable_id: "A", publication_seq: 2 }), false);
  assert.equal(gate.accept(newA, { stable_id: "A", publication_seq: 1 }), false);
  assert.equal(gate.accept(newA, { stable_id: "A", publication_seq: 2 }), true);
  const more = gate.begin("A", 180, 3);
  assert.equal(gate.accept(newA, { stable_id: "A", publication_seq: 4 }), false);
  assert.equal(gate.accept(more, { stable_id: "A", publication_seq: 3 }), true);
});

test("hidden inspection panes issue no reads and reopening refreshes the retained selection", () => {
  const gate = new InspectionRequestGate();
  const first = gate.beginForVisiblePane("A", 72, 1, true);
  assert.ok(first);
  let reads = 1;
  // Each hidden publication used to fetch and normalize another complete family/history.
  for (let publication = 2; publication <= 81; publication++) {
    if (gate.beginForVisiblePane("A", 72, publication, false)) reads++;
  }
  assert.equal(reads, 1);
  assert.equal(gate.accept(first, { stable_id: "A", publication_seq: 81 }), false);
  const reopened = gate.beginForVisiblePane("A", 180, 81, true);
  assert.ok(reopened, "same ID and publication still refresh when its pane reopens");
  assert.equal(reopened.pointLimit, 180, "the current history window is requested");
  assert.equal(gate.accept(reopened, { stable_id: "A", publication_seq: 80 }), false);
  assert.equal(gate.accept(reopened, { stable_id: "A", publication_seq: 81 }), true);
  gate.beginForVisiblePane("B", 180, 81, true);
  const backToA = gate.beginForVisiblePane("A", 180, 81, true);
  assert.ok(backToA);
  assert.equal(gate.accept(reopened, { stable_id: "A", publication_seq: 82 }), false);
  assert.equal(gate.accept(backToA, { stable_id: "A", publication_seq: 81 }), true);
});

test("a response delivered after its pane hides is decoded and acknowledged before discard", async () => {
  const wire: unknown = JSON.parse(
    readFileSync(
      new URL("../../src-tauri/src/fixtures/workload-inspection-v1.json", import.meta.url),
      "utf8",
    ),
  );
  const expected = decodeWorkloadInspection(wire);
  const gate = new InspectionRequestGate();
  const ticket = gate.beginForVisiblePane(expected.stable_id, 72, expected.publication_seq, true);
  assert.ok(ticket);
  const calls: string[] = [];
  let deliver: ((value: unknown) => void) | undefined;
  const delivery = new Promise<unknown>((resolve) => {
    deliver = resolve;
  });
  const invoke = async <T>(command: string): Promise<T> => {
    calls.push(command);
    return (command === "get_workload_inspection" ? await delivery : undefined) as T;
  };
  const pending = getWorkloadInspection(invoke, expected.stable_id, 72);
  gate.beginForVisiblePane(expected.stable_id, 72, expected.publication_seq, false);
  assert.ok(deliver);
  deliver(wire);
  const response = await pending;
  assert.deepEqual(calls, ["get_workload_inspection", "acknowledge_workload_inspection"]);
  assert.equal(gate.accept(ticket, response), false);
});

test("inspection decoder rejects unvalidated details and contradictory history", () => {
  assert.throws(() =>
    decodeWorkloadInspection({
      inspection_version: 1,
      runtime_protocol_version: 4,
      stable_id: "A",
      publication_seq: 2,
      sample_seq: 2,
      status: "current",
      catalog: null,
      history: [],
    }),
  );
  assert.throws(() => decodeWorkloadInspection({ inspection_version: 2 }));
});

test("timestamp chart retains real zero and inserts nulls for gaps", () => {
  const native = {
    value: 0,
    quality: "native" as const,
    source: "fixture" as const,
    network_scope: null,
    available: 1,
    total: 1,
  };
  const point = {
    sample_seq: 1,
    sampled_at_ms: 1000,
    interval_ms: 1000,
    gap_before: true,
    cpu: native,
    memory: native,
    io: native,
    network: native,
  };
  const series = inspectionSeries([point, { ...point, sample_seq: 2, sampled_at_ms: 5000 }], "cpu");
  assert.deepEqual(series[1], [0, null, 0]);
  assert.deepEqual(series[0], [1, 3, 5]);
});

test("history quality, scope, chronology, identity and retention are validated at the boundary", () => {
  const archive = new FixtureInspectionArchive();
  const snapshot = makeFixtureSnapshot(8);
  archive.observe(snapshot);
  const id = snapshot.process_view_rows.find((row) => row.kind === "process")!.detail.workload_id;
  const initial = archive.read(id, 72);
  const wire = { inspection_version: 1, runtime_protocol_version: 4, ...initial };
  assert.equal(decodeWorkloadInspection(wire).row?.detail.workload_id, id);
  const corruptions = [
    (value: typeof wire) => {
      value.history[0].cpu.quality = "held";
      value.history[0].cpu.value = 0;
    },
    (value: typeof wire) => {
      value.history[0].cpu.source = "unknown";
    },
    (value: typeof wire) => {
      value.history[0].cpu.available = 2;
      value.history[0].cpu.total = 1;
    },
    (value: typeof wire) => {
      value.history[0].sample_seq += 1;
    },
    (value: typeof wire) => {
      value.history.push(structuredClone(value.history[0]));
    },
    (value: typeof wire) => {
      value.stable_id = "process:99999:1";
    },
    (value: typeof wire) => {
      value.retained_points = 360;
      value.history_truncated = false;
    },
  ];
  for (const corrupt of corruptions) {
    const value = structuredClone(wire);
    corrupt(value);
    assert.throws(() => decodeWorkloadInspection(value));
  }
  const zero = structuredClone(wire);
  zero.history[0].cpu = {
    value: 0,
    quality: "native",
    source: "fixture",
    network_scope: null,
    available: 1,
    total: 1,
  };
  assert.equal(decodeWorkloadInspection(zero).history[0].cpu.value, 0);
  for (const quality of ["held", "unavailable"] as const) {
    const gap = structuredClone(wire);
    gap.history[0].cpu = {
      value: null,
      quality,
      source: "fixture",
      network_scope: null,
      available: 0,
      total: 1,
    };
    assert.equal(inspectionSeries(decodeWorkloadInspection(gap).history, "cpu")[1][0], null);
  }
});

test("the production Rust inspection response validates and retains canonical group members", () => {
  const response: unknown = JSON.parse(
    readFileSync(
      new URL("../../src-tauri/src/fixtures/workload-inspection-v1.json", import.meta.url),
      "utf8",
    ),
  );
  const inspection = decodeWorkloadInspection(response);
  assert.equal(inspection.status, "current");
  assert.equal(inspection.row?.kind, "group");
  assert.ok((inspection.catalog?.workloads.length ?? 0) > 1);
  assert.equal(inspection.history.length, 1);
});

test("delivered inspection replies are acknowledged after decoding, including malformed replies", async () => {
  const archive = new FixtureInspectionArchive();
  const snapshot = makeFixtureSnapshot(8);
  archive.observe(snapshot);
  const id = snapshot.process_view_rows[0].detail.workload_id;
  const wire = { inspection_version: 1, runtime_protocol_version: 4, ...archive.read(id, 72) };
  for (const valid of [true, false]) {
    const calls: string[] = [];
    const payload = { ...wire, inspection_version: valid ? 1 : 999 };
    const invoke = async <T>(command: string, args?: Record<string, unknown>): Promise<T> => {
      calls.push(command);
      if (command === "get_workload_inspection") return payload as T;
      assert.equal(args?.responseToken, wire.response_token);
      return undefined as T;
    };
    if (valid) assert.equal((await getWorkloadInspection(invoke, id, 72)).stable_id, id);
    else await assert.rejects(getWorkloadInspection(invoke, id, 72));
    assert.deepEqual(calls, ["get_workload_inspection", "acknowledge_workload_inspection"]);
  }
});
