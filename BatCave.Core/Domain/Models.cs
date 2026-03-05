using System.Text.Json.Serialization;

namespace BatCave.Core.Domain;

public static class EventContracts
{
    public const string TelemetryDelta = "telemetry_delta";
    public const string RuntimeHealth = "runtime_health";
    public const string CollectorWarning = "collector_warning";
}

public enum AccessState
{
    Full,
    Partial,
    Denied,
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

    public ulong RssBytes { get; init; }

    public ulong PrivateBytes { get; init; }

    public ulong IoReadBps { get; init; }

    public ulong IoWriteBps { get; init; }

    [JsonPropertyName("net_bps")]
    public ulong OtherIoBps { get; init; }

    [JsonIgnore]
    [Obsolete("Use OtherIoBps instead.")]
    public ulong NetBps
    {
        get => OtherIoBps;
        init => OtherIoBps = value;
    }

    public uint Threads { get; init; }

    public uint Handles { get; init; }

    public AccessState AccessState { get; init; }

    public ProcessIdentity Identity()
    {
        return new ProcessIdentity(Pid, StartTimeMs);
    }
}

public sealed record ProcessDeltaBatch
{
    public ulong Seq { get; init; }

    public IReadOnlyList<ProcessSample> Upserts { get; init; } = [];

    public IReadOnlyList<ProcessIdentity> Exits { get; init; } = [];
}

public enum SortColumn
{
    Pid,
    Name,
    CpuPct,
    RssBytes,
    IoReadBps,
    IoWriteBps,
    OtherIoBps,
    [Obsolete("Use OtherIoBps instead.")]
    NetBps = OtherIoBps,
    Threads,
    Handles,
    StartTimeMs,
    DiskBps,
}

public enum SortDirection
{
    Asc,
    Desc,
}

public sealed record QueryRequest
{
    public int Offset { get; init; }

    public int Limit { get; init; } = 5000;

    public SortColumn SortCol { get; init; } = SortColumn.CpuPct;

    public SortDirection SortDir { get; init; } = SortDirection.Desc;

    public string FilterText { get; init; } = string.Empty;
}

public sealed record QueryResponse
{
    public ulong Seq { get; init; }

    public int Total { get; init; }

    public IReadOnlyList<ProcessSample> Rows { get; init; } = [];
}

public sealed record SystemGlobalMetricsSample
{
    public ulong TsMs { get; init; }

    public double? CpuPct { get; init; }

    public ulong? MemoryUsedBytes { get; init; }

    public ulong? DiskReadBps { get; init; }

    public ulong? DiskWriteBps { get; init; }

    public ulong? OtherIoBps { get; init; }

    public SystemGlobalCpuSnapshot? CpuSnapshot { get; init; }

    public SystemGlobalMemorySnapshot? MemorySnapshot { get; init; }

    public IReadOnlyList<SystemGlobalDiskSnapshot> DiskSnapshots { get; init; } = [];

    public IReadOnlyList<SystemGlobalNetworkSnapshot> NetworkSnapshots { get; init; } = [];

    public bool CpuRateWarmed { get; init; }

    public bool RateCountersWarmed { get; init; }

    public bool ExtendedProbeCycleCompleted { get; init; }

    public bool IsReady { get; init; }
}

public sealed record SystemGlobalCpuSnapshot
{
    public string? ProcessorName { get; init; }

    public double? KernelPct { get; init; }

    public double? SpeedMHz { get; init; }

    public double? BaseSpeedMHz { get; init; }

    public int? Sockets { get; init; }

    public int? Cores { get; init; }

    public int? LogicalProcessors { get; init; }

    public bool? VirtualizationEnabled { get; init; }

    public ulong? L1CacheBytes { get; init; }

    public ulong? L2CacheBytes { get; init; }

    public ulong? L3CacheBytes { get; init; }

    public uint? ProcessCount { get; init; }

    public uint? ThreadCount { get; init; }

    public uint? HandleCount { get; init; }

    public ulong? UptimeSeconds { get; init; }

    public IReadOnlyList<double> LogicalProcessorUtilizationPct { get; init; } = [];

    public IReadOnlyList<double> LogicalProcessorKernelPct { get; init; } = [];
}

public sealed record SystemGlobalMemorySnapshot
{
    public ulong? TotalBytes { get; init; }

    public ulong? UsedBytes { get; init; }

    public ulong? AvailableBytes { get; init; }

    public ulong? CommittedUsedBytes { get; init; }

    public ulong? CommittedLimitBytes { get; init; }

    public ulong? CachedBytes { get; init; }

    public ulong? PagedPoolBytes { get; init; }

    public ulong? NonPagedPoolBytes { get; init; }

    public uint? SpeedMTps { get; init; }

    public int? SlotsUsed { get; init; }

    public int? SlotsTotal { get; init; }

    public string? FormFactor { get; init; }

    public ulong? HardwareReservedBytes { get; init; }
}

public sealed record SystemGlobalDiskSnapshot
{
    public string DiskId { get; init; } = string.Empty;

    public string DisplayName { get; init; } = string.Empty;

    public string? Model { get; init; }

    public string? TypeLabel { get; init; }

    public double? ActiveTimePct { get; init; }

    public double? AvgResponseMs { get; init; }

    public ulong? ReadBps { get; init; }

    public ulong? WriteBps { get; init; }

    public ulong? CapacityBytes { get; init; }

    public ulong? FormattedBytes { get; init; }

    public bool? IsSystemDisk { get; init; }

    public bool? HasPageFile { get; init; }
}

public sealed record SystemGlobalNetworkSnapshot
{
    public string AdapterId { get; init; } = string.Empty;

    public string DisplayName { get; init; } = string.Empty;

    public string? AdapterName { get; init; }

    public string? ConnectionType { get; init; }

    public string? IPv4Address { get; init; }

    public string? IPv6Address { get; init; }

    public ulong? SendBps { get; init; }

    public ulong? ReceiveBps { get; init; }

    public ulong? LinkSpeedBps { get; init; }
}

public sealed record RuntimeHealth
{
    public ulong Seq { get; init; }

    public ulong LastTickMs { get; init; }

    public double JitterP95Ms { get; init; }

    public ulong DroppedTicks { get; init; }

    public double AppCpuPct { get; init; }

    public ulong AppRssBytes { get; init; }

    public bool DegradeMode { get; init; }

    public ulong CollectorWarnings { get; init; }
}

public enum LaunchBlockReasonKind
{
    UnsupportedPlatform,
    RequiresWindows11,
}

public sealed record LaunchBlockReason
{
    public LaunchBlockReasonKind Kind { get; init; }

    [JsonIgnore(Condition = JsonIgnoreCondition.WhenWritingNull)]
    public string? Os { get; init; }

    [JsonIgnore(Condition = JsonIgnoreCondition.WhenWritingNull)]
    public uint? DetectedBuild { get; init; }

    public static LaunchBlockReason UnsupportedPlatform(string os)
    {
        return new LaunchBlockReason
        {
            Kind = LaunchBlockReasonKind.UnsupportedPlatform,
            Os = os,
        };
    }

    public static LaunchBlockReason RequiresWindows11(uint detectedBuild)
    {
        return new LaunchBlockReason
        {
            Kind = LaunchBlockReasonKind.RequiresWindows11,
            DetectedBuild = detectedBuild,
        };
    }
}

public sealed record LaunchContext
{
    public string Os { get; init; } = string.Empty;

    public uint WindowsBuild { get; init; }
}

public sealed record StartupGateStatus
{
    public bool Passed { get; init; }

    [JsonIgnore(Condition = JsonIgnoreCondition.WhenWritingNull)]
    public LaunchContext? Context { get; init; }

    [JsonIgnore(Condition = JsonIgnoreCondition.WhenWritingNull)]
    public LaunchBlockReason? Reason { get; init; }

    public static StartupGateStatus PassedContext(LaunchContext context)
    {
        return new StartupGateStatus
        {
            Passed = true,
            Context = context,
        };
    }

    public static StartupGateStatus Blocked(LaunchBlockReason reason)
    {
        return new StartupGateStatus
        {
            Passed = false,
            Reason = reason,
        };
    }
}

public sealed record UserSettings
{
    public SortColumn SortCol { get; init; } = SortColumn.CpuPct;

    public SortDirection SortDir { get; init; } = SortDirection.Desc;

    public string FilterText { get; init; } = string.Empty;

    public bool AdminMode { get; init; }

    public bool AdminPreferenceInitialized { get; init; }

    public int MetricTrendWindowSeconds { get; init; } = 60;
}

public sealed record WarmCache
{
    public ulong Seq { get; init; }

    public IReadOnlyList<ProcessSample> Rows { get; init; } = [];
}

public sealed record CollectorWarning
{
    public string Message { get; init; } = string.Empty;

    public ulong Seq { get; init; }
}

public sealed record ProcessMetadata
{
    public uint Pid { get; init; }

    public uint ParentPid { get; init; }

    public string? CommandLine { get; init; }

    public string? ExecutablePath { get; init; }
}
