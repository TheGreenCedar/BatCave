using BatCave.Core.Domain;

namespace BatCave.Core.Runtime;

internal sealed class CoalescedSettingsWriteQueue : IDisposable
{
    private readonly CoalescedLatestWriteQueue<UserSettings> _innerQueue;

    public CoalescedSettingsWriteQueue(Func<UserSettings, CancellationToken, Task> saveSettingsAsync)
    {
        _innerQueue = new CoalescedLatestWriteQueue<UserSettings>(saveSettingsAsync);
    }

    public void Enqueue(UserSettings settings)
    {
        _innerQueue.Enqueue(settings);
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
