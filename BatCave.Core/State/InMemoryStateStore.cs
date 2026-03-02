using BatCave.Core.Abstractions;
using BatCave.Core.Domain;

namespace BatCave.Core.State;

public sealed class InMemoryStateStore : IStateStore
{
    private readonly Dictionary<ProcessIdentity, ProcessSample> _rows = new();

    public void ApplyDelta(ProcessDeltaBatch delta)
    {
        foreach (ProcessSample row in delta.Upserts)
        {
            _rows[row.Identity()] = row;
        }

        foreach (ProcessIdentity identity in delta.Exits)
        {
            _rows.Remove(identity);
        }
    }

    public IReadOnlyList<ProcessSample> AllRows()
    {
        List<ProcessSample> rows = new(_rows.Count);
        foreach (ProcessSample row in _rows.Values)
        {
            rows.Add(row);
        }

        return rows;
    }

    public WarmCache ExportWarmCache(ulong seq)
    {
        List<ProcessSample> rows = new(_rows.Count);
        foreach (ProcessSample row in _rows.Values)
        {
            rows.Add(row);
        }

        return new WarmCache
        {
            Seq = seq,
            Rows = rows,
        };
    }

    public void ImportWarmCache(WarmCache cache)
    {
        _rows.Clear();
        foreach (ProcessSample row in cache.Rows)
        {
            _rows[row.Identity()] = row;
        }
    }

    public int RowCount()
    {
        return _rows.Count;
    }

    public void CompactTo(int maxRows)
    {
        if (_rows.Count <= maxRows)
        {
            return;
        }

        List<(ProcessIdentity Identity, ulong Score)> ranked = new(_rows.Count);
        foreach ((ProcessIdentity identity, ProcessSample row) in _rows)
        {
            ranked.Add((identity, ComputeActivityScore(row)));
        }

        ranked.Sort(static (left, right) => right.Score.CompareTo(left.Score));

        HashSet<ProcessIdentity> keep = new(maxRows);
        int keepCount = Math.Min(maxRows, ranked.Count);
        for (int index = 0; index < keepCount; index++)
        {
            keep.Add(ranked[index].Identity);
        }

        List<ProcessIdentity> toRemove = new(_rows.Count - keep.Count);
        foreach (ProcessIdentity identity in _rows.Keys)
        {
            if (!keep.Contains(identity))
            {
                toRemove.Add(identity);
            }
        }

        foreach (ProcessIdentity identity in toRemove)
        {
            _rows.Remove(identity);
        }
    }

    private static ulong ComputeActivityScore(ProcessSample row)
    {
        return (ulong)(row.CpuPct * 1000.0) + row.IoReadBps + row.IoWriteBps + row.NetBps + row.RssBytes / 1024;
    }
}
