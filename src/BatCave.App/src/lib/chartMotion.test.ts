/// <reference types="node" />

import assert from "node:assert/strict";
import test from "node:test";

import {
  createChartMotion,
  isCompatibleHistoryUpdate,
  shouldSnapChartMotion,
} from "./chartMotion.ts";

class TestScheduler {
  time = 0;
  cancelled: number[] = [];
  #nextFrame = 1;
  #callbacks = new Map<number, FrameRequestCallback>();

  now(): number {
    return this.time;
  }

  request(callback: FrameRequestCallback): number {
    const frame = this.#nextFrame++;
    this.#callbacks.set(frame, callback);
    return frame;
  }

  cancel(frame: number): void {
    this.cancelled.push(frame);
    this.#callbacks.delete(frame);
  }

  step(time: number): void {
    this.time = time;
    const callbacks = [...this.#callbacks.values()];
    this.#callbacks.clear();
    callbacks.forEach((callback) => callback(time));
  }

  get pendingFrames(): number {
    return this.#callbacks.size;
  }
}

test("compatible chart motion reaches an exact cloned target", () => {
  const scheduler = new TestScheduler();
  const renders: number[][] = [];
  const initial = [0.1, 0.2];
  const target = [0.1, 0.2, 0.3];
  const motion = createChartMotion(initial, (values) => renders.push(values), scheduler);

  motion.update(target);
  assert.deepEqual(renders.at(-1), [0.1, 0.1, 0.2]);

  scheduler.step(100);
  assert.deepEqual(
    renders.at(-1)?.map((value) => Number(value.toFixed(2))),
    [0.1, 0.15, 0.25],
  );

  scheduler.step(200);
  assert.deepEqual(renders.at(-1), target);
  assert.equal(scheduler.pendingFrames, 0);
  assert.deepEqual(initial, [0.1, 0.2]);
  assert.deepEqual(target, [0.1, 0.2, 0.3]);
  assert.notEqual(renders.at(-1), target);
});

test("a mid-animation update rebases from the currently rendered frame", () => {
  const scheduler = new TestScheduler();
  const renders: number[][] = [];
  const motion = createChartMotion([0, 10], (values) => renders.push(values), scheduler);

  motion.update([0, 10, 20]);
  scheduler.step(100);
  assert.deepEqual(renders.at(-1), [0, 5, 15]);

  motion.update([0, 10, 20, 30]);
  assert.deepEqual(renders.at(-1), [0, 0, 5, 15]);
  assert.equal(scheduler.cancelled.length, 1);

  scheduler.step(200);
  assert.deepEqual(renders.at(-1), [0, 5, 12.5, 22.5]);
  scheduler.step(300);
  assert.deepEqual(renders.at(-1), [0, 10, 20, 30]);
});

test("resets and incompatible histories snap immediately", () => {
  const scheduler = new TestScheduler();
  const renders: number[][] = [];
  const motion = createChartMotion([1, 2, 3], (values) => renders.push(values), scheduler);

  motion.update([9, 8, 7]);
  assert.deepEqual(renders.at(-1), [9, 8, 7]);
  assert.equal(scheduler.pendingFrames, 0);

  motion.update([]);
  assert.deepEqual(renders.at(-1), []);
  assert.equal(scheduler.pendingFrames, 0);

  assert.equal(isCompatibleHistoryUpdate([1, 2], [1, 2, 3]), true);
  assert.equal(isCompatibleHistoryUpdate([1, 2, 3], [2, 3, 4]), true);
  assert.equal(isCompatibleHistoryUpdate([1, 2, 3], [1, 2]), false);
});

test("hidden and reduced-motion conditions force the current target to finish", () => {
  assert.equal(shouldSnapChartMotion("visible", false), false);
  assert.equal(shouldSnapChartMotion("hidden", false), true);
  assert.equal(shouldSnapChartMotion("visible", true), true);

  const scheduler = new TestScheduler();
  const renders: number[][] = [];
  const motion = createChartMotion([1, 2], (values) => renders.push(values), scheduler);

  motion.update([1, 2, 3], { snap: shouldSnapChartMotion("hidden", false) });
  assert.deepEqual(renders.at(-1), [1, 2, 3]);
  assert.equal(scheduler.pendingFrames, 0);

  motion.update([1, 2, 3, 4]);
  assert.equal(scheduler.pendingFrames, 1);
  motion.finish();

  assert.deepEqual(renders.at(-1), [1, 2, 3, 4]);
  assert.equal(scheduler.pendingFrames, 0);
  assert.equal(scheduler.cancelled.length, 1);
});

test("destroy cancels pending work and prevents later writes", () => {
  const scheduler = new TestScheduler();
  const renders: number[][] = [];
  const motion = createChartMotion([1, 2], (values) => renders.push(values), scheduler);

  motion.update([1, 2, 3]);
  const renderCount = renders.length;
  motion.destroy();

  assert.equal(scheduler.pendingFrames, 0);
  assert.equal(scheduler.cancelled.length, 1);

  scheduler.step(200);
  motion.update([1, 2, 3, 4]);
  motion.finish();
  assert.equal(renders.length, renderCount);
});
