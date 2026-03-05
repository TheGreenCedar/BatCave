using BatCave.Core.Domain;
using System;

namespace BatCave.Services;

public sealed record RuntimeHealthSnapshot
{
    public bool RuntimeLoopEnabled { get; init; } = true;

    public bool RuntimeLoopRunning { get; init; }

    public bool StartupBlocked { get; init; }

    public string StatusSummary { get; init; } = "Runtime health unavailable.";

    public RuntimeHealth Health { get; init; } = new();

    public string? LastWarning { get; init; }

    public ulong LastWarningSeq { get; init; }

    public ulong UpdatedAtMs { get; init; }

    public bool IsReady => RuntimeLoopRunning && Health.Seq > 0 && !StartupBlocked;
}

public interface IRuntimeHealthService
{
    RuntimeHealthSnapshot Snapshot();

    void ReportRuntimeLoopState(bool enabled, bool running, bool startupBlocked, string statusSummary);

    void ReportHealth(RuntimeHealth health);

    void ReportWarning(CollectorWarning warning);
}

public sealed class RuntimeHealthService : IRuntimeHealthService
{
    private readonly object _sync = new();
    private RuntimeHealthSnapshot _snapshot = new();

    public RuntimeHealthSnapshot Snapshot()
    {
        lock (_sync)
        {
            return _snapshot;
        }
    }

    public void ReportRuntimeLoopState(bool enabled, bool running, bool startupBlocked, string statusSummary)
    {
        lock (_sync)
        {
            _snapshot = _snapshot with
            {
                RuntimeLoopEnabled = enabled,
                RuntimeLoopRunning = running,
                StartupBlocked = startupBlocked,
                StatusSummary = statusSummary,
                UpdatedAtMs = UnixNowMs(),
            };
        }
    }

    public void ReportHealth(RuntimeHealth health)
    {
        lock (_sync)
        {
            _snapshot = _snapshot with
            {
                Health = health,
                UpdatedAtMs = UnixNowMs(),
            };
        }
    }

    public void ReportWarning(CollectorWarning warning)
    {
        lock (_sync)
        {
            string warningMessage = warning.Message?.Trim() ?? string.Empty;
            if (!string.IsNullOrWhiteSpace(warningMessage))
            {
                _snapshot = _snapshot with
                {
                    LastWarning = warningMessage,
                    LastWarningSeq = warning.Seq,
                    UpdatedAtMs = UnixNowMs(),
                };
            }
        }
    }

    private static ulong UnixNowMs()
    {
        long now = DateTimeOffset.UtcNow.ToUnixTimeMilliseconds();
        return now <= 0 ? 0UL : (ulong)now;
    }
}
