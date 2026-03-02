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
        if (rows.Count > 0 && rows.Count != _cache.Rows.Count)
        {
            _cache.Rows = rows.ToDictionary(row => row.Identity(), row => row);
            _cache.PendingUpserts.Clear();
            _cache.PendingExits.Clear();
            _cache.Initialized = false;
        }

        if (!_cache.Initialized || _cache.SortCol != request.SortCol || _cache.SortDir != request.SortDir)
        {
            _cache.SortCol = request.SortCol;
            _cache.SortDir = request.SortDir;
            RebuildOrdering();
            _cache.PendingUpserts.Clear();
            _cache.PendingExits.Clear();
        }
        else
        {
            ApplyIncrementalUpdates();
        }

        string filterNeedle = request.FilterText.Trim().ToLowerInvariant();
        IReadOnlyList<ProcessIdentity> ordered = _cache.Ordered;
        if (!string.IsNullOrWhiteSpace(filterNeedle))
        {
            ordered = _cache.Ordered
                .Where(identity => MatchesFilter(_cache.Rows[identity], filterNeedle))
                .ToList();
        }

        int total = ordered.Count;
        (int start, int count) = SlicePage(request.Offset, request.Limit, total);
        List<ProcessSample> page = ordered
            .Skip(start)
            .Take(count)
            .Select(identity => _cache.Rows[identity])
            .ToList();

        return new QueryResponse
        {
            Seq = Math.Max(seq, _cache.LatestSeq),
            Total = total,
            Rows = page,
        };
    }

    private void RebuildOrdering()
    {
        _cache.Ordered = _cache.Rows.Keys.ToList();
        _cache.Ordered.Sort((left, right) => CompareIdentity(left, right, _cache.Rows, _cache.SortCol, _cache.SortDir));
        _cache.Initialized = true;
    }

    private void ApplyIncrementalUpdates()
    {
        if (_cache.PendingUpserts.Count == 0 && _cache.PendingExits.Count == 0)
        {
            return;
        }

        if (!_cache.Initialized)
        {
            RebuildOrdering();
            _cache.PendingUpserts.Clear();
            _cache.PendingExits.Clear();
            return;
        }

        HashSet<ProcessIdentity> exitsSeen = _cache.PendingExits.ToHashSet();
        List<ProcessIdentity> pendingUpserts = CollectPendingUpserts();

        _cache.PendingExits.Clear();
        _cache.PendingUpserts.Clear();

        int changeCount = exitsSeen.Count + pendingUpserts.Count;
        int rebuildThreshold = Math.Max(_cache.Rows.Count / 4, 96);
        if (changeCount >= rebuildThreshold)
        {
            RebuildOrdering();
            return;
        }

        if (exitsSeen.Count > 0 || pendingUpserts.Count > 0)
        {
            HashSet<ProcessIdentity> removed = exitsSeen;
            removed.UnionWith(pendingUpserts);

            _cache.Ordered = _cache.Ordered.Where(identity => !removed.Contains(identity)).ToList();
        }

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
        HashSet<ProcessIdentity> upsertsSeen = [];
        List<ProcessIdentity> pendingUpserts = [];
        foreach (ProcessIdentity identity in _cache.PendingUpserts)
        {
            if (upsertsSeen.Add(identity) && _cache.Rows.ContainsKey(identity))
            {
                pendingUpserts.Add(identity);
            }
        }

        return pendingUpserts;
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
        IReadOnlyDictionary<ProcessIdentity, ProcessSample> rows,
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
            SortColumn.NetBps => left.NetBps.CompareTo(right.NetBps),
            SortColumn.Threads => left.Threads.CompareTo(right.Threads),
            SortColumn.Handles => left.Handles.CompareTo(right.Handles),
            SortColumn.StartTimeMs => left.StartTimeMs.CompareTo(right.StartTimeMs),
            _ => 0,
        };

        return sortDir == SortDirection.Asc ? ordering : -ordering;
    }

    private sealed class SortCache
    {
        public ulong LatestSeq { get; set; }

        public SortColumn SortCol { get; set; } = SortColumn.CpuPct;

        public SortDirection SortDir { get; set; } = SortDirection.Desc;

        public List<ProcessIdentity> Ordered { get; set; } = [];

        public Dictionary<ProcessIdentity, ProcessSample> Rows { get; set; } = [];

        public List<ProcessIdentity> PendingUpserts { get; } = [];

        public List<ProcessIdentity> PendingExits { get; } = [];

        public bool Initialized { get; set; }
    }
}
