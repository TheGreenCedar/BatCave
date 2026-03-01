using BatCave.Core.Abstractions;
using BatCave.Core.Domain;

namespace BatCave.Core.Sort;

public sealed class NoopSortIndexEngine : ISortIndexEngine
{
    public void OnDelta(ProcessDeltaBatch delta)
    {
    }

    public QueryResponse Query(QueryRequest request, IReadOnlyList<ProcessSample> rows, ulong seq)
    {
        IEnumerable<ProcessSample> filtered = rows;
        if (!string.IsNullOrWhiteSpace(request.FilterText))
        {
            string needle = request.FilterText.Trim().ToLowerInvariant();
            filtered = filtered.Where(row =>
                row.Name.Contains(needle, StringComparison.OrdinalIgnoreCase)
                || row.Pid.ToString().Contains(needle, StringComparison.OrdinalIgnoreCase));
        }

        List<ProcessSample> page = filtered
            .Skip(Math.Max(0, request.Offset))
            .Take(Math.Max(0, request.Limit))
            .ToList();

        return new QueryResponse
        {
            Seq = seq,
            Total = filtered.Count(),
            Rows = page,
        };
    }
}
