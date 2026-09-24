export interface PollScheduler {
  setTimeout(callback: () => void, delayMs: number): number;
  clearTimeout(timeoutId: number): void;
}

export interface RuntimeVisibility {
  isHidden(): boolean;
  subscribe(callback: () => void): () => void;
}

interface RuntimePollingOptions {
  initialDelayMs: number;
  intervalMs: () => number;
  poll: () => Promise<void>;
  scheduler: PollScheduler;
  visibility?: RuntimeVisibility;
}

export function startRuntimePolling(options: RuntimePollingOptions): () => void {
  let timeoutId: number | undefined;
  let pending = false;
  let disposed = false;
  let running = false;

  const clearPending = () => {
    if (timeoutId !== undefined) {
      options.scheduler.clearTimeout(timeoutId);
      timeoutId = undefined;
    }
    pending = false;
  };

  const schedule = (delayMs: number) => {
    if (disposed || running || pending) return;
    if (options.visibility?.isHidden()) return;
    pending = true;
    timeoutId = options.scheduler.setTimeout(loop, delayMs);
  };

  const loop = async () => {
    pending = false;
    if (disposed || running || options.visibility?.isHidden()) return;
    running = true;
    try {
      await options.poll();
    } finally {
      running = false;
    }
    schedule(options.intervalMs());
  };

  const unsubscribe = options.visibility?.subscribe(() => {
    if (options.visibility?.isHidden()) {
      clearPending();
    } else {
      schedule(0);
    }
  });

  schedule(options.initialDelayMs);

  return () => {
    disposed = true;
    clearPending();
    unsubscribe?.();
  };
}

export function documentVisibility(): RuntimeVisibility {
  return {
    isHidden: () => document.visibilityState === "hidden",
    subscribe: (callback) => {
      document.addEventListener("visibilitychange", callback);
      return () => document.removeEventListener("visibilitychange", callback);
    },
  };
}
