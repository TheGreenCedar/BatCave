using BatCave.Core.Domain;

namespace BatCave.Core.Runtime;

internal sealed class CoalescedWarmCacheWriteQueue : IDisposable
{
    private readonly Func<WarmCache, CancellationToken, Task> _saveWarmCacheAsync;
    private readonly object _sync = new();

    private WarmCache? _latestPending;
    private bool _hasPending;
    private bool _disposed;
    private Task _drainTask = Task.CompletedTask;
    private TaskCompletionSource<bool>? _idleSignal;

    public CoalescedWarmCacheWriteQueue(Func<WarmCache, CancellationToken, Task> saveWarmCacheAsync)
    {
        _saveWarmCacheAsync = saveWarmCacheAsync;
    }

    public void Enqueue(WarmCache cache)
    {
        lock (_sync)
        {
            if (_disposed)
            {
                return;
            }

            _latestPending = cache;
            _hasPending = true;
            _ = EnsureIdleSignalAndDrainStartedNoLock();
        }
    }

    public Task FlushAsync(CancellationToken ct)
    {
        Task idleTask;
        lock (_sync)
        {
            if (!_hasPending && _drainTask.IsCompleted)
            {
                return Task.CompletedTask;
            }

            idleTask = EnsureIdleSignalAndDrainStartedNoLock().Task;
        }

        return idleTask.WaitAsync(ct);
    }

    public void Dispose()
    {
        Task flushTask = Task.CompletedTask;

        lock (_sync)
        {
            if (_disposed)
            {
                return;
            }

            _disposed = true;
            if (_hasPending || !_drainTask.IsCompleted)
            {
                flushTask = EnsureIdleSignalAndDrainStartedNoLock().Task;
            }
        }

        try
        {
            flushTask.GetAwaiter().GetResult();
        }
        catch
        {
            // best-effort flush on shutdown
        }
    }

    private async Task DrainAsync()
    {
        while (true)
        {
            if (!TryDequeuePending(out WarmCache? pending, out TaskCompletionSource<bool>? idleSignalToComplete))
            {
                idleSignalToComplete?.TrySetResult(true);
                return;
            }

            try
            {
                await _saveWarmCacheAsync(pending!, CancellationToken.None).ConfigureAwait(false);
            }
            catch
            {
                // keep runtime resilient if local persistence is temporarily unavailable
            }
        }
    }

    private TaskCompletionSource<bool> EnsureIdleSignalAndDrainStartedNoLock()
    {
        _idleSignal ??= new TaskCompletionSource<bool>(TaskCreationOptions.RunContinuationsAsynchronously);
        if (_drainTask.IsCompleted)
        {
            _drainTask = Task.Run(DrainAsync);
        }

        return _idleSignal;
    }

    private bool TryDequeuePending(out WarmCache? pending, out TaskCompletionSource<bool>? idleSignalToComplete)
    {
        lock (_sync)
        {
            if (_hasPending)
            {
                pending = _latestPending;
                _hasPending = false;
                idleSignalToComplete = null;
                return true;
            }

            pending = null;
            idleSignalToComplete = _idleSignal;
            _idleSignal = null;
            return false;
        }
    }
}
