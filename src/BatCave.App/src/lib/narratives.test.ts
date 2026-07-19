import assert from "node:assert/strict";
import test from "node:test";
import {
  NarrativeController,
  buildNarrativeFactPacket,
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

function resultFor(invocation: NarrativeInvocation, text: string): NarrativeResult {
  return {
    provider: "apple_foundation",
    publication_seq: invocation.request.publication_seq,
    fact_digest: invocation.request.fact_digest,
    text,
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

test("validation accepts one bounded qualitative sentence", () => {
  const invocation = fixtureInvocation();
  const accepted = validateNarrativeResult(
    invocation,
    resultFor(invocation, "Code Helper (Renderer) is the top CPU contributor right now."),
  );
  assert.equal(accepted?.text, "Code Helper (Renderer) is the top CPU contributor right now.");
});

test("validation rejects stale, malformed, multi-sentence, long, and invented numeric output", () => {
  const invocation = fixtureInvocation();
  assert.equal(
    validateNarrativeResult(invocation, {
      ...resultFor(invocation, "Current activity is 12%."),
      publication_seq: 20,
    }),
    null,
  );
  assert.equal(validateNarrativeResult(invocation, resultFor(invocation, "First. Second.")), null);
  assert.equal(
    validateNarrativeResult(invocation, resultFor(invocation, "- Hidden list item")),
    null,
  );
  assert.equal(
    validateNarrativeResult(invocation, resultFor(invocation, "Current activity is 12%.")),
    null,
  );
  assert.equal(
    validateNarrativeResult(invocation, resultFor(invocation, "CPU is likely to reach 99%.")),
    null,
  );
  assert.equal(
    validateNarrativeResult(
      invocation,
      resultFor(
        invocation,
        "The surface area of a large project depends on its components and resources.",
      ),
    ),
    null,
  );
  assert.equal(
    validateNarrativeResult(
      invocation,
      resultFor(
        invocation,
        "Code Helper uses a powerful CPU to perform development tasks efficiently.",
      ),
    ),
    null,
  );
  assert.equal(
    validateNarrativeResult(
      invocation,
      resultFor(invocation, "code Helper is the main CPU contributor right now."),
    ),
    null,
  );
  assert.equal(
    validateNarrativeResult(
      invocation,
      resultFor(invocation, "Code Helper is showing notable memory activity."),
    ),
    null,
  );
  assert.equal(
    validateNarrativeResult(invocation, resultFor(invocation, `${"x".repeat(181)}.`)),
    null,
  );
});

test("qualitative results survive metric refreshes but not semantic changes", () => {
  const invocation = fixtureInvocation();
  const accepted = validateNarrativeResult(
    invocation,
    resultFor(invocation, "Code Helper (Renderer) is the top CPU contributor right now."),
  );
  assert.ok(accepted);

  const refreshedFacts = {
    ...invocation.facts,
    metrics: invocation.facts.metrics.map((metric) => ({
      ...metric,
      rounded_value: metric.rounded_value + 17,
    })),
  };
  assert.equal(narrativeRelevanceKey(refreshedFacts), narrativeRelevanceKey(invocation.facts));
  assert.ok(
    isNarrativeRelevant(accepted, refreshedFacts, "workload_insight", "workload:code-helper"),
  );
  assert.equal(
    isNarrativeRelevant(
      accepted,
      { ...refreshedFacts, leading_resource: "memory" },
      "workload_insight",
      "workload:code-helper",
    ),
    false,
  );
});

test("numbers are allowed only when they are part of the supplied identity", () => {
  const facts = { ...fixtureInvocation().facts, display_name: "Code 2022" };
  const invocation = makeNarrativeInvocation("workload_insight", 22, facts, "workload:code-2022");
  assert.ok(
    validateNarrativeResult(
      invocation,
      resultFor(invocation, "Code 2022 is the top CPU contributor right now."),
    ),
  );
  assert.equal(
    validateNarrativeResult(
      invocation,
      resultFor(invocation, "Code 2022 is the top CPU contributor with 2022 CPU load."),
    ),
    null,
  );
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
  resolveGeneration?.(resultFor(invocation, "Code Helper (Renderer) is the CPU leader right now."));
  assert.equal((await pending)?.text, "Code Helper (Renderer) is the CPU leader right now.");
  assert.equal(
    (await controller.request(invocation))?.text,
    "Code Helper (Renderer) is the CPU leader right now.",
  );
  assert.equal(calls, 1);
});

test("controller rate limits changing samples for one subject", async () => {
  let now = 10_000;
  let calls = 0;
  const controller = new NarrativeController(
    async (invocation) => {
      calls += 1;
      return resultFor(invocation, "Code Helper (Renderer) is the CPU leader right now.");
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
  finish?.(resultFor(invocation, "Code Helper (Renderer) is the CPU leader right now."));
  assert.equal(await pending, null);
  assert.equal(await controller.request(invocation), null);
});
