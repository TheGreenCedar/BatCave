using System.Diagnostics;
using BatCave.Core.Abstractions;
using BatCave.Core.Collector;
using BatCave.Core.Domain;
using BatCave.Core.Pipeline;
using BatCave.Core.Sort;
using BatCave.Core.State;

namespace BatCave.Core.Runtime;

public sealed record BenchmarkSummary
{
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
}

public static class BenchmarkRunner
{
    public const double CpuBudgetPct = 1.0;
    public const ulong RssBudgetBytes = 150UL * 1024UL * 1024UL;

    public static BenchmarkSummary Run(int ticks, int sleepMs, CancellationToken ct)
    {
        int safeTicks = Math.Max(0, ticks);
        int safeSleepMs = Math.Max(0, sleepMs);

        Stopwatch startupStopwatch = Stopwatch.StartNew();
        using MonitoringRuntime runtime = new(
            new DefaultProcessCollectorFactory(),
            new DeltaTelemetryPipeline(),
            new InMemoryStateStore(),
            new IncrementalSortIndexEngine(),
            new BenchmarkPersistenceStore());
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

        return new BenchmarkSummary
        {
            Ticks = safeTicks,
            SleepMs = safeSleepMs,
            StartupMs = startupStopwatch.ElapsedMilliseconds,
            TickP95Ms = PercentileMath.Percentile95(tickSamples),
            SortP95Ms = PercentileMath.Percentile95(sortSamples),
            AvgAppCpuPct = avgCpu,
            AvgAppRssBytes = avgRss,
            BudgetPassed = budgetPassed,
            CpuBudgetPct = CpuBudgetPct,
            RssBudgetBytes = RssBudgetBytes,
        };
    }

    private static ulong AddSaturating(ulong left, ulong right)
    {
        ulong sum = left + right;
        return sum < left ? ulong.MaxValue : sum;
    }

    private sealed class BenchmarkPersistenceStore : IPersistenceStore
    {
        public string BaseDirectory => Path.GetTempPath();

        public UserSettings? LoadSettings()
        {
            return new UserSettings
            {
                AdminMode = false,
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
    }
}
