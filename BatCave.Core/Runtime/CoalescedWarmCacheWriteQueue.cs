using BatCave.Core.Domain;

namespace BatCave.Core.Runtime;

internal sealed class CoalescedWarmCacheWriteQueue(Func<WarmCache, CancellationToken, Task> saveWarmCacheAsync) : IDisposable
{
    private readonly CoalescedLatestWriteQueue<WarmCache> _innerQueue = new(saveWarmCacheAsync);

    public void Enqueue(WarmCache cache) => _innerQueue.Enqueue(cache);

    public Task FlushAsync(CancellationToken ct) => _innerQueue.FlushAsync(ct);

    public void Dispose() => _innerQueue.Dispose();
}
