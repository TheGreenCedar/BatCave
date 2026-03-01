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

        if (_highStreak >= 8)
        {
            _emitStride = 4;
        }
        else if (_highStreak >= 3)
        {
            _emitStride = 2;
        }
        else if (_lowStreak >= 10)
        {
            _emitStride = 1;
        }

        ulong warmCacheInterval = _emitStride switch
        {
            1 => 5,
            2 => 10,
            _ => 20,
        };

        int? compactMaxRows = null;
        if (overRss && rowCount > 3500)
        {
            compactMaxRows = 3500;
        }
        else if (_emitStride > 1 && rowCount > 5000)
        {
            compactMaxRows = 5000;
        }

        return new RuntimePolicy
        {
            // UI must remain tick-synchronous with runtime seq progression.
            EmitTelemetryDelta = true,
            WarmCacheInterval = warmCacheInterval,
            CompactMaxRows = compactMaxRows,
        };
    }

    public bool IsDegraded()
    {
        return _emitStride > 1;
    }
}
