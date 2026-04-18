using BatCave.Core.Abstractions;
using BatCave.Core.Domain;
using BatCave.Core.Runtime;
using BatCave.Core.Tests.Runtime.TestSupport;

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

        using MonitoringRuntime runtime = RuntimeTestHarness.CreateRuntime(collector, new TestPersistenceStore());
        RuntimeLoopService runtimeLoopService = new(runtime, TimeProvider.System, TimeSpan.FromMilliseconds(25));
        List<TickFaultedEventArgs> faults = [];
        int completedTickCount = 0;

        runtimeLoopService.TickFaulted += (_, args) => faults.Add(args);
        runtimeLoopService.TickCompleted += (_, _) => Interlocked.Increment(ref completedTickCount);

        await RunLoopForAsync(runtimeLoopService, runDurationMs: 950);

        Assert.NotEmpty(faults);
        Assert.Equal(1, faults[0].ConsecutiveFaults);
        Assert.Equal(250, faults[0].DelayMs);
        Assert.Contains("InvalidOperationException", faults[0].ExceptionType, StringComparison.Ordinal);
        Assert.True(completedTickCount > 0);
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

        using MonitoringRuntime runtime = RuntimeTestHarness.CreateRuntime(collector, new TestPersistenceStore());
        RuntimeLoopService runtimeLoopService = new(runtime, TimeProvider.System, TimeSpan.FromMilliseconds(25));
        List<int> delays = [];

        runtimeLoopService.TickFaulted += (_, args) => delays.Add(args.DelayMs);

        await RunLoopForAsync(runtimeLoopService, runDurationMs: 2_100);

        Assert.True(delays.Count >= 3);
        Assert.Equal(250, delays[0]);
        Assert.Equal(500, delays[1]);
        Assert.Equal(250, delays[2]);
    }

    [Fact]
    public async Task Start_WithStaleGeneration_DoesNotRunTicks()
    {
        SequenceCollector collector = new([_ => []]);
        using MonitoringRuntime runtime = RuntimeTestHarness.CreateRuntime(collector, new TestPersistenceStore());
        RuntimeLoopService runtimeLoopService = new(runtime, TimeProvider.System, TimeSpan.FromMilliseconds(25));

        int completedTickCount = 0;
        runtimeLoopService.TickCompleted += (_, _) => Interlocked.Increment(ref completedTickCount);

        long staleGeneration = runtimeLoopService.CurrentGeneration;
        runtimeLoopService.StopAndAdvanceGeneration();
        runtimeLoopService.Start(staleGeneration);

        await Task.Delay(200);
        runtimeLoopService.StopAndAdvanceGeneration();
        await Task.Delay(75);

        Assert.Equal(0, completedTickCount);
    }

    [Fact]
    public async Task StopAndAdvanceGenerationAsync_WhenTickInFlight_WaitsForQuiescentStop()
    {
        BlockingCollector collector = new();
        using MonitoringRuntime runtime = RuntimeTestHarness.CreateRuntime(collector, new TestPersistenceStore());
        RuntimeLoopService runtimeLoopService = new(runtime, TimeProvider.System, TimeSpan.FromMilliseconds(25));

        runtimeLoopService.Start(runtimeLoopService.CurrentGeneration);
        await collector.WaitForTickStartedAsync();

        Task stopTask = runtimeLoopService.StopAndAdvanceGenerationAsync(CancellationToken.None);
        Task completed = await Task.WhenAny(stopTask, Task.Delay(100));
        Assert.NotSame(stopTask, completed);

        collector.ReleaseTick();
        await stopTask.WaitAsync(TimeSpan.FromSeconds(2));
        Assert.Equal(1, collector.CompletedTicks);
    }

    [Fact]
    public async Task TickDuration_WhenOverInterval_RecordsDroppedTicksUsingConfiguredInterval()
    {
        SlowCollector collector = new(TimeSpan.FromMilliseconds(180));
        using MonitoringRuntime runtime = RuntimeTestHarness.CreateRuntime(collector, new TestPersistenceStore());
        RuntimeLoopService runtimeLoopService = new(runtime, TimeProvider.System, TimeSpan.FromMilliseconds(25));

        runtimeLoopService.Start(runtimeLoopService.CurrentGeneration);
        await collector.WaitForFirstTickCompletedAsync();
        await runtimeLoopService.StopAndAdvanceGenerationAsync(CancellationToken.None);

        Assert.True(runtime.GetRuntimeHealth().DroppedTicks > 0);
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
            Func<ulong, IReadOnlyList<ProcessSample>> nextCollectStep = _steps.Count > 1 ? _steps.Dequeue() : _steps.Peek();
            return nextCollectStep(seq);
        }

        public string? TakeWarning()
        {
            return null;
        }
    }

    private sealed class BlockingCollector : IProcessCollector
    {
        private readonly TaskCompletionSource<bool> _tickStarted = new(TaskCreationOptions.RunContinuationsAsynchronously);
        private readonly TaskCompletionSource<bool> _releaseTick = new(TaskCreationOptions.RunContinuationsAsynchronously);
        private int _completedTicks;

        public int CompletedTicks => Volatile.Read(ref _completedTicks);

        public IReadOnlyList<ProcessSample> CollectTick(ulong seq)
        {
            _tickStarted.TrySetResult(true);
            _releaseTick.Task.GetAwaiter().GetResult();
            Interlocked.Increment(ref _completedTicks);
            return [];
        }

        public string? TakeWarning()
        {
            return null;
        }

        public Task WaitForTickStartedAsync()
        {
            return _tickStarted.Task;
        }

        public void ReleaseTick()
        {
            _releaseTick.TrySetResult(true);
        }
    }

    private sealed class SlowCollector : IProcessCollector
    {
        private readonly TimeSpan _firstTickDelay;
        private readonly TaskCompletionSource<bool> _firstTickCompleted = new(TaskCreationOptions.RunContinuationsAsynchronously);
        private int _calls;

        public SlowCollector(TimeSpan firstTickDelay)
        {
            _firstTickDelay = firstTickDelay;
        }

        public IReadOnlyList<ProcessSample> CollectTick(ulong seq)
        {
            if (Interlocked.Increment(ref _calls) == 1)
            {
                Thread.Sleep(_firstTickDelay);
                _firstTickCompleted.TrySetResult(true);
            }

            return [];
        }

        public string? TakeWarning()
        {
            return null;
        }

        public Task WaitForFirstTickCompletedAsync()
        {
            return _firstTickCompleted.Task;
        }
    }

    private static async Task RunLoopForAsync(RuntimeLoopService runtimeLoopService, int runDurationMs)
    {
        runtimeLoopService.Start(runtimeLoopService.CurrentGeneration);
        await Task.Delay(runDurationMs);
        await runtimeLoopService.StopAndAdvanceGenerationAsync(CancellationToken.None);
    }
}
