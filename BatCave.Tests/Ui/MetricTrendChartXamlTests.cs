namespace BatCave.Tests.Ui;

public sealed class MetricTrendChartXamlTests
{
    [Fact]
    public void MetricTrendChartXaml_UsesCanvasPlotSurfaceForLayoutNeutralGeometry()
    {
        string xaml = File.ReadAllText(ResolveRepoPath("BatCave", "Controls", "MetricTrendChart.xaml"));

        Assert.Contains("x:Name=\"PlotCanvas\"", xaml, StringComparison.Ordinal);
        Assert.Contains("<Canvas", xaml, StringComparison.Ordinal);
        Assert.Contains("x:Name=\"GridPath\"", xaml, StringComparison.Ordinal);
        Assert.Contains("x:Name=\"AreaPolygon\"", xaml, StringComparison.Ordinal);
        Assert.Contains("x:Name=\"LinePolyline\"", xaml, StringComparison.Ordinal);
        Assert.Contains("x:Name=\"OverlayPolyline\"", xaml, StringComparison.Ordinal);
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
