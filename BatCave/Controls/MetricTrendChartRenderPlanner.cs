using BatCave.Charts;
using System;
using System.Collections.Generic;

namespace BatCave.Controls;

public static class MetricTrendChartRenderPlanner
{
    public const int MinVisiblePointCount = 60;
    public const int MaxVisiblePointCount = 120;

    public static int NormalizeVisiblePointCount(int candidate)
    {
        return candidate >= MaxVisiblePointCount ? MaxVisiblePointCount : MinVisiblePointCount;
    }

    public static MetricTrendChartRenderPlan CreatePlan(MetricTrendChartRenderRequest request)
    {
        IReadOnlyList<double> values = request.Values ?? Array.Empty<double>();
        IReadOnlyList<double> overlayValues = request.OverlayValues ?? Array.Empty<double>();
        int visiblePointCount = NormalizeVisiblePointCount(request.VisiblePointCount);

        (int lineStart, int lineCount) = ResolveWindow(values, visiblePointCount);
        (int overlayStart, int overlayCount) = ResolveWindow(overlayValues, visiblePointCount);

        IReadOnlyList<double> lineWindow = SliceWindow(values, lineStart, lineCount);
        IReadOnlyList<double> overlayWindow = SliceWindow(overlayValues, overlayStart, overlayCount);

        WindowStats lineStats = AnalyzeWindow(lineWindow);
        WindowStats overlayStats = AnalyzeWindow(overlayWindow);
        bool nonFiniteSeriesDetected = lineStats.HasNonFinite || overlayStats.HasNonFinite;

        double maxVisible = Math.Max(lineStats.Max, overlayStats.Max);
        (double floor, double? ceiling) = ResolveDomainPolicy(request.ScaleMode, request.DomainMaxOverride);
        double nextRawDomainMax = MetricTrendScaleDomain.ResolveNextRawDomainMax(
            previousRawDomainMax: request.PreviousRawDomainMax,
            maxVisible: maxVisible,
            floor: floor,
            ceiling: ceiling,
            paddingRatio: MetricTrendScaleDomain.DefaultPaddingRatio,
            decayFactor: MetricTrendScaleDomain.DefaultDecayFactor);

        double domainMax = MetricTrendScaleDomain.ResolveRenderedDomainMax(
            rawDomainMax: nextRawDomainMax,
            floor: floor,
            ceiling: ceiling);

        bool domainFallbackUsed = nonFiniteSeriesDetected && maxVisible <= 0d;
        if (!double.IsFinite(domainMax) || domainMax <= 0d)
        {
            domainFallbackUsed = true;
            domainMax = floor;
        }

        int populatedSlotCount = Math.Max(lineWindow.Count, overlayWindow.Count);
        int slotCount = populatedSlotCount == 0
            ? 0
            : visiblePointCount;
        int lineLeadingSlots = Math.Max(0, slotCount - lineWindow.Count);
        int overlayLeadingSlots = Math.Max(0, slotCount - overlayWindow.Count);

        return new MetricTrendChartRenderPlan(
            nextRawDomainMax,
            domainMax,
            maxVisible,
            nonFiniteSeriesDetected,
            domainFallbackUsed,
            lineWindow.Count == 0,
            overlayWindow.Count == 0,
            slotCount,
            new MetricTrendChartSeriesWindow(lineWindow, lineLeadingSlots),
            new MetricTrendChartSeriesWindow(overlayWindow, overlayLeadingSlots));
    }

    private static (double Floor, double? Ceiling) ResolveDomainPolicy(MetricTrendScaleMode scaleMode, double domainMaxOverride)
    {
        double? overrideMax = double.IsNaN(domainMaxOverride)
            ? null
            : Math.Max(0d, domainMaxOverride);

        return scaleMode switch
        {
            MetricTrendScaleMode.CpuPercent => (MetricTrendScaleDomain.CpuFloorPercent, MetricTrendScaleDomain.CpuCeilingPercent),
            MetricTrendScaleMode.MemoryBytes => (MetricTrendScaleDomain.MemoryFloorBytes, overrideMax),
            MetricTrendScaleMode.BitsRate => (MetricTrendScaleDomain.BitsRateFloor, overrideMax),
            _ => (MetricTrendScaleDomain.IoRateFloorBytes, overrideMax),
        };
    }

    private static (int Start, int Count) ResolveWindow(IReadOnlyList<double> values, int visiblePointCount)
    {
        if (values.Count == 0)
        {
            return (0, 0);
        }

        int count = Math.Min(values.Count, visiblePointCount);
        return (values.Count - count, count);
    }

    private static IReadOnlyList<double> SliceWindow(IReadOnlyList<double> values, int start, int count)
    {
        if (count <= 0 || values.Count == 0)
        {
            return Array.Empty<double>();
        }

        int safeStart = Math.Max(0, start);
        int safeCount = Math.Min(count, values.Count - safeStart);
        if (safeCount <= 0)
        {
            return Array.Empty<double>();
        }

        double[] window = new double[safeCount];
        for (int index = 0; index < safeCount; index++)
        {
            window[index] = values[safeStart + index];
        }

        return window;
    }

    private static WindowStats AnalyzeWindow(IReadOnlyList<double> values)
    {
        if (values.Count == 0)
        {
            return new WindowStats(0d, false);
        }

        double max = 0d;
        bool hasNonFinite = false;
        for (int index = 0; index < values.Count; index++)
        {
            double value = values[index];
            if (!double.IsFinite(value))
            {
                hasNonFinite = true;
                continue;
            }

            if (value > max)
            {
                max = value;
            }
        }

        return new WindowStats(max, hasNonFinite);
    }

    private readonly record struct WindowStats(double Max, bool HasNonFinite);
}

public readonly record struct MetricTrendChartRenderRequest(
    IReadOnlyList<double> Values,
    IReadOnlyList<double> OverlayValues,
    int VisiblePointCount,
    MetricTrendScaleMode ScaleMode,
    double DomainMaxOverride,
    double PreviousRawDomainMax);

public readonly record struct MetricTrendChartSeriesWindow(
    IReadOnlyList<double> Values,
    int LeadingSlots);

public readonly record struct MetricTrendChartRenderPlan(
    double NextRawDomainMax,
    double DomainMax,
    double MaxVisible,
    bool NonFiniteSeriesDetected,
    bool DomainFallbackUsed,
    bool LineFallbackUsed,
    bool OverlayFallbackUsed,
    int SlotCount,
    MetricTrendChartSeriesWindow LineSeries,
    MetricTrendChartSeriesWindow OverlaySeries);
