using BatCave.Core.Abstractions;
using BatCave.Core.Domain;
using Microsoft.Extensions.Logging;
using Microsoft.Extensions.Logging.Abstractions;

namespace BatCave.Core.Runtime;

public sealed class MonitoringRuntime : IMonitoringRuntime, IDisposable
{
    private readonly IProcessCollectorFactory _collectorFactory;
    private readonly ITelemetryPipeline _pipeline;
    private readonly IStateStore _stateStore;
    private readonly ISortIndexEngine _sortIndexEngine;
    private readonly IPersistenceStore _persistenceStore;
    private readonly ILogger<MonitoringRuntime> _logger;
    private readonly ResourceBudgetGuardian _budgetGuardian = new();
    private readonly List<double> _jitterSamples = [];

    private IProcessCollector _collector;
    private QueryRequest _queryRequest;
    private RuntimeHealth _health = new();
    private UserSettings _settings;
    private ulong _seq;

    public MonitoringRuntime(
        IProcessCollectorFactory collectorFactory,
        ITelemetryPipeline pipeline,
        IStateStore stateStore,
        ISortIndexEngine sortIndexEngine,
        IPersistenceStore persistenceStore,
        ILogger<MonitoringRuntime>? logger = null)
    {
        _collectorFactory = collectorFactory;
        _pipeline = pipeline;
        _stateStore = stateStore;
        _sortIndexEngine = sortIndexEngine;
        _persistenceStore = persistenceStore;
        _logger = logger ?? NullLogger<MonitoringRuntime>.Instance;

        _settings = _persistenceStore.LoadSettings() ?? new UserSettings();
        _collector = _collectorFactory.Create(_settings.AdminMode);

        _queryRequest = new QueryRequest
        {
            Offset = 0,
            Limit = 5000,
            SortCol = _settings.SortCol,
            SortDir = _settings.SortDir,
            FilterText = _settings.FilterText,
        };

        WarmCache? warmCache = _persistenceStore.LoadWarmCache();
        if (warmCache is not null)
        {
            _pipeline.SeedFromWarmCache(warmCache.Rows);
            _stateStore.ImportWarmCache(warmCache);
            _seq = Math.Max(_seq, warmCache.Seq);
        }

        _logger.LogInformation(
            "runtime_startup warm_cache_rows={WarmCacheRows} sort_col={SortCol} sort_dir={SortDir} filter_text={FilterText} admin_mode={AdminMode}",
            warmCache?.Rows.Count ?? 0,
            _settings.SortCol,
            _settings.SortDir,
            _settings.FilterText,
            _settings.AdminMode);
    }

    public QueryResponse GetSnapshot()
    {
        IReadOnlyList<ProcessSample> rows = _stateStore.AllRows();
        return _sortIndexEngine.Query(_queryRequest, rows, _seq);
    }

    public SortColumn CurrentSortColumn => _queryRequest.SortCol;

    public SortDirection CurrentSortDirection => _queryRequest.SortDir;

    public string CurrentFilterText => _queryRequest.FilterText;

    public RuntimeHealth GetRuntimeHealth()
    {
        return _health;
    }

    public void SetSort(SortColumn sortCol, SortDirection sortDir)
    {
        _settings = _settings with
        {
            SortCol = sortCol,
            SortDir = sortDir,
        };

        _queryRequest = _queryRequest with
        {
            SortCol = sortCol,
            SortDir = sortDir,
        };

        TryPersist(() => _persistenceStore.SaveSettingsAsync(_settings, CancellationToken.None));
    }

    public void SetFilter(string filterText)
    {
        _settings = _settings with
        {
            FilterText = filterText,
        };

        _queryRequest = _queryRequest with
        {
            FilterText = filterText,
        };

        TryPersist(() => _persistenceStore.SaveSettingsAsync(_settings, CancellationToken.None));
    }

    public bool IsAdminMode()
    {
        return _settings.AdminMode;
    }

    public Task RestartAsync(bool adminMode, CancellationToken ct)
    {
        _settings = _settings with
        {
            AdminMode = adminMode,
        };

        DisposeCollector(_collector);
        _collector = _collectorFactory.Create(adminMode);

        TryPersist(() => _persistenceStore.SaveSettingsAsync(_settings, ct));

        _logger.LogInformation("runtime_restart admin_mode={AdminMode} seq={Seq}", adminMode, _seq);

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

            _logger.LogWarning("collector_warning seq={Seq} message={Message}", _seq, warningMessage);
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

        if (_seq % policy.WarmCacheInterval == 0)
        {
            WarmCache cache = _stateStore.ExportWarmCache(_seq);
            TryPersist(() => _persistenceStore.SaveWarmCacheAsync(cache, CancellationToken.None));
        }

        _logger.LogInformation(
            "runtime_tick seq={Seq} rows={Rows} emit_delta={EmitDelta} degrade_mode={DegradeMode} jitter_p95_ms={JitterP95Ms} dropped_ticks={DroppedTicks} admin_mode={AdminMode}",
            _seq,
            _stateStore.RowCount(),
            policy.EmitTelemetryDelta,
            _health.DegradeMode,
            _health.JitterP95Ms,
            _health.DroppedTicks,
            _settings.AdminMode);

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

        _logger.LogWarning("runtime_dropped_ticks seq={Seq} dropped_delta={DroppedDelta} dropped_total={DroppedTotal}", _seq, dropped, _health.DroppedTicks);
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

    private static void TryPersist(Func<Task> saveAction)
    {
        try
        {
            saveAction().GetAwaiter().GetResult();
        }
        catch
        {
            // keep runtime resilient if local persistence is temporarily unavailable
        }
    }

    public void Dispose()
    {
        DisposeCollector(_collector);
    }

    private static void DisposeCollector(IProcessCollector collector)
    {
        if (collector is IDisposable disposable)
        {
            disposable.Dispose();
        }
    }
}
