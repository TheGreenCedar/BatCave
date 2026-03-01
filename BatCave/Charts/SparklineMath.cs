using System;
using System.Collections.Generic;
using System.Globalization;
using Windows.Foundation;

namespace BatCave.Charts;

public static class SparklineMath
{
    public const string FlatlineFallbackPoints = "0,0 1,0";

    public static IReadOnlyList<Point> BuildPoints(IReadOnlyList<double> values, double width, double height)
    {
        if (values.Count == 0 || width <= 0 || height <= 0)
        {
            return [];
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

        bool hasVariance = max > min;
        double verticalPadding = height > 2 ? 1d : 0d;
        double drawableHeight = Math.Max(1d, height - verticalPadding * 2d);
        int pointCount = values.Count == 1 ? 2 : values.Count;

        List<Point> points = new(pointCount);
        for (int index = 0; index < pointCount; index++)
        {
            double value = values.Count == 1 ? values[0] : values[index];
            double x = pointCount == 1 ? 0d : (index * width) / (pointCount - 1);
            double y = hasVariance
                ? verticalPadding + (1d - (value - min) / (max - min)) * drawableHeight
                : height / 2d;

            points.Add(new Point(Math.Round(x, 2), Math.Round(y, 2)));
        }

        return points;
    }

    public static string BuildPointString(IReadOnlyList<double> values, double width, double height)
    {
        IReadOnlyList<Point> points = BuildPoints(values, width, height);
        if (points.Count == 0)
        {
            return FlatlineFallbackPoints;
        }

        List<string> serialized = new(points.Count);
        foreach (Point point in points)
        {
            serialized.Add(
                point.X.ToString("F2", CultureInfo.InvariantCulture)
                + ","
                + point.Y.ToString("F2", CultureInfo.InvariantCulture));
        }

        return string.Join(" ", serialized);
    }
}
