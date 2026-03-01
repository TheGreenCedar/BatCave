using BatCave.Core.Collector;

namespace BatCave.Core.Operations;

public sealed class CliOperationsHost : ICliOperationsHost
{
    private static readonly HashSet<string> CliFlags = new(StringComparer.OrdinalIgnoreCase)
    {
        "--print-gate-status",
        "--benchmark",
        "--elevated-helper",
    };

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

        Console.Error.WriteLine("CLI mode is recognized but not fully implemented yet.");
        return Task.FromResult(2);
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
}
