using BatCave.Core.Domain;
using BatCave.Core.Runtime;

namespace BatCave.Core.Tests.Runtime;

public class ResourceBudgetGuardianTests
{
    [Fact]
    public void DegradeMode_TransitionsByOverBudgetAndRecoveryStreaks()
    {
        ResourceBudgetGuardian guardian = new();
        RuntimeHealth overBudget = new()
        {
            AppCpuPct = 2.0,
            AppRssBytes = 200UL * 1024UL * 1024UL,
        };
        RuntimeHealth healthy = new()
        {
            AppCpuPct = 0.1,
            AppRssBytes = 20UL * 1024UL * 1024UL,
        };

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
        RuntimeHealth overBudget = new()
        {
            AppCpuPct = 2.0,
            AppRssBytes = 200UL * 1024UL * 1024UL,
        };

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
        RuntimeHealth overBudget = new()
        {
            AppCpuPct = 2.0,
            AppRssBytes = 200UL * 1024UL * 1024UL,
        };

        RuntimePolicy policySeq1 = guardian.Evaluate(1, overBudget, rowCount: 1000);
        RuntimePolicy policySeq2 = guardian.Evaluate(2, overBudget, rowCount: 1000);
        RuntimePolicy policySeq3 = guardian.Evaluate(3, overBudget, rowCount: 1000);
        RuntimePolicy policySeq4 = guardian.Evaluate(4, overBudget, rowCount: 1000);

        Assert.True(policySeq1.EmitTelemetryDelta);
        Assert.True(policySeq2.EmitTelemetryDelta);
        Assert.False(policySeq3.EmitTelemetryDelta);
        Assert.True(policySeq4.EmitTelemetryDelta);
    }
}
