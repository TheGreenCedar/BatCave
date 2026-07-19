import assert from "node:assert/strict";
import test from "node:test";
import {
  NarrativeController,
  buildNarrativeFactPacket,
  makeNarrativeInvocation,
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

test("validation accepts one bounded sentence made only from supplied facts", () => {
  const invocation = fixtureInvocation();
  const accepted = validateNarrativeResult(
    invocation,
    resultFor(invocation, "Code Helper is the top CPU contributor at 12% right now."),
  );
  assert.equal(accepted?.text, "Code Helper is the top CPU contributor at 12% right now.");
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
    validateNarrativeResult(invocation, resultFor(invocation, "CPU is likely to reach 99%.")),
    null,
  );
  assert.equal(
    validateNarrativeResult(invocation, resultFor(invocation, `${"x".repeat(181)}.`)),
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
  resolveGeneration?.(resultFor(invocation, "CPU is currently at 12%."));
  assert.equal((await pending)?.text, "CPU is currently at 12%.");
  assert.equal((await controller.request(invocation))?.text, "CPU is currently at 12%.");
  assert.equal(calls, 1);
});

test("controller rate limits changing samples for one subject", async () => {
  let now = 10_000;
  let calls = 0;
  const controller = new NarrativeController(
    async (invocation) => {
      calls += 1;
      return resultFor(invocation, "CPU is currently at 12%.");
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
  finish?.(resultFor(invocation, "CPU is currently at 12%."));
  assert.equal(await pending, null);
  assert.equal(await controller.request(invocation), null);
});
