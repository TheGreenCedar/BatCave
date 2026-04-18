using BatCave.Core.Domain;
using BatCave.Core.Persistence;
using System.Diagnostics;
using System.Text.Json;

namespace BatCave.Core.Collector;

public enum BridgePollState
{
    Rows,
    Pending,
    Faulted,
}

public sealed record BridgePollResult
{
    public BridgePollState State { get; init; }

    public List<ProcessSample> Rows { get; init; } = [];

    public string? Reason { get; init; }

    public static BridgePollResult RowsResult(List<ProcessSample> rows)
    {
        return new BridgePollResult
        {
            State = BridgePollState.Rows,
            Rows = rows,
        };
    }

    public static BridgePollResult Pending()
    {
        return new BridgePollResult
        {
            State = BridgePollState.Pending,
        };
    }

    public static BridgePollResult Faulted(string reason)
    {
        return new BridgePollResult
        {
            State = BridgePollState.Faulted,
            Reason = reason,
        };
    }
}

public sealed class ElevatedBridgeClient : IDisposable
{
    private const ulong BridgeStaleTimeoutMs = 4_000;
    private const ulong BridgeStartupGraceMs = 10_000;

    private readonly string _dataFile;
    private readonly string _stopFile;
    private readonly string _token;

    private readonly uint _helperPid;
    private readonly Func<ulong> _nowMs;
    private readonly ulong _launchedMs;

    private ulong _lastSeq;
    private List<ProcessSample> _lastRows = [];
    private ulong? _lastSuccessMs;
    private string? _activeFaultReason;
    private readonly Queue<string> _pendingWarnings = [];

    private ElevatedBridgeClient(string dataFile, string stopFile, string token, uint helperPid, ulong? launchedMs = null, Func<ulong>? nowMs = null)
    {
        _dataFile = dataFile;
        _stopFile = stopFile;
        _token = token;
        _helperPid = helperPid;
        _nowMs = nowMs ?? NowMs;
        _launchedMs = launchedMs ?? _nowMs();
    }

    public static async Task<ElevatedBridgeClient> LaunchAsync(CancellationToken ct)
    {
        (string dataFile, string stopFile, string token) = BuildBridgeFiles();

        TryDelete(dataFile);
        TryDelete(stopFile);

        uint helperPid = await LaunchElevatedHelperProcessAsync(dataFile, stopFile, token, ct).ConfigureAwait(false);
        return new ElevatedBridgeClient(dataFile, stopFile, token, helperPid);
    }

    internal static ElevatedBridgeClient CreateForTest(string dataFile, string stopFile, string token, ulong launchedMs, Func<ulong>? nowMs = null)
    {
        return new ElevatedBridgeClient(dataFile, stopFile, token, helperPid: 0, launchedMs: launchedMs, nowMs: nowMs);
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

    private void TryReadLatestSnapshot(ulong now)
    {
        if (!File.Exists(_dataFile))
        {
            return;
        }

        try
        {
            string content = File.ReadAllText(_dataFile);
            ElevatedSnapshotFile? snapshot = JsonSerializer.Deserialize<ElevatedSnapshotFile>(content);
            if (snapshot is null)
            {
                return;
            }

            if (string.Equals(snapshot.Token, _token, StringComparison.Ordinal) && snapshot.Seq > _lastSeq)
            {
                _lastSeq = snapshot.Seq;
                _lastRows = snapshot.Rows;
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

    public void Dispose()
    {
        try
        {
            File.WriteAllText(_stopFile, "stop");
        }
        catch
        {
            // best effort cleanup
        }
    }

    public static int RunElevatedHelper(string dataFile, string stopFile, string token, CancellationToken ct)
    {
        string? parentDirectory = Path.GetDirectoryName(dataFile);
        if (!string.IsNullOrWhiteSpace(parentDirectory))
        {
            Directory.CreateDirectory(parentDirectory);
        }

        WindowsProcessCollector collector = new();
        string tempFile = dataFile + ".tmp";
        ulong seq = 0;

        while (!ct.IsCancellationRequested)
        {
            if (File.Exists(stopFile))
            {
                break;
            }

            seq++;
            IReadOnlyList<ProcessSample> rows = collector.CollectTick(seq);
            ElevatedSnapshotFile payload = new()
            {
                Token = token,
                Seq = seq,
                Rows = rows.ToList(),
            };

            try
            {
                string json = JsonSerializer.Serialize(payload);
                WriteSnapshotAtomically(dataFile, tempFile, json);
            }
            catch
            {
                // keep helper loop resilient and continue next tick
            }

            Thread.Sleep(TimeSpan.FromSeconds(1));
        }

        return 0;
    }

    internal static void WriteSnapshotAtomically(string dataFile, string tempFile, string payload)
    {
        File.WriteAllText(tempFile, payload);
        try
        {
            File.Move(tempFile, dataFile, overwrite: true);
        }
        catch
        {
            if (File.Exists(dataFile))
            {
                TryDelete(dataFile);
                File.Move(tempFile, dataFile, overwrite: true);
            }
            else
            {
                throw;
            }
        }
    }

    private static async Task<uint> LaunchElevatedHelperProcessAsync(
        string dataFile,
        string stopFile,
        string token,
        CancellationToken ct)
    {
        bool isPackagedProcess = IsLikelyPackagedProcess();

        string? executable = Environment.ProcessPath;
        if (string.IsNullOrWhiteSpace(executable))
        {
            throw new InvalidOperationException("failed to resolve current executable path");
        }

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
            string detail = string.IsNullOrWhiteSpace(stderr)
                ? "unknown elevation start failure"
                : stderr.Trim();
            string normalizedDetail = NormalizeElevationFailureDetail(detail);
            throw new InvalidOperationException(
                $"failed to start elevated helper (packaged={isPackagedProcess}): {normalizedDetail}");
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
            // best effort
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

    private static (string DataFile, string StopFile, string Token) BuildBridgeFiles()
    {
        string bridgeDir = Path.Combine(LocalJsonPersistenceStore.DefaultBaseDirectory(), "elevated-bridge");
        Directory.CreateDirectory(bridgeDir);

        string runId = $"{Environment.ProcessId}-{NowMs()}";
        string token = $"{runId}-token";
        string dataFile = Path.Combine(bridgeDir, $"snapshot-{runId}.json");
        string stopFile = Path.Combine(bridgeDir, $"stop-{runId}.signal");
        return (dataFile, stopFile, token);
    }

    private static ulong NowMs()
    {
        return (ulong)DateTimeOffset.UtcNow.ToUnixTimeMilliseconds();
    }

    private void EnqueueWarning(string message)
    {
        if (string.IsNullOrWhiteSpace(message))
        {
            return;
        }

        _pendingWarnings.Enqueue(message);
    }

    private sealed record ElevatedSnapshotFile
    {
        public string Token { get; init; } = string.Empty;

        public ulong Seq { get; init; }

        public List<ProcessSample> Rows { get; init; } = [];
    }
}
