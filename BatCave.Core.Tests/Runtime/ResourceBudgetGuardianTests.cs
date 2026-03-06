using BatCave.Core.Domain;
using BatCave.Core.Runtime;

namespace BatCave.Core.Tests.Runtime;

public class ResourceBudgetGuardianTests
{
    private const ulong Mb = 1024UL * 1024UL;

    [Fact]
    public void DegradeMode_TransitionsByOverBudgetAndRecoveryStreaks()
    {
        ResourceBudgetGuardian guardian = new();
        RuntimeHealth overBudget = CreateHealth(cpuPct: 6.5, rssBytes: 360UL * Mb);
        RuntimeHealth healthy = CreateHealth(cpuPct: 0.1, rssBytes: 20UL * Mb);

        for (ulong seq = 1; seq <= 3; seq++)
        {
            _ = guardian.Evaluate(seq, overBudget, rowCount: 1000);
        }

        Assert.True(guardian.IsDegraded());

        for (ulong seq = 4; seq <= 13; seq++)
        {
            _ = guardian.Evaluate(seq, healthy, rowCount: 1000);
        }

        Assert.False(guardian.IsDegraded());
    }

    [Fact]
    public void WarmCacheInterval_TracksEmitStridePolicy()
    {
        ResourceBudgetGuardian guardian = new();
        RuntimeHealth overBudget = CreateHealth(cpuPct: 6.5, rssBytes: 360UL * Mb);

        RuntimePolicy policyAfterThree = default!;
        for (ulong seq = 1; seq <= 3; seq++)
        {
            policyAfterThree = guardian.Evaluate(seq, overBudget, rowCount: 1000);
        }

        Assert.Equal(10UL, policyAfterThree.WarmCacheInterval);

        RuntimePolicy policyAfterEight = default!;
        for (ulong seq = 4; seq <= 8; seq++)
        {
            policyAfterEight = guardian.Evaluate(seq, overBudget, rowCount: 1000);
        }

        Assert.Equal(20UL, policyAfterEight.WarmCacheInterval);
    }

    [Fact]
    public void EmitTelemetryDelta_FollowsStrideWhenDegraded()
    {
        ResourceBudgetGuardian guardian = new();
        RuntimeHealth overBudget = CreateHealth(cpuPct: 6.5, rssBytes: 360UL * Mb);

        RuntimePolicy policySeq1 = guardian.Evaluate(1, overBudget, rowCount: 1000);
        RuntimePolicy policySeq2 = guardian.Evaluate(2, overBudget, rowCount: 1000);
        RuntimePolicy policySeq3 = guardian.Evaluate(3, overBudget, rowCount: 1000);
        RuntimePolicy policySeq4 = guardian.Evaluate(4, overBudget, rowCount: 1000);

        Assert.True(policySeq1.EmitTelemetryDelta);
        Assert.True(policySeq2.EmitTelemetryDelta);
        Assert.False(policySeq3.EmitTelemetryDelta);
        Assert.True(policySeq4.EmitTelemetryDelta);
    }

    [Fact]
    public void DegradeMode_StaysOff_BelowUpdatedBudgetThresholds()
    {
        ResourceBudgetGuardian guardian = new();
        RuntimeHealth belowBudget = CreateHealth(cpuPct: 5.9, rssBytes: 349UL * Mb);

        for (ulong seq = 1; seq <= 3; seq++)
        {
            _ = guardian.Evaluate(seq, belowBudget, rowCount: 1000);
        }

        Assert.False(guardian.IsDegraded());
    }

    [Fact]
    public void DegradeMode_Triggers_WhenEitherUpdatedBudgetThresholdIsReached()
    {
        ResourceBudgetGuardian cpuGuardian = new();
        RuntimeHealth cpuOnlyThreshold = CreateHealth(cpuPct: 6.0, rssBytes: 20UL * Mb);

        for (ulong seq = 1; seq <= 3; seq++)
        {
            _ = cpuGuardian.Evaluate(seq, cpuOnlyThreshold, rowCount: 1000);
        }

        Assert.True(cpuGuardian.IsDegraded());

        ResourceBudgetGuardian rssGuardian = new();
        RuntimeHealth rssOnlyThreshold = CreateHealth(cpuPct: 0.1, rssBytes: 350UL * Mb);

        for (ulong seq = 1; seq <= 3; seq++)
        {
            _ = rssGuardian.Evaluate(seq, rssOnlyThreshold, rowCount: 1000);
        }

        Assert.True(rssGuardian.IsDegraded());
    }

    private static RuntimeHealth CreateHealth(double cpuPct, ulong rssBytes)
    {
        return new RuntimeHealth
        {
            AppCpuPct = cpuPct,
            AppRssBytes = rssBytes,
        };
    }
}