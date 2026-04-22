using BatCave.Runtime.Benchmarking;
using BatCave.Runtime.Contracts;
using BatCave.Runtime.Serialization;
using System.Globalization;
using System.Text.Json;

namespace BatCave.Bench;

internal static class Program
{
    public static int Main(string[] args)
    {
        if (args.Any(static argument => string.Equals(argument, "--print-runtime-health", StringComparison.OrdinalIgnoreCase)))
        {
            RuntimeHealth snapshot = new()
            {
                RuntimeLoopEnabled = false,
                RuntimeLoopRunning = false,
                StartupBlocked = false,
                StatusSummary = "Runtime loop disabled for bench host diagnostics.",
                UpdatedAtMs = (ulong)Math.Max(0L, DateTimeOffset.UtcNow.ToUnixTimeMilliseconds()),
            };

            Console.WriteLine(JsonSerializer.Serialize(snapshot, JsonDefaults.SnakeCase));
            return 0;
        }

        if (!TryParseArgs(args, out int ticks, out int sleepMs, out bool strict, out BenchmarkGateOptions gateOptions, out string? error))
        {
            Console.Error.WriteLine(error);
            return 2;
        }

        BenchmarkSummary summary = BenchmarkRunner.Run(ticks, sleepMs, CancellationToken.None, gateOptions);
        Console.WriteLine(JsonSerializer.Serialize(summary, JsonDefaults.SnakeCase));
        return strict && !summary.StrictPassed ? 2 : 0;
    }

    private static bool TryParseArgs(
        string[] args,
        out int ticks,
        out int sleepMs,
        out bool strict,
        out BenchmarkGateOptions gateOptions,
        out string? error)
    {
        ticks = 120;
        sleepMs = 1000;
        strict = false;
        string? baselineJsonPath = null;
        double? minSpeedupMultiplier = null;
        double? maxP95Ms = null;
        error = null;

        for (int index = 0; index < args.Length; index++)
        {
            string argument = args[index];
            switch (argument)
            {
                case "--strict":
                    strict = true;
                    break;
                case "--ticks":
                    if (!TryReadIntValue(args, ref index, out ticks))
                    {
                        gateOptions = new BenchmarkGateOptions();
                        error = "Missing or invalid value for --ticks.";
                        return false;
                    }

                    break;
                case "--sleep-ms":
                    if (!TryReadIntValue(args, ref index, out sleepMs))
                    {
                        gateOptions = new BenchmarkGateOptions();
                        error = "Missing or invalid value for --sleep-ms.";
                        return false;
                    }

                    break;
                case "--baseline-json":
                    if (!TryReadStringValue(args, ref index, out baselineJsonPath))
                    {
                        gateOptions = new BenchmarkGateOptions();
                        error = "Missing value for --baseline-json.";
                        return false;
                    }

                    break;
                case "--min-speedup-multiplier":
                    if (!TryReadDoubleValue(args, ref index, out double minSpeedup) || minSpeedup <= 0d)
                    {
                        gateOptions = new BenchmarkGateOptions();
                        error = "Missing or invalid value for --min-speedup-multiplier (must be > 0).";
                        return false;
                    }

                    minSpeedupMultiplier = minSpeedup;
                    break;
                case "--max-p95-ms":
                    if (!TryReadDoubleValue(args, ref index, out double maxP95) || maxP95 <= 0d)
                    {
                        gateOptions = new BenchmarkGateOptions();
                        error = "Missing or invalid value for --max-p95-ms (must be > 0).";
                        return false;
                    }

                    maxP95Ms = maxP95;
                    break;
                default:
                    gateOptions = new BenchmarkGateOptions();
                    error = $"Unknown argument: {argument}";
                    return false;
            }
        }

        if (minSpeedupMultiplier.HasValue && string.IsNullOrWhiteSpace(baselineJsonPath))
        {
            gateOptions = new BenchmarkGateOptions();
            error = "--min-speedup-multiplier requires --baseline-json.";
            return false;
        }

        BenchmarkSummary? baseline = null;
        if (!string.IsNullOrWhiteSpace(baselineJsonPath) && !TryLoadBaselineSummary(baselineJsonPath, out baseline, out error))
        {
            gateOptions = new BenchmarkGateOptions();
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

    private static bool TryReadIntValue(string[] args, ref int index, out int value)
    {
        value = 0;
        return TryReadStringValue(args, ref index, out string? rawValue)
               && int.TryParse(rawValue, NumberStyles.Integer, CultureInfo.InvariantCulture, out value);
    }

    private static bool TryReadDoubleValue(string[] args, ref int index, out double value)
    {
        value = 0;
        return TryReadStringValue(args, ref index, out string? rawValue)
               && double.TryParse(rawValue, NumberStyles.Float | NumberStyles.AllowThousands, CultureInfo.InvariantCulture, out value);
    }

    private static bool TryReadStringValue(string[] args, ref int index, out string? value)
    {
        value = null;
        int valueIndex = index + 1;
        if (valueIndex >= args.Length)
        {
            return false;
        }

        value = args[valueIndex];
        index = valueIndex;
        return true;
    }

    private static bool TryLoadBaselineSummary(string baselineJsonPath, out BenchmarkSummary? baseline, out string? error)
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
            baseline = JsonSerializer.Deserialize<BenchmarkSummary>(File.ReadAllText(baselineJsonPath), JsonDefaults.SnakeCase);
            if (baseline is null)
            {
                error = $"Baseline JSON did not contain a benchmark summary: {baselineJsonPath}";
                return false;
            }

            return true;
        }
        catch (Exception ex)
        {
            error = $"Failed to read baseline JSON '{baselineJsonPath}': {ex.GetType().Name}: {ex.Message}";
            return false;
        }
    }
}
