using BatCave.Core.Domain;
using BatCave.Core.State;

namespace BatCave.Core.Tests.State;

public class InMemoryStateStoreTests
{
    [Fact]
    public void ApplyDelta_UpsertsAndExitsAreApplied()
    {
        InMemoryStateStore store = new();
        ProcessSample first = Sample(1, cpu: 1, io: 1);
        ProcessSample second = Sample(2, cpu: 2, io: 2);

        store.ApplyDelta(new ProcessDeltaBatch
        {
            Seq = 1,
            Upserts = [first, second],
            Exits = [],
        });

        Assert.Equal(2, store.RowCount());

        store.ApplyDelta(new ProcessDeltaBatch
        {
            Seq = 2,
            Upserts = [],
            Exits = [first.Identity()],
        });

        IReadOnlyList<ProcessSample> rows = store.AllRows();
        Assert.Single(rows);
        Assert.Equal(2U, rows[0].Pid);
    }

    [Fact]
    public void CompactTo_KeepsHighestActivityRows()
    {
        InMemoryStateStore store = new();
        ProcessSample low = Sample(1, cpu: 0.1, io: 1);
        ProcessSample medium = Sample(2, cpu: 2.0, io: 10);
        ProcessSample high = Sample(3, cpu: 20.0, io: 100);

        store.ApplyDelta(new ProcessDeltaBatch
        {
            Seq = 1,
            Upserts = [low, medium, high],
            Exits = [],
        });

        store.CompactTo(2);
        IReadOnlyList<ProcessSample> rows = store.AllRows();

        Assert.Equal(2, rows.Count);
        Assert.DoesNotContain(rows, row => row.Pid == 1);
    }

    private static ProcessSample Sample(uint pid, double cpu, ulong io)
    {
        return new ProcessSample
        {
            Seq = 1,
            TsMs = 10,
            Pid = pid,
            ParentPid = 1,
            StartTimeMs = pid * 100,
            Name = $"proc-{pid}",
            CpuPct = cpu,
            RssBytes = 1024 * pid,
            PrivateBytes = 512 * pid,
            IoReadBps = io,
            IoWriteBps = io,
            NetBps = io,
            Threads = 1,
            Handles = 1,
            AccessState = AccessState.Full,
        };
    }
}
