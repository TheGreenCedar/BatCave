using BatCave.Core.Domain;
using BatCave.ViewModels;

namespace BatCave.Tests.ViewModels;

public sealed class MetricHistoryBufferTests
{
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
        Assert.Empty(buffer.Net);
    }

    private static ProcessSample Sample(ulong seq, double cpu)
    {
        ulong value = (ulong)(seq * 100);
        return new ProcessSample
        {
            Seq = seq,
            TsMs = seq,
            Pid = 1,
            ParentPid = 0,
            StartTimeMs = 1,
            Name = "sample",
            CpuPct = cpu,
            RssBytes = value,
            PrivateBytes = value / 2,
            IoReadBps = value,
            IoWriteBps = value,
            NetBps = value,
            Threads = 1,
            Handles = 1,
            AccessState = AccessState.Full,
        };
    }
}
