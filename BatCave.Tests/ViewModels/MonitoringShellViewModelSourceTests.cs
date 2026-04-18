namespace BatCave.Tests.ViewModels;

public sealed class MonitoringShellViewModelSourceTests
{
    [Fact]
    public void MonitoringShellViewModelSource_CoalescesHotRuntimeEventsBeforeUiDrain()
    {
        string telemetrySource = File.ReadAllText(ResolveRepoPath("BatCave", "ViewModels", "MonitoringShellViewModel.Telemetry.cs"));
        string bootstrapSource = File.ReadAllText(ResolveRepoPath("BatCave", "ViewModels", "MonitoringShellViewModel.Bootstrap.cs"));

        Assert.Contains("QueuePendingTelemetryDelta(delta);", telemetrySource, StringComparison.Ordinal);
        Assert.Contains("QueuePendingRuntimeHealth(health);", bootstrapSource, StringComparison.Ordinal);
        Assert.Contains("QueuePendingCollectorWarning(warning);", bootstrapSource, StringComparison.Ordinal);
        Assert.Contains("RunDispatcherHandlerOnUiThread(DrainPendingUiEvents);", telemetrySource, StringComparison.Ordinal);
        Assert.Contains("private void DrainPendingUiEvents()", telemetrySource, StringComparison.Ordinal);
        Assert.Contains("private void MergePendingTelemetry(ProcessDeltaBatch delta)", telemetrySource, StringComparison.Ordinal);
        Assert.DoesNotContain("RunOnUiThread(() => ApplyTelemetryDelta(delta));", telemetrySource, StringComparison.Ordinal);
        Assert.DoesNotContain("RunOnUiThread(() => ApplyRuntimeHealth(health));", bootstrapSource, StringComparison.Ordinal);
        Assert.DoesNotContain("RunOnUiThread(() => ApplyCollectorWarning(warning));", bootstrapSource, StringComparison.Ordinal);
    }

    [Fact]
    public void MonitoringShellViewModelSource_UsesMethodGroupUiDrainForQueuedDetailRefresh()
    {
        string sortingSource = File.ReadAllText(ResolveRepoPath("BatCave", "ViewModels", "MonitoringShellViewModel.Sorting.cs"));
        string globalPerformanceSource = File.ReadAllText(ResolveRepoPath("BatCave", "ViewModels", "MonitoringShellViewModel.GlobalPerformance.cs"));

        Assert.Contains("private void RunDispatcherHandlerOnUiThread(DispatcherQueueHandler callback)", sortingSource, StringComparison.Ordinal);
        Assert.Contains("RunDispatcherHandlerOnUiThread(DrainQueuedGlobalDetailStateRefresh);", globalPerformanceSource, StringComparison.Ordinal);
        Assert.Contains("private void DrainQueuedGlobalDetailStateRefresh()", globalPerformanceSource, StringComparison.Ordinal);
        Assert.DoesNotContain("RunOnUiThread(() =>", globalPerformanceSource, StringComparison.Ordinal);
    }

    [Fact]
    public void MonitoringShellViewModelSource_DoesNotForceSelectionBindingReassertionsAfterSort()
    {
        string selectionSource = File.ReadAllText(ResolveRepoPath("BatCave", "ViewModels", "MonitoringShellViewModel.Selection.cs"));

        Assert.Contains("_ = TrySyncSelectedVisibleRowFromTrackedRows(ResolveVisibleSelectionAfterSort, out _);", selectionSource, StringComparison.Ordinal);
        Assert.DoesNotContain("ReassertSelectedVisibleRowBindingOnDispatcher(", selectionSource, StringComparison.Ordinal);
    }

    [Fact]
    public void MonitoringShellViewModelTelemetrySource_OnlyRefreshesFilterForMembershipChanges()
    {
        string telemetrySource = File.ReadAllText(ResolveRepoPath("BatCave", "ViewModels", "MonitoringShellViewModel.Telemetry.cs"));

        Assert.DoesNotContain("bool hasActiveVisibilityFilter = !adminModeEnabled || adminEnabledOnlyFilter;", telemetrySource, StringComparison.Ordinal);
        Assert.DoesNotContain("if (hasActiveTextFilter || hasActiveVisibilityFilter)", telemetrySource, StringComparison.Ordinal);
        Assert.Contains("refreshFilter |= DidVisibilityMembershipChange(", telemetrySource, StringComparison.Ordinal);
    }

    [Fact]
    public void MonitoringShellViewModelTelemetrySource_AvoidsBuildingUnusedGlobalDescriptorsForProcessInspectorRefresh()
    {
        string telemetrySource = File.ReadAllText(ResolveRepoPath("BatCave", "ViewModels", "MonitoringShellViewModel.Telemetry.cs"));

        Assert.Contains("BuildAndAppendProcessResourceRows();", telemetrySource, StringComparison.Ordinal);
        Assert.DoesNotContain("BuildAndAppendResourceRows(BuildGlobalResourceDescriptors(_latestGlobalMetricsSample));", telemetrySource, StringComparison.Ordinal);
    }

    [Fact]
    public void MonitoringShellViewModelSource_SelectionRefreshBranchesDirectlyToProcessInspectorRows()
    {
        string source = File.ReadAllText(ResolveRepoPath("BatCave", "ViewModels", "MonitoringShellViewModel.cs"));

        Assert.Contains("private void RefreshSelectionInspectorState()", source, StringComparison.Ordinal);
        Assert.Contains("if (SelectedRow is null)", source, StringComparison.Ordinal);
        Assert.Contains("BuildAndAppendResourceRows(BuildGlobalResourceDescriptors(_latestGlobalMetricsSample));", source, StringComparison.Ordinal);
        Assert.Contains("BuildAndAppendProcessResourceRows();", source, StringComparison.Ordinal);
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
