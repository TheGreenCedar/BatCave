using BatCave.Core.Abstractions;
using BatCave.Core.Domain;

namespace BatCave.Core.Sort;

public sealed class PassThroughSortIndexEngine : ISortIndexEngine
{
    public void OnDelta(ProcessDeltaBatch delta)
    {
        // UI shaping pipeline is the single source of sort/filter truth.
    }

    public QueryResponse Query(QueryRequest request, IReadOnlyList<ProcessSample> rows, ulong seq)
    {
        int total = rows.Count;
        int start = Math.Clamp(request.Offset, 0, total);
        int take = Math.Clamp(request.Limit, 0, total - start);
        List<ProcessSample> page = new(take);
        for (int index = 0; index < take; index++)
        {
            page.Add(rows[start + index]);
        }

        return new QueryResponse
        {
            Seq = seq,
            Total = total,
            Rows = page,
        };
    }
}
