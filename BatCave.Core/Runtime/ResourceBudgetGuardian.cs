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
    private const uint DegradeToStrideTwoHighStreakThreshold = 3;
    private const uint DegradeToStrideFourHighStreakThreshold = 8;
    private const uint RecoverToStrideOneLowStreakThreshold = 10;
    private const ulong NormalWarmCacheInterval = 5;
    private const ulong DegradedWarmCacheInterval = 10;
    private const ulong SevereWarmCacheInterval = 20;
    private const int OverRssCompactMaxRows = 3500;
    private const int DegradedCompactMaxRows = 5000;

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
        if (highStreak >= DegradeToStrideFourHighStreakThreshold)
        {
            return 4;
        }

        if (highStreak >= DegradeToStrideTwoHighStreakThreshold)
        {
            return 2;
        }

        if (lowStreak >= RecoverToStrideOneLowStreakThreshold)
        {
            return 1;
        }

        return currentStride;
    }

    private static ulong ResolveWarmCacheInterval(ulong emitStride)
    {
        return emitStride switch
        {
            1 => NormalWarmCacheInterval,
            2 => DegradedWarmCacheInterval,
            _ => SevereWarmCacheInterval,
        };
    }

    private static int? ResolveCompactMaxRows(bool overRss, int rowCount, ulong emitStride)
    {
        if (overRss && rowCount > OverRssCompactMaxRows)
        {
            return OverRssCompactMaxRows;
        }

        if (emitStride > 1 && rowCount > DegradedCompactMaxRows)
        {
            return DegradedCompactMaxRows;
        }

        return null;
    }

    private static bool ShouldEmitTelemetryDelta(ulong seq, ulong emitStride)
    {
        return emitStride <= 1 || seq % emitStride == 0;
    }
}
