using BatCave.Charts;
using System.Globalization;
using Windows.Foundation;

namespace BatCave.Tests;

public sealed class SparklineMathTests
{
    [Fact]
    public void BuildPoints_EmptySeries_ReturnsEmpty()
    {
        IReadOnlyList<Point> points = SparklineMath.BuildPoints([], 100, 20);
        Assert.Empty(points);
    }

    [Fact]
    public void BuildPointsWithFallback_EmptySeries_UsesFallback()
    {
        IReadOnlyList<Point> points = SparklineMath.BuildPointsWithFallback([], 100, 20);
        Assert.Equal(2, points.Count);
        Assert.Equal(new Point(0, 0), points[0]);
        Assert.Equal(new Point(1, 0), points[1]);
    }

    [Fact]
    public void BuildPoints_SingleValue_DuplicatesPoint()
    {
        IReadOnlyList<Point> points = SparklineMath.BuildPoints([42], 100, 20);
        Assert.Equal(2, points.Count);
        Assert.Equal(points[0].Y, points[1].Y);
        Assert.Equal(0, points[0].X);
        Assert.Equal(100, points[1].X);
    }

    [Fact]
    public void BuildPoints_ConstantSeries_CentersLine()
    {
        IReadOnlyList<Point> points = SparklineMath.BuildPoints([7, 7, 7], 100, 20);
        Assert.All(points, point => Assert.Equal(10, point.Y));
    }

    [Fact]
    public void BuildPoints_VarianceSeries_ScalesWithinBounds()
    {
        IReadOnlyList<Point> points = SparklineMath.BuildPoints([0, 50, 100], 100, 22);
        Assert.Equal(3, points.Count);
        Assert.Equal(new Point(0, 21), points[0]);
        Assert.Equal(new Point(50, 11), points[1]);
        Assert.Equal(new Point(100, 1), points[2]);
    }

    [Fact]
    public void BuildPointsInDomain_ClampsOutOfRangeValuesIntoDomain()
    {
        IReadOnlyList<Point> points = SparklineMath.BuildPointsInDomain([-10, 50, 150], 100, 20, 0, 100);
        Assert.Equal(3, points.Count);
        Assert.Equal(new Point(0, 19), points[0]);
        Assert.Equal(new Point(50, 10), points[1]);
        Assert.Equal(new Point(100, 1), points[2]);
    }

    [Fact]
    public void BuildFillPolygon_ClosesAreaToBaseline()
    {
        IReadOnlyList<Point> line =
        [
            new Point(0, 19),
            new Point(50, 10),
            new Point(100, 1),
        ];

        IReadOnlyList<Point> polygon = SparklineMath.BuildFillPolygon(line, 100, 20);
        Assert.Equal(5, polygon.Count);
        Assert.Equal(new Point(0, 20), polygon[0]);
        Assert.Equal(line[0], polygon[1]);
        Assert.Equal(line[1], polygon[2]);
        Assert.Equal(line[2], polygon[3]);
        Assert.Equal(new Point(100, 20), polygon[4]);
    }

    [Theory]
    [InlineData(0.01, 0.01)]
    [InlineData(1.1, 2)]
    [InlineData(2.01, 5)]
    [InlineData(5.01, 10)]
    [InlineData(11.3, 20)]
    [InlineData(222, 500)]
    public void RoundUpToNice_UsesOneTwoFiveProgression(double value, double expected)
    {
        Assert.Equal(expected, SparklineMath.RoundUpToNice(value));
    }

    [Theory]
    [MemberData(nameof(PointFallbackEquivalenceCases))]
    public void BuildPointsWithFallback_MatchesBuildPointsOrFallback(double[] values, double width, double height)
    {
        IReadOnlyList<Point> actual = SparklineMath.BuildPointsWithFallback(values, width, height);
        IReadOnlyList<Point> points = SparklineMath.BuildPoints(values, width, height);

        if (points.Count == 0)
        {
            AssertPointsEqual(
                new Point[]
                {
                    new Point(0, 0),
                    new Point(1, 0),
                },
                actual);
            return;
        }

        AssertPointsEqual(points, actual);
    }

    [Fact]
    public void BuildPointsWithFallback_RepeatedCalls_StayDeterministicAndInvariant()
    {
        CultureInfo originalCulture = CultureInfo.CurrentCulture;
        try
        {
            CultureInfo.CurrentCulture = new CultureInfo("fr-FR");

            double[] values = [12.34, 56.78, 5.4321, 99.999];
            IReadOnlyList<Point> baseline = SparklineMath.BuildPointsWithFallback(values, 96, 22);
            for (int attempt = 0; attempt < 20; attempt++)
            {
                AssertPointsEqual(baseline, SparklineMath.BuildPointsWithFallback(values, 96, 22));
            }
        }
        finally
        {
            CultureInfo.CurrentCulture = originalCulture;
        }
    }

    public static TheoryData<double[], double, double> PointFallbackEquivalenceCases =>
        new()
        {
            { [], 96, 22 },
            { [42], 96, 22 },
            { [7, 7, 7], 96, 22 },
            { [0, 50, 100], 96, 22 },
            { [-10, 0, 10, 5.25], 120, 18 },
            { [1.001, 1.002, 1.003, 1.0015, 1.004], 80, 16 },
            { [1000, 5, 999, 7, 998, 9, 997], 64, 12 },
            { [1, 2, 3], 0, 22 },
            { [1, 2, 3], 96, -1 },
        };

    private static void AssertPointsEqual(IReadOnlyList<Point> expected, IReadOnlyList<Point> actual)
    {
        Assert.Equal(expected.Count, actual.Count);
        for (int index = 0; index < expected.Count; index++)
        {
            Assert.Equal(expected[index], actual[index]);
        }
    }
}
