using System.Threading.Channels;

namespace BatCave.Core.Runtime;

internal sealed class CoalescedLatestWriteQueue<T> : IDisposable
{
    private readonly Func<T, CancellationToken, Task> _saveAsync;
    private readonly object _sync = new();
    private readonly Channel<QueuedWrite> _channel;
    private readonly Task _workerTask;

    private bool _disposed;
    private long _latestEnqueuedSerial;
    private long _lastCompletedSerial;
    private List<FlushWaiter>? _flushWaiters;

    public CoalescedLatestWriteQueue(Func<T, CancellationToken, Task> saveAsync)
    {
        _saveAsync = saveAsync;
        _channel = Channel.CreateBounded<QueuedWrite>(new BoundedChannelOptions(1)
        {
            SingleReader = true,
            SingleWriter = false,
            FullMode = BoundedChannelFullMode.DropOldest,
        });
        _workerTask = Task.Run(ProcessLoopAsync);
    }

    public void Enqueue(T value)
    {
        QueuedWrite queuedWrite;
        lock (_sync)
        {
            if (_disposed)
            {
                return;
            }

            queuedWrite = new QueuedWrite(++_latestEnqueuedSerial, value);
        }

        _ = _channel.Writer.TryWrite(queuedWrite);
    }

    public Task FlushAsync(CancellationToken ct)
    {
        Task pendingTask;
        lock (_sync)
        {
            if (_latestEnqueuedSerial <= _lastCompletedSerial)
            {
                return Task.CompletedTask;
            }

            TaskCompletionSource<bool> flushSignal = new(TaskCreationOptions.RunContinuationsAsynchronously);
            (_flushWaiters ??= []).Add(new FlushWaiter(_latestEnqueuedSerial, flushSignal));
            pendingTask = flushSignal.Task;
        }

        return pendingTask.WaitAsync(ct);
    }

    public void Dispose()
    {
        lock (_sync)
        {
            if (_disposed)
            {
                return;
            }

            _disposed = true;
        }

        _channel.Writer.TryComplete();

        try
        {
            _workerTask.GetAwaiter().GetResult();
        }
        catch
        {
            // best-effort flush during shutdown
        }
    }

    private async Task ProcessLoopAsync()
    {
        try
        {
            await foreach (QueuedWrite queuedWrite in _channel.Reader.ReadAllAsync().ConfigureAwait(false))
            {
                try
                {
                    await _saveAsync(queuedWrite.Value, CancellationToken.None).ConfigureAwait(false);
                }
                catch
                {
                    // keep runtime resilient if local persistence is temporarily unavailable
                }
                finally
                {
                    MarkCompleted(queuedWrite.Serial);
                }
            }
        }
        finally
        {
            CompleteAllFlushWaiters();
        }
    }

    private void MarkCompleted(long completedSerial)
    {
        List<TaskCompletionSource<bool>> readySignals = [];

        lock (_sync)
        {
            if (completedSerial > _lastCompletedSerial)
            {
                _lastCompletedSerial = completedSerial;
            }

            if (_flushWaiters is null || _flushWaiters.Count == 0)
            {
                return;
            }

            for (int index = _flushWaiters.Count - 1; index >= 0; index--)
            {
                FlushWaiter waiter = _flushWaiters[index];
                if (waiter.TargetSerial > _lastCompletedSerial)
                {
                    continue;
                }

                readySignals.Add(waiter.Signal);
                _flushWaiters.RemoveAt(index);
            }
        }

        foreach (TaskCompletionSource<bool> readySignal in readySignals)
        {
            readySignal.TrySetResult(true);
        }
    }

    private void CompleteAllFlushWaiters()
    {
        List<TaskCompletionSource<bool>> pendingSignals = [];

        lock (_sync)
        {
            if (_flushWaiters is null || _flushWaiters.Count == 0)
            {
                return;
            }

            pendingSignals.AddRange(_flushWaiters.Select(waiter => waiter.Signal));
            _flushWaiters.Clear();
        }

        foreach (TaskCompletionSource<bool> pendingSignal in pendingSignals)
        {
            pendingSignal.TrySetResult(true);
        }
    }

    private readonly record struct QueuedWrite(long Serial, T Value);

    private readonly record struct FlushWaiter(long TargetSerial, TaskCompletionSource<bool> Signal);
}
