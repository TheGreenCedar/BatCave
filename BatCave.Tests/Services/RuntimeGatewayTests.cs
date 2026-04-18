using BatCave.Core.Domain;
using BatCave.Services;
using BatCave.Tests.TestSupport;
using System.Diagnostics;

namespace BatCave.Tests.Services;

public class RuntimeGatewayTests
{
    [Fact]
    public async Task Publish_WhenSuppressedAndDeltaIsEmpty_DoesNotRaiseTelemetryDelta_ButRaisesHealth()
    {
        using RuntimeGateway gateway = new(new RuntimeHealthService());
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

        await Task.Delay(TimeSpan.FromMilliseconds(80));

        Assert.Equal(0, telemetryRaised);
        Assert.Equal(1, healthRaised);
    }

    [Fact]
    public async Task Publish_WhenSuppressedButDeltaHasChanges_EmitsEventually()
    {
        using RuntimeGateway gateway = new(new RuntimeHealthService());
        TaskCompletionSource<ProcessDeltaBatch> emitted = new(TaskCreationOptions.RunContinuationsAsynchronously);

        gateway.TelemetryDelta += (_, delta) => emitted.TrySetResult(delta);

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

        ProcessDeltaBatch delta = await emitted.Task.WaitAsync(TimeSpan.FromMilliseconds(500));
        Assert.Equal(1UL, delta.Seq);
        Assert.Single(delta.Upserts);
        Assert.Equal(sample.Identity(), delta.Upserts[0].Identity());
        Assert.Empty(delta.Exits);
    }

    [Fact]
    public async Task Publish_WhenRapidDeltasArrive_CoalescesToLatestState()
    {
        using RuntimeGateway gateway = new(new RuntimeHealthService());
        List<ProcessDeltaBatch> emitted = [];
        object sync = new();

        gateway.TelemetryDelta += (_, delta) =>
        {
            lock (sync)
            {
                emitted.Add(delta);
            }
        };

        ProcessSample first = Sample(pid: 31, seq: 1);
        ProcessSample latestForFirst = first with
        {
            Seq = 2,
            TsMs = 2,
        };
        ProcessSample second = Sample(pid: 42, seq: 3);

        gateway.Publish(new TickOutcome
        {
            Delta = new ProcessDeltaBatch
            {
                Seq = 1,
                Upserts = [first],
                Exits = [],
            },
            Health = new RuntimeHealth { Seq = 1 },
            EmitTelemetryDelta = false,
        });
        gateway.Publish(new TickOutcome
        {
            Delta = new ProcessDeltaBatch
            {
                Seq = 2,
                Upserts = [latestForFirst],
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
                Upserts = [second],
                Exits = [],
            },
            Health = new RuntimeHealth { Seq = 3 },
            EmitTelemetryDelta = false,
        });

        await Task.Delay(TimeSpan.FromMilliseconds(120));

        ProcessDeltaBatch? latest;
        lock (sync)
        {
            latest = emitted.LastOrDefault();
        }

        Assert.NotNull(latest);
        Assert.Equal(3UL, latest!.Seq);
        Assert.Equal(2, latest.Upserts.Count);

        ProcessSample firstIdentitySample = latest.Upserts.Single(sample => sample.Identity() == latestForFirst.Identity());
        Assert.Equal(2UL, firstIdentitySample.Seq);
    }

    [Fact]
    public async Task Publish_BurstingTelemetry_EmitsAtFrameCadence()
    {
        using RuntimeGateway gateway = new(new RuntimeHealthService());
        List<long> emissionMs = [];
        object sync = new();
        Stopwatch stopwatch = Stopwatch.StartNew();

        gateway.TelemetryDelta += (_, _) =>
        {
            lock (sync)
            {
                emissionMs.Add(stopwatch.ElapsedMilliseconds);
            }
        };

        for (int i = 1; i <= 24; i++)
        {
            gateway.Publish(new TickOutcome
            {
                Delta = new ProcessDeltaBatch
                {
                    Seq = (ulong)i,
                    Upserts = [Sample(pid: (uint)(30 + i), seq: (ulong)i)],
                    Exits = [],
                },
                Health = new RuntimeHealth { Seq = (ulong)i },
                EmitTelemetryDelta = false,
            });

            await Task.Delay(TimeSpan.FromMilliseconds(5));
        }

        await Task.Delay(TimeSpan.FromMilliseconds(180));

        long[] timestamps;
        lock (sync)
        {
            timestamps = [.. emissionMs];
        }

        Assert.True(timestamps.Length >= 2, $"Expected multiple frame-window emissions but saw {timestamps.Length}.");
        for (int i = 1; i < timestamps.Length; i++)
        {
            long elapsed = timestamps[i] - timestamps[i - 1];
            Assert.True(elapsed >= 20, $"Expected at least 20ms between emissions; observed {elapsed}ms.");
        }
    }

    [Fact]
    public async Task Publish_WhenQueueOverflows_PreservesLatestTruthWithExitSupersedingUpsert()
    {
        using RuntimeGateway gateway = new(new RuntimeHealthService());
        TaskCompletionSource<ProcessDeltaBatch> emitted = new(TaskCreationOptions.RunContinuationsAsynchronously);
        ProcessIdentity identity = Sample(pid: 900, seq: 1).Identity();

        gateway.TelemetryDelta += (_, delta) =>
        {
            if (delta.Exits.Contains(identity))
            {
                emitted.TrySetResult(delta);
            }
        };

        for (int index = 1; index <= 400; index++)
        {
            ProcessSample sample = Sample(pid: 900, seq: (ulong)index);
            gateway.Publish(new TickOutcome
            {
                Delta = new ProcessDeltaBatch
                {
                    Seq = (ulong)index,
                    Upserts = [sample],
                    Exits = [],
                },
                Health = new RuntimeHealth { Seq = (ulong)index },
                EmitTelemetryDelta = true,
            });
        }

        gateway.Publish(new TickOutcome
        {
            Delta = new ProcessDeltaBatch
            {
                Seq = 401,
                Upserts = [],
                Exits = [identity],
            },
            Health = new RuntimeHealth { Seq = 401 },
            EmitTelemetryDelta = true,
        });

        ProcessDeltaBatch flushed = await emitted.Task.WaitAsync(TimeSpan.FromMilliseconds(1000));
        Assert.Equal(401UL, flushed.Seq);
        Assert.DoesNotContain(flushed.Upserts, sample => sample.Identity() == identity);
        Assert.Contains(identity, flushed.Exits);
    }

    [Fact]
    public async Task Publish_BurstAcrossDistinctIdentities_PreservesEveryIrreversibleExit()
    {
        using RuntimeGateway gateway = new(new RuntimeHealthService());
        TaskCompletionSource<ProcessDeltaBatch> emitted = new(TaskCreationOptions.RunContinuationsAsynchronously);
        ProcessIdentity firstIdentity = Sample(pid: 910, seq: 1).Identity();
        ProcessIdentity secondIdentity = Sample(pid: 911, seq: 1).Identity();

        gateway.TelemetryDelta += (_, delta) =>
        {
            if (delta.Exits.Contains(firstIdentity) && delta.Exits.Contains(secondIdentity))
            {
                emitted.TrySetResult(delta);
            }
        };

        for (int index = 1; index <= 400; index++)
        {
            gateway.Publish(new TickOutcome
            {
                Delta = new ProcessDeltaBatch
                {
                    Seq = (ulong)index,
                    Upserts =
                    [
                        Sample(pid: 910, seq: (ulong)index),
                        Sample(pid: 911, seq: (ulong)index),
                    ],
                    Exits = [],
                },
                Health = new RuntimeHealth { Seq = (ulong)index },
                EmitTelemetryDelta = true,
            });
        }

        gateway.Publish(new TickOutcome
        {
            Delta = new ProcessDeltaBatch
            {
                Seq = 401,
                Upserts = [],
                Exits = [firstIdentity],
            },
            Health = new RuntimeHealth { Seq = 401 },
            EmitTelemetryDelta = true,
        });
        gateway.Publish(new TickOutcome
        {
            Delta = new ProcessDeltaBatch
            {
                Seq = 402,
                Upserts = [],
                Exits = [secondIdentity],
            },
            Health = new RuntimeHealth { Seq = 402 },
            EmitTelemetryDelta = true,
        });

        ProcessDeltaBatch flushed = await emitted.Task.WaitAsync(TimeSpan.FromMilliseconds(1000));
        Assert.Equal(402UL, flushed.Seq);
        Assert.Contains(firstIdentity, flushed.Exits);
        Assert.Contains(secondIdentity, flushed.Exits);
        Assert.DoesNotContain(flushed.Upserts, sample => sample.Identity() == firstIdentity || sample.Identity() == secondIdentity);
    }

    [Fact]
    public void Publish_HealthAndWarningRemainImmediateAndIndependent()
    {
        using RuntimeGateway gateway = new(new RuntimeHealthService());
        int healthRaised = 0;
        int warningRaised = 0;

        gateway.RuntimeHealthChanged += (_, _) => healthRaised++;
        gateway.CollectorWarningRaised += (_, _) => warningRaised++;

        gateway.Publish(new TickOutcome
        {
            Delta = new ProcessDeltaBatch
            {
                Seq = 1,
                Upserts = [Sample(pid: 31, seq: 1)],
                Exits = [],
            },
            Health = new RuntimeHealth { Seq = 1 },
            Warning = new CollectorWarning
            {
                Seq = 1,
                Message = "inline warning",
            },
            EmitTelemetryDelta = false,
        });

        Assert.Equal(1, healthRaised);
        Assert.Equal(1, warningRaised);

        gateway.PublishWarning(new CollectorWarning
        {
            Seq = 2,
            Message = "out-of-band warning",
        });

        Assert.Equal(1, healthRaised);
        Assert.Equal(2, warningRaised);
    }

    [Fact]
    public async Task Dispose_CancelsCoalescerAndDrainsPendingTelemetry()
    {
        RuntimeGateway gateway = new(new RuntimeHealthService());
        TaskCompletionSource<ProcessDeltaBatch> emitted = new(TaskCreationOptions.RunContinuationsAsynchronously);

        gateway.TelemetryDelta += (_, delta) => emitted.TrySetResult(delta);

        gateway.Publish(new TickOutcome
        {
            Delta = new ProcessDeltaBatch
            {
                Seq = 7,
                Upserts = [Sample(pid: 71, seq: 7)],
                Exits = [],
            },
            Health = new RuntimeHealth { Seq = 7 },
            EmitTelemetryDelta = false,
        });

        gateway.Dispose();

        ProcessDeltaBatch flushed = await emitted.Task.WaitAsync(TimeSpan.FromMilliseconds(500));
        Assert.Equal(7UL, flushed.Seq);
        Assert.Single(flushed.Upserts);
    }

    [Fact]
    public void PublishWarning_RaisesCollectorWarningEvent()
    {
        using RuntimeGateway gateway = new(new RuntimeHealthService());
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
