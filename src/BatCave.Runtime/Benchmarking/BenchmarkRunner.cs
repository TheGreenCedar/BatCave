using BatCave.Runtime.Collectors;
using BatCave.Runtime.Contracts;
using BatCave.Runtime.Persistence;
using BatCave.Runtime.Presentation;
using BatCave.Runtime.Store;
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

public interface IWinUiBenchmarkRunner
{
    ValueTask<BenchmarkSummary> RunAsync(int ticks, int sleepMs, BenchmarkGateOptions gates, CancellationToken ct);
}

public sealed class RuntimeWinUiBenchmarkRunner : IWinUiBenchmarkRunner
{
    public ValueTask<BenchmarkSummary> RunAsync(int ticks, int sleepMs, BenchmarkGateOptions gates, CancellationToken ct)
    {
        return ValueTask.FromResult(BenchmarkRunner.Run(ticks, sleepMs, ct, gates with { UsesAttachedDispatcher = false }));
    }
}

public static class BenchmarkRunner
{
    public const double CpuBudgetPct = 1.0;
    public const ulong RssBudgetBytes = 150UL * 1024UL * 1024UL;
    public const string CoreMeasurementOrigin = "headless_runtime";
    public const string WinUiMeasurementOrigin = "live_shell";

    public static BenchmarkSummary Run(
        int ticks,
        int sleepMs,
        CancellationToken ct,
        BenchmarkGateOptions? gateOptions = null)
    {
        ticks = Math.Max(1, ticks);
        sleepMs = Math.Max(0, sleepMs);
        BenchmarkGateOptions gates = gateOptions ?? new BenchmarkGateOptions();
        if (IsWinUiHost(gates))
        {
            return RunWinUiPath(ticks, sleepMs, ct, gates with { UsesAttachedDispatcher = false });
        }

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

    private static BenchmarkSummary RunWinUiPath(
        int ticks,
        int sleepMs,
        CancellationToken ct,
        BenchmarkGateOptions gates)
    {
        List<double> tickSamples = new(ticks);
        List<double> filterSamples = new(ticks);
        List<double> sortSamples = new(ticks);
        List<double> selectionSamples = new(ticks);
        List<double> uiBatchSamples = new(ticks);
        List<double> plotSamples = new(ticks);
        List<double> cpuSamples = new(ticks);
        List<ulong> rssSamples = new(ticks);
        RuntimeViewState viewState = new();
        double[] cpuTrend = new double[60];
        double[] memoryTrend = new double[60];
        int trendIndex = 0;
        Stopwatch total = Stopwatch.StartNew();
        using Process currentProcess = Process.GetCurrentProcess();
        TimeSpan previousCpu = currentProcess.TotalProcessorTime;
        long previousStamp = Stopwatch.GetTimestamp();
        RuntimeStore store = new(
            new WindowsProcessCollector(),
            new WindowsSystemMetricsCollector(),
            new BenchmarkPersistenceStore(),
            new RuntimeStoreOptions
            {
                TickInterval = TimeSpan.FromHours(1),
                SubscriberBufferCapacity = 2,
                WarmCacheWriteIntervalTicks = int.MaxValue,
                DefaultSettings = new RuntimeSettings
                {
                    Paused = true,
                    MetricWindowSeconds = 60,
                },
            });

        try
        {
            store.StartAsync(ct).GetAwaiter().GetResult();
            for (int index = 0; index < ticks; index++)
            {
                ct.ThrowIfCancellationRequested();

                Stopwatch tick = Stopwatch.StartNew();
                store.ExecuteAsync(new RefreshNowCommand(), ct).GetAwaiter().GetResult();
                RuntimeSnapshot snapshot = store.GetSnapshot();
                tick.Stop();
                tickSamples.Add(tick.Elapsed.TotalMilliseconds);

                Stopwatch filter = Stopwatch.StartNew();
                RuntimeQuery filterQuery = snapshot.Settings.Query with
                {
                    FilterText = BuildProbeFilter(snapshot, index),
                };
                store.ExecuteAsync(new SetProcessQueryCommand(filterQuery), ct).GetAwaiter().GetResult();
                snapshot = store.GetSnapshot();
                filter.Stop();
                filterSamples.Add(filter.Elapsed.TotalMilliseconds);

                Stopwatch sort = Stopwatch.StartNew();
                viewState = RuntimeViewReducer.Reduce(viewState, snapshot);
                sort.Stop();
                sortSamples.Add(sort.Elapsed.TotalMilliseconds);

                Stopwatch selection = Stopwatch.StartNew();
                ProcessSample? selected = viewState.SelectedIdentity.HasValue
                    ? viewState.Rows.FirstOrDefault(row => row.Identity().Equals(viewState.SelectedIdentity.Value))
                    : viewState.Rows.FirstOrDefault();
                selection.Stop();
                selectionSamples.Add(selection.Elapsed.TotalMilliseconds);

                Stopwatch uiBatch = Stopwatch.StartNew();
                _ = viewState.Rows.Take(200).Select(static row => new BenchmarkRowAdapter(row)).ToArray();
                uiBatch.Stop();
                uiBatchSamples.Add(uiBatch.Elapsed.TotalMilliseconds);

                Stopwatch plot = Stopwatch.StartNew();
                AppendTrend(snapshot, cpuTrend, memoryTrend, ref trendIndex);
                _ = selected;
                plot.Stop();
                plotSamples.Add(plot.Elapsed.TotalMilliseconds);

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
        }
        finally
        {
            store.DisposeAsync().AsTask().GetAwaiter().GetResult();
            total.Stop();
        }

        double tickP95 = Percentile95(tickSamples);
        double sortP95 = Percentile95(sortSamples);
        double avgCpu = cpuSamples.Count == 0 ? 0d : cpuSamples.Average();
        ulong avgRss = rssSamples.Count == 0 ? 0UL : (ulong)rssSamples.Average(static value => (double)value);
        double cpuBudgetPct = gates.CpuBudgetPct <= 0d ? CpuBudgetPct : gates.CpuBudgetPct;
        ulong rssBudgetBytes = gates.RssBudgetBytes == 0UL ? 256UL * 1024UL * 1024UL : gates.RssBudgetBytes;
        bool budgetPassed = avgCpu < cpuBudgetPct && avgRss < rssBudgetBytes;
        BenchmarkInteractionProbeP95 interaction = new()
        {
            FilterP95Ms = Math.Round(Percentile95(filterSamples), 4),
            SortP95Ms = Math.Round(sortP95, 4),
            SelectionP95Ms = Math.Round(Percentile95(selectionSamples), 4),
            BatchP95Ms = Math.Round(Percentile95(uiBatchSamples), 4),
            PlotP95Ms = Math.Round(Percentile95(plotSamples), 4),
        };
        double? minSpeedup = NormalizeOptionalPositive(gates.MinSpeedupMultiplier);
        double? maxP95 = NormalizeOptionalPositive(gates.MaxP95Ms);
        bool baselineMetadataMatched = BaselineMetadataMatches(gates.Baseline, gates.Host, gates.MeasurementOrigin);
        BenchmarkSummary? compatibleBaseline = baselineMetadataMatched ? gates.Baseline : null;
        BenchmarkComparison? comparison = BuildComparison(compatibleBaseline, tickP95, sortP95, minSpeedup);
        BenchmarkInteractionProbeP95? interactionProbe = NormalizeInteractionProbe(interaction);
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

    private static bool IsWinUiHost(BenchmarkGateOptions gates)
    {
        return string.Equals(gates.Host, "winui", StringComparison.OrdinalIgnoreCase)
               || string.Equals(gates.MeasurementOrigin, WinUiMeasurementOrigin, StringComparison.OrdinalIgnoreCase)
               || string.Equals(gates.MeasurementOrigin, "winui_cli", StringComparison.OrdinalIgnoreCase);
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

    private static void AppendTrend(RuntimeSnapshot snapshot, double[] cpuTrend, double[] memoryTrend, ref int trendIndex)
    {
        cpuTrend[trendIndex] = snapshot.System.CpuPct.GetValueOrDefault();
        ulong totalMemoryBytes = snapshot.System.MemoryTotalBytes.GetValueOrDefault();
        memoryTrend[trendIndex] = totalMemoryBytes == 0
            ? 0
            : snapshot.System.MemoryUsedBytes.GetValueOrDefault() * 100d / totalMemoryBytes;
        trendIndex = (trendIndex + 1) % cpuTrend.Length;
    }

    private static string BuildProbeFilter(RuntimeSnapshot snapshot, int index)
    {
        if (index % 2 != 0)
        {
            return string.Empty;
        }

        string? name = snapshot.Rows.FirstOrDefault(static row => !string.IsNullOrWhiteSpace(row.Name))?.Name;
        if (string.IsNullOrWhiteSpace(name))
        {
            return "__batcave_probe_no_match__";
        }

        return name.Length <= 3 ? name : name[..3];
    }

    private sealed record BenchmarkRowAdapter
    {
        public BenchmarkRowAdapter(ProcessSample sample)
        {
            Identity = sample.Identity();
            Name = string.IsNullOrWhiteSpace(sample.Name) ? $"PID {sample.Pid}" : sample.Name;
            PidText = sample.Pid.ToString(System.Globalization.CultureInfo.InvariantCulture);
            CpuText = sample.CpuPct.ToString("0.0", System.Globalization.CultureInfo.InvariantCulture) + "%";
            MemoryText = FormatBytes(sample.MemoryBytes);
            DiskText = FormatBytes(sample.DiskBps) + "/s";
            OtherIoText = FormatBytes(sample.OtherIoBps) + "/s";
        }

        public ProcessIdentity Identity { get; }
        public string Name { get; }
        public string PidText { get; }
        public string CpuText { get; }
        public string MemoryText { get; }
        public string DiskText { get; }
        public string OtherIoText { get; }
    }

    private sealed class BenchmarkPersistenceStore : IRuntimePersistenceStore
    {
        public string BaseDirectory { get; } = Path.GetTempPath();

        public RuntimeSettings? LoadSettings() => null;

        public Task SaveSettingsAsync(RuntimeSettings settings, CancellationToken ct) => Task.CompletedTask;

        public WarmCache? LoadWarmCache() => null;

        public Task SaveWarmCacheAsync(WarmCache cache, CancellationToken ct) => Task.CompletedTask;

        public Task AppendDiagnosticAsync(string category, object payload, CancellationToken ct) => Task.CompletedTask;
    }

    private static string FormatBytes(ulong bytes)
    {
        string[] units = ["B", "KB", "MB", "GB", "TB"];
        double value = bytes;
        int unit = 0;
        while (value >= 1024d && unit < units.Length - 1)
        {
            value /= 1024d;
            unit++;
        }

        return unit == 0
            ? $"{value:0} {units[unit]}"
            : $"{value:0.0} {units[unit]}";
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
