using BatCave.Controls;
using BatCave.Converters;
using BatCave.Core.Abstractions;
using BatCave.Core.Domain;
using BatCave.Core.Pipeline;
using BatCave.Core.Runtime;
using BatCave.Core.Sort;
using BatCave.Core.State;
using BatCave.Services;
using BatCave.Tests.TestSupport;
using BatCave.ViewModels;
using Microsoft.UI.Xaml;
using System.Collections.ObjectModel;
using System.Collections.Specialized;
using System.Diagnostics;

namespace BatCave.Tests.ViewModels;

public class MonitoringShellViewModelTests
{
    [Fact]
    public async Task Bootstrap_WhenGateBlocked_ShowsBlockedState()
    {
        SequenceLaunchPolicyGate gate = new(
            () => StartupGateStatus.Blocked(LaunchBlockReason.RequiresWindows11(21999)));
        TestMetadataProvider metadata = CreateNullMetadataProvider();
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
        TestMetadataProvider metadata = CreateNullMetadataProvider();
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
    public async Task GlobalPerformance_ShowsSkeletonUntilReadySampleArrives()
    {
        TestRuntimeEventGateway gateway = new();
        SystemGlobalMetricsSample notReady = CreateSystemGlobalMetricsSample(
            tsMs: 1,
            cpuPct: null,
            memoryUsedBytes: null,
            diskReadBps: null,
            diskWriteBps: null,
            otherIoBps: null,
            cpuRateWarmed: false,
            rateCountersWarmed: false,
            extendedProbeCycleCompleted: false,
            isReady: false);
        SystemGlobalMetricsSample ready = CreateSystemGlobalMetricsSample(
            tsMs: 2,
            cpuPct: 12,
            memoryUsedBytes: 10 * 1024UL * 1024UL,
            diskReadBps: 1024UL,
            diskWriteBps: 2048UL,
            otherIoBps: 4096UL,
            cpuRateWarmed: true,
            rateCountersWarmed: true,
            extendedProbeCycleCompleted: true,
            isReady: true);

        bool serveReady = false;
        TestSystemGlobalMetricsSampler sampler = new(notReady)
        {
            Handler = () => serveReady ? ready : notReady,
        };

        MonitoringShellViewModel viewModel = await CreateBootstrappedViewModelAsync(
            gateway,
            systemGlobalMetricsSampler: sampler);

        Assert.Equal(Visibility.Visible, viewModel.GlobalPerformanceSkeletonVisibility);
        Assert.Equal(Visibility.Collapsed, viewModel.GlobalPerformanceContentVisibility);

        serveReady = true;
        ProcessSample row = Sample(pid: 700, startTime: 7000, access: AccessState.Full);
        gateway.RaiseDelta(1, [row], []);
        gateway.RaiseDelta(2, [row with { Seq = 2, TsMs = 2 }], []);

        Assert.Equal(Visibility.Collapsed, viewModel.GlobalPerformanceSkeletonVisibility);
        Assert.Equal(Visibility.Visible, viewModel.GlobalPerformanceContentVisibility);
    }

    [Fact]
    public async Task Dispose_UnsubscribesFromTelemetryAndClearsViewState()
    {
        TestRuntimeEventGateway gateway = new();
        MonitoringShellViewModel viewModel = await CreateBootstrappedViewModelAsync(gateway);
        ProcessSample first = Sample(pid: 710, startTime: 7_100, access: AccessState.Full);
        ProcessSample second = Sample(pid: 711, startTime: 7_110, access: AccessState.Full);

        gateway.RaiseDelta(1, [first], []);
        Assert.Single(GetVisibleRows(viewModel));

        viewModel.Dispose();
        viewModel.Dispose();

        Assert.Empty(GetVisibleRows(viewModel));

        gateway.RaiseDelta(2, [second], []);
        Assert.Empty(GetVisibleRows(viewModel));
    }

    [Fact]
    public async Task AdminToggle_ControlsDeniedVisibilityAndAdminOnlyFilter()
    {
        TestRuntimeEventGateway gateway = new();
        MonitoringShellViewModel viewModel = await CreateBootstrappedViewModelAsync(gateway);

        ProcessSample full = Sample(pid: 1, startTime: 100, access: AccessState.Full);
        ProcessSample denied = Sample(pid: 2, startTime: 200, access: AccessState.Denied);
        gateway.RaiseDelta(1, [full, denied], []);

        Assert.Single(GetVisibleRows(viewModel));
        Assert.Equal(1U, GetVisibleRow(viewModel, 0).Pid);

        await viewModel.ToggleAdminModeAsync(true, CancellationToken.None);
        gateway.RaiseDelta(2, [full, denied], []);

        Assert.True(viewModel.AdminModeEnabled);
        Assert.Equal(2, GetVisibleRows(viewModel).Count);

        viewModel.AdminEnabledOnlyFilter = true;
        Assert.Single(GetVisibleRows(viewModel));
        Assert.Equal(AccessState.Full, GetVisibleRow(viewModel, 0).AccessState);
    }

    [Fact]
    public async Task InspectorSections_DefaultToSummary_AndSwitchViaCommand()
    {
        TestRuntimeEventGateway gateway = new();
        MonitoringShellViewModel viewModel = await CreateBootstrappedViewModelAsync(gateway);

        Assert.Equal(InspectorLayoutMode.SystemOverview, viewModel.InspectorLayoutMode);
        Assert.True(viewModel.IsSystemOverview);
        Assert.False(viewModel.IsProcessInspector);
        Assert.Equal("SYSTEM VIEW", viewModel.InspectorOverviewEyebrow);
        Assert.Equal("System Overview", viewModel.InspectorContextTitle);
        Assert.Equal(Visibility.Visible, viewModel.SystemSummarySectionVisibility);
        Assert.Equal(Visibility.Collapsed, viewModel.ProcessSummarySectionVisibility);
        Assert.True(viewModel.IsSummarySectionSelected);
        Assert.Equal(Visibility.Visible, viewModel.SummarySectionVisibility);
        Assert.Equal(Visibility.Collapsed, viewModel.PerformanceSectionVisibility);

        viewModel.SelectInspectorSectionCommand.Execute("Performance");

        Assert.True(viewModel.IsPerformanceSectionSelected);
        Assert.Equal(Visibility.Visible, viewModel.PerformanceSectionVisibility);
        Assert.Equal(Visibility.Collapsed, viewModel.SummarySectionVisibility);
    }

    [Fact]
    public async Task RuntimeStatusPresentation_MapsHealthyAndWarningStates()
    {
        TestRuntimeEventGateway gateway = new();
        MonitoringShellViewModel viewModel = await CreateBootstrappedViewModelAsync(gateway);

        Assert.Equal("Runtime Healthy", viewModel.RuntimeStatusTitle);
        Assert.Equal("HEALTH", viewModel.RuntimeStatusTag);
        Assert.Equal(RuntimeStatusTone.Success, viewModel.RuntimeStatusTone);
        Assert.Equal(Visibility.Visible, viewModel.RuntimeStatusSuccessVisibility);
        Assert.Equal(Visibility.Collapsed, viewModel.RuntimeStatusWarningVisibility);
        Assert.Equal(Visibility.Collapsed, viewModel.RuntimeStatusVisibility);

        gateway.PublishWarning(new CollectorWarning
        {
            Seq = 1,
            Message = "bridge warning",
        });

        Assert.Equal("Collector Warning", viewModel.RuntimeStatusTitle);
        Assert.Equal("bridge warning", viewModel.RuntimeStatusSummary);
        Assert.Equal("WARN", viewModel.RuntimeStatusTag);
        Assert.Equal(RuntimeStatusTone.Warning, viewModel.RuntimeStatusTone);
        Assert.Equal(Visibility.Visible, viewModel.RuntimeStatusWarningVisibility);
        Assert.Equal(Visibility.Collapsed, viewModel.RuntimeStatusSuccessVisibility);
        Assert.Equal(Visibility.Visible, viewModel.RuntimeStatusVisibility);

        gateway.Publish(new TickOutcome
        {
            Delta = new ProcessDeltaBatch
            {
                Seq = 2,
                Upserts = [],
                Exits = [],
            },
            Health = new RuntimeHealth
            {
                Seq = 2,
            },
            EmitTelemetryDelta = false,
        });

        Assert.Equal("Collector Warning", viewModel.RuntimeStatusTitle);
        Assert.Equal("bridge warning", viewModel.RuntimeStatusSummary);
    }
    [Fact]
    public async Task RuntimeStatusPresentation_KeepsDegradeModeInFooterOnly()
    {
        TestRuntimeEventGateway gateway = new();
        MonitoringShellViewModel viewModel = await CreateBootstrappedViewModelAsync(gateway);

        gateway.Publish(new TickOutcome
        {
            Delta = new ProcessDeltaBatch
            {
                Seq = 1,
                Upserts = [],
                Exits = [],
            },
            Health = new RuntimeHealth
            {
                Seq = 1,
                JitterP95Ms = 18,
                DegradeMode = true,
            },
            EmitTelemetryDelta = false,
        });

        Assert.Contains("degrade ON", viewModel.RuntimeHealthStatus, StringComparison.OrdinalIgnoreCase);
        Assert.Equal("Runtime Healthy", viewModel.RuntimeStatusTitle);
        Assert.Equal(RuntimeStatusTone.Success, viewModel.RuntimeStatusTone);
        Assert.Equal(Visibility.Collapsed, viewModel.RuntimeStatusWarningVisibility);
        Assert.Equal(Visibility.Visible, viewModel.RuntimeStatusSuccessVisibility);
        Assert.Equal(Visibility.Collapsed, viewModel.RuntimeStatusVisibility);
    }

    [Fact]
    public async Task SelectingProcess_SwitchesInspectorToProcessLayout_AndCompactsSummaryCards()
    {
        TestRuntimeEventGateway gateway = new();
        MonitoringShellViewModel viewModel = await CreateBootstrappedViewModelAsync(gateway);
        ProcessSample row = Sample(pid: 901, startTime: 9_010, access: AccessState.Full) with { Name = "batcave.exe" };

        gateway.RaiseDelta(1, [row], []);
        await viewModel.SelectRowAsync(row, CancellationToken.None);

        Assert.Equal(InspectorLayoutMode.ProcessInspector, viewModel.InspectorLayoutMode);
        Assert.True(viewModel.IsProcessInspector);
        Assert.False(viewModel.IsSystemOverview);
        Assert.Equal("PROCESS VIEW", viewModel.InspectorOverviewEyebrow);
        Assert.Equal(viewModel.DetailTitle, viewModel.InspectorContextTitle);
        Assert.Equal(Visibility.Visible, viewModel.ProcessSummarySectionVisibility);
        Assert.Equal(Visibility.Collapsed, viewModel.SystemSummarySectionVisibility);
        Assert.Equal(4, viewModel.SummaryStatCards.Count);
        Assert.Equal(960, viewModel.InspectorChartMaxWidth);
        Assert.Equal(232, viewModel.SummaryStatCardWidth);
    }

    [Fact]
    public async Task ProcessInspector_MiniTrendBuffersReuseExistingArraysAcrossTelemetryTicks()
    {
        TestRuntimeEventGateway gateway = new();
        MonitoringShellViewModel viewModel = await CreateBootstrappedViewModelAsync(gateway);
        ProcessSample row = Sample(pid: 902, startTime: 9_020, access: AccessState.Full) with
        {
            CpuPct = 11.5,
            RssBytes = 8 * 1024UL * 1024UL,
            IoReadBps = 2 * 1024UL,
            IoWriteBps = 4 * 1024UL,
            OtherIoBps = 6 * 1024UL,
        };

        gateway.RaiseDelta(1, [row], []);
        await viewModel.SelectRowAsync(row, CancellationToken.None);

        GlobalResourceRowViewState memoryRow = Assert.Single(viewModel.GlobalResourceRows, item => item.Kind == GlobalResourceKind.Memory);
        GlobalResourceRowViewState diskRow = Assert.Single(viewModel.GlobalResourceRows, item => item.Kind == GlobalResourceKind.Disk);
        GlobalResourceRowViewState networkRow = Assert.Single(viewModel.GlobalResourceRows, item => item.Kind == GlobalResourceKind.Network);
        double[] initialMemoryTrend = memoryRow.MiniTrendValues;
        double[] initialDiskTrend = diskRow.MiniTrendValues;
        double[] initialNetworkTrend = networkRow.MiniTrendValues;

        ProcessSample updated = row with
        {
            Seq = 2,
            TsMs = 2,
            CpuPct = 17.5,
            RssBytes = 24 * 1024UL * 1024UL,
            IoReadBps = 8 * 1024UL,
            IoWriteBps = 16 * 1024UL,
            OtherIoBps = 10 * 1024UL,
        };

        gateway.RaiseDelta(2, [updated], []);

        Assert.Same(initialMemoryTrend, memoryRow.MiniTrendValues);
        Assert.Same(initialDiskTrend, diskRow.MiniTrendValues);
        Assert.Same(initialNetworkTrend, networkRow.MiniTrendValues);
        Assert.Equal((double)updated.RssBytes, memoryRow.MiniTrendValues[^1]);
        Assert.Equal((double)(updated.IoReadBps + updated.IoWriteBps), diskRow.MiniTrendValues[^1]);
        Assert.Equal(updated.OtherIoBps * 8d, networkRow.MiniTrendValues[^1]);
    }

    [Fact]
    public async Task SelectingProcess_RaisesSummaryVisibilityProperties_AndShowsMetadataImmediately()
    {
        TestMetadataProvider metadata = new((pid, _, _) => Task.FromResult<ProcessMetadata?>(new ProcessMetadata
        {
            Pid = pid,
            ParentPid = 7,
            ExecutablePath = @"C:\\Apps\\demo.exe",
            CommandLine = "demo --flag",
        }));
        TestRuntimeEventGateway gateway = new();
        MonitoringShellViewModel viewModel = await CreateBootstrappedViewModelAsync(gateway, metadataProvider: metadata);

        List<string> raised = [];
        viewModel.PropertyChanged += (_, args) =>
        {
            if (!string.IsNullOrWhiteSpace(args.PropertyName))
            {
                raised.Add(args.PropertyName!);
            }
        };

        ProcessSample row = Sample(pid: 902, startTime: 9_020, access: AccessState.Full) with { Name = "demo.exe" };
        gateway.RaiseDelta(1, [row], []);
        raised.Clear();

        await viewModel.SelectRowAsync(row, CancellationToken.None);

        Assert.Contains(nameof(MonitoringShellViewModel.ProcessSummarySectionVisibility), raised);
        Assert.Contains(nameof(MonitoringShellViewModel.SystemSummarySectionVisibility), raised);
        Assert.Equal(Visibility.Visible, viewModel.ProcessSummarySectionVisibility);
        Assert.Equal(Visibility.Collapsed, viewModel.SystemSummarySectionVisibility);
        Assert.Equal("Metadata loaded.", viewModel.MetadataStatus);
        Assert.Contains("demo.exe", viewModel.DetailsPanePrimaryText, StringComparison.OrdinalIgnoreCase);
    }

    [Fact]
    public async Task ClearSelection_RaisesSummaryVisibilityProperties_AndRestoresSystemOverview()
    {
        TestMetadataProvider metadata = new((pid, _, _) => Task.FromResult<ProcessMetadata?>(new ProcessMetadata
        {
            Pid = pid,
            ParentPid = 7,
            ExecutablePath = @"C:\\Apps\\demo.exe",
            CommandLine = "demo --flag",
        }));
        TestRuntimeEventGateway gateway = new();
        MonitoringShellViewModel viewModel = await CreateBootstrappedViewModelAsync(gateway, metadataProvider: metadata);

        ProcessSample row = Sample(pid: 903, startTime: 9_030, access: AccessState.Full);
        gateway.RaiseDelta(1, [row], []);
        await viewModel.SelectRowAsync(row, CancellationToken.None);

        List<string> raised = [];
        viewModel.PropertyChanged += (_, args) =>
        {
            if (!string.IsNullOrWhiteSpace(args.PropertyName))
            {
                raised.Add(args.PropertyName!);
            }
        };

        raised.Clear();
        viewModel.ClearSelection();

        Assert.Contains(nameof(MonitoringShellViewModel.ProcessSummarySectionVisibility), raised);
        Assert.Contains(nameof(MonitoringShellViewModel.SystemSummarySectionVisibility), raised);
        Assert.Equal(Visibility.Collapsed, viewModel.ProcessSummarySectionVisibility);
        Assert.Equal(Visibility.Visible, viewModel.SystemSummarySectionVisibility);
        Assert.Equal("System Overview", viewModel.InspectorContextTitle);
        Assert.Equal("Select a process to load metadata.", viewModel.MetadataStatus);
    }

    [Fact]
    public async Task SelectingProcess_ImmediatelyExposesProcessSummaryMetadataWhileSummaryTabStaysActive()
    {
        TestMetadataProvider metadata = new((pid, _, _) => Task.FromResult<ProcessMetadata?>(new ProcessMetadata
        {
            Pid = pid,
            ParentPid = 42,
            ExecutablePath = @"C:\\Apps\\batcave.exe",
            CommandLine = "batcave --watch",
        }));
        TestRuntimeEventGateway gateway = new();
        MonitoringShellViewModel viewModel = await CreateBootstrappedViewModelAsync(gateway, metadataProvider: metadata);
        ProcessSample row = Sample(pid: 902, startTime: 9_020, access: AccessState.Full) with { Name = "batcave.exe" };

        gateway.RaiseDelta(1, [row], []);
        Assert.True(viewModel.IsSummarySectionSelected);

        await viewModel.SelectRowAsync(row, CancellationToken.None);

        Assert.Equal(Visibility.Visible, viewModel.ProcessSummarySectionVisibility);
        Assert.Equal(Visibility.Collapsed, viewModel.SystemSummarySectionVisibility);
        Assert.Equal("Metadata loaded.", viewModel.MetadataStatus);
        Assert.Contains("batcave.exe", viewModel.DetailsPanePrimaryText, StringComparison.OrdinalIgnoreCase);
        Assert.Equal("PROCESS VIEW", viewModel.InspectorOverviewEyebrow);
    }

    [Fact]
    public async Task AdminToggle_WhenRestartThrows_RestartsRuntimeLoopAndSurfacesError()
    {
        TestRuntimeEventGateway gateway = new();
        MonitoringRuntime runtime = new(
            new ThrowingOnAdminCollectorFactory(),
            new DeltaTelemetryPipeline(),
            new InMemoryStateStore(),
            new IncrementalSortIndexEngine(),
            new TestPersistenceStore(),
            new RuntimeHostOptions());
        RuntimeLoopService loopService = new(runtime, intervalOverride: TimeSpan.FromMilliseconds(5));
        RuntimeHealthService runtimeHealthService = new();
        MonitoringShellViewModel viewModel = new(
            CreatePassedGate(),
            runtime,
            loopService,
            gateway,
            runtimeHealthService,
            CreateNullMetadataProvider(),
            TestSystemGlobalMetricsSampler.Default);

        try
        {
            await runtime.InitializeAsync(CancellationToken.None);
            int tickCount = 0;
            loopService.TickCompleted += (_, _) => Interlocked.Increment(ref tickCount);
            loopService.Start(loopService.CurrentGeneration);
            await viewModel.BootstrapAsync(CancellationToken.None);
            await Task.Delay(30);
            int beforeToggle = Volatile.Read(ref tickCount);

            await viewModel.ToggleAdminModeAsync(true, CancellationToken.None);
            await Task.Delay(30);

            Assert.True(viewModel.HasAdminModeError);
            Assert.Contains("toggle boom", viewModel.AdminModeError, StringComparison.OrdinalIgnoreCase);
            Assert.False(viewModel.AdminModeEnabled);
            Assert.False(viewModel.AdminModePending);
            Assert.True(Volatile.Read(ref tickCount) > beforeToggle, "Expected runtime loop to keep ticking after failed restart.");
        }
        finally
        {
            loopService.StopAndAdvanceGeneration();
            runtime.Dispose();
        }
    }

    [Fact]
    public async Task AdminToggle_WhenRestartReturnsWarning_SurfacesWarningImmediately()
    {
        TestRuntimeEventGateway gateway = new();
        MonitoringRuntime runtime = new(
            new WarningOnAdminCollectorFactory(),
            new DeltaTelemetryPipeline(),
            new InMemoryStateStore(),
            new IncrementalSortIndexEngine(),
            new TestPersistenceStore(),
            new RuntimeHostOptions());
        RuntimeLoopService loopService = new(runtime, intervalOverride: TimeSpan.FromMilliseconds(5));
        RuntimeHealthService runtimeHealthService = new();
        MonitoringShellViewModel viewModel = new(
            CreatePassedGate(),
            runtime,
            loopService,
            gateway,
            runtimeHealthService,
            CreateNullMetadataProvider(),
            TestSystemGlobalMetricsSampler.Default);

        try
        {
            await runtime.InitializeAsync(CancellationToken.None);
            await viewModel.BootstrapAsync(CancellationToken.None);

            await viewModel.ToggleAdminModeAsync(true, CancellationToken.None);

            Assert.False(viewModel.AdminModeEnabled);
            Assert.True(viewModel.HasAdminModeError);
            Assert.Contains("bridge warning", viewModel.AdminModeError, StringComparison.OrdinalIgnoreCase);
            Assert.Contains("bridge warning", viewModel.RuntimeHealthStatus, StringComparison.OrdinalIgnoreCase);
        }
        finally
        {
            loopService.StopAndAdvanceGeneration();
            runtime.Dispose();
        }
    }

    [Fact]
    public async Task CollectorWarning_ClearsAfterQuietTickWindow()
    {
        TestRuntimeEventGateway gateway = new();
        MonitoringShellViewModel viewModel = await CreateBootstrappedViewModelAsync(gateway);

        gateway.PublishWarning(new CollectorWarning
        {
            Seq = 2,
            Message = "bridge warning",
        });

        Assert.True(viewModel.HasAdminModeError);
        Assert.Contains("bridge warning", viewModel.RuntimeHealthStatus, StringComparison.OrdinalIgnoreCase);

        for (ulong seq = 3; seq <= 10; seq++)
        {
            gateway.Publish(new TickOutcome
            {
                Delta = new ProcessDeltaBatch
                {
                    Seq = seq,
                    Upserts = [],
                    Exits = [],
                },
                Health = new RuntimeHealth
                {
                    Seq = seq,
                },
                EmitTelemetryDelta = false,
            });
        }

        Assert.False(viewModel.HasAdminModeError);
        Assert.DoesNotContain("last warning", viewModel.RuntimeHealthStatus, StringComparison.OrdinalIgnoreCase);
    }

    [Fact]
    public async Task MetadataSelection_UsesCacheAndSurfacesNonFatalErrors()
    {
        TestMetadataProvider metadata = new((pid, _, _) => Task.FromResult<ProcessMetadata?>(new ProcessMetadata
        {
            Pid = pid,
            ParentPid = 1,
            CommandLine = "demo --flag",
            ExecutablePath = "C:\\demo.exe",
        }));
        TestRuntimeEventGateway gateway = new();
        MonitoringShellViewModel viewModel = await CreateBootstrappedViewModelAsync(gateway, metadataProvider: metadata);

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
    public async Task Bootstrap_LoadsPersistedMetricTrendWindowSetting()
    {
        TestRuntimeEventGateway gateway = new();
        MonitoringShellViewModel viewModel = await CreateBootstrappedViewModelAsync(
            gateway,
            settings: new UserSettings
            {
                MetricTrendWindowSeconds = 120,
                AdminPreferenceInitialized = true,
            });

        Assert.Equal(120, viewModel.MetricTrendWindowSeconds);
        Assert.False(viewModel.IsTrendWindow60Selected);
        Assert.True(viewModel.IsTrendWindow120Selected);
    }

    [Fact]
    public async Task Bootstrap_NormalizesCompactSortState_WhenPersistedSortColumnIsNonCompact()
    {
        TestRuntimeEventGateway gateway = new();
        MonitoringShellViewModel viewModel = await CreateBootstrappedViewModelAsync(
            gateway,
            settings: new UserSettings
            {
                SortCol = SortColumn.Handles,
                SortDir = SortDirection.Asc,
                AdminPreferenceInitialized = true,
            });

        Assert.Equal(SortColumn.CpuPct, viewModel.CurrentSortColumn);
        Assert.Equal(SortDirection.Asc, viewModel.CurrentSortDirection);
        Assert.False(viewModel.IsCompactNameSortActive);
        Assert.True(viewModel.IsCompactCpuSortActive);
        Assert.False(viewModel.IsCompactMemorySortActive);
        Assert.False(viewModel.IsCompactDiskSortActive);
        Assert.False(viewModel.IsCompactNetworkSortActive);

        viewModel.CompactSortHeaderCommand.Execute("DiskBps");

        Assert.Equal(SortColumn.DiskBps, viewModel.CurrentSortColumn);
        Assert.True(viewModel.IsCompactDiskSortActive);
    }

    [Fact]
    public async Task CompactSortHeader_DiskUsesDiskBpsSortColumn()
    {
        TestRuntimeEventGateway gateway = new();
        MonitoringShellViewModel viewModel = await CreateBootstrappedViewModelAsync(gateway);

        viewModel.CompactSortHeaderCommand.Execute("DiskBps");

        Assert.Equal(SortColumn.DiskBps, viewModel.CurrentSortColumn);
        Assert.True(viewModel.IsCompactDiskSortActive);
    }

    [Fact]
    public async Task CompactTableTotals_UseGlobalPercentagesAndRowDiskNetworkFormatting()
    {
        TestRuntimeEventGateway gateway = new();
        TestSystemGlobalMetricsSampler sampler = new(CreateSystemGlobalMetricsSample(
            tsMs: 100,
            cpuPct: 34.2,
            memoryUsedBytes: 58 * 1024UL * 1024UL,
            diskReadBps: 0,
            diskWriteBps: 0,
            otherIoBps: 0,
            memorySnapshot: new SystemGlobalMemorySnapshot
            {
                UsedBytes = 58 * 1024UL * 1024UL,
                TotalBytes = 100 * 1024UL * 1024UL,
            },
            diskSnapshots:
            [
                new SystemGlobalDiskSnapshot
                {
                    DiskId = "d0",
                    DisplayName = "Disk 0",
                    ActiveTimePct = 24.4,
                },
            ],
            networkSnapshots:
            [
                new SystemGlobalNetworkSnapshot
                {
                    AdapterId = "n0",
                    DisplayName = "Ethernet",
                    SendBps = 20_000_000UL,
                    ReceiveBps = 40_000_000UL,
                    LinkSpeedBps = 1_000_000_000UL,
                },
            ]));

        MonitoringShellViewModel viewModel = await CreateBootstrappedViewModelAsync(
            gateway,
            systemGlobalMetricsSampler: sampler);

        ProcessSample first = Sample(pid: 210, startTime: 2100, access: AccessState.Full) with
        {
            CpuPct = 4.5,
            RssBytes = 2 * 1024UL * 1024UL,
            IoReadBps = 4000UL,
            IoWriteBps = 1000UL,
            OtherIoBps = 2500UL,
        };
        ProcessSample second = Sample(pid: 211, startTime: 2110, access: AccessState.Full) with
        {
            CpuPct = 10.25,
            RssBytes = 3 * 1024UL * 1024UL,
            IoReadBps = 3000UL,
            IoWriteBps = 2000UL,
            OtherIoBps = 3500UL,
        };

        gateway.RaiseDelta(1, [first, second], []);
        gateway.RaiseDelta(2, [first with { Seq = 2, TsMs = 2 }, second with { Seq = 2, TsMs = 2 }], []);

        Assert.Equal("2 processes", viewModel.CompactNameTotalLabel);
        Assert.Equal("34%", viewModel.CompactCpuTotalLabel);
        Assert.Equal("58%", viewModel.CompactMemoryTotalLabel);
        Assert.Equal("24%", viewModel.CompactDiskTotalLabel);
        Assert.Equal("6%", viewModel.CompactNetworkTotalLabel);

        ProcessRowViewState firstRow = GetVisibleRows(viewModel).Single(row => row.Pid == first.Pid);
        Assert.Equal(ValueFormat.FormatRate(first.IoReadBps + first.IoWriteBps), firstRow.DiskText);
        Assert.Equal(ValueFormat.FormatBitsRateFromBytes(first.OtherIoBps), firstRow.NetworkText);
    }

    [Fact]
    public async Task TelemetryDelta_DoesNotRaiseCompactSortVisualProperties()
    {
        TestRuntimeEventGateway gateway = new();
        MonitoringShellViewModel viewModel = await CreateBootstrappedViewModelAsync(gateway);

        HashSet<string> compactSortVisualProperties =
        [
            nameof(MonitoringShellViewModel.CompactNameSortLabel),
            nameof(MonitoringShellViewModel.CompactCpuSortLabel),
            nameof(MonitoringShellViewModel.CompactMemorySortLabel),
            nameof(MonitoringShellViewModel.CompactDiskSortLabel),
            nameof(MonitoringShellViewModel.CompactNetworkSortLabel),
            nameof(MonitoringShellViewModel.IsCompactNameSortActive),
            nameof(MonitoringShellViewModel.IsCompactCpuSortActive),
            nameof(MonitoringShellViewModel.IsCompactMemorySortActive),
            nameof(MonitoringShellViewModel.IsCompactDiskSortActive),
            nameof(MonitoringShellViewModel.IsCompactNetworkSortActive),
            nameof(MonitoringShellViewModel.CompactNameSortForeground),
            nameof(MonitoringShellViewModel.CompactCpuSortForeground),
            nameof(MonitoringShellViewModel.CompactMemorySortForeground),
            nameof(MonitoringShellViewModel.CompactDiskSortForeground),
            nameof(MonitoringShellViewModel.CompactNetworkSortForeground),
            nameof(MonitoringShellViewModel.CompactNameColumnBackground),
            nameof(MonitoringShellViewModel.CompactCpuColumnBackground),
            nameof(MonitoringShellViewModel.CompactMemoryColumnBackground),
            nameof(MonitoringShellViewModel.CompactDiskColumnBackground),
            nameof(MonitoringShellViewModel.CompactNetworkColumnBackground),
        ];

        List<string> raised = [];
        viewModel.PropertyChanged += (_, args) =>
        {
            if (!string.IsNullOrWhiteSpace(args.PropertyName))
            {
                raised.Add(args.PropertyName!);
            }
        };

        ProcessSample row = Sample(pid: 990, startTime: 9900, access: AccessState.Full) with { CpuPct = 10 };
        gateway.RaiseDelta(1, [row], []);
        raised.Clear();

        gateway.RaiseDelta(2, [row with { Seq = 2, TsMs = 2, CpuPct = 20 }], []);

        Assert.DoesNotContain(raised, compactSortVisualProperties.Contains);
    }

    [Fact]
    public async Task MetricTrendWindowSelected_SwitchesDisplayedTrendLength()
    {
        TestRuntimeEventGateway gateway = new();
        MonitoringShellViewModel viewModel = await CreateBootstrappedViewModelAsync(gateway);

        ProcessSample current = Sample(pid: 77, startTime: 7700, access: AccessState.Full) with { CpuPct = 0 };
        for (ulong seq = 1; seq <= 90; seq++)
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

        Assert.Equal(60, viewModel.CpuMetricTrendValues.Length);
        viewModel.MetricTrendWindowSelectedCommand.Execute("120");
        Assert.Equal(120, viewModel.MetricTrendWindowSeconds);
        Assert.Equal(120, viewModel.CpuMetricTrendValues.Length);
        Assert.All(viewModel.CpuMetricTrendValues.Take(30), value => Assert.Equal(0d, value));
        Assert.Equal(1d, viewModel.CpuMetricTrendValues[30]);
        Assert.Equal(90d, viewModel.CpuMetricTrendValues[^1]);
        Assert.True(viewModel.IsTrendWindow120Selected);
    }

    [Fact]
    public async Task MetricFocusSelectedCommand_UpdatesExpandedMetricSeriesAndLabels()
    {
        TestRuntimeEventGateway gateway = new();
        MonitoringShellViewModel viewModel = await CreateBootstrappedViewModelAsync(gateway);

        ProcessSample current = Sample(pid: 78, startTime: 7800, access: AccessState.Full);
        for (ulong seq = 1; seq <= 8; seq++)
        {
            current = current with
            {
                Seq = seq,
                TsMs = seq,
                CpuPct = 2 + seq,
                RssBytes = 1024UL * seq,
                IoReadBps = 4096UL * seq,
                IoWriteBps = 8192UL * seq,
                OtherIoBps = 2048UL * seq,
            };
            gateway.RaiseDelta(seq, [current], []);
        }

        await viewModel.SelectRowAsync(current, CancellationToken.None);

        viewModel.MetricFocusSelectedCommand.Execute("Memory");
        Assert.Equal("Memory Trend", viewModel.ExpandedMetricTitle);
        Assert.Contains("RSS", viewModel.ExpandedMetricValue, StringComparison.Ordinal);
        Assert.Equal(viewModel.MemoryMetricTrendValues.Length, viewModel.ExpandedMetricTrendValues.Length);

        viewModel.MetricFocusSelectedCommand.Execute("IoRead");
        Assert.Equal("Disk Read Trend", viewModel.ExpandedMetricTitle);
        Assert.Contains("read", viewModel.ExpandedMetricValue, StringComparison.OrdinalIgnoreCase);
        Assert.Equal(viewModel.IoReadMetricTrendValues.Length, viewModel.ExpandedMetricTrendValues.Length);
    }

    [Fact]
    public async Task TelemetryDelta_RefreshesVisibleRowsWithoutCollectionReset()
    {
        TestRuntimeEventGateway gateway = new();
        MonitoringShellViewModel viewModel = await CreateBootstrappedViewModelAsync(gateway);

        ProcessSample row = Sample(pid: 20, startTime: 2000, access: AccessState.Full);
        gateway.RaiseDelta(1, [row], []);
        ProcessRowViewState firstVisible = GetVisibleRow(viewModel, 0);

        ProcessSample updatedRow = row with { Seq = 2, TsMs = 2, CpuPct = 67.4 };
        gateway.RaiseDelta(2, [updatedRow], []);

        Assert.Single(GetVisibleRows(viewModel));
        Assert.Same(firstVisible, GetVisibleRow(viewModel, 0));
        Assert.Equal(updatedRow.CpuPct, GetVisibleRow(viewModel, 0).Sample.CpuPct);
    }

    [Fact]
    public async Task TelemetryDelta_Reorder_UsesMoveOperationsWithoutReplace()
    {
        TestRuntimeEventGateway gateway = new();
        MonitoringShellViewModel viewModel = await CreateBootstrappedViewModelAsync(gateway);

        ProcessSample high = Sample(pid: 100, startTime: 1000, access: AccessState.Full) with { CpuPct = 90 };
        ProcessSample mid = Sample(pid: 101, startTime: 1001, access: AccessState.Full) with { CpuPct = 50 };
        ProcessSample low = Sample(pid: 102, startTime: 1002, access: AccessState.Full) with { CpuPct = 10 };
        gateway.RaiseDelta(1, [high, mid, low], []);
        ProcessRowViewState highState = GetVisibleRows(viewModel).Single(row => row.Pid == high.Pid);
        ProcessRowViewState midState = GetVisibleRows(viewModel).Single(row => row.Pid == mid.Pid);
        ProcessRowViewState lowState = GetVisibleRows(viewModel).Single(row => row.Pid == low.Pid);

        ProcessSample highNowLow = high with { Seq = 2, TsMs = 2, CpuPct = 5 };
        ProcessSample midNowHigh = mid with { Seq = 2, TsMs = 2, CpuPct = 95 };
        ProcessSample lowNowMid = low with { Seq = 2, TsMs = 2, CpuPct = 55 };
        gateway.RaiseDelta(2, [highNowLow, midNowHigh, lowNowMid], []);

        Assert.Collection(
            GetVisibleRows(viewModel),
            row => Assert.Equal(mid.Pid, row.Pid),
            row => Assert.Equal(low.Pid, row.Pid),
            row => Assert.Equal(high.Pid, row.Pid));
        Assert.Same(midState, GetVisibleRow(viewModel, 0));
        Assert.Same(lowState, GetVisibleRow(viewModel, 1));
        Assert.Same(highState, GetVisibleRow(viewModel, 2));
    }

    [Fact]
    public async Task TelemetryDelta_Reorder_DoesNotRaiseCollectionReset()
    {
        TestRuntimeEventGateway gateway = new();
        MonitoringShellViewModel viewModel = await CreateBootstrappedViewModelAsync(gateway);

        List<NotifyCollectionChangedAction> actions = [];
        ((INotifyCollectionChanged)viewModel.VisibleRows).CollectionChanged += (_, args) =>
        {
            actions.Add(args.Action);
        };

        ProcessSample high = Sample(pid: 170, startTime: 1700, access: AccessState.Full) with { CpuPct = 90 };
        ProcessSample low = Sample(pid: 171, startTime: 1701, access: AccessState.Full) with { CpuPct = 10 };
        gateway.RaiseDelta(1, [high, low], []);
        actions.Clear();

        ProcessSample highNowLow = high with { Seq = 2, TsMs = 2, CpuPct = 5 };
        ProcessSample lowNowHigh = low with { Seq = 2, TsMs = 2, CpuPct = 95 };
        gateway.RaiseDelta(2, [highNowLow, lowNowHigh], []);

        Assert.DoesNotContain(NotifyCollectionChangedAction.Reset, actions);
        Assert.Contains(NotifyCollectionChangedAction.Move, actions);
    }

    [Fact]
    public async Task TelemetryDelta_WhenActiveSortKeyDoesNotChange_UpdatesRowWithoutCollectionChurn()
    {
        TestRuntimeEventGateway gateway = new();
        MonitoringShellViewModel viewModel = await CreateBootstrappedViewModelAsync(gateway);

        ProcessSample beta = Sample(pid: 180, startTime: 1800, access: AccessState.Full) with { Name = "beta", CpuPct = 15 };
        ProcessSample alpha = Sample(pid: 181, startTime: 1801, access: AccessState.Full) with { Name = "alpha", CpuPct = 70 };
        gateway.RaiseDelta(1, [beta, alpha], []);

        viewModel.ChangeSort(SortColumn.Name);
        ProcessRowViewState betaState = GetVisibleRows(viewModel).Single(row => row.Pid == beta.Pid);

        List<NotifyCollectionChangedAction> actions = [];
        ((INotifyCollectionChanged)viewModel.VisibleRows).CollectionChanged += (_, args) =>
        {
            actions.Add(args.Action);
        };

        gateway.RaiseDelta(2, [beta with { Seq = 2, TsMs = 2, CpuPct = 88 }, alpha with { Seq = 2, TsMs = 2, CpuPct = 5 }], []);

        Assert.Empty(actions);
        Assert.Same(betaState, GetVisibleRows(viewModel).Single(row => row.Pid == beta.Pid));
        Assert.Equal(88d, betaState.CpuPct);
    }

    [Fact]
    public async Task TelemetryDelta_SelectedProcessSameIdentity_UpdatesInspectorWithoutSelectionChromeChurn()
    {
        TestRuntimeEventGateway gateway = new();
        MonitoringShellViewModel viewModel = await CreateBootstrappedViewModelAsync(gateway);

        ProcessSample current = Sample(pid: 1810, startTime: 18_100, access: AccessState.Full) with
        {
            Name = "BatCave.exe",
            CpuPct = 12.5,
            RssBytes = 24 * 1024UL * 1024UL,
            IoReadBps = 1024UL,
            IoWriteBps = 2048UL,
            OtherIoBps = 4096UL,
        };

        gateway.RaiseDelta(1, [current], []);
        await viewModel.SelectRowAsync(current, CancellationToken.None);

        List<string> raised = [];
        viewModel.PropertyChanged += (_, args) =>
        {
            if (!string.IsNullOrWhiteSpace(args.PropertyName))
            {
                raised.Add(args.PropertyName!);
            }
        };

        raised.Clear();
        current = current with
        {
            Seq = 2,
            TsMs = 2,
            CpuPct = 44.5,
            RssBytes = 48 * 1024UL * 1024UL,
            IoReadBps = 8 * 1024UL,
            IoWriteBps = 16 * 1024UL,
            OtherIoBps = 10 * 1024UL,
        };
        gateway.RaiseDelta(2, [current], []);

        Assert.DoesNotContain(nameof(MonitoringShellViewModel.HasSelection), raised);
        Assert.DoesNotContain(nameof(MonitoringShellViewModel.DetailTitle), raised);
        Assert.DoesNotContain(nameof(MonitoringShellViewModel.InspectorLayoutMode), raised);
        Assert.DoesNotContain(nameof(MonitoringShellViewModel.ProcessSummarySectionVisibility), raised);
        Assert.DoesNotContain(nameof(MonitoringShellViewModel.SystemSummarySectionVisibility), raised);
        Assert.DoesNotContain(nameof(MonitoringShellViewModel.ProcessPrimaryChartIdentityKey), raised);
        Assert.DoesNotContain(nameof(MonitoringShellViewModel.SummaryStatCards), raised);
        Assert.Contains(nameof(MonitoringShellViewModel.CpuMetricTrendValues), raised);
        Assert.Equal($"{current.CpuPct:F2}%", viewModel.CpuMetricChipValue);
        Assert.Equal(ValueFormat.FormatBytes(current.RssBytes), viewModel.MemoryMetricChipValue);
    }

    [Fact]
    public async Task TelemetryDelta_SelectedProcessSameIdentity_ReusesSummaryStatCardsCollection()
    {
        TestRuntimeEventGateway gateway = new();
        MonitoringShellViewModel viewModel = await CreateBootstrappedViewModelAsync(gateway);

        ProcessSample current = Sample(pid: 1811, startTime: 18_110, access: AccessState.Full) with
        {
            Name = "BatCave.exe",
            CpuPct = 10.5,
            RssBytes = 16 * 1024UL * 1024UL,
            IoReadBps = 1024UL,
            IoWriteBps = 2048UL,
            OtherIoBps = 4096UL,
        };

        gateway.RaiseDelta(1, [current], []);
        await viewModel.SelectRowAsync(current, CancellationToken.None);

        ObservableCollection<GlobalStatItemViewState> summaryCards = viewModel.SummaryStatCards;
        List<NotifyCollectionChangedAction> actions = [];
        List<string> propertyChanges = [];
        summaryCards.CollectionChanged += (_, args) => actions.Add(args.Action);
        viewModel.PropertyChanged += (_, args) =>
        {
            if (!string.IsNullOrWhiteSpace(args.PropertyName))
            {
                propertyChanges.Add(args.PropertyName!);
            }
        };

        current = current with
        {
            Seq = 2,
            TsMs = 2,
            CpuPct = 35.5,
            RssBytes = 32 * 1024UL * 1024UL,
            IoReadBps = 8 * 1024UL,
            IoWriteBps = 16 * 1024UL,
            OtherIoBps = 10 * 1024UL,
        };
        gateway.RaiseDelta(2, [current], []);

        Assert.Same(summaryCards, viewModel.SummaryStatCards);
        Assert.Empty(actions);
        Assert.DoesNotContain(nameof(MonitoringShellViewModel.SummaryStatCards), propertyChanges);
        Assert.Equal(4, viewModel.SummaryStatCards.Count);
    }

    [Fact]
    public async Task TelemetryDelta_FilteredView_IgnoresOffscreenSortKeyUpdates()
    {
        TestRuntimeEventGateway gateway = new();
        MonitoringShellViewModel viewModel = await CreateBootstrappedViewModelAsync(gateway);

        ProcessSample target = Sample(pid: 182, startTime: 1820, access: AccessState.Full) with { Name = "BatCave.exe", CpuPct = 12 };
        ProcessSample hidden = Sample(pid: 183, startTime: 1830, access: AccessState.Full) with { Name = "other.exe", CpuPct = 10 };
        gateway.RaiseDelta(1, [target, hidden], []);

        viewModel.FilterText = "BatCave.exe";
        await Task.Delay(220);
        Assert.Single(GetVisibleRows(viewModel));

        List<NotifyCollectionChangedAction> actions = [];
        ((INotifyCollectionChanged)viewModel.VisibleRows).CollectionChanged += (_, args) =>
        {
            actions.Add(args.Action);
        };

        gateway.RaiseDelta(2, [target with { Seq = 2, TsMs = 2 }, hidden with { Seq = 2, TsMs = 2, CpuPct = 99 }], []);

        Assert.Empty(actions);
        Assert.Single(GetVisibleRows(viewModel));
        Assert.Equal(target.Pid, GetVisibleRow(viewModel, 0).Pid);
    }

    [Fact]
    public async Task TelemetryDelta_CpuResort_IgnoresInvisibleJitterButReordersOnMeaningfulChange()
    {
        TestRuntimeEventGateway gateway = new();
        MonitoringShellViewModel viewModel = await CreateBootstrappedViewModelAsync(gateway);

        ProcessSample first = Sample(pid: 110, startTime: 1110, access: AccessState.Full) with { CpuPct = 1.0040 };
        ProcessSample second = Sample(pid: 111, startTime: 1111, access: AccessState.Full) with { CpuPct = 1.0030 };
        gateway.RaiseDelta(1, [first, second], []);
        Assert.Equal(first.Pid, GetVisibleRow(viewModel, 0).Pid);

        // Both values still render as 1.00%; hidden jitter should not churn order.
        ProcessSample firstJitter = first with { Seq = 2, TsMs = 2, CpuPct = 1.0031 };
        ProcessSample secondJitter = second with { Seq = 2, TsMs = 2, CpuPct = 1.0049 };
        gateway.RaiseDelta(2, [firstJitter, secondJitter], []);

        Assert.Equal(first.Pid, GetVisibleRow(viewModel, 0).Pid);

        // Meaningful displayed difference should still reorder.
        ProcessSample secondMeaningful = secondJitter with { Seq = 3, TsMs = 3, CpuPct = 1.0200 };
        gateway.RaiseDelta(3, [firstJitter with { Seq = 3, TsMs = 3 }, secondMeaningful], []);

        Assert.Equal(second.Pid, GetVisibleRow(viewModel, 0).Pid);
    }

    [Fact]
    public async Task TelemetryDelta_HeartbeatOnly_DoesNotMaintainRowMiniTrendAndKeepsRowInstance()
    {
        TestRuntimeEventGateway gateway = new();
        MonitoringShellViewModel viewModel = await CreateBootstrappedViewModelAsync(gateway);

        ProcessSample row = Sample(pid: 21, startTime: 2100, access: AccessState.Full) with { CpuPct = 10 };
        gateway.RaiseDelta(1, [row], []);
        ProcessSample varied = row with { Seq = 2, TsMs = 2, CpuPct = 20 };
        gateway.RaiseDelta(2, [varied], []);
        ProcessRowViewState firstVisible = GetVisibleRow(viewModel, 0);
        Assert.Empty(firstVisible.CpuTrendValues);

        ProcessSample heartbeatOnlyUpdate = varied with { Seq = 3, TsMs = 3, ParentPid = varied.ParentPid + 1, PrivateBytes = varied.PrivateBytes + 1 };
        gateway.RaiseDelta(3, [heartbeatOnlyUpdate], []);
        Assert.Empty(firstVisible.CpuTrendValues);

        ProcessSample strideHeartbeat = heartbeatOnlyUpdate with { Seq = 4, TsMs = 4, ParentPid = heartbeatOnlyUpdate.ParentPid + 1 };
        gateway.RaiseDelta(4, [strideHeartbeat], []);
        Assert.Empty(firstVisible.CpuTrendValues);

        Assert.Same(firstVisible, GetVisibleRow(viewModel, 0));
    }

    [Fact]
    public async Task SelectedProcessTrend_AdvancesOnEmptyDeltaWhenValueUnchanged()
    {
        TestRuntimeEventGateway gateway = new();
        MonitoringShellViewModel viewModel = await CreateBootstrappedViewModelAsync(gateway);

        ProcessSample row = Sample(pid: 90, startTime: 9000, access: AccessState.Full) with { CpuPct = 3.25 };
        gateway.RaiseDelta(1, [row], []);
        viewModel.SelectedVisibleRowBinding = GetVisibleRow(viewModel, 0);
        double[] before = [.. viewModel.CpuMetricTrendValues];

        gateway.RaiseDelta(2, [], []);
        double[] after = viewModel.CpuMetricTrendValues;

        Assert.Equal(60, before.Length);
        Assert.Equal(before.Length, after.Length);
        Assert.Equal(59, before.Count(static value => value == 0d));
        Assert.Equal(58, after.Count(static value => value == 0d));
        Assert.Equal(3.25, after[^2], 2);
        Assert.Equal(3.25, after[^1], 2);
    }

    [Fact]
    public async Task TableMiniChart_WhenNoLongerRendered_RemainsUnmaintained()
    {
        TestRuntimeEventGateway gateway = new();
        MonitoringShellViewModel viewModel = await CreateBootstrappedViewModelAsync(gateway);

        ProcessSample row = Sample(pid: 91, startTime: 9100, access: AccessState.Full) with { CpuPct = 4.5 };
        gateway.RaiseDelta(1, [row], []);
        ProcessSample varied = row with { Seq = 2, TsMs = 2, CpuPct = 11.0 };
        gateway.RaiseDelta(2, [varied], []);
        ProcessRowViewState rowState = GetVisibleRow(viewModel, 0);
        Assert.Empty(rowState.CpuTrendValues);

        gateway.RaiseDelta(3, [], []);
        Assert.Empty(rowState.CpuTrendValues);

        gateway.RaiseDelta(4, [], []);
        Assert.Empty(rowState.CpuTrendValues);
    }

    [Fact]
    public async Task TableMiniChart_FirstSample_NoLongerPrefillsMiniTrendWindow()
    {
        TestRuntimeEventGateway gateway = new();
        MonitoringShellViewModel viewModel = await CreateBootstrappedViewModelAsync(gateway);

        ProcessSample row = Sample(pid: 192, startTime: 19200, access: AccessState.Full) with { CpuPct = 12.0 };
        gateway.RaiseDelta(1, [row], []);
        ProcessRowViewState rowState = GetVisibleRow(viewModel, 0);
        Assert.Empty(rowState.CpuTrendValues);

        gateway.RaiseDelta(2, [], []);
        Assert.Empty(rowState.CpuTrendValues);

        gateway.RaiseDelta(3, [], []);
        Assert.Empty(rowState.CpuTrendValues);
    }

    [Fact]
    public async Task MetricHistory_CapsAtConfiguredLimit()
    {
        TestRuntimeEventGateway gateway = new();
        MonitoringShellViewModel viewModel = await CreateBootstrappedViewModelAsync(gateway);

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

        Assert.Single(GetVisibleRows(viewModel));
        Assert.Empty(GetVisibleRow(viewModel, 0).CpuTrendValues);
        Assert.Equal(60, viewModel.CpuMetricTrendValues.Length);
    }

    [Fact]
    public async Task NoSelection_UsesGlobalSummaryForDetailTrends()
    {
        TestRuntimeEventGateway gateway = new();
        MonitoringShellViewModel viewModel = await CreateBootstrappedViewModelAsync(gateway);

        ProcessSample first = Sample(pid: 40, startTime: 4000, access: AccessState.Full) with { CpuPct = 35 };
        gateway.RaiseDelta(1, [first], []);

        Assert.Null(viewModel.SelectedRow);
        Assert.Equal("Global System Values", viewModel.DetailTitle);
        Assert.NotEqual("0.00%", viewModel.CpuMetricChipValue);
        Assert.NotEmpty(viewModel.ExpandedMetricTrendValues);
    }

    [Fact]
    public async Task NoSelection_UsesSamplerValuesWhenAvailable()
    {
        TestRuntimeEventGateway gateway = new();
        TestSystemGlobalMetricsSampler sampler = new(
            CreateSystemGlobalMetricsSample(
                tsMs: 11,
                cpuPct: 88.8,
                memoryUsedBytes: 10 * 1024UL * 1024UL,
                diskReadBps: 2 * 1024UL,
                diskWriteBps: 4 * 1024UL,
                otherIoBps: 6 * 1024UL));
        MonitoringShellViewModel viewModel = await CreateBootstrappedViewModelAsync(
            gateway,
            systemGlobalMetricsSampler: sampler);

        gateway.RaiseDelta(1, [], []);
        gateway.RaiseDelta(2, [], []);

        Assert.Null(viewModel.SelectedRow);
        Assert.Equal("Global System Values", viewModel.DetailTitle);
        Assert.Equal("88.80%", viewModel.CpuMetricChipValue);
        Assert.Equal(ValueFormat.FormatBytes(10 * 1024UL * 1024UL), viewModel.MemoryMetricChipValue);
        Assert.Equal(ValueFormat.FormatRate(2 * 1024UL), viewModel.IoReadMetricChipValue);
        Assert.Equal(ValueFormat.FormatRate(4 * 1024UL), viewModel.IoWriteMetricChipValue);
        Assert.Equal(ValueFormat.FormatRate(6 * 1024UL), viewModel.OtherIoMetricChipValue);
        Assert.Equal("88.8% CPU", viewModel.DetailMetricValue);
    }

    [Fact]
    public async Task NoSelection_SlowSampler_FirstFrameDoesNotBlockUiApply()
    {
        TestRuntimeEventGateway gateway = new();
        ManualResetEventSlim releaseSampler = new(initialState: false);
        TestSystemGlobalMetricsSampler sampler = new(
            CreateSystemGlobalMetricsSample(
                tsMs: 100,
                cpuPct: 42.0,
                memoryUsedBytes: 8 * 1024UL * 1024UL,
                diskReadBps: 1024UL,
                diskWriteBps: 2048UL,
                otherIoBps: 4096UL));
        sampler.Handler = () =>
        {
            releaseSampler.Wait(TimeSpan.FromMilliseconds(800));
            return sampler.Current;
        };

        MonitoringShellViewModel viewModel = await CreateBootstrappedViewModelAsync(
            gateway,
            systemGlobalMetricsSampler: sampler);

        Stopwatch stopwatch = Stopwatch.StartNew();
        gateway.RaiseDelta(1, [], []);
        gateway.RaiseDelta(2, [], []);
        stopwatch.Stop();
        releaseSampler.Set();

        Assert.True(stopwatch.ElapsedMilliseconds < 400, $"UI apply blocked for {stopwatch.ElapsedMilliseconds} ms");
        Assert.Equal("Global System Values", viewModel.DetailTitle);
    }

    [Fact]
    public async Task NoSelection_PerMetricUnavailable_ShowsNaOnlyForUnavailableMetric()
    {
        TestRuntimeEventGateway gateway = new();
        TestSystemGlobalMetricsSampler sampler = new(
            CreateSystemGlobalMetricsSample(
                tsMs: 12,
                cpuPct: null,
                memoryUsedBytes: 12 * 1024UL * 1024UL,
                diskReadBps: 1024UL,
                diskWriteBps: 2048UL,
                otherIoBps: 4096UL));
        MonitoringShellViewModel viewModel = await CreateBootstrappedViewModelAsync(
            gateway,
            systemGlobalMetricsSampler: sampler);

        gateway.RaiseDelta(1, [], []);
        gateway.RaiseDelta(2, [], []);

        Assert.Equal("n/a", viewModel.CpuMetricChipValue);
        Assert.Equal(ValueFormat.FormatBytes(12 * 1024UL * 1024UL), viewModel.MemoryMetricChipValue);
        Assert.Equal(ValueFormat.FormatRate(1024UL), viewModel.IoReadMetricChipValue);
        Assert.Equal(ValueFormat.FormatRate(2048UL), viewModel.IoWriteMetricChipValue);
        Assert.Equal(ValueFormat.FormatRate(4096UL), viewModel.OtherIoMetricChipValue);

        viewModel.MetricFocusSelectedCommand.Execute("Cpu");
        Assert.Contains("n/a", viewModel.DetailMetricValue, StringComparison.OrdinalIgnoreCase);

        viewModel.MetricFocusSelectedCommand.Execute("Memory");
        Assert.DoesNotContain("n/a", viewModel.DetailMetricValue, StringComparison.OrdinalIgnoreCase);
    }

    [Fact]
    public async Task GlobalMode_ListsCpuMemoryDiskAndNetworkRows_EvenWhenIdle()
    {
        TestRuntimeEventGateway gateway = new();
        TestSystemGlobalMetricsSampler sampler = new(
            CreateSystemGlobalMetricsSample(
                tsMs: 200,
                cpuPct: 21.5,
                memoryUsedBytes: 29 * 1024UL * 1024UL * 1024UL,
                diskReadBps: 0,
                diskWriteBps: 0,
                otherIoBps: 0,
                memorySnapshot: new SystemGlobalMemorySnapshot
                {
                    TotalBytes = 64UL * 1024UL * 1024UL * 1024UL,
                    UsedBytes = 29UL * 1024UL * 1024UL * 1024UL,
                },
                diskSnapshots:
                [
                    new SystemGlobalDiskSnapshot
                    {
                        DiskId = "disk0",
                        DisplayName = "Disk 0 (C:)",
                        TypeLabel = "SSD (NVMe)",
                        ActiveTimePct = 0,
                        ReadBps = 0,
                        WriteBps = 0,
                    },
                ],
                networkSnapshots:
                [
                    new SystemGlobalNetworkSnapshot
                    {
                        AdapterId = "eth0",
                        DisplayName = "Ethernet",
                        AdapterName = "Ethernet 2",
                        ConnectionType = "Ethernet",
                        SendBps = 0,
                        ReceiveBps = 0,
                    },
                ]));
        MonitoringShellViewModel viewModel = await CreateBootstrappedViewModelAsync(
            gateway,
            systemGlobalMetricsSampler: sampler);

        gateway.RaiseDelta(1, [], []);
        gateway.RaiseDelta(2, [], []);

        Assert.True(viewModel.IsGlobalPerformanceMode);
        Assert.Contains(viewModel.GlobalResourceRows, row => row.Kind == GlobalResourceKind.Cpu);
        Assert.Contains(viewModel.GlobalResourceRows, row => row.Kind == GlobalResourceKind.Memory);
        Assert.Contains(viewModel.GlobalResourceRows, row => row.Kind == GlobalResourceKind.Disk);
        Assert.Contains(viewModel.GlobalResourceRows, row => row.Kind == GlobalResourceKind.Network);

        GlobalResourceRowViewState memoryRow = Assert.Single(viewModel.GlobalResourceRows, row => row.Kind == GlobalResourceKind.Memory);
        Assert.Equal("45%", memoryRow.Subtitle);
        Assert.Equal($"{ValueFormat.FormatBytes(29UL * 1024UL * 1024UL * 1024UL)} / {ValueFormat.FormatBytes(64UL * 1024UL * 1024UL * 1024UL)}", memoryRow.ValueText);
    }

    [Fact]
    public async Task GlobalMode_CpuSpeedChange_UpdatesRowSubtitleAndCpuDetailSpeed()
    {
        TestRuntimeEventGateway gateway = new();
        ManualResetEventSlim firstSampleStarted = new(initialState: false);
        ManualResetEventSlim firstSampleRelease = new(initialState: false);
        ManualResetEventSlim firstSampleCompleted = new(initialState: false);
        ManualResetEventSlim secondSampleStarted = new(initialState: false);
        ManualResetEventSlim secondSampleRelease = new(initialState: false);
        ManualResetEventSlim secondSampleCompleted = new(initialState: false);

        SystemGlobalMetricsSample firstSample = CreateSystemGlobalMetricsSample(
            tsMs: 501,
            cpuPct: 37.0,
            memoryUsedBytes: 8 * 1024UL * 1024UL * 1024UL,
            diskReadBps: 0,
            diskWriteBps: 0,
            otherIoBps: 0,
            cpuSnapshot: new SystemGlobalCpuSnapshot
            {
                ProcessorName = "CPU",
                SpeedMHz = 3200,
                BaseSpeedMHz = 3000,
            });
        SystemGlobalMetricsSample secondSample = CreateSystemGlobalMetricsSample(
            tsMs: 502,
            cpuPct: 63.0,
            memoryUsedBytes: 8 * 1024UL * 1024UL * 1024UL,
            diskReadBps: 0,
            diskWriteBps: 0,
            otherIoBps: 0,
            cpuSnapshot: new SystemGlobalCpuSnapshot
            {
                ProcessorName = "CPU",
                SpeedMHz = 4100,
                BaseSpeedMHz = 3000,
            });
        TestSystemGlobalMetricsSampler sampler = new(firstSample);
        int sampleCallCount = 0;
        sampler.Handler = () =>
        {
            int call = Interlocked.Increment(ref sampleCallCount);
            if (call == 1)
            {
                firstSampleStarted.Set();
                if (!firstSampleRelease.Wait(TimeSpan.FromSeconds(2)))
                {
                    throw new TimeoutException("Timed out waiting to release first global CPU sample.");
                }

                firstSampleCompleted.Set();
                return firstSample;
            }

            if (call == 2)
            {
                secondSampleStarted.Set();
                if (!secondSampleRelease.Wait(TimeSpan.FromSeconds(2)))
                {
                    throw new TimeoutException("Timed out waiting to release second global CPU sample.");
                }

                secondSampleCompleted.Set();
                return secondSample;
            }

            return secondSample;
        };

        try
        {
            MonitoringShellViewModel viewModel = await CreateBootstrappedViewModelAsync(
                gateway,
                systemGlobalMetricsSampler: sampler);
            Assert.True(firstSampleStarted.Wait(TimeSpan.FromSeconds(2)), "First sample task did not start.");
            firstSampleRelease.Set();
            Assert.True(firstSampleCompleted.Wait(TimeSpan.FromSeconds(2)), "First sample task did not complete.");

            gateway.RaiseDelta(1, [], []);

            GlobalResourceRowViewState cpuRow = Assert.Single(viewModel.GlobalResourceRows, row => row.Kind == GlobalResourceKind.Cpu);
            viewModel.SelectedGlobalResource = cpuRow;

            string firstSpeed = ValueFormat.FormatFrequencyGHz(firstSample.CpuSnapshot?.SpeedMHz);
            Assert.Equal("37%", cpuRow.Subtitle);
            Assert.Equal(firstSpeed, cpuRow.ValueText);
            Assert.Equal($"37% {firstSpeed}", viewModel.GlobalDetailCurrentValue);
            Assert.Contains(viewModel.GlobalDetailStats, item => item.Label == "Speed" && item.Value == firstSpeed);

            Assert.True(secondSampleStarted.Wait(TimeSpan.FromSeconds(2)), "Second sample task did not start.");
            secondSampleRelease.Set();
            Assert.True(secondSampleCompleted.Wait(TimeSpan.FromSeconds(2)), "Second sample task did not complete.");

            gateway.RaiseDelta(2, [], []);

            Assert.Same(cpuRow, Assert.Single(viewModel.GlobalResourceRows, row => row.Kind == GlobalResourceKind.Cpu));
            string secondSpeed = ValueFormat.FormatFrequencyGHz(secondSample.CpuSnapshot?.SpeedMHz);
            Assert.Equal("63%", cpuRow.Subtitle);
            Assert.Equal(secondSpeed, cpuRow.ValueText);
            Assert.Equal($"63% {secondSpeed}", viewModel.GlobalDetailCurrentValue);
            Assert.Contains(viewModel.GlobalDetailStats, item => item.Label == "Speed" && item.Value == secondSpeed);
            Assert.DoesNotContain(firstSpeed, viewModel.GlobalDetailCurrentValue, StringComparison.Ordinal);
            Assert.DoesNotContain(viewModel.GlobalDetailStats, item => item.Label == "Speed" && item.Value == firstSpeed);
        }
        finally
        {
            firstSampleRelease.Set();
            secondSampleRelease.Set();
        }
    }

    [Fact]
    public async Task GlobalMode_DiskSelection_PersistsAcrossTransientDiskSnapshotDrop()
    {
        TestRuntimeEventGateway gateway = new();
        TestSystemGlobalMetricsSampler sampler = new(
            CreateSystemGlobalMetricsSample(
                tsMs: 300,
                cpuPct: 12.0,
                memoryUsedBytes: 8 * 1024UL * 1024UL * 1024UL,
                diskReadBps: 1000,
                diskWriteBps: 2000,
                otherIoBps: 0,
                diskSnapshots:
                [
                    new SystemGlobalDiskSnapshot
                    {
                        DiskId = "C:",
                        DisplayName = "Disk 0 (C:)",
                        TypeLabel = "SSD (NVMe)",
                        ActiveTimePct = 7,
                        ReadBps = 1000,
                        WriteBps = 2000,
                    },
                ]));
        MonitoringShellViewModel viewModel = await CreateBootstrappedViewModelAsync(
            gateway,
            systemGlobalMetricsSampler: sampler);

        gateway.RaiseDelta(1, [], []);
        gateway.RaiseDelta(2, [], []);

        GlobalResourceRowViewState diskRow = Assert.Single(viewModel.GlobalResourceRows.Where(row => row.Kind == GlobalResourceKind.Disk));
        viewModel.SelectedGlobalResource = diskRow;
        Assert.Equal("7%", diskRow.ValueText);
        Assert.Equal("7.0%", viewModel.GlobalDetailCurrentValue);

        sampler.Current = CreateSystemGlobalMetricsSample(
            tsMs: 301,
            cpuPct: 13.0,
            memoryUsedBytes: 8 * 1024UL * 1024UL * 1024UL,
            diskReadBps: 0,
            diskWriteBps: 0,
            otherIoBps: 0,
            diskSnapshots: []);
        gateway.RaiseDelta(2, [], []);

        Assert.NotNull(viewModel.SelectedGlobalResource);
        Assert.Equal(diskRow.ResourceId, viewModel.SelectedGlobalResource!.ResourceId);
        Assert.Contains(viewModel.GlobalResourceRows, row => row.ResourceId == diskRow.ResourceId);
        Assert.Equal("Disk 0 (C:)", viewModel.GlobalDetailTitle);
    }

    [Fact]
    public async Task GlobalMode_DiskSelection_SubOneActiveTimeRowValue_RendersZeroPercentButKeepsDetailPrecision()
    {
        TestRuntimeEventGateway gateway = new();
        TestSystemGlobalMetricsSampler sampler = new(
            CreateSystemGlobalMetricsSample(
                tsMs: 301,
                cpuPct: 13.0,
                memoryUsedBytes: 8 * 1024UL * 1024UL * 1024UL,
                diskReadBps: 1200,
                diskWriteBps: 3400,
                otherIoBps: 0,
                diskSnapshots:
                [
                    new SystemGlobalDiskSnapshot
                    {
                        DiskId = "C:",
                        DisplayName = "Disk 0 (C:)",
                        TypeLabel = "SSD (NVMe)",
                        ActiveTimePct = 0.6,
                        ReadBps = 1200,
                        WriteBps = 3400,
                    },
                ]));
        MonitoringShellViewModel viewModel = await CreateBootstrappedViewModelAsync(
            gateway,
            systemGlobalMetricsSampler: sampler);

        gateway.RaiseDelta(1, [], []);
        gateway.RaiseDelta(2, [], []);

        GlobalResourceRowViewState diskRow = Assert.Single(viewModel.GlobalResourceRows.Where(row => row.Kind == GlobalResourceKind.Disk));
        viewModel.SelectedGlobalResource = diskRow;

        Assert.Equal("0%", diskRow.ValueText);
        Assert.Equal("0.6%", viewModel.GlobalDetailCurrentValue);
        Assert.Contains(viewModel.GlobalDetailStats, item => item.Label == "Active time" && item.Value == "0.6%");
    }

    [Fact]
    public async Task GlobalMode_DiskSelection_TinyAverageResponse_StaysVisibleInDiskStats()
    {
        TestRuntimeEventGateway gateway = new();
        TestSystemGlobalMetricsSampler sampler = new(
            CreateSystemGlobalMetricsSample(
                tsMs: 302,
                cpuPct: 15.0,
                memoryUsedBytes: 8 * 1024UL * 1024UL * 1024UL,
                diskReadBps: 3000,
                diskWriteBps: 7000,
                otherIoBps: 0,
                diskSnapshots:
                [
                    new SystemGlobalDiskSnapshot
                    {
                        DiskId = "D:",
                        DisplayName = "Disk 1 (D:)",
                        TypeLabel = "SSD",
                        ActiveTimePct = 21,
                        AvgResponseMs = 0.0004,
                        ReadBps = 3000,
                        WriteBps = 7000,
                    },
                ]));
        MonitoringShellViewModel viewModel = await CreateBootstrappedViewModelAsync(
            gateway,
            systemGlobalMetricsSampler: sampler);

        gateway.RaiseDelta(1, [], []);
        gateway.RaiseDelta(2, [], []);

        GlobalResourceRowViewState diskRow = Assert.Single(viewModel.GlobalResourceRows.Where(row => row.Kind == GlobalResourceKind.Disk));
        viewModel.SelectedGlobalResource = diskRow;

        Assert.Contains(viewModel.GlobalDetailStats, item => item.Label == "Average response time" && item.Value == "<0.001 ms");
    }

    [Fact]
    public async Task GlobalMode_DiskSelection_NonFiniteActiveTime_UsesNaFallback()
    {
        TestRuntimeEventGateway gateway = new();
        TestSystemGlobalMetricsSampler sampler = new(
            CreateSystemGlobalMetricsSample(
                tsMs: 302,
                cpuPct: 12.0,
                memoryUsedBytes: 8 * 1024UL * 1024UL * 1024UL,
                diskReadBps: 1000,
                diskWriteBps: 2000,
                otherIoBps: 0,
                diskSnapshots:
                [
                    new SystemGlobalDiskSnapshot
                    {
                        DiskId = "C:",
                        DisplayName = "Disk 0 (C:)",
                        TypeLabel = "SSD (NVMe)",
                        ActiveTimePct = double.NaN,
                        ReadBps = 1000,
                        WriteBps = 2000,
                    },
                ]));
        MonitoringShellViewModel viewModel = await CreateBootstrappedViewModelAsync(
            gateway,
            systemGlobalMetricsSampler: sampler);

        gateway.RaiseDelta(1, [], []);
        gateway.RaiseDelta(2, [], []);

        GlobalResourceRowViewState diskRow = Assert.Single(viewModel.GlobalResourceRows.Where(row => row.Kind == GlobalResourceKind.Disk));
        Exception? exception = Record.Exception(() => viewModel.SelectedGlobalResource = diskRow);

        Assert.Null(exception);
        Assert.Equal("n/a", diskRow.ValueText);
        Assert.Equal("n/a", viewModel.GlobalDetailCurrentValue);
        Assert.Equal(Visibility.Visible, viewModel.GlobalAuxiliaryChartVisibility);
        Assert.NotEmpty(viewModel.GlobalAuxiliaryTrendValues);
        Assert.Contains(viewModel.GlobalDetailStats, item => item.Label == "Active time" && item.Value == "n/a");
    }

    [Fact]
    public async Task GlobalMode_DiskSelection_ShowsFiniteTransferTrend()
    {
        TestRuntimeEventGateway gateway = new();
        TestSystemGlobalMetricsSampler sampler = new(
            CreateSystemGlobalMetricsSample(
                tsMs: 303,
                cpuPct: 15.0,
                memoryUsedBytes: 8 * 1024UL * 1024UL * 1024UL,
                diskReadBps: 3000,
                diskWriteBps: 7000,
                otherIoBps: 0,
                diskSnapshots:
                [
                    new SystemGlobalDiskSnapshot
                    {
                        DiskId = "D:",
                        DisplayName = "Disk 1 (D:)",
                        TypeLabel = "SSD",
                        ActiveTimePct = 21,
                        ReadBps = 3000,
                        WriteBps = 7000,
                    },
                ]));
        MonitoringShellViewModel viewModel = await CreateBootstrappedViewModelAsync(
            gateway,
            systemGlobalMetricsSampler: sampler);

        gateway.RaiseDelta(1, [], []);

        GlobalResourceRowViewState diskRow = Assert.Single(viewModel.GlobalResourceRows.Where(row => row.Kind == GlobalResourceKind.Disk));
        viewModel.SelectedGlobalResource = diskRow;

        Assert.Equal(Visibility.Visible, viewModel.GlobalAuxiliaryChartVisibility);
        Assert.NotEmpty(viewModel.GlobalAuxiliaryTrendValues);
        Assert.All(viewModel.GlobalAuxiliaryTrendValues, value =>
        {
            Assert.True(double.IsFinite(value));
            Assert.True(value >= 0d);
        });
    }

    [Fact]
    public async Task CpuGraphModeLogical_WhenNonCpuSelected_KeepsCombinedChartsVisible()
    {
        TestRuntimeEventGateway gateway = new();
        TestSystemGlobalMetricsSampler sampler = new(
            CreateSystemGlobalMetricsSample(
                tsMs: 201,
                cpuPct: 30.0,
                memoryUsedBytes: 10 * 1024UL * 1024UL,
                diskReadBps: 0,
                diskWriteBps: 0,
                otherIoBps: 0,
                cpuSnapshot: new SystemGlobalCpuSnapshot
                {
                    LogicalProcessorUtilizationPct = [10, 20, 30, 40],
                    LogicalProcessorKernelPct = [3, 6, 9, 12],
                },
                diskSnapshots:
                [
                    new SystemGlobalDiskSnapshot
                    {
                        DiskId = "disk1",
                        DisplayName = "Disk 1 (D:)",
                        TypeLabel = "SSD",
                        ActiveTimePct = 5,
                        ReadBps = 10,
                        WriteBps = 20,
                    },
                ]));
        MonitoringShellViewModel viewModel = await CreateBootstrappedViewModelAsync(
            gateway,
            systemGlobalMetricsSampler: sampler);

        gateway.RaiseDelta(1, [], []);
        viewModel.CpuGraphModeSelectedCommand.Execute("LogicalProcessors");

        GlobalResourceRowViewState memoryRow = Assert.Single(viewModel.GlobalResourceRows.Where(row => row.Kind == GlobalResourceKind.Memory));
        viewModel.SelectedGlobalResource = memoryRow;

        Assert.Equal(Visibility.Collapsed, viewModel.GlobalCpuModeToggleVisibility);
        Assert.Equal(Visibility.Visible, viewModel.GlobalCombinedChartVisibility);
        Assert.Equal(Visibility.Collapsed, viewModel.GlobalCpuLogicalGridVisibility);

        GlobalResourceRowViewState cpuRow = Assert.Single(viewModel.GlobalResourceRows.Where(row => row.Kind == GlobalResourceKind.Cpu));
        viewModel.SelectedGlobalResource = cpuRow;

        Assert.Equal(Visibility.Visible, viewModel.GlobalCpuModeToggleVisibility);
        Assert.Equal(Visibility.Collapsed, viewModel.GlobalCombinedChartVisibility);
        Assert.Equal(Visibility.Visible, viewModel.GlobalCpuLogicalGridVisibility);
        Assert.NotEmpty(viewModel.GlobalCpuLogicalProcessorRows);
        LogicalProcessorTrendViewState logicalRow = viewModel.GlobalCpuLogicalProcessorRows[0];
        Assert.Equal("system:cpu:logical:0", logicalRow.ChartIdentityKey);
        Assert.NotEmpty(logicalRow.Values);
        Assert.NotEmpty(logicalRow.OverlayValues);
    }

    [Fact]
    public async Task GlobalSelection_SwitchingCpuToMemory_ChangesPrimaryChartIdentity_AndDisablesOverlay()
    {
        TestRuntimeEventGateway gateway = new();
        TestSystemGlobalMetricsSampler sampler = new(
            CreateSystemGlobalMetricsSample(
                tsMs: 202,
                cpuPct: 9.0,
                memoryUsedBytes: 8 * 1024UL * 1024UL,
                diskReadBps: 0,
                diskWriteBps: 0,
                otherIoBps: 0));
        MonitoringShellViewModel viewModel = await CreateBootstrappedViewModelAsync(
            gateway,
            systemGlobalMetricsSampler: sampler);

        gateway.RaiseDelta(1, [], []);
        Assert.Equal("system:cpu:primary:combined", viewModel.SystemPrimaryChartIdentityKey);

        GlobalResourceRowViewState memoryRow = Assert.Single(viewModel.GlobalResourceRows.Where(row => row.Kind == GlobalResourceKind.Memory));
        viewModel.SelectedGlobalResource = memoryRow;

        Assert.Equal("system:memory:primary:default", viewModel.SystemPrimaryChartIdentityKey);
        Assert.False(viewModel.GlobalShowSecondaryOverlay);
    }

    [Fact]
    public async Task GlobalNetworkSelection_UsesBitsScaleAndOverlay()
    {
        TestRuntimeEventGateway gateway = new();
        TestSystemGlobalMetricsSampler sampler = new(
            CreateSystemGlobalMetricsSample(
                tsMs: 202,
                cpuPct: 9.0,
                memoryUsedBytes: 8 * 1024UL * 1024UL,
                diskReadBps: 0,
                diskWriteBps: 0,
                otherIoBps: 0,
                networkSnapshots:
                [
                    new SystemGlobalNetworkSnapshot
                    {
                        AdapterId = "eth1",
                        DisplayName = "Ethernet",
                        AdapterName = "Ethernet 2",
                        ConnectionType = "Ethernet",
                        SendBps = 32_000,
                        ReceiveBps = 16_000,
                    },
                ]));
        MonitoringShellViewModel viewModel = await CreateBootstrappedViewModelAsync(
            gateway,
            systemGlobalMetricsSampler: sampler);

        gateway.RaiseDelta(1, [], []);
        GlobalResourceRowViewState networkRow = Assert.Single(viewModel.GlobalResourceRows.Where(row => row.Kind == GlobalResourceKind.Network));
        viewModel.SelectedGlobalResource = networkRow;

        Assert.Equal(MetricTrendScaleMode.BitsRate, viewModel.GlobalPrimaryScaleMode);
        Assert.True(viewModel.GlobalShowSecondaryOverlay);
    }

    [Fact]
    public async Task SelectedRow_UsesUnifiedProcessResourceRows_WithHybridMetricSet()
    {
        TestRuntimeEventGateway gateway = new();
        TestSystemGlobalMetricsSampler sampler = new(
            CreateSystemGlobalMetricsSample(
                tsMs: 250,
                cpuPct: 10.0,
                memoryUsedBytes: 8 * 1024UL * 1024UL,
                diskReadBps: 0,
                diskWriteBps: 0,
                otherIoBps: 0));
        MonitoringShellViewModel viewModel = await CreateBootstrappedViewModelAsync(
            gateway,
            systemGlobalMetricsSampler: sampler);

        ProcessSample row = Sample(pid: 250, startTime: 25_000, access: AccessState.Full) with
        {
            CpuPct = 12.5,
            RssBytes = 32 * 1024UL * 1024UL,
            IoReadBps = 1_500UL,
            IoWriteBps = 2_500UL,
            OtherIoBps = 3_000UL,
        };
        gateway.RaiseDelta(1, [row], []);
        await viewModel.SelectRowAsync(row, CancellationToken.None);

        Assert.False(viewModel.IsGlobalPerformanceMode);
        Assert.Equal(5, viewModel.GlobalResourceRows.Count);
        Assert.Contains(viewModel.GlobalResourceRows, item => item.Kind == GlobalResourceKind.Cpu);
        Assert.Contains(viewModel.GlobalResourceRows, item => item.Kind == GlobalResourceKind.Memory);
        Assert.Contains(viewModel.GlobalResourceRows, item => item.Kind == GlobalResourceKind.Disk);
        Assert.Contains(viewModel.GlobalResourceRows, item => item.Kind == GlobalResourceKind.Network);
        Assert.Contains(viewModel.GlobalResourceRows, item => item.Kind == GlobalResourceKind.OtherIo);
        Assert.Equal(GlobalResourceKind.Cpu, viewModel.SelectedGlobalResource?.Kind);
    }

    [Fact]
    public async Task ProcessMode_CpuLogicalSelection_ShowsPlaceholderInsteadOfLogicalGrid()
    {
        TestRuntimeEventGateway gateway = new();
        MonitoringShellViewModel viewModel = await CreateBootstrappedViewModelAsync(gateway);

        ProcessSample row = Sample(pid: 260, startTime: 26_000, access: AccessState.Full) with
        {
            CpuPct = 21.5,
        };
        gateway.RaiseDelta(1, [row], []);
        await viewModel.SelectRowAsync(row, CancellationToken.None);

        viewModel.CpuGraphModeSelectedCommand.Execute("LogicalProcessors");
        GlobalResourceRowViewState cpuRow = Assert.Single(viewModel.GlobalResourceRows.Where(item => item.Kind == GlobalResourceKind.Cpu));
        viewModel.SelectedGlobalResource = cpuRow;

        Assert.Equal(Visibility.Collapsed, viewModel.GlobalCpuLogicalGridVisibility);
        Assert.Equal(Visibility.Visible, viewModel.GlobalCpuLogicalPlaceholderVisibility);
        Assert.Equal(Visibility.Collapsed, viewModel.GlobalCombinedChartVisibility);
    }

    [Fact]
    public async Task ProcessSelection_SwitchingProcesses_ChangesProcessChartIdentity_AndMiniChartIdentity()
    {
        TestRuntimeEventGateway gateway = new();
        MonitoringShellViewModel viewModel = await CreateBootstrappedViewModelAsync(gateway);

        ProcessSample first = Sample(pid: 310, startTime: 31_000, access: AccessState.Full) with
        {
            CpuPct = 11.5,
        };
        ProcessSample second = Sample(pid: 311, startTime: 31_100, access: AccessState.Full) with
        {
            CpuPct = 19.5,
        };

        gateway.RaiseDelta(1, [first, second], []);
        await viewModel.SelectRowAsync(first, CancellationToken.None);
        string firstInspectorIdentity = viewModel.ProcessPrimaryChartIdentityKey;
        GlobalResourceRowViewState firstCpuRow = Assert.Single(viewModel.GlobalResourceRows.Where(item => item.Kind == GlobalResourceKind.Cpu));
        string firstMiniIdentity = firstCpuRow.ChartIdentityKey;

        await viewModel.SelectRowAsync(second, CancellationToken.None);
        string secondInspectorIdentity = viewModel.ProcessPrimaryChartIdentityKey;
        GlobalResourceRowViewState secondCpuRow = Assert.Single(viewModel.GlobalResourceRows.Where(item => item.Kind == GlobalResourceKind.Cpu));
        string secondMiniIdentity = secondCpuRow.ChartIdentityKey;

        Assert.Equal("process:pid310-start31000:proc:cpu:primary:combined", firstInspectorIdentity);
        Assert.Equal("process:pid311-start31100:proc:cpu:primary:combined", secondInspectorIdentity);
        Assert.NotEqual(firstInspectorIdentity, secondInspectorIdentity);
        Assert.Equal("proc:cpu:310:31000", firstMiniIdentity);
        Assert.Equal("proc:cpu:311:31100", secondMiniIdentity);
        Assert.NotEqual(firstMiniIdentity, secondMiniIdentity);
    }
    [Fact]
    public async Task ClearingSelection_RestoresSystemSummaryVisibility_AndHidesProcessSummary()
    {
        TestRuntimeEventGateway gateway = new();
        TestSystemGlobalMetricsSampler sampler = new(
            CreateSystemGlobalMetricsSample(
                tsMs: 261,
                cpuPct: 14.0,
                memoryUsedBytes: 8 * 1024UL * 1024UL,
                diskReadBps: 0,
                diskWriteBps: 0,
                otherIoBps: 0));
        MonitoringShellViewModel viewModel = await CreateBootstrappedViewModelAsync(
            gateway,
            systemGlobalMetricsSampler: sampler);

        gateway.RaiseDelta(1, [], []);
        GlobalResourceRowViewState memoryRow = Assert.Single(viewModel.GlobalResourceRows.Where(item => item.Kind == GlobalResourceKind.Memory));
        viewModel.SelectedGlobalResource = memoryRow;

        ProcessSample row = Sample(pid: 261, startTime: 26_100, access: AccessState.Full) with
        {
            CpuPct = 17.5,
        };
        gateway.RaiseDelta(1, [row], []);
        await viewModel.SelectRowAsync(row, CancellationToken.None);

        Assert.Equal(Visibility.Visible, viewModel.ProcessSummarySectionVisibility);
        Assert.Equal(Visibility.Collapsed, viewModel.SystemSummarySectionVisibility);
        Assert.Equal("process:pid261-start26100:proc:cpu:primary:combined", viewModel.ProcessPrimaryChartIdentityKey);

        viewModel.ClearSelection();

        Assert.True(viewModel.IsSummarySectionSelected);
        Assert.Equal(Visibility.Visible, viewModel.SystemSummarySectionVisibility);
        Assert.Equal(Visibility.Collapsed, viewModel.ProcessSummarySectionVisibility);
        Assert.Equal("SYSTEM VIEW", viewModel.InspectorOverviewEyebrow);
        Assert.Equal("system:memory:primary:default", viewModel.SystemPrimaryChartIdentityKey);
    }
    [Fact]
    public async Task SystemMiniTrendHistories_KeepAdvancing_WhileProcessInspectorIsActive()
    {
        TestRuntimeEventGateway gateway = new();
        ManualResetEventSlim firstSampleStarted = new(initialState: false);
        ManualResetEventSlim firstSampleRelease = new(initialState: false);
        ManualResetEventSlim firstSampleCompleted = new(initialState: false);
        ManualResetEventSlim secondSampleStarted = new(initialState: false);
        ManualResetEventSlim secondSampleRelease = new(initialState: false);
        ManualResetEventSlim secondSampleCompleted = new(initialState: false);

        SystemGlobalMetricsSample firstSample = CreateSystemGlobalMetricsSample(
            tsMs: 261,
            cpuPct: 14.0,
            memoryUsedBytes: 8 * 1024UL * 1024UL,
            diskReadBps: 0,
            diskWriteBps: 0,
            otherIoBps: 0);
        SystemGlobalMetricsSample secondSample = CreateSystemGlobalMetricsSample(
            tsMs: 262,
            cpuPct: 39.0,
            memoryUsedBytes: 24 * 1024UL * 1024UL,
            diskReadBps: 0,
            diskWriteBps: 0,
            otherIoBps: 0);
        TestSystemGlobalMetricsSampler sampler = new(firstSample);
        int sampleCallCount = 0;
        sampler.Handler = () =>
        {
            int call = Interlocked.Increment(ref sampleCallCount);
            if (call == 1)
            {
                firstSampleStarted.Set();
                if (!firstSampleRelease.Wait(TimeSpan.FromSeconds(2)))
                {
                    throw new TimeoutException("Timed out waiting to release first global mini-chart sample.");
                }

                firstSampleCompleted.Set();
                return firstSample;
            }

            if (call == 2)
            {
                secondSampleStarted.Set();
                if (!secondSampleRelease.Wait(TimeSpan.FromSeconds(2)))
                {
                    throw new TimeoutException("Timed out waiting to release second global mini-chart sample.");
                }

                secondSampleCompleted.Set();
                return secondSample;
            }

            return secondSample;
        };

        MonitoringShellViewModel viewModel = await CreateBootstrappedViewModelAsync(
            gateway,
            systemGlobalMetricsSampler: sampler);

        try
        {
            Assert.True(firstSampleStarted.Wait(TimeSpan.FromSeconds(2)), "First global mini-chart sample did not start.");
            firstSampleRelease.Set();
            Assert.True(firstSampleCompleted.Wait(TimeSpan.FromSeconds(2)), "First global mini-chart sample did not complete.");

            gateway.RaiseDelta(1, [], []);
            GlobalResourceRowViewState memoryRow = Assert.Single(viewModel.GlobalResourceRows, item => item.Kind == GlobalResourceKind.Memory);
            viewModel.SelectedGlobalResource = memoryRow;

            ProcessSample row = Sample(pid: 261, startTime: 26_100, access: AccessState.Full) with
            {
                CpuPct = 17.5,
            };
            gateway.RaiseDelta(2, [row], []);
            await viewModel.SelectRowAsync(row, CancellationToken.None);

            Assert.True(secondSampleStarted.Wait(TimeSpan.FromSeconds(2)), "Second global mini-chart sample did not start.");
            secondSampleRelease.Set();
            Assert.True(secondSampleCompleted.Wait(TimeSpan.FromSeconds(2)), "Second global mini-chart sample did not complete.");

            gateway.RaiseDelta(3, [row with { Seq = 3, TsMs = 3 }], []);
            viewModel.ClearSelection();

            GlobalResourceRowViewState restoredMemoryResource = Assert.IsType<GlobalResourceRowViewState>(viewModel.SelectedGlobalResource);
            Assert.Equal(GlobalResourceKind.Memory, restoredMemoryResource.Kind);
            Assert.Equal((double)secondSample.MemoryUsedBytes!.Value, restoredMemoryResource.MiniTrendValues[^1]);
            Assert.Contains(restoredMemoryResource.MiniTrendValues, value => value > 0d);
        }
        finally
        {
            firstSampleRelease.Set();
            secondSampleRelease.Set();
        }
    }

    [Fact]
    public async Task ClearingSelection_PreservesLastKnownSystemCpuSample_WhenNextSampleIsSparse()
    {
        TestRuntimeEventGateway gateway = new();
        SystemGlobalMetricsSample firstSample = CreateSystemGlobalMetricsSample(
            tsMs: 264,
            cpuPct: 52.0,
            memoryUsedBytes: 8 * 1024UL * 1024UL,
            diskReadBps: 0,
            diskWriteBps: 0,
            otherIoBps: 0,
            cpuSnapshot: new SystemGlobalCpuSnapshot
            {
                ProcessorName = "Processor",
                SpeedMHz = 3500,
                BaseSpeedMHz = 3200,
            });
        SystemGlobalMetricsSample sparseSecondSample = CreateSystemGlobalMetricsSample(
            tsMs: 265,
            cpuPct: null,
            memoryUsedBytes: 16 * 1024UL * 1024UL,
            diskReadBps: 0,
            diskWriteBps: 0,
            otherIoBps: 0,
            cpuSnapshot: null,
            memorySnapshot: new SystemGlobalMemorySnapshot
            {
                UsedBytes = 16 * 1024UL * 1024UL,
                TotalBytes = 32 * 1024UL * 1024UL,
            });

        int sampleCallCount = 0;
        TestSystemGlobalMetricsSampler sampler = new(firstSample)
        {
            Handler = () => Interlocked.Increment(ref sampleCallCount) == 1 ? firstSample : sparseSecondSample,
        };

        MonitoringShellViewModel viewModel = await CreateBootstrappedViewModelAsync(
            gateway,
            systemGlobalMetricsSampler: sampler);

        gateway.RaiseDelta(1, [], []);

        ProcessSample row = Sample(pid: 264, startTime: 26_400, access: AccessState.Full) with
        {
            CpuPct = 11.5,
        };
        gateway.RaiseDelta(2, [row], []);
        await viewModel.SelectRowAsync(row, CancellationToken.None);

        gateway.RaiseDelta(3, [row with { Seq = 3, TsMs = 3, CpuPct = 12.5 }], []);
        viewModel.ClearSelection();

        Assert.Equal(GlobalResourceKind.Cpu, viewModel.SelectedGlobalResource?.Kind);
        Assert.Equal("system:cpu:primary:combined", viewModel.SystemPrimaryChartIdentityKey);
        Assert.Equal("52% 3.50 GHz", viewModel.GlobalDetailCurrentValue);
        Assert.Equal("Processor", viewModel.GlobalDetailSubtitle);
        Assert.Equal(52.0, viewModel.GlobalPrimaryTrendValues[^1]);
        GlobalResourceRowViewState cpuRow = Assert.Single(viewModel.GlobalResourceRows, item => item.Kind == GlobalResourceKind.Cpu);
        Assert.Equal("52%", cpuRow.Subtitle);
        Assert.Equal("3.50 GHz", cpuRow.ValueText);
    }
    [Fact]
    public async Task CpuGraphModeSwitch_ChangesSystemPrimaryChartIdentity_WhenCpuRemainsSelected()
    {
        TestRuntimeEventGateway gateway = new();
        TestSystemGlobalMetricsSampler sampler = new(
            CreateSystemGlobalMetricsSample(
                tsMs: 275,
                cpuPct: 32.0,
                memoryUsedBytes: 8 * 1024UL * 1024UL,
                diskReadBps: 0,
                diskWriteBps: 0,
                otherIoBps: 0,
                cpuSnapshot: new SystemGlobalCpuSnapshot
                {
                    LogicalProcessorUtilizationPct = [12.0, 24.0],
                    LogicalProcessorKernelPct = [4.0, 8.0],
                }));
        MonitoringShellViewModel viewModel = await CreateBootstrappedViewModelAsync(
            gateway,
            systemGlobalMetricsSampler: sampler);

        gateway.RaiseDelta(1, [], []);
        Assert.Equal("system:cpu:primary:combined", viewModel.SystemPrimaryChartIdentityKey);

        viewModel.CpuGraphModeSelectedCommand.Execute("LogicalProcessors");

        Assert.Equal("system:cpu:primary:logical", viewModel.SystemPrimaryChartIdentityKey);
    }

    [Fact]
    public async Task ModeSwitch_RestoresLastSelectionPerGlobalAndProcessContexts()
    {
        TestRuntimeEventGateway gateway = new();
        TestSystemGlobalMetricsSampler sampler = new(
            CreateSystemGlobalMetricsSample(
                tsMs: 270,
                cpuPct: 40.0,
                memoryUsedBytes: 8 * 1024UL * 1024UL,
                diskReadBps: 1000UL,
                diskWriteBps: 2000UL,
                otherIoBps: 0));
        MonitoringShellViewModel viewModel = await CreateBootstrappedViewModelAsync(
            gateway,
            systemGlobalMetricsSampler: sampler);

        gateway.RaiseDelta(1, [], []);
        GlobalResourceRowViewState globalMemory = Assert.Single(viewModel.GlobalResourceRows.Where(item => item.Kind == GlobalResourceKind.Memory));
        viewModel.SelectedGlobalResource = globalMemory;

        ProcessSample row = Sample(pid: 270, startTime: 27_000, access: AccessState.Full) with
        {
            OtherIoBps = 4_000UL,
        };
        gateway.RaiseDelta(2, [row], []);
        await viewModel.SelectRowAsync(row, CancellationToken.None);
        GlobalResourceRowViewState processOtherIo = Assert.Single(viewModel.GlobalResourceRows.Where(item => item.Kind == GlobalResourceKind.OtherIo));
        viewModel.SelectedGlobalResource = processOtherIo;

        viewModel.ClearSelection();
        Assert.True(viewModel.IsGlobalPerformanceMode);
        Assert.Equal(GlobalResourceKind.Memory, viewModel.SelectedGlobalResource?.Kind);

        await viewModel.SelectRowAsync(row, CancellationToken.None);
        Assert.Equal(GlobalResourceKind.OtherIo, viewModel.SelectedGlobalResource?.Kind);
    }

    [Fact]
    public async Task ProcessMode_StatsIncludeMetadataFields()
    {
        TestMetadataProvider metadata = new((pid, _, _) => Task.FromResult<ProcessMetadata?>(new ProcessMetadata
        {
            Pid = pid,
            ParentPid = 42,
            ExecutablePath = @"C:\\Apps\\demo.exe",
            CommandLine = "demo --flag",
        }));
        TestRuntimeEventGateway gateway = new();
        MonitoringShellViewModel viewModel = await CreateBootstrappedViewModelAsync(gateway, metadataProvider: metadata);

        ProcessSample row = Sample(pid: 280, startTime: 28_000, access: AccessState.Full);
        gateway.RaiseDelta(1, [row], []);
        await viewModel.SelectRowAsync(row, CancellationToken.None);

        Assert.Contains(viewModel.GlobalDetailStats, item => item.Label == "Parent PID" && item.Value == "42");
        Assert.Contains(viewModel.GlobalDetailStats, item => item.Label == "Executable path" && item.Value.Contains("demo.exe", StringComparison.OrdinalIgnoreCase));
        Assert.Contains(viewModel.GlobalDetailStats, item => item.Label == "Command line" && item.Value.Contains("demo --flag", StringComparison.Ordinal));
    }

    [Fact]
    public async Task SelectedRow_ProcessMetricsRemainUnchangedWhenSamplerPresent()
    {
        TestRuntimeEventGateway gateway = new();
        TestSystemGlobalMetricsSampler sampler = new(
            CreateSystemGlobalMetricsSample(
                tsMs: 13,
                cpuPct: 91.7,
                memoryUsedBytes: 400 * 1024UL * 1024UL,
                diskReadBps: 8000UL,
                diskWriteBps: 9000UL,
                otherIoBps: 10000UL));
        MonitoringShellViewModel viewModel = await CreateBootstrappedViewModelAsync(
            gateway,
            systemGlobalMetricsSampler: sampler);

        ProcessSample row = Sample(pid: 140, startTime: 14000, access: AccessState.Full) with
        {
            CpuPct = 17.25,
            RssBytes = 24 * 1024UL * 1024UL,
            IoReadBps = 3000UL,
            IoWriteBps = 4000UL,
            OtherIoBps = 5000UL,
        };
        gateway.RaiseDelta(1, [row], []);

        await viewModel.SelectRowAsync(row, CancellationToken.None);

        Assert.Equal($"{row.CpuPct:F2}%", viewModel.CpuMetricChipValue);
        Assert.Equal(ValueFormat.FormatBytes(row.RssBytes), viewModel.MemoryMetricChipValue);
        Assert.Equal(ValueFormat.FormatRate(row.IoReadBps), viewModel.IoReadMetricChipValue);
        Assert.Equal(ValueFormat.FormatRate(row.IoWriteBps), viewModel.IoWriteMetricChipValue);
        Assert.Equal(ValueFormat.FormatRate(row.OtherIoBps), viewModel.OtherIoMetricChipValue);
        Assert.DoesNotContain("n/a", viewModel.DetailMetricValue, StringComparison.OrdinalIgnoreCase);
    }

    [Fact]
    public async Task ToggleSelection_SameIdentity_DoesNotClearSelection()
    {
        TestMetadataProvider metadata = new((pid, _, _) => Task.FromResult<ProcessMetadata?>(new ProcessMetadata
        {
            Pid = pid,
            ParentPid = 1,
        }));
        TestRuntimeEventGateway gateway = new();
        MonitoringShellViewModel viewModel = await CreateBootstrappedViewModelAsync(gateway, metadataProvider: metadata);

        ProcessSample row = Sample(pid: 30, startTime: 3000, access: AccessState.Full);
        gateway.RaiseDelta(1, [row], []);

        await viewModel.SelectRowAsync(row, CancellationToken.None);
        await viewModel.ToggleSelectionAsync(row, CancellationToken.None);

        Assert.NotNull(viewModel.SelectedRow);
        Assert.Equal(row.Identity(), viewModel.SelectedRow!.Identity());
        Assert.NotNull(viewModel.SelectedMetadata);
        Assert.True(viewModel.HasSelection);
    }

    [Fact]
    public async Task ToggleSelection_NullRowWhenSelectedProcessStillTracked_DoesNotClearSelection()
    {
        TestMetadataProvider metadata = new((pid, _, _) => Task.FromResult<ProcessMetadata?>(new ProcessMetadata
        {
            Pid = pid,
            ParentPid = 1,
        }));
        TestRuntimeEventGateway gateway = new();
        MonitoringShellViewModel viewModel = await CreateBootstrappedViewModelAsync(gateway, metadataProvider: metadata);

        ProcessSample row = Sample(pid: 31, startTime: 3100, access: AccessState.Full);
        gateway.RaiseDelta(1, [row], []);

        await viewModel.SelectRowAsync(row, CancellationToken.None);
        await viewModel.ToggleSelectionAsync(null, CancellationToken.None);

        Assert.NotNull(viewModel.SelectedRow);
        Assert.Equal(row.Identity(), viewModel.SelectedRow!.Identity());
        Assert.True(viewModel.HasSelection);
    }

    [Fact]
    public async Task SelectedVisibleRowBinding_WhenRowSelectedFromUi_SelectsDetailAndLoadsMetadata()
    {
        int metadataRequestCount = 0;
        TestMetadataProvider metadata = new((pid, _, _) =>
        {
            metadataRequestCount++;
            return Task.FromResult<ProcessMetadata?>(new ProcessMetadata
            {
                Pid = pid,
                ParentPid = 7,
            });
        });
        TestRuntimeEventGateway gateway = new();
        MonitoringShellViewModel viewModel = await CreateBootstrappedViewModelAsync(gateway, metadataProvider: metadata);

        ProcessSample row = Sample(pid: 60, startTime: 6000, access: AccessState.Full);
        gateway.RaiseDelta(1, [row], []);
        ProcessRowViewState rowState = GetVisibleRow(viewModel, 0);

        viewModel.SelectedVisibleRowBinding = rowState;

        Assert.NotNull(viewModel.SelectedRow);
        Assert.Equal(row.Identity(), viewModel.SelectedRow!.Identity());
        Assert.Same(rowState, viewModel.SelectedVisibleRow);
        Assert.NotNull(viewModel.SelectedMetadata);
        Assert.Equal(1, metadataRequestCount);
    }

    [Fact]
    public async Task SelectedVisibleRowBinding_WhenUiSendsTransientNull_KeepsSelectionAndRestoresVisibleRow()
    {
        TestRuntimeEventGateway gateway = new();
        MonitoringShellViewModel viewModel = await CreateBootstrappedViewModelAsync(gateway);
        int selectedVisibleRowBindingNotifications = 0;
        viewModel.PropertyChanged += (_, args) =>
        {
            if (args.PropertyName == nameof(MonitoringShellViewModel.SelectedVisibleRowBinding))
            {
                selectedVisibleRowBindingNotifications++;
            }
        };

        ProcessSample row = Sample(pid: 61, startTime: 6100, access: AccessState.Full);
        gateway.RaiseDelta(1, [row], []);
        ProcessRowViewState rowState = GetVisibleRow(viewModel, 0);

        viewModel.SelectedVisibleRowBinding = rowState;
        int beforeTransientNull = selectedVisibleRowBindingNotifications;
        viewModel.SelectedVisibleRowBinding = null;

        Assert.NotNull(viewModel.SelectedRow);
        Assert.Equal(row.Identity(), viewModel.SelectedRow!.Identity());
        Assert.Same(rowState, viewModel.SelectedVisibleRow);
        Assert.Same(rowState, viewModel.SelectedVisibleRowBinding);
        Assert.True(selectedVisibleRowBindingNotifications > beforeTransientNull);
    }

    [Fact]
    public async Task SelectedVisibleRowBinding_WhenSortChangesAndUiSendsNull_RestoresListSelection()
    {
        TestRuntimeEventGateway gateway = new();
        MonitoringShellViewModel viewModel = await CreateBootstrappedViewModelAsync(gateway);

        ProcessSample first = Sample(pid: 161, startTime: 16100, access: AccessState.Full) with { Name = "zeta", CpuPct = 30 };
        ProcessSample second = Sample(pid: 162, startTime: 16200, access: AccessState.Full) with { Name = "alpha", CpuPct = 90 };
        gateway.RaiseDelta(1, [first, second], []);

        ProcessRowViewState selected = GetVisibleRows(viewModel).Single(row => row.Pid == second.Pid);
        viewModel.SelectedVisibleRowBinding = selected;

        viewModel.ChangeSort(SortColumn.Name);
        viewModel.SelectedVisibleRowBinding = null;

        Assert.NotNull(viewModel.SelectedRow);
        Assert.Equal(second.Identity(), viewModel.SelectedRow!.Identity());
        Assert.Same(selected, viewModel.SelectedVisibleRowBinding);
    }

    [Fact]
    public async Task ChangeSort_WithSelection_PreservesSelectedVisibleRowBindingWithoutForcedReassertions()
    {
        TestRuntimeEventGateway gateway = new();
        MonitoringShellViewModel viewModel = await CreateBootstrappedViewModelAsync(gateway);
        int bindingNotificationCount = 0;
        viewModel.PropertyChanged += (_, args) =>
        {
            if (args.PropertyName == nameof(MonitoringShellViewModel.SelectedVisibleRowBinding))
            {
                bindingNotificationCount++;
            }
        };

        ProcessSample first = Sample(pid: 163, startTime: 16300, access: AccessState.Full) with { Name = "gamma", CpuPct = 10 };
        ProcessSample second = Sample(pid: 164, startTime: 16400, access: AccessState.Full) with { Name = "beta", CpuPct = 80 };
        gateway.RaiseDelta(1, [first, second], []);

        ProcessRowViewState selected = GetVisibleRows(viewModel).Single(row => row.Pid == second.Pid);
        viewModel.SelectedVisibleRowBinding = selected;
        int beforeSortChanges = bindingNotificationCount;

        viewModel.ChangeSort(SortColumn.Name);
        viewModel.ChangeSort(SortColumn.Name);

        Assert.Equal(beforeSortChanges, bindingNotificationCount);
        Assert.Same(selected, viewModel.SelectedVisibleRowBinding);
        Assert.Equal(second.Identity(), viewModel.SelectedRow!.Identity());
    }

    [Fact]
    public async Task SelectedVisibleRowBinding_WhenRowVisibilityChanges_PreservesSelectionContinuity()
    {
        TestRuntimeEventGateway gateway = new();
        MonitoringShellViewModel viewModel = await CreateBootstrappedViewModelAsync(gateway);

        ProcessSample full = Sample(pid: 62, startTime: 6200, access: AccessState.Full);
        ProcessSample denied = Sample(pid: 63, startTime: 6300, access: AccessState.Denied);
        gateway.RaiseDelta(1, [full, denied], []);

        await viewModel.ToggleAdminModeAsync(true, CancellationToken.None);
        gateway.RaiseDelta(2, [full, denied], []);

        ProcessRowViewState deniedState = GetVisibleRows(viewModel).Single(row => row.Pid == denied.Pid);
        viewModel.SelectedVisibleRowBinding = deniedState;

        viewModel.AdminEnabledOnlyFilter = true;

        Assert.NotNull(viewModel.SelectedRow);
        Assert.Equal(denied.Identity(), viewModel.SelectedRow!.Identity());
        Assert.Null(viewModel.SelectedVisibleRow);
        Assert.Null(viewModel.SelectedVisibleRowBinding);

        viewModel.AdminEnabledOnlyFilter = false;

        Assert.NotNull(viewModel.SelectedRow);
        Assert.Equal(denied.Identity(), viewModel.SelectedRow!.Identity());
        Assert.Same(deniedState, viewModel.SelectedVisibleRow);
        Assert.Same(deniedState, viewModel.SelectedVisibleRowBinding);
    }

    [Fact]
    public async Task SelectedVisibleRowBinding_WhenSelectedRowExits_ClearsSelectionState()
    {
        TestRuntimeEventGateway gateway = new();
        MonitoringShellViewModel viewModel = await CreateBootstrappedViewModelAsync(gateway);

        ProcessSample row = Sample(pid: 64, startTime: 6400, access: AccessState.Full);
        gateway.RaiseDelta(1, [row], []);
        viewModel.SelectedVisibleRowBinding = GetVisibleRow(viewModel, 0);

        gateway.RaiseDelta(2, [], [row.Identity()]);

        Assert.Null(viewModel.SelectedRow);
        Assert.Null(viewModel.SelectedVisibleRow);
        Assert.Null(viewModel.SelectedVisibleRowBinding);
        Assert.Null(viewModel.SelectedMetadata);
        Assert.Null(viewModel.MetadataError);
        Assert.False(viewModel.IsMetadataLoading);
    }

    private static SequenceLaunchPolicyGate CreatePassedGate()
    {
        return new SequenceLaunchPolicyGate(
            () => StartupGateStatus.PassedContext(new LaunchContext { Os = "windows", WindowsBuild = 26000 }));
    }

    private static TestMetadataProvider CreateNullMetadataProvider()
    {
        return new TestMetadataProvider((_, _, _) => Task.FromResult<ProcessMetadata?>(null));
    }

    private static MonitoringShellViewModel CreateViewModel(
        SequenceLaunchPolicyGate gate,
        TestMetadataProvider metadataProvider,
        TestRuntimeEventGateway gateway,
        UserSettings? settings = null,
        TestSystemGlobalMetricsSampler? systemGlobalMetricsSampler = null)
    {
        MonitoringRuntime runtime = new(
            new TestCollectorFactory(),
            new DeltaTelemetryPipeline(),
            new InMemoryStateStore(),
            new IncrementalSortIndexEngine(),
            new TestPersistenceStore(settings),
            new RuntimeHostOptions());
        runtime.InitializeAsync(CancellationToken.None).GetAwaiter().GetResult();
        RuntimeLoopService loopService = new(runtime);
        RuntimeHealthService runtimeHealthService = new();
        TestSystemGlobalMetricsSampler sampler = systemGlobalMetricsSampler ?? TestSystemGlobalMetricsSampler.Default;
        return new MonitoringShellViewModel(
            gate,
            runtime,
            loopService,
            gateway,
            runtimeHealthService,
            metadataProvider,
            sampler);
    }

    private static async Task<MonitoringShellViewModel> CreateBootstrappedViewModelAsync(
        TestRuntimeEventGateway gateway,
        SequenceLaunchPolicyGate? gate = null,
        TestMetadataProvider? metadataProvider = null,
        UserSettings? settings = null,
        TestSystemGlobalMetricsSampler? systemGlobalMetricsSampler = null)
    {
        MonitoringShellViewModel viewModel = CreateViewModel(
            gate ?? CreatePassedGate(),
            metadataProvider ?? CreateNullMetadataProvider(),
            gateway,
            settings,
            systemGlobalMetricsSampler);

        await viewModel.BootstrapAsync(CancellationToken.None);
        return viewModel;
    }

    private static IReadOnlyList<ProcessRowViewState> GetVisibleRows(MonitoringShellViewModel viewModel)
    {
        return viewModel.VisibleRows.Cast<ProcessRowViewState>().ToList();
    }

    private static ProcessRowViewState GetVisibleRow(MonitoringShellViewModel viewModel, int index)
    {
        return Assert.IsType<ProcessRowViewState>(viewModel.VisibleRows[index]);
    }

    private static ProcessSample Sample(uint pid, ulong startTime, AccessState access)
    {
        return TestProcessSamples.Create(
            pid: pid,
            seq: 1,
            tsMs: 1,
            parentPid: 1,
            startTimeMs: startTime,
            name: $"proc-{pid}",
            cpuPct: 1,
            rssBytes: 1024,
            privateBytes: 512,
            ioReadBps: 10,
            ioWriteBps: 10,
            otherIoBps: 10,
            threads: 2,
            handles: 3,
            accessState: access);
    }

    private static SystemGlobalMetricsSample CreateSystemGlobalMetricsSample(
        ulong tsMs,
        double? cpuPct,
        ulong? memoryUsedBytes,
        ulong? diskReadBps,
        ulong? diskWriteBps,
        ulong? otherIoBps,
        SystemGlobalCpuSnapshot? cpuSnapshot = null,
        SystemGlobalMemorySnapshot? memorySnapshot = null,
        IReadOnlyList<SystemGlobalDiskSnapshot>? diskSnapshots = null,
        IReadOnlyList<SystemGlobalNetworkSnapshot>? networkSnapshots = null,
        bool cpuRateWarmed = true,
        bool rateCountersWarmed = true,
        bool extendedProbeCycleCompleted = true,
        bool? isReady = null)
    {
        return new SystemGlobalMetricsSample
        {
            TsMs = tsMs,
            CpuPct = cpuPct,
            MemoryUsedBytes = memoryUsedBytes,
            DiskReadBps = diskReadBps,
            DiskWriteBps = diskWriteBps,
            OtherIoBps = otherIoBps,
            CpuSnapshot = cpuSnapshot,
            MemorySnapshot = memorySnapshot,
            DiskSnapshots = diskSnapshots ?? [],
            NetworkSnapshots = networkSnapshots ?? [],
            CpuRateWarmed = cpuRateWarmed,
            RateCountersWarmed = rateCountersWarmed,
            ExtendedProbeCycleCompleted = extendedProbeCycleCompleted,
            IsReady = isReady ?? (cpuRateWarmed && rateCountersWarmed && extendedProbeCycleCompleted),
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
            Func<StartupGateStatus> nextGateStep = _steps.Count > 1 ? _steps.Dequeue() : _steps.Peek();
            return nextGateStep();
        }
    }
    private sealed class ThrowingOnAdminCollectorFactory : IProcessCollectorFactory
    {
        public ValueTask<CollectorActivationResult> CreateAsync(bool adminMode, CancellationToken ct)
        {
            if (adminMode)
            {
                return ValueTask.FromException<CollectorActivationResult>(new InvalidOperationException("toggle boom"));
            }

            return ValueTask.FromResult(new CollectorActivationResult(new TestCollector(), EffectiveAdminMode: false, Warning: null));
        }
    }


    private sealed class WarningOnAdminCollectorFactory : IProcessCollectorFactory
    {
        public ValueTask<CollectorActivationResult> CreateAsync(bool adminMode, CancellationToken ct)
        {
            return ValueTask.FromResult(adminMode
                ? new CollectorActivationResult(new TestCollector(), EffectiveAdminMode: false, Warning: "bridge warning")
                : new CollectorActivationResult(new TestCollector(), EffectiveAdminMode: false, Warning: null));
        }
    }

    private sealed class TestCollectorFactory : IProcessCollectorFactory
    {
        public ValueTask<CollectorActivationResult> CreateAsync(bool adminMode, CancellationToken ct)
        {
            return ValueTask.FromResult(new CollectorActivationResult(new TestCollector(), adminMode, Warning: null));
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
        private UserSettings _settings;
        private WarmCache? _warmCache;

        public TestPersistenceStore(UserSettings? settings = null)
        {
            _settings = settings ?? new UserSettings
            {
                AdminPreferenceInitialized = true,
            };
        }

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

        public string? TakeWarning()
        {
            return null;
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

    private sealed class TestSystemGlobalMetricsSampler : ISystemGlobalMetricsSampler
    {
        public static TestSystemGlobalMetricsSampler Default { get; } = new(
            CreateSystemGlobalMetricsSample(
                tsMs: 1,
                cpuPct: 7.5,
                memoryUsedBytes: 5 * 1024UL * 1024UL,
                diskReadBps: 1024UL,
                diskWriteBps: 2048UL,
                otherIoBps: 4096UL));

        public Func<SystemGlobalMetricsSample>? Handler { get; set; }

        public SystemGlobalMetricsSample Current { get; set; }

        public TestSystemGlobalMetricsSampler(SystemGlobalMetricsSample sample)
        {
            Current = sample;
        }

        public SystemGlobalMetricsSample Sample()
        {
            return Handler?.Invoke() ?? Current;
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

        public void PublishWarning(CollectorWarning warning)
        {
            CollectorWarningRaised?.Invoke(this, warning);
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
