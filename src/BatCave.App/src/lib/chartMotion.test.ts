/// <reference types="node" />

import assert from "node:assert/strict";
import test from "node:test";

import {
  chartFrameData,
  createChartMotion,
  isCompatibleHistoryUpdate,
  shouldSnapChartMotion,
  type ChartMotionFrame,
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

test("a rolling update slides unchanged samples left and enters the new sample", () => {
  const scheduler = new TestScheduler();
  const renders: ChartMotionFrame[] = [];
  const initial = [10, 20, 30];
  const target = [20, 30, 40];
  const motion = createChartMotion(initial, (frame) => renders.push(frame), scheduler);

  motion.update(target);
  assert.deepEqual(renders.at(-1), {
    values: [10, 20, 30, 40],
    offset: 0,
    windowLength: 3,
  });

  scheduler.step(100);
  assert.deepEqual(renders.at(-1), {
    values: [10, 20, 30, 40],
    offset: 0.5,
    windowLength: 3,
  });
  assert.deepEqual(chartFrameData(renders.at(-1)!), [
    [-0.5, 0.5, 1.5, 2.5],
    [10, 20, 30, 40],
  ]);

  scheduler.step(200);
  assert.deepEqual(renders.at(-1), { values: target, offset: 0, windowLength: 3 });
  assert.equal(scheduler.pendingFrames, 0);
  assert.deepEqual(initial, [10, 20, 30]);
  assert.deepEqual(target, [20, 30, 40]);
  assert.notEqual(renders.at(-1)?.values, target);
});

test("a mid-animation rolling update rebases from the current horizontal position", () => {
  const scheduler = new TestScheduler();
  const renders: ChartMotionFrame[] = [];
  const motion = createChartMotion([0, 10, 20], (frame) => renders.push(frame), scheduler);

  motion.update([10, 20, 30]);
  scheduler.step(100);
  assert.equal(renders.at(-1)?.offset, 0.5);

  motion.update([20, 30, 40]);
  assert.deepEqual(renders.at(-1), {
    values: [0, 10, 20, 30, 40],
    offset: 0.5,
    windowLength: 3,
  });
  assert.equal(scheduler.cancelled.length, 1);

  scheduler.step(200);
  assert.equal(renders.at(-1)?.offset, 1.25);
  scheduler.step(300);
  assert.deepEqual(renders.at(-1), {
    values: [20, 30, 40],
    offset: 0,
    windowLength: 3,
  });
});

test("history growth, resets, and incompatible histories snap immediately", () => {
  const scheduler = new TestScheduler();
  const renders: ChartMotionFrame[] = [];
  const motion = createChartMotion([1, 2], (frame) => renders.push(frame), scheduler);

  motion.update([1, 2, 3]);
  assert.deepEqual(renders.at(-1), { values: [1, 2, 3], offset: 0, windowLength: 3 });
  assert.equal(scheduler.pendingFrames, 0);

  motion.update([9, 8, 7]);
  assert.deepEqual(renders.at(-1), { values: [9, 8, 7], offset: 0, windowLength: 3 });
  assert.equal(scheduler.pendingFrames, 0);

  motion.update([]);
  assert.deepEqual(renders.at(-1), { values: [], offset: 0, windowLength: 0 });
  assert.equal(scheduler.pendingFrames, 0);

  assert.equal(isCompatibleHistoryUpdate([1, 2], [1, 2, 3]), false);
  assert.equal(isCompatibleHistoryUpdate([1, 2, 3], [2, 3, 4]), true);
  assert.equal(isCompatibleHistoryUpdate([1, 2, 3], [1, 2]), false);
});

test("hidden and reduced-motion conditions force the current target to finish", () => {
  assert.equal(shouldSnapChartMotion("visible", false), false);
  assert.equal(shouldSnapChartMotion("hidden", false), true);
  assert.equal(shouldSnapChartMotion("visible", true), true);

  const scheduler = new TestScheduler();
  const renders: ChartMotionFrame[] = [];
  const motion = createChartMotion([1, 2, 3], (frame) => renders.push(frame), scheduler);

  motion.update([2, 3, 4], { snap: shouldSnapChartMotion("hidden", false) });
  assert.deepEqual(renders.at(-1), { values: [2, 3, 4], offset: 0, windowLength: 3 });
  assert.equal(scheduler.pendingFrames, 0);

  motion.update([3, 4, 5]);
  assert.equal(scheduler.pendingFrames, 1);
  motion.finish();

  assert.deepEqual(renders.at(-1), { values: [3, 4, 5], offset: 0, windowLength: 3 });
  assert.equal(scheduler.pendingFrames, 0);
  assert.equal(scheduler.cancelled.length, 1);
});

test("destroy cancels pending work and prevents later writes", () => {
  const scheduler = new TestScheduler();
  const renders: ChartMotionFrame[] = [];
  const motion = createChartMotion([1, 2, 3], (frame) => renders.push(frame), scheduler);

  motion.update([2, 3, 4]);
  const renderCount = renders.length;
  motion.destroy();

  assert.equal(scheduler.pendingFrames, 0);
  assert.equal(scheduler.cancelled.length, 1);

  scheduler.step(200);
  motion.update([3, 4, 5]);
  motion.finish();
  assert.equal(renders.length, renderCount);
});

test("single-point and empty frames retain uPlot's stable two-point fallback", () => {
  assert.deepEqual(chartFrameData({ values: [], offset: 0, windowLength: 0 }), [
    [0, 1],
    [0, 0],
  ]);
  assert.deepEqual(chartFrameData({ values: [7], offset: 0, windowLength: 1 }), [
    [0, 1],
    [0, 7],
  ]);
});
