using BatCave.Core.Domain;
using BatCave.Core.Runtime;
using BatCave.Core.Serialization;
using BatCave.ViewModels;
using Microsoft.Extensions.DependencyInjection;
using System.CommandLine;
using System.CommandLine.Parsing;
using System;
using System.Diagnostics;
using System.Globalization;
using System.IO;
using System.Linq;
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
        await runtime.InitializeAsync(ct).ConfigureAwait(false);
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
}

