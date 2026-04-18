namespace BatCave.Tests.Ui;

public class MainWindowSourceTests
{
    [Fact]
    public void MainWindowSource_AppliesPackagedWindowIconFromAssets()
    {
        string source = File.ReadAllText(ResolveRepoPath("BatCave", "MainWindow.xaml.cs"));

        Assert.Contains("TryApplyWindowIcon();", source, StringComparison.Ordinal);
        Assert.Contains("Path.Combine(AppContext.BaseDirectory, \"Assets\", \"BatCaveLogo.ico\")", source, StringComparison.Ordinal);
        Assert.Contains("WindowNative.GetWindowHandle(this)", source, StringComparison.Ordinal);
        Assert.Contains("AppWindow.GetFromWindowId(windowId)", source, StringComparison.Ordinal);
        Assert.Contains("_appWindow.SetIcon(iconPath);", source, StringComparison.Ordinal);
    }

    [Fact]
    public void MainWindowSource_DoesNotDriveInspectorChartsFromWindowHeight()
    {
        string source = File.ReadAllText(ResolveRepoPath("BatCave", "MainWindow.xaml.cs"));

        Assert.DoesNotContain("SizeChanged += OnWindowSizeChanged;", source, StringComparison.Ordinal);
        Assert.DoesNotContain("ApplyInspectorChartSizing(", source, StringComparison.Ordinal);
        Assert.DoesNotContain("SystemPrimaryTrendChart.Height =", source, StringComparison.Ordinal);
        Assert.DoesNotContain("SystemAuxTrendChart.Height =", source, StringComparison.Ordinal);
        Assert.DoesNotContain("ProcessPrimaryTrendChart.Height =", source, StringComparison.Ordinal);
        Assert.DoesNotContain("ProcessLogicalPlaceholder.MinHeight =", source, StringComparison.Ordinal);
    }

    [Fact]
    public void MainWindowSource_LogicalCpuLayout_UsesVisibleScrollerWidthOnly()
    {
        string source = File.ReadAllText(ResolveRepoPath("BatCave", "MainWindow.xaml.cs"));

        Assert.Contains("double scrollerWidth = GlobalCpuLogicalGridScroller.ActualWidth;", source, StringComparison.Ordinal);
        Assert.Contains("double.PositiveInfinity", source, StringComparison.Ordinal);
        Assert.DoesNotContain("GlobalCpuLogicalGridHost.ActualHeight", source, StringComparison.Ordinal);
        Assert.DoesNotContain("GlobalCpuLogicalRepeater.InvalidateMeasure();", source, StringComparison.Ordinal);
    }

    [Fact]
    public void MainWindowSource_UsesWidthDrivenLogicalCpuGridLayoutWithoutManualTileSizing()
    {
        string source = File.ReadAllText(ResolveRepoPath("BatCave", "MainWindow.xaml.cs"));

        Assert.Contains("GlobalCpuLogicalGridScroller.ActualWidth", source, StringComparison.Ordinal);
        Assert.DoesNotContain("GlobalCpuLogicalRepeater.InvalidateMeasure();", source, StringComparison.Ordinal);
        Assert.DoesNotContain("private void GlobalCpuLogicalRepeater_ElementPrepared(", source, StringComparison.Ordinal);
        Assert.DoesNotContain("element.Width =", source, StringComparison.Ordinal);
        Assert.DoesNotContain("element.Height =", source, StringComparison.Ordinal);
        Assert.DoesNotContain("trendChart.Height =", source, StringComparison.Ordinal);
    }

    [Fact]
    public void MainWindowSource_UsesCustomTitleBarWithoutHeaderDecorationAnimations()
    {
        string source = File.ReadAllText(ResolveRepoPath("BatCave", "MainWindow.xaml.cs"));

        Assert.Contains("ExtendsContentIntoTitleBar = true;", source, StringComparison.Ordinal);
        Assert.Contains("SetTitleBar(TitleBarDragRegion);", source, StringComparison.Ordinal);
        Assert.Contains("ShellRoot.ActualThemeChanged += ShellRoot_ActualThemeChanged;", source, StringComparison.Ordinal);
        Assert.Contains("AppWindowTitleBar.IsCustomizationSupported()", source, StringComparison.Ordinal);
        Assert.Contains("ApplyTitleBarButtonColors();", source, StringComparison.Ordinal);
        Assert.DoesNotContain("HeaderDecorationCanvas", source, StringComparison.Ordinal);
        Assert.DoesNotContain("HeaderRegion_SizeChanged", source, StringComparison.Ordinal);
        Assert.DoesNotContain("HeaderBatGlidePathData", source, StringComparison.Ordinal);
        Assert.DoesNotContain("ResetHeaderDecorationAnimations", source, StringComparison.Ordinal);
    }

    [Fact]
    public void MainWindowSource_DoesNotForceCompactProcessScrollResetOnSort()
    {
        string source = File.ReadAllText(ResolveRepoPath("BatCave", "MainWindow.xaml.cs"));

        Assert.Contains("private void CompactProcessSortHeader_Click(object sender, RoutedEventArgs e)", source, StringComparison.Ordinal);
        Assert.Contains("_compactProcessSortRestoreOffset = TryGetCompactProcessScrollOffset();", source, StringComparison.Ordinal);
        Assert.Contains("DispatcherQueue.TryEnqueue(CompleteCompactProcessSortInteraction);", source, StringComparison.Ordinal);
        Assert.Contains("CompactProcessListView.LayoutUpdated += CompactProcessListView_LayoutUpdated;", source, StringComparison.Ordinal);
        Assert.Contains("private void CompactProcessListView_LayoutUpdated(object? sender, object e)", source, StringComparison.Ordinal);
        Assert.Contains("DispatcherQueue.TryEnqueue(FinalizeCompactProcessSortInteraction);", source, StringComparison.Ordinal);
        Assert.Contains("private void RestoreCompactProcessScrollOffsetIfNeeded()", source, StringComparison.Ordinal);
        Assert.Contains("scrollViewer?.ChangeView(null, verticalOffset, null, disableAnimation: true);", source, StringComparison.Ordinal);
        Assert.Contains("CompactProcessListView.ScrollIntoView(ViewModel.VisibleRows[0], ScrollIntoViewAlignment.Leading);", source, StringComparison.Ordinal);
    }

    [Fact]
    public void MainWindowSource_QueuesOneTimeInitialScrollWhenVisibleRowsFirstLoad()
    {
        string source = File.ReadAllText(ResolveRepoPath("BatCave", "MainWindow.xaml.cs"));

        Assert.Contains("private bool _compactProcessInitialScrollPending = true;", source, StringComparison.Ordinal);
        Assert.Contains("if (ViewModel.VisibleRows is INotifyCollectionChanged visibleRows)", source, StringComparison.Ordinal);
        Assert.Contains("visibleRows.CollectionChanged += VisibleRows_CollectionChanged;", source, StringComparison.Ordinal);
        Assert.Contains("private void VisibleRows_CollectionChanged(object? sender, NotifyCollectionChangedEventArgs e)", source, StringComparison.Ordinal);
        Assert.Contains("QueueCompactProcessInitialScrollIfNeeded();", source, StringComparison.Ordinal);
        Assert.Contains("if (!_compactProcessInitialScrollPending || ViewModel.VisibleRows.Count <= 0)", source, StringComparison.Ordinal);
        Assert.Contains("_compactProcessInitialScrollPending = false;", source, StringComparison.Ordinal);
        Assert.Contains("visibleRows.CollectionChanged -= VisibleRows_CollectionChanged;", source, StringComparison.Ordinal);
    }

    [Fact]
    public void MainWindowSource_DisablesRepositionAnimationsForLiveProcessTable()
    {
        string source = File.ReadAllText(ResolveRepoPath("BatCave", "MainWindow.xaml"));

        Assert.Contains("x:Name=\"CompactProcessListView\"", source, StringComparison.Ordinal);
        Assert.DoesNotContain("RepositionThemeTransition", source, StringComparison.Ordinal);
        Assert.DoesNotContain("<ListView.ItemContainerTransitions>", source, StringComparison.Ordinal);
        Assert.DoesNotContain("SelectedItem=\"{x:Bind ViewModel.SelectedVisibleRowBinding, Mode=OneWay}\"", source, StringComparison.Ordinal);
    }

    [Fact]
    public void MainWindowSource_SyncsCompactProcessSelectionFromCodeBehind()
    {
        string source = File.ReadAllText(ResolveRepoPath("BatCave", "MainWindow.xaml.cs"));

        Assert.Contains("case nameof(MonitoringShellViewModel.SelectedVisibleRowBinding):", source, StringComparison.Ordinal);
        Assert.Contains("if (!_compactProcessSortPending)", source, StringComparison.Ordinal);
        Assert.Contains("SyncCompactProcessSelection();", source, StringComparison.Ordinal);
        Assert.Contains("private void QueueCompactProcessSelectionRestore()", source, StringComparison.Ordinal);
        Assert.Contains("DispatcherQueue.TryEnqueue(RestoreCompactProcessSelectionIfNeeded);", source, StringComparison.Ordinal);
        Assert.Contains("private void RestoreCompactProcessSelectionIfNeeded()", source, StringComparison.Ordinal);
        Assert.Contains("if (_compactProcessSortPending)", source, StringComparison.Ordinal);
        Assert.Contains("CompactProcessListView.SelectedItem = selectedVisibleRow;", source, StringComparison.Ordinal);
        Assert.DoesNotContain("ViewModel.SelectedVisibleRowBinding = null;", source, StringComparison.Ordinal);
    }

    [Fact]
    public void MainWindowSource_IgnoresTransientNullChurnForGlobalResourceSelection()
    {
        string source = File.ReadAllText(ResolveRepoPath("BatCave", "MainWindow.xaml.cs"));

        Assert.Contains("private void GlobalResourceListView_SelectionChanged(object sender, SelectionChangedEventArgs e)", source, StringComparison.Ordinal);
        Assert.Contains("if (listView.SelectedItem is GlobalResourceRowViewState selected)", source, StringComparison.Ordinal);
        Assert.Contains("if (ViewModel.SelectedGlobalResource is not null && ViewModel.GlobalResourceRows.Count > 0)", source, StringComparison.Ordinal);
        Assert.Contains("private bool _globalResourceSelectionRestoreQueued;", source, StringComparison.Ordinal);
        Assert.Contains("QueueGlobalResourceSelectionRestore();", source, StringComparison.Ordinal);
        Assert.Contains("private void QueueGlobalResourceSelectionRestore()", source, StringComparison.Ordinal);
        Assert.Contains("DispatcherQueue.TryEnqueue(RestoreGlobalResourceSelectionIfNeeded);", source, StringComparison.Ordinal);
        Assert.Contains("private void RestoreGlobalResourceSelectionIfNeeded()", source, StringComparison.Ordinal);
        Assert.Contains("GlobalResourceListView.SelectedItem = ViewModel.SelectedGlobalResource;", source, StringComparison.Ordinal);
        Assert.Contains("if (ViewModel.SelectedGlobalResource is not null)", source, StringComparison.Ordinal);
        Assert.Contains("ViewModel.SelectedGlobalResource = null;", source, StringComparison.Ordinal);
    }

    [Fact]
    public void MainWindowSource_CoalescesLogicalCpuLayoutQueueWithoutInlineLambda()
    {
        string source = File.ReadAllText(ResolveRepoPath("BatCave", "MainWindow.xaml.cs"));

        Assert.Contains("DispatcherQueue.TryEnqueue(ApplyQueuedLogicalCpuGridLayout);", source, StringComparison.Ordinal);
        Assert.Contains("private void ApplyQueuedLogicalCpuGridLayout()", source, StringComparison.Ordinal);
        Assert.DoesNotContain("DispatcherQueue.TryEnqueue(() =>", source, StringComparison.Ordinal);
    }

    [Fact]
    public void MainWindowSource_UpdatesMotionResourcesAndChartHoverOverlays()
    {
        string source = File.ReadAllText(ResolveRepoPath("BatCave", "MainWindow.xaml.cs"));

        Assert.Contains("new Windows.UI.ViewManagement.UISettings()", source, StringComparison.Ordinal);
        Assert.Contains("BatCaveInteractivePointerOverOpacity", source, StringComparison.Ordinal);
        Assert.Contains("BatCaveChartSmoothTransitionsEnabled", source, StringComparison.Ordinal);
        Assert.Contains("private void InspectorChartOverlay_PointerMoved(object sender, PointerRoutedEventArgs e)", source, StringComparison.Ordinal);
        Assert.Contains("private void InspectorChartOverlay_PointerExited(object sender, PointerRoutedEventArgs e)", source, StringComparison.Ordinal);
    }

    private static string ResolveRepoPath(params string[] relativeSegments)
    {
        DirectoryInfo? current = new(AppContext.BaseDirectory);
        while (current is not null)
        {
            string candidate = Path.Combine(current.FullName, "BatCave.slnx");
            if (File.Exists(candidate))
            {
                string resolved = current.FullName;
                foreach (string segment in relativeSegments)
                {
                    resolved = Path.Combine(resolved, segment);
                }

                return resolved;
            }

            current = current.Parent;
        }

        throw new DirectoryNotFoundException("Could not locate repository root from test base directory.");
    }
}
