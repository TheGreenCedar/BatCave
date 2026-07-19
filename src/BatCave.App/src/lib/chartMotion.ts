export const chartMotionDurationMs = 200;

export interface AnimationFrameScheduler {
  now(): number;
  request(callback: FrameRequestCallback): number;
  cancel(frame: number): void;
}

export interface ChartMotionFrame {
  values: number[];
  offset: number;
  windowLength: number;
}

export interface ChartMotion {
  update(values: readonly number[], options?: { snap?: boolean }): void;
  finish(): void;
  destroy(): void;
}

export function createChartMotion(
  initialValues: readonly number[],
  render: (frame: ChartMotionFrame) => void,
  scheduler: AnimationFrameScheduler,
  durationMs = chartMotionDurationMs,
): ChartMotion {
  let targetValues = [...initialValues];
  let renderedFrame = stableFrame(targetValues);
  let animationStartOffset = 0;
  let animationTargetOffset = 0;
  let animationStartedAt = 0;
  let animationFrame: number | undefined;
  let destroyed = false;

  function cancelPendingFrame(): void {
    if (animationFrame === undefined) return;
    scheduler.cancel(animationFrame);
    animationFrame = undefined;
  }

  function write(frame: ChartMotionFrame): void {
    renderedFrame = cloneFrame(frame);
    render(cloneFrame(renderedFrame));
  }

  function finish(): void {
    if (destroyed) return;
    cancelPendingFrame();
    write(stableFrame(targetValues));
  }

  function animate(timestamp: number): void {
    animationFrame = undefined;
    if (destroyed) return;

    const progress = Math.min(1, Math.max(0, (timestamp - animationStartedAt) / durationMs));
    if (progress === 1) {
      write(stableFrame(targetValues));
      return;
    }

    write({
      values: renderedFrame.values,
      offset: interpolate(animationStartOffset, animationTargetOffset, progress),
      windowLength: targetValues.length,
    });
    if (!destroyed) animationFrame = scheduler.request(animate);
  }

  function update(values: readonly number[], options: { snap?: boolean } = {}): void {
    if (destroyed) return;

    const nextValues = [...values];
    if (seriesEqual(targetValues, nextValues)) {
      if (options.snap) finish();
      return;
    }

    const animateUpdate =
      !options.snap && durationMs > 0 && isCompatibleHistoryUpdate(targetValues, nextValues);

    cancelPendingFrame();

    if (!animateUpdate) {
      targetValues = nextValues;
      write(stableFrame(targetValues));
      return;
    }

    const previousTarget = targetValues;
    targetValues = nextValues;

    const rebaseCurrentSlide = isSlidingFrameFor(renderedFrame, previousTarget);
    const sourceValues = rebaseCurrentSlide
      ? [...renderedFrame.values, nextValues.at(-1) ?? 0]
      : [...previousTarget, nextValues.at(-1) ?? 0];
    animationStartOffset = rebaseCurrentSlide ? renderedFrame.offset : 0;
    animationTargetOffset = sourceValues.length - targetValues.length;
    animationStartedAt = scheduler.now();
    write({
      values: sourceValues,
      offset: animationStartOffset,
      windowLength: targetValues.length,
    });
    animationFrame = scheduler.request(animate);
  }

  function destroy(): void {
    if (destroyed) return;
    destroyed = true;
    cancelPendingFrame();
  }

  return { update, finish, destroy };
}

export function shouldSnapChartMotion(
  visibilityState: DocumentVisibilityState,
  prefersReducedMotion: boolean,
): boolean {
  return visibilityState !== "visible" || prefersReducedMotion;
}

export function isCompatibleHistoryUpdate(
  current: readonly number[],
  next: readonly number[],
): boolean {
  return (
    current.length > 1 &&
    current.length === next.length &&
    seriesEqual(current.slice(1), next.slice(0, next.length - 1))
  );
}

export function chartFrameData(frame: ChartMotionFrame): [number[], number[]] {
  const values = frame.values.length > 1 ? frame.values : [0, frame.values[0] ?? 0];
  const x = values.map((_, index) => index - frame.offset);
  return [x, values];
}

function stableFrame(values: readonly number[]): ChartMotionFrame {
  return { values: [...values], offset: 0, windowLength: values.length };
}

function cloneFrame(frame: ChartMotionFrame): ChartMotionFrame {
  return { ...frame, values: [...frame.values] };
}

function isSlidingFrameFor(frame: ChartMotionFrame, target: readonly number[]): boolean {
  return (
    frame.windowLength === target.length &&
    frame.values.length > target.length &&
    frame.values.length - frame.offset > target.length
  );
}

function interpolate(start: number, target: number, progress: number): number {
  return start + (target - start) * progress;
}

function seriesEqual(left: readonly number[], right: readonly number[]): boolean {
  return left.length === right.length && left.every((value, index) => value === right[index]);
}
