using BatCave.App.Presentation;
using BatCave.Runtime.Benchmarking;
using BatCave.Runtime.Collectors;
using BatCave.Runtime.Contracts;
using BatCave.Runtime.Persistence;
using BatCave.Runtime.Store;
using Microsoft.UI.Dispatching;
using System;
using System.Collections.Generic;
using System.Diagnostics;
using System.Linq;
using System.Threading;
using System.Threading.Tasks;

namespace BatCave.App.Benchmarking;

public sealed class ShellWinUiBenchmarkRunner : IWinUiBenchmarkRunner
{
    private static readonly string[] SortColumns =
    [
        nameof(SortColumn.CpuPct),
        nameof(SortColumn.MemoryBytes),
        nameof(SortColumn.DiskBps),
        nameof(SortColumn.OtherIoBps),
        nameof(SortColumn.Threads),
        nameof(SortColumn.Handles),
        nameof(SortColumn.Pid),
        nameof(SortColumn.Name),
    ];

    public async ValueTask<BenchmarkSummary> RunAsync(int ticks, int sleepMs, BenchmarkGateOptions gates, CancellationToken ct)
    {
        ticks = Math.Max(1, ticks);
        sleepMs = Math.Max(0, sleepMs);
        List<double> tickSamples = new(ticks);
        List<double> filterSamples = new(ticks);
        List<double> sortSamples = new(ticks);
        List<double> selectionSamples = new(ticks);
        List<double> uiBatchSamples = new(ticks);
        List<double> plotSamples = new(ticks);
        List<double> cpuSamples = new(ticks);
        List<ulong> rssSamples = new(ticks);
        DispatcherQueue? dispatcherQueue = DispatcherQueue.GetForCurrentThread();
        using Process currentProcess = Process.GetCurrentProcess();
        using WindowsSystemMetricsCollector systemCollector = new();
        await using RuntimeStore store = new(
            new WindowsProcessCollector(),
            systemCollector,
            new BenchmarkPersistenceStore(),
            new RuntimeStoreOptions
            {
                RuntimeLoopEnabled = true,
                TickInterval = TimeSpan.FromHours(1),
                SubscriberBufferCapacity = 2,
                WarmCacheWriteIntervalTicks = int.MaxValue,
                DefaultSettings = new RuntimeSettings
                {
                    Paused = true,
                    MetricWindowSeconds = 60,
                },
            });
        await using ShellViewModel viewModel = new(store);
        if (dispatcherQueue is not null)
        {
            viewModel.AttachDispatcherQueue(dispatcherQueue);
        }

        Stopwatch startup = Stopwatch.StartNew();
        await store.StartAsync(ct).ConfigureAwait(true);
        viewModel.Start();
        await viewModel.RefreshCommand.ExecuteAsync(null);
        await WaitForViewModelSnapshotAsync(viewModel, store.GetSnapshot().Seq, ct).ConfigureAwait(true);
        startup.Stop();

        TimeSpan previousCpu = currentProcess.TotalProcessorTime;
        long previousStamp = Stopwatch.GetTimestamp();

        for (int index = 0; index < ticks; index++)
        {
            ct.ThrowIfCancellationRequested();

            Stopwatch tick = Stopwatch.StartNew();
            await viewModel.RefreshCommand.ExecuteAsync(null);
            RuntimeSnapshot snapshot = store.GetSnapshot();
            await WaitForViewModelSnapshotAsync(viewModel, snapshot.Seq, ct).ConfigureAwait(true);
            tick.Stop();
            tickSamples.Add(tick.Elapsed.TotalMilliseconds);

            Stopwatch filter = Stopwatch.StartNew();
            string filterText = BuildProbeFilter(snapshot, index);
            viewModel.FilterText = filterText;
            snapshot = await WaitForQueryAsync(store, filterText, ct).ConfigureAwait(true);
            await WaitForViewModelSnapshotAsync(viewModel, snapshot.Seq, ct).ConfigureAwait(true);
            filter.Stop();
            filterSamples.Add(filter.Elapsed.TotalMilliseconds);

            Stopwatch sort = Stopwatch.StartNew();
            await viewModel.SortCommand.ExecuteAsync(SortColumns[index % SortColumns.Length]);
            snapshot = store.GetSnapshot();
            await WaitForViewModelSnapshotAsync(viewModel, snapshot.Seq, ct).ConfigureAwait(true);
            sort.Stop();
            sortSamples.Add(sort.Elapsed.TotalMilliseconds);

            Stopwatch selection = Stopwatch.StartNew();
            viewModel.SelectedRow = viewModel.Rows.Count == 0
                ? null
                : viewModel.Rows[Math.Min(index, viewModel.Rows.Count - 1)];
            selection.Stop();
            selectionSamples.Add(selection.Elapsed.TotalMilliseconds);

            Stopwatch uiBatch = Stopwatch.StartNew();
            _ = viewModel.Rows
                .Take(200)
                .Select(static row => (row.Name, row.PidText, row.CpuText, row.MemoryText, row.DiskText, row.OtherIoText))
                .ToArray();
            uiBatch.Stop();
            uiBatchSamples.Add(uiBatch.Elapsed.TotalMilliseconds);

            Stopwatch plot = Stopwatch.StartNew();
            _ = viewModel.CpuTrendValues.Sum() + viewModel.MemoryTrendValues.Sum();
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
                await Task.Delay(sleepMs, ct).ConfigureAwait(true);
            }
        }

        BenchmarkInteractionProbeP95 interaction = new()
        {
            FilterP95Ms = Math.Round(Percentile95(filterSamples), 4),
            SortP95Ms = Math.Round(Percentile95(sortSamples), 4),
            SelectionP95Ms = Math.Round(Percentile95(selectionSamples), 4),
            BatchP95Ms = Math.Round(Percentile95(uiBatchSamples), 4),
            PlotP95Ms = Math.Round(Percentile95(plotSamples), 4),
        };

        BenchmarkMeasurement measurement = new()
        {
            Host = "winui",
            MeasurementOrigin = BenchmarkRunner.WinUiMeasurementOrigin,
            UsesAttachedDispatcher = dispatcherQueue is not null,
            Ticks = ticks,
            SleepMs = sleepMs,
            StartupMs = startup.ElapsedMilliseconds,
            TickP95Ms = Percentile95(tickSamples),
            SortP95Ms = Percentile95(sortSamples),
            AvgAppCpuPct = cpuSamples.Count == 0 ? 0d : cpuSamples.Average(),
            AvgAppRssBytes = rssSamples.Count == 0 ? 0UL : (ulong)rssSamples.Average(static value => (double)value),
            InteractionProbeP95 = interaction,
        };

        return BenchmarkRunner.CreateSummary(measurement, gates with
        {
            Host = "winui",
            MeasurementOrigin = BenchmarkRunner.WinUiMeasurementOrigin,
            UsesAttachedDispatcher = dispatcherQueue is not null,
            InteractionProbeP95 = interaction,
            RequireInteractionProbeSpeedup = gates.RequireInteractionProbeSpeedup,
        });
    }

    private static async Task WaitForViewModelSnapshotAsync(ShellViewModel viewModel, ulong seq, CancellationToken ct)
    {
        Stopwatch timeout = Stopwatch.StartNew();
        while (viewModel.SnapshotSeq < seq)
        {
            ct.ThrowIfCancellationRequested();
            if (timeout.Elapsed > TimeSpan.FromSeconds(5))
            {
                throw new TimeoutException($"Timed out waiting for WinUI shell snapshot {seq}.");
            }

            await Task.Delay(1, ct).ConfigureAwait(true);
        }
    }

    private static async Task<RuntimeSnapshot> WaitForQueryAsync(RuntimeStore store, string filterText, CancellationToken ct)
    {
        Stopwatch timeout = Stopwatch.StartNew();
        while (true)
        {
            RuntimeSnapshot snapshot = store.GetSnapshot();
            if (string.Equals(snapshot.Settings.Query.FilterText, filterText, StringComparison.Ordinal))
            {
                return snapshot;
            }

            ct.ThrowIfCancellationRequested();
            if (timeout.Elapsed > TimeSpan.FromSeconds(5))
            {
                throw new TimeoutException($"Timed out waiting for filter '{filterText}'.");
            }

            await Task.Delay(1, ct).ConfigureAwait(true);
        }
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

    private sealed class BenchmarkPersistenceStore : IRuntimePersistenceStore
    {
        public string BaseDirectory { get; } = Path.GetTempPath();

        public RuntimeSettings? LoadSettings() => null;

        public Task SaveSettingsAsync(RuntimeSettings settings, CancellationToken ct) => Task.CompletedTask;

        public WarmCache? LoadWarmCache() => null;

        public Task SaveWarmCacheAsync(WarmCache cache, CancellationToken ct) => Task.CompletedTask;

        public Task AppendDiagnosticAsync(string category, object payload, CancellationToken ct) => Task.CompletedTask;
    }
}
