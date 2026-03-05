namespace BatCave.Core.Runtime;

internal sealed class CoalescedLatestWriteQueue<T> : IDisposable
{
    private readonly Func<T, CancellationToken, Task> _saveAsync;
    private readonly object _sync = new();

    private T _latestPending = default!;
    private bool _hasPending;
    private bool _disposed;
    private Task _drainTask = Task.CompletedTask;
    private TaskCompletionSource<bool>? _idleSignal;

    public CoalescedLatestWriteQueue(Func<T, CancellationToken, Task> saveAsync)
    {
        _saveAsync = saveAsync;
    }

    public void Enqueue(T value)
    {
        lock (_sync)
        {
            if (_disposed)
            {
                return;
            }

            _latestPending = value;
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
            // best-effort flush during shutdown
        }
    }

    private async Task DrainAsync()
    {
        while (true)
        {
            if (!TryDequeuePending(out T pending, out TaskCompletionSource<bool>? idleSignalToComplete))
            {
                CompleteIdleSignal(idleSignalToComplete);
                return;
            }

            try
            {
                await _saveAsync(pending, CancellationToken.None).ConfigureAwait(false);
            }
            catch
            {
                // keep runtime resilient if local persistence is temporarily unavailable
            }
        }
    }

    private TaskCompletionSource<bool> EnsureIdleSignalAndDrainStartedNoLock()
    {
        _idleSignal ??= CreateIdleSignal();
        if (_drainTask.IsCompleted)
        {
            _drainTask = Task.Run(DrainAsync);
        }

        return _idleSignal;
    }

    private bool TryDequeuePending(out T pending, out TaskCompletionSource<bool>? idleSignalToComplete)
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

            pending = default!;
            idleSignalToComplete = _idleSignal;
            _idleSignal = null;
            return false;
        }
    }

    private static void CompleteIdleSignal(TaskCompletionSource<bool>? idleSignalToComplete)
    {
        idleSignalToComplete?.TrySetResult(true);
    }

    private static TaskCompletionSource<bool> CreateIdleSignal()
    {
        return new TaskCompletionSource<bool>(TaskCreationOptions.RunContinuationsAsynchronously);
    }
}
