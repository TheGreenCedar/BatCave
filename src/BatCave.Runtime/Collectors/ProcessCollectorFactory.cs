using BatCave.Runtime.Contracts;
using BatCave.Runtime.Persistence;
using BatCave.Runtime.Serialization;
using System.Diagnostics;
using System.Text.Json;

namespace BatCave.Runtime.Collectors;

public sealed record CollectorActivationResult(
    IProcessCollector Collector,
    bool EffectiveAdminMode,
    string? Warning);

public interface IProcessCollectorFactory
{
    ValueTask<CollectorActivationResult> CreateAsync(bool adminMode, CancellationToken ct);
}

public sealed class DefaultProcessCollectorFactory : IProcessCollectorFactory
{
    public async ValueTask<CollectorActivationResult> CreateAsync(bool adminMode, CancellationToken ct)
    {
        if (!adminMode)
        {
            return new CollectorActivationResult(new WindowsProcessCollector(), EffectiveAdminMode: false, Warning: null);
        }

        try
        {
            ElevatedBridgeClient bridge = await ElevatedBridgeClient.LaunchAsync(ct).ConfigureAwait(false);
            return new CollectorActivationResult(new BridgedProcessCollector(bridge), EffectiveAdminMode: true, Warning: null);
        }
        catch (OperationCanceledException)
        {
            throw;
        }
        catch (Exception ex)
        {
            return new CollectorActivationResult(
                new WindowsProcessCollector(),
                EffectiveAdminMode: false,
                Warning: $"admin_mode_start_failed requested_admin_mode=true fallback_admin_mode=false error={ex.GetType().Name}: {ex.Message}");
        }
    }
}

internal sealed class StaticProcessCollectorFactory(IProcessCollector collector) : IProcessCollectorFactory
{
    public ValueTask<CollectorActivationResult> CreateAsync(bool adminMode, CancellationToken ct)
    {
        string? warning = adminMode
            ? "admin_mode_start_failed requested_admin_mode=true fallback_admin_mode=false error=InvalidOperationException: no elevated collector factory is configured."
            : null;
        return ValueTask.FromResult(new CollectorActivationResult(collector, EffectiveAdminMode: false, warning));
    }
}

internal sealed class BridgedProcessCollector(ElevatedBridgeClient bridge) : IProcessCollector, IDisposable
{
    private readonly WindowsProcessCollector _local = new();
    private readonly Queue<string> _pendingWarnings = [];
    private string? _lastBridgeFaultReason;

    public IReadOnlyList<ProcessSample> Collect(ulong seq)
    {
        CaptureBridgeWarning();
        BridgePollResult pollResult = bridge.PollRows();
        CaptureBridgeWarning();

        return pollResult.State == BridgePollState.Rows
            ? StampRowsWithTick(pollResult.Rows, seq)
            : CollectFromLocalAfterBridgeState(pollResult, seq);
    }

    public string? TakeWarning()
    {
        return _pendingWarnings.TryDequeue(out string? warning) ? warning : null;
    }

    public void Dispose()
    {
        bridge.Dispose();
    }

    private IReadOnlyList<ProcessSample> CollectFromLocalAfterBridgeState(BridgePollResult pollResult, ulong seq)
    {
        if (pollResult.State == BridgePollState.Faulted)
        {
            if (!string.IsNullOrWhiteSpace(pollResult.Reason)
                && !string.Equals(_lastBridgeFaultReason, pollResult.Reason, StringComparison.Ordinal))
            {
                _pendingWarnings.Enqueue($"elevated_bridge_faulted: {pollResult.Reason}");
                _lastBridgeFaultReason = pollResult.Reason;
            }
        }
        else
        {
            _lastBridgeFaultReason = null;
        }

        return _local.Collect(seq);
    }

    private static IReadOnlyList<ProcessSample> StampRowsWithTick(IReadOnlyList<ProcessSample> rows, ulong seq)
    {
        ulong timestamp = (ulong)Math.Max(0L, DateTimeOffset.UtcNow.ToUnixTimeMilliseconds());
        return Array.AsReadOnly(rows.Select(row => row with
        {
            Seq = seq,
            TsMs = timestamp,
        }).ToArray());
    }

    private void CaptureBridgeWarning()
    {
        string? warning = bridge.TakeWarning();
        if (!string.IsNullOrWhiteSpace(warning))
        {
            _pendingWarnings.Enqueue(warning);
        }
    }
}

internal enum BridgePollState
{
    Rows,
    Pending,
    Faulted,
}

internal sealed record BridgePollResult
{
    public BridgePollState State { get; init; }

    public IReadOnlyList<ProcessSample> Rows { get; init; } = [];

    public string? Reason { get; init; }

    public static BridgePollResult RowsResult(IReadOnlyList<ProcessSample> rows) => new()
    {
        State = BridgePollState.Rows,
        Rows = rows,
    };

    public static BridgePollResult Pending() => new()
    {
        State = BridgePollState.Pending,
    };

    public static BridgePollResult Faulted(string reason) => new()
    {
        State = BridgePollState.Faulted,
        Reason = reason,
    };
}

internal sealed class ElevatedBridgeClient : IDisposable
{
    private const ulong BridgeStaleTimeoutMs = 4_000;
    private const ulong BridgeStartupGraceMs = 10_000;

    private readonly string _dataFile;
    private readonly string _stopFile;
    private readonly string _token;
    private readonly Func<ulong> _nowMs;
    private readonly ulong _launchedMs;
    private readonly Queue<string> _pendingWarnings = [];
    private ulong _lastSeq;
    private IReadOnlyList<ProcessSample> _lastRows = [];
    private ulong? _lastSuccessMs;
    private string? _activeFaultReason;

    private ElevatedBridgeClient(
        string dataFile,
        string stopFile,
        string token,
        ulong? launchedMs = null,
        Func<ulong>? nowMs = null)
    {
        _dataFile = dataFile;
        _stopFile = stopFile;
        _token = token;
        _nowMs = nowMs ?? NowMs;
        _launchedMs = launchedMs ?? _nowMs();
    }

    public static async Task<ElevatedBridgeClient> LaunchAsync(CancellationToken ct)
    {
        (string dataFile, string stopFile, string token) = BuildBridgeFiles();

        TryDelete(dataFile);
        TryDelete(stopFile);

        _ = await LaunchElevatedHelperProcessAsync(dataFile, stopFile, token, ct).ConfigureAwait(false);
        return new ElevatedBridgeClient(dataFile, stopFile, token);
    }

    internal static ElevatedBridgeClient CreateForTest(
        string dataFile,
        string stopFile,
        string token,
        ulong launchedMs,
        Func<ulong>? nowMs = null)
    {
        return new ElevatedBridgeClient(dataFile, stopFile, token, launchedMs, nowMs);
    }

    public BridgePollResult PollRows()
    {
        ulong now = _nowMs();
        TryReadLatestSnapshot(now);
        BridgePollResult? pendingOrFault = GetPendingOrFaultBeforeRows(now);
        if (pendingOrFault is not null)
        {
            return pendingOrFault;
        }

        _activeFaultReason = null;
        return BridgePollResult.RowsResult(_lastRows);
    }

    public string? TakeWarning()
    {
        return _pendingWarnings.TryDequeue(out string? warning) ? warning : null;
    }

    public void Dispose()
    {
        try
        {
            File.WriteAllText(_stopFile, "stop");
        }
        catch
        {
            // Best-effort helper shutdown.
        }
    }

    private void TryReadLatestSnapshot(ulong now)
    {
        if (!File.Exists(_dataFile))
        {
            return;
        }

        try
        {
            string content = File.ReadAllText(_dataFile);
            ElevatedSnapshotFile? snapshot = JsonSerializer.Deserialize<ElevatedSnapshotFile>(content, JsonDefaults.SnakeCase);
            if (snapshot is null)
            {
                return;
            }

            if (string.Equals(snapshot.Token, _token, StringComparison.Ordinal) && snapshot.Seq > _lastSeq)
            {
                _lastSeq = snapshot.Seq;
                _lastRows = Array.AsReadOnly(snapshot.Rows.ToArray());
                _lastSuccessMs = now;
            }
        }
        catch (Exception ex)
        {
            EnqueueWarning($"elevated_bridge_snapshot_parse_failed file={_dataFile} error={ex.GetType().Name}: {ex.Message}");
        }
    }

    private BridgePollResult? GetPendingOrFaultBeforeRows(ulong now)
    {
        if (_lastSuccessMs is null)
        {
            return GetStartupPendingOrFault(now);
        }

        return GetStaleFaultIfAny(now, _lastSuccessMs.Value);
    }

    private BridgePollResult GetStartupPendingOrFault(ulong now)
    {
        ulong startupElapsed = now - _launchedMs;
        if (startupElapsed > BridgeStartupGraceMs)
        {
            return Fault($"no elevated bridge snapshot received within startup grace window ({BridgeStartupGraceMs} ms)");
        }

        _activeFaultReason = null;
        return BridgePollResult.Pending();
    }

    private BridgePollResult? GetStaleFaultIfAny(ulong now, ulong lastSuccessMs)
    {
        ulong staleFor = now - lastSuccessMs;
        if (staleFor <= BridgeStaleTimeoutMs)
        {
            return null;
        }

        return Fault("elevated bridge snapshot stream stalled");
    }

    private BridgePollResult Fault(string reason)
    {
        if (!string.Equals(_activeFaultReason, reason, StringComparison.Ordinal))
        {
            _activeFaultReason = reason;
            EnqueueWarning(reason);
        }

        return BridgePollResult.Faulted(reason);
    }

    private static async Task<uint> LaunchElevatedHelperProcessAsync(
        string dataFile,
        string stopFile,
        string token,
        CancellationToken ct)
    {
        bool isPackagedProcess = IsLikelyPackagedProcess();
        string executable = Environment.ProcessPath
                            ?? throw new InvalidOperationException("failed to resolve current executable path");

        string script = string.Join(" ",
            "$ErrorActionPreference='Stop';",
            $"$exe='{EscapePowerShellLiteral(executable)}';",
            $"$args=@('--elevated-helper','--data-file','{EscapePowerShellLiteral(dataFile)}','--stop-file','{EscapePowerShellLiteral(stopFile)}','--token','{EscapePowerShellLiteral(token)}');",
            "$p=Start-Process -FilePath $exe -ArgumentList $args -Verb RunAs -WindowStyle Hidden -PassThru;",
            "$p.Id");

        using Process process = new()
        {
            StartInfo = new ProcessStartInfo
            {
                FileName = "powershell",
                Arguments = $"-NoProfile -NonInteractive -Command \"{script}\"",
                RedirectStandardOutput = true,
                RedirectStandardError = true,
                UseShellExecute = false,
                CreateNoWindow = true,
            },
        };

        process.Start();
        string stdout = await process.StandardOutput.ReadToEndAsync(ct).ConfigureAwait(false);
        string stderr = await process.StandardError.ReadToEndAsync(ct).ConfigureAwait(false);
        await process.WaitForExitAsync(ct).ConfigureAwait(false);

        if (process.ExitCode != 0)
        {
            string detail = string.IsNullOrWhiteSpace(stderr) ? "unknown elevation start failure" : stderr.Trim();
            throw new InvalidOperationException(
                $"failed to start elevated helper (packaged={isPackagedProcess}): {NormalizeElevationFailureDetail(detail)}");
        }

        return ParseHelperPid(stdout);
    }

    private static uint ParseHelperPid(string stdout)
    {
        foreach (string line in stdout.Split(Environment.NewLine, StringSplitOptions.RemoveEmptyEntries | StringSplitOptions.TrimEntries))
        {
            if (uint.TryParse(line, out uint pid))
            {
                return pid;
            }
        }

        throw new InvalidOperationException("failed to parse elevated helper pid from PowerShell output");
    }

    private static (string DataFile, string StopFile, string Token) BuildBridgeFiles()
    {
        string bridgeDir = Path.Combine(LocalJsonRuntimePersistenceStore.DefaultBaseDirectory(), "elevated-bridge");
        Directory.CreateDirectory(bridgeDir);

        string runId = $"{Environment.ProcessId}-{NowMs()}";
        string token = $"{runId}-token";
        string dataFile = Path.Combine(bridgeDir, $"snapshot-{runId}.json");
        string stopFile = Path.Combine(bridgeDir, $"stop-{runId}.signal");
        return (dataFile, stopFile, token);
    }

    private static void TryDelete(string path)
    {
        try
        {
            if (File.Exists(path))
            {
                File.Delete(path);
            }
        }
        catch
        {
            // Best effort.
        }
    }

    private static string EscapePowerShellLiteral(string value)
    {
        return value.Replace("'", "''", StringComparison.Ordinal);
    }

    private static string NormalizeElevationFailureDetail(string detail)
    {
        if (detail.Contains("operation was canceled by the user", StringComparison.OrdinalIgnoreCase)
            || detail.Contains("operation was cancelled by the user", StringComparison.OrdinalIgnoreCase))
        {
            return $"elevation canceled by user ({detail})";
        }

        return detail;
    }

    private static bool IsLikelyPackagedProcess()
    {
        string? packageFamilyName = Environment.GetEnvironmentVariable("PACKAGE_FAMILY_NAME");
        if (!string.IsNullOrWhiteSpace(packageFamilyName))
        {
            return true;
        }

        string? processPath = Environment.ProcessPath;
        return !string.IsNullOrWhiteSpace(processPath)
               && processPath.Contains(@"\WindowsApps\", StringComparison.OrdinalIgnoreCase);
    }

    private static ulong NowMs()
    {
        return (ulong)Math.Max(0L, DateTimeOffset.UtcNow.ToUnixTimeMilliseconds());
    }

    private void EnqueueWarning(string message)
    {
        if (!string.IsNullOrWhiteSpace(message))
        {
            _pendingWarnings.Enqueue(message);
        }
    }

    private sealed record ElevatedSnapshotFile
    {
        public string Token { get; init; } = string.Empty;

        public ulong Seq { get; init; }

        public IReadOnlyList<ProcessSample> Rows { get; init; } = [];
    }
}
