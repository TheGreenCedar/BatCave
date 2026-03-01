using System.Text.Json;
using BatCave.Core.Abstractions;
using BatCave.Core.Collector;
using BatCave.Core.Domain;
using BatCave.Core.Runtime;
using BatCave.Core.Serialization;

namespace BatCave.Core.Operations;

public sealed class CliOperationsHost : ICliOperationsHost
{
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
        if (args.Contains("--elevated-helper", StringComparer.OrdinalIgnoreCase))
        {
            return Task.FromResult(ExecuteElevatedHelper(args, ct));
        }

        if (args.Contains("--print-gate-status", StringComparer.OrdinalIgnoreCase))
        {
            return Task.FromResult(ExecuteGateStatus());
        }

        if (args.Contains("--benchmark", StringComparer.OrdinalIgnoreCase))
        {
            return Task.FromResult(ExecuteBenchmark(args, ct));
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

    private static int ExecuteBenchmark(string[] args, CancellationToken ct)
    {
        int ticks = ParseOptionInt(args, "--ticks", 120);
        int sleepMs = ParseOptionInt(args, "--sleep-ms", 1000);
        bool strict = args.Contains("--strict", StringComparer.OrdinalIgnoreCase);

        BenchmarkSummary summary = BenchmarkRunner.Run(ticks, sleepMs, ct);
        WriteJson(summary);

        if (strict && !summary.BudgetPassed)
        {
            return 2;
        }

        return 0;
    }

    private static int ExecuteElevatedHelper(string[] args, CancellationToken ct)
    {
        string? dataFile = GetOptionValue(args, "--data-file");
        string? stopFile = GetOptionValue(args, "--stop-file");
        string? token = GetOptionValue(args, "--token");

        if (string.IsNullOrWhiteSpace(dataFile) || string.IsNullOrWhiteSpace(stopFile) || string.IsNullOrWhiteSpace(token))
        {
            Console.Error.WriteLine("Missing elevated helper arguments. Required: --data-file, --stop-file, --token.");
            return 2;
        }

        return ElevatedBridgeClient.RunElevatedHelper(dataFile, stopFile, token, ct);
    }

    private static int ParseOptionInt(string[] args, string optionName, int defaultValue)
    {
        string? value = GetOptionValue(args, optionName);
        return int.TryParse(value, out int parsed) ? parsed : defaultValue;
    }

    private static string? GetOptionValue(string[] args, string optionName)
    {
        for (int i = 0; i < args.Length - 1; i++)
        {
            if (string.Equals(args[i], optionName, StringComparison.OrdinalIgnoreCase))
            {
                return args[i + 1];
            }
        }

        return null;
    }

    private static void WriteJson<T>(T value)
    {
        string payload = JsonSerializer.Serialize(value, JsonDefaults.SnakeCase);
        Console.WriteLine(payload);
    }
}
