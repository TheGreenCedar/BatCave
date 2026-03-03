using BatCave.Core.Domain;

namespace BatCave.Core.Runtime;

internal sealed class CoalescedSettingsWriteQueue : IDisposable
{
    private readonly Func<UserSettings, CancellationToken, Task> _saveSettingsAsync;
    private readonly object _sync = new();

    private UserSettings _latestPending = new();
    private bool _hasPending;
    private bool _disposed;
    private Task _drainTask = Task.CompletedTask;
    private TaskCompletionSource<bool>? _idleSignal;

    public CoalescedSettingsWriteQueue(Func<UserSettings, CancellationToken, Task> saveSettingsAsync)
    {
        _saveSettingsAsync = saveSettingsAsync;
    }

    public void Enqueue(UserSettings settings)
    {
        lock (_sync)
        {
            if (_disposed)
            {
                return;
            }

            _latestPending = settings;
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
            // best effort flush during shutdown
        }
    }

    private async Task DrainAsync()
    {
        while (true)
        {
            if (!TryDequeuePending(out UserSettings? pending, out TaskCompletionSource<bool>? idleSignalToComplete))
            {
                CompleteIdleSignal(idleSignalToComplete);
                return;
            }

            try
            {
                await _saveSettingsAsync(pending!, CancellationToken.None).ConfigureAwait(false);
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

    private bool TryDequeuePending(out UserSettings? pending, out TaskCompletionSource<bool>? idleSignalToComplete)
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

    private static void CompleteIdleSignal(TaskCompletionSource<bool>? idleSignalToComplete)
    {
        idleSignalToComplete?.TrySetResult(true);
    }

    private static TaskCompletionSource<bool> CreateIdleSignal()
    {
        return new TaskCompletionSource<bool>(TaskCreationOptions.RunContinuationsAsynchronously);
    }
}
