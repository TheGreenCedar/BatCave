using System.Text.RegularExpressions;

namespace BatCave.Tests.Ui;

public class MainWindowXamlAccessibilityTests
{
    [Fact]
    public void MainWindowXaml_UsesThemeResourcesForTopLevelShellText()
    {
        string xaml = File.ReadAllText(ResolveRepoPath("BatCave", "MainWindow.xaml"));

        Assert.Contains("Text=\"{ThemeResource ShellTitleText}\"", xaml, StringComparison.Ordinal);
        Assert.Contains("Text=\"{ThemeResource ShellSubtitleText}\"", xaml, StringComparison.Ordinal);
        Assert.Contains("PlaceholderText=\"{ThemeResource FilterPlaceholderText}\"", xaml, StringComparison.Ordinal);
        Assert.Contains("Background=\"{ThemeResource BatCaveCanvasBrush}\"", xaml, StringComparison.Ordinal);
    }

    [Fact]
    public void MainWindowXaml_ExposesRuntimeStatusStrip_AndDiagnosticsFooter()
    {
        string xaml = File.ReadAllText(ResolveRepoPath("BatCave", "MainWindow.xaml"));

        Assert.Contains("AutomationProperties.Name=\"Runtime Status Strip\"", xaml, StringComparison.Ordinal);
        Assert.Contains("Background=\"{Binding ViewModel.RuntimeStatusSurfaceBrush, ElementName=RootWindow, Mode=OneWay}\"", xaml, StringComparison.Ordinal);
        Assert.Contains("Fill=\"{Binding ViewModel.RuntimeStatusAccentBrush, ElementName=RootWindow, Mode=OneWay}\"", xaml, StringComparison.Ordinal);
        Assert.Contains("Background=\"{Binding ViewModel.RuntimeStatusTagBackgroundBrush, ElementName=RootWindow, Mode=OneWay}\"", xaml, StringComparison.Ordinal);
        Assert.Contains("Text=\"{Binding ViewModel.RuntimeStatusTag, ElementName=RootWindow, Mode=OneWay}\"", xaml, StringComparison.Ordinal);
        Assert.Contains("Foreground=\"{Binding ViewModel.RuntimeStatusTitleBrush, ElementName=RootWindow, Mode=OneWay}\"", xaml, StringComparison.Ordinal);
        Assert.Contains("Foreground=\"{Binding ViewModel.RuntimeStatusSummaryBrush, ElementName=RootWindow, Mode=OneWay}\"", xaml, StringComparison.Ordinal);
        Assert.Contains("Text=\"{Binding ViewModel.RuntimeStatusTitle, ElementName=RootWindow, Mode=OneWay}\"", xaml, StringComparison.Ordinal);
        Assert.Contains("Text=\"{Binding ViewModel.RuntimeStatusSummary, ElementName=RootWindow, Mode=OneWay}\"", xaml, StringComparison.Ordinal);
        Assert.Contains("<controls:RuntimeStatusFooter", xaml, StringComparison.Ordinal);
    }

    [Fact]
    public void MainWindowXaml_HasAccessibilityLabelsForTrendCharts()
    {
        string xaml = File.ReadAllText(ResolveRepoPath("BatCave", "MainWindow.xaml"));

        Assert.Contains("AutomationProperties.Name=\"Global resource mini trend chart\"", xaml, StringComparison.Ordinal);
        Assert.Contains("AutomationProperties.Name=\"Global primary trend chart\"", xaml, StringComparison.Ordinal);
        Assert.Contains("AutomationProperties.Name=\"Logical CPU trend chart\"", xaml, StringComparison.Ordinal);
    }

    [Fact]
    public void MainWindowXaml_BindsVisiblePointCountForTrendChartsToRootViewModel()
    {
        string xaml = File.ReadAllText(ResolveRepoPath("BatCave", "MainWindow.xaml"));
        const string binding = "VisiblePointCount=\"{Binding ViewModel.MetricTrendWindowSeconds, ElementName=RootWindow, Mode=OneWay}\"";

        Assert.Equal(5, Regex.Matches(xaml, Regex.Escape(binding)).Count);
        Assert.DoesNotContain("VisiblePointCount=\"{Binding MetricTrendWindowSeconds, Mode=OneWay}\"", xaml, StringComparison.Ordinal);
    }

    [Fact]
    public void MainWindowXaml_DoesNotUseHardcodedCpuTrendHexColors()
    {
        string xaml = File.ReadAllText(ResolveRepoPath("BatCave", "MainWindow.xaml"));

        Assert.DoesNotMatch(new Regex("#FF0B84D8|#330B84D8|#FF075C8F", RegexOptions.IgnoreCase), xaml);
        Assert.Contains("ThemeResource ChartCpuStrokeBrush", xaml, StringComparison.Ordinal);
    }

    [Fact]
    public void MainWindowXaml_UsesAdaptiveHeaderControls_AndInspectorSections()
    {
        string xaml = File.ReadAllText(ResolveRepoPath("BatCave", "MainWindow.xaml"));

        Assert.Contains("x:Name=\"HeaderControlsInline\"", xaml, StringComparison.Ordinal);
        Assert.Contains("x:Name=\"HeaderControlsPhone\"", xaml, StringComparison.Ordinal);
        Assert.Contains("x:Name=\"FilterTextBox\"", xaml, StringComparison.Ordinal);
        Assert.Contains("x:Name=\"AdminModeToggle\"", xaml, StringComparison.Ordinal);
        Assert.Contains("Command=\"{Binding SelectInspectorSectionCommand, Mode=OneWay}\"", xaml, StringComparison.Ordinal);
        Assert.Contains("CommandParameter=\"Summary\"", xaml, StringComparison.Ordinal);
        Assert.Contains("CommandParameter=\"Performance\"", xaml, StringComparison.Ordinal);
        Assert.Contains("CommandParameter=\"Details\"", xaml, StringComparison.Ordinal);
        Assert.Contains("Style=\"{StaticResource BatCaveInspectorTabToggleButtonStyle}\"", xaml, StringComparison.Ordinal);
        Assert.Contains("Text=\"{Binding InspectorOverviewEyebrow, Mode=OneWay}\"", xaml, StringComparison.Ordinal);
        Assert.Contains("Text=\"{Binding InspectorOverviewSummary, Mode=OneWay}\"", xaml, StringComparison.Ordinal);
    }

    [Fact]
    public void MainWindowXaml_CompactProcessTable_UsesFlatSelectionAndDiskSort()
    {
        string xaml = File.ReadAllText(ResolveRepoPath("BatCave", "MainWindow.xaml"));

        Assert.Contains("x:Name=\"CompactProcessListView\"", xaml, StringComparison.Ordinal);
        Assert.Contains("AutomationProperties.Name=\"Compact Process Table\"", xaml, StringComparison.Ordinal);
        Assert.Contains("CommandParameter=\"DiskBps\"", xaml, StringComparison.Ordinal);
        Assert.Contains("Text=\"{Binding DiskText, Mode=OneWay}\"", xaml, StringComparison.Ordinal);
        Assert.Contains("Text=\"{Binding NetworkText, Mode=OneWay}\"", xaml, StringComparison.Ordinal);
        Assert.Contains("ResourceKey=\"BatCaveSelectionBrush\"", xaml, StringComparison.Ordinal);
    }

    [Fact]
    public void MainWindowXaml_UsesMetricSwitcherAndCompactInspectorChrome()
    {
        string xaml = File.ReadAllText(ResolveRepoPath("BatCave", "MainWindow.xaml"));

        Assert.Contains("x:Name=\"GlobalResourceListView\"", xaml, StringComparison.Ordinal);
        Assert.Contains("Content=\"Combined\"", xaml, StringComparison.Ordinal);
        Assert.Contains("Content=\"Logical\"", xaml, StringComparison.Ordinal);
        Assert.Contains("Style=\"{StaticResource BatCaveGhostButtonStyle}\"", xaml, StringComparison.Ordinal);
        Assert.Contains("Visibility=\"{Binding SystemSummarySectionVisibility, Mode=OneWay}\"", xaml, StringComparison.Ordinal);
        Assert.Contains("Visibility=\"{Binding ProcessSummarySectionVisibility, Mode=OneWay}\"", xaml, StringComparison.Ordinal);
        Assert.DoesNotContain("Content=\"Clear Selection\"", xaml, StringComparison.Ordinal);
    }

    [Fact]
    public void MainWindowXaml_DoesNotExposeDeprecatedAdvancedProcessArtifacts()
    {
        string xaml = File.ReadAllText(ResolveRepoPath("BatCave", "MainWindow.xaml"));

        Assert.DoesNotContain("x:Name=\"ProcessTableModeToggle\"", xaml, StringComparison.Ordinal);
        Assert.DoesNotContain("Text=\"Advanced Columns\"", xaml, StringComparison.Ordinal);
        Assert.DoesNotContain("AutomationProperties.Name=\"Advanced process columns toggle\"", xaml, StringComparison.Ordinal);
        Assert.DoesNotContain("ViewModel.AdvancedProcessTableVisibility", xaml, StringComparison.Ordinal);
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
