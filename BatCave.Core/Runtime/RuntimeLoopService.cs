using BatCave.Core.Domain;

namespace BatCave.Core.Runtime;

public sealed class TickFaultedEventArgs : EventArgs
{
    public long Generation { get; init; }

    public int ConsecutiveFaults { get; init; }

    public int DelayMs { get; init; }

    public string ExceptionType { get; init; } = string.Empty;

    public string Message { get; init; } = string.Empty;

    public ulong TsMs { get; init; }
}

public sealed class RuntimeLoopService
{
    private const int BaseBackoffMs = 250;
    private const int MaxBackoffMs = 8_000;

    private readonly MonitoringRuntime _runtime;
    private readonly TimeProvider _timeProvider;
    private readonly TimeSpan _interval;
    private readonly object _sync = new();

    private CancellationTokenSource? _cts;
    private Task? _loopTask;
    private long _generation = 1;

    public RuntimeLoopService(
        MonitoringRuntime runtime,
        TimeProvider? timeProvider = null,
        TimeSpan? intervalOverride = null)
    {
        _runtime = runtime;
        _timeProvider = timeProvider ?? TimeProvider.System;
        _interval = intervalOverride ?? TimeSpan.FromSeconds(1);
    }

    public event EventHandler<TickOutcome>? TickCompleted;
    public event EventHandler<TickFaultedEventArgs>? TickFaulted;

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
        DateTimeOffset nextTick = _timeProvider.GetUtcNow().Add(_interval);
        using PeriodicTimer timer = new(_interval, _timeProvider);
        int consecutiveFaults = 0;

        while (!ct.IsCancellationRequested && await timer.WaitForNextTickAsync(ct).ConfigureAwait(false))
        {
            if (Interlocked.Read(ref _generation) != generation)
            {
                break;
            }

            DateTimeOffset tickStart = _timeProvider.GetUtcNow();
            double jitterMs = Math.Abs((tickStart - nextTick).TotalMilliseconds);

            try
            {
                TickOutcome outcome = _runtime.Tick(jitterMs);
                TickCompleted?.Invoke(this, outcome);
                consecutiveFaults = 0;

                nextTick = nextTick.Add(_interval);
                DateTimeOffset loopEnd = _timeProvider.GetUtcNow();

                if (loopEnd > nextTick + _interval)
                {
                    long lagMs = (long)(loopEnd - nextTick).TotalMilliseconds;
                    ulong dropped = lagMs > 0 ? (ulong)(lagMs / 1000) : 0;
                    if (dropped > 0)
                    {
                        _runtime.RecordDroppedTicks(dropped);
                    }

                    nextTick = loopEnd.Add(_interval);
                }
            }
            catch (OperationCanceledException) when (ct.IsCancellationRequested)
            {
                break;
            }
            catch (Exception ex)
            {
                consecutiveFaults++;
                int delayMs = ResolveBackoffDelayMs(consecutiveFaults);
                TickFaulted?.Invoke(this, new TickFaultedEventArgs
                {
                    Generation = generation,
                    ConsecutiveFaults = consecutiveFaults,
                    DelayMs = delayMs,
                    ExceptionType = ex.GetType().FullName ?? ex.GetType().Name,
                    Message = ex.Message,
                    TsMs = (ulong)_timeProvider.GetUtcNow().ToUnixTimeMilliseconds(),
                });

                try
                {
                    await Task.Delay(TimeSpan.FromMilliseconds(delayMs), ct).ConfigureAwait(false);
                }
                catch (OperationCanceledException) when (ct.IsCancellationRequested)
                {
                    break;
                }

                nextTick = _timeProvider.GetUtcNow().Add(_interval);
            }
        }
    }

    private static int ResolveBackoffDelayMs(int consecutiveFaults)
    {
        int exponent = Math.Max(0, consecutiveFaults - 1);
        double delay = BaseBackoffMs * Math.Pow(2, exponent);
        return (int)Math.Min(MaxBackoffMs, delay);
    }
}
