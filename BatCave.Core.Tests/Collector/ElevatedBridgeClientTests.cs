using BatCave.Core.Collector;
using BatCave.Core.Tests.TestSupport;
using System.Text.Json;

namespace BatCave.Core.Tests.Collector;

public class ElevatedBridgeClientTests
{
    [Fact]
    public void WriteSnapshotAtomically_WritesTargetAndCleansTemp()
    {
        using TestTempDirectory tempDir = TestTempDirectory.Create("batcave-bridge-tests");
        string dataFile = Path.Combine(tempDir.DirectoryPath, "snapshot.json");
        string tempFile = dataFile + ".tmp";

        ElevatedBridgeClient.WriteSnapshotAtomically(dataFile, tempFile, "{\"seq\":1}");

        Assert.True(File.Exists(dataFile));
        Assert.False(File.Exists(tempFile));
        Assert.Equal("{\"seq\":1}", File.ReadAllText(dataFile));
    }

    [Fact]
    public void PollRows_FaultsWhenStartupGraceExpiresWithoutSnapshot()
    {
        using TestTempDirectory tempDir = TestTempDirectory.Create("batcave-bridge-tests");
        string dataFile = Path.Combine(tempDir.DirectoryPath, "snapshot.json");
        string stopFile = Path.Combine(tempDir.DirectoryPath, "stop.signal");
        ElevatedBridgeClient.NowMsOverrideForTest = () => 20_000;
        try
        {
            ElevatedBridgeClient client = ElevatedBridgeClient.CreateForTest(dataFile, stopFile, "token", launchedMs: 0);
            BridgePollResult result = client.PollRows();

            Assert.Equal(BridgePollState.Faulted, result.State);
            Assert.Contains("startup grace", result.Reason ?? string.Empty, StringComparison.OrdinalIgnoreCase);
        }
        finally
        {
            ElevatedBridgeClient.NowMsOverrideForTest = null;
        }
    }

    [Fact]
    public void PollRows_FaultsWhenSnapshotStreamGoesStale()
    {
        using TestTempDirectory tempDir = TestTempDirectory.Create("batcave-bridge-tests");
        string dataFile = Path.Combine(tempDir.DirectoryPath, "snapshot.json");
        string stopFile = Path.Combine(tempDir.DirectoryPath, "stop.signal");
        try
        {
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
        }
    }

    [Fact]
    public void PollRows_WhenSnapshotParseFails_QueuesWarning()
    {
        using TestTempDirectory tempDir = TestTempDirectory.Create("batcave-bridge-tests");
        string dataFile = Path.Combine(tempDir.DirectoryPath, "snapshot.json");
        string stopFile = Path.Combine(tempDir.DirectoryPath, "stop.signal");
        ElevatedBridgeClient.NowMsOverrideForTest = () => 1_000;
        try
        {
            File.WriteAllText(dataFile, "{ invalid-json");

            ElevatedBridgeClient client = ElevatedBridgeClient.CreateForTest(dataFile, stopFile, "token", launchedMs: 1_000);

            BridgePollResult result = client.PollRows();
            string? warning = client.TakeWarning();

            Assert.Equal(BridgePollState.Pending, result.State);
            Assert.NotNull(warning);
            Assert.Contains("snapshot_parse_failed", warning, StringComparison.OrdinalIgnoreCase);
        }
        finally
        {
            ElevatedBridgeClient.NowMsOverrideForTest = null;
        }
    }
}
