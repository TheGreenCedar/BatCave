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
        return new MonitoringRuntime(
            new DelegatingCollectorFactory(collector),
            new DeltaTelemetryPipeline(),
            new InMemoryStateStore(),
            new IncrementalSortIndexEngine(),
            persistenceStore);
    }

    private sealed class DelegatingCollectorFactory : IProcessCollectorFactory
    {
        private readonly IProcessCollector _collector;

        public DelegatingCollectorFactory(IProcessCollector collector)
        {
            _collector = collector;
        }

        public IProcessCollector Create(bool _)
        {
            return _collector;
        }
    }
}
