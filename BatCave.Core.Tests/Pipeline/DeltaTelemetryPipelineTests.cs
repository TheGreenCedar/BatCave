using BatCave.Core.Domain;
using BatCave.Core.Pipeline;

namespace BatCave.Core.Tests.Pipeline;

public class DeltaTelemetryPipelineTests
{
    [Fact]
    public void PidReuse_EmitsExitForStaleIdentity()
    {
        DeltaTelemetryPipeline pipeline = new();
        ProcessSample oldIdentity = Sample(pid: 55, startTimeMs: 1_000, cpu: 5);
        ProcessSample reusedPid = Sample(pid: 55, startTimeMs: 9_000, cpu: 7);

        pipeline.ApplyRaw(1, [oldIdentity]);
        ProcessDeltaBatch delta = pipeline.ApplyRaw(2, [reusedPid]);

        Assert.Single(delta.Upserts);
        Assert.Equal(reusedPid.Identity(), delta.Upserts[0].Identity());
        Assert.Single(delta.Exits);
        Assert.Equal(oldIdentity.Identity(), delta.Exits[0]);
    }

    [Fact]
    public void UnchangedRows_EmitHeartbeatEveryEighthTick()
    {
        DeltaTelemetryPipeline pipeline = new();
        ProcessSample baseline = Sample(pid: 100, startTimeMs: 2_000, cpu: 1);

        ProcessDeltaBatch first = pipeline.ApplyRaw(1, [baseline]);
        ProcessDeltaBatch second = pipeline.ApplyRaw(2, [baseline]);
        ProcessDeltaBatch eighth = pipeline.ApplyRaw(8, [baseline]);
        ProcessDeltaBatch ninth = pipeline.ApplyRaw(9, [baseline]);

        Assert.Single(first.Upserts);
        Assert.Empty(second.Upserts);
        Assert.Empty(eighth.Upserts);
        Assert.Single(ninth.Upserts);
    }

    [Fact]
    public void LowCpuDelta_StillEmitsUpsert()
    {
        DeltaTelemetryPipeline pipeline = new();
        ProcessSample baseline = Sample(pid: 77, startTimeMs: 3_000, cpu: 1.0000);
        ProcessSample withSmallCpuDelta = baseline with { CpuPct = 1.0002 };

        pipeline.ApplyRaw(1, [baseline]);
        ProcessDeltaBatch delta = pipeline.ApplyRaw(2, [withSmallCpuDelta]);

        Assert.Single(delta.Upserts);
        Assert.Equal(1.0002, delta.Upserts[0].CpuPct, 4);
    }

    [Fact]
    public void WarmCacheSeeding_ReconcilesExitedIdentities()
    {
        DeltaTelemetryPipeline pipeline = new();
        ProcessSample warmCacheRow = Sample(pid: 201, startTimeMs: 1_111, cpu: 0.2);
        ProcessSample liveRow = Sample(pid: 202, startTimeMs: 2_222, cpu: 0.3);

        pipeline.SeedFromWarmCache([warmCacheRow]);
        ProcessDeltaBatch delta = pipeline.ApplyRaw(1, [liveRow]);

        Assert.Single(delta.Upserts);
        Assert.Equal(liveRow.Identity(), delta.Upserts[0].Identity());
        Assert.Single(delta.Exits);
        Assert.Equal(warmCacheRow.Identity(), delta.Exits[0]);
    }

    private static ProcessSample Sample(uint pid, ulong startTimeMs, double cpu)
    {
        return new ProcessSample
        {
            Seq = 1,
            TsMs = 10,
            Pid = pid,
            ParentPid = 1,
            StartTimeMs = startTimeMs,
            Name = $"proc-{pid}",
            CpuPct = cpu,
            RssBytes = 10_000,
            PrivateBytes = 5_000,
            IoReadBps = 10,
            IoWriteBps = 11,
            NetBps = 12,
            Threads = 4,
            Handles = 5,
            AccessState = AccessState.Full,
        };
    }
}
