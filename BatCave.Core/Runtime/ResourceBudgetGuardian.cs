using BatCave.Core.Domain;

namespace BatCave.Core.Runtime;

public sealed record RuntimePolicy
{
    public bool EmitTelemetryDelta { get; init; }

    public ulong WarmCacheInterval { get; init; }

    public int? CompactMaxRows { get; init; }
}

public sealed class ResourceBudgetGuardian
{
    private const ulong Mb = 1024 * 1024;
    private const double CpuBudgetPct = 1.0;
    private const ulong RssBudgetBytes = 150 * Mb;

    private uint _highStreak;
    private uint _lowStreak;
    private ulong _emitStride = 1;

    public RuntimePolicy Evaluate(ulong seq, RuntimeHealth health, int rowCount)
    {
        bool overCpu = health.AppCpuPct >= CpuBudgetPct;
        bool overRss = health.AppRssBytes >= RssBudgetBytes;
        bool overBudget = overCpu || overRss;

        if (overBudget)
        {
            _highStreak++;
            _lowStreak = 0;
        }
        else
        {
            _lowStreak++;
            _highStreak = 0;
        }

        _emitStride = ResolveEmitStride(_emitStride, _highStreak, _lowStreak);

        ulong warmCacheInterval = ResolveWarmCacheInterval(_emitStride);
        int? compactMaxRows = ResolveCompactMaxRows(overRss, rowCount, _emitStride);

        return new RuntimePolicy
        {
            EmitTelemetryDelta = ShouldEmitTelemetryDelta(seq, _emitStride),
            WarmCacheInterval = warmCacheInterval,
            CompactMaxRows = compactMaxRows,
        };
    }

    public bool IsDegraded()
    {
        return _emitStride > 1;
    }

    private static ulong ResolveEmitStride(ulong currentStride, uint highStreak, uint lowStreak)
    {
        if (highStreak >= 8)
        {
            return 4;
        }

        if (highStreak >= 3)
        {
            return 2;
        }

        if (lowStreak >= 10)
        {
            return 1;
        }

        return currentStride;
    }

    private static ulong ResolveWarmCacheInterval(ulong emitStride)
    {
        return emitStride switch
        {
            1 => 5,
            2 => 10,
            _ => 20,
        };
    }

    private static int? ResolveCompactMaxRows(bool overRss, int rowCount, ulong emitStride)
    {
        if (overRss && rowCount > 3500)
        {
            return 3500;
        }

        if (emitStride > 1 && rowCount > 5000)
        {
            return 5000;
        }

        return null;
    }

    private static bool ShouldEmitTelemetryDelta(ulong seq, ulong emitStride)
    {
        return emitStride <= 1 || seq % emitStride == 0;
    }
}
