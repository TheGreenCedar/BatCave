namespace BatCave.Tests.Ui;

public sealed class MetricTrendChartSourceTests
{
    [Fact]
    public void MetricTrendChartSource_UsesDedicatedViewportTransitionPathForWindowSwitches()
    {
        string source = File.ReadAllText(ResolveRepoPath("BatCave", "Controls", "MetricTrendChart.xaml.cs"));

        Assert.Contains("ChartIdentityKeyProperty = DependencyProperty.Register(", source, StringComparison.Ordinal);
        Assert.Contains("nameof(ChartIdentityKey)", source, StringComparison.Ordinal);
        Assert.Contains("_pendingViewportSwitch = true;", source, StringComparison.Ordinal);
        Assert.Contains("_transitionSnapshot.VisiblePointCount != visibleCount", source, StringComparison.Ordinal);
        Assert.Contains("_requiresTransitionReset = true;", source, StringComparison.Ordinal);
        Assert.Contains("_requiresSeriesRebuild = true;", source, StringComparison.Ordinal);
        Assert.Contains("bool useInteractiveTransitions = ShouldUseInteractiveTransitions();", source, StringComparison.Ordinal);
        Assert.Contains("bool shrinkingViewportSwitch = viewportSwitchRequested && IsShrinkingViewportSwitch(visibleCount);", source, StringComparison.Ordinal);
        Assert.Contains("if (viewportSwitchRequested && !shrinkingViewportSwitch && ShouldUseViewportTransition(renderMeta.Plan, visibleCount, useInteractiveTransitions))", source, StringComparison.Ordinal);
        Assert.Contains("ApplyViewportTransition(renderMeta.Plan, renderMeta.DomainMax, visibleCount, showAxes);", source, StringComparison.Ordinal);
        Assert.Contains("ApplyViewportCutover(renderMeta.Plan, renderMeta.DomainMax, visibleCount, showAxes);", source, StringComparison.Ordinal);
        Assert.Contains("ReplaceActiveSeries(plan);", source, StringComparison.Ordinal);
        Assert.Contains("SnapshotCurrentStateIntoTransitionSurface();", source, StringComparison.Ordinal);
    }

    [Fact]
    public void MetricTrendChartSource_StopsPriorViewportCrossfadeBeforeStartingNewOne()
    {
        string source = File.ReadAllText(ResolveRepoPath("BatCave", "Controls", "MetricTrendChart.xaml.cs"));

        Assert.Contains("StopViewportTransitionCrossfade();", source, StringComparison.Ordinal);
        Assert.Contains("EnsureViewportTransitionCleanupTimer(duration);", source, StringComparison.Ordinal);
        Assert.Contains("_viewportTransitionCleanupTimer.Stop();", source, StringComparison.Ordinal);
        Assert.Contains("_viewportTransitionCleanupTimer.Tick -= ViewportTransitionCleanupTimer_Tick;", source, StringComparison.Ordinal);
        Assert.Contains("ClearTransitionSurface();", source, StringComparison.Ordinal);
        Assert.Contains("TransitionChart.Series = Array.Empty<ISeries>();", source, StringComparison.Ordinal);
        Assert.Contains("TransitionChart.Visibility = Visibility.Visible;", source, StringComparison.Ordinal);
        Assert.Contains("TransitionChart.Visibility = Visibility.Collapsed;", source, StringComparison.Ordinal);
        Assert.Contains("storyboard.Completed += ViewportTransitionStoryboard_Completed;", source, StringComparison.Ordinal);
    }

    [Fact]
    public void MetricTrendChartSource_KeepsSteadyStateUpdatesOnExistingSeriesPath()
    {
        string source = File.ReadAllText(ResolveRepoPath("BatCave", "Controls", "MetricTrendChart.xaml.cs"));

        Assert.Contains("MetricTrendTransitionMath.CanAnimateTransition(", source, StringComparison.Ordinal);
        Assert.Contains("bool canAnimate = !shrinkingViewportSwitch && MetricTrendTransitionMath.CanAnimateTransition(", source, StringComparison.Ordinal);
        Assert.Contains("|| property == ChartIdentityKeyProperty", source, StringComparison.Ordinal);
        Assert.Contains("_pendingViewportSwitch = false;", source, StringComparison.Ordinal);
        Assert.Contains("PlotBorder.SizeChanged += PlotBorder_SizeChanged;", source, StringComparison.Ordinal);
        Assert.Contains("ResetTransitionState();", source, StringComparison.Ordinal);
        Assert.Contains("bool replaceSeries = false;", source, StringComparison.Ordinal);
        Assert.Contains("RebuildChartSeries();", source, StringComparison.Ordinal);
        Assert.Contains("replaceSeries = true;", source, StringComparison.Ordinal);
        Assert.Contains("private void ApplyViewportCutover(", source, StringComparison.Ordinal);
        Assert.Contains("ApplyAxes(TrendChart, _xAxis, _yAxis, plan, domainMax, visibleCount, showAxes, canAnimate: false);", source, StringComparison.Ordinal);
        Assert.Contains("_hasTransitionSnapshot = false;", source, StringComparison.Ordinal);
        Assert.Contains("_dynamicDomainMaxRaw = 0d;", source, StringComparison.Ordinal);
        Assert.Contains("ApplyAxes(TrendChart, _xAxis, _yAxis, renderMeta.Plan, renderMeta.DomainMax, visibleCount, showAxes, canAnimate);", source, StringComparison.Ordinal);
        Assert.Contains("ReplaceActiveSeries(renderMeta.Plan);", source, StringComparison.Ordinal);
        Assert.Contains("ApplySeries(renderMeta.Plan);", source, StringComparison.Ordinal);
        Assert.Contains("UpdateObservablePoints(_primaryPoints, plan.LineSeries, renderFallback: true);", source, StringComparison.Ordinal);
        Assert.DoesNotContain("|| property == DomainMaxOverrideProperty\r\n                || property == ShowOverlayProperty", source, StringComparison.Ordinal);
    }

    [Fact]
    public void MetricTrendChartSource_HardResetsShrinkingViewportSwitches()
    {
        string source = File.ReadAllText(ResolveRepoPath("BatCave", "Controls", "MetricTrendChart.xaml.cs"));

        Assert.Contains("private bool IsShrinkingViewportSwitch(int visibleCount)", source, StringComparison.Ordinal);
        Assert.Contains("visibleCount < _transitionSnapshot.VisiblePointCount", source, StringComparison.Ordinal);
        Assert.Contains("StopViewportTransitionCrossfade();", source, StringComparison.Ordinal);
        Assert.Contains("RebuildChartSeries();", source, StringComparison.Ordinal);
        Assert.Contains("ReplaceActiveSeries(plan);", source, StringComparison.Ordinal);
        Assert.Contains("&& visibleCount >= _transitionSnapshot.VisiblePointCount", source, StringComparison.Ordinal);
    }

    [Fact]
    public void MetricTrendChartSource_HardResetsRecycledChartsOnDataContextSwap()
    {
        string source = File.ReadAllText(ResolveRepoPath("BatCave", "Controls", "MetricTrendChart.xaml.cs"));

        Assert.Contains("private object? _lastDataContext;", source, StringComparison.Ordinal);
        Assert.Contains("bool dataContextChanged = !ReferenceEquals(_lastDataContext, args.NewValue);", source, StringComparison.Ordinal);
        Assert.Contains("_lastDataContext = args.NewValue;", source, StringComparison.Ordinal);
        Assert.Contains("ResetForDataContextSwap();", source, StringComparison.Ordinal);
        Assert.Contains("private void ResetForDataContextSwap()", source, StringComparison.Ordinal);
        Assert.Contains("ResetTransitionState();", source, StringComparison.Ordinal);
        Assert.Contains("ScheduleRender();", source, StringComparison.Ordinal);
    }

    [Fact]
    public void MetricTrendChartSource_LeavesSizeChangeResetPathUntouched()
    {
        string source = File.ReadAllText(ResolveRepoPath("BatCave", "Controls", "MetricTrendChart.xaml.cs"));

        Assert.Contains("private void PlotBorder_SizeChanged(object sender, SizeChangedEventArgs e)", source, StringComparison.Ordinal);
        Assert.Contains("if (e.NewSize.Width <= 1d || e.NewSize.Height <= 1d)", source, StringComparison.Ordinal);
        Assert.Contains("ResetTransitionState();", source, StringComparison.Ordinal);
        Assert.DoesNotContain("ResetForDataContextSwap();\r\n            return;", source, StringComparison.Ordinal);
    }

    [Fact]
    public void MetricTrendChartSource_InvalidatesLiveChartsSurfaceAfterSeriesUpdates()
    {
        string source = File.ReadAllText(ResolveRepoPath("BatCave", "Controls", "MetricTrendChart.xaml.cs"));

        Assert.Contains("private void InvalidateChartSurfaces(bool includeTransitionSurface = false)", source, StringComparison.Ordinal);
        Assert.Contains("((IChartView)TrendChart).Invalidate();", source, StringComparison.Ordinal);
        Assert.Contains("((IChartView)TransitionChart).Invalidate();", source, StringComparison.Ordinal);
        Assert.Contains("InvalidateChartSurfaces();", source, StringComparison.Ordinal);
        Assert.Contains("InvalidateChartSurfaces(includeTransitionSurface: true);", source, StringComparison.Ordinal);
    }

    [Fact]
    public void MetricTrendChartSource_HardResetReusesCachedSeriesAndPointCollections()
    {
        string source = File.ReadAllText(ResolveRepoPath("BatCave", "Controls", "MetricTrendChart.xaml.cs"));

        Assert.Contains("EnsureSeriesCacheInitialized();", source, StringComparison.Ordinal);
        Assert.Contains("ResetObservablePoints(_primaryPoints);", source, StringComparison.Ordinal);
        Assert.Contains("ResetObservablePoints(_overlayPoints);", source, StringComparison.Ordinal);
        Assert.Contains("ResetObservablePoints(_transitionPrimaryPoints);", source, StringComparison.Ordinal);
        Assert.Contains("ResetObservablePoints(_transitionOverlayPoints);", source, StringComparison.Ordinal);
        Assert.Contains("CopyPoints(_primaryPoints, _transitionPrimaryPoints);", source, StringComparison.Ordinal);
        Assert.Contains("CopyPoints(_overlayPoints, _transitionOverlayPoints);", source, StringComparison.Ordinal);
        Assert.DoesNotContain("CreateObservablePointsCollection(plan.LineSeries, renderFallback: true);", source, StringComparison.Ordinal);
        Assert.DoesNotContain("_primaryPoints = CreateObservablePointsCollection(plan.LineSeries, renderFallback: true);", source, StringComparison.Ordinal);
        Assert.DoesNotContain("_transitionPrimaryPoints = ClonePoints(_primaryPoints);", source, StringComparison.Ordinal);
        Assert.Contains("TrendChart.Series = Array.Empty<ISeries>();", source, StringComparison.Ordinal);
        Assert.Contains("ClearTransitionSurface();", source, StringComparison.Ordinal);
        Assert.Contains("InvalidateChartSurfaces(includeTransitionSurface: true);", source, StringComparison.Ordinal);
    }

    [Fact]
    public void MetricTrendChartSource_UsesLightweightTransitionGateForSmallCharts()
    {
        string source = File.ReadAllText(ResolveRepoPath("BatCave", "Controls", "MetricTrendChart.xaml.cs"));

        Assert.Contains("private const double InteractiveChartMinWidth = 220d;", source, StringComparison.Ordinal);
        Assert.Contains("private const double InteractiveChartMinHeight = 140d;", source, StringComparison.Ordinal);
        Assert.Contains("private bool ShouldUseInteractiveTransitions()", source, StringComparison.Ordinal);
        Assert.Contains("&& ShowGrid", source, StringComparison.Ordinal);
        Assert.Contains("&& PlotBorder.ActualWidth >= InteractiveChartMinWidth", source, StringComparison.Ordinal);
        Assert.Contains("&& PlotBorder.ActualHeight >= InteractiveChartMinHeight;", source, StringComparison.Ordinal);
    }

    [Fact]
    public void MetricTrendChartSource_ReusesCachedPaintsInsteadOfAllocatingPerFrame()
    {
        string source = File.ReadAllText(ResolveRepoPath("BatCave", "Controls", "MetricTrendChart.xaml.cs"));

        Assert.Contains("InitializePaintCache();", source, StringComparison.Ordinal);
        Assert.Contains("_primaryStrokePaint = CreateStrokePaint(", source, StringComparison.Ordinal);
        Assert.Contains("_transitionPrimaryStrokePaint = CreateStrokePaint(", source, StringComparison.Ordinal);
        Assert.Contains("_gridPaint = CreateGridPaint(", source, StringComparison.Ordinal);
        Assert.Contains("UpdateStrokePaint(", source, StringComparison.Ordinal);
        Assert.Contains("UpdateFillPaint(", source, StringComparison.Ordinal);
        Assert.Contains("UpdateGridPaint(_gridPaint", source, StringComparison.Ordinal);
        Assert.Contains("private void ReleaseChartSurfaceReferences()", source, StringComparison.Ordinal);
    }

    [Fact]
    public void MetricTrendChartSource_RendersInlineOnUiThreadAndCoalescesMidRenderRequests()
    {
        string source = File.ReadAllText(ResolveRepoPath("BatCave", "Controls", "MetricTrendChart.xaml.cs"));

        Assert.Contains("private bool _isProcessingRender;", source, StringComparison.Ordinal);
        Assert.Contains("private bool _renderRequestedWhileProcessing;", source, StringComparison.Ordinal);
        Assert.Contains("if (_isProcessingRender)", source, StringComparison.Ordinal);
        Assert.Contains("_renderRequestedWhileProcessing = true;", source, StringComparison.Ordinal);
        Assert.Contains("ProcessScheduledRender();", source, StringComparison.Ordinal);
        Assert.DoesNotContain("TryEnqueue(ScheduleRender)", source, StringComparison.Ordinal);
        Assert.DoesNotContain("TryEnqueue(() => chart.HandleChartPropertyChanged(args.Property, args.OldValue, args.NewValue))", source, StringComparison.Ordinal);
        Assert.DoesNotContain("if (DispatcherQueue is { } queue)\r\n        {\r\n            _ = queue.TryEnqueue(ProcessScheduledRender);\r\n            return;\r\n        }", source, StringComparison.Ordinal);
        Assert.Contains("while (_renderRequestedWhileProcessing);", source, StringComparison.Ordinal);
    }

    [Fact]
    public void MetricTrendChartSource_SkipsSteadyStateRenderWorkWhenPlotSurfaceIsCollapsed()
    {
        string source = File.ReadAllText(ResolveRepoPath("BatCave", "Controls", "MetricTrendChart.xaml.cs"));

        Assert.Contains("if (!TryEnsureUiThreadForRender() || !IsLoaded)", source, StringComparison.Ordinal);
        Assert.Contains("if (PlotBorder.ActualWidth <= 1d || PlotBorder.ActualHeight <= 1d)", source, StringComparison.Ordinal);
        Assert.Contains("ResetTransitionState();", source, StringComparison.Ordinal);
        Assert.Contains("return;", source, StringComparison.Ordinal);
    }

    [Fact]
    public void MetricTrendChartSource_DoesNotWakeEveryInspectorChartForAnyTrendValuesProperty()
    {
        string source = File.ReadAllText(ResolveRepoPath("BatCave", "Controls", "MetricTrendChart.xaml.cs"));

        Assert.Contains("propertyName.Equals(\"GlobalPrimaryTrendValues\", StringComparison.Ordinal)", source, StringComparison.Ordinal);
        Assert.Contains("propertyName.Equals(\"GlobalSecondaryTrendValues\", StringComparison.Ordinal)", source, StringComparison.Ordinal);
        Assert.Contains("propertyName.Equals(\"GlobalAuxiliaryTrendValues\", StringComparison.Ordinal)", source, StringComparison.Ordinal);
        Assert.DoesNotContain("propertyName.EndsWith(\"TrendValues\", StringComparison.Ordinal)", source, StringComparison.Ordinal);
    }

    [Fact]
    public void MainWindowSource_DisablesSmoothPointTransitionsOnLargeInspectorCharts()
    {
        string source = File.ReadAllText(ResolveRepoPath("BatCave", "MainWindow.xaml"));

        Assert.Contains("x:Name=\"SystemPrimaryTrendChart\"", source, StringComparison.Ordinal);
        Assert.Contains("x:Name=\"ProcessPrimaryTrendChart\"", source, StringComparison.Ordinal);
        Assert.Equal(2, CountOccurrences(source, "EnableSmoothPointTransitions=\"False\""));
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

    private static int CountOccurrences(string source, string value)
    {
        int count = 0;
        int searchIndex = 0;
        while ((searchIndex = source.IndexOf(value, searchIndex, StringComparison.Ordinal)) >= 0)
        {
            count++;
            searchIndex += value.Length;
        }

        return count;
    }
}

