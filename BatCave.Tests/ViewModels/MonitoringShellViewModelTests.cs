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
using Windows.Foundation;

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
            });

        Assert.Equal(120, viewModel.MetricTrendWindowSeconds);
        Assert.False(viewModel.IsTrendWindow60Selected);
        Assert.True(viewModel.IsTrendWindow120Selected);
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
        Assert.Equal(90, viewModel.CpuMetricTrendValues.Length);
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
    public async Task TelemetryDelta_HeartbeatOnly_RespectsSparklineStrideAndKeepsRowInstance()
    {
        TestRuntimeEventGateway gateway = new();
        MonitoringShellViewModel viewModel = await CreateBootstrappedViewModelAsync(gateway);

        ProcessSample row = Sample(pid: 21, startTime: 2100, access: AccessState.Full) with { CpuPct = 10 };
        gateway.RaiseDelta(1, [row], []);
        ProcessSample varied = row with { Seq = 2, TsMs = 2, CpuPct = 20 };
        gateway.RaiseDelta(2, [varied], []);
        ProcessRowViewState firstVisible = GetVisibleRow(viewModel, 0);
        IReadOnlyList<Point> beforeTrend = ClonePointCollection(firstVisible.CpuTrendGeometry);

        ProcessSample heartbeatOnlyUpdate = varied with { Seq = 3, TsMs = 3, ParentPid = varied.ParentPid + 1, PrivateBytes = varied.PrivateBytes + 1 };
        gateway.RaiseDelta(3, [heartbeatOnlyUpdate], []);
        IReadOnlyList<Point> afterOddHeartbeat = firstVisible.CpuTrendGeometry;

        ProcessSample strideHeartbeat = heartbeatOnlyUpdate with { Seq = 4, TsMs = 4, ParentPid = heartbeatOnlyUpdate.ParentPid + 1 };
        gateway.RaiseDelta(4, [strideHeartbeat], []);
        IReadOnlyList<Point> afterEvenHeartbeat = firstVisible.CpuTrendGeometry;

        Assert.Same(firstVisible, GetVisibleRow(viewModel, 0));
        AssertPointCollectionsEqual(beforeTrend, afterOddHeartbeat);
        AssertPointCollectionsNotEqual(beforeTrend, afterEvenHeartbeat);
    }

    [Fact]
    public async Task SelectedProcessTrend_AdvancesOnEmptyDeltaWhenValueUnchanged()
    {
        TestRuntimeEventGateway gateway = new();
        MonitoringShellViewModel viewModel = await CreateBootstrappedViewModelAsync(gateway);

        ProcessSample row = Sample(pid: 90, startTime: 9000, access: AccessState.Full) with { CpuPct = 3.25 };
        gateway.RaiseDelta(1, [row], []);
        viewModel.SelectedVisibleRowBinding = GetVisibleRow(viewModel, 0);
        int before = viewModel.CpuMetricTrendValues.Length;

        gateway.RaiseDelta(2, [], []);
        int after = viewModel.CpuMetricTrendValues.Length;

        Assert.True(after > before);
        Assert.All(viewModel.CpuMetricTrendValues, value => Assert.Equal(3.25, value, 2));
    }

    [Fact]
    public async Task TableMiniChart_AdvancesOnStrideWhenNoUpsertArrives()
    {
        TestRuntimeEventGateway gateway = new();
        MonitoringShellViewModel viewModel = await CreateBootstrappedViewModelAsync(gateway);

        ProcessSample row = Sample(pid: 91, startTime: 9100, access: AccessState.Full) with { CpuPct = 4.5 };
        gateway.RaiseDelta(1, [row], []);
        ProcessSample varied = row with { Seq = 2, TsMs = 2, CpuPct = 11.0 };
        gateway.RaiseDelta(2, [varied], []);
        ProcessRowViewState rowState = GetVisibleRow(viewModel, 0);
        IReadOnlyList<Point> before = ClonePointCollection(rowState.CpuTrendGeometry);

        gateway.RaiseDelta(3, [], []);
        IReadOnlyList<Point> afterOdd = rowState.CpuTrendGeometry;

        gateway.RaiseDelta(4, [], []);
        IReadOnlyList<Point> afterEven = rowState.CpuTrendGeometry;

        AssertPointCollectionsEqual(before, afterOdd);
        AssertPointCollectionsNotEqual(before, afterEven);
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
        Assert.Equal(120, GetVisibleRow(viewModel, 0).CpuTrendGeometry.Count);
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
    public async Task ChangeSort_WithSelection_ReassertsSelectedVisibleRowBindingForColumnAndDirectionChanges()
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
        int afterColumnChange = bindingNotificationCount;
        viewModel.ChangeSort(SortColumn.Name);

        Assert.True(afterColumnChange > beforeSortChanges);
        Assert.True(bindingNotificationCount > afterColumnChange);
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
        UserSettings? settings = null)
    {
        MonitoringRuntime runtime = new(
            new TestCollectorFactory(),
            new DeltaTelemetryPipeline(),
            new InMemoryStateStore(),
            new IncrementalSortIndexEngine(),
            new TestPersistenceStore(settings));
        RuntimeLoopService loopService = new(runtime);
        return new MonitoringShellViewModel(gate, runtime, loopService, gateway, metadataProvider);
    }

    private static async Task<MonitoringShellViewModel> CreateBootstrappedViewModelAsync(
        TestRuntimeEventGateway gateway,
        SequenceLaunchPolicyGate? gate = null,
        TestMetadataProvider? metadataProvider = null,
        UserSettings? settings = null)
    {
        MonitoringShellViewModel viewModel = CreateViewModel(
            gate ?? CreatePassedGate(),
            metadataProvider ?? CreateNullMetadataProvider(),
            gateway,
            settings);

        await viewModel.BootstrapAsync(CancellationToken.None);
        return viewModel;
    }

    private static IReadOnlyList<Point> ClonePointCollection(IReadOnlyList<Point> points)
    {
        List<Point> clone = new(points.Count);
        foreach (Point point in points)
        {
            clone.Add(new Point(point.X, point.Y));
        }

        return clone;
    }

    private static void AssertPointCollectionsEqual(IReadOnlyList<Point> expected, IReadOnlyList<Point> actual)
    {
        Assert.True(ArePointCollectionsEqual(expected, actual));
    }

    private static void AssertPointCollectionsNotEqual(IReadOnlyList<Point> expected, IReadOnlyList<Point> actual)
    {
        Assert.False(ArePointCollectionsEqual(expected, actual));
    }

    private static bool ArePointCollectionsEqual(IReadOnlyList<Point> left, IReadOnlyList<Point> right)
    {
        if (left.Count != right.Count)
        {
            return false;
        }

        for (int index = 0; index < left.Count; index++)
        {
            if (left[index] != right[index])
            {
                return false;
            }
        }

        return true;
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

    private sealed class TestCollectorFactory : IProcessCollectorFactory
    {
        public IProcessCollector Create(bool _)
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
        private UserSettings _settings;
        private WarmCache? _warmCache;

        public TestPersistenceStore(UserSettings? settings = null)
        {
            _settings = settings ?? new UserSettings();
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
