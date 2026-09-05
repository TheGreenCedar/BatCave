import assert from "node:assert/strict";
import test from "node:test";
import {
  NarrativeController,
  buildNarrativeFactPacket,
  admittedNarrativeCandidates,
  renderNarrative,
  isNarrativeRelevant,
  makeNarrativeInvocation,
  narrativeRelevanceKey,
  validateNarrativeResult,
  type NarrativeInvocation,
  type NarrativeResult,
} from "./narratives.ts";

function fixtureInvocation(publicationSeq = 21): NarrativeInvocation {
  const facts = buildNarrativeFactPacket({
    displayName: "Code Helper (Renderer)",
    category: "Development",
    cpuPercent: 12.04,
    memoryBytes: 522 * 1024 ** 2,
    ioBytesPerSecond: 3_100,
    networkBytesPerSecond: 0,
    leadingResource: "cpu",
    rankingState: "top_contributor",
    measurementLimitations: [{ kind: "network", quality: "unavailable" }],
  });
  return makeNarrativeInvocation("workload_insight", publicationSeq, facts, "workload:code-helper");
}

function resultFor(
  invocation: NarrativeInvocation,
  explanationId: NarrativeResult["explanation_id"] = "cpu_usage",
): NarrativeResult {
  return {
    provider: "apple_foundation",
    publication_seq: invocation.request.publication_seq,
    fact_digest: invocation.request.fact_digest,
    surface: invocation.request.surface,
    ...(invocation.request.subject_stable_id
      ? { subject_stable_id: invocation.request.subject_stable_id }
      : {}),
    explanation_id: explanationId,
  };
}

test("fact packets round only allowed workload facts and have stable digests", () => {
  const first = fixtureInvocation();
  const second = fixtureInvocation();
  assert.deepEqual(first, second);
  assert.deepEqual(first.facts.metrics, [
    { kind: "cpu", rounded_value: 12, unit: "percent" },
    { kind: "memory", rounded_value: 522, unit: "megabytes" },
    { kind: "io", rounded_value: 3, unit: "kilobytes_per_second" },
    { kind: "network", rounded_value: 0, unit: "kilobytes_per_second" },
  ]);
  const serialized = JSON.stringify(first.facts);
  assert.doesNotMatch(serialized, /pid|path|collector|provenance|diagnostic/iu);
});

test("fact packet text stays inside the native provider bounds", () => {
  const facts = buildNarrativeFactPacket({
    displayName: `  ${"w".repeat(140)}\nignored  `,
    category: "c".repeat(100),
    cpuPercent: 0,
    memoryBytes: 0,
    ioBytesPerSecond: 0,
    networkBytesPerSecond: 0,
    rankingState: "normal",
  });
  assert.equal(facts.display_name.length, 120);
  assert.equal(facts.category.length, 80);
  assert.doesNotMatch(facts.display_name, /[\r\n\t]/u);
});

test("0.1% CPU never admits heavy pressure from vocabulary overlap", () => {
  const base = fixtureInvocation();
  const facts = {
    ...base.facts,
    metrics: base.facts.metrics.map((metric) => ({
      ...metric,
      rounded_value: metric.kind === "cpu" ? 0.1 : metric.rounded_value,
    })),
  };
  const invocation = makeNarrativeInvocation("workload_insight", 21, facts, "workload:code-helper");
  const legacy = {
    provider: "apple_foundation",
    publication_seq: 21,
    fact_digest: invocation.request.fact_digest,
    text: "Code Helper (Renderer) is showing heavy CPU pressure right now.",
  };
  // @ts-expect-error Deliberate hostile old-provider envelope.
  assert.equal(validateNarrativeResult(invocation, legacy), null);
  const accepted = validateNarrativeResult(invocation, resultFor(invocation));
  assert.equal(
    renderNarrative(accepted, facts, "workload_insight", "workload:code-helper"),
    "Code Helper (Renderer): 0.1% CPU relative to one logical core in this sample.",
  );
});

test("validation binds selection to offered evidence, subject, surface, publication and provider", () => {
  const invocation = fixtureInvocation();
  assert.ok(validateNarrativeResult(invocation, resultFor(invocation)));
  for (const hostile of [
    { ...resultFor(invocation), publication_seq: 20 },
    { ...resultFor(invocation), fact_digest: "different" },
    { ...resultFor(invocation), subject_stable_id: "workload:other" },
    { ...resultFor(invocation), surface: "overview_contributor" },
    { ...resultFor(invocation), explanation_id: "network_activity" },
    { ...resultFor(invocation), explanation_id: "cpu_heavy_pressure" },
    { ...resultFor(invocation), provider: "external" },
  ]) {
    // @ts-expect-error Runtime boundary receives malformed enum values deliberately.
    assert.equal(validateNarrativeResult(invocation, hostile), null);
  }
});

test("candidate admission excludes zero, unavailable and stale evidence and rejects invalid units", () => {
  const base = fixtureInvocation().facts;
  assert.deepEqual(admittedNarrativeCandidates(base), [
    "cpu_usage",
    "memory_usage",
    "disk_activity",
  ]);
  for (const quality of ["unavailable", "stale"] satisfies Array<"unavailable" | "stale">) {
    assert.deepEqual(
      admittedNarrativeCandidates({
        ...base,
        measurement_limitations: [
          { kind: "cpu", quality },
          { kind: "memory", quality },
          { kind: "io", quality },
        ],
      }),
      [],
    );
  }
  for (const roundedValue of [Number.NaN, Number.POSITIVE_INFINITY, -1, 0.11, 0.10000000001]) {
    assert.deepEqual(
      admittedNarrativeCandidates({
        ...base,
        metrics: [{ kind: "cpu", rounded_value: roundedValue, unit: "percent" }],
      }),
      [],
    );
  }
  assert.deepEqual(
    admittedNarrativeCandidates({
      ...base,
      metrics: [{ kind: "cpu", rounded_value: 12, unit: "megabytes" }],
    }),
    [],
  );
});

test("selection survives routine refresh while rendering only current measurements", () => {
  const invocation = fixtureInvocation();
  const accepted = validateNarrativeResult(invocation, resultFor(invocation));
  assert.ok(accepted);
  const refreshed = {
    ...invocation.facts,
    metrics: invocation.facts.metrics.map((metric) => ({
      ...metric,
      rounded_value: metric.kind === "cpu" ? 0.1 : metric.rounded_value,
    })),
  };
  assert.equal(narrativeRelevanceKey(refreshed), narrativeRelevanceKey(invocation.facts));
  assert.equal(
    renderNarrative(accepted, refreshed, "workload_insight", "workload:code-helper"),
    "Code Helper (Renderer): 0.1% CPU relative to one logical core in this sample.",
  );
  const noCpu = {
    ...refreshed,
    metrics: refreshed.metrics.map((metric) => ({
      ...metric,
      rounded_value: metric.kind === "cpu" ? 0 : metric.rounded_value,
    })),
  };
  assert.equal(
    isNarrativeRelevant(accepted, noCpu, "workload_insight", "workload:code-helper"),
    false,
  );
  assert.equal(renderNarrative(accepted, refreshed, "workload_insight", "workload:other"), null);
});

test("host owns current units and qualifications and invalidates quality changes", () => {
  const base = fixtureInvocation();
  const facts = {
    ...base.facts,
    display_name: "Code 2022",
    measurement_limitations: [
      { kind: "memory", quality: "estimated" },
      { kind: "io", quality: "limited" },
    ],
  } satisfies NarrativeInvocation["facts"];
  const invocation = makeNarrativeInvocation("workload_insight", 22, facts, "workload:code-2022");
  for (const [id, expected] of [
    ["memory_usage", "Code 2022: 522 MiB of memory in this sample (estimated)."],
    [
      "disk_activity",
      "Code 2022: 3 KiB/s of recorded read/write I/O in this sample (limited coverage).",
    ],
  ] satisfies Array<[NarrativeResult["explanation_id"], string]>) {
    const accepted = validateNarrativeResult(invocation, resultFor(invocation, id));
    assert.equal(
      renderNarrative(accepted, facts, "workload_insight", "workload:code-2022"),
      expected,
    );
    assert.equal(
      renderNarrative(
        accepted,
        { ...facts, measurement_limitations: [] },
        "workload_insight",
        "workload:code-2022",
      ),
      null,
    );
  }
});

test("controller caches exact facts and never runs more than one generation", async () => {
  const invocation = fixtureInvocation();
  let resolveGeneration: ((result: NarrativeResult) => void) | undefined;
  let calls = 0;
  const controller = new NarrativeController((current) => {
    calls += 1;
    return new Promise((resolve) => {
      resolveGeneration = resolve;
      assert.equal(current.request.fact_digest, invocation.request.fact_digest);
    });
  });

  const pending = controller.request(invocation);
  assert.equal(await controller.request(fixtureInvocation(22)), null);
  resolveGeneration?.(resultFor(invocation));
  assert.equal((await pending)?.explanation_id, "cpu_usage");
  assert.equal((await controller.request(invocation))?.explanation_id, "cpu_usage");
  assert.equal(calls, 1);
});

test("controller rate limits changing samples for one subject", async () => {
  let now = 10_000;
  let calls = 0;
  const controller = new NarrativeController(
    async (invocation) => {
      calls += 1;
      return resultFor(invocation);
    },
    { now: () => now, minimumIntervalMs: 30_000 },
  );
  assert.ok(await controller.request(fixtureInvocation(21)));
  assert.equal(await controller.request(fixtureInvocation(22)), null);
  now += 30_000;
  assert.ok(await controller.request(fixtureInvocation(22)));
  assert.equal(calls, 2);
});

test("cancel and teardown discard an in-flight result", async () => {
  const invocation = fixtureInvocation();
  let finish: ((result: NarrativeResult) => void) | undefined;
  const controller = new NarrativeController(
    () =>
      new Promise((resolve) => {
        finish = resolve;
      }),
  );
  const pending = controller.request(invocation);
  controller.dispose();
  finish?.(resultFor(invocation));
  assert.equal(await pending, null);
  assert.equal(await controller.request(invocation), null);
});

test("controller does not infer when facts offer fewer than two explanations", async () => {
  let calls = 0;
  const controller = new NarrativeController(async (invocation) => {
    calls += 1;
    return resultFor(invocation);
  });
  const base = fixtureInvocation();
  const facts = {
    ...base.facts,
    metrics: base.facts.metrics.map((metric) => ({
      ...metric,
      rounded_value: metric.kind === "cpu" ? 0.1 : 0,
    })),
  };
  const invocation = makeNarrativeInvocation("workload_insight", 21, facts, "workload:code-helper");
  assert.equal(await controller.request(invocation), null);
  assert.equal(calls, 0);
});

test("overview offers only its selected resource and never infers to restate one fact", async () => {
  let calls = 0;
  const controller = new NarrativeController(async (invocation) => {
    calls += 1;
    return resultFor(invocation, "memory_usage");
  });
  const facts = {
    ...fixtureInvocation().facts,
    leading_resource: "memory",
  } satisfies NarrativeInvocation["facts"];
  assert.deepEqual(admittedNarrativeCandidates(facts, "overview_contributor"), ["memory_usage"]);
  const invocation = makeNarrativeInvocation(
    "overview_contributor",
    21,
    facts,
    "workload:code-helper",
  );
  assert.equal(await controller.request(invocation), null);
  assert.equal(calls, 0);
});
