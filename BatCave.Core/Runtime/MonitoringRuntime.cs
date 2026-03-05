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
    private readonly RuntimeHostOptions _runtimeHostOptions;
    private readonly CoalescedSettingsWriteQueue _settingsWriteQueue;
    private readonly CoalescedWarmCacheWriteQueue _warmCacheWriteQueue;
    private readonly ILogger<MonitoringRuntime> _logger;
    private readonly ResourceBudgetGuardian _budgetGuardian = new();
    private readonly SlidingWindowP95Histogram _jitterHistogram = new(JitterWindowSize, maxBucketInclusive: 4_000);
    private readonly object _runtimeWarningSync = new();
    private readonly Queue<string> _runtimeWarnings = [];

    private IProcessCollector _collector;
    private QueryRequest _queryRequest;
    private RuntimeHealth _health = new();
    private UserSettings _settings;
    private bool _effectiveAdminMode;
    private ulong _seq;

    public MonitoringRuntime(
        IProcessCollectorFactory collectorFactory,
        ITelemetryPipeline pipeline,
        IStateStore stateStore,
        ISortIndexEngine sortIndexEngine,
        IPersistenceStore persistenceStore,
        RuntimeHostOptions runtimeHostOptions,
        ILogger<MonitoringRuntime>? logger = null)
    {
        _collectorFactory = collectorFactory;
        _pipeline = pipeline;
        _stateStore = stateStore;
        _sortIndexEngine = sortIndexEngine;
        _persistenceStore = persistenceStore;
        _runtimeHostOptions = RuntimeHostOptionsValidator.Validate(runtimeHostOptions);
        _settingsWriteQueue = new CoalescedSettingsWriteQueue(_persistenceStore.SaveSettingsAsync);
        _warmCacheWriteQueue = new CoalescedWarmCacheWriteQueue(_persistenceStore.SaveWarmCacheAsync);
        _logger = logger ?? NullLogger<MonitoringRuntime>.Instance;

        _settings = _persistenceStore.LoadSettings() ?? BuildDefaultSettings();
        RuntimeHostOptionsValidator.ValidatePersistedSettings(_settings);
        bool adminPreferenceMigrated = false;
        if (!_settings.AdminPreferenceInitialized)
        {
            _settings = _settings with
            {
                AdminMode = _runtimeHostOptions.DefaultAdminMode,
                AdminPreferenceInitialized = true,
            };
            adminPreferenceMigrated = true;
        }

        int normalizedMetricTrendWindowSeconds = NormalizeMetricTrendWindowSeconds(_settings.MetricTrendWindowSeconds);
        bool metricTrendWindowNormalized = _settings.MetricTrendWindowSeconds != normalizedMetricTrendWindowSeconds;
        _settings = _settings with
        {
            MetricTrendWindowSeconds = normalizedMetricTrendWindowSeconds,
        };
        bool requestedStartupAdminMode = _settings.AdminMode;
        CollectorActivationResult startupCollector = ActivateCollector(requestedStartupAdminMode);
        _collector = startupCollector.Collector;
        _effectiveAdminMode = startupCollector.AdminMode;
        EnqueueRuntimeWarning(startupCollector.Warning);
        if (metricTrendWindowNormalized || adminPreferenceMigrated)
        {
            PersistSettings();
        }

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
        UpdateSettingsAndQuery(
            settings => settings with { SortCol = sortCol, SortDir = sortDir },
            request => request with { SortCol = sortCol, SortDir = sortDir });
    }

    public void SetFilter(string filterText)
    {
        UpdateSettingsAndQuery(
            settings => settings with { FilterText = filterText },
            request => request with { FilterText = filterText });
    }

    public bool IsAdminMode()
    {
        return _effectiveAdminMode;
    }

    public int CurrentMetricTrendWindowSeconds => _settings.MetricTrendWindowSeconds;

    public void SetMetricTrendWindowSeconds(int seconds)
    {
        int normalized = NormalizeMetricTrendWindowSeconds(seconds);
        if (_settings.MetricTrendWindowSeconds == normalized)
        {
            return;
        }

        _settings = _settings with
        {
            MetricTrendWindowSeconds = normalized,
        };

        PersistSettings();
    }

    private void UpdateSettingsAndQuery(
        Func<UserSettings, UserSettings> settingsUpdater,
        Func<QueryRequest, QueryRequest> queryUpdater)
    {
        _settings = settingsUpdater(_settings);
        _queryRequest = queryUpdater(_queryRequest);
        PersistSettings();
    }

    public async Task RestartAsync(bool adminMode, CancellationToken ct)
    {
        CollectorActivationResult nextCollector = ActivateCollector(adminMode);
        IProcessCollector previousCollector = _collector;
        _collector = nextCollector.Collector;
        _effectiveAdminMode = nextCollector.AdminMode;
        _settings = _settings with
        {
            AdminMode = adminMode,
            AdminPreferenceInitialized = true,
        };

        DisposeCollector(previousCollector);
        EnqueueRuntimeWarning(nextCollector.Warning);

        PersistSettings();
        await FlushSettingsQueueAsync(ct).ConfigureAwait(false);

        _logger.LogInformation(
            "runtime_restart requested_admin_mode={RequestedAdminMode} effective_admin_mode={EffectiveAdminMode} seq={Seq}",
            adminMode,
            _effectiveAdminMode,
            _seq);
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
        _settingsWriteQueue.Enqueue(_settings);
    }

    private async Task FlushSettingsQueueAsync(CancellationToken ct)
    {
        try
        {
            await _settingsWriteQueue.FlushAsync(ct).ConfigureAwait(false);
        }
        catch
        {
            // keep runtime resilient if local persistence is temporarily unavailable
        }
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

    private static int NormalizeMetricTrendWindowSeconds(int seconds)
    {
        return RuntimeHostOptionsValidator.IsSupportedMetricTrendWindowSeconds(seconds)
            ? seconds
            : 60;
    }

    private UserSettings BuildDefaultSettings()
    {
        return new UserSettings
        {
            SortCol = _runtimeHostOptions.DefaultSortColumn,
            SortDir = _runtimeHostOptions.DefaultSortDirection,
            FilterText = _runtimeHostOptions.DefaultFilterText,
            AdminMode = _runtimeHostOptions.DefaultAdminMode,
            AdminPreferenceInitialized = true,
            MetricTrendWindowSeconds = _runtimeHostOptions.DefaultMetricTrendWindowSeconds,
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
            "runtime_startup warm_cache_rows={WarmCacheRows} sort_col={SortCol} sort_dir={SortDir} filter_text={FilterText} requested_admin_mode={RequestedAdminMode} effective_admin_mode={EffectiveAdminMode}",
            warmCache?.Rows.Count ?? 0,
            _settings.SortCol,
            _settings.SortDir,
            _settings.FilterText,
            _settings.AdminMode,
            _effectiveAdminMode);
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
        return EvaluateAndApplyRuntimePolicy(out rowCount);
    }

    private RuntimePolicy EvaluateAndApplyRuntimePolicy(out int rowCount)
    {
        rowCount = _stateStore.RowCount();
        RuntimePolicy policy = _budgetGuardian.Evaluate(_seq, _health, rowCount);
        rowCount = ApplyCompactionPolicy(policy, rowCount);

        _health = _health with
        {
            DegradeMode = _budgetGuardian.IsDegraded(),
        };

        return policy;
    }

    private int ApplyCompactionPolicy(RuntimePolicy policy, int rowCount)
    {
        if (policy.CompactMaxRows is not int maxRows)
        {
            return rowCount;
        }

        _stateStore.CompactTo(maxRows);
        return _stateStore.RowCount();
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
        string? runtimeWarning = TryDequeueRuntimeWarning();
        if (!string.IsNullOrWhiteSpace(runtimeWarning))
        {
            return runtimeWarning;
        }

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
            _effectiveAdminMode);

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
        _jitterHistogram.AddSampleMs(Math.Abs(jitterMs));
    }

    private void UpdateHealthForCurrentTick(IReadOnlyList<ProcessSample> raw)
    {
        ProcessSample? selfSample = FindSelfSample(raw);
        _health = _health with
        {
            Seq = _seq,
            LastTickMs = (ulong)DateTimeOffset.UtcNow.ToUnixTimeMilliseconds(),
            JitterP95Ms = _jitterHistogram.Percentile95Ms(),
            AppCpuPct = selfSample?.CpuPct ?? _health.AppCpuPct,
            AppRssBytes = selfSample?.RssBytes ?? EstimateRssFromRows(_stateStore.RowCount()),
        };
    }

    private static ProcessSample? FindSelfSample(IReadOnlyList<ProcessSample> raw)
    {
        uint processId = (uint)Environment.ProcessId;
        for (int index = 0; index < raw.Count; index++)
        {
            ProcessSample sample = raw[index];
            if (sample.Pid == processId)
            {
                return sample;
            }
        }

        return null;
    }

    private void PersistWarmCacheIfDue(ulong warmCacheInterval)
    {
        if (_seq % warmCacheInterval != 0)
        {
            return;
        }

        WarmCache cache = _stateStore.ExportWarmCache(_seq);
        _warmCacheWriteQueue.Enqueue(cache);
    }

    public void RecordDroppedTicks(ulong dropped)
    {
        _health = _health with
        {
            DroppedTicks = _health.DroppedTicks + dropped,
        };

        _logger.LogWarning("runtime_dropped_ticks seq={Seq} dropped_delta={DroppedDelta} dropped_total={DroppedTotal}", _seq, dropped, _health.DroppedTicks);
    }

    private static ulong EstimateRssFromRows(int rowCount)
    {
        const ulong baseline = 48 * 1024 * 1024;
        return baseline + (ulong)Math.Max(0, rowCount) * 2_048;
    }

    public void Dispose()
    {
        _warmCacheWriteQueue.Dispose();
        _settingsWriteQueue.Dispose();
        DisposeCollector(_collector);
    }

    private static void DisposeCollector(IProcessCollector collector)
    {
        if (collector is IDisposable disposable)
        {
            disposable.Dispose();
        }
    }

    private CollectorActivationResult ActivateCollector(bool adminMode)
    {
        try
        {
            return new CollectorActivationResult(
                Collector: _collectorFactory.Create(adminMode),
                AdminMode: adminMode,
                Warning: null);
        }
        catch (Exception ex) when (adminMode)
        {
            _logger.LogWarning(
                ex,
                "collector_admin_mode_start_failed requested_admin_mode={RequestedAdminMode}. falling_back_to_non_admin",
                adminMode);

            return new CollectorActivationResult(
                Collector: _collectorFactory.Create(adminMode: false),
                AdminMode: false,
                Warning:
                    $"admin_mode_start_failed requested_admin_mode=true fallback_admin_mode=false error={ex.GetType().Name}: {ex.Message}");
        }
    }

    private void EnqueueRuntimeWarning(string? warning)
    {
        if (string.IsNullOrWhiteSpace(warning))
        {
            return;
        }

        lock (_runtimeWarningSync)
        {
            _runtimeWarnings.Enqueue(warning);
        }
    }

    private string? TryDequeueRuntimeWarning()
    {
        lock (_runtimeWarningSync)
        {
            return _runtimeWarnings.TryDequeue(out string? warning) ? warning : null;
        }
    }

    private sealed record CollectorActivationResult(IProcessCollector Collector, bool AdminMode, string? Warning);
}
