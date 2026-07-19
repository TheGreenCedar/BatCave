export const chartMotionDurationMs = 200;

export interface AnimationFrameScheduler {
  now(): number;
  request(callback: FrameRequestCallback): number;
  cancel(frame: number): void;
}

export interface ChartMotion {
  update(values: readonly number[], options?: { snap?: boolean }): void;
  finish(): void;
  destroy(): void;
}

export function createChartMotion(
  initialValues: readonly number[],
  render: (values: number[]) => void,
  scheduler: AnimationFrameScheduler,
  durationMs = chartMotionDurationMs,
): ChartMotion {
  let renderedValues = [...initialValues];
  let targetValues = [...initialValues];
  let animationStartValues = [...initialValues];
  let animationStartedAt = 0;
  let animationFrame: number | undefined;
  let destroyed = false;

  function cancelPendingFrame(): void {
    if (animationFrame === undefined) return;
    scheduler.cancel(animationFrame);
    animationFrame = undefined;
  }

  function write(values: readonly number[]): void {
    renderedValues = [...values];
    render([...renderedValues]);
  }

  function finish(): void {
    if (destroyed) return;
    cancelPendingFrame();
    animationStartValues = [...targetValues];
    write(targetValues);
  }

  function animate(timestamp: number): void {
    animationFrame = undefined;
    if (destroyed) return;

    const progress = Math.min(1, Math.max(0, (timestamp - animationStartedAt) / durationMs));
    if (progress === 1) {
      animationStartValues = [...targetValues];
      write(targetValues);
      return;
    }

    write(interpolateSeries(animationStartValues, targetValues, progress));
    if (!destroyed) {
      animationFrame = scheduler.request(animate);
    }
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

    targetValues = nextValues;
    cancelPendingFrame();

    if (!animateUpdate) {
      animationStartValues = [...targetValues];
      write(targetValues);
      return;
    }

    animationStartValues = tailAlignSeries(renderedValues, targetValues.length);
    animationStartedAt = scheduler.now();
    write(animationStartValues);
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
  if (current.length === 0 || next.length === 0 || next.length < current.length) {
    return false;
  }

  if (next.length > current.length) {
    return seriesEqual(current, next.slice(0, current.length));
  }

  return current.length > 1 && seriesEqual(current.slice(1), next.slice(0, next.length - 1));
}

export function tailAlignSeries(values: readonly number[], length: number): number[] {
  if (length <= 0) return [];
  if (values.length >= length) return values.slice(-length);

  const leadingValue = values[0] ?? 0;
  return [...Array.from({ length: length - values.length }, () => leadingValue), ...values];
}

export function interpolateSeries(
  start: readonly number[],
  target: readonly number[],
  progress: number,
): number[] {
  const boundedProgress = Math.min(1, Math.max(0, progress));
  const alignedStart = tailAlignSeries(start, target.length);

  return target.map(
    (value, index) => alignedStart[index] + (value - alignedStart[index]) * boundedProgress,
  );
}

function seriesEqual(left: readonly number[], right: readonly number[]): boolean {
  return left.length === right.length && left.every((value, index) => value === right[index]);
}
