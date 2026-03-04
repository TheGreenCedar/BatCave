using BatCave.Core.Abstractions;
using BatCave.Core.Domain;
using BatCave.Core.Serialization;
using System.Text;
using System.Text.Json;

namespace BatCave.Core.Persistence;

public sealed class LocalJsonPersistenceStore : IPersistenceStore
{
    private readonly JsonSerializerOptions _prettyJson = JsonDefaults.SnakeCase;
    private readonly JsonSerializerOptions _compactJson = new(JsonDefaults.SnakeCase)
    {
        WriteIndented = false,
    };

    private readonly string _settingsPath;
    private readonly string _warmCachePath;
    private readonly string _logsDirectory;
    private readonly object _warningSync = new();
    private readonly Queue<string> _pendingWarnings = new();

    public LocalJsonPersistenceStore(string? baseDirectory = null)
    {
        BaseDirectory = baseDirectory ?? DefaultBaseDirectory();
        _settingsPath = Path.Combine(BaseDirectory, "settings.json");
        _warmCachePath = Path.Combine(BaseDirectory, "warm-cache.json");
        _logsDirectory = Path.Combine(BaseDirectory, "logs");

        EnsureBaseDirectories();
    }

    public string BaseDirectory { get; }

    public UserSettings? LoadSettings()
    {
        return LoadJson<UserSettings>(_settingsPath);
    }

    public async Task SaveSettingsAsync(UserSettings settings, CancellationToken ct)
    {
        await ExecuteWithWarningAsync(
            operation: "save_settings",
            path: _settingsPath,
            action: () => WriteJsonAtomicAsync(_settingsPath, settings, _prettyJson, ct)).ConfigureAwait(false);
    }

    public WarmCache? LoadWarmCache()
    {
        return LoadJson<WarmCache>(_warmCachePath);
    }

    public async Task SaveWarmCacheAsync(WarmCache cache, CancellationToken ct)
    {
        await ExecuteWithWarningAsync(
            operation: "save_warm_cache",
            path: _warmCachePath,
            action: () => WriteJsonAtomicAsync(_warmCachePath, cache, _compactJson, ct)).ConfigureAwait(false);
    }

    public async Task AppendDiagnosticAsync(string category, object payload, CancellationToken ct)
    {
        await ExecuteWithWarningAsync(
            operation: "append_diagnostic",
            path: _logsDirectory,
            action: () => AppendDiagnosticCoreAsync(category, payload, ct)).ConfigureAwait(false);
    }

    public string? TakeWarning()
    {
        lock (_warningSync)
        {
            return _pendingWarnings.Count > 0 ? _pendingWarnings.Dequeue() : null;
        }
    }

    public static string DefaultBaseDirectory()
    {
        string localAppData = Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData);
        if (string.IsNullOrWhiteSpace(localAppData))
        {
            localAppData = ".";
        }

        return Path.Combine(localAppData, "BatCaveMonitor");
    }

    private T? LoadJson<T>(string path)
    {
        try
        {
            if (!File.Exists(path))
            {
                return default;
            }

            string content = File.ReadAllText(path, Encoding.UTF8);
            return JsonSerializer.Deserialize<T>(content, _compactJson);
        }
        catch (Exception ex)
        {
            EnqueueWarning("load_json", path, ex);
            return default;
        }
    }

    private static async Task WriteJsonAtomicAsync<T>(
        string path,
        T value,
        JsonSerializerOptions options,
        CancellationToken ct)
    {
        string tempPath = path + ".tmp";
        string json = JsonSerializer.Serialize(value, options);
        await File.WriteAllTextAsync(tempPath, json, Encoding.UTF8, ct).ConfigureAwait(false);

        File.Move(tempPath, path, overwrite: true);
    }

    private async Task AppendDiagnosticCoreAsync(string category, object payload, CancellationToken ct)
    {
        EnsureLogDirectory();
        string logPath = ResolveDailyLogPath();
        DiagnosticEntry entry = CreateDiagnosticEntry(category, payload);
        string line = JsonSerializer.Serialize(entry, _compactJson);
        await File.AppendAllTextAsync(logPath, line + Environment.NewLine, Encoding.UTF8, ct).ConfigureAwait(false);
        RotateLogFiles(maxFiles: 14);
    }

    private async Task ExecuteWithWarningAsync(string operation, string path, Func<Task> action)
    {
        try
        {
            await action().ConfigureAwait(false);
        }
        catch (Exception ex)
        {
            EnqueueWarning(operation, path, ex);
            throw;
        }
    }

    private void RotateLogFiles(int maxFiles)
    {
        FileInfo[] files = new DirectoryInfo(_logsDirectory)
            .GetFiles("*.jsonl")
            .OrderByDescending(file => file.LastWriteTimeUtc)
            .ToArray();

        if (files.Length <= maxFiles)
        {
            return;
        }

        foreach (FileInfo staleFile in files.Skip(maxFiles))
        {
            try
            {
                staleFile.Delete();
            }
            catch (Exception ex)
            {
                EnqueueWarning("rotate_log_delete", staleFile.FullName, ex);
                // best effort rotation; keep local-only behavior even if a stale file cannot be removed
            }
        }
    }

    private void EnqueueWarning(string operation, string path, Exception ex)
    {
        string warning = $"persistence_{operation}_failed path={path} error={ex.GetType().Name}: {ex.Message}";
        lock (_warningSync)
        {
            _pendingWarnings.Enqueue(warning);
        }
    }

    private void EnsureBaseDirectories()
    {
        Directory.CreateDirectory(BaseDirectory);
        EnsureLogDirectory();
    }

    private void EnsureLogDirectory()
    {
        Directory.CreateDirectory(_logsDirectory);
    }

    private string ResolveDailyLogPath()
    {
        string fileName = $"monitor-{DateTime.UtcNow:yyyyMMdd}.jsonl";
        return Path.Combine(_logsDirectory, fileName);
    }

    private static DiagnosticEntry CreateDiagnosticEntry(string category, object payload)
    {
        return new DiagnosticEntry
        {
            TsMs = (ulong)DateTimeOffset.UtcNow.ToUnixTimeMilliseconds(),
            Category = category,
            Payload = payload,
        };
    }

    private sealed record DiagnosticEntry
    {
        public ulong TsMs { get; init; }

        public string Category { get; init; } = string.Empty;

        public object Payload { get; init; } = new();
    }
}
