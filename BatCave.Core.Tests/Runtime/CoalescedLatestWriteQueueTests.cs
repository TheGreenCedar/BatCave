using BatCave.Core.Runtime;

namespace BatCave.Core.Tests.Runtime;

public class CoalescedLatestWriteQueueTests
{
    [Fact]
    public async Task FlushAsync_WhenMultipleValuesQueued_PersistsLatestPendingValue()
    {
        List<int> persistedValues = [];
        TaskCompletionSource<bool> allowFirstSave = new(TaskCreationOptions.RunContinuationsAsynchronously);
        int saveCalls = 0;

        using CoalescedLatestWriteQueue<int> queue = new(async (value, ct) =>
        {
            int call = Interlocked.Increment(ref saveCalls);
            if (call == 1)
            {
                await allowFirstSave.Task.WaitAsync(ct);
            }

            lock (persistedValues)
            {
                persistedValues.Add(value);
            }
        });

        queue.Enqueue(1);
        Assert.True(
            SpinWait.SpinUntil(() => Volatile.Read(ref saveCalls) == 1, millisecondsTimeout: 2_000),
            "The first save did not start within the expected window.");
        queue.Enqueue(2);
        queue.Enqueue(3);

        allowFirstSave.TrySetResult(true);
        await queue.FlushAsync(CancellationToken.None);

        Assert.Equal([1, 3], persistedValues);
    }

    [Fact]
    public void Dispose_WhenValuePending_PerformsBestEffortFlush()
    {
        List<int> persistedValues = [];

        using (CoalescedLatestWriteQueue<int> queue = new((value, _) =>
               {
                   lock (persistedValues)
                   {
                       persistedValues.Add(value);
                   }

                   return Task.CompletedTask;
               }))
        {
            queue.Enqueue(7);
        }

        Assert.Equal([7], persistedValues);
    }

    [Fact]
    public async Task FlushAsync_WhenPersistenceFails_Throws()
    {
        using CoalescedLatestWriteQueue<int> queue = new((_, _) =>
            Task.FromException(new InvalidOperationException("disk unavailable")));

        queue.Enqueue(7);

        InvalidOperationException exception = await Assert.ThrowsAsync<InvalidOperationException>(
            () => queue.FlushAsync(CancellationToken.None));
        Assert.True(
            exception.Message.Contains("durably save", StringComparison.OrdinalIgnoreCase)
            || exception.Message.Contains("disk unavailable", StringComparison.OrdinalIgnoreCase));
    }

    [Fact]
    public async Task FlushAsync_WhenNewerValuePersistsAfterFailure_CompletesSuccessfully()
    {
        int saveCalls = 0;
        using CoalescedLatestWriteQueue<int> queue = new((_, _) =>
        {
            int call = Interlocked.Increment(ref saveCalls);
            return call == 1
                ? Task.FromException(new InvalidOperationException("disk unavailable"))
                : Task.CompletedTask;
        });

        queue.Enqueue(7);
        await Assert.ThrowsAsync<InvalidOperationException>(() => queue.FlushAsync(CancellationToken.None));

        queue.Enqueue(8);
        await queue.FlushAsync(CancellationToken.None);
    }
}
