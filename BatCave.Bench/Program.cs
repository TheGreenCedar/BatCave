using BatCave.Core.Runtime;
using BatCave.Core.Serialization;
using System.Globalization;
using System.Text.Json;

namespace BatCave.Bench;

internal static class Program
{
    public static int Main(string[] args)
    {
        if (!TryParseArgs(
                args,
                out int ticks,
                out int sleepMs,
                out bool strict,
                out BenchmarkGateOptions gateOptions,
                out string? error))
        {
            Console.Error.WriteLine(error);
            return 2;
        }

        BenchmarkSummary summary = BenchmarkRunner.Run(ticks, sleepMs, CancellationToken.None, gateOptions);
        Console.WriteLine(JsonSerializer.Serialize(summary, JsonDefaults.SnakeCase));

        if (strict && !summary.StrictPassed)
        {
            return 2;
        }

        return 0;
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
                gateOptions = new BenchmarkGateOptions();
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
        if (!string.IsNullOrWhiteSpace(baselineJsonPath)
            && !TryLoadBaselineSummary(baselineJsonPath, out baseline, out error))
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
}
