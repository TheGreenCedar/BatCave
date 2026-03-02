using System.Diagnostics;
using System.Text.Json;
using BatCave.Core.Domain;
using BatCave.Core.Persistence;

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

    public IReadOnlyList<ProcessSample> Rows { get; init; } = [];

    public string? Reason { get; init; }

    public static BridgePollResult RowsResult(IReadOnlyList<ProcessSample> rows)
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
    internal static Func<ulong>? NowMsOverrideForTest { get; set; }

    private readonly string _dataFile;
    private readonly string _stopFile;
    private readonly string _token;

    private readonly uint _helperPid;
    private readonly ulong _launchedMs;

    private ulong _lastSeq;
    private List<ProcessSample> _lastRows = [];
    private ulong? _lastSuccessMs;
    private string? _faultReason;

    private ElevatedBridgeClient(string dataFile, string stopFile, string token, uint helperPid, ulong? launchedMs = null)
    {
        _dataFile = dataFile;
        _stopFile = stopFile;
        _token = token;
        _helperPid = helperPid;
        _launchedMs = launchedMs ?? NowMs();
    }

    public static async Task<ElevatedBridgeClient> LaunchAsync(CancellationToken ct)
    {
        string bridgeDir = Path.Combine(LocalJsonPersistenceStore.DefaultBaseDirectory(), "elevated-bridge");
        Directory.CreateDirectory(bridgeDir);

        string runId = $"{Environment.ProcessId}-{NowMs()}";
        string token = $"{runId}-token";
        string dataFile = Path.Combine(bridgeDir, $"snapshot-{runId}.json");
        string stopFile = Path.Combine(bridgeDir, $"stop-{runId}.signal");

        TryDelete(dataFile);
        TryDelete(stopFile);

        uint helperPid = await LaunchElevatedHelperProcessAsync(dataFile, stopFile, token, ct).ConfigureAwait(false);
        return new ElevatedBridgeClient(dataFile, stopFile, token, helperPid);
    }

    internal static ElevatedBridgeClient CreateForTest(string dataFile, string stopFile, string token, ulong launchedMs)
    {
        return new ElevatedBridgeClient(dataFile, stopFile, token, helperPid: 0, launchedMs: launchedMs);
    }

    public BridgePollResult PollRows()
    {
        if (!string.IsNullOrWhiteSpace(_faultReason))
        {
            return BridgePollResult.Faulted(_faultReason!);
        }

        ulong now = NowMs();
        TryReadLatestSnapshot(now);

        if (_lastSuccessMs is null)
        {
            ulong startupElapsed = now - _launchedMs;
            if (startupElapsed > BridgeStartupGraceMs)
            {
                return SetFault($"no elevated bridge snapshot received within startup grace window ({BridgeStartupGraceMs} ms)");
            }

            return BridgePollResult.Pending();
        }

        ulong staleFor = now - _lastSuccessMs.Value;
        if (staleFor > BridgeStaleTimeoutMs)
        {
            return SetFault($"elevated bridge snapshot stream stalled for {staleFor} ms");
        }

        return BridgePollResult.RowsResult(_lastRows);
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
        catch
        {
            // ignore one-off parse errors and continue polling
        }
    }

    private BridgePollResult SetFault(string reason)
    {
        _faultReason = reason;
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
            throw new InvalidOperationException($"failed to start elevated helper: {detail}");
        }

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

    private static ulong NowMs()
    {
        if (NowMsOverrideForTest is not null)
        {
            return NowMsOverrideForTest();
        }

        return (ulong)DateTimeOffset.UtcNow.ToUnixTimeMilliseconds();
    }

    private sealed record ElevatedSnapshotFile
    {
        public string Token { get; init; } = string.Empty;

        public ulong Seq { get; init; }

        public List<ProcessSample> Rows { get; init; } = [];
    }
}
