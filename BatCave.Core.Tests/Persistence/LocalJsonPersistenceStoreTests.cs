using BatCave.Core.Domain;
using BatCave.Core.Persistence;

namespace BatCave.Core.Tests.Persistence;

public class LocalJsonPersistenceStoreTests
{
    [Fact]
    public async Task Settings_RoundTrip_UsesConfiguredBaseDirectory()
    {
        string baseDir = CreateTempDirectory();
        try
        {
            LocalJsonPersistenceStore store = new(baseDir);
            UserSettings settings = new()
            {
                SortCol = SortColumn.Name,
                SortDir = SortDirection.Asc,
                FilterText = "svc",
                AdminMode = true,
            };

            await store.SaveSettingsAsync(settings, CancellationToken.None);
            UserSettings? loaded = store.LoadSettings();

            Assert.NotNull(loaded);
            Assert.Equal(settings.SortCol, loaded!.SortCol);
            Assert.Equal(settings.SortDir, loaded.SortDir);
            Assert.Equal(settings.FilterText, loaded.FilterText);
            Assert.Equal(settings.AdminMode, loaded.AdminMode);

            string settingsPath = Path.Combine(baseDir, "settings.json");
            Assert.True(File.Exists(settingsPath));
            Assert.StartsWith(baseDir, settingsPath, StringComparison.OrdinalIgnoreCase);
        }
        finally
        {
            DeleteDirectory(baseDir);
        }
    }

    [Fact]
    public async Task WarmCache_And_Diagnostics_WriteLocalFilesOnly()
    {
        string baseDir = CreateTempDirectory();
        try
        {
            LocalJsonPersistenceStore store = new(baseDir);
            WarmCache cache = new()
            {
                Seq = 42,
                Rows =
                [
                    new ProcessSample
                    {
                        Seq = 42,
                        TsMs = 100,
                        Pid = 123,
                        ParentPid = 1,
                        StartTimeMs = 20,
                        Name = "demo.exe",
                        CpuPct = 1.2,
                        RssBytes = 10,
                        PrivateBytes = 5,
                        IoReadBps = 1,
                        IoWriteBps = 2,
                        NetBps = 3,
                        Threads = 4,
                        Handles = 5,
                        AccessState = AccessState.Full,
                    },
                ],
            };

            await store.SaveWarmCacheAsync(cache, CancellationToken.None);
            await store.AppendDiagnosticAsync("runtime_tick", new { seq = 42 }, CancellationToken.None);

            WarmCache? loadedCache = store.LoadWarmCache();
            Assert.NotNull(loadedCache);
            Assert.Equal(42UL, loadedCache!.Seq);
            Assert.Single(loadedCache.Rows);

            string logsDir = Path.Combine(baseDir, "logs");
            string[] logFiles = Directory.GetFiles(logsDir, "*.jsonl");
            Assert.Single(logFiles);
            Assert.StartsWith(baseDir, logFiles[0], StringComparison.OrdinalIgnoreCase);

            string content = await File.ReadAllTextAsync(logFiles[0]);
            Assert.Contains("runtime_tick", content, StringComparison.OrdinalIgnoreCase);
            Assert.DoesNotContain("http://", content, StringComparison.OrdinalIgnoreCase);
            Assert.DoesNotContain("https://", content, StringComparison.OrdinalIgnoreCase);
        }
        finally
        {
            DeleteDirectory(baseDir);
        }
    }

    private static string CreateTempDirectory()
    {
        string path = Path.Combine(Path.GetTempPath(), $"batcave-core-tests-{Guid.NewGuid():N}");
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
