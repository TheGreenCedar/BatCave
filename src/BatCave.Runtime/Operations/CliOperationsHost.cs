using BatCave.Runtime.Benchmarking;
using BatCave.Runtime.Collectors;
using BatCave.Runtime.Contracts;
using BatCave.Runtime.Serialization;
using System.Globalization;
using System.Text.Json;

namespace BatCave.Runtime.Operations;

public sealed class CliOperationsHost(
    ILaunchPolicyGate launchPolicyGate,
    IRuntimeStore runtimeStore,
    IEnumerable<IWinUiBenchmarkRunner>? winUiBenchmarkRunners = null)
{
    private static readonly HashSet<string> CliFlags = new(StringComparer.OrdinalIgnoreCase)
    {
        "--print-gate-status",
        "--print-runtime-health",
        "--benchmark",
        "--elevated-helper",
    };

    private static readonly HashSet<string> KnownOptions = new(StringComparer.OrdinalIgnoreCase)
    {
        "--print-gate-status",
        "--print-runtime-health",
        "--benchmark",
        "--elevated-helper",
        "--strict",
        "--ticks",
        "--sleep-ms",
        "--benchmark-host",
        "--baseline-json",
        "--min-speedup-multiplier",
        "--max-p95-ms",
        "--data-file",
        "--stop-file",
        "--token",
    };

    private static readonly HashSet<string> ValueOptions = new(StringComparer.OrdinalIgnoreCase)
    {
        "--ticks",
        "--sleep-ms",
        "--benchmark-host",
        "--baseline-json",
        "--min-speedup-multiplier",
        "--max-p95-ms",
        "--data-file",
        "--stop-file",
        "--token",
    };

    private readonly ILaunchPolicyGate _launchPolicyGate = launchPolicyGate;
    private readonly IRuntimeStore _runtimeStore = runtimeStore;
    private readonly IWinUiBenchmarkRunner? _winUiBenchmarkRunner = winUiBenchmarkRunners?.LastOrDefault();

    public bool IsCliMode(string[] args) => args.Any(CliFlags.Contains);

    public async Task<int> ExecuteAsync(string[] args, CancellationToken ct)
    {
        if (!TryValidateArgs(args, out string? parseError))
        {
            Console.Error.WriteLine(parseError);
            return 2;
        }

        if (args.Any(Is("--print-gate-status")))
        {
            StartupGateStatus status = _launchPolicyGate.Enforce();
            WriteJson(status);
            return status.Passed ? 0 : 2;
        }

        if (args.Any(Is("--print-runtime-health")))
        {
            RuntimeSnapshot snapshot = _runtimeStore.GetSnapshot();
            WriteJson(snapshot.Health);
            return 0;
        }

        if (args.Any(Is("--benchmark")))
        {
            return await ExecuteBenchmarkAsync(args, ct).ConfigureAwait(false);
        }

        if (args.Any(Is("--elevated-helper")))
        {
            return ExecuteElevatedHelper(args, ct);
        }

        Console.Error.WriteLine("CLI mode is recognized but no command was selected.");
        return 2;
    }

    private static int ExecuteElevatedHelper(string[] args, CancellationToken ct)
    {
        string? dataFile = ParseStringOption(args, "--data-file");
        string? stopFile = ParseStringOption(args, "--stop-file");
        string? token = ParseStringOption(args, "--token");

        if (string.IsNullOrWhiteSpace(dataFile) || string.IsNullOrWhiteSpace(stopFile) || string.IsNullOrWhiteSpace(token))
        {
            Console.Error.WriteLine("Missing elevated helper arguments. Required: --data-file, --stop-file, --token.");
            return 2;
        }

        return ElevatedBridgeHelper.RunElevatedHelper(dataFile, stopFile, token, ct);
    }

    private async Task<int> ExecuteBenchmarkAsync(string[] args, CancellationToken ct)
    {
        int ticks = ParseIntOption(args, "--ticks", 120);
        int sleepMs = ParseIntOption(args, "--sleep-ms", 1000);
        bool strict = args.Any(Is("--strict"));
        if (!TryBuildGateOptions(args, out BenchmarkGateOptions gates, out string? error))
        {
            Console.Error.WriteLine(error);
            return 2;
        }

        BenchmarkSummary summary = IsWinUiHost(gates) && _winUiBenchmarkRunner is not null
            ? await _winUiBenchmarkRunner.RunAsync(ticks, sleepMs, gates, ct).ConfigureAwait(false)
            : BenchmarkRunner.Run(ticks, sleepMs, ct, gates);
        WriteJson(summary);
        return strict && !summary.StrictPassed ? 2 : 0;
    }

    private static bool TryBuildGateOptions(string[] args, out BenchmarkGateOptions gates, out string? error)
    {
        gates = new BenchmarkGateOptions();
        error = null;
        string? baselinePath = ParseStringOption(args, "--baseline-json");
        string host = ParseStringOption(args, "--benchmark-host") ?? "core";

        if (!TryParsePositiveDouble(ParseStringOption(args, "--min-speedup-multiplier"), "--min-speedup-multiplier", out double? minSpeedup, out error)
            || !TryParsePositiveDouble(ParseStringOption(args, "--max-p95-ms"), "--max-p95-ms", out double? maxP95, out error))
        {
            return false;
        }

        if (!string.Equals(host, "core", StringComparison.OrdinalIgnoreCase)
            && !string.Equals(host, "winui", StringComparison.OrdinalIgnoreCase))
        {
            error = "--benchmark-host must be 'core' or 'winui'.";
            return false;
        }

        if (minSpeedup.HasValue && string.IsNullOrWhiteSpace(baselinePath))
        {
            error = "--min-speedup-multiplier requires --baseline-json.";
            return false;
        }

        BenchmarkSummary? baseline = null;
        if (!string.IsNullOrWhiteSpace(baselinePath) && !TryReadBaseline(baselinePath, out baseline, out error))
        {
            return false;
        }

        bool winuiHost = string.Equals(host, "winui", StringComparison.OrdinalIgnoreCase);
        gates = new BenchmarkGateOptions
        {
            Host = winuiHost ? "winui" : "core",
            MeasurementOrigin = winuiHost ? BenchmarkRunner.WinUiMeasurementOrigin : BenchmarkRunner.CoreMeasurementOrigin,
            UsesAttachedDispatcher = winuiHost,
            CpuBudgetPct = BenchmarkRunner.CpuBudgetPct,
            RssBudgetBytes = winuiHost ? 256UL * 1024UL * 1024UL : BenchmarkRunner.RssBudgetBytes,
            Baseline = baseline,
            MinSpeedupMultiplier = minSpeedup,
            MaxP95Ms = maxP95,
            RequireInteractionProbeSpeedup = winuiHost && minSpeedup.HasValue,
        };
        return true;
    }

    private static bool TryReadBaseline(string path, out BenchmarkSummary? baseline, out string? error)
    {
        baseline = null;
        error = null;
        if (!File.Exists(path))
        {
            error = $"Baseline file not found: {path}";
            return false;
        }

        try
        {
            baseline = JsonSerializer.Deserialize<BenchmarkSummary>(File.ReadAllText(path), JsonDefaults.SnakeCase);
            if (baseline is null)
            {
                error = $"Baseline JSON did not contain a benchmark summary: {path}";
                return false;
            }

            return true;
        }
        catch (Exception ex)
        {
            error = $"Failed to read baseline JSON '{path}': {ex.GetType().Name}: {ex.Message}";
            return false;
        }
    }

    private static bool TryParsePositiveDouble(string? raw, string optionName, out double? value, out string? error)
    {
        value = null;
        error = null;
        if (string.IsNullOrWhiteSpace(raw))
        {
            return true;
        }

        if (!double.TryParse(raw, NumberStyles.Float | NumberStyles.AllowThousands, CultureInfo.InvariantCulture, out double parsed)
            || parsed <= 0d)
        {
            error = $"Missing or invalid value for {optionName} (must be > 0).";
            return false;
        }

        value = parsed;
        return true;
    }

    private static int ParseIntOption(string[] args, string optionName, int defaultValue)
    {
        string? raw = ParseStringOption(args, optionName);
        return int.TryParse(raw, NumberStyles.Integer, CultureInfo.InvariantCulture, out int parsed)
            ? parsed
            : defaultValue;
    }

    private static string? ParseStringOption(string[] args, string optionName)
    {
        for (int index = 0; index < args.Length - 1; index++)
        {
            if (string.Equals(args[index], optionName, StringComparison.OrdinalIgnoreCase))
            {
                return args[index + 1];
            }
        }

        return null;
    }

    private static bool TryValidateArgs(string[] args, out string? error)
    {
        error = null;
        for (int index = 0; index < args.Length; index++)
        {
            string argument = args[index];
            if (!argument.StartsWith("--", StringComparison.Ordinal))
            {
                continue;
            }

            if (!KnownOptions.Contains(argument))
            {
                error = $"Unknown argument: {argument}";
                return false;
            }

            if (ValueOptions.Contains(argument)
                && (index + 1 >= args.Length || args[index + 1].StartsWith("--", StringComparison.Ordinal)))
            {
                error = $"Missing value for {argument}.";
                return false;
            }
        }

        return true;
    }

    private static Func<string, bool> Is(string optionName)
    {
        return value => string.Equals(value, optionName, StringComparison.OrdinalIgnoreCase);
    }

    private static bool IsWinUiHost(BenchmarkGateOptions gates)
    {
        return string.Equals(gates.Host, "winui", StringComparison.OrdinalIgnoreCase)
               || string.Equals(gates.MeasurementOrigin, BenchmarkRunner.WinUiMeasurementOrigin, StringComparison.OrdinalIgnoreCase)
               || string.Equals(gates.MeasurementOrigin, "winui_cli", StringComparison.OrdinalIgnoreCase);
    }

    private static void WriteJson<T>(T value)
    {
        Console.WriteLine(JsonSerializer.Serialize(value, JsonDefaults.SnakeCase));
    }
}
