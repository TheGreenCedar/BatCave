using BatCave.Core.Abstractions;
using BatCave.Core.Domain;
using Microsoft.Extensions.Logging;
using Microsoft.Extensions.Logging.Abstractions;

namespace BatCave.Core.Runtime;

public sealed class MonitoringRuntime : IMonitoringRuntime, IDisposable
{
    private const int JitterWindowSize = 120;
    private const ulong TickHealthSummaryInterval = 30;

    private readonly IProcessCollectorFactory _collectorFactory;
    private readonly ITelemetryPipeline _pipeline;
    private readonly IStateStore _stateStore;
    private readonly ISortIndexEngine _sortIndexEngine;
    private readonly IPersistenceStore _persistenceStore;
    private readonly ILogger<MonitoringRuntime> _logger;
    private readonly ResourceBudgetGuardian _budgetGuardian = new();
    private readonly double[] _jitterSamples = new double[JitterWindowSize];
    private readonly double[] _jitterScratch = new double[JitterWindowSize];

    private IProcessCollector _collector;
    private QueryRequest _queryRequest;
    private RuntimeHealth _health = new();
    private UserSettings _settings;
    private ulong _seq;
    private int _jitterSampleCount;
    private int _jitterSampleCursor;

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

        _queryRequest = BuildQueryRequest(_settings);

        WarmCache? warmCache = LoadWarmCache();
        LogStartup(warmCache);
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
        _settings = _settings with { SortCol = sortCol, SortDir = sortDir };
        _queryRequest = _queryRequest with { SortCol = sortCol, SortDir = sortDir };
        PersistSettings();
    }

    public void SetFilter(string filterText)
    {
        _settings = _settings with { FilterText = filterText };
        _queryRequest = _queryRequest with { FilterText = filterText };
        PersistSettings();
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
        CollectorWarning? warning = CaptureRuntimeWarning();

        ProcessDeltaBatch delta = ApplyRawDelta(raw);

        RuntimePolicy policy = ApplyHealthAndPolicy(raw, jitterMs, out int rowCount);

        PersistWarmCacheIfDue(policy.WarmCacheInterval);
        warning ??= CaptureRuntimeWarning();

        LogTick(policy, rowCount);

        return new TickOutcome
        {
            Delta = delta,
            Health = _health,
            Warning = warning,
            EmitTelemetryDelta = policy.EmitTelemetryDelta,
        };
    }

    private void PersistSettings()
    {
        TryPersist(() => _persistenceStore.SaveSettingsAsync(_settings, CancellationToken.None));
    }

    private static QueryRequest BuildQueryRequest(UserSettings settings)
    {
        return new QueryRequest
        {
            Offset = 0,
            Limit = 5000,
            SortCol = settings.SortCol,
            SortDir = settings.SortDir,
            FilterText = settings.FilterText,
        };
    }

    private WarmCache? LoadWarmCache()
    {
        WarmCache? warmCache = _persistenceStore.LoadWarmCache();
        if (warmCache is null)
        {
            return null;
        }

        _pipeline.SeedFromWarmCache(warmCache.Rows);
        _stateStore.ImportWarmCache(warmCache);
        _seq = Math.Max(_seq, warmCache.Seq);
        return warmCache;
    }

    private void LogStartup(WarmCache? warmCache)
    {
        _logger.LogInformation(
            "runtime_startup warm_cache_rows={WarmCacheRows} sort_col={SortCol} sort_dir={SortDir} filter_text={FilterText} admin_mode={AdminMode}",
            warmCache?.Rows.Count ?? 0,
            _settings.SortCol,
            _settings.SortDir,
            _settings.FilterText,
            _settings.AdminMode);
    }

    private ProcessDeltaBatch ApplyRawDelta(IReadOnlyList<ProcessSample> raw)
    {
        ProcessDeltaBatch delta = _pipeline.ApplyRaw(_seq, raw);
        _stateStore.ApplyDelta(delta);
        _sortIndexEngine.OnDelta(delta);
        return delta;
    }

    private RuntimePolicy ApplyHealthAndPolicy(IReadOnlyList<ProcessSample> raw, double jitterMs, out int rowCount)
    {
        UpdateJitterSamples(jitterMs);
        UpdateHealthForCurrentTick(raw);

        rowCount = _stateStore.RowCount();
        RuntimePolicy policy = _budgetGuardian.Evaluate(_seq, _health, rowCount);

        if (policy.CompactMaxRows is int maxRows)
        {
            _stateStore.CompactTo(maxRows);
            rowCount = _stateStore.RowCount();
        }

        _health = _health with
        {
            DegradeMode = _budgetGuardian.IsDegraded(),
        };

        return policy;
    }

    private CollectorWarning? CaptureRuntimeWarning()
    {
        string? warningMessage = TakeNextWarningMessage();

        if (string.IsNullOrWhiteSpace(warningMessage))
        {
            return null;
        }

        CollectorWarning warning = new()
        {
            Message = warningMessage,
            Seq = _seq,
        };

        _health = _health with
        {
            CollectorWarnings = _health.CollectorWarnings + 1,
        };

        _logger.LogWarning("runtime_warning seq={Seq} message={Message}", _seq, warningMessage);
        return warning;
    }

    private string? TakeNextWarningMessage()
    {
        string? warningMessage = _collector.TakeWarning();
        if (!string.IsNullOrWhiteSpace(warningMessage))
        {
            return warningMessage;
        }

        return _persistenceStore.TakeWarning();
    }

    private void LogTick(RuntimePolicy policy, int rowCount)
    {
        _logger.LogDebug(
            "runtime_tick seq={Seq} rows={Rows} emit_delta={EmitDelta} degrade_mode={DegradeMode} jitter_p95_ms={JitterP95Ms} dropped_ticks={DroppedTicks} admin_mode={AdminMode}",
            _seq,
            rowCount,
            policy.EmitTelemetryDelta,
            _health.DegradeMode,
            _health.JitterP95Ms,
            _health.DroppedTicks,
            _settings.AdminMode);

        if (_seq % TickHealthSummaryInterval == 0)
        {
            _logger.LogInformation(
                "runtime_health_summary seq={Seq} rows={Rows} app_cpu_pct={AppCpuPct:F3} app_rss_bytes={AppRssBytes} degrade_mode={DegradeMode} dropped_ticks={DroppedTicks}",
                _seq,
                rowCount,
                _health.AppCpuPct,
                _health.AppRssBytes,
                _health.DegradeMode,
                _health.DroppedTicks);
        }
    }

    private void UpdateJitterSamples(double jitterMs)
    {
        _jitterSamples[_jitterSampleCursor] = Math.Abs(jitterMs);
        _jitterSampleCursor = (_jitterSampleCursor + 1) % JitterWindowSize;
        _jitterSampleCount = Math.Min(_jitterSampleCount + 1, JitterWindowSize);
    }

    private void UpdateHealthForCurrentTick(IReadOnlyList<ProcessSample> raw)
    {
        ProcessSample? selfSample = null;
        for (int index = 0; index < raw.Count; index++)
        {
            ProcessSample sample = raw[index];
            if (sample.Pid == (uint)Environment.ProcessId)
            {
                selfSample = sample;
                break;
            }
        }

        RotateJitterSamplesIntoScratch();
        _health = _health with
        {
            Seq = _seq,
            LastTickMs = (ulong)DateTimeOffset.UtcNow.ToUnixTimeMilliseconds(),
            JitterP95Ms = PercentileMath.Percentile95(_jitterScratch, _jitterSampleCount, _jitterScratch),
            AppCpuPct = selfSample?.CpuPct ?? _health.AppCpuPct,
            AppRssBytes = selfSample?.RssBytes ?? EstimateRssFromRows(_stateStore.RowCount()),
        };
    }

    private void PersistWarmCacheIfDue(ulong warmCacheInterval)
    {
        if (_seq % warmCacheInterval != 0)
        {
            return;
        }

        WarmCache cache = _stateStore.ExportWarmCache(_seq);
        TryPersist(() => _persistenceStore.SaveWarmCacheAsync(cache, CancellationToken.None));
    }

    public void RecordDroppedTicks(ulong dropped)
    {
        _health = _health with
        {
            DroppedTicks = _health.DroppedTicks + dropped,
        };

        _logger.LogWarning("runtime_dropped_ticks seq={Seq} dropped_delta={DroppedDelta} dropped_total={DroppedTotal}", _seq, dropped, _health.DroppedTicks);
    }

    private void RotateJitterSamplesIntoScratch()
    {
        if (_jitterSampleCount == 0)
        {
            return;
        }

        if (_jitterSampleCount < JitterWindowSize)
        {
            Array.Copy(_jitterSamples, 0, _jitterScratch, 0, _jitterSampleCount);
            return;
        }

        int tailLength = JitterWindowSize - _jitterSampleCursor;
        Array.Copy(_jitterSamples, _jitterSampleCursor, _jitterScratch, 0, tailLength);
        Array.Copy(_jitterSamples, 0, _jitterScratch, tailLength, _jitterSampleCursor);
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
