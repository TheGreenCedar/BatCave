using System.Text.Json;
using BatCave.Core.Runtime;
using BatCave.Core.Serialization;

namespace BatCave.Bench;

internal static class Program
{
    public static int Main(string[] args)
    {
        if (!TryParseArgs(args, out int ticks, out int sleepMs, out bool strict, out string? error))
        {
            Console.Error.WriteLine(error);
            return 2;
        }

        BenchmarkSummary summary = BenchmarkRunner.Run(ticks, sleepMs, CancellationToken.None);
        Console.WriteLine(JsonSerializer.Serialize(summary, JsonDefaults.SnakeCase));

        if (strict && !summary.BudgetPassed)
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
        out string? error)
    {
        ticks = 120;
        sleepMs = 1000;
        strict = false;
        error = null;

        for (int index = 0; index < args.Length; index++)
        {
            if (!TryParseArgument(args, ref index, ref ticks, ref sleepMs, ref strict, out error))
            {
                return false;
            }
        }

        return true;
    }

    private static bool TryParseArgument(
        string[] args,
        ref int index,
        ref int ticks,
        ref int sleepMs,
        ref bool strict,
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
            default:
                error = $"Unknown argument: {argument}";
                return false;
        }
    }

    private static bool TryReadIntValue(string[] args, ref int index, out int value)
    {
        value = 0;
        int valueIndex = index + 1;
        if (valueIndex >= args.Length)
        {
            return false;
        }

        if (!int.TryParse(args[valueIndex], out value))
        {
            return false;
        }

        index = valueIndex;
        return true;
    }
}
