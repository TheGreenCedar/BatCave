using BatCave.Core.Abstractions;
using BatCave.Core.Domain;

namespace BatCave.Core.Runtime;

public sealed class MonitoringRuntime : IMonitoringRuntime
{
    private readonly IProcessCollector _collector;
    private readonly ITelemetryPipeline _pipeline;
    private readonly IStateStore _stateStore;
    private readonly ISortIndexEngine _sortIndexEngine;
    private readonly ResourceBudgetGuardian _budgetGuardian = new();
    private readonly List<double> _jitterSamples = [];

    private QueryRequest _queryRequest = new();
    private RuntimeHealth _health = new();
    private ulong _seq;

    public MonitoringRuntime(
        IProcessCollector collector,
        ITelemetryPipeline pipeline,
        IStateStore stateStore,
        ISortIndexEngine sortIndexEngine)
    {
        _collector = collector;
        _pipeline = pipeline;
        _stateStore = stateStore;
        _sortIndexEngine = sortIndexEngine;
    }

    public QueryResponse GetSnapshot()
    {
        IReadOnlyList<ProcessSample> rows = _stateStore.AllRows();
        return _sortIndexEngine.Query(_queryRequest, rows, _seq);
    }

    public RuntimeHealth GetRuntimeHealth()
    {
        return _health;
    }

    public void SetSort(SortColumn sortCol, SortDirection sortDir)
    {
        _queryRequest = _queryRequest with
        {
            SortCol = sortCol,
            SortDir = sortDir,
        };
    }

    public void SetFilter(string filterText)
    {
        _queryRequest = _queryRequest with
        {
            FilterText = filterText,
        };
    }

    public bool IsAdminMode()
    {
        return false;
    }

    public Task RestartAsync(bool adminMode, CancellationToken ct)
    {
        return Task.CompletedTask;
    }

    public TickOutcome Tick(double jitterMs)
    {
        _seq++;
        IReadOnlyList<ProcessSample> raw = _collector.CollectTick(_seq);
        string? warningMessage = _collector.TakeWarning();

        CollectorWarning? warning = null;
        if (!string.IsNullOrWhiteSpace(warningMessage))
        {
            warning = new CollectorWarning
            {
                Message = warningMessage,
                Seq = _seq,
            };
            _health = _health with
            {
                CollectorWarnings = _health.CollectorWarnings + 1,
            };
        }
        else if (raw.Count == 0)
        {
            warning = new CollectorWarning
            {
                Message = "collector returned zero rows",
                Seq = _seq,
            };
            _health = _health with
            {
                CollectorWarnings = _health.CollectorWarnings + 1,
            };
        }

        ProcessDeltaBatch delta = _pipeline.ApplyRaw(_seq, raw);
        _stateStore.ApplyDelta(delta);
        _sortIndexEngine.OnDelta(delta);

        _jitterSamples.Add(Math.Abs(jitterMs));
        if (_jitterSamples.Count > 120)
        {
            _jitterSamples.RemoveRange(0, _jitterSamples.Count - 120);
        }

        ProcessSample? selfSample = raw.FirstOrDefault(sample => sample.Pid == (uint)Environment.ProcessId);

        _health = _health with
        {
            Seq = _seq,
            LastTickMs = (ulong)DateTimeOffset.UtcNow.ToUnixTimeMilliseconds(),
            JitterP95Ms = Percentile95(_jitterSamples),
            AppCpuPct = selfSample?.CpuPct ?? _health.AppCpuPct,
            AppRssBytes = selfSample?.RssBytes ?? EstimateRssFromRows(_stateStore.RowCount()),
        };

        RuntimePolicy policy = _budgetGuardian.Evaluate(_seq, _health, _stateStore.RowCount());

        if (policy.CompactMaxRows is int maxRows)
        {
            _stateStore.CompactTo(maxRows);
        }

        _health = _health with
        {
            DegradeMode = _budgetGuardian.IsDegraded(),
        };

        return new TickOutcome
        {
            Delta = delta,
            Health = _health,
            Warning = warning,
            EmitTelemetryDelta = policy.EmitTelemetryDelta,
        };
    }

    public void RecordDroppedTicks(ulong dropped)
    {
        _health = _health with
        {
            DroppedTicks = _health.DroppedTicks + dropped,
        };
    }

    private static double Percentile95(IReadOnlyList<double> values)
    {
        if (values.Count == 0)
        {
            return 0;
        }

        List<double> sorted = values.OrderBy(value => value).ToList();
        int index = Math.Min(sorted.Count - 1, Math.Max(0, (int)Math.Ceiling(sorted.Count * 0.95) - 1));
        return sorted[index];
    }

    private static ulong EstimateRssFromRows(int rowCount)
    {
        const ulong baseline = 48 * 1024 * 1024;
        return baseline + (ulong)Math.Max(0, rowCount) * 2_048;
    }
}
