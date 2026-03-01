using BatCave.Core.Abstractions;
using BatCave.Core.Domain;
using BatCave.Core.Pipeline;
using BatCave.Core.Runtime;
using BatCave.Core.Sort;
using BatCave.Core.State;
using BatCave.Services;
using BatCave.ViewModels;
using Microsoft.UI.Xaml;
using System.Collections.Specialized;

namespace BatCave.Tests.ViewModels;

public class MonitoringShellViewModelTests
{
    [Fact]
    public async Task Bootstrap_WhenGateBlocked_ShowsBlockedState()
    {
        SequenceLaunchPolicyGate gate = new(
            () => StartupGateStatus.Blocked(LaunchBlockReason.RequiresWindows11(21999)));
        TestMetadataProvider metadata = new((_, _, _) => Task.FromResult<ProcessMetadata?>(null));
        TestRuntimeEventGateway gateway = new();
        MonitoringShellViewModel viewModel = CreateViewModel(gate, metadata, gateway);

        await viewModel.BootstrapAsync(CancellationToken.None);

        Assert.True(viewModel.IsBlocked);
        Assert.False(viewModel.IsLive);
        Assert.Equal(Visibility.Visible, viewModel.BlockedVisibility);
        Assert.Contains("Windows 11", viewModel.BlockedReasonMessage, StringComparison.OrdinalIgnoreCase);
    }

    [Fact]
    public async Task RetryBootstrap_RecoversAfterTransientFailure()
    {
        SequenceLaunchPolicyGate gate = new(
            () => throw new InvalidOperationException("transient gate check failure"),
            () => StartupGateStatus.PassedContext(new LaunchContext { Os = "windows", WindowsBuild = 26000 }));
        TestMetadataProvider metadata = new((_, _, _) => Task.FromResult<ProcessMetadata?>(null));
        TestRuntimeEventGateway gateway = new();
        MonitoringShellViewModel viewModel = CreateViewModel(gate, metadata, gateway);

        await viewModel.BootstrapAsync(CancellationToken.None);
        Assert.True(viewModel.IsStartupError);

        await viewModel.RetryBootstrapAsync(CancellationToken.None);

        Assert.False(viewModel.IsStartupError);
        Assert.True(viewModel.IsLive);
        Assert.Equal(Visibility.Visible, viewModel.LiveVisibility);
    }

    [Fact]
    public async Task AdminToggle_ControlsDeniedVisibilityAndAdminOnlyFilter()
    {
        SequenceLaunchPolicyGate gate = new(
            () => StartupGateStatus.PassedContext(new LaunchContext { Os = "windows", WindowsBuild = 26000 }));
        TestMetadataProvider metadata = new((_, _, _) => Task.FromResult<ProcessMetadata?>(null));
        TestRuntimeEventGateway gateway = new();
        MonitoringShellViewModel viewModel = CreateViewModel(gate, metadata, gateway);

        await viewModel.BootstrapAsync(CancellationToken.None);

        ProcessSample full = Sample(pid: 1, startTime: 100, access: AccessState.Full);
        ProcessSample denied = Sample(pid: 2, startTime: 200, access: AccessState.Denied);
        gateway.RaiseDelta(1, [full, denied], []);

        Assert.Single(viewModel.VisibleRows);
        Assert.Equal(1U, viewModel.VisibleRows[0].Pid);

        await viewModel.ToggleAdminModeAsync(true, CancellationToken.None);
        gateway.RaiseDelta(2, [full, denied], []);

        Assert.True(viewModel.AdminModeEnabled);
        Assert.Equal(2, viewModel.VisibleRows.Count);

        viewModel.AdminEnabledOnlyFilter = true;
        Assert.Single(viewModel.VisibleRows);
        Assert.Equal(AccessState.Full, viewModel.VisibleRows[0].AccessState);
    }

    [Fact]
    public async Task MetadataSelection_UsesCacheAndSurfacesNonFatalErrors()
    {
        SequenceLaunchPolicyGate gate = new(
            () => StartupGateStatus.PassedContext(new LaunchContext { Os = "windows", WindowsBuild = 26000 }));
        TestMetadataProvider metadata = new((pid, _, _) => Task.FromResult<ProcessMetadata?>(new ProcessMetadata
        {
            Pid = pid,
            ParentPid = 1,
            CommandLine = "demo --flag",
            ExecutablePath = "C:\\demo.exe",
        }));
        TestRuntimeEventGateway gateway = new();
        MonitoringShellViewModel viewModel = CreateViewModel(gate, metadata, gateway);

        await viewModel.BootstrapAsync(CancellationToken.None);

        ProcessSample first = Sample(pid: 10, startTime: 1000, access: AccessState.Full);
        ProcessSample second = Sample(pid: 11, startTime: 1100, access: AccessState.Full);
        gateway.RaiseDelta(1, [first, second], []);

        await viewModel.SelectRowAsync(first, CancellationToken.None);
        Assert.NotNull(viewModel.SelectedMetadata);
        Assert.Null(viewModel.MetadataError);

        metadata.Handler = (_, _, _) => throw new InvalidOperationException("metadata boom");
        await viewModel.SelectRowAsync(second, CancellationToken.None);
        Assert.True(viewModel.IsLive);
        Assert.Null(viewModel.SelectedMetadata);
        Assert.Contains("metadata boom", viewModel.MetadataError ?? string.Empty, StringComparison.OrdinalIgnoreCase);

        await viewModel.SelectRowAsync(first, CancellationToken.None);
        Assert.NotNull(viewModel.SelectedMetadata);
        Assert.Null(viewModel.MetadataError);
    }

    [Fact]
    public async Task TelemetryDelta_RefreshesVisibleRowsWithoutCollectionReset()
    {
        SequenceLaunchPolicyGate gate = new(
            () => StartupGateStatus.PassedContext(new LaunchContext { Os = "windows", WindowsBuild = 26000 }));
        TestMetadataProvider metadata = new((_, _, _) => Task.FromResult<ProcessMetadata?>(null));
        TestRuntimeEventGateway gateway = new();
        MonitoringShellViewModel viewModel = CreateViewModel(gate, metadata, gateway);

        await viewModel.BootstrapAsync(CancellationToken.None);

        ProcessSample row = Sample(pid: 20, startTime: 2000, access: AccessState.Full);
        gateway.RaiseDelta(1, [row], []);

        bool sawReset = false;
        viewModel.VisibleRows.CollectionChanged += (_, args) =>
        {
            if (args.Action == NotifyCollectionChangedAction.Reset)
            {
                sawReset = true;
            }
        };

        ProcessSample updatedRow = row with { Seq = 2, TsMs = 2, CpuPct = 67.4 };
        gateway.RaiseDelta(2, [updatedRow], []);

        Assert.False(sawReset);
        Assert.Single(viewModel.VisibleRows);
        Assert.Equal(updatedRow.CpuPct, viewModel.VisibleRows[0].Sample.CpuPct);
    }

    [Fact]
    public async Task TelemetryDelta_HeartbeatOnly_DoesNotReplaceVisibleRowInstance()
    {
        SequenceLaunchPolicyGate gate = new(
            () => StartupGateStatus.PassedContext(new LaunchContext { Os = "windows", WindowsBuild = 26000 }));
        TestMetadataProvider metadata = new((_, _, _) => Task.FromResult<ProcessMetadata?>(null));
        TestRuntimeEventGateway gateway = new();
        MonitoringShellViewModel viewModel = CreateViewModel(gate, metadata, gateway);

        await viewModel.BootstrapAsync(CancellationToken.None);

        ProcessSample row = Sample(pid: 21, startTime: 2100, access: AccessState.Full) with { CpuPct = 10 };
        gateway.RaiseDelta(1, [row], []);
        ProcessSample varied = row with { Seq = 2, TsMs = 2, CpuPct = 20 };
        gateway.RaiseDelta(2, [varied], []);
        ProcessRowViewState firstVisible = viewModel.VisibleRows[0];
        string beforeTrend = firstVisible.CpuTrendPoints;

        ProcessSample heartbeatOnlyUpdate = varied with { Seq = 3, TsMs = 3, ParentPid = varied.ParentPid + 1, PrivateBytes = varied.PrivateBytes + 1 };
        gateway.RaiseDelta(3, [heartbeatOnlyUpdate], []);
        string afterTrend = firstVisible.CpuTrendPoints;

        Assert.Same(firstVisible, viewModel.VisibleRows[0]);
        Assert.NotEqual(beforeTrend, afterTrend);
    }

    [Fact]
    public async Task MetricHistory_CapsAtConfiguredLimit()
    {
        SequenceLaunchPolicyGate gate = new(
            () => StartupGateStatus.PassedContext(new LaunchContext { Os = "windows", WindowsBuild = 26000 }));
        TestMetadataProvider metadata = new((_, _, _) => Task.FromResult<ProcessMetadata?>(null));
        TestRuntimeEventGateway gateway = new();
        MonitoringShellViewModel viewModel = CreateViewModel(gate, metadata, gateway);

        await viewModel.BootstrapAsync(CancellationToken.None);

        ProcessSample current = Sample(pid: 22, startTime: 2200, access: AccessState.Full) with { CpuPct = 0 };
        for (ulong seq = 1; seq <= 130; seq++)
        {
            current = current with
            {
                Seq = seq,
                TsMs = seq,
                CpuPct = seq,
            };
            gateway.RaiseDelta(seq, [current], []);
        }

        await viewModel.SelectRowAsync(current, CancellationToken.None);

        Assert.Single(viewModel.VisibleRows);
        Assert.Equal(120, CountPoints(viewModel.VisibleRows[0].CpuTrendPoints));
        Assert.Equal(120, CountPoints(viewModel.CpuMetricTrendPoints));
    }

    [Fact]
    public async Task NoSelection_UsesGlobalSummaryForDetailTrends()
    {
        SequenceLaunchPolicyGate gate = new(
            () => StartupGateStatus.PassedContext(new LaunchContext { Os = "windows", WindowsBuild = 26000 }));
        TestMetadataProvider metadata = new((_, _, _) => Task.FromResult<ProcessMetadata?>(null));
        TestRuntimeEventGateway gateway = new();
        MonitoringShellViewModel viewModel = CreateViewModel(gate, metadata, gateway);

        await viewModel.BootstrapAsync(CancellationToken.None);

        ProcessSample first = Sample(pid: 40, startTime: 4000, access: AccessState.Full) with { CpuPct = 35 };
        gateway.RaiseDelta(1, [first], []);

        Assert.Null(viewModel.SelectedRow);
        Assert.Equal("Global System Values", viewModel.DetailTitle);
        Assert.NotEqual("0.00%", viewModel.CpuMetricChipValue);
        Assert.NotEmpty(viewModel.ExpandedMetricTrendPoints);
    }

    [Fact]
    public async Task ToggleSelection_SameIdentity_DoesNotClearSelection()
    {
        SequenceLaunchPolicyGate gate = new(
            () => StartupGateStatus.PassedContext(new LaunchContext { Os = "windows", WindowsBuild = 26000 }));
        TestMetadataProvider metadata = new((pid, _, _) => Task.FromResult<ProcessMetadata?>(new ProcessMetadata
        {
            Pid = pid,
            ParentPid = 1,
        }));
        TestRuntimeEventGateway gateway = new();
        MonitoringShellViewModel viewModel = CreateViewModel(gate, metadata, gateway);

        await viewModel.BootstrapAsync(CancellationToken.None);

        ProcessSample row = Sample(pid: 30, startTime: 3000, access: AccessState.Full);
        gateway.RaiseDelta(1, [row], []);

        await viewModel.SelectRowAsync(row, CancellationToken.None);
        await viewModel.ToggleSelectionAsync(row, CancellationToken.None);

        Assert.NotNull(viewModel.SelectedRow);
        Assert.Equal(row.Identity(), viewModel.SelectedRow!.Identity());
        Assert.NotNull(viewModel.SelectedMetadata);
        Assert.True(viewModel.HasSelection);
    }

    private static MonitoringShellViewModel CreateViewModel(
        SequenceLaunchPolicyGate gate,
        TestMetadataProvider metadataProvider,
        TestRuntimeEventGateway gateway)
    {
        MonitoringRuntime runtime = new(
            new TestCollectorFactory(),
            new DeltaTelemetryPipeline(),
            new InMemoryStateStore(),
            new IncrementalSortIndexEngine(),
            new TestPersistenceStore());
        RuntimeLoopService loopService = new(runtime);
        return new MonitoringShellViewModel(gate, runtime, loopService, gateway, metadataProvider);
    }

    private static int CountPoints(string serializedPoints)
    {
        if (string.IsNullOrWhiteSpace(serializedPoints))
        {
            return 0;
        }

        return serializedPoints.Split(' ', StringSplitOptions.RemoveEmptyEntries).Length;
    }

    private static ProcessSample Sample(uint pid, ulong startTime, AccessState access)
    {
        return new ProcessSample
        {
            Seq = 1,
            TsMs = 1,
            Pid = pid,
            ParentPid = 1,
            StartTimeMs = startTime,
            Name = $"proc-{pid}",
            CpuPct = 1,
            RssBytes = 1024,
            PrivateBytes = 512,
            IoReadBps = 10,
            IoWriteBps = 10,
            NetBps = 10,
            Threads = 2,
            Handles = 3,
            AccessState = access,
        };
    }

    private sealed class SequenceLaunchPolicyGate : ILaunchPolicyGate
    {
        private readonly Queue<Func<StartupGateStatus>> _steps;

        public SequenceLaunchPolicyGate(params Func<StartupGateStatus>[] steps)
        {
            _steps = new Queue<Func<StartupGateStatus>>(steps);
        }

        public StartupGateStatus Enforce()
        {
            Func<StartupGateStatus> step = _steps.Count > 1 ? _steps.Dequeue() : _steps.Peek();
            return step();
        }
    }

    private sealed class TestCollectorFactory : IProcessCollectorFactory
    {
        public IProcessCollector Create(bool adminMode)
        {
            return new TestCollector();
        }
    }

    private sealed class TestCollector : IProcessCollector
    {
        public IReadOnlyList<ProcessSample> CollectTick(ulong seq)
        {
            return [];
        }

        public string? TakeWarning()
        {
            return null;
        }
    }

    private sealed class TestPersistenceStore : IPersistenceStore
    {
        private UserSettings _settings = new();
        private WarmCache? _warmCache;

        public string BaseDirectory => Path.GetTempPath();

        public UserSettings? LoadSettings()
        {
            return _settings;
        }

        public Task SaveSettingsAsync(UserSettings settings, CancellationToken ct)
        {
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
    }

    private sealed class TestMetadataProvider : IProcessMetadataProvider
    {
        public Func<uint, ulong, CancellationToken, Task<ProcessMetadata?>> Handler { get; set; }

        public TestMetadataProvider(Func<uint, ulong, CancellationToken, Task<ProcessMetadata?>> handler)
        {
            Handler = handler;
        }

        public Task<ProcessMetadata?> GetAsync(uint pid, ulong startTimeMs, CancellationToken ct)
        {
            return Handler(pid, startTimeMs, ct);
        }
    }

    private sealed class TestRuntimeEventGateway : IRuntimeEventGateway
    {
        public event EventHandler<ProcessDeltaBatch>? TelemetryDelta;
        public event EventHandler<RuntimeHealth>? RuntimeHealthChanged;
        public event EventHandler<CollectorWarning>? CollectorWarningRaised;

        public void Publish(TickOutcome outcome)
        {
            if (outcome.EmitTelemetryDelta)
            {
                TelemetryDelta?.Invoke(this, outcome.Delta);
            }

            RuntimeHealthChanged?.Invoke(this, outcome.Health);

            if (outcome.Warning is not null)
            {
                CollectorWarningRaised?.Invoke(this, outcome.Warning);
            }
        }

        public void RaiseDelta(ulong seq, IReadOnlyList<ProcessSample> upserts, IReadOnlyList<ProcessIdentity> exits)
        {
            TelemetryDelta?.Invoke(this, new ProcessDeltaBatch
            {
                Seq = seq,
                Upserts = upserts,
                Exits = exits,
            });
        }
    }
}
