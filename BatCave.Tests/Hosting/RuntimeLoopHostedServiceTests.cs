using BatCave.Core.Abstractions;
using BatCave.Core.Domain;
using BatCave.Core.Runtime;
using BatCave.Hosting;
using BatCave.Services;
using Microsoft.Extensions.DependencyInjection;
using Microsoft.Extensions.Logging.Abstractions;

namespace BatCave.Tests.Hosting;

public class RuntimeLoopHostedServiceTests
{
    [Fact]
    public async Task StartAsync_WhenLoopDisabled_DoesNotStartController()
    {
        FakeRuntimeLoopController runtimeLoopController = new();
        RuntimeHealthService runtimeHealthService = new();
        RuntimeLoopHostedService hostedService = new(
            runtimeLoopController,
            new FakeRuntimeEventGateway(),
            new FakeLaunchPolicyGate(StartupGateStatus.PassedContext(new LaunchContext { Os = "Windows", WindowsBuild = 22631 })),
            new RuntimeHostOptions { EnableRuntimeLoop = false },
            runtimeHealthService,
            NullLogger<RuntimeLoopHostedService>.Instance);

        await hostedService.StartAsync(CancellationToken.None);

        Assert.Equal(0, runtimeLoopController.StartCalls);
        RuntimeHealthSnapshot snapshot = runtimeHealthService.Snapshot();
        Assert.False(snapshot.RuntimeLoopEnabled);
        Assert.False(snapshot.RuntimeLoopRunning);
        Assert.False(snapshot.StartupBlocked);
    }

    [Fact]
    public async Task StartAsync_WhenPolicyBlocked_DoesNotStartController_AndSetsBlockedHealth()
    {
        FakeRuntimeLoopController runtimeLoopController = new();
        RuntimeHealthService runtimeHealthService = new();
        RuntimeLoopHostedService hostedService = new(
            runtimeLoopController,
            new FakeRuntimeEventGateway(),
            new FakeLaunchPolicyGate(StartupGateStatus.Blocked(LaunchBlockReason.RequiresWindows11(19045))),
            new RuntimeHostOptions { EnableRuntimeLoop = true },
            runtimeHealthService,
            NullLogger<RuntimeLoopHostedService>.Instance);

        await hostedService.StartAsync(CancellationToken.None);

        Assert.Equal(0, runtimeLoopController.StartCalls);
        RuntimeHealthSnapshot snapshot = runtimeHealthService.Snapshot();
        Assert.True(snapshot.RuntimeLoopEnabled);
        Assert.False(snapshot.RuntimeLoopRunning);
        Assert.True(snapshot.StartupBlocked);
        Assert.Contains("blocked", snapshot.StatusSummary, StringComparison.OrdinalIgnoreCase);
    }

    [Fact]
    public async Task StartStop_WhenPolicyAllows_StartsAndStopsRuntimeLoopOnce()
    {
        FakeRuntimeLoopController runtimeLoopController = new();
        RuntimeHealthService runtimeHealthService = new();
        RuntimeLoopHostedService hostedService = new(
            runtimeLoopController,
            new FakeRuntimeEventGateway(),
            new FakeLaunchPolicyGate(StartupGateStatus.PassedContext(new LaunchContext { Os = "Windows", WindowsBuild = 22631 })),
            new RuntimeHostOptions { EnableRuntimeLoop = true },
            runtimeHealthService,
            NullLogger<RuntimeLoopHostedService>.Instance);

        await hostedService.StartAsync(CancellationToken.None);
        await hostedService.StopAsync(CancellationToken.None);

        Assert.Equal(1, runtimeLoopController.StartCalls);
        Assert.Equal(1, runtimeLoopController.StopCalls);
        RuntimeHealthSnapshot snapshot = runtimeHealthService.Snapshot();
        Assert.True(snapshot.RuntimeLoopEnabled);
        Assert.False(snapshot.RuntimeLoopRunning);
        Assert.False(snapshot.StartupBlocked);
    }

    [Fact]
    public void AddBatCaveRuntimeServices_RegistersRuntimeHostComposition()
    {
        ServiceCollection services = new();
        services.AddLogging();

        services.AddBatCaveRuntimeServices(new RuntimeHostOptions());

        Assert.Contains(services, descriptor => descriptor.ServiceType == typeof(RuntimeHostOptions));
        Assert.Contains(services, descriptor => descriptor.ServiceType == typeof(IRuntimeHealthService));
        Assert.Contains(services, descriptor => descriptor.ServiceType == typeof(IRuntimeEventGateway));
        Assert.Contains(services, descriptor => descriptor.ServiceType == typeof(RuntimeLoopService));
        Assert.Contains(services, descriptor => descriptor.ServiceType == typeof(IRuntimeLoopController));
        Assert.Contains(services, descriptor => descriptor.ServiceType == typeof(Microsoft.Extensions.Hosting.IHostedService));
    }

    private sealed class FakeLaunchPolicyGate : ILaunchPolicyGate
    {
        private readonly StartupGateStatus _status;

        public FakeLaunchPolicyGate(StartupGateStatus status)
        {
            _status = status;
        }

        public StartupGateStatus Enforce()
        {
            return _status;
        }
    }

    private sealed class FakeRuntimeEventGateway : IRuntimeEventGateway
    {
        public event EventHandler<ProcessDeltaBatch>? TelemetryDelta;

        public event EventHandler<RuntimeHealth>? RuntimeHealthChanged;

        public event EventHandler<CollectorWarning>? CollectorWarningRaised;

        public void Publish(TickOutcome outcome)
        {
            RuntimeHealthChanged?.Invoke(this, outcome.Health);
            if (outcome.Warning is not null)
            {
                CollectorWarningRaised?.Invoke(this, outcome.Warning);
            }

            if (outcome.EmitTelemetryDelta || outcome.Delta.Upserts.Count > 0 || outcome.Delta.Exits.Count > 0)
            {
                TelemetryDelta?.Invoke(this, outcome.Delta);
            }
        }

        public void PublishWarning(CollectorWarning warning)
        {
            CollectorWarningRaised?.Invoke(this, warning);
        }
    }

    private sealed class FakeRuntimeLoopController : IRuntimeLoopController
    {
        public event EventHandler<TickOutcome>? TickCompleted;

        public event EventHandler<TickFaultedEventArgs>? TickFaulted;

        public int StartCalls { get; private set; }

        public int StopCalls { get; private set; }

        public long CurrentGeneration { get; private set; } = 1;

        public void Start(long generation)
        {
            StartCalls++;
            CurrentGeneration = generation;
        }

        public void StopAndAdvanceGeneration()
        {
            StopCalls++;
            CurrentGeneration++;
        }
    }
}
