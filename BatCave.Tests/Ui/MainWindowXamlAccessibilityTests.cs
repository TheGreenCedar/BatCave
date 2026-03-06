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

        Assert.Equal(4, Regex.Matches(xaml, Regex.Escape(binding)).Count);
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
    public void MainWindowXaml_UsesCompactResponsiveHeaderControls()
    {
        string xaml = File.ReadAllText(ResolveRepoPath("BatCave", "MainWindow.xaml"));

        Assert.Contains("x:Name=\"HeaderControlsInline\"", xaml, StringComparison.Ordinal);
        Assert.Contains("x:Name=\"HeaderControlsPhone\"", xaml, StringComparison.Ordinal);
        Assert.Contains("x:Name=\"FilterTextBox\"", xaml, StringComparison.Ordinal);
        Assert.Contains("x:Name=\"AdminModeToggle\"", xaml, StringComparison.Ordinal);
    }

    [Fact]
    public void MainWindowXaml_DoesNotExposeAdminWarningBanner()
    {
        string xaml = File.ReadAllText(ResolveRepoPath("BatCave", "MainWindow.xaml"));

        Assert.DoesNotContain("AutomationProperties.Name=\"Runtime Warning Banner\"", xaml, StringComparison.Ordinal);
        Assert.DoesNotContain("ViewModel.AdminModeError", xaml, StringComparison.Ordinal);
        Assert.DoesNotContain("ViewModel.AdminErrorVisibility", xaml, StringComparison.Ordinal);
    }

    [Fact]
    public void MainWindowXaml_CompactProcessTable_UsesTaskManagerColumnsAndDiskBpsSort()
    {
        string xaml = File.ReadAllText(ResolveRepoPath("BatCave", "MainWindow.xaml"));

        Assert.Contains("x:Name=\"CompactProcessListView\"", xaml, StringComparison.Ordinal);
        Assert.Contains("AutomationProperties.Name=\"Compact Process Table\"", xaml, StringComparison.Ordinal);
        Assert.Contains("Command=\"{x:Bind ViewModel.CompactSortHeaderCommand}\"", xaml, StringComparison.Ordinal);
        Assert.Contains("CommandParameter=\"DiskBps\"", xaml, StringComparison.Ordinal);
        Assert.Contains("Text=\"{x:Bind DiskText, Mode=OneWay}\"", xaml, StringComparison.Ordinal);
        Assert.Contains("Text=\"{x:Bind NetworkText, Mode=OneWay}\"", xaml, StringComparison.Ordinal);
    }

    [Fact]
    public void MainWindowXaml_CompactProcessTable_HasSmoothRepositionAndStrongerSelectionVisuals()
    {
        string xaml = File.ReadAllText(ResolveRepoPath("BatCave", "MainWindow.xaml"));

        int compactListIndex = xaml.IndexOf("x:Name=\"CompactProcessListView\"", StringComparison.Ordinal);
        Assert.True(compactListIndex >= 0, "Compact process ListView definition not found.");

        string compactSection = xaml[compactListIndex..];
        Assert.Contains("<ListView.ItemContainerTransitions>", compactSection, StringComparison.Ordinal);
        Assert.Contains("<RepositionThemeTransition IsStaggeringEnabled=\"False\" />", compactSection, StringComparison.Ordinal);
        Assert.Contains("ListViewItemBackgroundSelected\" ResourceKey=\"AccentFillColorDefaultBrush\"", compactSection, StringComparison.Ordinal);
        Assert.Contains("ListViewItemForegroundSelected\" ResourceKey=\"TextOnAccentFillColorPrimaryBrush\"", compactSection, StringComparison.Ordinal);
        Assert.Contains("ListViewItemSelectionIndicatorVisualEnabled\">True", compactSection, StringComparison.Ordinal);
    }

    [Fact]
    public void MainWindowXaml_DoesNotExposeAdvancedProcessTableArtifacts_AndKeepsCompactTable()
    {
        string xaml = File.ReadAllText(ResolveRepoPath("BatCave", "MainWindow.xaml"));

        Assert.DoesNotContain("x:Name=\"ProcessTableModeToggle\"", xaml, StringComparison.Ordinal);
        Assert.DoesNotContain("Text=\"Advanced Columns\"", xaml, StringComparison.Ordinal);
        Assert.DoesNotContain("AutomationProperties.Name=\"Advanced process columns toggle\"", xaml, StringComparison.Ordinal);
        Assert.DoesNotContain("AutomationProperties.Name=\"Hidden compact sort indicator\"", xaml, StringComparison.Ordinal);
        Assert.DoesNotContain("ViewModel.AdvancedProcessTableVisibility", xaml, StringComparison.Ordinal);
        Assert.DoesNotContain("ViewModel.CompactHiddenSortActiveVisibility", xaml, StringComparison.Ordinal);
        Assert.Contains("x:Name=\"CompactProcessListView\"", xaml, StringComparison.Ordinal);
        Assert.Contains("AutomationProperties.Name=\"Compact Process Table\"", xaml, StringComparison.Ordinal);
    }

    [Fact]
    public void MainWindowXaml_RemovesAdminEnabledOnlyFilterAndTaskTableTitle()
    {
        string xaml = File.ReadAllText(ResolveRepoPath("BatCave", "MainWindow.xaml"));

        Assert.DoesNotContain("Admin Enabled Only Filter", xaml, StringComparison.Ordinal);
        Assert.DoesNotContain("Admin-enabled only", xaml, StringComparison.Ordinal);
        Assert.DoesNotContain("Task Manager Table", xaml, StringComparison.Ordinal);
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
