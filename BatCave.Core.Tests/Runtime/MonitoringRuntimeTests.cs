using BatCave.Core.Abstractions;
using BatCave.Core.Domain;
using BatCave.Core.Runtime;
using BatCave.Core.Tests.Runtime.TestSupport;

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
        TickOutcome outcome = runtime.Tick(jitterMs: 0);

        Assert.NotNull(outcome.Warning);
        Assert.Contains("persistence_save_settings_failed", outcome.Warning!.Message, StringComparison.OrdinalIgnoreCase);
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
}
