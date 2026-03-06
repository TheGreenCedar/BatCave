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
        Assert.True(plan.ChartHeight >= LogicalCpuGridLayout.TileMinChartHeight);
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

    [Fact]
    public void Resolve_RoomyFiniteViewport_ExpandsBeyondNaturalSize()
    {
        LogicalCpuGridLayoutResult naturalPlan = LogicalCpuGridLayout.Resolve(
            itemCount: 16,
            availableWidth: 720d,
            availableHeight: double.PositiveInfinity);

        LogicalCpuGridLayoutResult roomyPlan = LogicalCpuGridLayout.Resolve(
            itemCount: 16,
            availableWidth: 720d,
            availableHeight: 720d);

        Assert.Equal(naturalPlan.Columns, roomyPlan.Columns);
        Assert.True(roomyPlan.ItemHeight > naturalPlan.ItemHeight);
        Assert.True(roomyPlan.ChartHeight > naturalPlan.ChartHeight);
    }

    [Fact]
    public void Resolve_WideTallDesktopLayout_UsesMoreInspectorHeight()
    {
        LogicalCpuGridLayoutResult naturalPlan = LogicalCpuGridLayout.Resolve(
            itemCount: 16,
            availableWidth: 960d,
            availableHeight: double.PositiveInfinity);

        LogicalCpuGridLayoutResult roomyPlan = LogicalCpuGridLayout.Resolve(
            itemCount: 16,
            availableWidth: 960d,
            availableHeight: 720d);

        Assert.True(roomyPlan.Columns < naturalPlan.Columns);
        Assert.True(roomyPlan.ChartHeight > naturalPlan.ChartHeight);
        Assert.True(GetUnusedVerticalSpace(16, naturalPlan, 720d) > GetUnusedVerticalSpace(16, roomyPlan, 720d));
    }

    [Fact]
    public void Resolve_LowCountWidePane_StaysBalanced()
    {
        LogicalCpuGridLayoutResult plan = LogicalCpuGridLayout.Resolve(
            itemCount: 4,
            availableWidth: 1200d,
            availableHeight: 720d);

        Assert.InRange(plan.Columns, 2, 3);
        Assert.True(GetRowCount(4, plan.Columns) > 1);
        Assert.True(plan.ChartHeight > LogicalCpuGridLayout.TileTargetChartHeight);
    }

    [Fact]
    public void Resolve_WhenPaneIsTooShort_ClampsToMinimumHeightAndFallsBackToScroll()
    {
        LogicalCpuGridLayoutResult compactPlan = LogicalCpuGridLayout.Resolve(
            itemCount: 16,
            availableWidth: 720d,
            availableHeight: 140d);

        Assert.Equal(LogicalCpuGridLayout.TileMinHeight, compactPlan.ItemHeight);
        Assert.Equal(LogicalCpuGridLayout.TileMinChartHeight, compactPlan.ChartHeight);
        Assert.True(GetUsedHeight(16, compactPlan) > 140d);
    }

    private static int GetRowCount(int itemCount, int columns) => (itemCount + columns - 1) / columns;

    private static double GetUsedHeight(int itemCount, LogicalCpuGridLayoutResult plan)
    {
        int rows = GetRowCount(itemCount, plan.Columns);
        return (rows * plan.ItemHeight) + (rows * LogicalCpuGridLayout.TileItemMargin * 2d);
    }

    private static double GetUnusedVerticalSpace(int itemCount, LogicalCpuGridLayoutResult plan, double availableHeight)
    {
        return Math.Max(0d, availableHeight - GetUsedHeight(itemCount, plan));
    }
}
