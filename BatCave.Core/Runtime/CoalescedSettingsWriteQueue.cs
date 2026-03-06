using BatCave.Core.Domain;

namespace BatCave.Core.Runtime;

internal sealed class CoalescedSettingsWriteQueue(Func<UserSettings, CancellationToken, Task> saveSettingsAsync) : IDisposable
{
    private readonly CoalescedLatestWriteQueue<UserSettings> _innerQueue = new(saveSettingsAsync);

    public void Enqueue(UserSettings settings) => _innerQueue.Enqueue(settings);

    public Task FlushAsync(CancellationToken ct) => _innerQueue.FlushAsync(ct);

    public void Dispose() => _innerQueue.Dispose();
}
