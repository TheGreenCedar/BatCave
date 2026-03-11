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
        Assert.Contains("bool shrinkingViewportSwitch = viewportSwitchRequested && IsShrinkingViewportSwitch(visibleCount);", source, StringComparison.Ordinal);
        Assert.Contains("if (viewportSwitchRequested && !shrinkingViewportSwitch && ShouldUseViewportTransition(renderMeta.Plan, visibleCount))", source, StringComparison.Ordinal);
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
        Assert.Contains("_viewportTransitionCleanupTimer?.Stop();", source, StringComparison.Ordinal);
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
        Assert.Contains("RebuildChartSeries();", source, StringComparison.Ordinal);
        Assert.Contains("private void ApplyViewportCutover(", source, StringComparison.Ordinal);
        Assert.Contains("ApplyAxes(TrendChart, _xAxis, _yAxis, plan, domainMax, visibleCount, showAxes, canAnimate: false);", source, StringComparison.Ordinal);
        Assert.Contains("_hasTransitionSnapshot = false;", source, StringComparison.Ordinal);
        Assert.Contains("_dynamicDomainMaxRaw = 0d;", source, StringComparison.Ordinal);
        Assert.Contains("ApplyAxes(TrendChart, _xAxis, _yAxis, renderMeta.Plan, renderMeta.DomainMax, visibleCount, showAxes, canAnimate);", source, StringComparison.Ordinal);
        Assert.Contains("ApplySeries(renderMeta.Plan);", source, StringComparison.Ordinal);
        Assert.Contains("UpdateObservablePoints(_primaryPoints, plan.LineSeries, renderFallback: true);", source, StringComparison.Ordinal);
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
