using BatCave.Controls;
using System;
using Windows.Foundation;

namespace BatCave.Charts;

public static class MetricTrendTransitionMath
{
    public const int DefaultDurationMs = 120;
    public const int MinDurationMs = 60;
    public const int MaxDurationMs = 600;

    public static int NormalizeDurationMs(int candidate)
    {
        return Math.Clamp(candidate, MinDurationMs, MaxDurationMs);
    }

    public static double ComputeProgress(TimeSpan elapsed, int durationMs)
    {
        int normalizedDuration = NormalizeDurationMs(durationMs);
        if (elapsed <= TimeSpan.Zero)
        {
            return 0d;
        }

        double progress = elapsed.TotalMilliseconds / normalizedDuration;
        return Math.Clamp(progress, 0d, 1d);
    }

    public static double EaseOutCubic(double progress)
    {
        double clamped = Math.Clamp(progress, 0d, 1d);
        double remaining = 1d - clamped;
        return 1d - remaining * remaining * remaining;
    }

    public static Point InterpolatePoint(Point start, Point target, double easedProgress)
    {
        double clamped = Math.Clamp(easedProgress, 0d, 1d);
        double x = start.X + (target.X - start.X) * clamped;
        double y = start.Y + (target.Y - start.Y) * clamped;
        return new Point(Math.Round(x, 2), Math.Round(y, 2));
    }

    public static bool CanAnimateTransition(
        bool enableTransitions,
        bool hasPreviousFrame,
        in MetricTrendTransitionSnapshot previous,
        in MetricTrendTransitionSnapshot next)
    {
        if (!enableTransitions
            || !hasPreviousFrame
            || next.FallbackUsed
            || next.LinePointCount <= 0)
        {
            return false;
        }

        if (previous.LinePointCount <= 0
            || previous.LinePointCount != next.LinePointCount
            || previous.OverlayPointCount != next.OverlayPointCount)
        {
            return false;
        }

        if (previous.VisiblePointCount != next.VisiblePointCount
            || previous.ScaleMode != next.ScaleMode
            || !AreDomainOverridesEquivalent(previous.DomainMaxOverride, next.DomainMaxOverride))
        {
            return false;
        }

        return AreSizesEquivalent(previous.Width, next.Width)
               && AreSizesEquivalent(previous.Height, next.Height);
    }

    private static bool AreDomainOverridesEquivalent(double left, double right)
    {
        return left.Equals(right)
               || (double.IsNaN(left) && double.IsNaN(right));
    }

    private static bool AreSizesEquivalent(double left, double right)
    {
        if (!double.IsFinite(left) || !double.IsFinite(right))
        {
            return false;
        }

        return Math.Abs(left - right) < 0.01d;
    }
}

public readonly record struct MetricTrendTransitionSnapshot(
    int VisiblePointCount,
    MetricTrendScaleMode ScaleMode,
    double DomainMaxOverride,
    double Width,
    double Height,
    int LinePointCount,
    int OverlayPointCount,
    bool FallbackUsed);
