using BatCave.Core.Domain;
using BatCave.Rendering;
using BatCave.Tests.TestSupport;

namespace BatCave.Tests.Rendering;

public sealed class TelemetryDeltaAccumulatorTests
{
    [Fact]
    public void MergeAndDrain_ExitSupersedesPriorUpsert()
    {
        TelemetryDeltaAccumulator accumulator = new();
        ProcessSample first = Sample(pid: 100, seq: 1, cpuPct: 10);
        ProcessSample second = Sample(pid: 200, seq: 1, cpuPct: 20);
        ProcessSample secondUpdated = second with { Seq = 2, TsMs = 2, CpuPct = 33 };

        _ = accumulator.Enqueue(new ProcessDeltaBatch
        {
            Seq = 1,
            Upserts = [first, second],
            Exits = [],
        });

        _ = accumulator.Enqueue(new ProcessDeltaBatch
        {
            Seq = 2,
            Upserts = [secondUpdated],
            Exits = [first.Identity()],
        });

        bool drained = accumulator.TryDrain(out ProcessDeltaBatch merged, out int queueDepth);

        Assert.True(drained);
        Assert.Equal(2, queueDepth);
        Assert.Equal(2UL, merged.Seq);
        Assert.Single(merged.Upserts);
        Assert.Equal(second.Identity(), merged.Upserts[0].Identity());
        Assert.Equal(33d, merged.Upserts[0].CpuPct);
        Assert.Single(merged.Exits);
        Assert.Equal(first.Identity(), merged.Exits[0]);
        Assert.Equal(0, accumulator.PendingBatchCount);
    }

    [Fact]
    public void MergeAndDrain_LastWriteWinsWithinFrame()
    {
        TelemetryDeltaAccumulator accumulator = new();
        ProcessSample initial = Sample(pid: 300, seq: 1, cpuPct: 1);
        ProcessSample updated = initial with { Seq = 2, TsMs = 2, CpuPct = 77 };

        _ = accumulator.Enqueue(new ProcessDeltaBatch
        {
            Seq = 1,
            Upserts = [initial],
            Exits = [],
        });

        _ = accumulator.Enqueue(new ProcessDeltaBatch
        {
            Seq = 2,
            Upserts = [updated],
            Exits = [],
        });

        bool drained = accumulator.TryDrain(out ProcessDeltaBatch merged, out _);
        bool drainedAgain = accumulator.TryDrain(out ProcessDeltaBatch _, out _);

        Assert.True(drained);
        Assert.False(drainedAgain);
        Assert.Single(merged.Upserts);
        Assert.Equal(77d, merged.Upserts[0].CpuPct);
        Assert.Equal(2UL, merged.Upserts[0].Seq);
        Assert.Empty(merged.Exits);
    }

    private static ProcessSample Sample(uint pid, ulong seq, double cpuPct)
    {
        return TestProcessSamples.Create(
            pid: pid,
            seq: seq,
            tsMs: seq,
            parentPid: 1,
            startTimeMs: 1000 + seq,
            name: $"proc-{pid}",
            cpuPct: cpuPct,
            rssBytes: 2048,
            privateBytes: 1024,
            ioReadBps: 10,
            ioWriteBps: 20,
            otherIoBps: 30,
            threads: 2,
            handles: 4,
            accessState: AccessState.Full);
    }
}
