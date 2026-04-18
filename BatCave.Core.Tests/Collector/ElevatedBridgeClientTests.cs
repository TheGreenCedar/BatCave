using BatCave.Core.Collector;
using BatCave.Core.Domain;
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
        ElevatedBridgeClient client = ElevatedBridgeClient.CreateForTest(dataFile, stopFile, "token", launchedMs: 0, nowMs: () => 20_000);
        BridgePollResult result = client.PollRows();

        Assert.Equal(BridgePollState.Faulted, result.State);
        Assert.Contains("startup grace", result.Reason ?? string.Empty, StringComparison.OrdinalIgnoreCase);
    }

    [Fact]
    public void PollRows_FaultsWhenSnapshotStreamGoesStale()
    {
        using TestTempDirectory tempDir = TestTempDirectory.Create("batcave-bridge-tests");
        string dataFile = Path.Combine(tempDir.DirectoryPath, "snapshot.json");
        string stopFile = Path.Combine(tempDir.DirectoryPath, "stop.signal");
        File.WriteAllText(dataFile, JsonSerializer.Serialize(new
        {
            Token = "token",
            Seq = 1UL,
            Rows = Array.Empty<object>(),
        }));

        ulong nowMs = 1_000;
        ElevatedBridgeClient client = ElevatedBridgeClient.CreateForTest(dataFile, stopFile, "token", launchedMs: 0, nowMs: () => nowMs);
        BridgePollResult initial = client.PollRows();
        Assert.Equal(BridgePollState.Rows, initial.State);

        nowMs = 6_000;
        BridgePollResult stale = client.PollRows();
        Assert.Equal(BridgePollState.Faulted, stale.State);
        Assert.Contains("stalled", stale.Reason ?? string.Empty, StringComparison.OrdinalIgnoreCase);
    }

    [Fact]
    public void PollRows_WhenSnapshotParseFails_QueuesWarning()
    {
        using TestTempDirectory tempDir = TestTempDirectory.Create("batcave-bridge-tests");
        string dataFile = Path.Combine(tempDir.DirectoryPath, "snapshot.json");
        string stopFile = Path.Combine(tempDir.DirectoryPath, "stop.signal");
        File.WriteAllText(dataFile, "{ invalid-json");

        ElevatedBridgeClient client = ElevatedBridgeClient.CreateForTest(dataFile, stopFile, "token", launchedMs: 1_000, nowMs: () => 1_000);

        BridgePollResult result = client.PollRows();
        string? warning = client.TakeWarning();

        Assert.Equal(BridgePollState.Pending, result.State);
        Assert.NotNull(warning);
        Assert.Contains("snapshot_parse_failed", warning, StringComparison.OrdinalIgnoreCase);
    }

    [Fact]
    public void PollRows_WhenFreshSnapshotArrivesAfterFault_Recovers()
    {
        using TestTempDirectory tempDir = TestTempDirectory.Create("batcave-bridge-tests");
        string dataFile = Path.Combine(tempDir.DirectoryPath, "snapshot.json");
        string stopFile = Path.Combine(tempDir.DirectoryPath, "stop.signal");
        ulong nowMs = 20_000;
        ElevatedBridgeClient client = ElevatedBridgeClient.CreateForTest(dataFile, stopFile, "token", launchedMs: 0, nowMs: () => nowMs);
        Assert.Equal(BridgePollState.Faulted, client.PollRows().State);

        ProcessSample recovered = new()
        {
            Pid = 901,
            Seq = 2,
            TsMs = 2,
            ParentPid = 1,
            StartTimeMs = 9_010,
            Name = "bridge-recovered",
            CpuPct = 4,
            RssBytes = 1024,
            PrivateBytes = 512,
            IoReadBps = 8,
            IoWriteBps = 9,
            OtherIoBps = 10,
            Threads = 2,
            Handles = 3,
            AccessState = AccessState.Full,
        };
        File.WriteAllText(dataFile, JsonSerializer.Serialize(new
        {
            Token = "token",
            Seq = 2UL,
            Rows = new[] { recovered },
        }));

        nowMs = 20_100;
        BridgePollResult result = client.PollRows();

        Assert.Equal(BridgePollState.Rows, result.State);
        ProcessSample row = Assert.Single(result.Rows);
        Assert.Equal(recovered.Identity(), row.Identity());
    }

    [Fact]
    public void PollRows_WhenSameFaultPersists_QueuesWarningOnlyOnceUntilRecovery()
    {
        using TestTempDirectory tempDir = TestTempDirectory.Create("batcave-bridge-tests");
        string dataFile = Path.Combine(tempDir.DirectoryPath, "snapshot.json");
        string stopFile = Path.Combine(tempDir.DirectoryPath, "stop.signal");
        ulong nowMs = 20_000;
        ElevatedBridgeClient client = ElevatedBridgeClient.CreateForTest(dataFile, stopFile, "token", launchedMs: 0, nowMs: () => nowMs);

        BridgePollResult first = client.PollRows();
        string? firstWarning = client.TakeWarning();

        nowMs = 20_500;
        BridgePollResult second = client.PollRows();
        string? secondWarning = client.TakeWarning();

        Assert.Equal(BridgePollState.Faulted, first.State);
        Assert.Equal(BridgePollState.Faulted, second.State);
        Assert.NotNull(firstWarning);
        Assert.Null(secondWarning);
    }

    [Fact]
    public void PollRows_WhenStaleStreamPersists_QueuesWarningOnlyOnce()
    {
        using TestTempDirectory tempDir = TestTempDirectory.Create("batcave-bridge-tests");
        string dataFile = Path.Combine(tempDir.DirectoryPath, "snapshot.json");
        string stopFile = Path.Combine(tempDir.DirectoryPath, "stop.signal");
        File.WriteAllText(dataFile, JsonSerializer.Serialize(new
        {
            Token = "token",
            Seq = 1UL,
            Rows = Array.Empty<object>(),
        }));

        ulong nowMs = 1_000;
        ElevatedBridgeClient client = ElevatedBridgeClient.CreateForTest(dataFile, stopFile, "token", launchedMs: 0, nowMs: () => nowMs);
        Assert.Equal(BridgePollState.Rows, client.PollRows().State);

        nowMs = 6_000;
        BridgePollResult first = client.PollRows();
        string? firstWarning = client.TakeWarning();

        nowMs = 6_500;
        BridgePollResult second = client.PollRows();
        string? secondWarning = client.TakeWarning();

        Assert.Equal(BridgePollState.Faulted, first.State);
        Assert.Equal(BridgePollState.Faulted, second.State);
        Assert.NotNull(firstWarning);
        Assert.Null(secondWarning);
    }
}
