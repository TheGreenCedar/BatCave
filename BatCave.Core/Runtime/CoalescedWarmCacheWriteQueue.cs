using BatCave.Core.Domain;

namespace BatCave.Core.Runtime;

internal sealed class CoalescedWarmCacheWriteQueue : IDisposable
{
    private readonly CoalescedLatestWriteQueue<WarmCache> _innerQueue;

    public CoalescedWarmCacheWriteQueue(Func<WarmCache, CancellationToken, Task> saveWarmCacheAsync)
    {
        _innerQueue = new CoalescedLatestWriteQueue<WarmCache>(saveWarmCacheAsync);
    }

    public void Enqueue(WarmCache cache)
    {
        _innerQueue.Enqueue(cache);
    }

    public Task FlushAsync(CancellationToken ct)
    {
        return _innerQueue.FlushAsync(ct);
    }

    public void Dispose()
    {
        _innerQueue.Dispose();
    }
}
