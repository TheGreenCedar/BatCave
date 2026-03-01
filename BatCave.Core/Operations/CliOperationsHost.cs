using System.CommandLine;
using System.CommandLine.Parsing;
using System.Text.Json;
using BatCave.Core.Abstractions;
using BatCave.Core.Collector;
using BatCave.Core.Domain;
using BatCave.Core.Runtime;
using BatCave.Core.Serialization;

namespace BatCave.Core.Operations;

public sealed class CliOperationsHost : ICliOperationsHost
{
    private static readonly Option<bool> PrintGateStatusOption = new("--print-gate-status");
    private static readonly Option<bool> BenchmarkOption = new("--benchmark");
    private static readonly Option<bool> ElevatedHelperOption = new("--elevated-helper");
    private static readonly Option<bool> StrictOption = new("--strict");
    private static readonly Option<string?> TicksOption = new("--ticks");
    private static readonly Option<string?> SleepMsOption = new("--sleep-ms");
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
        if (parseResult.Errors.Count > 0)
        {
            foreach (ParseError parseError in parseResult.Errors)
            {
                Console.Error.WriteLine(parseError.Message);
            }

            return Task.FromResult(2);
        }

        if (parseResult.GetValue(ElevatedHelperOption))
        {
            return Task.FromResult(ExecuteElevatedHelper(parseResult, ct));
        }

        if (parseResult.GetValue(PrintGateStatusOption))
        {
            return Task.FromResult(ExecuteGateStatus());
        }

        if (parseResult.GetValue(BenchmarkOption))
        {
            return Task.FromResult(ExecuteBenchmark(parseResult, ct));
        }

        Console.Error.WriteLine("CLI mode is recognized but no command was selected.");
        return Task.FromResult(2);
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

        BenchmarkSummary summary = BenchmarkRunner.Run(ticks, sleepMs, ct);
        WriteJson(summary);

        if (strict && !summary.BudgetPassed)
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

    private static void WriteJson<T>(T value)
    {
        string payload = JsonSerializer.Serialize(value, JsonDefaults.SnakeCase);
        Console.WriteLine(payload);
    }
}
