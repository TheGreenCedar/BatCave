using BatCave.Core.Domain;

namespace BatCave.Core.Abstractions;

public sealed record CollectorActivationResult(
    IProcessCollector Collector,
    bool EffectiveAdminMode,
    string? Warning);

public interface ILaunchPolicyGate
{
    StartupGateStatus Enforce();
}

public interface IProcessCollectorFactory
{
    ValueTask<CollectorActivationResult> CreateAsync(bool adminMode, CancellationToken ct);
}

public interface IProcessCollector
{
    IReadOnlyList<ProcessSample> CollectTick(ulong seq);

    string? TakeWarning();
}

public interface ITelemetryPipeline
{
    void SeedFromWarmCache(IReadOnlyList<ProcessSample> rows);

    ProcessDeltaBatch ApplyRaw(ulong seq, IReadOnlyList<ProcessSample> raw);
}

public interface IStateStore
{
    void ApplyDelta(ProcessDeltaBatch delta);

    IReadOnlyList<ProcessSample> AllRows();

    WarmCache ExportWarmCache(ulong seq);

    void ImportWarmCache(WarmCache cache);

    int RowCount();

    void CompactTo(int maxRows);
}

public interface ISortIndexEngine
{
    void OnDelta(ProcessDeltaBatch delta);

    QueryResponse Query(QueryRequest request, IReadOnlyList<ProcessSample> rows, ulong seq);
}

public interface IPersistenceStore
{
    UserSettings? LoadSettings();

    Task SaveSettingsAsync(UserSettings settings, CancellationToken ct);

    WarmCache? LoadWarmCache();

    Task SaveWarmCacheAsync(WarmCache cache, CancellationToken ct);

    Task AppendDiagnosticAsync(string category, object payload, CancellationToken ct);

    string? TakeWarning();

    string BaseDirectory { get; }
}

public interface IMonitoringRuntime
{
    Task<CollectorActivationResult> InitializeAsync(CancellationToken ct);

    QueryResponse GetSnapshot();

    RuntimeHealth GetRuntimeHealth();

    void SetSort(SortColumn sortCol, SortDirection sortDir);

    void SetFilter(string filterText);

    bool IsAdminMode();

    int CurrentMetricTrendWindowSeconds { get; }

    void SetMetricTrendWindowSeconds(int seconds);

    Task<CollectorActivationResult> RestartAsync(bool adminMode, CancellationToken ct);

    void RecordDroppedTicks(ulong dropped);
}

public interface IProcessMetadataProvider
{
    Task<ProcessMetadata?> GetAsync(uint pid, ulong startTimeMs, CancellationToken ct);
}

public interface ISystemGlobalMetricsSampler
{
    SystemGlobalMetricsSample Sample();
}
