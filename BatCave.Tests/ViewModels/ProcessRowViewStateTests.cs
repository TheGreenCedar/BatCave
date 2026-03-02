using BatCave.Core.Domain;
using BatCave.ViewModels;

namespace BatCave.Tests.ViewModels;

public sealed class ProcessRowViewStateTests
{
    [Fact]
    public void UpdateSample_OnlyRaisesChangedDisplayedProperties()
    {
        ProcessSample initial = Sample(cpuPct: 10, rssBytes: 1000, ioReadBps: 200, ioWriteBps: 300, netBps: 400, threads: 5, handles: 6);
        ProcessSample updated = initial with { Seq = 2, TsMs = 2, CpuPct = 25 };
        ProcessRowViewState state = new(initial, "0,0 1,1");

        List<string> changed = [];
        state.PropertyChanged += (_, args) =>
        {
            if (!string.IsNullOrWhiteSpace(args.PropertyName))
            {
                changed.Add(args.PropertyName!);
            }
        };

        state.UpdateSample(updated);

        Assert.Equal(3, changed.Count);
        Assert.Contains(nameof(ProcessRowViewState.CpuPct), changed);
        Assert.Contains(nameof(ProcessRowViewState.CpuSortBucket), changed);
        Assert.Contains(nameof(ProcessRowViewState.CpuText), changed);
    }

    [Fact]
    public void UpdateSample_NoDisplayedChanges_DoesNotRaiseNotifications()
    {
        ProcessSample initial = Sample(cpuPct: 10, rssBytes: 1000, ioReadBps: 200, ioWriteBps: 300, netBps: 400, threads: 5, handles: 6);
        ProcessSample heartbeatOnly = initial with { Seq = 2, TsMs = 2, ParentPid = initial.ParentPid + 1, PrivateBytes = initial.PrivateBytes + 1 };
        ProcessRowViewState state = new(initial, "0,0 1,1");

        int changeCount = 0;
        state.PropertyChanged += (_, args) =>
        {
            if (!string.IsNullOrWhiteSpace(args.PropertyName))
            {
                changeCount++;
            }
        };

        state.UpdateSample(heartbeatOnly);

        Assert.Equal(0, changeCount);
    }

    [Fact]
    public void UpdateSample_UpdatesDisplayTextFieldsForMetricChanges()
    {
        ProcessSample initial = Sample(cpuPct: 10, rssBytes: 1024, ioReadBps: 2048, ioWriteBps: 3072, netBps: 4096, threads: 5, handles: 6);
        ProcessSample updated = initial with
        {
            Seq = 2,
            TsMs = 2,
            CpuPct = 25.5,
            RssBytes = 2048,
            IoReadBps = 4096,
            IoWriteBps = 5120,
            NetBps = 6144,
        };

        ProcessRowViewState state = new(initial, "0,0 1,1");
        state.UpdateSample(updated);

        Assert.Equal("25.50%", state.CpuText);
        Assert.EndsWith("/s", state.IoReadText, StringComparison.Ordinal);
        Assert.EndsWith("/s", state.IoWriteText, StringComparison.Ordinal);
        Assert.EndsWith("/s", state.NetText, StringComparison.Ordinal);
        Assert.Contains("KB", state.RssText, StringComparison.Ordinal);
    }

    private static ProcessSample Sample(
        double cpuPct,
        ulong rssBytes,
        ulong ioReadBps,
        ulong ioWriteBps,
        ulong netBps,
        uint threads,
        uint handles)
    {
        return new ProcessSample
        {
            Seq = 1,
            TsMs = 1,
            Pid = 500,
            ParentPid = 1,
            StartTimeMs = 1234,
            Name = "proc-500",
            CpuPct = cpuPct,
            RssBytes = rssBytes,
            PrivateBytes = 512,
            IoReadBps = ioReadBps,
            IoWriteBps = ioWriteBps,
            NetBps = netBps,
            Threads = threads,
            Handles = handles,
            AccessState = AccessState.Full,
        };
    }
}
