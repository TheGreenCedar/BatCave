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
    private const double TilePreferredChartHeightRatio = 0.42d;
    private const double TileExpandedChartHeightRatio = 0.60d;
    private const double PixelComparisonTolerance = 0.25d;
    private const double AspectComparisonTolerance = 0.0001d;

    public static LogicalCpuGridLayoutResult Resolve(int itemCount, double availableWidth, double availableHeight)
    {
        int safeItemCount = Math.Max(1, itemCount);
        double safeWidth = Math.Max(TileMinWidth, availableWidth);
        if (!double.IsFinite(availableHeight) || availableHeight <= 0d)
        {
            return ResolveWidthPreferred(safeItemCount, safeWidth);
        }

        int maxColumns = GetMaxFeasibleColumns(safeItemCount, safeWidth);
        LogicalCpuGridLayoutCandidate bestCandidate = default;
        bool hasCandidate = false;

        for (int columns = 1; columns <= maxColumns; columns++)
        {
            LogicalCpuGridLayoutCandidate candidate = CreateFiniteCandidate(
                safeItemCount,
                safeWidth,
                availableHeight,
                columns);

            if (!hasCandidate || candidate.IsBetterThan(bestCandidate))
            {
                bestCandidate = candidate;
                hasCandidate = true;
            }
        }

        return hasCandidate
            ? bestCandidate.ToResult()
            : ResolveWidthPreferred(safeItemCount, safeWidth);
    }

    private static LogicalCpuGridLayoutResult ResolveWidthPreferred(int itemCount, double availableWidth)
    {
        int columns = GetTargetColumnCount(itemCount, availableWidth);
        double itemWidth = GetItemWidth(availableWidth, columns);
        double chartHeight = Math.Max(
            TileMinChartHeight,
            Math.Max(TileTargetChartHeight, itemWidth * TilePreferredChartHeightRatio));
        double itemHeight = Math.Min(TileMaxHeight, Math.Max(TileMinHeight, TileLabelReserve + chartHeight));

        return new LogicalCpuGridLayoutResult(
            columns,
            itemWidth,
            itemHeight,
            Math.Max(TileMinChartHeight, itemHeight - TileLabelReserve));
    }

    private static LogicalCpuGridLayoutCandidate CreateFiniteCandidate(
        int itemCount,
        double availableWidth,
        double availableHeight,
        int columns)
    {
        int rows = (itemCount + columns - 1) / columns;
        double itemWidth = GetItemWidth(availableWidth, columns);
        double verticalMargins = rows * TileItemMargin * 2d;
        double heightBudgetPerItem = (availableHeight - verticalMargins) / rows;
        bool fitsWithoutOverflow = heightBudgetPerItem >= TileMinHeight;
        double itemHeight;

        if (fitsWithoutOverflow)
        {
            double expandedChartHeight = Math.Max(
                TileTargetChartHeight,
                itemWidth * TileExpandedChartHeightRatio);
            itemHeight = Math.Min(heightBudgetPerItem, TileLabelReserve + expandedChartHeight);
        }
        else
        {
            itemHeight = TileMinHeight;
        }

        itemHeight = Math.Max(TileMinHeight, itemHeight);
        double chartHeight = Math.Max(TileMinChartHeight, itemHeight - TileLabelReserve);
        double totalHeight = (rows * itemHeight) + verticalMargins;
        double horizontalMargins = columns * TileItemMargin * 2d;
        double totalWidth = (columns * itemWidth) + horizontalMargins;
        double overflowHeight = Math.Max(0d, totalHeight - availableHeight);
        double unusedHeight = fitsWithoutOverflow ? Math.Max(0d, availableHeight - totalHeight) : 0d;
        double hostAspectRatio = availableWidth / availableHeight;
        double gridAspectRatio = totalWidth / totalHeight;
        double aspectDelta = Math.Abs(Math.Log(gridAspectRatio / hostAspectRatio));

        return new LogicalCpuGridLayoutCandidate(
            columns,
            rows,
            itemWidth,
            itemHeight,
            chartHeight,
            overflowHeight,
            unusedHeight,
            aspectDelta);
    }

    private static int GetTargetColumnCount(int itemCount, double availableWidth)
    {
        double horizontalUnit = TileTargetWidth + (TileItemMargin * 2d);
        int maxFeasibleColumns = GetMaxFeasibleColumns(itemCount, availableWidth);
        int targetColumns = (int)Math.Floor((availableWidth + (TileItemMargin * 2d)) / horizontalUnit);
        return Math.Clamp(targetColumns, 1, maxFeasibleColumns);
    }

    private static int GetMaxFeasibleColumns(int itemCount, double availableWidth)
    {
        double minColumnUnit = TileMinWidth + (TileItemMargin * 2d);
        int maxColumnsByWidth = Math.Max(1, (int)Math.Floor(availableWidth / minColumnUnit));
        return Math.Clamp(maxColumnsByWidth, 1, itemCount);
    }

    private static double GetItemWidth(double availableWidth, int columns)
    {
        double horizontalMargins = columns * TileItemMargin * 2d;
        return Math.Max(TileMinWidth, (availableWidth - horizontalMargins) / columns);
    }

    private readonly record struct LogicalCpuGridLayoutCandidate(
        int Columns,
        int Rows,
        double ItemWidth,
        double ItemHeight,
        double ChartHeight,
        double OverflowHeight,
        double UnusedHeight,
        double AspectDelta)
    {
        public bool IsBetterThan(LogicalCpuGridLayoutCandidate other)
        {
            bool fitsWithoutOverflow = OverflowHeight <= PixelComparisonTolerance;
            bool otherFitsWithoutOverflow = other.OverflowHeight <= PixelComparisonTolerance;
            if (fitsWithoutOverflow != otherFitsWithoutOverflow)
            {
                return fitsWithoutOverflow;
            }

            if (!fitsWithoutOverflow &&
                !NearlyEqual(OverflowHeight, other.OverflowHeight, PixelComparisonTolerance))
            {
                return OverflowHeight < other.OverflowHeight;
            }

            if (!NearlyEqual(ChartHeight, other.ChartHeight, PixelComparisonTolerance))
            {
                return ChartHeight > other.ChartHeight;
            }

            if (!NearlyEqual(AspectDelta, other.AspectDelta, AspectComparisonTolerance))
            {
                return AspectDelta < other.AspectDelta;
            }

            if (!NearlyEqual(UnusedHeight, other.UnusedHeight, PixelComparisonTolerance))
            {
                return UnusedHeight < other.UnusedHeight;
            }

            if (Columns != other.Columns)
            {
                return Columns > other.Columns;
            }

            if (!NearlyEqual(ItemWidth, other.ItemWidth, PixelComparisonTolerance))
            {
                return ItemWidth > other.ItemWidth;
            }

            return Rows < other.Rows;
        }

        public LogicalCpuGridLayoutResult ToResult() =>
            new(
                Columns,
                ItemWidth,
                ItemHeight,
                ChartHeight);

        private static bool NearlyEqual(double left, double right, double tolerance)
        {
            return Math.Abs(left - right) <= tolerance;
        }
    }
}

public readonly record struct LogicalCpuGridLayoutResult(
    int Columns,
    double ItemWidth,
    double ItemHeight,
    double ChartHeight);
