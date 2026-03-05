using System;

namespace BatCave.Layouts;

public static class LogicalCpuGridLayout
{
    public const double TileTargetWidth = 170d;
    public const double TileTargetChartHeight = 120d;
    public const double TileLabelReserve = 20d;
    public const double TileMinWidth = 56d;
    public const double TileMinHeight = 28d;
    public const double TileItemMargin = 2d;

    public static LogicalCpuGridLayoutResult Resolve(int itemCount, double availableWidth, double availableHeight)
    {
        int safeItemCount = Math.Max(1, itemCount);
        double bestItemWidth = Math.Max(TileMinWidth, availableWidth);
        double bestItemHeight = Math.Max(TileMinHeight, availableHeight / safeItemCount);
        int bestColumns = 1;
        double bestScore = double.NegativeInfinity;

        for (int columns = 1; columns <= safeItemCount; columns++)
        {
            int rows = (safeItemCount + columns - 1) / columns;
            double horizontalMargins = columns * TileItemMargin * 2d;
            double verticalMargins = rows * TileItemMargin * 2d;
            double itemWidth = (availableWidth - horizontalMargins) / columns;
            double itemHeight = (availableHeight - verticalMargins) / rows;
            double chartHeight = itemHeight - TileLabelReserve;
            if (itemWidth <= 0d || chartHeight <= 0d)
            {
                continue;
            }

            double widthFit = itemWidth / TileTargetWidth;
            double heightFit = chartHeight / TileTargetChartHeight;
            double score = Math.Min(widthFit, heightFit);
            if (score > bestScore)
            {
                bestScore = score;
                bestColumns = columns;
                bestItemWidth = itemWidth;
                bestItemHeight = itemHeight;
            }
        }

        return new LogicalCpuGridLayoutResult(
            bestColumns,
            Math.Max(TileMinWidth, bestItemWidth),
            Math.Max(TileMinHeight, bestItemHeight));
    }
}

public readonly record struct LogicalCpuGridLayoutResult(
    int Columns,
    double ItemWidth,
    double ItemHeight);