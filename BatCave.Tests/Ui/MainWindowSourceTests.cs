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
        Assert.Contains("appWindow.SetIcon(iconPath);", source, StringComparison.Ordinal);
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
    public void MainWindowSource_RebuildsHeaderDecorationsWithoutSettingCanvasDimensions()
    {
        string source = File.ReadAllText(ResolveRepoPath("BatCave", "MainWindow.xaml.cs"));

        Assert.Contains("HeaderDecorationCanvas.Clip = new RectangleGeometry", source, StringComparison.Ordinal);
        Assert.DoesNotContain("HeaderDecorationCanvas.Width =", source, StringComparison.Ordinal);
        Assert.DoesNotContain("HeaderDecorationCanvas.Height =", source, StringComparison.Ordinal);
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
