using BatCave.Core.Domain;
using BatCave.Services;

namespace BatCave.Tests.Services;

public class RuntimeGatewayTests
{
    [Fact]
    public void Publish_WhenSuppressed_DoesNotRaiseTelemetryDelta_ButRaisesHealth()
    {
        RuntimeGateway gateway = new();
        int telemetryRaised = 0;
        int healthRaised = 0;

        gateway.TelemetryDelta += (_, _) => telemetryRaised++;
        gateway.RuntimeHealthChanged += (_, _) => healthRaised++;

        gateway.Publish(new TickOutcome
        {
            Delta = new ProcessDeltaBatch
            {
                Seq = 1,
                Upserts = [Sample(pid: 11, seq: 1)],
                Exits = [],
            },
            Health = new RuntimeHealth { Seq = 1 },
            EmitTelemetryDelta = false,
        });

        Assert.Equal(0, telemetryRaised);
        Assert.Equal(1, healthRaised);
    }

    [Fact]
    public void Publish_WhenSuppressedThenEmitted_FlushesMergedPendingDelta()
    {
        RuntimeGateway gateway = new();
        List<ProcessDeltaBatch> emitted = [];

        gateway.TelemetryDelta += (_, delta) => emitted.Add(delta);

        ProcessSample first = Sample(pid: 31, seq: 1);
        ProcessSample updated = first with { Seq = 2, TsMs = 2, CpuPct = 7.5 };
        ProcessIdentity exited = new(77, 7700);
        ProcessSample extra = Sample(pid: 88, seq: 2);

        gateway.Publish(new TickOutcome
        {
            Delta = new ProcessDeltaBatch
            {
                Seq = 1,
                Upserts = [first],
                Exits = [exited],
            },
            Health = new RuntimeHealth { Seq = 1 },
            EmitTelemetryDelta = false,
        });

        gateway.Publish(new TickOutcome
        {
            Delta = new ProcessDeltaBatch
            {
                Seq = 2,
                Upserts = [updated, extra],
                Exits = [],
            },
            Health = new RuntimeHealth { Seq = 2 },
            EmitTelemetryDelta = false,
        });

        gateway.Publish(new TickOutcome
        {
            Delta = new ProcessDeltaBatch
            {
                Seq = 3,
                Upserts = [],
                Exits = [],
            },
            Health = new RuntimeHealth { Seq = 3 },
            EmitTelemetryDelta = true,
        });

        Assert.Single(emitted);
        ProcessDeltaBatch delta = emitted[0];
        Assert.Equal(3UL, delta.Seq);
        Assert.Equal(2, delta.Upserts.Count);
        Assert.Contains(delta.Upserts, row => row.Identity() == updated.Identity() && Math.Abs(row.CpuPct - 7.5) < 0.001);
        Assert.Contains(delta.Upserts, row => row.Identity() == extra.Identity());
        Assert.Single(delta.Exits);
        Assert.Equal(exited, delta.Exits[0]);
    }

    [Fact]
    public void PublishWarning_RaisesCollectorWarningEvent()
    {
        RuntimeGateway gateway = new();
        CollectorWarning? captured = null;

        gateway.CollectorWarningRaised += (_, warning) => captured = warning;

        gateway.PublishWarning(new CollectorWarning
        {
            Seq = 0,
            Message = "runtime loop fault",
        });

        Assert.NotNull(captured);
        Assert.Equal("runtime loop fault", captured!.Message);
    }

    private static ProcessSample Sample(uint pid, ulong seq)
    {
        return new ProcessSample
        {
            Seq = seq,
            TsMs = seq,
            Pid = pid,
            ParentPid = 1,
            StartTimeMs = seq * 100,
            Name = $"proc-{pid}",
            CpuPct = 4.0,
            RssBytes = 1000,
            PrivateBytes = 600,
            IoReadBps = 10,
            IoWriteBps = 12,
            OtherIoBps = 8,
            Threads = 3,
            Handles = 6,
            AccessState = AccessState.Full,
        };
    }
}
