using System.Reflection;
using BatCave.Core.Collector;

namespace BatCave.Core.Tests.Collector;

public class DefaultProcessCollectorTests
{
    [Fact]
    public void CollectTick_WhenBridgeParseFails_QueuesBridgeWarning()
    {
        string dir = CreateTempDirectory();
        try
        {
            string dataFile = Path.Combine(dir, "snapshot.json");
            string stopFile = Path.Combine(dir, "stop.signal");
            File.WriteAllText(dataFile, "{ not-json");

            ElevatedBridgeClient.NowMsOverrideForTest = () => 1_000;
            ElevatedBridgeClient bridge = ElevatedBridgeClient.CreateForTest(dataFile, stopFile, "token", launchedMs: 1_000);

            using DefaultProcessCollector collector = new(adminMode: false);
            InjectBridgeForTest(collector, bridge);

            _ = collector.CollectTick(seq: 1);

            string? warning = collector.TakeWarning();
            Assert.False(string.IsNullOrWhiteSpace(warning));
            Assert.Contains("snapshot_parse_failed", warning ?? string.Empty, StringComparison.OrdinalIgnoreCase);
        }
        finally
        {
            ElevatedBridgeClient.NowMsOverrideForTest = null;
            DeleteDirectory(dir);
        }
    }

    [Fact]
    public void CollectTick_WhenBridgeFaults_QueuesFallbackWarning()
    {
        string dir = CreateTempDirectory();
        try
        {
            string dataFile = Path.Combine(dir, "snapshot.json");
            string stopFile = Path.Combine(dir, "stop.signal");

            ElevatedBridgeClient.NowMsOverrideForTest = () => 20_000;
            ElevatedBridgeClient bridge = ElevatedBridgeClient.CreateForTest(dataFile, stopFile, "token", launchedMs: 0);

            using DefaultProcessCollector collector = new(adminMode: false);
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
            DeleteDirectory(dir);
        }
    }

    private static void InjectBridgeForTest(DefaultProcessCollector collector, ElevatedBridgeClient bridge)
    {
        FieldInfo? bridgeField = typeof(DefaultProcessCollector).GetField("_bridge", BindingFlags.Instance | BindingFlags.NonPublic);
        Assert.NotNull(bridgeField);
        bridgeField!.SetValue(collector, bridge);
    }

    private static string CreateTempDirectory()
    {
        string path = Path.Combine(Path.GetTempPath(), $"batcave-default-collector-tests-{Guid.NewGuid():N}");
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
