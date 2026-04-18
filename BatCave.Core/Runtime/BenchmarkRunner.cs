using BatCave.Core.Abstractions;
using BatCave.Core.Collector;
using BatCave.Core.Domain;
using BatCave.Core.Pipeline;
using BatCave.Core.Sort;
using BatCave.Core.State;
using System.Diagnostics;

namespace BatCave.Core.Runtime;

public sealed record BenchmarkSummary
{
    public string Host { get; init; } = string.Empty;

    public string MeasurementOrigin { get; init; } = string.Empty;

    public bool UsesAttachedDispatcher { get; init; }

    public int Ticks { get; init; }

    public int SleepMs { get; init; }

    public long StartupMs { get; init; }

    public double TickP95Ms { get; init; }

    public double SortP95Ms { get; init; }

    public double AvgAppCpuPct { get; init; }

    public ulong AvgAppRssBytes { get; init; }

    public bool BudgetPassed { get; init; }

    public double CpuBudgetPct { get; init; }

    public ulong RssBudgetBytes { get; init; }

    public BenchmarkComparison? BaselineComparison { get; init; }

    public bool BaselineMetadataMatched { get; init; } = true;

    public BenchmarkInteractionProbeP95? InteractionProbeP95 { get; init; }

    public BenchmarkInteractionComparison? InteractionBaselineComparison { get; init; }

    public double? MinSpeedupMultiplier { get; init; }

    public double? MaxP95Ms { get; init; }

    public bool CoreSpeedupPassed { get; init; }

    public bool InteractionSpeedupPassed { get; init; }

    public bool SpeedupPassed { get; init; }

    public bool MaxP95Passed { get; init; }

    public bool StrictPassed { get; init; }
}

public sealed record BenchmarkComparison
{
    public double BaselineTickP95Ms { get; init; }

    public double BaselineSortP95Ms { get; init; }

    public double TickP95Speedup { get; init; }

    public double SortP95Speedup { get; init; }

    public bool MeetsMinSpeedup { get; init; }
}

public sealed record BenchmarkInteractionProbeP95
{
    public double FilterP95Ms { get; init; }

    public double SortP95Ms { get; init; }

    public double SelectionP95Ms { get; init; }

    public double BatchP95Ms { get; init; }

    public double PlotP95Ms { get; init; }
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

    public bool MeetsMinSpeedup { get; init; }
}

public sealed record BenchmarkGateOptions
{
    public BenchmarkSummary? Baseline { get; init; }

    public double? MinSpeedupMultiplier { get; init; }

    public double? MaxP95Ms { get; init; }

    public BenchmarkInteractionProbeP95? InteractionProbeP95 { get; init; }

    public bool RequireInteractionProbeSpeedup { get; init; }
}

public sealed record BenchmarkMeasurement
{
    public string Host { get; init; } = string.Empty;

    public string MeasurementOrigin { get; init; } = string.Empty;

    public bool UsesAttachedDispatcher { get; init; }

    public int Ticks { get; init; }

    public int SleepMs { get; init; }

    public long StartupMs { get; init; }

    public double TickP95Ms { get; init; }

    public double SortP95Ms { get; init; }

    public double AvgAppCpuPct { get; init; }

    public ulong AvgAppRssBytes { get; init; }
}

public static class BenchmarkRunner
{
    public const double CpuBudgetPct = 1.0;
    public const ulong RssBudgetBytes = 150UL * 1024UL * 1024UL;

    public static BenchmarkSummary Run(
        int ticks,
        int sleepMs,
        CancellationToken ct,
        BenchmarkGateOptions? gateOptions = null)
    {
        int safeTicks = Math.Max(0, ticks);
        int safeSleepMs = Math.Max(0, sleepMs);

        Stopwatch startupStopwatch = Stopwatch.StartNew();
        using MonitoringRuntime runtime = new(
            new DefaultProcessCollectorFactory(),
            new DeltaTelemetryPipeline(),
            new InMemoryStateStore(),
            new IncrementalSortIndexEngine(),
            new BenchmarkPersistenceStore(),
            new RuntimeHostOptions());
        runtime.InitializeAsync(ct).GetAwaiter().GetResult();
        runtime.GetSnapshot();
        startupStopwatch.Stop();

        List<double> tickSamples = new(safeTicks);
        List<double> sortSamples = new(safeTicks);
        double cpuAccum = 0;
        ulong rssAccum = 0;

        for (int index = 0; index < safeTicks; index++)
        {
            ct.ThrowIfCancellationRequested();

            Stopwatch tickStopwatch = Stopwatch.StartNew();
            runtime.Tick(0);
            tickStopwatch.Stop();
            tickSamples.Add(tickStopwatch.Elapsed.TotalMilliseconds);

            Stopwatch sortStopwatch = Stopwatch.StartNew();
            runtime.GetSnapshot();
            sortStopwatch.Stop();
            sortSamples.Add(sortStopwatch.Elapsed.TotalMilliseconds);

            RuntimeHealth health = runtime.GetRuntimeHealth();
            cpuAccum += health.AppCpuPct;
            rssAccum = AddSaturating(rssAccum, health.AppRssBytes);

            if (safeSleepMs > 0)
            {
                Thread.Sleep(safeSleepMs);
            }
        }

        double avgCpu = safeTicks > 0 ? cpuAccum / safeTicks : 0;
        ulong avgRss = safeTicks > 0 ? rssAccum / (ulong)safeTicks : 0;
        bool budgetPassed = avgCpu < CpuBudgetPct && avgRss < RssBudgetBytes;
        double tickP95Ms = PercentileMath.Percentile95(tickSamples);
        double sortP95Ms = PercentileMath.Percentile95(sortSamples);

        return CreateSummary(
            new BenchmarkMeasurement
            {
                Host = "core",
                MeasurementOrigin = "headless_runtime",
                UsesAttachedDispatcher = false,
                Ticks = safeTicks,
                SleepMs = safeSleepMs,
                StartupMs = startupStopwatch.ElapsedMilliseconds,
                TickP95Ms = tickP95Ms,
                SortP95Ms = sortP95Ms,
                AvgAppCpuPct = avgCpu,
                AvgAppRssBytes = avgRss,
            },
            gateOptions);
    }

    public static BenchmarkSummary CreateSummary(
        BenchmarkMeasurement measurement,
        BenchmarkGateOptions? gateOptions = null)
    {
        ArgumentNullException.ThrowIfNull(measurement);

        BenchmarkGateOptions normalizedGateOptions = gateOptions ?? new BenchmarkGateOptions();
        double normalizedTickP95Ms = NormalizeNonNegative(measurement.TickP95Ms);
        double normalizedSortP95Ms = NormalizeNonNegative(measurement.SortP95Ms);
        double normalizedAvgCpuPct = NormalizeNonNegative(measurement.AvgAppCpuPct);
        ulong normalizedAvgRssBytes = measurement.AvgAppRssBytes;
        bool budgetPassed = normalizedAvgCpuPct < CpuBudgetPct && normalizedAvgRssBytes < RssBudgetBytes;
        double? minSpeedupMultiplier = NormalizeOptionalPositive(normalizedGateOptions.MinSpeedupMultiplier);
        double? maxP95Ms = NormalizeOptionalPositive(normalizedGateOptions.MaxP95Ms);
        string normalizedHost = NormalizeMetadataToken(measurement.Host, "unknown");
        string normalizedMeasurementOrigin = NormalizeMetadataToken(measurement.MeasurementOrigin, "unknown");
        bool baselineMetadataMatched = BaselineMetadataMatches(
            normalizedGateOptions.Baseline,
            normalizedHost,
            normalizedMeasurementOrigin);

        BenchmarkSummary? compatibleBaseline = baselineMetadataMatched
            ? normalizedGateOptions.Baseline
            : null;

        BenchmarkComparison? comparison = BuildBaselineComparison(
            compatibleBaseline,
            normalizedTickP95Ms,
            normalizedSortP95Ms,
            minSpeedupMultiplier);

        BenchmarkInteractionProbeP95? interactionProbe = NormalizeInteractionProbe(normalizedGateOptions.InteractionProbeP95);
        BenchmarkInteractionComparison? interactionComparison = BuildInteractionComparison(
            compatibleBaseline?.InteractionProbeP95,
            interactionProbe,
            minSpeedupMultiplier);

        bool coreSpeedupPassed = minSpeedupMultiplier is null || (comparison?.MeetsMinSpeedup ?? false);
        bool interactionSpeedupPassed = minSpeedupMultiplier is null
                                        || interactionComparison?.MeetsMinSpeedup == true
                                        || (interactionComparison is null
                                            && !normalizedGateOptions.RequireInteractionProbeSpeedup);
        bool speedupPassed = coreSpeedupPassed && interactionSpeedupPassed;
        bool maxP95Passed = maxP95Ms is null
                            || (normalizedTickP95Ms <= maxP95Ms.Value && normalizedSortP95Ms <= maxP95Ms.Value);
        bool strictPassed = budgetPassed && speedupPassed && maxP95Passed;

        return new BenchmarkSummary
        {
            Host = normalizedHost,
            MeasurementOrigin = normalizedMeasurementOrigin,
            UsesAttachedDispatcher = measurement.UsesAttachedDispatcher,
            Ticks = Math.Max(0, measurement.Ticks),
            SleepMs = Math.Max(0, measurement.SleepMs),
            StartupMs = Math.Max(0L, measurement.StartupMs),
            TickP95Ms = normalizedTickP95Ms,
            SortP95Ms = normalizedSortP95Ms,
            AvgAppCpuPct = normalizedAvgCpuPct,
            AvgAppRssBytes = normalizedAvgRssBytes,
            BudgetPassed = budgetPassed,
            CpuBudgetPct = CpuBudgetPct,
            RssBudgetBytes = RssBudgetBytes,
            BaselineComparison = comparison,
            BaselineMetadataMatched = baselineMetadataMatched,
            InteractionProbeP95 = interactionProbe,
            InteractionBaselineComparison = interactionComparison,
            MinSpeedupMultiplier = minSpeedupMultiplier,
            MaxP95Ms = maxP95Ms,
            CoreSpeedupPassed = coreSpeedupPassed,
            InteractionSpeedupPassed = interactionSpeedupPassed,
            SpeedupPassed = speedupPassed,
            MaxP95Passed = maxP95Passed,
            StrictPassed = strictPassed,
        };
    }

    private static ulong AddSaturating(ulong left, ulong right)
    {
        ulong sum = left + right;
        return sum < left ? ulong.MaxValue : sum;
    }

    private static BenchmarkComparison? BuildBaselineComparison(
        BenchmarkSummary? baseline,
        double tickP95Ms,
        double sortP95Ms,
        double? minSpeedupMultiplier)
    {
        if (baseline is null)
        {
            return null;
        }

        double tickSpeedup = ComputeSpeedupMultiplier(baseline.TickP95Ms, tickP95Ms);
        double sortSpeedup = ComputeSpeedupMultiplier(baseline.SortP95Ms, sortP95Ms);
        bool meetsMinSpeedup = minSpeedupMultiplier is null
                               || (tickSpeedup >= minSpeedupMultiplier.Value
                                   && sortSpeedup >= minSpeedupMultiplier.Value);

        return new BenchmarkComparison
        {
            BaselineTickP95Ms = baseline.TickP95Ms,
            BaselineSortP95Ms = baseline.SortP95Ms,
            TickP95Speedup = tickSpeedup,
            SortP95Speedup = sortSpeedup,
            MeetsMinSpeedup = meetsMinSpeedup,
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

        double filterSpeedup = ComputeSpeedupMultiplier(baseline.FilterP95Ms, current.FilterP95Ms);
        double sortSpeedup = ComputeSpeedupMultiplier(baseline.SortP95Ms, current.SortP95Ms);
        double selectionSpeedup = ComputeSpeedupMultiplier(baseline.SelectionP95Ms, current.SelectionP95Ms);
        double batchSpeedup = ComputeSpeedupMultiplier(baseline.BatchP95Ms, current.BatchP95Ms);
        double plotSpeedup = ComputeSpeedupMultiplier(baseline.PlotP95Ms, current.PlotP95Ms);
        bool meetsMinSpeedup = minSpeedupMultiplier is null
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
            MeetsMinSpeedup = meetsMinSpeedup,
        };
    }

    private static double ComputeSpeedupMultiplier(double baselineP95Ms, double currentP95Ms)
    {
        if (baselineP95Ms <= 0d && currentP95Ms <= 0d)
        {
            return 1d;
        }

        if (currentP95Ms <= 0d)
        {
            return double.MaxValue;
        }

        if (baselineP95Ms <= 0d)
        {
            return 0d;
        }

        return baselineP95Ms / currentP95Ms;
    }

    private static double? NormalizeOptionalPositive(double? value)
    {
        if (!value.HasValue
            || value.Value <= 0d
            || double.IsNaN(value.Value)
            || double.IsInfinity(value.Value))
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

    private static bool HasComparableProbeSamples(
        BenchmarkInteractionProbeP95 baseline,
        BenchmarkInteractionProbeP95 current)
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
        if (value <= 0d || double.IsNaN(value) || double.IsInfinity(value))
        {
            return 0d;
        }

        return value;
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
        string baselineMeasurementOrigin = NormalizeMetadataToken(baseline.MeasurementOrigin, string.Empty);
        if (string.IsNullOrWhiteSpace(currentHost)
            || string.IsNullOrWhiteSpace(currentMeasurementOrigin))
        {
            return false;
        }

        if (string.IsNullOrWhiteSpace(baselineHost)
            || string.IsNullOrWhiteSpace(baselineMeasurementOrigin))
        {
            return true;
        }

        return string.Equals(baselineHost, currentHost, StringComparison.OrdinalIgnoreCase)
               && string.Equals(baselineMeasurementOrigin, currentMeasurementOrigin, StringComparison.OrdinalIgnoreCase);
    }

    private static string NormalizeMetadataToken(string? value, string fallback)
    {
        return string.IsNullOrWhiteSpace(value) ? fallback : value.Trim();
    }

    private sealed class BenchmarkPersistenceStore : IPersistenceStore
    {
        public string BaseDirectory => Path.GetTempPath();

        public UserSettings? LoadSettings()
        {
            return new UserSettings
            {
                AdminMode = false,
                AdminPreferenceInitialized = true,
                SortCol = SortColumn.CpuPct,
                SortDir = SortDirection.Desc,
                FilterText = string.Empty,
            };
        }

        public Task SaveSettingsAsync(UserSettings settings, CancellationToken ct)
        {
            return Task.CompletedTask;
        }

        public WarmCache? LoadWarmCache()
        {
            return null;
        }

        public Task SaveWarmCacheAsync(WarmCache cache, CancellationToken ct)
        {
            return Task.CompletedTask;
        }

        public Task AppendDiagnosticAsync(string category, object payload, CancellationToken ct)
        {
            return Task.CompletedTask;
        }

        public string? TakeWarning()
        {
            return null;
        }
    }
}





