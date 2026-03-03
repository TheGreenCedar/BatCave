using BatCave.Core.Abstractions;
using BatCave.Core.Domain;

namespace BatCave.Core.Tests.Runtime.TestSupport;

internal sealed class TestPersistenceStore : IPersistenceStore
{
    private readonly Queue<string> _warnings = [];
    private readonly List<UserSettings> _savedSettings = [];
    private UserSettings _settings = new();
    private WarmCache? _warmCache;

    public bool FailSaveSettings { get; set; }

    public string BaseDirectory => Path.GetTempPath();

    public UserSettings? LoadSettings()
    {
        return _settings;
    }

    public Task SaveSettingsAsync(UserSettings settings, CancellationToken ct)
    {
        if (FailSaveSettings)
        {
            EnqueueWarning("persistence_save_settings_failed path=settings.json error=IOException: write denied");
            throw new IOException("write denied");
        }

        _settings = settings;
        _savedSettings.Add(settings);
        return Task.CompletedTask;
    }

    public WarmCache? LoadWarmCache()
    {
        return _warmCache;
    }

    public Task SaveWarmCacheAsync(WarmCache cache, CancellationToken ct)
    {
        _warmCache = cache;
        return Task.CompletedTask;
    }

    public Task AppendDiagnosticAsync(string category, object payload, CancellationToken ct)
    {
        return Task.CompletedTask;
    }

    public string? TakeWarning()
    {
        return _warnings.Count > 0 ? _warnings.Dequeue() : null;
    }

    public void EnqueueWarning(string warning)
    {
        _warnings.Enqueue(warning);
    }

    public IReadOnlyList<UserSettings> GetSavedSettingsSnapshot()
    {
        return _savedSettings.ToArray();
    }
}
