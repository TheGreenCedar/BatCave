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
        return _rows.Values.ToList();
    }

    public WarmCache ExportWarmCache(ulong seq)
    {
        return new WarmCache
        {
            Seq = seq,
            Rows = _rows.Values.ToList(),
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

        HashSet<ProcessIdentity> keep = _rows.Values
            .Select(row => new
            {
                Identity = row.Identity(),
                ActivityScore = ComputeActivityScore(row),
            })
            .OrderByDescending(entry => entry.ActivityScore)
            .Take(maxRows)
            .Select(entry => entry.Identity)
            .ToHashSet();

        List<ProcessIdentity> toRemove = _rows.Keys.Where(identity => !keep.Contains(identity)).ToList();
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
