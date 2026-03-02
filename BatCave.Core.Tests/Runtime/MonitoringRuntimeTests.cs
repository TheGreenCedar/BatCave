using BatCave.Core.Abstractions;
using BatCave.Core.Domain;
using BatCave.Core.Pipeline;
using BatCave.Core.Runtime;
using BatCave.Core.Sort;
using BatCave.Core.State;

namespace BatCave.Core.Tests.Runtime;

public class MonitoringRuntimeTests
{
    [Fact]
    public void Tick_WhenPersistenceWarningQueued_SurfacesWarningAndIncrementsCounter()
    {
        TestPersistenceStore persistenceStore = new();
        TestCollector collector = new();
        using MonitoringRuntime runtime = CreateRuntime(collector, persistenceStore);

        persistenceStore.EnqueueWarning("persistence_load_json_failed path=settings.json error=JsonException: invalid json");

        TickOutcome outcome = runtime.Tick(jitterMs: 0);

        Assert.NotNull(outcome.Warning);
        Assert.Contains("persistence_load_json_failed", outcome.Warning!.Message, StringComparison.OrdinalIgnoreCase);
        Assert.Equal(1UL, outcome.Health.CollectorWarnings);
    }

    [Fact]
    public void SetFilter_WhenPersistenceSaveFails_QueuesWarningForNextTick()
    {
        TestPersistenceStore persistenceStore = new();
        persistenceStore.FailSaveSettings = true;
        TestCollector collector = new();
        using MonitoringRuntime runtime = CreateRuntime(collector, persistenceStore);

        runtime.SetFilter("svc");
        TickOutcome outcome = runtime.Tick(jitterMs: 0);

        Assert.NotNull(outcome.Warning);
        Assert.Contains("persistence_save_settings_failed", outcome.Warning!.Message, StringComparison.OrdinalIgnoreCase);
    }

    [Fact]
    public void Tick_PrioritizesCollectorWarningBeforePersistenceWarning()
    {
        TestPersistenceStore persistenceStore = new();
        persistenceStore.EnqueueWarning("persistence_warning");
        TestCollector collector = new();
        collector.EnqueueWarning("collector_warning");
        using MonitoringRuntime runtime = CreateRuntime(collector, persistenceStore);

        TickOutcome first = runtime.Tick(jitterMs: 0);
        TickOutcome second = runtime.Tick(jitterMs: 0);

        Assert.NotNull(first.Warning);
        Assert.Contains("collector_warning", first.Warning!.Message, StringComparison.OrdinalIgnoreCase);
        Assert.NotNull(second.Warning);
        Assert.Contains("persistence_warning", second.Warning!.Message, StringComparison.OrdinalIgnoreCase);
    }

    private static MonitoringRuntime CreateRuntime(TestCollector collector, TestPersistenceStore persistenceStore)
    {
        return new MonitoringRuntime(
            new TestCollectorFactory(collector),
            new DeltaTelemetryPipeline(),
            new InMemoryStateStore(),
            new IncrementalSortIndexEngine(),
            persistenceStore);
    }

    private sealed class TestCollectorFactory : IProcessCollectorFactory
    {
        private readonly IProcessCollector _collector;

        public TestCollectorFactory(IProcessCollector collector)
        {
            _collector = collector;
        }

        public IProcessCollector Create(bool adminMode)
        {
            return _collector;
        }
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

        public void EnqueueWarning(string warning)
        {
            _warnings.Enqueue(warning);
        }
    }

    private sealed class TestPersistenceStore : IPersistenceStore
    {
        private readonly Queue<string> _warnings = [];
        private UserSettings _settings = new();
        private WarmCache? _warmCache;

        public bool FailSaveSettings { get; set; }

        public string BaseDirectory => Path.GetTempPath();

        public UserSettings? LoadSettings()
        {
            return _settings;
        }

        public Task SaveSettingsAsync(UserSettings settings, CancellationToken ct)
        {
            if (FailSaveSettings)
            {
                EnqueueWarning("persistence_save_settings_failed path=settings.json error=IOException: write denied");
                throw new IOException("write denied");
            }

            _settings = settings;
            return Task.CompletedTask;
        }

        public WarmCache? LoadWarmCache()
        {
            return _warmCache;
        }

        public Task SaveWarmCacheAsync(WarmCache cache, CancellationToken ct)
        {
            _warmCache = cache;
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

        public void EnqueueWarning(string warning)
        {
            _warnings.Enqueue(warning);
        }
    }
}
