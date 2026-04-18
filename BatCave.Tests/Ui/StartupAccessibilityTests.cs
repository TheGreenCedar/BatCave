using System.Linq;
using System.Text.RegularExpressions;

namespace BatCave.Tests.Ui;

public sealed class StartupAccessibilityTests
{
    [Fact]
    public void StartupStatePanelXaml_AnnouncesBlockedAndErrorStates()
    {
        string xaml = File.ReadAllText(ResolveRepoPath("BatCave", "Controls", "StartupStatePanel.xaml"));

        Assert.Equal(2, Regex.Matches(xaml, "AutomationProperties.LiveSetting=\"Assertive\"", RegexOptions.CultureInvariant).Count);
        Assert.Contains("AutomationProperties.Name=\"Startup blocked state\"", xaml, StringComparison.Ordinal);
        Assert.Contains("AutomationProperties.Name=\"Startup error state\"", xaml, StringComparison.Ordinal);
        Assert.Contains("AutomationProperties.Name=\"Retry Startup\"", xaml, StringComparison.Ordinal);
    }

    [Fact]
    public void StartupStatePanelCodeBehind_FocusesRetryWhenStartupErrorAppears()
    {
        string source = File.ReadAllText(ResolveRepoPath("BatCave", "Controls", "StartupStatePanel.xaml.cs"));

        Assert.Contains("RetryBootstrapButton.Focus(FocusState.Programmatic);", source, StringComparison.Ordinal);
        Assert.Contains("nameof(MonitoringShellViewModel.IsStartupError)", source, StringComparison.Ordinal);
    }

    [Fact]
    public void RuntimeStatusFooterXaml_UsesSingleLiveRegionForMeaningfulRuntimeAnnouncements()
    {
        string xaml = File.ReadAllText(ResolveRepoPath("BatCave", "Controls", "RuntimeStatusFooter.xaml"));

        Assert.Single(Regex.Matches(xaml, "AutomationProperties.LiveSetting=\"Polite\"", RegexOptions.CultureInvariant).Cast<Match>());
        Assert.Contains("AutomationProperties.Name=\"Runtime Status Announcement\"", xaml, StringComparison.Ordinal);
        Assert.Contains("Text=\"{Binding RuntimeStatusSummary, Mode=OneWay}\"", xaml, StringComparison.Ordinal);
        Assert.Contains("Text=\"{Binding RuntimeHealthStatus, Mode=OneWay}\"", xaml, StringComparison.Ordinal);
        Assert.Contains("Text=\"{Binding InteractionTimingProbe, Mode=OneWay}\"", xaml, StringComparison.Ordinal);
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
