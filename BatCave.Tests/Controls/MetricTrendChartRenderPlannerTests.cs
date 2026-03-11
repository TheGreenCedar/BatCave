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
    public void CreatePlan_UsesFixedVisibleWindowAndAlignedOverlayPoints()
    {
        double[] values = Enumerable.Range(1, 100).Select(static value => (double)value).ToArray();
        double[] overlayValues = Enumerable.Range(1, 40).Select(static value => (double)value).ToArray();

        MetricTrendChartRenderPlan plan = MetricTrendChartRenderPlanner.CreatePlan(new MetricTrendChartRenderRequest(
            values,
            overlayValues,
            120,
            MetricTrendScaleMode.CpuPercent,
            double.NaN,
            0d));

        Assert.Equal(120, plan.SlotCount);
        Assert.Equal(100, plan.LineSeries.Values.Count);
        Assert.Equal(40, plan.OverlaySeries.Values.Count);
        Assert.Equal(20, plan.LineSeries.LeadingSlots);
        Assert.Equal(80, plan.OverlaySeries.LeadingSlots);
        Assert.False(plan.DomainFallbackUsed);
        Assert.True(plan.DomainMax >= plan.MaxVisible);
    }

    [Fact]
    public void CreatePlan_PartialHistory_RetainsStableSlotCountAcrossTicks()
    {
        double[] initialValues = Enumerable.Range(1, 58).Select(static value => (double)value).ToArray();
        double[] nextValues = Enumerable.Range(1, 59).Select(static value => (double)value).ToArray();

        MetricTrendChartRenderPlan initialPlan = MetricTrendChartRenderPlanner.CreatePlan(new MetricTrendChartRenderRequest(
            initialValues,
            Array.Empty<double>(),
            60,
            MetricTrendScaleMode.CpuPercent,
            double.NaN,
            0d));
        MetricTrendChartRenderPlan nextPlan = MetricTrendChartRenderPlanner.CreatePlan(new MetricTrendChartRenderRequest(
            nextValues,
            Array.Empty<double>(),
            60,
            MetricTrendScaleMode.CpuPercent,
            double.NaN,
            0d));

        Assert.Equal(60, initialPlan.SlotCount);
        Assert.Equal(60, nextPlan.SlotCount);
        Assert.Equal(2, initialPlan.LineSeries.LeadingSlots);
        Assert.Equal(1, nextPlan.LineSeries.LeadingSlots);
    }

    [Fact]
    public void CreatePlan_NonFiniteDomain_FallsBackToScaleFloor()
    {
        double[] values = [double.PositiveInfinity, double.NaN];

        MetricTrendChartRenderPlan plan = MetricTrendChartRenderPlanner.CreatePlan(new MetricTrendChartRenderRequest(
            values,
            Array.Empty<double>(),
            60,
            MetricTrendScaleMode.CpuPercent,
            double.PositiveInfinity,
            double.NaN));

        Assert.True(plan.NonFiniteSeriesDetected);
        Assert.True(plan.DomainFallbackUsed);
        Assert.Equal(MetricTrendScaleDomain.CpuFloorPercent, plan.DomainMax);
    }

    [Fact]
    public void CreatePlan_EmptySeries_UsesFallbackFlagsWithoutInventingSlots()
    {
        MetricTrendChartRenderPlan plan = MetricTrendChartRenderPlanner.CreatePlan(new MetricTrendChartRenderRequest(
            Array.Empty<double>(),
            Array.Empty<double>(),
            60,
            MetricTrendScaleMode.CpuPercent,
            double.NaN,
            0d));

        Assert.Equal(0, plan.SlotCount);
        Assert.True(plan.LineFallbackUsed);
        Assert.True(plan.OverlayFallbackUsed);
        Assert.Empty(plan.LineSeries.Values);
        Assert.Empty(plan.OverlaySeries.Values);
    }
}
