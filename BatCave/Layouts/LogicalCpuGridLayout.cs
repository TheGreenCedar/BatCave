using System;

namespace BatCave.Layouts;

public static class LogicalCpuGridLayout
{
    public const double TileTargetWidth = 170d;
    public const double TileTargetChartHeight = 56d;
    public const double TileMinChartHeight = 24d;
    public const double TileLabelReserve = 48d;
    public const double TileMinWidth = 56d;
    public const double TileMinHeight = TileLabelReserve + TileMinChartHeight;
    public const double TileMaxHeight = 148d;
    public const double TileItemMargin = 2d;

    public static LogicalCpuGridLayoutResult Resolve(int itemCount, double availableWidth, double availableHeight)
    {
        int safeItemCount = Math.Max(1, itemCount);
        double safeWidth = Math.Max(TileMinWidth, availableWidth);
        double horizontalUnit = TileTargetWidth + (TileItemMargin * 2d);
        int bestColumns = Math.Clamp((int)Math.Floor((safeWidth + (TileItemMargin * 2d)) / horizontalUnit), 1, safeItemCount);
        int rows = (safeItemCount + bestColumns - 1) / bestColumns;
        double horizontalMargins = bestColumns * TileItemMargin * 2d;
        double itemWidth = Math.Max(TileMinWidth, (safeWidth - horizontalMargins) / bestColumns);
        double chartHeight = Math.Max(TileMinChartHeight, Math.Min(TileTargetChartHeight, itemWidth * 0.42d));
        double itemHeight = Math.Max(TileMinHeight, TileLabelReserve + chartHeight);

        if (double.IsFinite(availableHeight) && availableHeight > 0d)
        {
            double verticalMargins = rows * TileItemMargin * 2d;
            double maxHeightPerItem = (availableHeight - verticalMargins) / rows;
            if (maxHeightPerItem > TileMinHeight)
            {
                // Keep logical tiles stable on tall panes and only shrink them when
                // the window becomes too short to fit the current row count.
                itemHeight = Math.Min(itemHeight, Math.Min(maxHeightPerItem, TileMaxHeight));
            }
            else
            {
                // Once the viewport is too short for natural tiles, clamp to minimum
                // and let the host scroller handle remaining overflow.
                itemHeight = TileMinHeight;
            }
        }

        chartHeight = Math.Max(TileMinChartHeight, itemHeight - TileLabelReserve);

        return new LogicalCpuGridLayoutResult(
            bestColumns,
            Math.Max(TileMinWidth, itemWidth),
            Math.Max(TileMinHeight, itemHeight),
            chartHeight);
    }
}

public readonly record struct LogicalCpuGridLayoutResult(
    int Columns,
    double ItemWidth,
    double ItemHeight,
    double ChartHeight);
