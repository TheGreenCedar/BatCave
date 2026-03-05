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

        Assert.Contains("AutomationProperties.Name=\"Expanded metric trend chart\"", xaml, StringComparison.Ordinal);
        Assert.Contains("AutomationProperties.Name=\"Global primary trend chart\"", xaml, StringComparison.Ordinal);
        Assert.Contains("AutomationProperties.Name=\"Logical CPU trend chart\"", xaml, StringComparison.Ordinal);
    }

    [Fact]
    public void MainWindowXaml_DoesNotUseHardcodedCpuTrendHexColors()
    {
        string xaml = File.ReadAllText(ResolveRepoPath("BatCave", "MainWindow.xaml"));

        Assert.DoesNotMatch(new Regex("#FF0B84D8|#330B84D8|#FF075C8F", RegexOptions.IgnoreCase), xaml);
        Assert.Contains("ThemeResource ChartCpuStrokeBrush", xaml, StringComparison.Ordinal);
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
