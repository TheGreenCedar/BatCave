using BatCave.Core.Domain;

namespace BatCave.Core.Runtime;

public sealed class RuntimeLoopService
{
    private readonly MonitoringRuntime _runtime;
    private readonly TimeProvider _timeProvider;
    private readonly object _sync = new();

    private CancellationTokenSource? _cts;
    private Task? _loopTask;
    private long _generation = 1;

    public RuntimeLoopService(MonitoringRuntime runtime, TimeProvider? timeProvider = null)
    {
        _runtime = runtime;
        _timeProvider = timeProvider ?? TimeProvider.System;
    }

    public event EventHandler<TickOutcome>? TickCompleted;

    public long CurrentGeneration => Interlocked.Read(ref _generation);

    public void Start(long generation)
    {
        lock (_sync)
        {
            if (_loopTask is { IsCompleted: false })
            {
                return;
            }

            _cts?.Dispose();
            _cts = new CancellationTokenSource();
            _loopTask = RunLoopAsync(generation, _cts.Token);
        }
    }

    public void StopAndAdvanceGeneration()
    {
        lock (_sync)
        {
            Interlocked.Increment(ref _generation);
            _cts?.Cancel();
        }
    }

    private async Task RunLoopAsync(long generation, CancellationToken ct)
    {
        TimeSpan interval = TimeSpan.FromSeconds(1);
        DateTimeOffset nextTick = _timeProvider.GetUtcNow().Add(interval);
        using PeriodicTimer timer = new(interval, _timeProvider);

        while (!ct.IsCancellationRequested && await timer.WaitForNextTickAsync(ct).ConfigureAwait(false))
        {
            if (Interlocked.Read(ref _generation) != generation)
            {
                break;
            }

            DateTimeOffset tickStart = _timeProvider.GetUtcNow();
            double jitterMs = Math.Abs((tickStart - nextTick).TotalMilliseconds);

            TickOutcome outcome = _runtime.Tick(jitterMs);
            TickCompleted?.Invoke(this, outcome);

            nextTick = nextTick.Add(interval);
            DateTimeOffset loopEnd = _timeProvider.GetUtcNow();

            if (loopEnd > nextTick + interval)
            {
                long lagMs = (long)(loopEnd - nextTick).TotalMilliseconds;
                ulong dropped = lagMs > 0 ? (ulong)(lagMs / 1000) : 0;
                if (dropped > 0)
                {
                    _runtime.RecordDroppedTicks(dropped);
                }

                nextTick = loopEnd.Add(interval);
            }
        }
    }
}
