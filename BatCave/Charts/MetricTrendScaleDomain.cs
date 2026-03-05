using System;

namespace BatCave.Charts;

internal static class MetricTrendScaleDomain
{
    public const double CpuFloorPercent = 5d;
    public const double CpuCeilingPercent = 100d;
    public const double CpuPaddingRatio = 1.08d;
    public const double CpuDecayFactor = 0.03d;
    public const double MemoryFloorBytes = 256d * 1024d * 1024d;
    public const double IoRateFloorBytes = 1d * 1024d * 1024d;
    public const double BitsRateFloor = 1d * 1000d * 1000d;
    public const double DefaultPaddingRatio = 1.12d;
    public const double DefaultDecayFactor = 0.08d;

    private const double DomainSnapEpsilonRatio = 0.02d;

    public static double ResolveNextRawDomainMax(
        double previousRawDomainMax,
        double maxVisible,
        double floor,
        double? ceiling,
        double paddingRatio,
        double decayFactor)
    {
        double target = ResolveTargetDomainMax(
            maxVisible: maxVisible,
            floor: floor,
            ceiling: ceiling,
            paddingRatio: paddingRatio);

        if (previousRawDomainMax <= 0d || target >= previousRawDomainMax)
        {
            return target;
        }

        double decayed = previousRawDomainMax + (target - previousRawDomainMax) * decayFactor;
        decayed = Clamp(decayed, floor, ceiling);

        double snapThreshold = Math.Max(1e-6d, floor * DomainSnapEpsilonRatio);
        if (Math.Abs(decayed - target) <= snapThreshold)
        {
            return target;
        }

        return decayed;
    }

    public static double ResolveTargetDomainMax(
        double maxVisible,
        double floor,
        double? ceiling,
        double paddingRatio)
    {
        double normalizedMaxVisible = double.IsFinite(maxVisible)
            ? Math.Max(0d, maxVisible)
            : 0d;

        double padded = Math.Max(normalizedMaxVisible * paddingRatio, floor);
        double rounded = SparklineMath.RoundUpToNice(padded);
        return Clamp(rounded, floor, ceiling);
    }

    public static double ResolveRenderedDomainMax(
        double rawDomainMax,
        double floor,
        double? ceiling,
        bool roundUpToNice = true)
    {
        double clamped = Clamp(rawDomainMax, floor, ceiling);
        if (clamped <= floor)
        {
            return floor;
        }

        if (!roundUpToNice)
        {
            return clamped;
        }

        double rounded = SparklineMath.RoundUpToNice(clamped);
        return Clamp(rounded, floor, ceiling);
    }

    private static double Clamp(double value, double floor, double? ceiling)
    {
        double clamped = Math.Max(floor, value);
        if (ceiling.HasValue)
        {
            clamped = Math.Min(ceiling.Value, clamped);
        }

        return clamped;
    }
}
