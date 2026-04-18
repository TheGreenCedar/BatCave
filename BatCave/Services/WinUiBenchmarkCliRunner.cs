using BatCave.Core.Domain;
using BatCave.Core.Runtime;
using BatCave.Core.Serialization;
using BatCave.ViewModels;
using Microsoft.Extensions.DependencyInjection;
using Microsoft.UI.Dispatching;
using System.CommandLine;
using System.CommandLine.Parsing;
using System;
using System.Collections.Generic;
using System.Diagnostics;
using System.Globalization;
using System.IO;
using System.Linq;
using System.Reflection;
using System.Text.Json;
using System.Threading;
using System.Threading.Tasks;

namespace BatCave.Services;

internal static class WinUiBenchmarkCliRunner
{
    private const int FilterDebounceSettleMs = 180;
    private static readonly Option<bool> BenchmarkOption = new("--benchmark");
    private static readonly Option<bool> StrictOption = new("--strict");
    private static readonly Option<string?> TicksOption = new("--ticks");
    private static readonly Option<string?> SleepMsOption = new("--sleep-ms");
    private static readonly Option<string?> BaselineJsonOption = new("--baseline-json");
    private static readonly Option<string?> MinSpeedupMultiplierOption = new("--min-speedup-multiplier");
    private static readonly Option<string?> MaxP95MsOption = new("--max-p95-ms");
    private static readonly RootCommand BenchmarkRootCommand = CreateRootCommand();

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
        if (!TryCreateOptions(args, out WinUiBenchmarkCliOptions options, out IReadOnlyList<string> errors))
        {
            foreach (string error in errors)
            {
                Console.Error.WriteLine(error);
            }

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
        DispatcherQueue dispatcherQueue = ResolveShellDispatcherQueue(services);

        Stopwatch startupStopwatch = Stopwatch.StartNew();
        await runtime.InitializeAsync(ct);
        await viewModel.BootstrapAsync(ct);
        viewModel.ResetInteractionProbeRecorder();
        startupStopwatch.Stop();

        LiveShellBenchmarkMeasurement liveShellMeasurement = await Task.Run(
            () => DriveLiveShellWorkloadAsync(
                dispatcherQueue,
                runtime,
                runtimeEventGateway,
                viewModel,
                options.Ticks,
                options.SleepMs,
                ct),
            ct).ConfigureAwait(false);

        InteractionProbeP95Snapshot interactionSnapshot = await InvokeOnDispatcherAsync(
            dispatcherQueue,
            viewModel.SnapshotInteractionProbeP95,
            ct).ConfigureAwait(false);
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

        return BenchmarkRunner.CreateSummary(
            new BenchmarkMeasurement
            {
                Host = "winui",
                MeasurementOrigin = "live_shell",
                UsesAttachedDispatcher = true,
                Ticks = liveShellMeasurement.Ticks,
                SleepMs = liveShellMeasurement.SleepMs,
                StartupMs = startupStopwatch.ElapsedMilliseconds,
                TickP95Ms = liveShellMeasurement.TickP95Ms,
                SortP95Ms = liveShellMeasurement.SortP95Ms,
                AvgAppCpuPct = liveShellMeasurement.AvgAppCpuPct,
                AvgAppRssBytes = liveShellMeasurement.AvgAppRssBytes,
            },
            gateOptions);
    }

    private static async Task<LiveShellBenchmarkMeasurement> DriveLiveShellWorkloadAsync(
        DispatcherQueue dispatcherQueue,
        MonitoringRuntime runtime,
        IRuntimeEventGateway runtimeEventGateway,
        MonitoringShellViewModel viewModel,
        int ticks,
        int sleepMs,
        CancellationToken ct)
    {
        int iterationCount = Math.Max(1, ticks);
        int warmupTicks = Math.Max(4, Math.Min(16, iterationCount / 5));
        List<double> tickSamples = new(iterationCount);
        List<double> sortSamples = new(iterationCount);
        double cpuAccum = 0d;
        ulong rssAccum = 0UL;

        for (int index = 0; index < warmupTicks; index++)
        {
            await PublishTelemetryTickAsync(dispatcherQueue, runtime, runtimeEventGateway, ct).ConfigureAwait(false);
        }

        if (sleepMs > 0)
        {
            await Task.Delay(sleepMs, ct).ConfigureAwait(false);
        }

        for (int index = 0; index < iterationCount; index++)
        {
            ct.ThrowIfCancellationRequested();
            tickSamples.Add(await PublishTelemetryTickAsync(dispatcherQueue, runtime, runtimeEventGateway, ct).ConfigureAwait(false));

            SortColumn sortColumn = SortColumns[index % SortColumns.Length];
            sortSamples.Add(await MeasureSortInteractionAsync(dispatcherQueue, viewModel, sortColumn, ct).ConfigureAwait(false));

            await InvokeOnDispatcherAsync(
                dispatcherQueue,
                () =>
                {
                    viewModel.FilterText = ResolveFilterText(viewModel, index);
                },
                ct).ConfigureAwait(false);
            await Task.Delay(FilterDebounceSettleMs, ct).ConfigureAwait(false);
            await AwaitDispatcherIdleAsync(dispatcherQueue, ct).ConfigureAwait(false);

            await MeasureSelectionInteractionAsync(dispatcherQueue, viewModel, index, ct).ConfigureAwait(false);

            await MeasurePlotInteractionAsync(dispatcherQueue, viewModel, index, ct).ConfigureAwait(false);

            RuntimeHealth health = runtime.GetRuntimeHealth();
            cpuAccum += health.AppCpuPct;
            rssAccum = AddSaturating(rssAccum, health.AppRssBytes);

            if (sleepMs > 0)
            {
                await Task.Delay(sleepMs, ct).ConfigureAwait(false);
            }
        }

        await AwaitDispatcherIdleAsync(dispatcherQueue, ct).ConfigureAwait(false);

        return new LiveShellBenchmarkMeasurement
        {
            Ticks = iterationCount,
            SleepMs = Math.Max(0, sleepMs),
            TickP95Ms = Percentile95(tickSamples),
            SortP95Ms = Percentile95(sortSamples),
            AvgAppCpuPct = iterationCount > 0 ? cpuAccum / iterationCount : 0d,
            AvgAppRssBytes = iterationCount > 0 ? rssAccum / (ulong)iterationCount : 0UL,
        };
    }

    private static async Task<double> PublishTelemetryTickAsync(
        DispatcherQueue dispatcherQueue,
        MonitoringRuntime runtime,
        IRuntimeEventGateway runtimeEventGateway,
        CancellationToken ct)
    {
        Stopwatch stopwatch = Stopwatch.StartNew();
        TickOutcome outcome = runtime.Tick(0);
        Task telemetryDrain = ShouldAwaitTelemetryDrain(outcome)
            ? WaitForTelemetryDeltaAsync(runtimeEventGateway, outcome.Delta.Seq, ct)
            : Task.CompletedTask;

        runtimeEventGateway.Publish(outcome);
        await telemetryDrain.ConfigureAwait(false);
        await AwaitDispatcherIdleAsync(dispatcherQueue, ct).ConfigureAwait(false);
        stopwatch.Stop();
        return stopwatch.Elapsed.TotalMilliseconds;
    }

    private static bool ShouldAwaitTelemetryDrain(TickOutcome outcome)
    {
        return outcome.EmitTelemetryDelta
               || outcome.Delta.Upserts.Count > 0
               || outcome.Delta.Exits.Count > 0;
    }

    private static async Task<double> MeasureSortInteractionAsync(
        DispatcherQueue dispatcherQueue,
        MonitoringShellViewModel viewModel,
        SortColumn sortColumn,
        CancellationToken ct)
    {
        long startedAt = Stopwatch.GetTimestamp();
        await InvokeOnDispatcherAsync(dispatcherQueue, () => viewModel.ChangeSort(sortColumn), ct).ConfigureAwait(false);
        await AwaitDispatcherIdleAsync(dispatcherQueue, ct).ConfigureAwait(false);
        return ElapsedMilliseconds(startedAt);
    }

    private static async Task MeasureSelectionInteractionAsync(
        DispatcherQueue dispatcherQueue,
        MonitoringShellViewModel viewModel,
        int index,
        CancellationToken ct)
    {
        await InvokeOnDispatcherAsync(
            dispatcherQueue,
            async () =>
            {
                if (!TryGetVisibleRowForSelection(viewModel, index, out ProcessRowViewState rowState))
                {
                    return;
                }

                long selectionStartedAt = Stopwatch.GetTimestamp();
                await viewModel.SelectRowAsync(rowState.Sample, ct);
                viewModel.RecordSelectionSettleProbe(Stopwatch.GetTimestamp() - selectionStartedAt);
            },
            ct).ConfigureAwait(false);
        await AwaitDispatcherIdleAsync(dispatcherQueue, ct).ConfigureAwait(false);
    }

    private static async Task MeasurePlotInteractionAsync(
        DispatcherQueue dispatcherQueue,
        MonitoringShellViewModel viewModel,
        int index,
        CancellationToken ct)
    {
        await InvokeOnDispatcherAsync(
            dispatcherQueue,
            () =>
            {
                long plotStartedAt = Stopwatch.GetTimestamp();
                viewModel.MetricFocus = (DetailMetricFocus)(index % 5);
                viewModel.RecordPlotRefreshProbe(Stopwatch.GetTimestamp() - plotStartedAt);
            },
            ct).ConfigureAwait(false);
        await AwaitDispatcherIdleAsync(dispatcherQueue, ct).ConfigureAwait(false);
    }

    private static Task AwaitDispatcherIdleAsync(
        DispatcherQueue dispatcherQueue,
        CancellationToken ct)
    {
        return InvokeOnDispatcherAsync(dispatcherQueue, static () => { }, ct);
    }

    private static async Task InvokeOnDispatcherAsync(
        DispatcherQueue dispatcherQueue,
        Func<Task> callback,
        CancellationToken ct)
    {
        if (dispatcherQueue.HasThreadAccess)
        {
            ct.ThrowIfCancellationRequested();
            await callback().ConfigureAwait(false);
            return;
        }

        TaskCompletionSource completion = new(TaskCreationOptions.RunContinuationsAsynchronously);
        using CancellationTokenRegistration cancellationRegistration = ct.Register(static state =>
        {
            ((TaskCompletionSource)state!).TrySetCanceled();
        }, completion);

        if (!dispatcherQueue.TryEnqueue(async () =>
            {
                try
                {
                    await callback().ConfigureAwait(false);
                    completion.TrySetResult();
                }
                catch (OperationCanceledException)
                {
                    completion.TrySetCanceled();
                }
                catch (Exception ex)
                {
                    completion.TrySetException(ex);
                }
            }))
        {
            throw new InvalidOperationException("Failed to enqueue WinUI benchmark work on the dispatcher queue.");
        }

        await completion.Task.ConfigureAwait(false);
    }

    private static async Task InvokeOnDispatcherAsync(
        DispatcherQueue dispatcherQueue,
        Action callback,
        CancellationToken ct)
    {
        await InvokeOnDispatcherAsync(
            dispatcherQueue,
            () =>
            {
                callback();
                return Task.CompletedTask;
            },
            ct).ConfigureAwait(false);
    }

    private static async Task<T> InvokeOnDispatcherAsync<T>(
        DispatcherQueue dispatcherQueue,
        Func<T> callback,
        CancellationToken ct)
    {
        T result = default!;
        await InvokeOnDispatcherAsync(
            dispatcherQueue,
            () =>
            {
                result = callback();
                return Task.CompletedTask;
            },
            ct).ConfigureAwait(false);
        return result;
    }

    private static Task WaitForTelemetryDeltaAsync(
        IRuntimeEventGateway runtimeEventGateway,
        ulong minimumSequence,
        CancellationToken ct)
    {
        TaskCompletionSource completion = new(TaskCreationOptions.RunContinuationsAsynchronously);
        EventHandler<ProcessDeltaBatch>? handler = null;
        CancellationTokenRegistration cancellationRegistration = default;
        handler = (_, delta) =>
        {
            if (delta.Seq < minimumSequence)
            {
                return;
            }

            runtimeEventGateway.TelemetryDelta -= handler;
            cancellationRegistration.Dispose();
            completion.TrySetResult();
        };

        runtimeEventGateway.TelemetryDelta += handler;
        cancellationRegistration = ct.Register(static state =>
        {
            StateTuple tuple = (StateTuple)state!;
            tuple.Gateway.TelemetryDelta -= tuple.Handler;
            tuple.Completion.TrySetCanceled();
        }, new StateTuple(runtimeEventGateway, handler, completion));

        return completion.Task;
    }

    private static double ElapsedMilliseconds(long startedAt)
    {
        return (Stopwatch.GetTimestamp() - startedAt) * 1000d / Stopwatch.Frequency;
    }

    private static ulong AddSaturating(ulong left, ulong right)
    {
        ulong sum = left + right;
        return sum < left ? ulong.MaxValue : sum;
    }

    private static double Percentile95(List<double> samples)
    {
        if (samples.Count == 0)
        {
            return 0d;
        }

        double[] ordered = [.. samples];
        Array.Sort(ordered);
        int percentileIndex = Math.Min(
            ordered.Length - 1,
            Math.Max(0, (int)Math.Ceiling(ordered.Length * 0.95d) - 1));

        return ordered[percentileIndex];
    }

    private static DispatcherQueue ResolveShellDispatcherQueue(IServiceProvider services)
    {
        Type? mainWindowType = services
            .GetType()
            .Assembly
            .GetType("BatCave.MainWindow", throwOnError: false);

        if (mainWindowType is null)
        {
            mainWindowType = AppDomain.CurrentDomain
                .GetAssemblies()
                .Select(assembly => assembly.GetType("BatCave.MainWindow", throwOnError: false))
                .FirstOrDefault(static candidate => candidate is not null);
        }

        if (mainWindowType is null)
        {
            throw new InvalidOperationException("Could not resolve the BatCave shell window type for WinUI benchmark execution.");
        }

        object? shellWindow = services.GetService(mainWindowType);
        if (shellWindow is null)
        {
            throw new InvalidOperationException("Could not resolve the BatCave shell window service for WinUI benchmark execution.");
        }

        PropertyInfo? dispatcherQueueProperty = mainWindowType.GetProperty(nameof(DispatcherQueue));
        if (dispatcherQueueProperty?.GetValue(shellWindow) is not DispatcherQueue dispatcherQueue)
        {
            throw new InvalidOperationException("Could not resolve the BatCave shell dispatcher queue for WinUI benchmark execution.");
        }

        return dispatcherQueue;
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

    internal static bool TryCreateOptions(
        string[] args,
        out WinUiBenchmarkCliOptions options,
        out IReadOnlyList<string> errors)
    {
        ParseResult parseResult = BenchmarkRootCommand.Parse(args);
        if (parseResult.Errors.Count > 0)
        {
            options = new WinUiBenchmarkCliOptions();
            errors = parseResult.Errors.Select(parseError => parseError.Message).ToArray();
            return false;
        }

        return TryCreateOptions(parseResult, out options, out errors);
    }

    private static RootCommand CreateRootCommand()
    {
        RootCommand command = new();
        command.Add(BenchmarkOption);
        command.Add(StrictOption);
        command.Add(TicksOption);
        command.Add(SleepMsOption);
        command.Add(BaselineJsonOption);
        command.Add(MinSpeedupMultiplierOption);
        command.Add(MaxP95MsOption);
        return command;
    }

    private static bool TryCreateOptions(
        ParseResult parseResult,
        out WinUiBenchmarkCliOptions options,
        out IReadOnlyList<string> errors)
    {
        List<string> parseErrors = [];
        int ticks = ParseIntOption(parseResult.GetValue(TicksOption), "--ticks", defaultValue: 120, parseErrors);
        int sleepMs = ParseIntOption(parseResult.GetValue(SleepMsOption), "--sleep-ms", defaultValue: 1000, parseErrors);
        string? baselineJsonPath = parseResult.GetValue(BaselineJsonOption);
        double? minSpeedupMultiplier = ParsePositiveDoubleOption(
            parseResult.GetValue(MinSpeedupMultiplierOption),
            "--min-speedup-multiplier",
            parseErrors);
        double? maxP95Ms = ParsePositiveDoubleOption(
            parseResult.GetValue(MaxP95MsOption),
            "--max-p95-ms",
            parseErrors);

        if (parseErrors.Count > 0)
        {
            options = new WinUiBenchmarkCliOptions();
            errors = parseErrors;
            return false;
        }

        if (minSpeedupMultiplier.HasValue && string.IsNullOrWhiteSpace(baselineJsonPath))
        {
            options = new WinUiBenchmarkCliOptions();
            errors = ["--min-speedup-multiplier requires --baseline-json."];
            return false;
        }

        BenchmarkSummary? baseline = null;
        if (!string.IsNullOrWhiteSpace(baselineJsonPath)
            && !TryLoadBaselineSummary(baselineJsonPath, out baseline, out string? baselineError))
        {
            options = new WinUiBenchmarkCliOptions();
            errors = [baselineError!];
            return false;
        }

        options = new WinUiBenchmarkCliOptions
        {
            Ticks = ticks,
            SleepMs = sleepMs,
            Strict = parseResult.GetValue(StrictOption),
            GateOptions = new BenchmarkGateOptions
            {
                Baseline = baseline,
                MinSpeedupMultiplier = minSpeedupMultiplier,
                MaxP95Ms = maxP95Ms,
            },
        };
        errors = Array.Empty<string>();
        return true;
    }

    private static int ParseIntOption(
        string? rawValue,
        string optionName,
        int defaultValue,
        List<string> errors)
    {
        if (string.IsNullOrWhiteSpace(rawValue))
        {
            return defaultValue;
        }

        if (int.TryParse(rawValue, NumberStyles.Integer, CultureInfo.InvariantCulture, out int value))
        {
            return value;
        }

        errors.Add($"Missing or invalid value for {optionName}.");
        return defaultValue;
    }

    private static double? ParsePositiveDoubleOption(
        string? rawValue,
        string optionName,
        List<string> errors)
    {
        if (string.IsNullOrWhiteSpace(rawValue))
        {
            return null;
        }

        if (double.TryParse(rawValue, NumberStyles.Float | NumberStyles.AllowThousands, CultureInfo.InvariantCulture, out double value)
            && value > 0d)
        {
            return value;
        }

        errors.Add($"Missing or invalid value for {optionName} (must be > 0).");
        return null;
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

    internal sealed record WinUiBenchmarkCliOptions
    {
        public int Ticks { get; init; } = 120;

        public int SleepMs { get; init; } = 1000;

        public bool Strict { get; init; }

        public BenchmarkGateOptions GateOptions { get; init; } = new();
    }

    private sealed record LiveShellBenchmarkMeasurement
    {
        public int Ticks { get; init; }

        public int SleepMs { get; init; }

        public double TickP95Ms { get; init; }

        public double SortP95Ms { get; init; }

        public double AvgAppCpuPct { get; init; }

        public ulong AvgAppRssBytes { get; init; }
    }

    private sealed record StateTuple(
        IRuntimeEventGateway Gateway,
        EventHandler<ProcessDeltaBatch> Handler,
        TaskCompletionSource Completion);
}
