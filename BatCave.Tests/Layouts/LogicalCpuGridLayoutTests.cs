using BatCave.Layouts;

namespace BatCave.Tests.Layouts;

public sealed class LogicalCpuGridLayoutTests
{
    [Fact]
    public void Resolve_SingleTile_FillsAvailableSpaceWithinMinimums()
    {
        LogicalCpuGridLayoutResult plan = LogicalCpuGridLayout.Resolve(
            itemCount: 1,
            availableWidth: 320d,
            availableHeight: 200d);

        Assert.Equal(1, plan.Columns);
        Assert.True(plan.ItemWidth >= LogicalCpuGridLayout.TileMinWidth);
        Assert.True(plan.ItemHeight >= LogicalCpuGridLayout.TileMinHeight);
    }

    [Fact]
    public void Resolve_ManyTiles_PrefersMultipleColumns()
    {
        LogicalCpuGridLayoutResult plan = LogicalCpuGridLayout.Resolve(
            itemCount: 16,
            availableWidth: 720d,
            availableHeight: 420d);

        Assert.True(plan.Columns > 1);
        Assert.True(plan.ItemWidth >= LogicalCpuGridLayout.TileMinWidth);
        Assert.True(plan.ItemHeight >= LogicalCpuGridLayout.TileMinHeight);
    }
}
