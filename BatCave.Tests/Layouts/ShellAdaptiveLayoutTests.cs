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
    public void LogicalCpuGridLayout_DoesNotExpandTileHeightBeyondItsNaturalSize()
    {
        LogicalCpuGridLayoutResult natural = LogicalCpuGridLayout.Resolve(16, 960, double.PositiveInfinity);
        LogicalCpuGridLayoutResult tall = LogicalCpuGridLayout.Resolve(16, 960, 720);

        Assert.Equal(natural.ItemHeight, tall.ItemHeight);
        Assert.Equal(tall.ItemHeight, LogicalCpuGridLayout.Resolve(16, 960, 1200).ItemHeight);
        Assert.True(tall.ItemHeight <= LogicalCpuGridLayout.TileMaxHeight);
    }
}
