using BatCave.Controls;
using System;
using Windows.Foundation;

namespace BatCave.Charts;

public enum MetricTrendTransitionMode
{
    Instant,
    Interpolate,
    SlideLeft,
}

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
        return ResolveTransitionMode(enableTransitions, hasPreviousFrame, previous, next) != MetricTrendTransitionMode.Instant;
    }

    public static MetricTrendTransitionMode ResolveTransitionMode(
        bool enableTransitions,
        bool hasPreviousFrame,
        in MetricTrendTransitionSnapshot previous,
        in MetricTrendTransitionSnapshot next)
    {
        if (!enableTransitions
            || !hasPreviousFrame
            || previous.LinePointCount <= 0
            || next.LinePointCount <= 0)
        {
            return MetricTrendTransitionMode.Instant;
        }

        if (previous.LinePointCount != next.LinePointCount
            || previous.OverlayPointCount != next.OverlayPointCount)
        {
            return MetricTrendTransitionMode.Instant;
        }

        if (previous.VisiblePointCount != next.VisiblePointCount
            || previous.ScaleMode != next.ScaleMode)
        {
            return MetricTrendTransitionMode.Instant;
        }

        if (!AreSizesEquivalent(previous.Width, next.Width)
            || !AreSizesEquivalent(previous.Height, next.Height))
        {
            return MetricTrendTransitionMode.Instant;
        }

        if (next.LinePointCount >= 2)
        {
            return MetricTrendTransitionMode.SlideLeft;
        }

        return MetricTrendTransitionMode.Interpolate;
    }

    public static double ComputeSlideOffset(double slotWidth, double easedProgress)
    {
        if (!double.IsFinite(slotWidth) || slotWidth <= 0d)
        {
            return 0d;
        }

        double clamped = Math.Clamp(easedProgress, 0d, 1d);
        double offset = slotWidth * (1d - clamped);
        return Math.Round(Math.Max(0d, offset), 2);
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
