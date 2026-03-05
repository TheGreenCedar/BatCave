using BatCave.Core.Domain;
using BatCave.Core.Sort;

namespace BatCave.Core.Tests.Sort;

public class IncrementalSortIndexEngineTests
{
    [Fact]
    public void DeltaUpdates_ReorderRowsForActiveSort()
    {
        IncrementalSortIndexEngine engine = new();
        QueryRequest request = new()
        {
            Offset = 0,
            Limit = 10,
            SortCol = SortColumn.CpuPct,
            SortDir = SortDirection.Desc,
            FilterText = string.Empty,
        };

        ProcessSample alpha = Sample(pid: 10, "alpha", cpu: 20, rss: 10);
        ProcessSample beta = Sample(pid: 20, "beta", cpu: 50, rss: 20);

        engine.OnDelta(new ProcessDeltaBatch
        {
            Seq = 1,
            Upserts = [alpha, beta],
            Exits = [],
        });

        QueryResponse baseline = engine.Query(request, [alpha, beta], 1);
        Assert.Equal(new uint[] { 20, 10 }, baseline.Rows.Select(row => row.Pid).ToArray());

        ProcessSample alphaHot = alpha with { CpuPct = 70 };
        engine.OnDelta(new ProcessDeltaBatch
        {
            Seq = 2,
            Upserts = [alphaHot],
            Exits = [],
        });

        QueryResponse reordered = engine.Query(request, [alphaHot, beta], 2);
        Assert.Equal(new uint[] { 10, 20 }, reordered.Rows.Select(row => row.Pid).ToArray());
    }

    [Fact]
    public void Exits_RemoveRowsFromResults()
    {
        IncrementalSortIndexEngine engine = new();
        QueryRequest request = new()
        {
            SortCol = SortColumn.Pid,
            SortDir = SortDirection.Asc,
            Limit = 10,
        };

        ProcessSample a = Sample(pid: 1, "a", cpu: 1, rss: 1);
        ProcessSample b = Sample(pid: 2, "b", cpu: 1, rss: 1);
        ProcessSample c = Sample(pid: 3, "c", cpu: 1, rss: 1);

        engine.OnDelta(new ProcessDeltaBatch
        {
            Seq = 1,
            Upserts = [a, b, c],
            Exits = [],
        });
        engine.Query(request, [a, b, c], 1);

        engine.OnDelta(new ProcessDeltaBatch
        {
            Seq = 2,
            Upserts = [],
            Exits = [b.Identity()],
        });

        QueryResponse response = engine.Query(request, [a, c], 2);
        Assert.Equal(2, response.Total);
        Assert.DoesNotContain(response.Rows, row => row.Pid == 2);
    }

    [Fact]
    public void FilterAndPaging_ReturnExpectedTotalsAndRows()
    {
        IncrementalSortIndexEngine engine = new();
        ProcessSample a = Sample(pid: 10, "chrome", cpu: 3, rss: 1);
        ProcessSample b = Sample(pid: 11, "code", cpu: 2, rss: 1);
        ProcessSample c = Sample(pid: 12, "chrome helper", cpu: 1, rss: 1);

        engine.OnDelta(new ProcessDeltaBatch
        {
            Seq = 1,
            Upserts = [a, b, c],
            Exits = [],
        });

        QueryRequest request = new()
        {
            Offset = 1,
            Limit = 1,
            SortCol = SortColumn.Name,
            SortDir = SortDirection.Asc,
            FilterText = "chrome",
        };

        QueryResponse response = engine.Query(request, [a, b, c], 1);
        Assert.Equal(2, response.Total);
        Assert.Single(response.Rows);
        Assert.Equal(12U, response.Rows[0].Pid);
    }

    [Fact]
    public void DiskBpsSort_OrdersByReadPlusWrite()
    {
        IncrementalSortIndexEngine engine = new();
        QueryRequest request = new()
        {
            SortCol = SortColumn.DiskBps,
            SortDir = SortDirection.Desc,
            Limit = 10,
        };

        ProcessSample low = Sample(pid: 1, "low", cpu: 1, rss: 1) with { IoReadBps = 100, IoWriteBps = 10 };
        ProcessSample mid = Sample(pid: 2, "mid", cpu: 1, rss: 1) with { IoReadBps = 50, IoWriteBps = 200 };
        ProcessSample high = Sample(pid: 3, "high", cpu: 1, rss: 1) with { IoReadBps = 300, IoWriteBps = 0 };

        engine.OnDelta(new ProcessDeltaBatch
        {
            Seq = 1,
            Upserts = [low, mid, high],
            Exits = [],
        });

        QueryResponse response = engine.Query(request, [low, mid, high], 1);
        Assert.Equal(new uint[] { 3, 2, 1 }, response.Rows.Select(row => row.Pid).ToArray());
    }

    [Fact]
    public void DiskBpsSort_SaturatesOnOverflow()
    {
        IncrementalSortIndexEngine engine = new();
        QueryRequest request = new()
        {
            SortCol = SortColumn.DiskBps,
            SortDir = SortDirection.Desc,
            Limit = 10,
        };

        ProcessSample overflow = Sample(pid: 10, "overflow", cpu: 1, rss: 1) with { IoReadBps = ulong.MaxValue, IoWriteBps = 1 };
        ProcessSample nearMax = Sample(pid: 20, "nearMax", cpu: 1, rss: 1) with { IoReadBps = ulong.MaxValue - 2, IoWriteBps = 1 };

        engine.OnDelta(new ProcessDeltaBatch
        {
            Seq = 1,
            Upserts = [overflow, nearMax],
            Exits = [],
        });

        QueryResponse response = engine.Query(request, [overflow, nearMax], 1);
        Assert.Equal(new uint[] { 10, 20 }, response.Rows.Select(row => row.Pid).ToArray());
    }

    private static ProcessSample Sample(uint pid, string name, double cpu, ulong rss)
    {
        return new ProcessSample
        {
            Seq = 1,
            TsMs = 10,
            Pid = pid,
            ParentPid = 1,
            StartTimeMs = pid * 100,
            Name = name,
            CpuPct = cpu,
            RssBytes = rss,
            PrivateBytes = rss / 2,
            IoReadBps = 5,
            IoWriteBps = 6,
            OtherIoBps = 7,
            Threads = 3,
            Handles = 4,
            AccessState = AccessState.Full,
        };
    }
}
