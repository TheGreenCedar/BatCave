namespace BatCave.App.Tests;

public sealed class AppSourceContractTests
{
    [Fact]
    public void MainWindow_UsesNativeMonitoringControlsAndAccessibleNames()
    {
        string xaml = ReadRepoFile("src", "BatCave.App", "MainWindow.xaml");

        Assert.Contains("<CommandBar", xaml);
        Assert.Contains("<NavigationView", xaml);
        Assert.Contains("<ListView", xaml);
        Assert.Contains("<ToggleSwitch", xaml);
        Assert.Contains("<KeyboardAccelerator", xaml);
        Assert.Contains("SizeChanged=\"Root_SizeChanged\"", xaml);
        Assert.Contains("AutomationProperties.Name=\"Process List\"", xaml);
        Assert.Contains("AutomationProperties.Name=\"Process Filter\"", xaml);
        Assert.Contains("AutomationProperties.Name=\"Admin Mode\"", xaml);
        Assert.Contains("AutomationProperties.Name=\"Workflow Navigation\"", xaml);
    }

    [Fact]
    public void MainWindow_KeepsCompactProcessListUsableInNarrowLayout()
    {
        string windowCodeBehind = ReadRepoFile("src", "BatCave.App", "MainWindow.xaml.cs");

        Assert.Contains("WideLayoutMinWidth = 1100", windowCodeBehind);
        Assert.Contains("PreferredInitialWindowWidth = 1600", windowCodeBehind);
        Assert.Contains("EnsureUsableInitialWindowSize", windowCodeBehind);
        Assert.Contains("DisplayArea.GetFromWindowId", windowCodeBehind);
        Assert.Contains("if (fallbackWidth > 0)", windowCodeBehind);
        Assert.Contains("ApplyResponsiveLayoutIfChanged(GetResponsiveWidth(Root.ActualWidth))", windowCodeBehind);
        Assert.Contains("NarrowProcessPaneMaxHeight = 360", windowCodeBehind);
        Assert.Contains("CompactProcessListMaxWidth = 1280", windowCodeBehind);
        Assert.Contains("bool compactProcesses = width < CompactProcessListMaxWidth", windowCodeBehind);
        Assert.Contains("DesktopProcessTable.Visibility = compactProcesses ? Visibility.Collapsed : Visibility.Visible", windowCodeBehind);
        Assert.DoesNotContain("NarrowProcessPaneMaxHeight = 58", windowCodeBehind);
    }

    [Fact]
    public void DesktopProcessTableKeepsVisibleSortableHeadersAtDesktopWidths()
    {
        string xaml = ReadRepoFile("src", "BatCave.App", "MainWindow.xaml");
        string windowCodeBehind = ReadRepoFile("src", "BatCave.App", "MainWindow.xaml.cs");

        Assert.Contains("x:Name=\"DesktopProcessTable\"", xaml);
        Assert.Contains("Content=\"Name\"", xaml);
        Assert.Contains("Content=\"CPU\"", xaml);
        Assert.Contains("Content=\"Memory\"", xaml);
        Assert.Contains("Content=\"Disk\"", xaml);
        Assert.Contains("Content=\"Other I/O\"", xaml);
        Assert.Contains("Content=\"PID\"", xaml);
        Assert.Contains("x:Name=\"CpuSortButton\"", xaml);
        Assert.Contains("Click=\"SortButton_Click\"", xaml);
        Assert.Contains("Tag=\"CpuPct\"", xaml);
        Assert.Contains("Tag=\"MemoryBytes\"", xaml);
        Assert.Contains("SortButton_Click", windowCodeBehind);
    }

    [Fact]
    public void DesktopProcessTableMarksActiveSortColumnAndDirection()
    {
        string xaml = ReadRepoFile("src", "BatCave.App", "MainWindow.xaml");
        string windowCodeBehind = ReadRepoFile("src", "BatCave.App", "MainWindow.xaml.cs");
        string viewModel = ReadRepoFile("src", "BatCave.App", "Presentation", "ShellViewModel.cs");

        Assert.Contains("x:Name=\"NameSortButton\"", xaml);
        Assert.Contains("x:Name=\"CpuSortButton\"", xaml);
        Assert.Contains("x:Name=\"MemorySortButton\"", xaml);
        Assert.Contains("x:Name=\"DiskSortButton\"", xaml);
        Assert.Contains("x:Name=\"OtherIoSortButton\"", xaml);
        Assert.Contains("x:Name=\"PidSortButton\"", xaml);
        Assert.Contains("x:Name=\"AttentionSortButton\"", xaml);
        Assert.Contains("CurrentSortColumn", viewModel);
        Assert.Contains("CurrentSortDirection", viewModel);
        Assert.Contains("UpdateSortHeaderVisualState", windowCodeBehind);
        Assert.Contains("ApplySortHeaderVisualState(AttentionSortButton, SortColumn.Attention", windowCodeBehind);
        Assert.Contains("ApplySortHeaderVisualState(CpuSortButton, SortColumn.CpuPct", windowCodeBehind);
        Assert.Contains("directionMarker", windowCodeBehind);
        Assert.Contains("FontWeights.SemiBold", windowCodeBehind);
        Assert.Contains("BatCavePrimaryBrush", windowCodeBehind);
    }

    [Fact]
    public void AdminToggle_BindsToRequestedStateNotEffectiveElevation()
    {
        string xaml = ReadRepoFile("src", "BatCave.App", "MainWindow.xaml");
        string viewModel = ReadRepoFile("src", "BatCave.App", "Presentation", "ShellViewModel.cs");
        string windowCodeBehind = ReadRepoFile("src", "BatCave.App", "MainWindow.xaml.cs");

        Assert.Contains("IsOn=\"{x:Bind ViewModel.AdminModeRequested, Mode=OneWay}\"", xaml);
        Assert.Contains("public bool AdminModeRequested", viewModel);
        Assert.Contains("AdminModeRequested = snapshot.Settings.AdminModeRequested", viewModel);
        Assert.Contains("toggle.IsOn != ViewModel.AdminModeRequested", windowCodeBehind);
        Assert.Contains("AdminStatusText", viewModel);
        Assert.Contains("RetryAdminModeCommand", xaml);
    }

    [Fact]
    public void MainWindow_CloseDisposesViewModelAndShutsDownHostAsync()
    {
        string app = ReadRepoFile("src", "BatCave.App", "App.xaml.cs");
        string windowCodeBehind = ReadRepoFile("src", "BatCave.App", "MainWindow.xaml.cs");

        Assert.Contains("await ViewModel.DisposeAsync()", windowCodeBehind);
        Assert.Contains("finally", windowCodeBehind);
        Assert.Contains("await App.ShutdownServicesAsync()", windowCodeBehind);
        Assert.Contains("ShutdownServicesAsync", app);
        Assert.Contains("ShutdownHostCoreAsync", app);
        Assert.Contains("host is IAsyncDisposable asyncDisposable", app);
        Assert.Contains("await asyncDisposable.DisposeAsync()", app);
    }

    [Fact]
    public void MainWindow_RemovesLegacyDashboardDecorationAndHeavyCharts()
    {
        string xaml = ReadRepoFile("src", "BatCave.App", "MainWindow.xaml");

        Assert.DoesNotContain("LiveCharts", xaml, StringComparison.OrdinalIgnoreCase);
        Assert.DoesNotContain("CartesianChart", xaml, StringComparison.OrdinalIgnoreCase);
        Assert.DoesNotContain("<Path", xaml, StringComparison.OrdinalIgnoreCase);
        Assert.DoesNotContain("Hero", xaml, StringComparison.OrdinalIgnoreCase);
        Assert.Contains("controls:SparklineControl", xaml);
    }

    [Fact]
    public void WinUiLayerConsumesRuntimeContractsAndReducerOnly()
    {
        string viewModel = ReadRepoFile("src", "BatCave.App", "Presentation", "ShellViewModel.cs");
        string windowCodeBehind = ReadRepoFile("src", "BatCave.App", "MainWindow.xaml.cs");

        Assert.Contains("RuntimeViewReducer.Reduce", viewModel);
        Assert.Contains("ApplyResponsiveLayout", windowCodeBehind);
        Assert.Contains("IRuntimeStore", viewModel);
        Assert.DoesNotContain("WindowsProcessCollector", viewModel);
        Assert.DoesNotContain("IProcessCollector", viewModel);
        Assert.DoesNotContain("WindowsProcessCollector", windowCodeBehind);
        Assert.DoesNotContain("IProcessCollector", windowCodeBehind);
    }

    [Fact]
    public void ShellViewModel_PreservesProcessRowsAcrossSnapshotUpdates()
    {
        string viewModel = ReadRepoFile("src", "BatCave.App", "Presentation", "ShellViewModel.cs");
        string rowViewModel = ReadRepoFile("src", "BatCave.App", "Presentation", "ProcessRowViewModel.cs");

        Assert.Contains("ApplyProcessRows", viewModel);
        Assert.DoesNotContain("Rows.Clear()", viewModel);
        Assert.DoesNotContain("Rows[index] = viewRow", viewModel);
        Assert.Contains("Rows.Move", viewModel);
        Assert.Contains("current.Update(sample", viewModel);
        Assert.Contains("current.UpdateSample(sample)", viewModel);
        Assert.Contains(": ObservableObject", rowViewModel);
        Assert.Contains("HasSameDisplayState", rowViewModel);
        Assert.Contains("public void Update(ProcessSample sample", rowViewModel);
        Assert.Contains("public void UpdateSample(ProcessSample sample", rowViewModel);
    }

    [Fact]
    public void ProcessRowsUseOneWayBindingsAndStableSelectionColors()
    {
        string xaml = ReadRepoFile("src", "BatCave.App", "MainWindow.xaml");

        Assert.Contains("ListViewItemBackgroundSelected", xaml);
        Assert.Contains("<TransitionCollection />", xaml);
        Assert.Contains("Text=\"{x:Bind CpuText, Mode=OneWay}\"", xaml);
        Assert.Contains("Text=\"{x:Bind MemoryText, Mode=OneWay}\"", xaml);
        Assert.Contains("Text=\"{x:Bind AttentionBadgeText, Mode=OneWay}\"", xaml);
        Assert.Contains("Foreground=\"{ThemeResource BatCaveTextPrimaryBrush}\"", xaml);
    }

    [Fact]
    public void InspectorTrendsFollowSelectedProcessWithoutRotatingBuffers()
    {
        string xaml = ReadRepoFile("src", "BatCave.App", "MainWindow.xaml");
        string viewModel = ReadRepoFile("src", "BatCave.App", "Presentation", "ShellViewModel.cs");
        string sparkline = ReadRepoFile("src", "BatCave.App", "Controls", "SparklineControl.xaml");

        Assert.Contains("TrendScopeText", xaml);
        Assert.Contains("PROCESS CPU TREND", viewModel);
        Assert.Contains("_processTrendsByIdentity", viewModel);
        Assert.Contains("SelectedRow is { } selectedRow", viewModel);
        Assert.DoesNotContain("RotateTrend", viewModel);
        Assert.Contains("BatCaveChartFillBrush", sparkline);
        Assert.Contains("x:Name=\"Area\"", sparkline);
    }

    [Fact]
    public void ShellViewModel_KeepsSystemOverviewDefaultAndCanClearSelection()
    {
        string viewModel = ReadRepoFile("src", "BatCave.App", "Presentation", "ShellViewModel.cs");
        string reducer = ReadRepoFile("src", "BatCave.Runtime", "Presentation", "RuntimeViewReducer.cs");
        string xaml = ReadRepoFile("src", "BatCave.App", "MainWindow.xaml");

        Assert.Contains("SelectedRow = selectedIdentity.HasValue", viewModel);
        Assert.Contains(": null", viewModel);
        Assert.Contains("ClearSelectionCommand", xaml);
        Assert.Contains("ClearSelectionVisibility", viewModel);
        Assert.Contains("KeyboardAccelerator Key=\"Escape\"", xaml);
        Assert.Contains("return null;", reducer);
    }

    [Fact]
    public void CompactProcessModeKeepsSortingAndConstrainsRowText()
    {
        string xaml = ReadRepoFile("src", "BatCave.App", "MainWindow.xaml");
        string windowCodeBehind = ReadRepoFile("src", "BatCave.App", "MainWindow.xaml.cs");

        Assert.Contains("x:Name=\"CompactProcessSurface\"", xaml);
        Assert.Contains("AutomationProperties.Name=\"Compact Sort\"", xaml);
        Assert.Contains("<Button.Flyout>", xaml);
        Assert.Contains("<MenuFlyoutItem Text=\"CPU\" Tag=\"CpuPct\" Click=\"SortMenuItem_Click\" />", xaml);
        Assert.Contains("SortMenuItem_Click", windowCodeBehind);
        Assert.Contains("CompactProcessSurface.Visibility", windowCodeBehind);

        int compactStart = xaml.IndexOf("x:Name=\"CompactProcessList\"", StringComparison.Ordinal);
        Assert.True(compactStart >= 0);
        string compactTemplate = xaml[compactStart..];
        Assert.Contains("TextTrimming=\"CharacterEllipsis\"", compactTemplate);
        Assert.Contains("<Grid Grid.Row=\"0\" ColumnSpacing=\"8\">", compactTemplate);
        Assert.Contains("<Grid Grid.Row=\"1\" ColumnSpacing=\"8\">", compactTemplate);
        Assert.DoesNotContain("Orientation=\"Horizontal\"", compactTemplate);
    }

    [Fact]
    public void MainWindow_ProvidesCockpitInspectorTabsAndRuntimeConfidence()
    {
        string xaml = ReadRepoFile("src", "BatCave.App", "MainWindow.xaml");
        string viewModel = ReadRepoFile("src", "BatCave.App", "Presentation", "ShellViewModel.cs");

        Assert.Contains("x:Name=\"TriageCockpit\"", xaml);
        Assert.Contains("Attention Cockpit", xaml);
        Assert.Contains("x:Name=\"OverviewChartGrid\"", xaml);
        Assert.Contains("CPU SIMPLE CHART", xaml);
        Assert.Contains("CPU MULTI-CORE VIEW", xaml);
        Assert.Contains("LIVE MEMORY CHART", xaml);
        Assert.Contains("DISK READ CHART", xaml);
        Assert.Contains("DISK WRITE CHART", xaml);
        Assert.Contains("LIVE NETWORK CHART", xaml);
        Assert.Contains("SystemCpuTrendValues", xaml);
        Assert.Contains("LogicalCpuCharts", xaml);
        Assert.Contains("SystemMemoryTrendValues", xaml);
        Assert.Contains("SystemDiskReadTrendValues", xaml);
        Assert.Contains("SystemDiskWriteTrendValues", xaml);
        Assert.Contains("SystemNetworkTrendValues", xaml);
        Assert.Contains("x:Name=\"HealthPane\"", xaml);
        Assert.Contains("x:Name=\"ValidationPane\"", xaml);
        Assert.Contains("WorkflowNav_SelectionChanged", xaml);
        Assert.Contains("ProcessWorkspaceVisibility", xaml);
        Assert.Contains("TriageHeadlineText", xaml);
        Assert.Contains("TriageAttentionTitle", viewModel);
        Assert.Contains("TriageCpuDetail", viewModel);
        Assert.Contains("TopAttentionProcess", viewModel);
        Assert.Contains("ShellWorkflowMode", viewModel);
        Assert.Contains("SelectWorkflow", viewModel);
        Assert.Contains("<Pivot AutomationProperties.Name=\"Inspector Mode Tabs\">", xaml);
        Assert.Contains("<PivotItem Header=\"Summary\">", xaml);
        Assert.Contains("<PivotItem Header=\"Performance\">", xaml);
        Assert.Contains("<PivotItem Header=\"Details\">", xaml);
        Assert.Contains("ProcessStoryText", xaml);
        Assert.Contains("ProcessTimelineItems", xaml);
        Assert.Contains("RuntimeActionTitle", xaml);
        Assert.Contains("HealthDiagnosticsText", xaml);
        Assert.Contains("RUNTIME CHARTS", xaml);
        Assert.Contains("TickP95TrendValues", xaml);
        Assert.Contains("SortP95TrendValues", xaml);
        Assert.Contains("JitterP95TrendValues", xaml);
        Assert.Contains("RuntimeConfidenceText", xaml);
        Assert.Contains("RuntimePerfText", xaml);
        Assert.Contains("RuntimeBudgetText", viewModel);
        Assert.Contains("LogicalCpuChartViewModel", viewModel);
        Assert.Contains("SystemNetworkChartText", viewModel);
        Assert.Contains("BenchmarkStatusText", viewModel);
        Assert.Contains("BenchmarkEvidenceText", viewModel);
        Assert.Contains("ValidationHeadlineText", xaml);
    }

    [Fact]
    public void ProcessRowsExposeContextMenuActions()
    {
        string xaml = ReadRepoFile("src", "BatCave.App", "MainWindow.xaml");
        string windowCodeBehind = ReadRepoFile("src", "BatCave.App", "MainWindow.xaml.cs");
        string viewModel = ReadRepoFile("src", "BatCave.App", "Presentation", "ShellViewModel.cs");

        Assert.Contains("ProcessRow_RightTapped", xaml);
        Assert.Contains("ProcessRowMenuItem_Click", xaml);
        Assert.Contains("Text=\"Copy details\"", xaml);
        Assert.Contains("Text=\"Copy PID\"", xaml);
        Assert.Contains("Text=\"Filter to process\"", xaml);
        Assert.Contains("FilterToSelectedProcess", viewModel);
        Assert.Contains("CopyTextToClipboard", windowCodeBehind);
    }

    [Fact]
    public void ShellViewModel_DebouncesFiltersAndSupportsQuickFilters()
    {
        string xaml = ReadRepoFile("src", "BatCave.App", "MainWindow.xaml");
        string viewModel = ReadRepoFile("src", "BatCave.App", "Presentation", "ShellViewModel.cs");

        Assert.Contains("FilterDebounceMs = 150", viewModel);
        Assert.Contains("QueueFilterUpdate", viewModel);
        Assert.Contains("ClearFilterCommand", xaml);
        Assert.Contains("ApplyQuickFilterCommand", xaml);
        Assert.Contains("CommandParameter=\"HighCpu\"", xaml);
        Assert.Contains("CommandParameter=\"HighMemory\"", xaml);
        Assert.Contains("CommandParameter=\"ActiveIo\"", xaml);
        Assert.Contains("CommandParameter=\"LimitedAccess\"", xaml);
        Assert.Contains("EmptyStateText", viewModel);
    }

    [Fact]
    public void ProcessInspector_ExposesAttentionDetailsAndCopyCommand()
    {
        string xaml = ReadRepoFile("src", "BatCave.App", "MainWindow.xaml");
        string windowCodeBehind = ReadRepoFile("src", "BatCave.App", "MainWindow.xaml.cs");
        string rowViewModel = ReadRepoFile("src", "BatCave.App", "Presentation", "ProcessRowViewModel.cs");

        Assert.Contains("AttentionBadgeText", xaml);
        Assert.Contains("InspectorLastChangeText", xaml);
        Assert.Contains("CopyDetails_Click", xaml);
        Assert.Contains("Clipboard.SetContent", windowCodeBehind);
        Assert.Contains("ToClipboardText", rowViewModel);
        Assert.Contains("ParentPidText", rowViewModel);
        Assert.Contains("PrivateMemoryText", rowViewModel);
        Assert.Contains("AccessStateText", rowViewModel);
    }

    [Fact]
    public void ShellViewModel_UsesSystemIoMetricsForInspectorOverview()
    {
        string viewModel = ReadRepoFile("src", "BatCave.App", "Presentation", "ShellViewModel.cs");

        Assert.DoesNotContain("?? \"0 B/s\"", viewModel);
        Assert.Contains("FormatSystemDisk(_snapshot.System)", viewModel);
        Assert.Contains("FormatNullableRate(_snapshot.System.OtherIoBps)", viewModel);
        Assert.Contains("OnPropertyChanged(nameof(InspectorDiskText))", viewModel);
        Assert.Contains("OnPropertyChanged(nameof(InspectorOtherIoText))", viewModel);
    }

    [Fact]
    public void AppLaunch_SeparatesCliFromInteractiveRuntimeAndGatesBeforeOpeningMonitor()
    {
        string app = ReadRepoFile("src", "BatCave.App", "App.xaml.cs");

        Assert.Contains("CreateHost(registerRuntimeLoop: !cliMode)", app);
        Assert.Contains("AddBatCaveRuntime(registerHostedService: registerRuntimeLoop)", app);

        int cliBranch = app.IndexOf("if (cliMode)", StringComparison.Ordinal);
        int cliExecute = app.IndexOf("GetRequiredService<CliOperationsHost>()", StringComparison.Ordinal);
        int gate = app.IndexOf("StartupGateStatus gateStatus", StringComparison.Ordinal);
        int gateExit = app.IndexOf("if (!gateStatus.Passed)", StringComparison.Ordinal);
        int hostStart = app.IndexOf("await _host.StartAsync()", StringComparison.Ordinal);
        int mainWindow = app.IndexOf("GetRequiredService<MainWindow>()", StringComparison.Ordinal);

        Assert.True(cliBranch >= 0);
        Assert.True(cliBranch < cliExecute);
        Assert.True(cliExecute < gate);
        Assert.True(gate < gateExit);
        Assert.True(gateExit < hostStart);
        Assert.True(hostStart < mainWindow);
    }

    [Fact]
    public void WinUiBenchmark_DrivesShellViewModelAndDispatcherPath()
    {
        string app = ReadRepoFile("src", "BatCave.App", "App.xaml.cs");
        string runner = ReadRepoFile("src", "BatCave.App", "Benchmarking", "ShellWinUiBenchmarkRunner.cs");

        Assert.Contains("IWinUiBenchmarkRunner", app);
        Assert.Contains("ShellWinUiBenchmarkRunner", app);
        Assert.Contains("ShellViewModel", runner);
        Assert.Contains("DispatcherQueue.GetForCurrentThread", runner);
        Assert.Contains("RefreshCommand.ExecuteAsync", runner);
        Assert.Contains("SortCommand.ExecuteAsync", runner);
        Assert.Contains("WaitForViewModelSnapshotAsync", runner);
    }

    [Fact]
    public void ProjectFile_PreservesWinUiSdkAndAppIdentityContracts()
    {
        string project = ReadRepoFile("src", "BatCave.App", "BatCave.App.csproj");
        string buildProps = ReadRepoFile("Directory.Build.props");

        Assert.Contains("<TargetFramework Condition=\"'$(TargetFramework)' == ''\">net10.0-windows10.0.19041.0</TargetFramework>", buildProps);
        Assert.Contains("<UseWinUI>true</UseWinUI>", project);
        Assert.Contains("<AssemblyName>BatCave</AssemblyName>", project);
        Assert.Contains("<ApplicationManifest>app.manifest</ApplicationManifest>", project);
        Assert.Contains("<PackageReference Include=\"Microsoft.WindowsAppSDK\"", project);
    }

    private static string ReadRepoFile(params string[] segments)
    {
        string root = FindRepositoryRoot();
        return File.ReadAllText(Path.Combine([root, .. segments]));
    }

    private static string FindRepositoryRoot()
    {
        DirectoryInfo? directory = new(AppContext.BaseDirectory);
        while (directory is not null)
        {
            if (File.Exists(Path.Combine(directory.FullName, "BatCave.slnx")))
            {
                return directory.FullName;
            }

            directory = directory.Parent;
        }

        throw new InvalidOperationException("Could not locate repository root.");
    }
}
