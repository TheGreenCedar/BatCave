using BatCave.Charts;

namespace BatCave.Tests;

public sealed class SparklineMathTests
{
    [Fact]
    public void BuildPoints_EmptySeries_ReturnsEmpty()
    {
        IReadOnlyList<Windows.Foundation.Point> points = SparklineMath.BuildPoints([], 100, 20);
        Assert.Empty(points);
    }

    [Fact]
    public void BuildPointString_EmptySeries_UsesFallback()
    {
        string points = SparklineMath.BuildPointString([], 100, 20);
        Assert.Equal(SparklineMath.FlatlineFallbackPoints, points);
    }

    [Fact]
    public void BuildPoints_SingleValue_DuplicatesPoint()
    {
        IReadOnlyList<Windows.Foundation.Point> points = SparklineMath.BuildPoints([42], 100, 20);
        Assert.Equal(2, points.Count);
        Assert.Equal(points[0].Y, points[1].Y);
        Assert.Equal(0, points[0].X);
        Assert.Equal(100, points[1].X);
    }

    [Fact]
    public void BuildPoints_ConstantSeries_CentersLine()
    {
        IReadOnlyList<Windows.Foundation.Point> points = SparklineMath.BuildPoints([7, 7, 7], 100, 20);
        Assert.All(points, point => Assert.Equal(10, point.Y));
    }

    [Fact]
    public void BuildPoints_VarianceSeries_ScalesWithinBounds()
    {
        IReadOnlyList<Windows.Foundation.Point> points = SparklineMath.BuildPoints([0, 50, 100], 100, 22);
        Assert.Equal(3, points.Count);
        Assert.Equal(new Windows.Foundation.Point(0, 21), points[0]);
        Assert.Equal(new Windows.Foundation.Point(50, 11), points[1]);
        Assert.Equal(new Windows.Foundation.Point(100, 1), points[2]);
    }
}
