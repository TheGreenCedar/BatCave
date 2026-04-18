using BatCave.Core.Abstractions;
using BatCave.Core.Domain;
using BatCave.Core.Pipeline;
using BatCave.Core.Runtime;
using BatCave.Core.Sort;
using BatCave.Core.State;
using BatCave.Core.Tests.Runtime.TestSupport;
using System.Diagnostics;

namespace BatCave.Core.Tests.Runtime;

public class MonitoringRuntimeTests
{
    [Fact]
    public void Tick_WhenPersistenceWarningQueued_SurfacesWarningAndIncrementsCounter()
    {
        using MonitoringRuntime runtime = CreateRuntime(out _, out TestPersistenceStore persistenceStore);

        persistenceStore.EnqueueWarning("persistence_load_json_failed path=settings.json error=JsonException: invalid json");

        TickOutcome outcome = runtime.Tick(jitterMs: 0);

        Assert.NotNull(outcome.Warning);
        Assert.Contains("persistence_load_json_failed", outcome.Warning!.Message, StringComparison.OrdinalIgnoreCase);
        Assert.Equal(1UL, outcome.Health.CollectorWarnings);
    }

    [Fact]
    public void SetFilter_WhenPersistenceSaveFails_QueuesWarningForNextTick()
    {
        using MonitoringRuntime runtime = CreateRuntime(
            out _,
            out _,
            configurePersistenceStore: store => store.FailSaveSettings = true);

        runtime.SetFilter("svc");
        TickOutcome outcome = TickUntilWarning(runtime);

        Assert.NotNull(outcome.Warning);
        Assert.Contains("persistence_save_settings_failed", outcome.Warning!.Message, StringComparison.OrdinalIgnoreCase);
    }

    [Fact]
    public async Task SetMetricTrendWindowSeconds_PersistsNormalizedWindow()
    {
        BlockingSettingsPersistenceStore persistenceStore = new(blockFirstSave: true);
        MonitoringRuntime runtime = CreateRuntime(new TestCollector(), persistenceStore);

        try
        {
            runtime.SetMetricTrendWindowSeconds(120);
            await persistenceStore.WaitForFirstSaveStartedAsync();

            runtime.SetMetricTrendWindowSeconds(75);
            Assert.Equal(60, runtime.CurrentMetricTrendWindowSeconds);

            persistenceStore.ReleaseFirstSave();
        }
        finally
        {
            runtime.Dispose();
        }

        IReadOnlyList<UserSettings> saves = persistenceStore.GetSavedSettingsSnapshot();
        Assert.Equal(2, saves.Count);
        Assert.Equal(120, saves[0].MetricTrendWindowSeconds);
        Assert.Equal(60, saves[1].MetricTrendWindowSeconds);
    }

    [Fact]
    public void Constructor_WhenPersistedMetricTrendWindowIsInvalid_NormalizesToDefault()
    {
        PreloadedSettingsPersistenceStore persistenceStore = new(new UserSettings
        {
            MetricTrendWindowSeconds = 7,
            AdminPreferenceInitialized = true,
        });

        using (MonitoringRuntime runtime = CreateRuntime(new TestCollector(), persistenceStore))
        {
            Assert.Equal(60, runtime.CurrentMetricTrendWindowSeconds);
        }

        Assert.Equal(1, persistenceStore.SettingsSaveCount);
        Assert.Equal(60, persistenceStore.CurrentSettings.MetricTrendWindowSeconds);
    }

    [Fact]
    public void Constructor_WhenPersistedSortColumnIsInvalid_FailsFast()
    {
        PreloadedSettingsPersistenceStore persistenceStore = new(new UserSettings
        {
            SortCol = (SortColumn)987,
            AdminPreferenceInitialized = true,
        });

        InvalidOperationException exception = Assert.Throws<InvalidOperationException>(
            () => CreateRuntime(new TestCollector(), persistenceStore));

        Assert.Contains("invalid sort column", exception.Message, StringComparison.OrdinalIgnoreCase);
    }

    [Fact]
    public void Constructor_WhenPersistedSortColumnIsHidden_NormalizesToCpuPctPreservingDirectionAndPersistsOnce()
    {
        PreloadedSettingsPersistenceStore persistenceStore = new(new UserSettings
        {
            SortCol = SortColumn.Handles,
            SortDir = SortDirection.Asc,
            AdminPreferenceInitialized = true,
            MetricTrendWindowSeconds = 60,
        });

        using (MonitoringRuntime runtime = CreateRuntime(new TestCollector(), persistenceStore))
        {
            Assert.Equal(SortColumn.CpuPct, runtime.CurrentSortColumn);
            Assert.Equal(SortDirection.Asc, runtime.CurrentSortDirection);
        }

        Assert.Equal(SortColumn.CpuPct, persistenceStore.CurrentSettings.SortCol);
        Assert.Equal(SortDirection.Asc, persistenceStore.CurrentSettings.SortDir);
        Assert.Equal(1, persistenceStore.SettingsSaveCount);
    }

    [Fact]
    public void Constructor_WhenAdminPreferenceUninitialized_DefaultsAdminPreferenceOn()
    {
        PreloadedSettingsPersistenceStore persistenceStore = new(new UserSettings
        {
            AdminMode = false,
            AdminPreferenceInitialized = false,
        });

        using (MonitoringRuntime runtime = CreateRuntime(new TestCollector(), persistenceStore))
        {
            Assert.True(runtime.IsAdminMode());
        }

        Assert.True(persistenceStore.CurrentSettings.AdminMode);
        Assert.True(persistenceStore.CurrentSettings.AdminPreferenceInitialized);
        Assert.Equal(1, persistenceStore.SettingsSaveCount);
    }

    [Fact]
    public void Constructor_WhenAdminPreferenceInitializedExplicitOff_RemainsOff()
    {
        PreloadedSettingsPersistenceStore persistenceStore = new(new UserSettings
        {
            AdminMode = false,
            AdminPreferenceInitialized = true,
        });

        using MonitoringRuntime runtime = CreateRuntime(new TestCollector(), persistenceStore);

        Assert.False(runtime.IsAdminMode());
        Assert.False(persistenceStore.CurrentSettings.AdminMode);
        Assert.True(persistenceStore.CurrentSettings.AdminPreferenceInitialized);
        Assert.Equal(0, persistenceStore.SettingsSaveCount);
    }

    [Fact]
    public void Tick_PrioritizesCollectorWarningBeforePersistenceWarning()
    {
        using MonitoringRuntime runtime = CreateRuntime(out TestCollector collector, out TestPersistenceStore persistenceStore);
        persistenceStore.EnqueueWarning("persistence_warning");
        collector.EnqueueCollectorWarning("collector_warning");

        TickOutcome first = runtime.Tick(jitterMs: 0);
        TickOutcome second = runtime.Tick(jitterMs: 0);

        Assert.NotNull(first.Warning);
        Assert.Contains("collector_warning", first.Warning!.Message, StringComparison.OrdinalIgnoreCase);
        Assert.NotNull(second.Warning);
        Assert.Contains("persistence_warning", second.Warning!.Message, StringComparison.OrdinalIgnoreCase);
    }

    [Fact]
    public async Task SetFilterAndSort_WhenSaveIsInFlight_CoalescesToLatestPendingSettings()
    {
        BlockingSettingsPersistenceStore persistenceStore = new(blockFirstSave: true);
        MonitoringRuntime runtime = CreateRuntime(new TestCollector(), persistenceStore);

        try
        {
            runtime.SetFilter("first");
            await persistenceStore.WaitForFirstSaveStartedAsync();

            runtime.SetFilter("second");
            runtime.SetSort(SortColumn.Name, SortDirection.Asc);
            runtime.SetFilter("final");

            persistenceStore.ReleaseFirstSave();
        }
        finally
        {
            runtime.Dispose();
        }

        IReadOnlyList<UserSettings> saves = persistenceStore.GetSavedSettingsSnapshot();
        Assert.Equal(2, saves.Count);
        Assert.Equal("first", saves[0].FilterText);
        Assert.Equal("final", saves[1].FilterText);
        Assert.Equal(SortColumn.Name, saves[1].SortCol);
        Assert.Equal(SortDirection.Asc, saves[1].SortDir);
    }

    [Fact]
    public async Task Dispose_WhenSettingsSaveInFlight_WaitsForFlush()
    {
        BlockingSettingsPersistenceStore persistenceStore = new(blockFirstSave: true);
        MonitoringRuntime runtime = CreateRuntime(new TestCollector(), persistenceStore);

        runtime.SetFilter("flush-on-dispose");
        await persistenceStore.WaitForFirstSaveStartedAsync();

        Task disposeTask = Task.Run(runtime.Dispose);
        Task completed = await Task.WhenAny(disposeTask, Task.Delay(100));
        Assert.NotSame(disposeTask, completed);

        persistenceStore.ReleaseFirstSave();
        await disposeTask.WaitAsync(TimeSpan.FromSeconds(5));

        IReadOnlyList<UserSettings> saves = persistenceStore.GetSavedSettingsSnapshot();
        Assert.Single(saves);
        Assert.Equal("flush-on-dispose", saves[0].FilterText);
    }

    [Fact]
    public async Task RestartAsync_WhenSettingsSaveInFlight_WaitsForSettingsFlush()
    {
        BlockingSettingsPersistenceStore persistenceStore = new(blockFirstSave: true);
        using MonitoringRuntime runtime = CreateRuntime(new TestCollector(), persistenceStore);

        runtime.SetFilter("keep-filter");
        await persistenceStore.WaitForFirstSaveStartedAsync();

        Task<CollectorActivationResult> restartTask = runtime.RestartAsync(adminMode: true, CancellationToken.None);
        Task completed = await Task.WhenAny(restartTask, Task.Delay(100));
        Assert.NotSame(restartTask, completed);

        persistenceStore.ReleaseFirstSave();
        await restartTask.WaitAsync(TimeSpan.FromSeconds(5));

        IReadOnlyList<UserSettings> saves = persistenceStore.GetSavedSettingsSnapshot();
        Assert.Equal(2, saves.Count);
        Assert.Equal("keep-filter", saves[1].FilterText);
        Assert.True(saves[1].AdminMode);
        Assert.True(runtime.IsAdminMode());
    }

    [Fact]
    public async Task InitializeAsync_WhenPersistedAdminModeCollectorFails_FallsBackToNonAdmin()
    {
        FailingAdminCollectorFactory collectorFactory = new();
        PreloadedSettingsPersistenceStore persistenceStore = new(new UserSettings
        {
            AdminMode = true,
            AdminPreferenceInitialized = true,
        });

        using MonitoringRuntime runtime = CreateRuntime(collectorFactory, persistenceStore);

        CollectorActivationResult activation = await runtime.InitializeAsync(CancellationToken.None);
        TickOutcome outcome = runtime.Tick(jitterMs: 0);

        Assert.Equal([true], collectorFactory.RequestedModes);
        Assert.False(runtime.IsAdminMode());
        Assert.False(activation.EffectiveAdminMode);
        Assert.True(persistenceStore.CurrentSettings.AdminMode);
        Assert.Equal(0, persistenceStore.SettingsSaveCount);
        Assert.NotNull(activation.Warning);
        Assert.NotNull(outcome.Warning);
        Assert.Contains("admin_mode_start_failed", outcome.Warning!.Message, StringComparison.OrdinalIgnoreCase);
    }

    [Fact]
    public async Task RestartAsync_WhenAdminCollectorFails_FallsBackToNonAdminWithoutThrowing()
    {
        FailingAdminCollectorFactory collectorFactory = new();
        PreloadedSettingsPersistenceStore persistenceStore = new(new UserSettings
        {
            AdminMode = false,
            AdminPreferenceInitialized = true,
        });

        using MonitoringRuntime runtime = CreateRuntime(collectorFactory, persistenceStore);

        await runtime.InitializeAsync(CancellationToken.None);
        CollectorActivationResult activation = await runtime.RestartAsync(adminMode: true, CancellationToken.None);
        TickOutcome outcome = runtime.Tick(jitterMs: 0);

        Assert.Equal([false, true], collectorFactory.RequestedModes);
        Assert.False(runtime.IsAdminMode());
        Assert.True(persistenceStore.CurrentSettings.AdminMode);
        Assert.NotNull(activation.Warning);
        Assert.Contains("admin_mode_start_failed", activation.Warning!, StringComparison.OrdinalIgnoreCase);
        Assert.Null(outcome.Warning);
    }

    [Fact]
    public async Task Tick_WhenWarmCacheSaveIsInFlight_DoesNotBlockTickPath()
    {
        BlockingWarmCachePersistenceStore persistenceStore = new();
        using MonitoringRuntime runtime = CreateRuntime(new TestCollector(), persistenceStore);

        for (int index = 0; index < 5; index++)
        {
            runtime.Tick(jitterMs: 0);
        }

        await persistenceStore.WaitForWarmCacheSaveStartedAsync();

        Stopwatch stopwatch = Stopwatch.StartNew();
        runtime.Tick(jitterMs: 0);
        stopwatch.Stop();

        Assert.True(stopwatch.ElapsedMilliseconds < 100, $"Expected non-blocking tick, observed {stopwatch.ElapsedMilliseconds}ms.");

        persistenceStore.ReleaseWarmCacheSave();
    }

    [Fact]
    public void Tick_TracksStreamingJitterP95AcrossRollingWindow()
    {
        using MonitoringRuntime runtime = CreateRuntime(new TestCollector(), new TestPersistenceStore());

        runtime.Tick(jitterMs: -1);
        runtime.Tick(jitterMs: 2);
        runtime.Tick(jitterMs: 3);
        TickOutcome spiky = runtime.Tick(jitterMs: 120);
        Assert.Equal(120d, spiky.Health.JitterP95Ms);

        for (int index = 0; index < 120; index++)
        {
            runtime.Tick(jitterMs: 2);
        }

        RuntimeHealth health = runtime.GetRuntimeHealth();
        Assert.Equal(2d, health.JitterP95Ms);
    }

    [Fact]
    public void Tick_AfterCompaction_RehydratesCompactedLiveRowsOnNextTick()
    {
        StickyCompactionCollector collector = new();
        using MonitoringRuntime runtime = CreateRuntime(collector, new TestPersistenceStore());
        runtime.SetSort(SortColumn.Pid, SortDirection.Asc);

        runtime.Tick(jitterMs: 0);
        runtime.Tick(jitterMs: 0);
        runtime.Tick(jitterMs: 0);

        QueryResponse compactedSnapshot = runtime.GetSnapshot();
        Assert.Equal(5000, compactedSnapshot.Total);
        Assert.DoesNotContain(compactedSnapshot.Rows, row => row.Identity() == collector.ColdRowIdentity);

        runtime.Tick(jitterMs: 0);

        QueryResponse restoredSnapshot = runtime.GetSnapshot();
        Assert.Equal(5001, restoredSnapshot.Total);
        Assert.Contains(restoredSnapshot.Rows, row => row.Identity() == collector.ColdRowIdentity);
    }

    [Fact]
    public void Tick_WhenFirstLiveCollectionFailsAfterWarmCache_KeepsWarmCacheRowsVisible()
    {
        ProcessSample warmRow = new()
        {
            Pid = 910,
            Seq = 4,
            TsMs = 4,
            ParentPid = 1,
            StartTimeMs = 9_100,
            Name = "warm-cache-proc",
            CpuPct = 22,
            RssBytes = 4 * 1024UL,
            PrivateBytes = 2 * 1024UL,
            IoReadBps = 10,
            IoWriteBps = 11,
            OtherIoBps = 12,
            Threads = 3,
            Handles = 5,
            AccessState = AccessState.Full,
        };

        WarmCachePersistenceStore persistenceStore = new(new WarmCache
        {
            Seq = 4,
            Rows = [warmRow],
        });
        SnapshotFailureCollector collector = new();
        using MonitoringRuntime runtime = CreateRuntime(collector, persistenceStore);

        QueryResponse beforeTick = runtime.GetSnapshot();
        TickOutcome outcome = runtime.Tick(jitterMs: 0);
        QueryResponse afterTick = runtime.GetSnapshot();

        Assert.Single(beforeTick.Rows);
        Assert.NotNull(outcome.Warning);
        Assert.Contains("failed to create process snapshot", outcome.Warning!.Message, StringComparison.OrdinalIgnoreCase);
        ProcessSample retained = Assert.Single(afterTick.Rows);
        Assert.Equal(warmRow.Identity(), retained.Identity());
    }

    private static MonitoringRuntime CreateRuntime(
        out TestCollector collector,
        out TestPersistenceStore persistenceStore,
        Action<TestPersistenceStore>? configurePersistenceStore = null,
        Action<TestCollector>? configureCollector = null)
    {
        persistenceStore = new TestPersistenceStore();
        collector = new TestCollector();
        configurePersistenceStore?.Invoke(persistenceStore);
        configureCollector?.Invoke(collector);
        return RuntimeTestHarness.CreateRuntime(collector, persistenceStore);
    }

    private static MonitoringRuntime CreateRuntime(IProcessCollector collector, IPersistenceStore persistenceStore)
    {
        return CreateRuntime(collector, persistenceStore, new RuntimeHostOptions());
    }

    private static MonitoringRuntime CreateRuntime(
        IProcessCollector collector,
        IPersistenceStore persistenceStore,
        RuntimeHostOptions runtimeHostOptions)
    {
        MonitoringRuntime runtime = new(
            new DelegatingCollectorFactory(collector),
            new DeltaTelemetryPipeline(),
            new InMemoryStateStore(),
            new IncrementalSortIndexEngine(),
            persistenceStore,
            runtimeHostOptions);
        runtime.InitializeAsync(CancellationToken.None).GetAwaiter().GetResult();
        return runtime;
    }

    private static MonitoringRuntime CreateRuntime(IProcessCollectorFactory collectorFactory, IPersistenceStore persistenceStore)
    {
        return new MonitoringRuntime(
            collectorFactory,
            new DeltaTelemetryPipeline(),
            new InMemoryStateStore(),
            new IncrementalSortIndexEngine(),
            persistenceStore,
            new RuntimeHostOptions());
    }

    private static TickOutcome TickUntilWarning(MonitoringRuntime runtime, int maxAttempts = 10)
    {
        TickOutcome outcome = runtime.Tick(jitterMs: 0);
        for (int attempt = 0; attempt < maxAttempts && outcome.Warning is null; attempt++)
        {
            Thread.Sleep(10);
            outcome = runtime.Tick(jitterMs: 0);
        }

        return outcome;
    }

    private sealed class TestCollector : IProcessCollector
    {
        private readonly Queue<string> _warnings = [];

        public IReadOnlyList<ProcessSample> CollectTick(ulong seq)
        {
            return [];
        }

        public string? TakeWarning()
        {
            return _warnings.Count > 0 ? _warnings.Dequeue() : null;
        }

        public void EnqueueCollectorWarning(string warning)
        {
            _warnings.Enqueue(warning);
        }
    }

    private sealed class BlockingSettingsPersistenceStore : IPersistenceStore
    {
        private readonly object _sync = new();
        private readonly Queue<string> _warnings = [];
        private readonly List<UserSettings> _savedSettings = [];
        private readonly TaskCompletionSource<bool> _firstSaveStarted = new(TaskCreationOptions.RunContinuationsAsynchronously);
        private readonly TaskCompletionSource<bool> _releaseFirstSave = new(TaskCreationOptions.RunContinuationsAsynchronously);
        private readonly bool _blockFirstSave;
        private int _saveSettingsCount;

        public BlockingSettingsPersistenceStore(bool blockFirstSave)
        {
            _blockFirstSave = blockFirstSave;
            if (!blockFirstSave)
            {
                _releaseFirstSave.TrySetResult(true);
            }
        }

        public string BaseDirectory => Path.GetTempPath();

        public UserSettings? LoadSettings()
        {
            return new UserSettings
            {
                AdminPreferenceInitialized = true,
            };
        }

        public async Task SaveSettingsAsync(UserSettings settings, CancellationToken ct)
        {
            int saveIndex = Interlocked.Increment(ref _saveSettingsCount);
            _firstSaveStarted.TrySetResult(true);

            if (_blockFirstSave && saveIndex == 1)
            {
                await _releaseFirstSave.Task.WaitAsync(ct);
            }

            lock (_sync)
            {
                _savedSettings.Add(settings);
            }
        }

        public WarmCache? LoadWarmCache()
        {
            return null;
        }

        public Task SaveWarmCacheAsync(WarmCache cache, CancellationToken ct)
        {
            return Task.CompletedTask;
        }

        public Task AppendDiagnosticAsync(string category, object payload, CancellationToken ct)
        {
            return Task.CompletedTask;
        }

        public string? TakeWarning()
        {
            lock (_sync)
            {
                return _warnings.Count > 0 ? _warnings.Dequeue() : null;
            }
        }

        public Task WaitForFirstSaveStartedAsync()
        {
            return _firstSaveStarted.Task;
        }

        public void ReleaseFirstSave()
        {
            _releaseFirstSave.TrySetResult(true);
        }

        public IReadOnlyList<UserSettings> GetSavedSettingsSnapshot()
        {
            lock (_sync)
            {
                return _savedSettings.ToArray();
            }
        }
    }

    private sealed class BlockingWarmCachePersistenceStore : IPersistenceStore
    {
        private readonly Queue<string> _warnings = [];
        private readonly TaskCompletionSource<bool> _warmCacheSaveStarted = new(TaskCreationOptions.RunContinuationsAsynchronously);
        private readonly TaskCompletionSource<bool> _releaseWarmCacheSave = new(TaskCreationOptions.RunContinuationsAsynchronously);
        private int _warmCacheSaveCount;

        public string BaseDirectory => Path.GetTempPath();

        public UserSettings? LoadSettings()
        {
            return new UserSettings();
        }

        public Task SaveSettingsAsync(UserSettings settings, CancellationToken ct)
        {
            return Task.CompletedTask;
        }

        public WarmCache? LoadWarmCache()
        {
            return null;
        }

        public async Task SaveWarmCacheAsync(WarmCache cache, CancellationToken ct)
        {
            int invocation = Interlocked.Increment(ref _warmCacheSaveCount);
            if (invocation == 1)
            {
                _warmCacheSaveStarted.TrySetResult(true);
                await _releaseWarmCacheSave.Task.WaitAsync(ct);
            }
        }

        public Task AppendDiagnosticAsync(string category, object payload, CancellationToken ct)
        {
            return Task.CompletedTask;
        }

        public string? TakeWarning()
        {
            return _warnings.TryDequeue(out string? warning) ? warning : null;
        }

        public Task WaitForWarmCacheSaveStartedAsync()
        {
            return _warmCacheSaveStarted.Task;
        }

        public void ReleaseWarmCacheSave()
        {
            _releaseWarmCacheSave.TrySetResult(true);
        }
    }

    private sealed class FailingAdminCollectorFactory : IProcessCollectorFactory
    {
        public List<bool> RequestedModes { get; } = [];

        public ValueTask<CollectorActivationResult> CreateAsync(bool adminMode, CancellationToken ct)
        {
            RequestedModes.Add(adminMode);
            if (adminMode)
            {
                return ValueTask.FromResult(new CollectorActivationResult(
                    new TestCollector(),
                    EffectiveAdminMode: false,
                    Warning: "admin_mode_start_failed requested_admin_mode=true fallback_admin_mode=false error=InvalidOperationException: elevation failed"));
            }

            return ValueTask.FromResult(new CollectorActivationResult(new TestCollector(), EffectiveAdminMode: false, Warning: null));
        }
    }

    private sealed class NullSettingsPersistenceStore : IPersistenceStore
    {
        public string BaseDirectory => Path.GetTempPath();

        public UserSettings? LoadSettings()
        {
            return null;
        }

        public Task SaveSettingsAsync(UserSettings settings, CancellationToken ct)
        {
            return Task.CompletedTask;
        }

        public WarmCache? LoadWarmCache()
        {
            return null;
        }

        public Task SaveWarmCacheAsync(WarmCache cache, CancellationToken ct)
        {
            return Task.CompletedTask;
        }

        public Task AppendDiagnosticAsync(string category, object payload, CancellationToken ct)
        {
            return Task.CompletedTask;
        }

        public string? TakeWarning()
        {
            return null;
        }
    }

    private sealed class StickyCompactionCollector : IProcessCollector
    {
        private readonly IReadOnlyList<ProcessSample> _rows;

        public StickyCompactionCollector()
        {
            List<ProcessSample> rows = new(capacity: 5001);
            ProcessSample coldRow = new()
            {
                Pid = 1,
                Seq = 0,
                TsMs = 0,
                ParentPid = 0,
                StartTimeMs = 10,
                Name = "cold-proc",
                CpuPct = 0,
                RssBytes = 0,
                PrivateBytes = 0,
                IoReadBps = 0,
                IoWriteBps = 0,
                OtherIoBps = 0,
                Threads = 1,
                Handles = 1,
                AccessState = AccessState.Full,
            };
            rows.Add(coldRow);
            ColdRowIdentity = coldRow.Identity();

            uint selfPid = (uint)Environment.ProcessId;
            rows.Add(new ProcessSample
            {
                Pid = selfPid,
                Seq = 0,
                TsMs = 0,
                ParentPid = 0,
                StartTimeMs = 20,
                Name = "batcave-self",
                CpuPct = 15,
                RssBytes = 1024,
                PrivateBytes = 512,
                IoReadBps = 32,
                IoWriteBps = 16,
                OtherIoBps = 8,
                Threads = 4,
                Handles = 8,
                AccessState = AccessState.Full,
            });

            for (uint index = 0; index < 4_999; index++)
            {
                uint pid = 10_000 + index;
                rows.Add(new ProcessSample
                {
                    Pid = pid,
                    Seq = 0,
                    TsMs = 0,
                    ParentPid = 0,
                    StartTimeMs = 100 + pid,
                    Name = $"hot-{pid}",
                    CpuPct = 25,
                    RssBytes = 4 * 1024UL * 1024UL,
                    PrivateBytes = 2 * 1024UL * 1024UL,
                    IoReadBps = 2_000,
                    IoWriteBps = 1_000,
                    OtherIoBps = 500,
                    Threads = 3,
                    Handles = 6,
                    AccessState = AccessState.Full,
                });
            }

            _rows = rows;
        }

        public ProcessIdentity ColdRowIdentity { get; }

        public IReadOnlyList<ProcessSample> CollectTick(ulong seq)
        {
            return
            [
                .. _rows.Select(row => row with
                {
                    Seq = seq,
                    TsMs = seq,
                }),
            ];
        }

        public string? TakeWarning()
        {
            return null;
        }
    }

    private sealed class DelegatingCollectorFactory : IProcessCollectorFactory
    {
        private readonly IProcessCollector _collector;

        public DelegatingCollectorFactory(IProcessCollector collector)
        {
            _collector = collector;
        }

        public ValueTask<CollectorActivationResult> CreateAsync(bool _, CancellationToken ct)
        {
            return ValueTask.FromResult(new CollectorActivationResult(_collector, EffectiveAdminMode: _, Warning: null));
        }
    }

    private sealed class SnapshotFailureCollector : IProcessCollector
    {
        private bool _warningAvailable = true;

        public IReadOnlyList<ProcessSample> CollectTick(ulong seq)
        {
            return [];
        }

        public string? TakeWarning()
        {
            if (!_warningAvailable)
            {
                return null;
            }

            _warningAvailable = false;
            return "failed to create process snapshot";
        }
    }

    private sealed class WarmCachePersistenceStore : IPersistenceStore
    {
        private readonly WarmCache _warmCache;

        public WarmCachePersistenceStore(WarmCache warmCache)
        {
            _warmCache = warmCache;
        }

        public string BaseDirectory => Path.GetTempPath();

        public UserSettings? LoadSettings()
        {
            return new UserSettings
            {
                AdminPreferenceInitialized = true,
            };
        }

        public Task SaveSettingsAsync(UserSettings settings, CancellationToken ct)
        {
            return Task.CompletedTask;
        }

        public WarmCache? LoadWarmCache()
        {
            return _warmCache;
        }

        public Task SaveWarmCacheAsync(WarmCache cache, CancellationToken ct)
        {
            return Task.CompletedTask;
        }

        public Task AppendDiagnosticAsync(string category, object payload, CancellationToken ct)
        {
            return Task.CompletedTask;
        }

        public string? TakeWarning()
        {
            return null;
        }
    }

    private sealed class PreloadedSettingsPersistenceStore : IPersistenceStore
    {
        private readonly Queue<string> _warnings = [];
        private UserSettings _settings;
        private int _settingsSaveCount;

        public PreloadedSettingsPersistenceStore(UserSettings settings)
        {
            _settings = settings;
        }

        public string BaseDirectory => Path.GetTempPath();

        public int SettingsSaveCount => _settingsSaveCount;

        public UserSettings CurrentSettings => _settings;

        public UserSettings? LoadSettings()
        {
            return _settings;
        }

        public Task SaveSettingsAsync(UserSettings settings, CancellationToken ct)
        {
            _settings = settings;
            _settingsSaveCount++;
            return Task.CompletedTask;
        }

        public WarmCache? LoadWarmCache()
        {
            return null;
        }

        public Task SaveWarmCacheAsync(WarmCache cache, CancellationToken ct)
        {
            return Task.CompletedTask;
        }

        public Task AppendDiagnosticAsync(string category, object payload, CancellationToken ct)
        {
            return Task.CompletedTask;
        }

        public string? TakeWarning()
        {
            return _warnings.Count > 0 ? _warnings.Dequeue() : null;
        }
    }
}










