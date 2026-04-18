using BatCave.Core.Collector;
using BatCave.Core.Domain;

namespace BatCave.Core.Tests.Collector;

public class WindowsProcessCollectorTests
{
    [Fact]
    public void CollectTick_WhenSnapshotAcquisitionFails_RetainsLastSuccessfulRows()
    {
        Queue<IReadOnlyList<ProcessSample>?> snapshots = new(
        [
            [
                new ProcessSample
                {
                    Pid = 444,
                    Seq = 1,
                    TsMs = 1,
                    ParentPid = 1,
                    StartTimeMs = 4_440,
                    Name = "batcave-proc",
                    CpuPct = 12,
                    RssBytes = 2048,
                    PrivateBytes = 1024,
                    IoReadBps = 10,
                    IoWriteBps = 11,
                    OtherIoBps = 12,
                    Threads = 3,
                    Handles = 5,
                    AccessState = AccessState.Full,
                },
            ],
            null,
        ]);

        WindowsProcessCollector collector = new(seq =>
        {
            IReadOnlyList<ProcessSample>? rows = snapshots.Dequeue();
            return rows is null
                ? null
                :
                [
                    .. rows.Select(row => row with
                    {
                        Seq = seq,
                        TsMs = seq,
                    }),
                ];
        });

        IReadOnlyList<ProcessSample> first = collector.CollectTick(1);
        IReadOnlyList<ProcessSample> second = collector.CollectTick(2);

        ProcessSample replayed = Assert.Single(second);
        Assert.Equal(first[0].Identity(), replayed.Identity());
        Assert.Equal(2UL, replayed.Seq);
        Assert.NotNull(collector.TakeWarning());
    }
}
