using BatCave.Core.Abstractions;
using BatCave.Core.Domain;
using BatCave.Core.Pipeline;
using BatCave.Core.Sort;
using BatCave.Core.State;
using BatCave.Core.Runtime;

namespace BatCave.Core.Tests.Runtime;

public class RuntimeLoopServiceTests
{
    [Fact]
    public async Task TickFaulted_ExceptionDoesNotStopLoop()
    {
        SequenceCollector collector = new(
        [
            _ => throw new InvalidOperationException("boom-1"),
            _ => [],
        ]);

        using MonitoringRuntime runtime = CreateRuntime(collector);
        RuntimeLoopService service = new(runtime, TimeProvider.System, TimeSpan.FromMilliseconds(25));
        List<TickFaultedEventArgs> faults = [];
        int completed = 0;

        service.TickFaulted += (_, args) => faults.Add(args);
        service.TickCompleted += (_, _) => Interlocked.Increment(ref completed);

        service.Start(service.CurrentGeneration);
        await Task.Delay(950);
        service.StopAndAdvanceGeneration();
        await Task.Delay(150);

        Assert.NotEmpty(faults);
        Assert.Equal(1, faults[0].ConsecutiveFaults);
        Assert.Equal(250, faults[0].DelayMs);
        Assert.Contains("InvalidOperationException", faults[0].ExceptionType, StringComparison.Ordinal);
        Assert.True(completed > 0);
    }

    [Fact]
    public async Task TickFaulted_BackoffEscalatesAndResetsAfterSuccess()
    {
        SequenceCollector collector = new(
        [
            _ => throw new InvalidOperationException("boom-1"),
            _ => throw new InvalidOperationException("boom-2"),
            _ => [],
            _ => throw new InvalidOperationException("boom-3"),
            _ => [],
        ]);

        using MonitoringRuntime runtime = CreateRuntime(collector);
        RuntimeLoopService service = new(runtime, TimeProvider.System, TimeSpan.FromMilliseconds(25));
        List<int> delays = [];

        service.TickFaulted += (_, args) => delays.Add(args.DelayMs);

        service.Start(service.CurrentGeneration);
        await Task.Delay(2_100);
        service.StopAndAdvanceGeneration();
        await Task.Delay(150);

        Assert.True(delays.Count >= 3);
        Assert.Equal(250, delays[0]);
        Assert.Equal(500, delays[1]);
        Assert.Equal(250, delays[2]);
    }

    private static MonitoringRuntime CreateRuntime(IProcessCollector collector)
    {
        return new MonitoringRuntime(
            new TestCollectorFactory(collector),
            new DeltaTelemetryPipeline(),
            new InMemoryStateStore(),
            new IncrementalSortIndexEngine(),
            new TestPersistenceStore());
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

    private sealed class SequenceCollector : IProcessCollector
    {
        private readonly Queue<Func<ulong, IReadOnlyList<ProcessSample>>> _steps;

        public SequenceCollector(IEnumerable<Func<ulong, IReadOnlyList<ProcessSample>>> steps)
        {
            _steps = new Queue<Func<ulong, IReadOnlyList<ProcessSample>>>(steps);
        }

        public IReadOnlyList<ProcessSample> CollectTick(ulong seq)
        {
            Func<ulong, IReadOnlyList<ProcessSample>> step = _steps.Count > 1 ? _steps.Dequeue() : _steps.Peek();
            return step(seq);
        }

        public string? TakeWarning()
        {
            return null;
        }
    }

    private sealed class TestPersistenceStore : IPersistenceStore
    {
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
}
