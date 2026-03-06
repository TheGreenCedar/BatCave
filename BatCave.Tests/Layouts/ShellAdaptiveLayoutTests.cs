using BatCave.Layouts;

namespace BatCave.Tests.Layouts;

public class ShellAdaptiveLayoutTests
{
    [Theory]
    [InlineData(300, ShellAdaptiveMode.Phone)]
    [InlineData(859.99, ShellAdaptiveMode.Phone)]
    [InlineData(860, ShellAdaptiveMode.Medium)]
    [InlineData(1200, ShellAdaptiveMode.Medium)]
    [InlineData(1260, ShellAdaptiveMode.Wide)]
    [InlineData(1600, ShellAdaptiveMode.Wide)]
    public void Resolve_ReturnsExpectedModeForWidth(double width, ShellAdaptiveMode expected)
    {
        ShellAdaptiveMode actual = ShellAdaptiveLayout.Resolve(width);

        Assert.Equal(expected, actual);
    }

    [Fact]
    public void LogicalCpuGridLayout_TallViewport_ExpandsBeyondNaturalSize()
    {
        LogicalCpuGridLayoutResult natural = LogicalCpuGridLayout.Resolve(16, 960d, double.PositiveInfinity);
        LogicalCpuGridLayoutResult tall = LogicalCpuGridLayout.Resolve(16, 960d, 720d);
        LogicalCpuGridLayoutResult taller = LogicalCpuGridLayout.Resolve(16, 960d, 1200d);

        Assert.True(tall.ItemHeight > natural.ItemHeight);
        Assert.True(tall.ChartHeight > natural.ChartHeight);
        Assert.True(taller.ItemHeight >= tall.ItemHeight);
    }
}
