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
            string argument = args[index];
            switch (argument)
            {
                case "--strict":
                    strict = true;
                    break;
                case "--ticks":
                {
                    if (!TryReadIntValue(args, ref index, out ticks))
                    {
                        error = "Missing or invalid value for --ticks.";
                        return false;
                    }

                    break;
                }
                case "--sleep-ms":
                {
                    if (!TryReadIntValue(args, ref index, out sleepMs))
                    {
                        error = "Missing or invalid value for --sleep-ms.";
                        return false;
                    }

                    break;
                }
                default:
                    error = $"Unknown argument: {argument}";
                    return false;
            }
        }

        return true;
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
