using BatCave.Charts;
using BatCave.Controls;

namespace BatCave.Tests.Controls;

public sealed class MetricTrendChartRenderPlannerTests
{
    [Theory]
    [InlineData(1, 60)]
    [InlineData(60, 60)]
    [InlineData(90, 60)]
    [InlineData(120, 120)]
    [InlineData(140, 120)]
    public void NormalizeVisiblePointCount_ClampsToSupportedWindows(int candidate, int expected)
    {
        int actual = MetricTrendChartRenderPlanner.NormalizeVisiblePointCount(candidate);

        Assert.Equal(expected, actual);
    }

    [Fact]
    public void CreatePlan_UsesTrailingWindowsAndAlignedOverlayPoints()
    {
        double[] values = Enumerable.Range(1, 100).Select(static value => (double)value).ToArray();
        double[] overlayValues = Enumerable.Range(1, 40).Select(static value => (double)value).ToArray();

        MetricTrendChartRenderPlan plan = MetricTrendChartRenderPlanner.CreatePlan(new MetricTrendChartRenderRequest(
            values,
            overlayValues,
            120,
            320d,
            160d,
            MetricTrendScaleMode.CpuPercent,
            double.NaN,
            0d));

        Assert.Equal(100, plan.LinePoints.Count);
        Assert.Equal(40, plan.OverlayPoints.Count);
        Assert.False(plan.DomainFallbackUsed);
        Assert.True(plan.DomainMax >= plan.MaxVisible);
    }

    [Fact]
    public void CreatePlan_NonFiniteDomain_FallsBackToScaleFloor()
    {
        double[] values = [double.PositiveInfinity, double.NaN];

        MetricTrendChartRenderPlan plan = MetricTrendChartRenderPlanner.CreatePlan(new MetricTrendChartRenderRequest(
            values,
            Array.Empty<double>(),
            60,
            320d,
            160d,
            MetricTrendScaleMode.CpuPercent,
            double.PositiveInfinity,
            double.NaN));

        Assert.True(plan.NonFiniteSeriesDetected);
        Assert.True(plan.DomainFallbackUsed);
        Assert.Equal(MetricTrendScaleDomain.CpuFloorPercent, plan.DomainMax);
    }
}
