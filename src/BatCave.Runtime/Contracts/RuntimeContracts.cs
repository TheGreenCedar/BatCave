namespace BatCave.Runtime.Contracts;

public static class RuntimeEventKinds
{
    public const string Snapshot = "snapshot";
    public const string Delta = "delta";
    public const string RuntimeHealth = "runtime_health";
    public const string CollectorWarning = "collector_warning";
}

public enum AccessState
{
    Full,
    Partial,
    Denied,
}

public enum SortColumn
{
    Attention,
    Name,
    Pid,
    CpuPct,
    MemoryBytes,
    DiskBps,
    OtherIoBps,
    Threads,
    Handles,
    StartTimeMs,
}

public enum SortDirection
{
    Asc,
    Desc,
}

public readonly record struct ProcessIdentity(uint Pid, ulong StartTimeMs);

public sealed record ProcessSample
{
    public ulong Seq { get; init; }
    public ulong TsMs { get; init; }
    public uint Pid { get; init; }
    public uint ParentPid { get; init; }
    public ulong StartTimeMs { get; init; }
    public string Name { get; init; } = string.Empty;
    public double CpuPct { get; init; }
    public ulong MemoryBytes { get; init; }
    public ulong PrivateBytes { get; init; }
    public ulong DiskBps { get; init; }
    public ulong OtherIoBps { get; init; }
    public uint Threads { get; init; }
    public uint Handles { get; init; }
    public AccessState AccessState { get; init; } = AccessState.Full;

    public ProcessIdentity Identity() => new(Pid, StartTimeMs);
}

public sealed record SystemMetricsSnapshot
{
    public ulong TsMs { get; init; }
    public double? CpuPct { get; init; }
    public double? KernelCpuPct { get; init; }
    public ulong? MemoryUsedBytes { get; init; }
    public ulong? MemoryTotalBytes { get; init; }
    public ulong? MemoryAvailableBytes { get; init; }
    public ulong? DiskReadBps { get; init; }
    public ulong? DiskWriteBps { get; init; }
    public ulong? NetworkBytesBps { get; init; }
    public ulong? OtherIoBps { get; init; }
    public IReadOnlyList<double> LogicalCpuPct { get; init; } = [];
    public int LogicalProcessorCount { get; init; } = Environment.ProcessorCount;
    public bool IsReady { get; init; }
}

public sealed record RuntimeWarning
{
    public ulong Seq { get; init; }
    public ulong TsMs { get; init; }
    public string Category { get; init; } = string.Empty;
    public string Message { get; init; } = string.Empty;
}

public sealed record RuntimeHealth
{
    public bool RuntimeLoopEnabled { get; init; }
    public bool RuntimeLoopRunning { get; init; }
    public bool StartupBlocked { get; init; }
    public string StatusSummary { get; init; } = "Runtime starting.";
    public ulong UpdatedAtMs { get; init; }
    public ulong Seq { get; init; }
    public double TickP95Ms { get; init; }
    public double SortP95Ms { get; init; }
    public double JitterP95Ms { get; init; }
    public ulong DroppedTicks { get; init; }
    public double AppCpuPct { get; init; }
    public ulong AppRssBytes { get; init; }
    public bool DegradeMode { get; init; }
    public string? LastWarning { get; init; }
}

public sealed record RuntimeQuery
{
    public string FilterText { get; init; } = string.Empty;
    public SortColumn SortColumn { get; init; } = SortColumn.CpuPct;
    public SortDirection SortDirection { get; init; } = SortDirection.Desc;
    public int Limit { get; init; } = 5000;
}

public sealed record RuntimeSettings
{
    public RuntimeQuery Query { get; init; } = new();
    public bool AdminModeRequested { get; init; }
    public bool AdminModeEnabled { get; init; }
    public int MetricWindowSeconds { get; init; } = 60;
    public bool Paused { get; init; }
}

public sealed record RuntimeSnapshot
{
    public string EventKind { get; init; } = RuntimeEventKinds.Snapshot;
    public ulong Seq { get; init; }
    public ulong TsMs { get; init; }
    public RuntimeSettings Settings { get; init; } = new();
    public RuntimeHealth Health { get; init; } = new();
    public SystemMetricsSnapshot System { get; init; } = new();
    public IReadOnlyList<ProcessSample> Rows { get; init; } = [];
    public int TotalProcessCount { get; init; }
    public IReadOnlyList<RuntimeWarning> Warnings { get; init; } = [];
}

public sealed record RuntimeDelta
{
    public string EventKind { get; init; } = RuntimeEventKinds.Delta;
    public ulong Seq { get; init; }
    public ulong TsMs { get; init; }
    public IReadOnlyList<ProcessSample> Upserts { get; init; } = [];
    public IReadOnlyList<ProcessIdentity> Exits { get; init; } = [];
    public RuntimeHealth? Health { get; init; }
    public SystemMetricsSnapshot? System { get; init; }
    public RuntimeSnapshot Snapshot { get; init; } = new();
}

public abstract record RuntimeCommand;

public sealed record SetProcessQueryCommand(RuntimeQuery Query) : RuntimeCommand;

public sealed record SetAdminModeCommand(bool Enabled) : RuntimeCommand;

public sealed record SetMetricWindowCommand(int Seconds) : RuntimeCommand;

public sealed record RefreshNowCommand : RuntimeCommand;

public sealed record PauseRuntimeCommand : RuntimeCommand;

public sealed record ResumeRuntimeCommand : RuntimeCommand;

public sealed record WarmCache
{
    public ulong Seq { get; init; }
    public IReadOnlyList<ProcessSample> Rows { get; init; } = [];
}

public sealed record LaunchContext
{
    public string Os { get; init; } = string.Empty;
    public uint WindowsBuild { get; init; }
}

public sealed record LaunchBlockReason
{
    public string Code { get; init; } = string.Empty;
    public string Message { get; init; } = string.Empty;

    public static LaunchBlockReason UnsupportedPlatform(string platform) => new()
    {
        Code = "unsupported_platform",
        Message = $"BatCave requires Windows. Current platform: {platform}.",
    };

    public static LaunchBlockReason RequiresWindows11(uint build) => new()
    {
        Code = "requires_windows_11",
        Message = $"BatCave requires Windows 11 build 22000 or newer. Current build: {build}.",
    };
}

public sealed record StartupGateStatus
{
    public bool Passed { get; init; }
    public LaunchContext? Context { get; init; }
    public LaunchBlockReason? Reason { get; init; }

    public static StartupGateStatus PassedContext(LaunchContext context) => new()
    {
        Passed = true,
        Context = context,
    };

    public static StartupGateStatus Blocked(LaunchBlockReason reason) => new()
    {
        Passed = false,
        Reason = reason,
    };
}

public interface IRuntimeStore
{
    RuntimeSnapshot GetSnapshot();

    IAsyncEnumerable<RuntimeDelta> SubscribeAsync(CancellationToken ct);

    Task ExecuteAsync(RuntimeCommand command, CancellationToken ct);
}
