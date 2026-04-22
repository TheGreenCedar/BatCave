using BatCave.Runtime.Contracts;
using BatCave.Runtime.Serialization;
using System.Text.Json;

namespace BatCave.Runtime.Persistence;

public interface IRuntimePersistenceStore
{
    RuntimeSettings? LoadSettings();

    Task SaveSettingsAsync(RuntimeSettings settings, CancellationToken ct);

    WarmCache? LoadWarmCache();

    Task SaveWarmCacheAsync(WarmCache cache, CancellationToken ct);

    Task AppendDiagnosticAsync(string category, object payload, CancellationToken ct);

    string BaseDirectory { get; }

    string? TakeWarning() => null;
}

public sealed class LocalJsonRuntimePersistenceStore : IRuntimePersistenceStore
{
    private const string SettingsFileName = "settings.json";
    private const string WarmCacheFileName = "warm-cache.json";
    private const string DiagnosticsFileName = "diagnostics.jsonl";
    private readonly SemaphoreSlim _writeGate = new(1, 1);
    private readonly object _warningSync = new();
    private readonly Queue<string> _pendingWarnings = [];

    public LocalJsonRuntimePersistenceStore(string? baseDirectory = null)
    {
        BaseDirectory = string.IsNullOrWhiteSpace(baseDirectory)
            ? DefaultBaseDirectory()
            : baseDirectory;
        Directory.CreateDirectory(BaseDirectory);
    }

    public string BaseDirectory { get; }

    public static string DefaultBaseDirectory()
    {
        string localAppData = Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData);
        return Path.Combine(localAppData, "BatCaveMonitor");
    }

    public RuntimeSettings? LoadSettings()
    {
        string path = Path.Combine(BaseDirectory, SettingsFileName);
        string? payload = TryReadFile(path);
        if (payload is null)
        {
            return null;
        }

        try
        {
            using JsonDocument document = JsonDocument.Parse(payload);
            JsonElement root = document.RootElement;
            return IsLegacySettings(root)
                ? MigrateLegacySettings(root)
                : JsonSerializer.Deserialize<RuntimeSettings>(payload, JsonDefaults.SnakeCase);
        }
        catch (Exception ex)
        {
            EnqueueWarning("load_json", path, ex);
            return null;
        }
    }

    public Task SaveSettingsAsync(RuntimeSettings settings, CancellationToken ct)
    {
        return WriteJsonAtomicAsync(Path.Combine(BaseDirectory, SettingsFileName), settings, ct);
    }

    public WarmCache? LoadWarmCache()
    {
        string path = Path.Combine(BaseDirectory, WarmCacheFileName);
        string? payload = TryReadFile(path);
        if (payload is null)
        {
            return null;
        }

        try
        {
            using JsonDocument document = JsonDocument.Parse(payload);
            JsonElement root = document.RootElement;
            return HasLegacyWarmCacheRows(root)
                ? MigrateLegacyWarmCache(path, root)
                : JsonSerializer.Deserialize<WarmCache>(payload, JsonDefaults.SnakeCase);
        }
        catch (Exception ex)
        {
            EnqueueWarning("load_json", path, ex);
            return null;
        }
    }

    public Task SaveWarmCacheAsync(WarmCache cache, CancellationToken ct)
    {
        return WriteJsonAtomicAsync(Path.Combine(BaseDirectory, WarmCacheFileName), cache, ct);
    }

    public async Task AppendDiagnosticAsync(string category, object payload, CancellationToken ct)
    {
        Directory.CreateDirectory(BaseDirectory);
        string path = Path.Combine(BaseDirectory, DiagnosticsFileName);
        var envelope = new
        {
            ts_ms = Math.Max(0L, DateTimeOffset.UtcNow.ToUnixTimeMilliseconds()),
            category,
            payload,
        };
        string line = JsonSerializer.Serialize(envelope, JsonDefaults.SnakeCase);
        await File.AppendAllTextAsync(path, line + Environment.NewLine, ct).ConfigureAwait(false);
    }

    public string? TakeWarning()
    {
        lock (_warningSync)
        {
            return _pendingWarnings.Count == 0 ? null : _pendingWarnings.Dequeue();
        }
    }

    private static string? TryReadFile(string path)
    {
        if (!File.Exists(path))
        {
            return null;
        }

        return File.ReadAllText(path);
    }

    private async Task WriteJsonAtomicAsync<T>(string path, T value, CancellationToken ct)
    {
        Directory.CreateDirectory(Path.GetDirectoryName(path)!);
        string tempPath = $"{path}.{Guid.NewGuid():N}.tmp";
        await _writeGate.WaitAsync(ct).ConfigureAwait(false);
        try
        {
            string payload = JsonSerializer.Serialize(value, JsonDefaults.SnakeCase);
            await File.WriteAllTextAsync(tempPath, payload, ct).ConfigureAwait(false);
            File.Move(tempPath, path, overwrite: true);
        }
        finally
        {
            if (File.Exists(tempPath))
            {
                File.Delete(tempPath);
            }

            _writeGate.Release();
        }
    }

    private void EnqueueWarning(string operation, string path, Exception ex)
    {
        EnqueueWarning($"persistence_{operation}_failed path={path} error={ex.GetType().Name}: {ex.Message}");
    }

    private void EnqueueWarning(string warning)
    {
        lock (_warningSync)
        {
            _pendingWarnings.Enqueue(warning);
        }
    }

    private static bool IsLegacySettings(JsonElement root)
    {
        return root.ValueKind == JsonValueKind.Object
               && (root.TryGetProperty("sort_col", out _)
                   || root.TryGetProperty("sort_dir", out _)
                   || root.TryGetProperty("filter_text", out _)
                   || root.TryGetProperty("admin_mode", out _)
                   || root.TryGetProperty("admin_preference_initialized", out _)
                   || root.TryGetProperty("metric_trend_window_seconds", out _));
    }

    private static RuntimeSettings MigrateLegacySettings(JsonElement root)
    {
        bool adminPreferenceInitialized = TryGetBool(root, "admin_preference_initialized") ?? true;
        bool adminMode = adminPreferenceInitialized
            ? TryGetBool(root, "admin_mode") ?? true
            : true;

        return new RuntimeSettings
        {
            Query = new RuntimeQuery
            {
                FilterText = TryGetString(root, "filter_text") ?? string.Empty,
                SortColumn = TryGetSortColumn(root, "sort_col") ?? SortColumn.CpuPct,
                SortDirection = TryGetSortDirection(root, "sort_dir") ?? SortDirection.Desc,
                Limit = TryGetInt(root, "limit") ?? 5000,
            },
            AdminModeRequested = adminMode,
            AdminModeEnabled = false,
            MetricWindowSeconds = TryGetInt(root, "metric_trend_window_seconds") ?? 60,
        };
    }

    private bool HasLegacyWarmCacheRows(JsonElement root)
    {
        if (root.ValueKind != JsonValueKind.Object
            || !root.TryGetProperty("rows", out JsonElement rows)
            || rows.ValueKind != JsonValueKind.Array)
        {
            return false;
        }

        foreach (JsonElement row in rows.EnumerateArray())
        {
            if (row.ValueKind == JsonValueKind.Object
                && (row.TryGetProperty("rss_bytes", out _)
                    || row.TryGetProperty("io_read_bps", out _)
                    || row.TryGetProperty("io_write_bps", out _)
                    || row.TryGetProperty("io_other_bps", out _)
                    || row.TryGetProperty("net_bps", out _)))
            {
                return true;
            }
        }

        return false;
    }

    private WarmCache MigrateLegacyWarmCache(string path, JsonElement root)
    {
        List<ProcessSample> rows = [];
        if (!root.TryGetProperty("rows", out JsonElement rowArray) || rowArray.ValueKind != JsonValueKind.Array)
        {
            EnqueueWarning($"persistence_legacy_warm_cache_rows_skipped path={path} error=RowsMissingOrInvalid");
            return new WarmCache { Seq = TryGetUlong(root, "seq") ?? 0UL, Rows = rows };
        }

        int index = 0;
        foreach (JsonElement row in rowArray.EnumerateArray())
        {
            if (TryMigrateLegacyWarmCacheRow(row, out ProcessSample? sample))
            {
                rows.Add(sample!);
            }
            else
            {
                EnqueueWarning($"persistence_legacy_warm_cache_row_skipped path={path} index={index} error=UnreadableRow");
            }

            index++;
        }

        return new WarmCache
        {
            Seq = TryGetUlong(root, "seq") ?? 0UL,
            Rows = rows,
        };
    }

    private static bool TryMigrateLegacyWarmCacheRow(JsonElement row, out ProcessSample? sample)
    {
        sample = null;
        if (row.ValueKind != JsonValueKind.Object)
        {
            return false;
        }

        uint? pid = TryGetUint(row, "pid");
        string? name = TryGetString(row, "name");
        if (!pid.HasValue || string.IsNullOrWhiteSpace(name))
        {
            return false;
        }

        ulong read = TryGetUlong(row, "io_read_bps") ?? 0UL;
        ulong write = TryGetUlong(row, "io_write_bps") ?? 0UL;
        sample = new ProcessSample
        {
            Seq = TryGetUlong(row, "seq") ?? 0UL,
            TsMs = TryGetUlong(row, "ts_ms") ?? 0UL,
            Pid = pid.Value,
            ParentPid = TryGetUint(row, "parent_pid") ?? 0U,
            StartTimeMs = TryGetUlong(row, "start_time_ms") ?? 0UL,
            Name = name,
            CpuPct = TryGetDouble(row, "cpu_pct") ?? 0d,
            MemoryBytes = TryGetUlong(row, "memory_bytes") ?? TryGetUlong(row, "rss_bytes") ?? 0UL,
            PrivateBytes = TryGetUlong(row, "private_bytes") ?? 0UL,
            DiskBps = TryGetUlong(row, "disk_bps") ?? SaturatingAdd(read, write),
            OtherIoBps = TryGetUlong(row, "other_io_bps")
                         ?? TryGetUlong(row, "io_other_bps")
                         ?? TryGetUlong(row, "net_bps")
                         ?? 0UL,
            Threads = TryGetUint(row, "threads") ?? 0U,
            Handles = TryGetUint(row, "handles") ?? 0U,
            AccessState = TryGetAccessState(row, "access_state") ?? AccessState.Full,
        };
        return true;
    }

    private static string? TryGetString(JsonElement root, string propertyName)
    {
        return root.TryGetProperty(propertyName, out JsonElement value) && value.ValueKind == JsonValueKind.String
            ? value.GetString()
            : null;
    }

    private static bool? TryGetBool(JsonElement root, string propertyName)
    {
        if (!root.TryGetProperty(propertyName, out JsonElement value))
        {
            return null;
        }

        return value.ValueKind switch
        {
            JsonValueKind.True => true,
            JsonValueKind.False => false,
            _ => null,
        };
    }

    private static int? TryGetInt(JsonElement root, string propertyName)
    {
        return root.TryGetProperty(propertyName, out JsonElement value) && value.TryGetInt32(out int parsed)
            ? parsed
            : null;
    }

    private static uint? TryGetUint(JsonElement root, string propertyName)
    {
        return root.TryGetProperty(propertyName, out JsonElement value) && value.TryGetUInt32(out uint parsed)
            ? parsed
            : null;
    }

    private static ulong? TryGetUlong(JsonElement root, string propertyName)
    {
        return root.TryGetProperty(propertyName, out JsonElement value) && value.TryGetUInt64(out ulong parsed)
            ? parsed
            : null;
    }

    private static double? TryGetDouble(JsonElement root, string propertyName)
    {
        return root.TryGetProperty(propertyName, out JsonElement value) && value.TryGetDouble(out double parsed)
            ? parsed
            : null;
    }

    private static SortColumn? TryGetSortColumn(JsonElement root, string propertyName)
    {
        string? value = TryGetString(root, propertyName);
        if (string.IsNullOrWhiteSpace(value))
        {
            return root.TryGetProperty(propertyName, out JsonElement numeric)
                   && numeric.TryGetInt32(out int parsed)
                   && Enum.IsDefined(typeof(SortColumn), parsed)
                ? (SortColumn)parsed
                : null;
        }

        return NormalizeEnumToken(value) switch
        {
            "pid" => SortColumn.Pid,
            "attention" => SortColumn.Attention,
            "name" => SortColumn.Name,
            "cpupct" => SortColumn.CpuPct,
            "rssbytes" or "memorybytes" => SortColumn.MemoryBytes,
            "ioreadbps" or "iowritebps" or "diskbps" => SortColumn.DiskBps,
            "otheriobps" or "iootherbps" or "netbps" => SortColumn.OtherIoBps,
            "threads" => SortColumn.Threads,
            "handles" => SortColumn.Handles,
            "starttimems" => SortColumn.StartTimeMs,
            _ => null,
        };
    }

    private static SortDirection? TryGetSortDirection(JsonElement root, string propertyName)
    {
        string? value = TryGetString(root, propertyName);
        if (!string.IsNullOrWhiteSpace(value))
        {
            return NormalizeEnumToken(value) switch
            {
                "asc" => SortDirection.Asc,
                "desc" => SortDirection.Desc,
                _ => null,
            };
        }

        return root.TryGetProperty(propertyName, out JsonElement numeric)
               && numeric.TryGetInt32(out int parsed)
               && Enum.IsDefined(typeof(SortDirection), parsed)
            ? (SortDirection)parsed
            : null;
    }

    private static AccessState? TryGetAccessState(JsonElement root, string propertyName)
    {
        string? value = TryGetString(root, propertyName);
        if (!string.IsNullOrWhiteSpace(value))
        {
            return NormalizeEnumToken(value) switch
            {
                "full" => AccessState.Full,
                "partial" => AccessState.Partial,
                "denied" => AccessState.Denied,
                _ => null,
            };
        }

        return root.TryGetProperty(propertyName, out JsonElement numeric)
               && numeric.TryGetInt32(out int parsed)
               && Enum.IsDefined(typeof(AccessState), parsed)
            ? (AccessState)parsed
            : null;
    }

    private static string NormalizeEnumToken(string value)
    {
        return value.Replace("_", string.Empty, StringComparison.Ordinal)
            .Replace("-", string.Empty, StringComparison.Ordinal)
            .Trim()
            .ToLowerInvariant();
    }

    private static ulong SaturatingAdd(ulong left, ulong right)
    {
        return ulong.MaxValue - left < right ? ulong.MaxValue : left + right;
    }
}
