using System.Text.Json;
using BatCave.Core.Collector;

namespace BatCave.Core.Tests.Collector;

public class ElevatedBridgeClientTests
{
    [Fact]
    public void WriteSnapshotAtomically_WritesTargetAndCleansTemp()
    {
        string dir = CreateTempDirectory();
        try
        {
            string dataFile = Path.Combine(dir, "snapshot.json");
            string tempFile = dataFile + ".tmp";

            ElevatedBridgeClient.WriteSnapshotAtomically(dataFile, tempFile, "{\"seq\":1}");

            Assert.True(File.Exists(dataFile));
            Assert.False(File.Exists(tempFile));
            Assert.Equal("{\"seq\":1}", File.ReadAllText(dataFile));
        }
        finally
        {
            DeleteDirectory(dir);
        }
    }

    [Fact]
    public void PollRows_FaultsWhenStartupGraceExpiresWithoutSnapshot()
    {
        string dir = CreateTempDirectory();
        try
        {
            string dataFile = Path.Combine(dir, "snapshot.json");
            string stopFile = Path.Combine(dir, "stop.signal");
            ElevatedBridgeClient.NowMsOverrideForTest = () => 20_000;

            ElevatedBridgeClient client = ElevatedBridgeClient.CreateForTest(dataFile, stopFile, "token", launchedMs: 0);
            BridgePollResult result = client.PollRows();

            Assert.Equal(BridgePollState.Faulted, result.State);
            Assert.Contains("startup grace", result.Reason ?? string.Empty, StringComparison.OrdinalIgnoreCase);
        }
        finally
        {
            ElevatedBridgeClient.NowMsOverrideForTest = null;
            DeleteDirectory(dir);
        }
    }

    [Fact]
    public void PollRows_FaultsWhenSnapshotStreamGoesStale()
    {
        string dir = CreateTempDirectory();
        try
        {
            string dataFile = Path.Combine(dir, "snapshot.json");
            string stopFile = Path.Combine(dir, "stop.signal");

            File.WriteAllText(dataFile, JsonSerializer.Serialize(new
            {
                Token = "token",
                Seq = 1UL,
                Rows = Array.Empty<object>(),
            }));

            ulong nowMs = 1_000;
            ElevatedBridgeClient.NowMsOverrideForTest = () => nowMs;

            ElevatedBridgeClient client = ElevatedBridgeClient.CreateForTest(dataFile, stopFile, "token", launchedMs: 0);
            BridgePollResult initial = client.PollRows();
            Assert.Equal(BridgePollState.Rows, initial.State);

            nowMs = 6_000;
            BridgePollResult stale = client.PollRows();
            Assert.Equal(BridgePollState.Faulted, stale.State);
            Assert.Contains("stalled", stale.Reason ?? string.Empty, StringComparison.OrdinalIgnoreCase);
        }
        finally
        {
            ElevatedBridgeClient.NowMsOverrideForTest = null;
            DeleteDirectory(dir);
        }
    }

    private static string CreateTempDirectory()
    {
        string path = Path.Combine(Path.GetTempPath(), $"batcave-bridge-tests-{Guid.NewGuid():N}");
        Directory.CreateDirectory(path);
        return path;
    }

    private static void DeleteDirectory(string path)
    {
        try
        {
            if (Directory.Exists(path))
            {
                Directory.Delete(path, recursive: true);
            }
        }
        catch
        {
            // best effort cleanup
        }
    }
}
