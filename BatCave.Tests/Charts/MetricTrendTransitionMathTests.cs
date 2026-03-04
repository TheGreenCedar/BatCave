using BatCave.Charts;
using BatCave.Controls;
using System;
using Windows.Foundation;

namespace BatCave.Tests.Charts;

public sealed class MetricTrendTransitionMathTests
{
    [Fact]
    public void ComputeProgress_IsMonotonicAcrossElapsedTime()
    {
        double p0 = MetricTrendTransitionMath.ComputeProgress(TimeSpan.Zero, 120);
        double p1 = MetricTrendTransitionMath.ComputeProgress(TimeSpan.FromMilliseconds(30), 120);
        double p2 = MetricTrendTransitionMath.ComputeProgress(TimeSpan.FromMilliseconds(60), 120);
        double p3 = MetricTrendTransitionMath.ComputeProgress(TimeSpan.FromMilliseconds(120), 120);

        Assert.True(p0 <= p1);
        Assert.True(p1 <= p2);
        Assert.True(p2 <= p3);
    }

    [Theory]
    [InlineData(-15, 0d)]
    [InlineData(0, 0d)]
    [InlineData(120, 1d)]
    [InlineData(240, 1d)]
    public void ComputeProgress_ClampsToUnitInterval(double elapsedMs, double expected)
    {
        double actual = MetricTrendTransitionMath.ComputeProgress(TimeSpan.FromMilliseconds(elapsedMs), 120);
        Assert.Equal(expected, actual, 6);
    }

    [Fact]
    public void InterpolatePoint_EndsAtTargetWhenComplete()
    {
        Point start = new(0d, 10d);
        Point target = new(12d, 4d);

        Point actual = MetricTrendTransitionMath.InterpolatePoint(start, target, 1d);

        Assert.Equal(target, actual);
    }

    [Fact]
    public void Retargeting_FromCurrentInterpolatedPoint_HasNoDiscontinuity()
    {
        Point initialStart = new(0d, 0d);
        Point initialTarget = new(10d, 10d);
        double easedHalf = MetricTrendTransitionMath.EaseOutCubic(0.5d);
        Point current = MetricTrendTransitionMath.InterpolatePoint(initialStart, initialTarget, easedHalf);

        Point retargeted = MetricTrendTransitionMath.InterpolatePoint(current, new Point(20d, 20d), 0d);

        Assert.Equal(current, retargeted);
    }

    [Fact]
    public void CanAnimateTransition_TrueWhenCompatible()
    {
        MetricTrendTransitionSnapshot previous = new(
            VisiblePointCount: 60,
            ScaleMode: MetricTrendScaleMode.CpuPercent,
            DomainMaxOverride: double.NaN,
            Width: 120d,
            Height: 36d,
            LinePointCount: 60,
            OverlayPointCount: 60,
            FallbackUsed: false);
        MetricTrendTransitionSnapshot next = previous with { FallbackUsed = false };

        bool canAnimate = MetricTrendTransitionMath.CanAnimateTransition(
            enableTransitions: true,
            hasPreviousFrame: true,
            previous,
            next);

        Assert.True(canAnimate);
    }

    [Theory]
    [InlineData(false, true, 60, 60, MetricTrendScaleMode.CpuPercent, MetricTrendScaleMode.CpuPercent, 120, 120, 36, 36, false)]
    [InlineData(true, false, 60, 60, MetricTrendScaleMode.CpuPercent, MetricTrendScaleMode.CpuPercent, 120, 120, 36, 36, false)]
    [InlineData(true, true, 60, 120, MetricTrendScaleMode.CpuPercent, MetricTrendScaleMode.CpuPercent, 120, 120, 36, 36, false)]
    [InlineData(true, true, 60, 60, MetricTrendScaleMode.CpuPercent, MetricTrendScaleMode.MemoryBytes, 120, 120, 36, 36, false)]
    [InlineData(true, true, 60, 60, MetricTrendScaleMode.CpuPercent, MetricTrendScaleMode.CpuPercent, 120, 128, 36, 36, false)]
    [InlineData(true, true, 60, 60, MetricTrendScaleMode.CpuPercent, MetricTrendScaleMode.CpuPercent, 120, 120, 36, 40, false)]
    [InlineData(true, true, 60, 60, MetricTrendScaleMode.CpuPercent, MetricTrendScaleMode.CpuPercent, 120, 120, 36, 36, true)]
    public void CanAnimateTransition_ReturnsFalseForGuards(
        bool enableTransitions,
        bool hasPreviousFrame,
        int previousVisibleCount,
        int nextVisibleCount,
        MetricTrendScaleMode previousScaleMode,
        MetricTrendScaleMode nextScaleMode,
        double previousWidth,
        double nextWidth,
        double previousHeight,
        double nextHeight,
        bool nextFallbackUsed)
    {
        MetricTrendTransitionSnapshot previous = new(
            VisiblePointCount: previousVisibleCount,
            ScaleMode: previousScaleMode,
            DomainMaxOverride: double.NaN,
            Width: previousWidth,
            Height: previousHeight,
            LinePointCount: 60,
            OverlayPointCount: 60,
            FallbackUsed: false);
        MetricTrendTransitionSnapshot next = new(
            VisiblePointCount: nextVisibleCount,
            ScaleMode: nextScaleMode,
            DomainMaxOverride: double.NaN,
            Width: nextWidth,
            Height: nextHeight,
            LinePointCount: 60,
            OverlayPointCount: 60,
            FallbackUsed: nextFallbackUsed);

        bool canAnimate = MetricTrendTransitionMath.CanAnimateTransition(
            enableTransitions,
            hasPreviousFrame,
            previous,
            next);

        Assert.False(canAnimate);
    }
}
