namespace BatCave.Tests.Ui;

public sealed class MetricTrendChartXamlTests
{
    [Fact]
    public void MetricTrendChartXaml_UsesLiveChartsPlotSurfaceBehindFacadeContract()
    {
        string xaml = File.ReadAllText(ResolveRepoPath("BatCave", "Controls", "MetricTrendChart.xaml"));

        Assert.Contains("xmlns:lvc=\"using:LiveChartsCore.SkiaSharpView.WinUI\"", xaml, StringComparison.Ordinal);
        Assert.Contains("x:Name=\"TrendChart\"", xaml, StringComparison.Ordinal);
        Assert.Contains("x:Name=\"TransitionChart\"", xaml, StringComparison.Ordinal);
        Assert.Contains("<lvc:CartesianChart", xaml, StringComparison.Ordinal);
        Assert.Contains("IsHitTestVisible=\"False\"", xaml, StringComparison.Ordinal);
        Assert.Contains("Opacity=\"0\"", xaml, StringComparison.Ordinal);
        Assert.Contains("Visibility=\"Collapsed\"", xaml, StringComparison.Ordinal);
        Assert.Contains("x:Name=\"PlotBorder\"", xaml, StringComparison.Ordinal);
        Assert.Contains("x:Name=\"GridPath\"", xaml, StringComparison.Ordinal);
        Assert.Contains("x:Name=\"TopRightScaleLabel\"", xaml, StringComparison.Ordinal);
        Assert.Contains("x:Name=\"TimeWindowLabel\"", xaml, StringComparison.Ordinal);
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
