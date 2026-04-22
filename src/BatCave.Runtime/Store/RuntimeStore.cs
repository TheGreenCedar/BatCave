using BatCave.Runtime.Collectors;
using BatCave.Runtime.Contracts;
using BatCave.Runtime.Persistence;
using Microsoft.Extensions.Hosting;
using Microsoft.Extensions.Logging;
using Microsoft.Extensions.Logging.Abstractions;
using System.Diagnostics;
using System.Threading.Channels;

namespace BatCave.Runtime.Store;

public sealed record RuntimeStoreOptions
{
    public bool RuntimeLoopEnabled { get; init; } = true;
    public TimeSpan TickInterval { get; init; } = TimeSpan.FromSeconds(1);
    public int SubscriberBufferCapacity { get; init; } = 8;
    public int MaxWarningCount { get; init; } = 16;
    public int WarmCacheWriteIntervalTicks { get; init; } = 10;
    public double DegradeCpuThresholdPct { get; init; } = 6.0d;
    public ulong DegradeRssThresholdBytes { get; init; } = 350UL * 1024UL * 1024UL;
    public RuntimeSettings DefaultSettings { get; init; } = new()
    {
        AdminModeRequested = true,
        AdminModeEnabled = false,
        MetricWindowSeconds = 60,
    };
}

public sealed class RuntimeStore : IRuntimeStore, IHostedService, IAsyncDisposable
{
    private IProcessCollector _processCollector;
    private readonly IProcessCollectorFactory _processCollectorFactory;
    private readonly ISystemMetricsCollector _systemMetricsCollector;
    private readonly IRuntimePersistenceStore _persistenceStore;
    private readonly RuntimeStoreOptions _options;
    private readonly ILogger<RuntimeStore> _logger;
    private readonly Channel<CommandEnvelope> _commands;
    private readonly List<Channel<RuntimeDelta>> _subscribers = [];
    private readonly List<Task> _persistenceTasks = [];
    private readonly object _subscriberSync = new();
    private readonly object _persistenceTaskSync = new();
    private readonly Dictionary<ProcessIdentity, ProcessSample> _allRows = [];
    private readonly Queue<RuntimeWarning> _warnings = [];
    private readonly P95Window _tickP95 = new(120);
    private readonly P95Window _sortP95 = new(120);
    private readonly P95Window _jitterP95 = new(120);
    private readonly Process _currentProcess = Process.GetCurrentProcess();

    private CancellationTokenSource? _stopCts;
    private Task? _loopTask;
    private RuntimeSnapshot _snapshot = new();
    private RuntimeSettings _settings;
    private SystemMetricsSnapshot _system = new();
    private RuntimeHealth _health = new();
    private TimeSpan _previousProcessCpu;
    private long _previousProcessCpuStamp;
    private bool _disposed;
    private ulong _seq;
    private ulong _droppedTicks;

    public RuntimeStore(
        IProcessCollector processCollector,
        ISystemMetricsCollector systemMetricsCollector,
        IRuntimePersistenceStore persistenceStore,
        RuntimeStoreOptions? options = null,
        ILogger<RuntimeStore>? logger = null,
        IProcessCollectorFactory? processCollectorFactory = null)
    {
        _processCollector = processCollector;
        _processCollectorFactory = processCollectorFactory ?? new StaticProcessCollectorFactory(processCollector);
        _systemMetricsCollector = systemMetricsCollector;
        _persistenceStore = persistenceStore;
        _options = options ?? new RuntimeStoreOptions();
        _logger = logger ?? NullLogger<RuntimeStore>.Instance;
        _commands = Channel.CreateBounded<CommandEnvelope>(new BoundedChannelOptions(64)
        {
            SingleReader = true,
            SingleWriter = false,
            FullMode = BoundedChannelFullMode.Wait,
        });

        RuntimeSettings? loadedSettings = _persistenceStore.LoadSettings();
        DrainPersistenceWarnings();
        _settings = NormalizeSettings(loadedSettings ?? _options.DefaultSettings);
        WarmCache? warmCache = _persistenceStore.LoadWarmCache();
        DrainPersistenceWarnings();
        ImportWarmCache(warmCache);
        _health = BuildHealth(runtimeLoopRunning: false, tickMs: 0, sortMs: 0, warning: _warnings.LastOrDefault()?.Message);
        _snapshot = BuildSnapshot();
        _previousProcessCpu = _currentProcess.TotalProcessorTime;
        _previousProcessCpuStamp = Stopwatch.GetTimestamp();
    }

    public RuntimeSnapshot GetSnapshot() => Volatile.Read(ref _snapshot);

    public async IAsyncEnumerable<RuntimeDelta> SubscribeAsync([System.Runtime.CompilerServices.EnumeratorCancellation] CancellationToken ct)
    {
        Channel<RuntimeDelta> channel = Channel.CreateBounded<RuntimeDelta>(new BoundedChannelOptions(_options.SubscriberBufferCapacity)
        {
            SingleReader = true,
            SingleWriter = false,
            FullMode = BoundedChannelFullMode.DropOldest,
        });

        lock (_subscriberSync)
        {
            _subscribers.Add(channel);
        }

        try
        {
            await foreach (RuntimeDelta delta in channel.Reader.ReadAllAsync(ct).ConfigureAwait(false))
            {
                yield return delta;
            }
        }
        finally
        {
            lock (_subscriberSync)
            {
                _subscribers.Remove(channel);
            }

            channel.Writer.TryComplete();
        }
    }

    public async Task ExecuteAsync(RuntimeCommand command, CancellationToken ct)
    {
        ObjectDisposedException.ThrowIf(_disposed, this);
        TaskCompletionSource completion = new(TaskCreationOptions.RunContinuationsAsynchronously);
        await _commands.Writer.WriteAsync(new CommandEnvelope(command, completion), ct).ConfigureAwait(false);
        await completion.Task.WaitAsync(ct).ConfigureAwait(false);
    }

    public Task StartAsync(CancellationToken cancellationToken)
    {
        ObjectDisposedException.ThrowIf(_disposed, this);
        if (_loopTask is { IsCompleted: false })
        {
            return Task.CompletedTask;
        }

        _stopCts = CancellationTokenSource.CreateLinkedTokenSource(cancellationToken);
        _loopTask = RunLoopAsync(_stopCts.Token);
        return Task.CompletedTask;
    }

    public async Task StopAsync(CancellationToken cancellationToken)
    {
        if (_stopCts is null)
        {
            return;
        }

        _stopCts.Cancel();
        if (_loopTask is not null)
        {
            try
            {
                await _loopTask.WaitAsync(cancellationToken).ConfigureAwait(false);
            }
            catch (OperationCanceledException) when (!cancellationToken.IsCancellationRequested)
            {
            }
        }
    }

    public async ValueTask DisposeAsync()
    {
        if (_disposed)
        {
            return;
        }

        _disposed = true;
        _commands.Writer.TryComplete();
        if (_stopCts is not null)
        {
            await StopAsync(CancellationToken.None).ConfigureAwait(false);
            _stopCts.Dispose();
        }

        lock (_subscriberSync)
        {
            foreach (Channel<RuntimeDelta> subscriber in _subscribers)
            {
                subscriber.Writer.TryComplete();
            }

            _subscribers.Clear();
        }

        await FlushPersistenceTasksAsync().ConfigureAwait(false);
        DisposeCollector(_processCollector);
        _currentProcess.Dispose();
    }

    private async Task RunLoopAsync(CancellationToken ct)
    {
        DateTimeOffset nextTick = DateTimeOffset.UtcNow;
        while (!ct.IsCancellationRequested)
        {
            await DrainCommandsAsync(ct).ConfigureAwait(false);

            if (!_settings.Paused && DateTimeOffset.UtcNow >= nextTick)
            {
                DateTimeOffset scheduledTick = nextTick;
                RuntimeDelta delta = Tick(scheduledTick);
                Publish(delta);
                nextTick = DateTimeOffset.UtcNow.Add(_options.TickInterval);
            }

            TimeSpan wait = _settings.Paused
                ? Timeout.InfiniteTimeSpan
                : nextTick - DateTimeOffset.UtcNow;
            if (wait != Timeout.InfiniteTimeSpan && wait < TimeSpan.Zero)
            {
                wait = TimeSpan.Zero;
            }

            try
            {
                Task commandWait = _commands.Reader.WaitToReadAsync(ct).AsTask();
                Task delay = wait == Timeout.InfiniteTimeSpan
                    ? Task.Delay(Timeout.InfiniteTimeSpan, ct)
                    : Task.Delay(wait, ct);
                await Task.WhenAny(commandWait, delay).ConfigureAwait(false);
            }
            catch (OperationCanceledException) when (ct.IsCancellationRequested)
            {
                break;
            }
        }
    }

    private async Task DrainCommandsAsync(CancellationToken ct)
    {
        while (_commands.Reader.TryRead(out CommandEnvelope? envelope))
        {
            try
            {
                await ApplyCommandAsync(envelope.Command, ct).ConfigureAwait(false);
                envelope.Completion.TrySetResult();
            }
            catch (Exception ex)
            {
                envelope.Completion.TrySetException(ex);
            }
        }
    }

    private async Task ApplyCommandAsync(RuntimeCommand command, CancellationToken ct)
    {
        switch (command)
        {
            case SetProcessQueryCommand setQuery:
                _settings = _settings with { Query = NormalizeQuery(setQuery.Query) };
                QueueSettingsPersistence();
                PublishSnapshotOnly();
                break;
            case SetAdminModeCommand setAdmin:
                CollectorActivationResult activation = await _processCollectorFactory
                    .CreateAsync(setAdmin.Enabled, ct)
                    .ConfigureAwait(false);
                IProcessCollector previousCollector = _processCollector;
                _processCollector = activation.Collector;
                if (!ReferenceEquals(previousCollector, activation.Collector))
                {
                    DisposeCollector(previousCollector);
                }

                _settings = _settings with
                {
                    AdminModeRequested = setAdmin.Enabled,
                    AdminModeEnabled = setAdmin.Enabled && activation.EffectiveAdminMode,
                };
                QueueSettingsPersistence();
                string? adminWarning = activation.Warning
                    ?? (setAdmin.Enabled && !activation.EffectiveAdminMode
                        ? "admin_mode_start_failed requested_admin_mode=true fallback_admin_mode=false error=Unknown: elevated collector was not activated."
                        : null);
                PublishSnapshotOnly(
                    "admin_mode",
                    adminWarning ?? (setAdmin.Enabled ? null : "Admin mode disabled."));
                break;
            case SetMetricWindowCommand setWindow:
                _settings = _settings with { MetricWindowSeconds = NormalizeMetricWindow(setWindow.Seconds) };
                QueueSettingsPersistence();
                PublishSnapshotOnly();
                break;
            case RefreshNowCommand:
                Publish(Tick(DateTimeOffset.UtcNow));
                break;
            case PauseRuntimeCommand:
                _settings = _settings with { Paused = true };
                PublishSnapshotOnly();
                break;
            case ResumeRuntimeCommand:
                _settings = _settings with { Paused = false };
                PublishSnapshotOnly();
                break;
        }
    }

    private RuntimeDelta Tick(DateTimeOffset scheduledTick)
    {
        _seq++;
        Stopwatch tickWatch = Stopwatch.StartNew();
        IReadOnlyList<ProcessSample> collectedRows = _processCollector.Collect(_seq);
        tickWatch.Stop();
        DrainCollectorWarnings(_processCollector);
        DrainPersistenceWarnings();

        HashSet<ProcessIdentity> seen = [];
        List<ProcessSample> upserts = [];
        foreach (ProcessSample sample in collectedRows)
        {
            ProcessIdentity identity = sample.Identity();
            seen.Add(identity);
            bool changed = !_allRows.TryGetValue(identity, out ProcessSample? previous)
                || HasMeaningfulProcessChange(previous, sample);

            _allRows[identity] = sample;
            if (changed)
            {
                upserts.Add(sample);
            }
        }

        List<ProcessIdentity> exits = [];
        foreach (ProcessIdentity identity in _allRows.Keys.ToArray())
        {
            if (!seen.Contains(identity))
            {
                _allRows.Remove(identity);
                exits.Add(identity);
            }
        }

        _system = _systemMetricsCollector.Sample();
        double tickMs = tickWatch.Elapsed.TotalMilliseconds;
        _tickP95.Add(tickMs);
        double jitterMs = Math.Abs((DateTimeOffset.UtcNow - scheduledTick).TotalMilliseconds);
        _jitterP95.Add(jitterMs);
        TrackDroppedTicks(jitterMs);
        Stopwatch sortWatch = Stopwatch.StartNew();
        RuntimeSnapshot snapshot = BuildSnapshot();
        sortWatch.Stop();
        _sortP95.Add(sortWatch.Elapsed.TotalMilliseconds);
        _health = BuildHealth(runtimeLoopRunning: !_settings.Paused, tickMs, sortWatch.Elapsed.TotalMilliseconds, warning: null);
        snapshot = snapshot with { Health = _health };
        Volatile.Write(ref _snapshot, snapshot);

        if (_seq % (ulong)Math.Max(1, _options.WarmCacheWriteIntervalTicks) == 0)
        {
            QueueWarmCachePersistence(_seq, _allRows.Values);
        }

        return new RuntimeDelta
        {
            Seq = _seq,
            TsMs = snapshot.TsMs,
            Upserts = Freeze(upserts),
            Exits = Freeze(exits),
            Health = _health,
            System = _system,
            Snapshot = snapshot,
        };
    }

    private RuntimeSnapshot BuildSnapshot()
    {
        ulong nowMs = UnixNowMs();
        ProcessSample[] rows = ShapeRows(_allRows.Values, _settings.Query);
        return new RuntimeSnapshot
        {
            Seq = _seq,
            TsMs = nowMs,
            Settings = _settings,
            Health = _health,
            System = _system,
            Rows = Freeze(rows),
            TotalProcessCount = _allRows.Count,
            Warnings = Freeze(_warnings),
        };
    }

    private ProcessSample[] ShapeRows(IEnumerable<ProcessSample> rows, RuntimeQuery query)
    {
        IEnumerable<ProcessSample> shaped = rows;
        string needle = query.FilterText.Trim();
        if (!string.IsNullOrWhiteSpace(needle))
        {
            shaped = shaped.Where(row =>
                row.Name.Contains(needle, StringComparison.OrdinalIgnoreCase)
                || row.Pid.ToString(System.Globalization.CultureInfo.InvariantCulture).Contains(needle, StringComparison.OrdinalIgnoreCase));
        }

        IOrderedEnumerable<ProcessSample> ordered = query.SortColumn switch
        {
            SortColumn.Name => query.SortDirection == SortDirection.Asc
                ? shaped.OrderBy(static row => row.Name, StringComparer.OrdinalIgnoreCase)
                : shaped.OrderByDescending(static row => row.Name, StringComparer.OrdinalIgnoreCase),
            SortColumn.Pid => Order(shaped, static row => row.Pid, query.SortDirection),
            SortColumn.MemoryBytes => Order(shaped, static row => row.MemoryBytes, query.SortDirection),
            SortColumn.DiskBps => Order(shaped, static row => row.DiskBps, query.SortDirection),
            SortColumn.OtherIoBps => Order(shaped, static row => row.OtherIoBps, query.SortDirection),
            SortColumn.Threads => Order(shaped, static row => row.Threads, query.SortDirection),
            SortColumn.Handles => Order(shaped, static row => row.Handles, query.SortDirection),
            SortColumn.StartTimeMs => Order(shaped, static row => row.StartTimeMs, query.SortDirection),
            _ => Order(shaped, static row => row.CpuPct, query.SortDirection),
        };

        return ordered
            .ThenBy(static row => row.Name, StringComparer.OrdinalIgnoreCase)
            .Take(query.Limit)
            .ToArray();
    }

    private static IOrderedEnumerable<ProcessSample> Order<TKey>(
        IEnumerable<ProcessSample> rows,
        Func<ProcessSample, TKey> keySelector,
        SortDirection direction)
    {
        return direction == SortDirection.Asc
            ? rows.OrderBy(keySelector)
            : rows.OrderByDescending(keySelector);
    }

    private RuntimeHealth BuildHealth(bool runtimeLoopRunning, double tickMs, double sortMs, string? warning)
    {
        double appCpuPct = SampleAppCpuPct();
        ulong rssBytes = 0;
        try
        {
            _currentProcess.Refresh();
            rssBytes = (ulong)Math.Max(0L, _currentProcess.WorkingSet64);
        }
        catch
        {
        }

        bool cpuDegraded = appCpuPct >= _options.DegradeCpuThresholdPct;
        bool rssDegraded = rssBytes >= _options.DegradeRssThresholdBytes;
        bool degradeMode = cpuDegraded || rssDegraded;
        RuntimeWarning? currentWarning = _warnings.LastOrDefault(item => item.Seq == _seq);
        string status = BuildStatusSummary(runtimeLoopRunning, warning, currentWarning, cpuDegraded, rssDegraded);
        return new RuntimeHealth
        {
            RuntimeLoopEnabled = _options.RuntimeLoopEnabled,
            RuntimeLoopRunning = _options.RuntimeLoopEnabled && runtimeLoopRunning,
            StartupBlocked = false,
            StatusSummary = status,
            UpdatedAtMs = UnixNowMs(),
            Seq = _seq,
            TickP95Ms = _tickP95.Value,
            SortP95Ms = _sortP95.Value,
            JitterP95Ms = _jitterP95.Value,
            DroppedTicks = _droppedTicks,
            AppCpuPct = appCpuPct,
            AppRssBytes = rssBytes,
            DegradeMode = degradeMode,
            LastWarning = _warnings.LastOrDefault()?.Message,
        };
    }

    private string BuildStatusSummary(
        bool runtimeLoopRunning,
        string? warning,
        RuntimeWarning? currentWarning,
        bool cpuDegraded,
        bool rssDegraded)
    {
        if (!_options.RuntimeLoopEnabled)
        {
            return "Runtime loop disabled.";
        }

        if (_settings.Paused)
        {
            return "Paused.";
        }

        if (!string.IsNullOrWhiteSpace(warning))
        {
            return warning;
        }

        if (currentWarning is not null)
        {
            string category = string.IsNullOrWhiteSpace(currentWarning.Category)
                ? "Runtime"
                : currentWarning.Category.Replace('_', ' ');
            return $"{char.ToUpperInvariant(category[0])}{category[1..]} warning: {currentWarning.Message}";
        }

        if (_settings.AdminModeRequested && !_settings.AdminModeEnabled)
        {
            return "Standard access: admin mode requested but elevation is inactive.";
        }

        if (cpuDegraded && rssDegraded)
        {
            return "Degraded: app CPU and RSS above budget.";
        }

        if (cpuDegraded)
        {
            return "Degraded: app CPU above budget.";
        }

        if (rssDegraded)
        {
            return "Degraded: app RSS above budget.";
        }

        return runtimeLoopRunning ? "Healthy." : "Runtime starting.";
    }

    private double SampleAppCpuPct()
    {
        try
        {
            _currentProcess.Refresh();
            TimeSpan cpu = _currentProcess.TotalProcessorTime;
            long now = Stopwatch.GetTimestamp();
            double elapsedMs = Math.Max(1d, (now - _previousProcessCpuStamp) * 1000d / Stopwatch.Frequency);
            double cpuDeltaMs = Math.Max(0d, (cpu - _previousProcessCpu).TotalMilliseconds);
            _previousProcessCpu = cpu;
            _previousProcessCpuStamp = now;
            return cpuDeltaMs / elapsedMs / Math.Max(1, Environment.ProcessorCount) * 100d;
        }
        catch
        {
            return 0d;
        }
    }

    private void TrackDroppedTicks(double jitterMs)
    {
        double intervalMs = Math.Max(1d, _options.TickInterval.TotalMilliseconds);
        if (jitterMs > intervalMs * 1.5d)
        {
            _droppedTicks += (ulong)(jitterMs / intervalMs);
        }
    }

    private void PublishSnapshotOnly(string? warningCategory = null, string? warningMessage = null)
    {
        _seq++;
        if (!string.IsNullOrWhiteSpace(warningCategory) && !string.IsNullOrWhiteSpace(warningMessage))
        {
            AddWarning(warningCategory, warningMessage);
        }

        _health = BuildHealth(runtimeLoopRunning: !_settings.Paused, tickMs: 0, sortMs: 0, warning: warningMessage);
        RuntimeSnapshot snapshot = BuildSnapshot() with { Health = _health };
        Volatile.Write(ref _snapshot, snapshot);
        Publish(new RuntimeDelta
        {
            Seq = _seq,
            TsMs = snapshot.TsMs,
            Health = _health,
            System = _system,
            Snapshot = snapshot,
        });
    }

    private void Publish(RuntimeDelta delta)
    {
        Channel<RuntimeDelta>[] subscribers;
        lock (_subscriberSync)
        {
            subscribers = [.. _subscribers];
        }

        foreach (Channel<RuntimeDelta> subscriber in subscribers)
        {
            _ = subscriber.Writer.TryWrite(delta);
        }
    }

    private void AddWarning(string category, string message)
    {
        _warnings.Enqueue(new RuntimeWarning
        {
            Seq = _seq,
            TsMs = UnixNowMs(),
            Category = category,
            Message = message,
        });

        while (_warnings.Count > Math.Max(1, _options.MaxWarningCount))
        {
            _warnings.Dequeue();
        }
    }

    private void DrainCollectorWarnings(IProcessCollector collector)
    {
        for (int index = 0; index < Math.Max(1, _options.MaxWarningCount); index++)
        {
            string? warning = collector.TakeWarning();
            if (string.IsNullOrWhiteSpace(warning))
            {
                return;
            }

            AddWarning("collector", warning);
        }
    }

    private void DrainPersistenceWarnings()
    {
        for (int index = 0; index < Math.Max(1, _options.MaxWarningCount); index++)
        {
            string? warning = _persistenceStore.TakeWarning();
            if (string.IsNullOrWhiteSpace(warning))
            {
                return;
            }

            AddWarning("persistence", warning);
        }
    }

    private void ImportWarmCache(WarmCache? cache)
    {
        if (cache?.Rows is not { Count: > 0 })
        {
            return;
        }

        _seq = cache.Seq;
        foreach (ProcessSample row in cache.Rows)
        {
            _allRows[row.Identity()] = row;
        }
    }

    private void QueueSettingsPersistence()
    {
        RuntimeSettings settings = _settings;
        TrackPersistenceTask(Task.Run(async () =>
        {
            try
            {
                await _persistenceStore.SaveSettingsAsync(settings, CancellationToken.None).ConfigureAwait(false);
            }
            catch (Exception ex)
            {
                _logger.LogWarning(ex, "settings_save_failed");
            }
        }, CancellationToken.None));
    }

    private void QueueWarmCachePersistence(ulong seq, IEnumerable<ProcessSample> rows)
    {
        WarmCache cache = new()
        {
            Seq = seq,
            Rows = Freeze(rows),
        };

        TrackPersistenceTask(Task.Run(async () =>
        {
            try
            {
                await _persistenceStore.SaveWarmCacheAsync(cache, CancellationToken.None).ConfigureAwait(false);
            }
            catch (Exception ex)
            {
                _logger.LogWarning(ex, "warm_cache_save_failed");
            }
        }));
    }

    private void TrackPersistenceTask(Task task)
    {
        lock (_persistenceTaskSync)
        {
            _persistenceTasks.Add(task);
        }

        _ = task.ContinueWith(
            completed =>
            {
                lock (_persistenceTaskSync)
                {
                    _persistenceTasks.Remove(completed);
                }
            },
            CancellationToken.None,
            TaskContinuationOptions.ExecuteSynchronously,
            TaskScheduler.Default);
    }

    private async Task FlushPersistenceTasksAsync()
    {
        Task[] tasks;
        lock (_persistenceTaskSync)
        {
            tasks = [.. _persistenceTasks];
        }

        if (tasks.Length > 0)
        {
            await Task.WhenAll(tasks).ConfigureAwait(false);
        }
    }

    private static RuntimeSettings NormalizeSettings(RuntimeSettings settings)
    {
        return settings with
        {
            Query = NormalizeQuery(settings.Query),
            AdminModeEnabled = false,
            MetricWindowSeconds = NormalizeMetricWindow(settings.MetricWindowSeconds),
        };
    }

    private static RuntimeQuery NormalizeQuery(RuntimeQuery query)
    {
        return query with
        {
            FilterText = query.FilterText.Trim(),
            Limit = Math.Clamp(query.Limit, 1, 20_000),
        };
    }

    private static int NormalizeMetricWindow(int seconds) => seconds <= 60 ? 60 : 120;

    private static ulong UnixNowMs() => (ulong)Math.Max(0L, DateTimeOffset.UtcNow.ToUnixTimeMilliseconds());

    private static IReadOnlyList<T> Freeze<T>(IEnumerable<T> values) => Array.AsReadOnly(values.ToArray());

    private static bool HasMeaningfulProcessChange(ProcessSample previous, ProcessSample current)
    {
        return previous.ParentPid != current.ParentPid
               || !string.Equals(previous.Name, current.Name, StringComparison.Ordinal)
               || !previous.CpuPct.Equals(current.CpuPct)
               || previous.MemoryBytes != current.MemoryBytes
               || previous.PrivateBytes != current.PrivateBytes
               || previous.DiskBps != current.DiskBps
               || previous.OtherIoBps != current.OtherIoBps
               || previous.Threads != current.Threads
               || previous.Handles != current.Handles
               || previous.AccessState != current.AccessState;
    }

    private static void DisposeCollector(IProcessCollector collector)
    {
        if (collector is IDisposable disposable)
        {
            disposable.Dispose();
        }
    }

    private sealed record CommandEnvelope(RuntimeCommand Command, TaskCompletionSource Completion);
}

internal sealed class P95Window
{
    private readonly double[] _samples;
    private int _next;
    private int _count;

    public P95Window(int capacity)
    {
        _samples = new double[Math.Max(1, capacity)];
    }

    public double Value { get; private set; }

    public void Add(double sample)
    {
        _samples[_next] = Math.Max(0d, sample);
        _next = (_next + 1) % _samples.Length;
        _count = Math.Min(_count + 1, _samples.Length);
        Value = Calculate();
    }

    private double Calculate()
    {
        if (_count == 0)
        {
            return 0d;
        }

        double[] scratch = new double[_count];
        Array.Copy(_samples, scratch, _count);
        Array.Sort(scratch);
        int index = (int)Math.Ceiling(_count * 0.95d) - 1;
        return scratch[Math.Clamp(index, 0, scratch.Length - 1)];
    }
}
