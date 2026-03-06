using System;

namespace BatCave.Layouts;

public static class LogicalCpuGridLayout
{
    public const double TileTargetWidth = 170d;
    public const double TileTargetChartHeight = 56d;
    public const double TileLabelReserve = 20d;
    public const double TileMinWidth = 56d;
    public const double TileMinHeight = 28d;
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
        double chartHeight = Math.Max(TileMinHeight - TileLabelReserve, Math.Min(TileTargetChartHeight, itemWidth * 0.42d));
        double itemHeight = Math.Max(TileMinHeight, TileLabelReserve + chartHeight);

        if (double.IsFinite(availableHeight) && availableHeight > 0d)
        {
            double verticalMargins = rows * TileItemMargin * 2d;
            double maxHeightPerItem = (availableHeight - verticalMargins) / rows;
            if (maxHeightPerItem > TileMinHeight)
            {
                itemHeight = Math.Min(itemHeight, maxHeightPerItem);
            }
        }

        return new LogicalCpuGridLayoutResult(
            bestColumns,
            Math.Max(TileMinWidth, itemWidth),
            Math.Max(TileMinHeight, itemHeight));
    }
}

public readonly record struct LogicalCpuGridLayoutResult(
    int Columns,
    double ItemWidth,
    double ItemHeight);
