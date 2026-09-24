/// <reference types="node" />

import assert from "node:assert/strict";
import test from "node:test";

import { startRuntimePolling, type RuntimeVisibility } from "./runtimePolling.ts";

class FakeScheduler {
  #nextId = 1;
  #timers = new Map<number, { callback: () => void; delayMs: number }>();
  #intervals = new Map<number, { callback: () => void; intervalMs: number }>();

  setTimeout = (callback: () => void, delayMs: number): number => {
    const id = this.#nextId++;
    this.#timers.set(id, { callback, delayMs });
    return id;
  };

  clearTimeout = (timeoutId: number): void => {
    this.#timers.delete(timeoutId);
  };

  setInterval = (callback: () => void, intervalMs: number): number => {
    const id = this.#nextId++;
    this.#intervals.set(id, { callback, intervalMs });
    return id;
  };

  clearInterval = (intervalId: number): void => {
    this.#intervals.delete(intervalId);
  };

  get intervalCount(): number {
    return this.#intervals.size;
  }

  tickIntervals(): void {
    for (const interval of this.#intervals.values()) interval.callback();
  }

  get pending(): number {
    return this.#timers.size;
  }

  pendingDelays(): number[] {
    return [...this.#timers.values()].map((timer) => timer.delayMs);
  }

  runNext(): void {
    const entry = this.#timers.entries().next();
    assert.ok(!entry.done, "a timer is pending");
    this.#timers.delete(entry.value[0]);
    entry.value[1].callback();
  }
}

class FakeVisibility implements RuntimeVisibility {
  hidden = false;
  #callbacks = new Set<() => void>();

  isHidden(): boolean {
    return this.hidden;
  }

  subscribe(callback: () => void): () => void {
    this.#callbacks.add(callback);
    return () => this.#callbacks.delete(callback);
  }

  emit(hidden: boolean): void {
    this.hidden = hidden;
    for (const callback of this.#callbacks) callback();
  }
}

function deferred(): { promise: Promise<void>; resolve: () => void } {
  let resolve!: () => void;
  const promise = new Promise<void>((res) => {
    resolve = res;
  });
  return { promise, resolve };
}

async function flush(): Promise<void> {
  await Promise.resolve();
  await Promise.resolve();
}

test("hidden state clears the pending timer and stops scheduling", async () => {
  const scheduler = new FakeScheduler();
  const visibility = new FakeVisibility();
  let polls = 0;
  const stop = startRuntimePolling({
    initialDelayMs: 120,
    intervalMs: () => 500,
    poll: async () => {
      polls += 1;
    },
    scheduler,
    visibility,
  });

  visibility.emit(true);
  assert.equal(scheduler.pending, 0, "hidden clears the pending timer");
  assert.equal(polls, 0);
  stop();
});

test("hidden before the first poll prevents the initial timer", () => {
  const scheduler = new FakeScheduler();
  const visibility = new FakeVisibility();
  visibility.hidden = true;
  const stop = startRuntimePolling({
    initialDelayMs: 120,
    intervalMs: () => 500,
    poll: async () => {},
    scheduler,
    visibility,
  });
  assert.equal(scheduler.pending, 0);
  stop();
});

test("visible again restarts the loop with a zero delay", async () => {
  const scheduler = new FakeScheduler();
  const visibility = new FakeVisibility();
  let polls = 0;
  const stop = startRuntimePolling({
    initialDelayMs: 120,
    intervalMs: () => 500,
    poll: async () => {
      polls += 1;
    },
    scheduler,
    visibility,
  });

  visibility.emit(true);
  assert.equal(scheduler.pending, 0);

  visibility.emit(false);
  assert.deepEqual(scheduler.pendingDelays(), [0]);

  scheduler.runNext();
  await flush();
  assert.equal(polls, 1);
  assert.deepEqual(scheduler.pendingDelays(), [500]);
  stop();
});

test("repeated visibility events never run two loops", async () => {
  const scheduler = new FakeScheduler();
  const visibility = new FakeVisibility();
  let polls = 0;
  const stop = startRuntimePolling({
    initialDelayMs: 0,
    intervalMs: () => 500,
    poll: async () => {
      polls += 1;
    },
    scheduler,
    visibility,
  });

  visibility.emit(false);
  visibility.emit(false);
  visibility.emit(true);
  visibility.emit(false);
  assert.equal(scheduler.pending, 1, "one pending timer");

  scheduler.runNext();
  await flush();
  assert.equal(polls, 1);
  assert.equal(scheduler.pending, 1);
  stop();
});

test("a poll still in flight when hidden resumes once, never overlapping", async () => {
  const scheduler = new FakeScheduler();
  const visibility = new FakeVisibility();
  const gate = deferred();
  let started = 0;
  let finished = 0;
  const stop = startRuntimePolling({
    initialDelayMs: 0,
    intervalMs: () => 500,
    poll: async () => {
      started += 1;
      await gate.promise;
      finished += 1;
    },
    scheduler,
    visibility,
  });

  scheduler.runNext();
  assert.equal(started, 1);

  // Visibility churn while the poll is in flight must not start a second poll.
  visibility.emit(true);
  visibility.emit(false);
  visibility.emit(false);
  await flush();
  assert.equal(started, 1, "no overlapping poll started");
  assert.equal(scheduler.pending, 0, "no duplicate timer while running");

  gate.resolve();
  await flush();
  assert.equal(finished, 1);
  assert.equal(scheduler.pending, 1, "next interval scheduled after the poll");
  stop();
});

test("dispose clears pending timers and unsubscribes visibility", async () => {
  const scheduler = new FakeScheduler();
  const visibility = new FakeVisibility();
  const stop = startRuntimePolling({
    initialDelayMs: 0,
    intervalMs: () => 500,
    poll: async () => {},
    scheduler,
    visibility,
  });

  stop();
  assert.equal(scheduler.pending, 0);
  visibility.emit(false);
  assert.equal(scheduler.pending, 0, "no restart after dispose");
});

test("watchdog restarts polling when visibility flips without an event", async () => {
  const scheduler = new FakeScheduler();
  const visibility = new FakeVisibility();
  visibility.hidden = true;
  let polls = 0;
  const stop = startRuntimePolling({
    initialDelayMs: 120,
    intervalMs: () => 500,
    poll: async () => {
      polls += 1;
    },
    scheduler,
    visibility,
  });

  assert.equal(scheduler.pending, 0, "hidden at start schedules nothing");
  assert.equal(scheduler.intervalCount, 1, "watchdog armed");

  // The page became visible without delivering a visibilitychange event.
  visibility.hidden = false;
  scheduler.tickIntervals();
  assert.deepEqual(scheduler.pendingDelays(), [0]);

  scheduler.runNext();
  await flush();
  assert.equal(polls, 1);
  stop();
});

test("watchdog never starts a second concurrent loop", async () => {
  const scheduler = new FakeScheduler();
  const visibility = new FakeVisibility();
  let polls = 0;
  const stop = startRuntimePolling({
    initialDelayMs: 0,
    intervalMs: () => 500,
    poll: async () => {
      polls += 1;
    },
    scheduler,
    visibility,
  });

  // The initial poll timer is already pending; the watchdog must not add another.
  assert.equal(scheduler.pending, 1);
  scheduler.tickIntervals();
  assert.equal(scheduler.pending, 1, "watchdog dedupes against a pending poll");

  scheduler.runNext();
  await flush();
  assert.equal(polls, 1);
  assert.equal(scheduler.pending, 1, "interval timer pending after the poll");
  scheduler.tickIntervals();
  assert.equal(scheduler.pending, 1, "watchdog dedupes against the interval timer");
  stop();
});

test("dispose clears the watchdog", () => {
  const scheduler = new FakeScheduler();
  const visibility = new FakeVisibility();
  const stop = startRuntimePolling({
    initialDelayMs: 0,
    intervalMs: () => 500,
    poll: async () => {},
    scheduler,
    visibility,
  });

  assert.equal(scheduler.intervalCount, 1);
  stop();
  assert.equal(scheduler.intervalCount, 0, "watchdog cleared on dispose");
});
