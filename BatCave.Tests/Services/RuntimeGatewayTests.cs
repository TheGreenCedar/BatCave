using BatCave.Core.Domain;
using BatCave.Services;
using BatCave.Tests.TestSupport;

namespace BatCave.Tests.Services;

public class RuntimeGatewayTests
{
    [Fact]
    public void Publish_WhenSuppressedAndDeltaIsEmpty_DoesNotRaiseTelemetryDelta_ButRaisesHealth()
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
                Upserts = [],
                Exits = [],
            },
            Health = new RuntimeHealth { Seq = 1 },
            EmitTelemetryDelta = false,
        });

        Assert.Equal(0, telemetryRaised);
        Assert.Equal(1, healthRaised);
    }

    [Fact]
    public void Publish_WhenSuppressedButDeltaHasChanges_EmitsImmediately()
    {
        RuntimeGateway gateway = new();
        List<ProcessDeltaBatch> emitted = [];

        gateway.TelemetryDelta += (_, delta) => emitted.Add(delta);

        ProcessSample sample = Sample(pid: 31, seq: 1);

        gateway.Publish(new TickOutcome
        {
            Delta = new ProcessDeltaBatch
            {
                Seq = 1,
                Upserts = [sample],
                Exits = [],
            },
            Health = new RuntimeHealth { Seq = 1 },
            EmitTelemetryDelta = false,
        });

        Assert.Single(emitted);
        ProcessDeltaBatch delta = emitted[0];
        Assert.Equal(1UL, delta.Seq);
        Assert.Single(delta.Upserts);
        Assert.Equal(sample.Identity(), delta.Upserts[0].Identity());
        Assert.Empty(delta.Exits);
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
        return TestProcessSamples.Create(
            pid: pid,
            seq: seq,
            tsMs: seq,
            parentPid: 1,
            startTimeMs: seq * 100,
            name: $"proc-{pid}",
            cpuPct: 4.0,
            rssBytes: 1000,
            privateBytes: 600,
            ioReadBps: 10,
            ioWriteBps: 12,
            otherIoBps: 8,
            threads: 3,
            handles: 6,
            accessState: AccessState.Full);
    }
}
