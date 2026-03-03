using System;
using System.Collections.Generic;
using System.Globalization;
using System.Text;
using Microsoft.UI.Xaml.Media;
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
        for (int index = 0; index < points.Count; index++)
        {
            collection.Add(points[index]);
        }

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
        if (values.Count == 0 || width <= 0 || height <= 0)
        {
            layout = default;
            return false;
        }

        double max = values[0];
        double min = values[0];
        for (int index = 1; index < values.Count; index++)
        {
            double value = values[index];
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
            SingleValue: values[0]);
        return true;
    }

    private static Point ResolvePoint(
        IReadOnlyList<double> values,
        double width,
        double height,
        in SparklineLayout layout,
        int index)
    {
        double value = layout.SingleValueSeries ? layout.SingleValue : values[index];
        double x = (index * width) / layout.XDenominator;
        double y = layout.HasVariance
            ? layout.VerticalPadding + (1d - (value - layout.Min) / (layout.Max - layout.Min)) * layout.DrawableHeight
            : height / 2d;

        return new Point(Math.Round(x, 2), Math.Round(y, 2));
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
