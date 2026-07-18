export interface PollScheduler {
  setTimeout(callback: () => void, delayMs: number): number;
  clearTimeout(timeoutId: number): void;
}

interface RuntimePollingOptions {
  initialDelayMs: number;
  intervalMs: () => number;
  poll: () => Promise<void>;
  scheduler: PollScheduler;
}

export function startRuntimePolling(options: RuntimePollingOptions): () => void {
  let timeoutId: number | undefined;
  let disposed = false;

  const loop = async () => {
    await options.poll();
    if (!disposed) {
      timeoutId = options.scheduler.setTimeout(loop, options.intervalMs());
    }
  };

  timeoutId = options.scheduler.setTimeout(loop, options.initialDelayMs);

  return () => {
    disposed = true;
    if (timeoutId !== undefined) {
      options.scheduler.clearTimeout(timeoutId);
    }
  };
}
