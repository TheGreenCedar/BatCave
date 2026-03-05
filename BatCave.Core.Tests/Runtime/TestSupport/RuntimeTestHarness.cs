using BatCave.Core.Abstractions;
using BatCave.Core.Pipeline;
using BatCave.Core.Runtime;
using BatCave.Core.Sort;
using BatCave.Core.State;

namespace BatCave.Core.Tests.Runtime.TestSupport;

internal static class RuntimeTestHarness
{
    public static MonitoringRuntime CreateRuntime(IProcessCollector collector, IPersistenceStore persistenceStore)
    {
        MonitoringRuntime runtime = new(
            new DelegatingCollectorFactory(collector),
            new DeltaTelemetryPipeline(),
            new InMemoryStateStore(),
            new IncrementalSortIndexEngine(),
            persistenceStore,
            new RuntimeHostOptions());
        runtime.InitializeAsync(CancellationToken.None).GetAwaiter().GetResult();
        return runtime;
    }

    private sealed class DelegatingCollectorFactory : IProcessCollectorFactory
    {
        private readonly IProcessCollector _collector;

        public DelegatingCollectorFactory(IProcessCollector collector)
        {
            _collector = collector;
        }

        public ValueTask<CollectorActivationResult> CreateAsync(bool adminMode, CancellationToken ct)
        {
            return ValueTask.FromResult(new CollectorActivationResult(_collector, adminMode, Warning: null));
        }
    }
}
