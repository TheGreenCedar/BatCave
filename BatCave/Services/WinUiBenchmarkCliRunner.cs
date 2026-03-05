using BatCave.Core.Domain;
using BatCave.Core.Runtime;
using BatCave.Core.Serialization;
using BatCave.ViewModels;
using Microsoft.Extensions.DependencyInjection;
using System;
using System.Diagnostics;
using System.Globalization;
using System.IO;
using System.Text.Json;
using System.Threading;
using System.Threading.Tasks;

namespace BatCave.Services;

internal static class WinUiBenchmarkCliRunner
{
    private const int FilterDebounceSettleMs = 180;

    private static readonly SortColumn[] SortColumns =
    [
        SortColumn.CpuPct,
        SortColumn.RssBytes,
        SortColumn.DiskBps,
        SortColumn.IoReadBps,
        SortColumn.IoWriteBps,
        SortColumn.OtherIoBps,
        SortColumn.Threads,
        SortColumn.Handles,
        SortColumn.Pid,
        SortColumn.Name,
    ];

    public static bool IsBenchmarkCommand(string[] args)
    {
        foreach (string arg in args)
        {
            if (string.Equals(arg, "--benchmark", StringComparison.OrdinalIgnoreCase))
            {
                return true;
            }
        }

        return false;
    }

    public static async Task<int> ExecuteAsync(IServiceProvider services, string[] args, CancellationToken ct)
    {
        if (!TryParseArgs(args, out WinUiBenchmarkCliOptions options, out string? error))
        {
            Console.Error.WriteLine(error);
            return 2;
        }

        BenchmarkSummary summary = await RunAsync(services, options, ct).ConfigureAwait(false);
        Console.WriteLine(JsonSerializer.Serialize(summary, JsonDefaults.SnakeCase));
        return options.Strict && !summary.StrictPassed ? 2 : 0;
    }

    private static async Task<BenchmarkSummary> RunAsync(
        IServiceProvider services,
        WinUiBenchmarkCliOptions options,
        CancellationToken ct)
    {
        MonitoringRuntime runtime = services.GetRequiredService<MonitoringRuntime>();
        IRuntimeEventGateway runtimeEventGateway = services.GetRequiredService<IRuntimeEventGateway>();
        MonitoringShellViewModel viewModel = services.GetRequiredService<MonitoringShellViewModel>();

        await viewModel.BootstrapAsync(ct).ConfigureAwait(false);
        viewModel.ResetInteractionProbeRecorder();
        await DriveInteractionProbeWorkloadAsync(runtime, runtimeEventGateway, viewModel, options.Ticks, options.SleepMs, ct)
            .ConfigureAwait(false);

        InteractionProbeP95Snapshot interactionSnapshot = viewModel.SnapshotInteractionProbeP95();
        BenchmarkInteractionProbeP95 interactionProbeP95 = new()
        {
            FilterP95Ms = interactionSnapshot.FilterApplyMs,
            SortP95Ms = interactionSnapshot.SortCompleteMs,
            SelectionP95Ms = interactionSnapshot.SelectionSettleMs,
            BatchP95Ms = interactionSnapshot.UiBatchMs,
            PlotP95Ms = interactionSnapshot.PlotRefreshMs,
        };

        BenchmarkGateOptions gateOptions = options.GateOptions with
        {
            InteractionProbeP95 = interactionProbeP95,
            RequireInteractionProbeSpeedup = true,
        };

        return BenchmarkRunner.Run(options.Ticks, options.SleepMs, ct, gateOptions);
    }

    private static async Task DriveInteractionProbeWorkloadAsync(
        MonitoringRuntime runtime,
        IRuntimeEventGateway runtimeEventGateway,
        MonitoringShellViewModel viewModel,
        int ticks,
        int sleepMs,
        CancellationToken ct)
    {
        int iterationCount = Math.Max(1, ticks);
        int warmupTicks = Math.Max(4, Math.Min(16, iterationCount / 5));

        for (int index = 0; index < warmupTicks; index++)
        {
            PublishTelemetryTick(runtime, runtimeEventGateway);
        }

        if (sleepMs > 0)
        {
            await Task.Delay(sleepMs, ct).ConfigureAwait(false);
        }

        for (int index = 0; index < iterationCount; index++)
        {
            ct.ThrowIfCancellationRequested();
            PublishTelemetryTick(runtime, runtimeEventGateway);

            SortColumn sortColumn = SortColumns[index % SortColumns.Length];
            viewModel.ChangeSort(sortColumn);

            viewModel.FilterText = ResolveFilterText(viewModel, index);
            await Task.Delay(FilterDebounceSettleMs, ct).ConfigureAwait(false);

            if (TryGetVisibleRowForSelection(viewModel, index, out ProcessRowViewState rowState))
            {
                long selectionStartedAt = Stopwatch.GetTimestamp();
                await viewModel.SelectRowAsync(rowState.Sample, ct).ConfigureAwait(false);
                viewModel.RecordSelectionSettleProbe(Stopwatch.GetTimestamp() - selectionStartedAt);
            }

            long plotStartedAt = Stopwatch.GetTimestamp();
            viewModel.MetricFocus = (DetailMetricFocus)(index % 5);
            viewModel.RecordPlotRefreshProbe(Stopwatch.GetTimestamp() - plotStartedAt);

            if (sleepMs > 0)
            {
                await Task.Delay(sleepMs, ct).ConfigureAwait(false);
            }
        }

        await Task.Delay(50, ct).ConfigureAwait(false);
    }

    private static void PublishTelemetryTick(
        MonitoringRuntime runtime,
        IRuntimeEventGateway runtimeEventGateway)
    {
        TickOutcome outcome = runtime.Tick(0);
        runtimeEventGateway.Publish(outcome);
    }

    private static string ResolveFilterText(MonitoringShellViewModel viewModel, int index)
    {
        if (index % 2 == 0)
        {
            return string.Empty;
        }

        if (TryGetVisibleRowForSelection(viewModel, index, out ProcessRowViewState rowState)
            && !string.IsNullOrWhiteSpace(rowState.Name))
        {
            string name = rowState.Name.Trim();
            if (name.Length >= 3)
            {
                return name[..3];
            }

            return name;
        }

        return "svc";
    }

    private static bool TryGetVisibleRowForSelection(
        MonitoringShellViewModel viewModel,
        int index,
        out ProcessRowViewState rowState)
    {
        rowState = null!;
        int visibleCount = viewModel.VisibleRows.Count;
        if (visibleCount <= 0)
        {
            return false;
        }

        int targetIndex = index % Math.Min(2, visibleCount);
        if (viewModel.VisibleRows[targetIndex] is not ProcessRowViewState typed)
        {
            return false;
        }

        rowState = typed;
        return true;
    }

    private static bool TryParseArgs(
        string[] args,
        out WinUiBenchmarkCliOptions options,
        out string? error)
    {
        int ticks = 120;
        int sleepMs = 1000;
        bool strict = false;
        string? baselineJsonPath = null;
        double? minSpeedupMultiplier = null;
        double? maxP95Ms = null;
        error = null;

        for (int index = 0; index < args.Length; index++)
        {
            if (!TryParseArgument(
                    args,
                    ref index,
                    ref ticks,
                    ref sleepMs,
                    ref strict,
                    ref baselineJsonPath,
                    ref minSpeedupMultiplier,
                    ref maxP95Ms,
                    out error))
            {
                options = new WinUiBenchmarkCliOptions();
                return false;
            }
        }

        if (minSpeedupMultiplier.HasValue && string.IsNullOrWhiteSpace(baselineJsonPath))
        {
            error = "--min-speedup-multiplier requires --baseline-json.";
            options = new WinUiBenchmarkCliOptions();
            return false;
        }

        BenchmarkSummary? baseline = null;
        if (!string.IsNullOrWhiteSpace(baselineJsonPath)
            && !TryLoadBaselineSummary(baselineJsonPath, out baseline, out error))
        {
            options = new WinUiBenchmarkCliOptions();
            return false;
        }

        options = new WinUiBenchmarkCliOptions
        {
            Ticks = ticks,
            SleepMs = sleepMs,
            Strict = strict,
            GateOptions = new BenchmarkGateOptions
            {
                Baseline = baseline,
                MinSpeedupMultiplier = minSpeedupMultiplier,
                MaxP95Ms = maxP95Ms,
            },
        };

        return true;
    }

    private static bool TryParseArgument(
        string[] args,
        ref int index,
        ref int ticks,
        ref int sleepMs,
        ref bool strict,
        ref string? baselineJsonPath,
        ref double? minSpeedupMultiplier,
        ref double? maxP95Ms,
        out string? error)
    {
        error = null;
        string argument = args[index];

        switch (argument)
        {
            case "--benchmark":
                return true;
            case "--strict":
                strict = true;
                return true;
            case "--ticks":
                if (!TryReadIntValue(args, ref index, out int parsedTicks))
                {
                    error = "Missing or invalid value for --ticks.";
                    return false;
                }

                ticks = parsedTicks;
                return true;
            case "--sleep-ms":
                if (!TryReadIntValue(args, ref index, out int parsedSleepMs))
                {
                    error = "Missing or invalid value for --sleep-ms.";
                    return false;
                }

                sleepMs = parsedSleepMs;
                return true;
            case "--baseline-json":
                if (!TryReadStringValue(args, ref index, out string parsedBaselinePath))
                {
                    error = "Missing value for --baseline-json.";
                    return false;
                }

                baselineJsonPath = parsedBaselinePath;
                return true;
            case "--min-speedup-multiplier":
                if (!TryReadDoubleValue(args, ref index, out double parsedMinSpeedupMultiplier)
                    || parsedMinSpeedupMultiplier <= 0d)
                {
                    error = "Missing or invalid value for --min-speedup-multiplier (must be > 0).";
                    return false;
                }

                minSpeedupMultiplier = parsedMinSpeedupMultiplier;
                return true;
            case "--max-p95-ms":
                if (!TryReadDoubleValue(args, ref index, out double parsedMaxP95Ms)
                    || parsedMaxP95Ms <= 0d)
                {
                    error = "Missing or invalid value for --max-p95-ms (must be > 0).";
                    return false;
                }

                maxP95Ms = parsedMaxP95Ms;
                return true;
            default:
                error = $"Unknown argument: {argument}";
                return false;
        }
    }

    private static bool TryReadIntValue(string[] args, ref int index, out int value)
    {
        value = 0;
        if (!TryReadStringValue(args, ref index, out string rawValue))
        {
            return false;
        }

        return int.TryParse(rawValue, NumberStyles.Integer, CultureInfo.InvariantCulture, out value);
    }

    private static bool TryReadDoubleValue(string[] args, ref int index, out double value)
    {
        value = 0;
        if (!TryReadStringValue(args, ref index, out string rawValue))
        {
            return false;
        }

        return double.TryParse(rawValue, NumberStyles.Float | NumberStyles.AllowThousands, CultureInfo.InvariantCulture, out value);
    }

    private static bool TryReadStringValue(string[] args, ref int index, out string value)
    {
        value = string.Empty;
        int valueIndex = index + 1;
        if (valueIndex >= args.Length)
        {
            return false;
        }

        value = args[valueIndex];
        index = valueIndex;
        return true;
    }

    private static bool TryLoadBaselineSummary(
        string baselineJsonPath,
        out BenchmarkSummary? baseline,
        out string? error)
    {
        baseline = null;
        error = null;

        if (!File.Exists(baselineJsonPath))
        {
            error = $"Baseline file not found: {baselineJsonPath}";
            return false;
        }

        try
        {
            string payload = File.ReadAllText(baselineJsonPath);
            baseline = JsonSerializer.Deserialize<BenchmarkSummary>(payload, JsonDefaults.SnakeCase);
            if (baseline is null)
            {
                error = $"Baseline JSON did not contain a benchmark summary: {baselineJsonPath}";
                return false;
            }
        }
        catch (Exception ex)
        {
            error = $"Failed to read baseline JSON '{baselineJsonPath}': {ex.GetType().Name}: {ex.Message}";
            return false;
        }

        return true;
    }

    private sealed record WinUiBenchmarkCliOptions
    {
        public int Ticks { get; init; } = 120;

        public int SleepMs { get; init; } = 1000;

        public bool Strict { get; init; }

        public BenchmarkGateOptions GateOptions { get; init; } = new();
    }
}
