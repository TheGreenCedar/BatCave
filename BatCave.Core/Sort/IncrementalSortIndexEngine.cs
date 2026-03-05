using BatCave.Core.Abstractions;
using BatCave.Core.Domain;

namespace BatCave.Core.Sort;

public sealed class IncrementalSortIndexEngine : ISortIndexEngine
{
    private readonly SortCache _cache = new();

    public void OnDelta(ProcessDeltaBatch delta)
    {
        _cache.LatestSeq = delta.Seq;

        foreach (ProcessIdentity exit in delta.Exits)
        {
            _cache.Rows.Remove(exit);
            _cache.PendingExits.Add(exit);
        }

        foreach (ProcessSample upsert in delta.Upserts)
        {
            ProcessIdentity identity = upsert.Identity();
            _cache.Rows[identity] = upsert;
            _cache.PendingUpserts.Add(identity);
        }
    }

    public QueryResponse Query(QueryRequest request, IReadOnlyList<ProcessSample> rows, ulong seq)
    {
        List<ProcessIdentity> ordered = ResolveOrderedRows(request, rows);

        int total = ordered.Count;
        (int start, int count) = SlicePage(request.Offset, request.Limit, total);
        List<ProcessSample> page = BuildPage(ordered, start, count);

        return new QueryResponse
        {
            Seq = Math.Max(seq, _cache.LatestSeq),
            Total = total,
            Rows = page,
        };
    }

    private List<ProcessIdentity> ResolveOrderedRows(QueryRequest request, IReadOnlyList<ProcessSample> rows)
    {
        ReconcileCacheRowsAndOrdering(request, rows);
        return BuildFilteredRows(request.FilterText);
    }

    private void ReconcileCacheRowsAndOrdering(QueryRequest request, IReadOnlyList<ProcessSample> rows)
    {
        if (ShouldResetCacheRows(rows))
        {
            ResetCacheRows(rows);
        }

        EnsureOrdering(request);
    }

    private List<ProcessIdentity> BuildFilteredRows(string filterText)
    {
        return ApplyFilter(filterText);
    }

    private bool ShouldResetCacheRows(IReadOnlyList<ProcessSample> rows)
    {
        return rows.Count > 0 && rows.Count != _cache.Rows.Count;
    }

    private void ResetCacheRows(IReadOnlyList<ProcessSample> rows)
    {
        _cache.Rows = rows.ToDictionary(row => row.Identity(), row => row);
        ClearPendingChanges();
        _cache.Initialized = false;
    }

    private void EnsureOrdering(QueryRequest request)
    {
        if (IsSortDefinitionChanged(request))
        {
            _cache.SortCol = request.SortCol;
            _cache.SortDir = request.SortDir;
            RebuildOrdering();
            ClearPendingChanges();
            return;
        }

        ApplyIncrementalUpdates();
    }

    private bool IsSortDefinitionChanged(QueryRequest request)
    {
        return !_cache.Initialized || _cache.SortCol != request.SortCol || _cache.SortDir != request.SortDir;
    }

    private List<ProcessIdentity> ApplyFilter(string filterText)
    {
        string filterNeedle = filterText.Trim().ToLowerInvariant();
        if (string.IsNullOrWhiteSpace(filterNeedle))
        {
            return _cache.Ordered;
        }

        _cache.Filtered.Clear();
        foreach (ProcessIdentity identity in _cache.Ordered)
        {
            if (MatchesFilter(_cache.Rows[identity], filterNeedle))
            {
                _cache.Filtered.Add(identity);
            }
        }

        return _cache.Filtered;
    }

    private List<ProcessSample> BuildPage(List<ProcessIdentity> ordered, int start, int count)
    {
        List<ProcessSample> page = new(count);
        int end = start + count;
        for (int index = start; index < end; index++)
        {
            page.Add(_cache.Rows[ordered[index]]);
        }

        return page;
    }

    private void RebuildOrdering()
    {
        _cache.Ordered.Clear();
        foreach (ProcessIdentity identity in _cache.Rows.Keys)
        {
            _cache.Ordered.Add(identity);
        }

        _cache.Ordered.Sort((left, right) => CompareIdentity(left, right, _cache.Rows, _cache.SortCol, _cache.SortDir));
        _cache.Initialized = true;
    }

    private void ApplyIncrementalUpdates()
    {
        if (!HasPendingChanges())
        {
            return;
        }

        if (!_cache.Initialized)
        {
            RebuildOrdering();
            ClearPendingChanges();
            return;
        }

        CopyPendingExitsToScratch();

        List<ProcessIdentity> pendingUpserts = CollectPendingUpserts();
        ClearPendingChanges();

        if (ShouldRebuildForChangeCount(_cache.ExitsScratch.Count + pendingUpserts.Count))
        {
            RebuildOrdering();
            return;
        }

        RemoveChangedIdentitiesFromOrdering(pendingUpserts);
        InsertPendingUpserts(pendingUpserts);
    }

    private bool HasPendingChanges()
    {
        return _cache.PendingUpserts.Count > 0 || _cache.PendingExits.Count > 0;
    }

    private void CopyPendingExitsToScratch()
    {
        _cache.ExitsScratch.Clear();
        foreach (ProcessIdentity identity in _cache.PendingExits)
        {
            _cache.ExitsScratch.Add(identity);
        }
    }

    private bool ShouldRebuildForChangeCount(int changeCount)
    {
        int rebuildThreshold = Math.Max(_cache.Rows.Count / 4, 96);
        return changeCount >= rebuildThreshold;
    }

    private void RemoveChangedIdentitiesFromOrdering(List<ProcessIdentity> pendingUpserts)
    {
        if (_cache.ExitsScratch.Count == 0 && pendingUpserts.Count == 0)
        {
            return;
        }

        HashSet<ProcessIdentity> removed = _cache.ExitsScratch;
        removed.UnionWith(pendingUpserts);

        _cache.RemainingScratch.Clear();
        foreach (ProcessIdentity identity in _cache.Ordered)
        {
            if (!removed.Contains(identity))
            {
                _cache.RemainingScratch.Add(identity);
            }
        }

        _cache.Ordered.Clear();
        _cache.Ordered.AddRange(_cache.RemainingScratch);
    }

    private void InsertPendingUpserts(IReadOnlyList<ProcessIdentity> pendingUpserts)
    {
        foreach (ProcessIdentity identity in pendingUpserts)
        {
            int insertAt = InsertionIndex(identity);
            _cache.Ordered.Insert(insertAt, identity);
        }
    }

    private int InsertionIndex(ProcessIdentity target)
    {
        int low = 0;
        int high = _cache.Ordered.Count;

        while (low < high)
        {
            int mid = low + (high - low) / 2;
            ProcessIdentity existing = _cache.Ordered[mid];
            int ordering = CompareIdentity(existing, target, _cache.Rows, _cache.SortCol, _cache.SortDir);
            if (ordering < 0)
            {
                low = mid + 1;
            }
            else
            {
                high = mid;
            }
        }

        return low;
    }

    private List<ProcessIdentity> CollectPendingUpserts()
    {
        _cache.UpsertsSeenScratch.Clear();
        _cache.PendingUpsertsScratch.Clear();
        foreach (ProcessIdentity identity in _cache.PendingUpserts)
        {
            if (_cache.UpsertsSeenScratch.Add(identity) && _cache.Rows.ContainsKey(identity))
            {
                _cache.PendingUpsertsScratch.Add(identity);
            }
        }

        return _cache.PendingUpsertsScratch;
    }

    private void ClearPendingChanges()
    {
        _cache.PendingUpserts.Clear();
        _cache.PendingExits.Clear();
    }

    private static bool MatchesFilter(ProcessSample row, string filterNeedle)
    {
        return row.Name.Contains(filterNeedle, StringComparison.OrdinalIgnoreCase)
               || row.Pid.ToString().Contains(filterNeedle, StringComparison.OrdinalIgnoreCase);
    }

    private static (int Start, int Count) SlicePage(int offset, int limit, int total)
    {
        int start = Math.Min(Math.Max(0, offset), total);
        int end = Math.Min(total, start + Math.Max(0, limit));
        return (start, end - start);
    }

    private static int CompareIdentity(
        ProcessIdentity left,
        ProcessIdentity right,
        Dictionary<ProcessIdentity, ProcessSample> rows,
        SortColumn sortCol,
        SortDirection sortDir)
    {
        bool hasLeft = rows.TryGetValue(left, out ProcessSample? leftRow);
        bool hasRight = rows.TryGetValue(right, out ProcessSample? rightRow);

        int ordering = (hasLeft, hasRight) switch
        {
            (true, true) => CompareRows(leftRow!, rightRow!, sortCol, sortDir),
            (false, true) => 1,
            (true, false) => -1,
            _ => 0,
        };

        if (ordering != 0)
        {
            return ordering;
        }

        int pidOrder = left.Pid.CompareTo(right.Pid);
        if (pidOrder != 0)
        {
            return pidOrder;
        }

        return left.StartTimeMs.CompareTo(right.StartTimeMs);
    }

    private static int CompareRows(ProcessSample left, ProcessSample right, SortColumn sortCol, SortDirection sortDir)
    {
        int ordering = sortCol switch
        {
            SortColumn.Pid => left.Pid.CompareTo(right.Pid),
            SortColumn.Name => string.Compare(left.Name, right.Name, StringComparison.Ordinal),
            SortColumn.CpuPct => left.CpuPct.CompareTo(right.CpuPct),
            SortColumn.RssBytes => left.RssBytes.CompareTo(right.RssBytes),
            SortColumn.IoReadBps => left.IoReadBps.CompareTo(right.IoReadBps),
            SortColumn.IoWriteBps => left.IoWriteBps.CompareTo(right.IoWriteBps),
            SortColumn.OtherIoBps => left.OtherIoBps.CompareTo(right.OtherIoBps),
            SortColumn.DiskBps => SaturatingSum(left.IoReadBps, left.IoWriteBps).CompareTo(SaturatingSum(right.IoReadBps, right.IoWriteBps)),
            SortColumn.Threads => left.Threads.CompareTo(right.Threads),
            SortColumn.Handles => left.Handles.CompareTo(right.Handles),
            SortColumn.StartTimeMs => left.StartTimeMs.CompareTo(right.StartTimeMs),
            _ => 0,
        };

        return sortDir == SortDirection.Asc ? ordering : -ordering;
    }

    private static ulong SaturatingSum(ulong left, ulong right)
    {
        return ulong.MaxValue - left < right ? ulong.MaxValue : left + right;
    }

    private sealed class SortCache
    {
        public ulong LatestSeq { get; set; }

        public SortColumn SortCol { get; set; } = SortColumn.CpuPct;

        public SortDirection SortDir { get; set; } = SortDirection.Desc;

        public List<ProcessIdentity> Ordered { get; set; } = [];

        public List<ProcessIdentity> Filtered { get; } = [];

        public Dictionary<ProcessIdentity, ProcessSample> Rows { get; set; } = [];

        public List<ProcessIdentity> PendingUpserts { get; } = [];

        public List<ProcessIdentity> PendingExits { get; } = [];

        public HashSet<ProcessIdentity> ExitsScratch { get; } = [];

        public HashSet<ProcessIdentity> UpsertsSeenScratch { get; } = [];

        public List<ProcessIdentity> PendingUpsertsScratch { get; } = [];

        public List<ProcessIdentity> RemainingScratch { get; } = [];

        public bool Initialized { get; set; }
    }
}
