using Microsoft.UI.Xaml.Media;
using System;
using System.Collections.Generic;
using System.Globalization;
using System.Text;
using Windows.Foundation;

namespace BatCave.Charts;

public static class SparklineMath
{
    public const string FlatlineFallbackPoints = "0,0 1,0";
    private static readonly IReadOnlyList<Point> FlatlineFallbackGeometry =
        new Point[]
        {
            new Point(0, 0),
            new Point(1, 0),
        };

    public static IReadOnlyList<Point> BuildPointsWithFallback(IReadOnlyList<double> values, double width, double height)
    {
        IReadOnlyList<Point> points = BuildPoints(values, width, height);
        return points.Count == 0 ? FlatlineFallbackGeometry : points;
    }

    public static PointCollection BuildPointCollectionWithFallback(IReadOnlyList<double> values, double width, double height)
    {
        return ToPointCollection(BuildPointsWithFallback(values, width, height));
    }

    public static PointCollection ToPointCollection(IReadOnlyList<Point> points)
    {
        PointCollection collection = new();
        CopyToPointCollection(points, collection);

        return collection;
    }

    public static IReadOnlyList<Point> BuildPoints(IReadOnlyList<double> values, double width, double height)
    {
        if (!TryCreateLayout(values, width, height, out SparklineLayout layout))
        {
            return [];
        }

        return BuildResolvedPoints(values, width, height, in layout);
    }

    public static IReadOnlyList<Point> BuildPointsInDomain(
        IReadOnlyList<double> values,
        double width,
        double height,
        double minDomain,
        double maxDomain)
    {
        if (values.Count == 0
            || !double.IsFinite(width)
            || !double.IsFinite(height)
            || width <= 0
            || height <= 0)
        {
            return [];
        }

        if (!double.IsFinite(minDomain) || !double.IsFinite(maxDomain))
        {
            return [];
        }

        if (maxDomain <= minDomain)
        {
            maxDomain = minDomain + 1d;
        }

        bool singleValueSeries = values.Count == 1;
        int pointCount = singleValueSeries ? 2 : values.Count;
        double verticalPadding = height > 2 ? 1d : 0d;
        double drawableHeight = Math.Max(1d, height - verticalPadding * 2d);
        double denominator = pointCount - 1d;
        double range = Math.Max(1e-9, maxDomain - minDomain);

        List<Point> points = new(pointCount);
        for (int index = 0; index < pointCount; index++)
        {
            double value = singleValueSeries ? values[0] : values[index];
            double sanitizedValue = SanitizeNumericValue(value, minDomain);
            double clamped = Math.Clamp(sanitizedValue, minDomain, maxDomain);
            double x = denominator <= 0
                ? 0d
                : (index * width) / denominator;
            double y = verticalPadding + (1d - (clamped - minDomain) / range) * drawableHeight;
            double safeX = SanitizeNumericValue(x, 0d);
            double safeY = SanitizeNumericValue(y, 0d);
            points.Add(new Point(Math.Round(safeX, 2), Math.Round(safeY, 2)));
        }

        return points;
    }

    public static IReadOnlyList<Point> BuildPointsInDomainWithFallback(
        IReadOnlyList<double> values,
        double width,
        double height,
        double minDomain,
        double maxDomain)
    {
        IReadOnlyList<Point> points = BuildPointsInDomain(values, width, height, minDomain, maxDomain);
        return points.Count == 0 ? FlatlineFallbackGeometry : points;
    }

    public static bool WritePointsInDomainWithFallback(
        IReadOnlyList<double> values,
        int startIndex,
        int count,
        double width,
        double height,
        double minDomain,
        double maxDomain,
        PointCollection destination)
    {
        ArgumentNullException.ThrowIfNull(destination);

        if (count <= 0
            || values.Count == 0
            || !double.IsFinite(width)
            || !double.IsFinite(height)
            || width <= 0
            || height <= 0
            || !double.IsFinite(minDomain)
            || !double.IsFinite(maxDomain))
        {
            WriteFlatlineFallback(destination);
            return true;
        }

        int safeStart = Math.Max(0, startIndex);
        int safeCount = Math.Min(count, values.Count - safeStart);
        if (safeCount <= 0)
        {
            WriteFlatlineFallback(destination);
            return true;
        }

        if (maxDomain <= minDomain)
        {
            maxDomain = minDomain + 1d;
        }

        bool singleValueSeries = safeCount == 1;
        int pointCount = singleValueSeries ? 2 : safeCount;
        double verticalPadding = height > 2 ? 1d : 0d;
        double drawableHeight = Math.Max(1d, height - verticalPadding * 2d);
        double denominator = pointCount - 1d;
        double range = Math.Max(1e-9, maxDomain - minDomain);

        EnsurePointCollectionSize(destination, pointCount);
        for (int index = 0; index < pointCount; index++)
        {
            int sourceIndex = singleValueSeries ? safeStart : safeStart + index;
            double value = SanitizeNumericValue(values[sourceIndex], minDomain);
            double clamped = Math.Clamp(value, minDomain, maxDomain);
            double x = denominator <= 0 ? 0d : (index * width) / denominator;
            double y = verticalPadding + (1d - (clamped - minDomain) / range) * drawableHeight;
            destination[index] = new Point(
                Math.Round(SanitizeNumericValue(x, 0d), 2),
                Math.Round(SanitizeNumericValue(y, 0d), 2));
        }

        return false;
    }

    public static bool WritePointsInDomainWithFallback(
        ReadOnlySpan<double> values,
        double width,
        double height,
        double minDomain,
        double maxDomain,
        PointCollection destination)
    {
        ArgumentNullException.ThrowIfNull(destination);

        if (values.Length <= 0
            || !double.IsFinite(width)
            || !double.IsFinite(height)
            || width <= 0
            || height <= 0
            || !double.IsFinite(minDomain)
            || !double.IsFinite(maxDomain))
        {
            WriteFlatlineFallback(destination);
            return true;
        }

        if (maxDomain <= minDomain)
        {
            maxDomain = minDomain + 1d;
        }

        bool singleValueSeries = values.Length == 1;
        int pointCount = singleValueSeries ? 2 : values.Length;
        double verticalPadding = height > 2 ? 1d : 0d;
        double drawableHeight = Math.Max(1d, height - verticalPadding * 2d);
        double denominator = pointCount - 1d;
        double range = Math.Max(1e-9, maxDomain - minDomain);

        EnsurePointCollectionSize(destination, pointCount);
        for (int index = 0; index < pointCount; index++)
        {
            double value = SanitizeNumericValue(singleValueSeries ? values[0] : values[index], minDomain);
            double clamped = Math.Clamp(value, minDomain, maxDomain);
            double x = denominator <= 0 ? 0d : (index * width) / denominator;
            double y = verticalPadding + (1d - (clamped - minDomain) / range) * drawableHeight;
            destination[index] = new Point(
                Math.Round(SanitizeNumericValue(x, 0d), 2),
                Math.Round(SanitizeNumericValue(y, 0d), 2));
        }

        return false;
    }

    public static void WriteFillPolygon(
        IList<Point> linePoints,
        double height,
        PointCollection destination)
    {
        ArgumentNullException.ThrowIfNull(destination);

        if (linePoints.Count == 0 || !double.IsFinite(height) || height <= 0)
        {
            destination.Clear();
            return;
        }

        int polygonPointCount = linePoints.Count + 2;
        double baseline = Math.Round(height, 2);
        EnsurePointCollectionSize(destination, polygonPointCount);
        destination[0] = new Point(Math.Round(linePoints[0].X, 2), baseline);
        for (int index = 0; index < linePoints.Count; index++)
        {
            destination[index + 1] = linePoints[index];
        }

        destination[^1] = new Point(Math.Round(linePoints[^1].X, 2), baseline);
    }

    public static IReadOnlyList<Point> BuildFillPolygon(
        IReadOnlyList<Point> linePoints,
        double width,
        double height)
    {
        if (linePoints.Count == 0
            || !double.IsFinite(width)
            || !double.IsFinite(height)
            || width <= 0
            || height <= 0)
        {
            return [];
        }

        double baseline = Math.Round(height, 2);
        List<Point> points = new(linePoints.Count + 2)
        {
            new Point(Math.Round(linePoints[0].X, 2), baseline),
        };

        for (int index = 0; index < linePoints.Count; index++)
        {
            points.Add(linePoints[index]);
        }

        points.Add(new Point(Math.Round(linePoints[^1].X, 2), baseline));
        return points;
    }

    public static double RoundUpToNice(double value)
    {
        if (!double.IsFinite(value) || value <= 0)
        {
            return 1d;
        }

        double exponent = Math.Floor(Math.Log10(value));
        double magnitude = Math.Pow(10d, exponent);
        double normalized = value / magnitude;

        double rounded = normalized <= 1d
            ? 1d
            : normalized <= 2d
                ? 2d
                : normalized <= 5d
                    ? 5d
                    : 10d;

        return rounded * magnitude;
    }

    public static string BuildPointString(IReadOnlyList<double> values, double width, double height)
    {
        if (!TryCreateLayout(values, width, height, out SparklineLayout layout))
        {
            return FlatlineFallbackPoints;
        }

        StringBuilder builder = new(layout.PointCount * 14);
        AppendResolvedPoints(builder, values, width, height, in layout);

        return builder.ToString();
    }

    private static IReadOnlyList<Point> BuildResolvedPoints(
        IReadOnlyList<double> values,
        double width,
        double height,
        in SparklineLayout layout)
    {
        List<Point> points = new(layout.PointCount);
        ForEachResolvedPoint(values, width, height, in layout, (_, point) => points.Add(point));
        return points;
    }

    private static void AppendResolvedPoints(
        StringBuilder builder,
        IReadOnlyList<double> values,
        double width,
        double height,
        in SparklineLayout layout)
    {
        ForEachResolvedPoint(values, width, height, in layout, (index, point) =>
        {
            if (index > 0)
            {
                builder.Append(' ');
            }

            AppendInvariantDouble(builder, point.X);
            builder.Append(',');
            AppendInvariantDouble(builder, point.Y);
        });
    }

    private static void ForEachResolvedPoint(
        IReadOnlyList<double> values,
        double width,
        double height,
        in SparklineLayout layout,
        Action<int, Point> visitor)
    {
        for (int index = 0; index < layout.PointCount; index++)
        {
            visitor(index, ResolvePoint(values, width, height, in layout, index));
        }
    }

    private static bool TryCreateLayout(IReadOnlyList<double> values, double width, double height, out SparklineLayout layout)
    {
        if (values.Count == 0
            || !double.IsFinite(width)
            || !double.IsFinite(height)
            || width <= 0
            || height <= 0)
        {
            layout = default;
            return false;
        }

        double firstValue = SanitizeNumericValue(values[0], 0d);
        double max = firstValue;
        double min = firstValue;
        for (int index = 1; index < values.Count; index++)
        {
            double value = SanitizeNumericValue(values[index], 0d);
            if (value > max)
            {
                max = value;
            }

            if (value < min)
            {
                min = value;
            }
        }

        bool singleValueSeries = values.Count == 1;
        int pointCount = singleValueSeries ? 2 : values.Count;
        double verticalPadding = height > 2 ? 1d : 0d;
        double drawableHeight = Math.Max(1d, height - verticalPadding * 2d);

        layout = new SparklineLayout(
            min,
            max,
            HasVariance: max > min,
            verticalPadding,
            drawableHeight,
            singleValueSeries,
            pointCount,
            XDenominator: pointCount - 1,
            SingleValue: firstValue);
        return true;
    }

    private static Point ResolvePoint(
        IReadOnlyList<double> values,
        double width,
        double height,
        in SparklineLayout layout,
        int index)
    {
        double value = layout.SingleValueSeries
            ? layout.SingleValue
            : SanitizeNumericValue(values[index], layout.Min);
        double x = (index * width) / layout.XDenominator;
        double y = layout.HasVariance
            ? layout.VerticalPadding + (1d - (value - layout.Min) / (layout.Max - layout.Min)) * layout.DrawableHeight
            : height / 2d;
        double safeX = SanitizeNumericValue(x, 0d);
        double safeY = SanitizeNumericValue(y, 0d);

        return new Point(Math.Round(safeX, 2), Math.Round(safeY, 2));
    }

    private static void AppendInvariantDouble(StringBuilder builder, double value)
    {
        Span<char> buffer = stackalloc char[32];
        if (value.TryFormat(buffer, out int written, "F2", CultureInfo.InvariantCulture))
        {
            builder.Append(buffer[..written]);
            return;
        }

        builder.Append(value.ToString("F2", CultureInfo.InvariantCulture));
    }

    private static double SanitizeNumericValue(double value, double fallback)
    {
        return double.IsFinite(value) ? value : fallback;
    }

    private static void CopyToPointCollection(IReadOnlyList<Point> points, PointCollection destination)
    {
        destination.Clear();
        for (int index = 0; index < points.Count; index++)
        {
            Point point = points[index];
            double safeX = SanitizeNumericValue(point.X, 0d);
            double safeY = SanitizeNumericValue(point.Y, 0d);
            destination.Add(new Point(Math.Round(safeX, 2), Math.Round(safeY, 2)));
        }
    }

    private static void EnsurePointCollectionSize(PointCollection destination, int count)
    {
        while (destination.Count > count)
        {
            destination.RemoveAt(destination.Count - 1);
        }

        while (destination.Count < count)
        {
            destination.Add(new Point(0d, 0d));
        }
    }

    private static void WriteFlatlineFallback(PointCollection destination)
    {
        EnsurePointCollectionSize(destination, 2);
        destination[0] = FlatlineFallbackGeometry[0];
        destination[1] = FlatlineFallbackGeometry[1];
    }

    private readonly record struct SparklineLayout(
        double Min,
        double Max,
        bool HasVariance,
        double VerticalPadding,
        double DrawableHeight,
        bool SingleValueSeries,
        int PointCount,
        double XDenominator,
        double SingleValue);
}
