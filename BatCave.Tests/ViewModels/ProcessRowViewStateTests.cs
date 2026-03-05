using BatCave.Core.Domain;
using BatCave.Tests.TestSupport;
using BatCave.ViewModels;
using Windows.Foundation;

namespace BatCave.Tests.ViewModels;

public sealed class ProcessRowViewStateTests
{
    [Fact]
    public void UpdateSample_OnlyRaisesChangedDisplayedProperties()
    {
        ProcessSample initial = Sample(cpuPct: 10, rssBytes: 1000, ioReadBps: 200, ioWriteBps: 300, netBps: 400, threads: 5, handles: 6);
        ProcessSample updated = initial with { Seq = 2, TsMs = 2, CpuPct = 25 };
        ProcessRowViewState state = new(initial, CreateTrendGeometry());

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
        ProcessRowViewState state = new(initial, CreateTrendGeometry());

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
            OtherIoBps = 6144,
        };

        ProcessRowViewState state = new(initial, CreateTrendGeometry());
        state.UpdateSample(updated);

        Assert.Equal("25.50%", state.CpuText);
        Assert.EndsWith("/s", state.IoReadText, StringComparison.Ordinal);
        Assert.EndsWith("/s", state.IoWriteText, StringComparison.Ordinal);
        Assert.EndsWith("/s", state.OtherIoText, StringComparison.Ordinal);
        Assert.Equal("9.0 KB/s", state.DiskText);
        Assert.Equal("49.2 Kbps", state.NetworkText);
        Assert.Contains("KB", state.RssText, StringComparison.Ordinal);
    }

    [Fact]
    public void UpdateCpuTrendValues_TargetLengthMatches_ReplacesValuesInstanceWhenChanged()
    {
        ProcessRowViewState state = new(
            Sample(cpuPct: 10, rssBytes: 1000, ioReadBps: 200, ioWriteBps: 300, netBps: 400, threads: 5, handles: 6),
            CreateTrendGeometry(),
            cpuTrendValues: [1d, 2d, 3d]);

        IReadOnlyList<double> originalReference = state.CpuTrendValues;

        state.UpdateCpuTrendValues([4d, 5d, 6d], visiblePointCount: 3);

        Assert.NotSame(originalReference, state.CpuTrendValues);
        AssertDoubleValues([4d, 5d, 6d], state.CpuTrendValues);
    }

    [Fact]
    public void UpdateCpuTrendValues_PropertyChangedForCpuTrendValues_FiresOnlyWhenValuesChanged()
    {
        ProcessRowViewState state = new(
            Sample(cpuPct: 10, rssBytes: 1000, ioReadBps: 200, ioWriteBps: 300, netBps: 400, threads: 5, handles: 6),
            CreateTrendGeometry(),
            cpuTrendValues: [1d, 2d, 3d]);

        int cpuTrendChangeCount = 0;
        state.PropertyChanged += (_, args) =>
        {
            if (args.PropertyName == nameof(ProcessRowViewState.CpuTrendValues))
            {
                cpuTrendChangeCount++;
            }
        };

        state.UpdateCpuTrendValues([1d, 2d, 3d], visiblePointCount: 3);
        state.UpdateCpuTrendValues([1d, 2d, 4d], visiblePointCount: 3);
        state.UpdateCpuTrendValues([1d, 2d, 4d], visiblePointCount: 3);

        Assert.Equal(1, cpuTrendChangeCount);
        AssertDoubleValues([1d, 2d, 4d], state.CpuTrendValues);
    }

    [Fact]
    public void UpdateCpuTrendValues_ForceRenderTick_RaisesCpuTrendValuesEvenWhenSequenceMatches()
    {
        ProcessRowViewState state = new(
            Sample(cpuPct: 10, rssBytes: 1000, ioReadBps: 200, ioWriteBps: 300, netBps: 400, threads: 5, handles: 6),
            CreateTrendGeometry(),
            cpuTrendValues: [1d, 2d, 3d]);

        int cpuTrendChangeCount = 0;
        state.PropertyChanged += (_, args) =>
        {
            if (args.PropertyName == nameof(ProcessRowViewState.CpuTrendValues))
            {
                cpuTrendChangeCount++;
            }
        };

        state.UpdateCpuTrendValues([1d, 2d, 3d], visiblePointCount: 3, forceRenderTick: true);

        Assert.Equal(1, cpuTrendChangeCount);
        AssertDoubleValues([1d, 2d, 3d], state.CpuTrendValues);
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
        return TestProcessSamples.Create(
            pid: 500,
            seq: 1,
            tsMs: 1,
            parentPid: 1,
            startTimeMs: 1234,
            name: "proc-500",
            cpuPct: cpuPct,
            rssBytes: rssBytes,
            privateBytes: 512,
            ioReadBps: ioReadBps,
            ioWriteBps: ioWriteBps,
            otherIoBps: netBps,
            threads: threads,
            handles: handles,
            accessState: AccessState.Full);
    }

    private static IReadOnlyList<Point> CreateTrendGeometry()
    {
        return new Point[]
        {
            new Point(0, 0),
            new Point(1, 1),
        };
    }

    private static void AssertDoubleValues(double[] expected, IReadOnlyList<double> actual)
    {
        Assert.Equal(expected.Length, actual.Count);
        for (int index = 0; index < expected.Length; index++)
        {
            Assert.Equal(expected[index], actual[index]);
        }
    }
}

