using BatCave.Runtime.Collectors;
using BatCave.Runtime.Benchmarking;
using BatCave.Runtime.Contracts;
using BatCave.Runtime.Persistence;
using BatCave.Runtime.Presentation;
using BatCave.Runtime.Serialization;
using BatCave.Runtime.Store;
using System.Text.Json;

namespace BatCave.Runtime.Tests;

public sealed class RuntimeContractTests
{
    [Fact]
    public async Task RuntimeStore_AppliesQueryCommandsAndPublishesOrderedDeltas()
    {
        MemoryPersistenceStore persistence = new();
        await using RuntimeStore store = CreateStore(persistence, SubscriberBufferCapacity: 4);
        using CancellationTokenSource timeout = new(TimeSpan.FromSeconds(5));
        await store.StartAsync(timeout.Token);

        await using IAsyncEnumerator<RuntimeDelta> subscription = store.SubscribeAsync(timeout.Token).GetAsyncEnumerator(timeout.Token);
        RuntimeDelta delta = await ReadNextAfterAsync(
            subscription,
            () => store.ExecuteAsync(new RefreshNowCommand(), timeout.Token),
            timeout.Token);

        Assert.Equal(RuntimeEventKinds.Delta, delta.EventKind);
        Assert.True(delta.Seq > 0);
        Assert.NotEmpty(delta.Snapshot.Rows);
        Assert.Equal(delta.Seq, delta.Snapshot.Seq);

        await store.ExecuteAsync(
            new SetProcessQueryCommand(new RuntimeQuery
            {
                FilterText = "code",
                SortColumn = SortColumn.Name,
                SortDirection = SortDirection.Asc,
                Limit = 10,
            }),
            timeout.Token);

        RuntimeSnapshot filtered = store.GetSnapshot();
        ProcessSample row = Assert.Single(filtered.Rows);
        Assert.Equal("Code", row.Name);
        Assert.Equal("code", filtered.Settings.Query.FilterText);
        Assert.Single(persistence.SavedSettings);
    }

    [Fact]
    public async Task RuntimeStore_SnapshotsExposeReadOnlyCopies()
    {
        await using RuntimeStore store = CreateStore(new MemoryPersistenceStore(), SubscriberBufferCapacity: 4);
        using CancellationTokenSource timeout = new(TimeSpan.FromSeconds(5));
        await store.StartAsync(timeout.Token);
        await store.ExecuteAsync(new RefreshNowCommand(), timeout.Token);

        RuntimeSnapshot snapshot = store.GetSnapshot();
        IList<ProcessSample> rows = Assert.IsAssignableFrom<IList<ProcessSample>>(snapshot.Rows);

        Assert.True(rows.IsReadOnly);
        Assert.Throws<NotSupportedException>(() => rows[0] = Sample(999, "Injected", 999));
        Assert.DoesNotContain(store.GetSnapshot().Rows, row => row.Pid == 999);
    }

    [Fact]
    public async Task RuntimeStore_BoundedSubscribersKeepLatestUsefulDelta()
    {
        await using RuntimeStore store = CreateStore(new MemoryPersistenceStore(), SubscriberBufferCapacity: 1);
        using CancellationTokenSource timeout = new(TimeSpan.FromSeconds(5));
        await store.StartAsync(timeout.Token);

        await using IAsyncEnumerator<RuntimeDelta> subscription = store.SubscribeAsync(timeout.Token).GetAsyncEnumerator(timeout.Token);
        _ = await ReadNextAfterAsync(
            subscription,
            () => store.ExecuteAsync(new RefreshNowCommand(), timeout.Token),
            timeout.Token);

        for (int index = 0; index < 6; index++)
        {
            await store.ExecuteAsync(new SetAdminModeCommand(index % 2 == 0), timeout.Token);
        }

        RuntimeDelta latest = await ReadNextAsync(subscription, timeout.Token);
        Assert.Equal(store.GetSnapshot().Settings.AdminModeEnabled, latest.Snapshot.Settings.AdminModeEnabled);
        Assert.Equal(store.GetSnapshot().Warnings.Last().Message, latest.Snapshot.Warnings.Last().Message);
    }

    [Fact]
    public void LocalJsonPersistence_RecoversFromCorruptFiles()
    {
        string directory = Path.Combine(Path.GetTempPath(), "BatCaveRuntimeTests", Guid.NewGuid().ToString("N"));
        try
        {
            Directory.CreateDirectory(directory);
            File.WriteAllText(Path.Combine(directory, "settings.json"), "{not-json");
            File.WriteAllText(Path.Combine(directory, "warm-cache.json"), "{not-json");

            LocalJsonRuntimePersistenceStore persistence = new(directory);

            Assert.Null(persistence.LoadSettings());
            Assert.Null(persistence.LoadWarmCache());
        }
        finally
        {
            if (Directory.Exists(directory))
            {
                Directory.Delete(directory, recursive: true);
            }
        }
    }

    [Fact]
    public async Task LocalJsonPersistence_ConcurrentWritesUseIndependentTempFiles()
    {
        string directory = Path.Combine(Path.GetTempPath(), "BatCaveRuntimeTests", Guid.NewGuid().ToString("N"));
        try
        {
            LocalJsonRuntimePersistenceStore persistence = new(directory);
            Task[] writes = Enumerable.Range(0, 12)
                .Select(index => persistence.SaveSettingsAsync(new RuntimeSettings
                {
                    Query = new RuntimeQuery { FilterText = $"filter-{index}" },
                }, CancellationToken.None))
                .ToArray();

            await Task.WhenAll(writes);

            Assert.NotNull(persistence.LoadSettings());
            Assert.Empty(Directory.GetFiles(directory, "*.tmp"));
        }
        finally
        {
            if (Directory.Exists(directory))
            {
                Directory.Delete(directory, recursive: true);
            }
        }
    }

    [Fact]
    public void CliContracts_SerializeSnakeCaseJson()
    {
        string json = JsonSerializer.Serialize(new RuntimeHealth
        {
            RuntimeLoopEnabled = true,
            RuntimeLoopRunning = true,
            StatusSummary = "Runtime healthy.",
        }, JsonDefaults.SnakeCase);

        Assert.Contains("\"runtime_loop_enabled\"", json);
        Assert.Contains("true", json);
        Assert.Contains("\"status_summary\"", json);
        Assert.Contains("\"Runtime healthy.\"", json);
        Assert.DoesNotContain("RuntimeLoopEnabled", json);
    }

    [Fact]
    public void BenchmarkRunner_HonorsHostSpecificBudgetMetadata()
    {
        BenchmarkSummary summary = BenchmarkRunner.Run(
            ticks: 1,
            sleepMs: 0,
            CancellationToken.None,
            new BenchmarkGateOptions
            {
                Host = "winui",
                MeasurementOrigin = BenchmarkRunner.WinUiMeasurementOrigin,
                RssBudgetBytes = 256UL * 1024UL * 1024UL,
            });

        Assert.Equal("winui", summary.Host);
        Assert.Equal(BenchmarkRunner.WinUiMeasurementOrigin, summary.MeasurementOrigin);
        Assert.Equal(256UL * 1024UL * 1024UL, summary.RssBudgetBytes);
    }

    [Fact]
    public void RuntimeViewReducer_PreservesSelectionAndRaisesHealthBanner()
    {
        ProcessSample alpha = Sample(10, "Alpha", 1);
        ProcessSample code = Sample(20, "Code", 2);
        RuntimeViewState initial = RuntimeViewReducer.Reduce(null, Snapshot(alpha, code));
        RuntimeViewState selected = initial with
        {
            SelectedIdentity = code.Identity(),
            SelectedProcess = code,
        };

        RuntimeViewState updated = RuntimeViewReducer.Reduce(
            selected,
            Snapshot([
                code with { CpuPct = 44.4 },
                alpha],
                warnings: [new RuntimeWarning { Category = "collector", Message = "Collector access was partially denied." }]));

        Assert.Equal(code.Identity(), updated.SelectedIdentity);
        Assert.Equal(44.4, updated.SelectedProcess?.CpuPct);
        Assert.Equal("Collector access was partially denied.", updated.HealthBanner);
        Assert.True(updated.HasHealthBanner);
    }

    private static RuntimeStore CreateStore(MemoryPersistenceStore persistence, int SubscriberBufferCapacity)
    {
        return new RuntimeStore(
            new FixedProcessCollector(Sample(10, "Alpha", 1), Sample(20, "Code", 9), Sample(30, "Beta", 2)),
            new FixedSystemMetricsCollector(),
            persistence,
            new RuntimeStoreOptions
            {
                TickInterval = TimeSpan.FromHours(1),
                SubscriberBufferCapacity = SubscriberBufferCapacity,
                WarmCacheWriteIntervalTicks = 1000,
            });
    }

    private static async Task<RuntimeDelta> ReadNextAfterAsync(
        IAsyncEnumerator<RuntimeDelta> subscription,
        Func<Task> trigger,
        CancellationToken ct)
    {
        Task<bool> moveNext = Task.Run(async () => await subscription.MoveNextAsync().AsTask().ConfigureAwait(false), ct);
        await Task.Delay(25, ct);
        await trigger().WaitAsync(ct);
        Assert.True(await moveNext.WaitAsync(ct));
        return subscription.Current;
    }

    private static async Task<RuntimeDelta> ReadNextAsync(IAsyncEnumerator<RuntimeDelta> subscription, CancellationToken ct)
    {
        Assert.True(await subscription.MoveNextAsync().AsTask().WaitAsync(ct));
        return subscription.Current;
    }

    private static RuntimeSnapshot Snapshot(params ProcessSample[] rows)
    {
        return Snapshot(rows, warnings: []);
    }

    private static RuntimeSnapshot Snapshot(ProcessSample[] rows, RuntimeWarning[] warnings)
    {
        return new RuntimeSnapshot
        {
            Seq = rows.FirstOrDefault()?.Seq ?? 0,
            Rows = Array.AsReadOnly(rows),
            TotalProcessCount = rows.Length,
            Warnings = Array.AsReadOnly(warnings),
            Health = new RuntimeHealth
            {
                RuntimeLoopEnabled = true,
                RuntimeLoopRunning = true,
                StatusSummary = "Runtime healthy.",
            },
        };
    }

    private static ProcessSample Sample(uint pid, string name, double cpu)
    {
        return new ProcessSample
        {
            Seq = 1,
            TsMs = 1,
            Pid = pid,
            StartTimeMs = pid * 1000UL,
            Name = name,
            CpuPct = cpu,
            MemoryBytes = pid * 1024UL,
            DiskBps = pid * 10UL,
            OtherIoBps = pid * 5UL,
            Threads = pid,
            Handles = pid * 2,
        };
    }

    private sealed class FixedProcessCollector(params ProcessSample[] rows) : IProcessCollector
    {
        public IReadOnlyList<ProcessSample> Collect(ulong seq)
        {
            ulong tsMs = (ulong)Math.Max(0L, DateTimeOffset.UtcNow.ToUnixTimeMilliseconds());
            return rows.Select(row => row with { Seq = seq, TsMs = tsMs }).ToArray();
        }
    }

    private sealed class FixedSystemMetricsCollector : ISystemMetricsCollector
    {
        public SystemMetricsSnapshot Sample()
        {
            return new SystemMetricsSnapshot
            {
                TsMs = (ulong)Math.Max(0L, DateTimeOffset.UtcNow.ToUnixTimeMilliseconds()),
                CpuPct = 17.5,
                MemoryUsedBytes = 2UL * 1024UL * 1024UL,
                MemoryTotalBytes = 8UL * 1024UL * 1024UL,
                IsReady = true,
            };
        }
    }

    private sealed class MemoryPersistenceStore : IRuntimePersistenceStore
    {
        private RuntimeSettings? _settings;
        private WarmCache? _warmCache;

        public List<RuntimeSettings> SavedSettings { get; } = [];

        public string BaseDirectory { get; } = Path.GetTempPath();

        public RuntimeSettings? LoadSettings() => _settings;

        public Task SaveSettingsAsync(RuntimeSettings settings, CancellationToken ct)
        {
            _settings = settings;
            SavedSettings.Add(settings);
            return Task.CompletedTask;
        }

        public WarmCache? LoadWarmCache() => _warmCache;

        public Task SaveWarmCacheAsync(WarmCache cache, CancellationToken ct)
        {
            _warmCache = cache;
            return Task.CompletedTask;
        }

        public Task AppendDiagnosticAsync(string category, object payload, CancellationToken ct) => Task.CompletedTask;
    }
}
