using BatCave.Core.Domain;
using BatCave.Tests.TestSupport;
using BatCave.ViewModels;

namespace BatCave.Tests.ViewModels;

public sealed class MetricHistoryBufferTests
{
    [Fact]
    public void Constructor_StartsSeriesEmpty()
    {
        MetricHistoryBuffer buffer = new(limit: 4);

        Assert.Empty(buffer.Cpu);
        Assert.Empty(buffer.Memory);
        Assert.Empty(buffer.IoRead);
        Assert.Empty(buffer.IoWrite);
        Assert.Empty(buffer.OtherIo);
    }

    [Fact]
    public void Append_WhenExceedingLimit_RetainsNewestValuesInOrder()
    {
        MetricHistoryBuffer buffer = new(limit: 3);
        buffer.Append(Sample(seq: 1, cpu: 1));
        buffer.Append(Sample(seq: 2, cpu: 2));
        buffer.Append(Sample(seq: 3, cpu: 3));
        buffer.Append(Sample(seq: 4, cpu: 4));
        buffer.Append(Sample(seq: 5, cpu: 5));

        Assert.Equal([3d, 4d, 5d], buffer.Cpu.ToArray());
    }

    [Fact]
    public void Reset_ClearsAllSeries()
    {
        MetricHistoryBuffer buffer = new(limit: 4);
        buffer.Append(Sample(seq: 1, cpu: 1));
        buffer.Append(Sample(seq: 2, cpu: 2));

        buffer.Reset();

        Assert.Empty(buffer.Cpu);
        Assert.Empty(buffer.Memory);
        Assert.Empty(buffer.IoRead);
        Assert.Empty(buffer.IoWrite);
        Assert.Empty(buffer.OtherIo);
    }

    private static ProcessSample Sample(ulong seq, double cpu)
    {
        ulong value = (ulong)(seq * 100);
        return TestProcessSamples.Create(
            pid: 1,
            seq: seq,
            tsMs: seq,
            parentPid: 0,
            startTimeMs: 1,
            name: "sample",
            cpuPct: cpu,
            rssBytes: value,
            privateBytes: value / 2,
            ioReadBps: value,
            ioWriteBps: value,
            otherIoBps: value,
            threads: 1,
            handles: 1,
            accessState: AccessState.Full);
    }
}
