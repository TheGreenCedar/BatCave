using BatCave.Core.Abstractions;
using BatCave.Core.Collector;
using BatCave.Core.Domain;
using BatCave.Core.Runtime;
using BatCave.Core.Serialization;
using System.CommandLine;
using System.CommandLine.Parsing;
using System.Globalization;
using System.Text.Json;

namespace BatCave.Core.Operations;

public sealed class CliOperationsHost : ICliOperationsHost
{
    private static readonly Option<bool> PrintGateStatusOption = new("--print-gate-status");
    private static readonly Option<bool> BenchmarkOption = new("--benchmark");
    private static readonly Option<bool> ElevatedHelperOption = new("--elevated-helper");
    private static readonly Option<bool> StrictOption = new("--strict");
    private static readonly Option<string?> TicksOption = new("--ticks");
    private static readonly Option<string?> SleepMsOption = new("--sleep-ms");
    private static readonly Option<string?> BaselineJsonOption = new("--baseline-json");
    private static readonly Option<string?> MinSpeedupMultiplierOption = new("--min-speedup-multiplier");
    private static readonly Option<string?> MaxP95MsOption = new("--max-p95-ms");
    private static readonly Option<string?> DataFileOption = new("--data-file");
    private static readonly Option<string?> StopFileOption = new("--stop-file");
    private static readonly Option<string?> TokenOption = new("--token");

    private static readonly RootCommand CliRootCommand = CreateRootCommand();

    private static readonly HashSet<string> CliFlags = new(StringComparer.OrdinalIgnoreCase)
    {
        "--print-gate-status",
        "--benchmark",
        "--elevated-helper",
    };

    private readonly ILaunchPolicyGate _launchPolicyGate;

    public CliOperationsHost(ILaunchPolicyGate launchPolicyGate)
    {
        _launchPolicyGate = launchPolicyGate;
    }

    public bool IsCliMode(string[] args)
    {
        return args.Any(CliFlags.Contains);
    }

    public Task<int> ExecuteAsync(string[] args, CancellationToken ct)
    {
        ParseResult parseResult = CliRootCommand.Parse(args);
        if (TryWriteParseErrors(parseResult))
        {
            return Task.FromResult(2);
        }

        return Task.FromResult(ExecuteParsedCommand(parseResult, ct));
    }

    private int ExecuteGateStatus()
    {
        StartupGateStatus status = _launchPolicyGate.Enforce();
        WriteJson(status);
        return status.Passed ? 0 : 2;
    }

    private static RootCommand CreateRootCommand()
    {
        RootCommand command = new();
        command.Add(PrintGateStatusOption);
        command.Add(BenchmarkOption);
        command.Add(ElevatedHelperOption);
        command.Add(StrictOption);
        command.Add(TicksOption);
        command.Add(SleepMsOption);
        command.Add(BaselineJsonOption);
        command.Add(MinSpeedupMultiplierOption);
        command.Add(MaxP95MsOption);
        command.Add(DataFileOption);
        command.Add(StopFileOption);
        command.Add(TokenOption);
        return command;
    }

    private static int ExecuteBenchmark(ParseResult parseResult, CancellationToken ct)
    {
        int ticks = ParseOptionInt(parseResult.GetValue(TicksOption), 120);
        int sleepMs = ParseOptionInt(parseResult.GetValue(SleepMsOption), 1000);
        bool strict = parseResult.GetValue(StrictOption);

        if (!TryBuildGateOptions(parseResult, out BenchmarkGateOptions gateOptions, out string? error))
        {
            Console.Error.WriteLine(error);
            return 2;
        }

        BenchmarkSummary summary = BenchmarkRunner.Run(ticks, sleepMs, ct, gateOptions);
        WriteJson(summary);

        if (strict && !summary.StrictPassed)
        {
            return 2;
        }

        return 0;
    }

    private static int ExecuteElevatedHelper(ParseResult parseResult, CancellationToken ct)
    {
        string? dataFile = parseResult.GetValue(DataFileOption);
        string? stopFile = parseResult.GetValue(StopFileOption);
        string? token = parseResult.GetValue(TokenOption);

        if (string.IsNullOrWhiteSpace(dataFile) || string.IsNullOrWhiteSpace(stopFile) || string.IsNullOrWhiteSpace(token))
        {
            Console.Error.WriteLine("Missing elevated helper arguments. Required: --data-file, --stop-file, --token.");
            return 2;
        }

        return ElevatedBridgeClient.RunElevatedHelper(dataFile, stopFile, token, ct);
    }

    private static int ParseOptionInt(string? value, int defaultValue)
    {
        return int.TryParse(value, out int parsed) ? parsed : defaultValue;
    }

    private static bool TryBuildGateOptions(
        ParseResult parseResult,
        out BenchmarkGateOptions gateOptions,
        out string? error)
    {
        gateOptions = new BenchmarkGateOptions();
        error = null;

        string? baselineJsonPath = parseResult.GetValue(BaselineJsonOption);

        if (!TryParseOptionalPositiveDouble(
                parseResult.GetValue(MinSpeedupMultiplierOption),
                "--min-speedup-multiplier",
                out double? minSpeedupMultiplier,
                out error))
        {
            return false;
        }

        if (!TryParseOptionalPositiveDouble(
                parseResult.GetValue(MaxP95MsOption),
                "--max-p95-ms",
                out double? maxP95Ms,
                out error))
        {
            return false;
        }

        if (minSpeedupMultiplier.HasValue && string.IsNullOrWhiteSpace(baselineJsonPath))
        {
            error = "--min-speedup-multiplier requires --baseline-json.";
            return false;
        }

        BenchmarkSummary? baseline = null;
        if (!string.IsNullOrWhiteSpace(baselineJsonPath)
            && !TryLoadBaselineSummary(baselineJsonPath, out baseline, out error))
        {
            return false;
        }

        gateOptions = new BenchmarkGateOptions
        {
            Baseline = baseline,
            MinSpeedupMultiplier = minSpeedupMultiplier,
            MaxP95Ms = maxP95Ms,
        };

        return true;
    }

    private static bool TryParseOptionalPositiveDouble(
        string? rawValue,
        string optionName,
        out double? parsed,
        out string? error)
    {
        parsed = null;
        error = null;

        if (string.IsNullOrWhiteSpace(rawValue))
        {
            return true;
        }

        if (!double.TryParse(
                rawValue,
                NumberStyles.Float | NumberStyles.AllowThousands,
                CultureInfo.InvariantCulture,
                out double value)
            || value <= 0d)
        {
            error = $"Invalid value for {optionName}. Expected a positive number.";
            return false;
        }

        parsed = value;
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

    private static bool TryWriteParseErrors(ParseResult parseResult)
    {
        if (parseResult.Errors.Count == 0)
        {
            return false;
        }

        foreach (ParseError parseError in parseResult.Errors)
        {
            Console.Error.WriteLine(parseError.Message);
        }

        return true;
    }

    private int ExecuteParsedCommand(ParseResult parseResult, CancellationToken ct)
    {
        return (
            parseResult.GetValue(ElevatedHelperOption),
            parseResult.GetValue(PrintGateStatusOption),
            parseResult.GetValue(BenchmarkOption)) switch
        {
            (true, _, _) => ExecuteElevatedHelper(parseResult, ct),
            (_, true, _) => ExecuteGateStatus(),
            (_, _, true) => ExecuteBenchmark(parseResult, ct),
            _ => WriteNoCommandSelected(),
        };
    }

    private static int WriteNoCommandSelected()
    {
        Console.Error.WriteLine("CLI mode is recognized but no command was selected.");
        return 2;
    }

    private static void WriteJson<T>(T value)
    {
        string payload = JsonSerializer.Serialize(value, JsonDefaults.SnakeCase);
        Console.WriteLine(payload);
    }
}
