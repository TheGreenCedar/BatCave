using BatCave.Core.Collector;
using BatCave.Core.Tests.TestSupport;
using System.Reflection;

namespace BatCave.Core.Tests.Collector;

public class DefaultProcessCollectorTests
{
    [Fact]
    public void CollectTick_WhenBridgeParseFails_QueuesBridgeWarning()
    {
        using TestTempDirectory tempDir = TestTempDirectory.Create("batcave-default-collector-tests");
        string dataFile = Path.Combine(tempDir.DirectoryPath, "snapshot.json");
        string stopFile = Path.Combine(tempDir.DirectoryPath, "stop.signal");
        ElevatedBridgeClient.NowMsOverrideForTest = () => 1_000;
        try
        {
            File.WriteAllText(dataFile, "{ not-json");

            ElevatedBridgeClient bridge = ElevatedBridgeClient.CreateForTest(dataFile, stopFile, "token", launchedMs: 1_000);

            using DefaultProcessCollector collector = new();
            InjectBridgeForTest(collector, bridge);

            _ = collector.CollectTick(seq: 1);

            string? warning = collector.TakeWarning();
            Assert.False(string.IsNullOrWhiteSpace(warning));
            Assert.Contains("snapshot_parse_failed", warning ?? string.Empty, StringComparison.OrdinalIgnoreCase);
        }
        finally
        {
            ElevatedBridgeClient.NowMsOverrideForTest = null;
        }
    }

    [Fact]
    public void CollectTick_WhenBridgeFaults_QueuesFallbackWarning()
    {
        using TestTempDirectory tempDir = TestTempDirectory.Create("batcave-default-collector-tests");
        string dataFile = Path.Combine(tempDir.DirectoryPath, "snapshot.json");
        string stopFile = Path.Combine(tempDir.DirectoryPath, "stop.signal");
        ElevatedBridgeClient.NowMsOverrideForTest = () => 20_000;
        try
        {
            ElevatedBridgeClient bridge = ElevatedBridgeClient.CreateForTest(dataFile, stopFile, "token", launchedMs: 0);

            using DefaultProcessCollector collector = new();
            InjectBridgeForTest(collector, bridge);

            _ = collector.CollectTick(seq: 1);

            string? warning = collector.TakeWarning();
            Assert.False(string.IsNullOrWhiteSpace(warning));
            Assert.True(
                warning!.Contains("elevated_bridge_faulted", StringComparison.OrdinalIgnoreCase)
                || warning.Contains("startup grace", StringComparison.OrdinalIgnoreCase));
        }
        finally
        {
            ElevatedBridgeClient.NowMsOverrideForTest = null;
        }
    }

    private static void InjectBridgeForTest(DefaultProcessCollector collector, ElevatedBridgeClient bridge)
    {
        FieldInfo? bridgeField = typeof(DefaultProcessCollector).GetField("_bridge", BindingFlags.Instance | BindingFlags.NonPublic);
        Assert.NotNull(bridgeField);
        bridgeField!.SetValue(collector, bridge);
    }

}


