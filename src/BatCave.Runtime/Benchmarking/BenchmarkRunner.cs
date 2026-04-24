using BatCave.Runtime.Collectors;
using BatCave.Runtime.Contracts;
using System.Diagnostics;

namespace BatCave.Runtime.Benchmarking;

public sealed record BenchmarkInteractionProbeP95
{
    public double FilterP95Ms { get; init; }
    public double SortP95Ms { get; init; }
    public double SelectionP95Ms { get; init; }
    public double BatchP95Ms { get; init; }
    public double PlotP95Ms { get; init; }
}

public sealed record BenchmarkComparison
{
    public double BaselineTickP95Ms { get; init; }
    public double BaselineSortP95Ms { get; init; }
    public double TickP95Speedup { get; init; }
    public double SortP95Speedup { get; init; }
    public bool MeetsMinSpeedup { get; init; } = true;
}

public sealed record BenchmarkInteractionComparison
{
    public BenchmarkInteractionProbeP95 BaselineP95Ms { get; init; } = new();
    public BenchmarkInteractionProbeP95 CurrentP95Ms { get; init; } = new();
    public double FilterP95Speedup { get; init; }
    public double SortP95Speedup { get; init; }
    public double SelectionP95Speedup { get; init; }
    public double BatchP95Speedup { get; init; }
    public double PlotP95Speedup { get; init; }
    public bool MeetsMinSpeedup { get; init; } = true;
}

public sealed record BenchmarkGateOptions
{
    public string Host { get; init; } = "core";
    public string MeasurementOrigin { get; init; } = BenchmarkRunner.CoreMeasurementOrigin;
    public bool UsesAttachedDispatcher { get; init; }
    public BenchmarkSummary? Baseline { get; init; }
    public double? MinSpeedupMultiplier { get; init; }
    public double? MaxP95Ms { get; init; }
    public BenchmarkInteractionProbeP95? InteractionProbeP95 { get; init; }
    public bool RequireInteractionProbeSpeedup { get; init; }
    public double CpuBudgetPct { get; init; } = BenchmarkRunner.CpuBudgetPct;
    public ulong RssBudgetBytes { get; init; } = BenchmarkRunner.RssBudgetBytes;
}

public sealed record BenchmarkMeasurement
{
    public string Host { get; init; } = "core";
    public string MeasurementOrigin { get; init; } = BenchmarkRunner.CoreMeasurementOrigin;
    public bool UsesAttachedDispatcher { get; init; }
    public int Ticks { get; init; }
    public int SleepMs { get; init; }
    public long StartupMs { get; init; }
    public double TickP95Ms { get; init; }
    public double SortP95Ms { get; init; }
    public double AvgAppCpuPct { get; init; }
    public ulong AvgAppRssBytes { get; init; }
    public BenchmarkInteractionProbeP95? InteractionProbeP95 { get; init; }
}

public sealed record BenchmarkSummary
{
    public string Host { get; init; } = "core";
    public string MeasurementOrigin { get; init; } = BenchmarkRunner.CoreMeasurementOrigin;
    public bool UsesAttachedDispatcher { get; init; }
    public int Ticks { get; init; }
    public int SleepMs { get; init; }
    public long StartupMs { get; init; }
    public double TickP95Ms { get; init; }
    public double SortP95Ms { get; init; }
    public double AvgAppCpuPct { get; init; }
    public ulong AvgAppRssBytes { get; init; }
    public bool BudgetPassed { get; init; }
    public double CpuBudgetPct { get; init; } = BenchmarkRunner.CpuBudgetPct;
    public ulong RssBudgetBytes { get; init; } = BenchmarkRunner.RssBudgetBytes;
    public bool BaselineMetadataMatched { get; init; } = true;
    public BenchmarkComparison? BaselineComparison { get; init; }
    public BenchmarkInteractionProbeP95? InteractionProbeP95 { get; init; }
    public BenchmarkInteractionComparison? InteractionBaselineComparison { get; init; }
    public double? MinSpeedupMultiplier { get; init; }
    public double? MaxP95Ms { get; init; }
    public bool CoreSpeedupPassed { get; init; } = true;
    public bool InteractionSpeedupPassed { get; init; } = true;
    public bool SpeedupPassed { get; init; } = true;
    public bool MaxP95Passed { get; init; } = true;
    public bool StrictPassed { get; init; } = true;
}

public static class BenchmarkRunner
{
    public const double CpuBudgetPct = 1.0;
    public const ulong RssBudgetBytes = 150UL * 1024UL * 1024UL;
    public const string CoreMeasurementOrigin = "headless_runtime";

    public static BenchmarkSummary Run(
        int ticks,
        int sleepMs,
        CancellationToken ct,
        BenchmarkGateOptions? gateOptions = null)
    {
        ticks = Math.Max(1, ticks);
        sleepMs = Math.Max(0, sleepMs);
        BenchmarkGateOptions gates = gateOptions ?? new BenchmarkGateOptions();

        WindowsProcessCollector collector = new();
        List<double> tickSamples = new(ticks);
        List<double> sortSamples = new(ticks);
        List<double> cpuSamples = new(ticks);
        List<ulong> rssSamples = new(ticks);
        Stopwatch total = Stopwatch.StartNew();
        Process currentProcess = Process.GetCurrentProcess();
        TimeSpan previousCpu = currentProcess.TotalProcessorTime;
        long previousStamp = Stopwatch.GetTimestamp();

        for (int index = 0; index < ticks; index++)
        {
            ct.ThrowIfCancellationRequested();
            Stopwatch tick = Stopwatch.StartNew();
            IReadOnlyList<ProcessSample> rows = collector.Collect((ulong)(index + 1));
            tick.Stop();
            tickSamples.Add(tick.Elapsed.TotalMilliseconds);

            Stopwatch sort = Stopwatch.StartNew();
            _ = rows
                .OrderByDescending(static row => row.CpuPct)
                .ThenBy(static row => row.Name, StringComparer.OrdinalIgnoreCase)
                .Take(200)
                .ToArray();
            sort.Stop();
            sortSamples.Add(sort.Elapsed.TotalMilliseconds);

            currentProcess.Refresh();
            TimeSpan currentCpu = currentProcess.TotalProcessorTime;
            long currentStamp = Stopwatch.GetTimestamp();
            double elapsedMs = Math.Max(1d, (currentStamp - previousStamp) * 1000d / Stopwatch.Frequency);
            double cpuDeltaMs = Math.Max(0d, (currentCpu - previousCpu).TotalMilliseconds);
            cpuSamples.Add(cpuDeltaMs / elapsedMs / Math.Max(1, Environment.ProcessorCount) * 100d);
            rssSamples.Add((ulong)Math.Max(0L, currentProcess.WorkingSet64));
            previousCpu = currentCpu;
            previousStamp = currentStamp;

            if (sleepMs > 0)
            {
                Thread.Sleep(sleepMs);
            }
        }

        total.Stop();
        double tickP95 = Percentile95(tickSamples);
        double sortP95 = Percentile95(sortSamples);
        double avgCpu = cpuSamples.Count == 0 ? 0d : cpuSamples.Average();
        ulong avgRss = rssSamples.Count == 0 ? 0UL : (ulong)rssSamples.Average(static value => (double)value);
        double cpuBudgetPct = gates.CpuBudgetPct <= 0d ? CpuBudgetPct : gates.CpuBudgetPct;
        ulong rssBudgetBytes = gates.RssBudgetBytes == 0UL ? RssBudgetBytes : gates.RssBudgetBytes;
        bool budgetPassed = avgCpu < cpuBudgetPct && avgRss < rssBudgetBytes;
        double? minSpeedup = NormalizeOptionalPositive(gates.MinSpeedupMultiplier);
        double? maxP95 = NormalizeOptionalPositive(gates.MaxP95Ms);
        bool baselineMetadataMatched = BaselineMetadataMatches(gates.Baseline, gates.Host, gates.MeasurementOrigin);
        BenchmarkSummary? compatibleBaseline = baselineMetadataMatched ? gates.Baseline : null;
        BenchmarkComparison? comparison = BuildComparison(compatibleBaseline, tickP95, sortP95, minSpeedup);
        BenchmarkInteractionProbeP95? interactionProbe = NormalizeInteractionProbe(gates.InteractionProbeP95);
        BenchmarkInteractionComparison? interactionComparison = BuildInteractionComparison(
            compatibleBaseline?.InteractionProbeP95,
            interactionProbe,
            minSpeedup);
        bool coreSpeedupPassed = minSpeedup is null || (comparison?.MeetsMinSpeedup ?? false);
        bool interactionSpeedupPassed = InteractionSpeedupPassed(gates.RequireInteractionProbeSpeedup, minSpeedup, interactionComparison);
        bool maxP95Passed = !maxP95.HasValue || (tickP95 <= maxP95.Value && sortP95 <= maxP95.Value);

        return new BenchmarkSummary
        {
            Host = gates.Host,
            MeasurementOrigin = gates.MeasurementOrigin,
            UsesAttachedDispatcher = gates.UsesAttachedDispatcher,
            Ticks = ticks,
            SleepMs = sleepMs,
            StartupMs = total.ElapsedMilliseconds,
            TickP95Ms = Math.Round(tickP95, 4),
            SortP95Ms = Math.Round(sortP95, 4),
            AvgAppCpuPct = avgCpu,
            AvgAppRssBytes = avgRss,
            CpuBudgetPct = cpuBudgetPct,
            RssBudgetBytes = rssBudgetBytes,
            BudgetPassed = budgetPassed,
            BaselineMetadataMatched = baselineMetadataMatched,
            BaselineComparison = comparison,
            InteractionProbeP95 = interactionProbe,
            InteractionBaselineComparison = interactionComparison,
            MinSpeedupMultiplier = minSpeedup,
            MaxP95Ms = maxP95,
            CoreSpeedupPassed = coreSpeedupPassed,
            InteractionSpeedupPassed = interactionSpeedupPassed,
            SpeedupPassed = coreSpeedupPassed && interactionSpeedupPassed,
            MaxP95Passed = maxP95Passed,
            StrictPassed = budgetPassed && coreSpeedupPassed && interactionSpeedupPassed && maxP95Passed,
        };
    }

    public static BenchmarkSummary CreateSummary(BenchmarkMeasurement measurement, BenchmarkGateOptions gateOptions)
    {
        BenchmarkGateOptions gates = gateOptions with
        {
            Host = measurement.Host,
            MeasurementOrigin = measurement.MeasurementOrigin,
            UsesAttachedDispatcher = measurement.UsesAttachedDispatcher,
        };
        double tickP95 = measurement.TickP95Ms;
        double sortP95 = measurement.SortP95Ms;
        double avgCpu = measurement.AvgAppCpuPct;
        ulong avgRss = measurement.AvgAppRssBytes;
        double cpuBudgetPct = gates.CpuBudgetPct <= 0d ? CpuBudgetPct : gates.CpuBudgetPct;
        ulong rssBudgetBytes = gates.RssBudgetBytes == 0UL ? RssBudgetBytes : gates.RssBudgetBytes;
        bool budgetPassed = avgCpu < cpuBudgetPct && avgRss < rssBudgetBytes;
        double? minSpeedup = NormalizeOptionalPositive(gates.MinSpeedupMultiplier);
        double? maxP95 = NormalizeOptionalPositive(gates.MaxP95Ms);
        bool baselineMetadataMatched = BaselineMetadataMatches(gates.Baseline, gates.Host, gates.MeasurementOrigin);
        BenchmarkSummary? compatibleBaseline = baselineMetadataMatched ? gates.Baseline : null;
        BenchmarkComparison? comparison = BuildComparison(compatibleBaseline, tickP95, sortP95, minSpeedup);
        BenchmarkInteractionProbeP95? interactionProbe = NormalizeInteractionProbe(
            measurement.InteractionProbeP95 ?? gates.InteractionProbeP95);
        BenchmarkInteractionComparison? interactionComparison = BuildInteractionComparison(
            compatibleBaseline?.InteractionProbeP95,
            interactionProbe,
            minSpeedup);
        bool coreSpeedupPassed = minSpeedup is null || (comparison?.MeetsMinSpeedup ?? false);
        bool interactionSpeedupPassed = InteractionSpeedupPassed(gates.RequireInteractionProbeSpeedup, minSpeedup, interactionComparison);
        bool maxP95Passed = !maxP95.HasValue || (tickP95 <= maxP95.Value && sortP95 <= maxP95.Value);

        return new BenchmarkSummary
        {
            Host = gates.Host,
            MeasurementOrigin = gates.MeasurementOrigin,
            UsesAttachedDispatcher = gates.UsesAttachedDispatcher,
            Ticks = Math.Max(1, measurement.Ticks),
            SleepMs = Math.Max(0, measurement.SleepMs),
            StartupMs = measurement.StartupMs,
            TickP95Ms = Math.Round(tickP95, 4),
            SortP95Ms = Math.Round(sortP95, 4),
            AvgAppCpuPct = avgCpu,
            AvgAppRssBytes = avgRss,
            CpuBudgetPct = cpuBudgetPct,
            RssBudgetBytes = rssBudgetBytes,
            BudgetPassed = budgetPassed,
            BaselineMetadataMatched = baselineMetadataMatched,
            BaselineComparison = comparison,
            InteractionProbeP95 = interactionProbe,
            InteractionBaselineComparison = interactionComparison,
            MinSpeedupMultiplier = minSpeedup,
            MaxP95Ms = maxP95,
            CoreSpeedupPassed = coreSpeedupPassed,
            InteractionSpeedupPassed = interactionSpeedupPassed,
            SpeedupPassed = coreSpeedupPassed && interactionSpeedupPassed,
            MaxP95Passed = maxP95Passed,
            StrictPassed = budgetPassed && coreSpeedupPassed && interactionSpeedupPassed && maxP95Passed,
        };
    }

    private static BenchmarkComparison? BuildComparison(
        BenchmarkSummary? baseline,
        double tickP95,
        double sortP95,
        double? minSpeedupMultiplier)
    {
        if (baseline is null)
        {
            return null;
        }

        double tickSpeedup = ComputeSpeedup(baseline.TickP95Ms, tickP95);
        double sortSpeedup = ComputeSpeedup(baseline.SortP95Ms, sortP95);
        bool meets = !minSpeedupMultiplier.HasValue
                     || (tickSpeedup >= minSpeedupMultiplier.Value && sortSpeedup >= minSpeedupMultiplier.Value);
        return new BenchmarkComparison
        {
            BaselineTickP95Ms = baseline.TickP95Ms,
            BaselineSortP95Ms = baseline.SortP95Ms,
            TickP95Speedup = tickSpeedup,
            SortP95Speedup = sortSpeedup,
            MeetsMinSpeedup = meets,
        };
    }

    private static BenchmarkInteractionComparison? BuildInteractionComparison(
        BenchmarkInteractionProbeP95? baseline,
        BenchmarkInteractionProbeP95? current,
        double? minSpeedupMultiplier)
    {
        if (baseline is null || current is null || !HasComparableProbeSamples(baseline, current))
        {
            return null;
        }

        double filterSpeedup = ComputeSpeedup(baseline.FilterP95Ms, current.FilterP95Ms);
        double sortSpeedup = ComputeSpeedup(baseline.SortP95Ms, current.SortP95Ms);
        double selectionSpeedup = ComputeSpeedup(baseline.SelectionP95Ms, current.SelectionP95Ms);
        double batchSpeedup = ComputeSpeedup(baseline.BatchP95Ms, current.BatchP95Ms);
        double plotSpeedup = ComputeSpeedup(baseline.PlotP95Ms, current.PlotP95Ms);
        bool meets = !minSpeedupMultiplier.HasValue
                     || (filterSpeedup >= minSpeedupMultiplier.Value
                         && sortSpeedup >= minSpeedupMultiplier.Value
                         && selectionSpeedup >= minSpeedupMultiplier.Value
                         && batchSpeedup >= minSpeedupMultiplier.Value
                         && plotSpeedup >= minSpeedupMultiplier.Value);

        return new BenchmarkInteractionComparison
        {
            BaselineP95Ms = baseline,
            CurrentP95Ms = current,
            FilterP95Speedup = filterSpeedup,
            SortP95Speedup = sortSpeedup,
            SelectionP95Speedup = selectionSpeedup,
            BatchP95Speedup = batchSpeedup,
            PlotP95Speedup = plotSpeedup,
            MeetsMinSpeedup = meets,
        };
    }

    private static double ComputeSpeedup(double baseline, double current)
    {
        if (baseline <= 0d || current <= 0d)
        {
            return 1d;
        }

        return baseline / current;
    }

    private static bool InteractionSpeedupPassed(
        bool requireInteractionProbeSpeedup,
        double? minSpeedupMultiplier,
        BenchmarkInteractionComparison? interactionComparison)
    {
        if (!minSpeedupMultiplier.HasValue)
        {
            return true;
        }

        return interactionComparison?.MeetsMinSpeedup == true
               || (interactionComparison is null && !requireInteractionProbeSpeedup);
    }

    private static bool BaselineMetadataMatches(
        BenchmarkSummary? baseline,
        string currentHost,
        string currentMeasurementOrigin)
    {
        if (baseline is null)
        {
            return true;
        }

        string baselineHost = NormalizeMetadataToken(baseline.Host, string.Empty);
        string baselineOrigin = NormalizeMetadataToken(baseline.MeasurementOrigin, string.Empty);
        string host = NormalizeMetadataToken(currentHost, string.Empty);
        string origin = NormalizeMetadataToken(currentMeasurementOrigin, string.Empty);
        if (string.IsNullOrWhiteSpace(host) || string.IsNullOrWhiteSpace(origin))
        {
            return false;
        }

        if (string.IsNullOrWhiteSpace(baselineHost) || string.IsNullOrWhiteSpace(baselineOrigin))
        {
            return true;
        }

        return string.Equals(baselineHost, host, StringComparison.OrdinalIgnoreCase)
               && string.Equals(baselineOrigin, origin, StringComparison.OrdinalIgnoreCase);
    }

    private static string NormalizeMetadataToken(string? value, string fallback)
    {
        return string.IsNullOrWhiteSpace(value) ? fallback : value.Trim();
    }

    private static double? NormalizeOptionalPositive(double? value)
    {
        if (!value.HasValue || value.Value <= 0d || double.IsNaN(value.Value) || double.IsInfinity(value.Value))
        {
            return null;
        }

        return value.Value;
    }

    private static BenchmarkInteractionProbeP95? NormalizeInteractionProbe(BenchmarkInteractionProbeP95? value)
    {
        if (value is null)
        {
            return null;
        }

        BenchmarkInteractionProbeP95 normalized = new()
        {
            FilterP95Ms = NormalizeNonNegative(value.FilterP95Ms),
            SortP95Ms = NormalizeNonNegative(value.SortP95Ms),
            SelectionP95Ms = NormalizeNonNegative(value.SelectionP95Ms),
            BatchP95Ms = NormalizeNonNegative(value.BatchP95Ms),
            PlotP95Ms = NormalizeNonNegative(value.PlotP95Ms),
        };

        return HasAnyProbeSample(normalized) ? normalized : null;
    }

    private static bool HasComparableProbeSamples(BenchmarkInteractionProbeP95 baseline, BenchmarkInteractionProbeP95 current)
    {
        return HasPositiveComparablePair(baseline.FilterP95Ms, current.FilterP95Ms)
               && HasPositiveComparablePair(baseline.SortP95Ms, current.SortP95Ms)
               && HasPositiveComparablePair(baseline.SelectionP95Ms, current.SelectionP95Ms)
               && HasPositiveComparablePair(baseline.BatchP95Ms, current.BatchP95Ms)
               && HasPositiveComparablePair(baseline.PlotP95Ms, current.PlotP95Ms);
    }

    private static bool HasPositiveComparablePair(double baselineValue, double currentValue)
    {
        return baselineValue > 0d && currentValue > 0d;
    }

    private static bool HasAnyProbeSample(BenchmarkInteractionProbeP95 probe)
    {
        return probe.FilterP95Ms > 0d
               || probe.SortP95Ms > 0d
               || probe.SelectionP95Ms > 0d
               || probe.BatchP95Ms > 0d
               || probe.PlotP95Ms > 0d;
    }

    private static double NormalizeNonNegative(double value)
    {
        return double.IsNaN(value) || double.IsInfinity(value) || value < 0d ? 0d : value;
    }

    private static double Percentile95(IReadOnlyList<double> samples)
    {
        if (samples.Count == 0)
        {
            return 0d;
        }

        double[] ordered = [.. samples];
        Array.Sort(ordered);
        int index = (int)Math.Ceiling(ordered.Length * 0.95d) - 1;
        return ordered[Math.Clamp(index, 0, ordered.Length - 1)];
    }
}
