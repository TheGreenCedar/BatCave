using BatCave.Runtime.Contracts;
using BatCave.Runtime.Serialization;
using System.Text.Json;

namespace BatCave.Runtime.Collectors;

public static class ElevatedBridgeHelper
{
    private static readonly TimeSpan DefaultSampleInterval = TimeSpan.FromSeconds(1);

    public static int RunElevatedHelper(string dataFile, string stopFile, string token, CancellationToken ct)
    {
        return RunElevatedHelper(
            dataFile,
            stopFile,
            token,
            new WindowsProcessCollector(),
            DefaultSampleInterval,
            ct);
    }

    internal static int RunElevatedHelper(
        string dataFile,
        string stopFile,
        string token,
        IProcessCollector collector,
        TimeSpan sampleInterval,
        CancellationToken ct)
    {
        string? parentDirectory = Path.GetDirectoryName(dataFile);
        if (!string.IsNullOrWhiteSpace(parentDirectory))
        {
            Directory.CreateDirectory(parentDirectory);
        }

        string tempFile = dataFile + ".tmp";
        ulong seq = 0;

        while (!ct.IsCancellationRequested)
        {
            if (File.Exists(stopFile))
            {
                break;
            }

            seq++;
            IReadOnlyList<ProcessSample> rows = collector.Collect(seq);
            ElevatedSnapshotFile payload = new()
            {
                Token = token,
                Seq = seq,
                Rows = rows.ToArray(),
            };

            try
            {
                string json = JsonSerializer.Serialize(payload, JsonDefaults.SnakeCase);
                WriteSnapshotAtomically(dataFile, tempFile, json);
            }
            catch
            {
                // Keep the helper resilient; the next tick can repair a transient file race.
            }

            if (ct.WaitHandle.WaitOne(sampleInterval))
            {
                break;
            }
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
                File.Delete(dataFile);
                File.Move(tempFile, dataFile, overwrite: true);
            }
            else
            {
                throw;
            }
        }
    }

    private sealed record ElevatedSnapshotFile
    {
        public string Token { get; init; } = string.Empty;

        public ulong Seq { get; init; }

        public IReadOnlyList<ProcessSample> Rows { get; init; } = [];
    }
}
