using BatCave.Core.Domain;
using System;
using System.Collections.Generic;
using System.Linq;
using System.Threading;
using System.Threading.Channels;
using System.Threading.Tasks;

namespace BatCave.Services;

public interface IRuntimeEventGateway
{
    event EventHandler<ProcessDeltaBatch>? TelemetryDelta;

    event EventHandler<RuntimeHealth>? RuntimeHealthChanged;

    event EventHandler<CollectorWarning>? CollectorWarningRaised;

    void Publish(TickOutcome outcome);

    void PublishWarning(CollectorWarning warning);
}

public sealed class RuntimeGateway : IRuntimeEventGateway, IDisposable
{
    private const int TelemetryChannelCapacity = 64;
    private static readonly TimeSpan TelemetryFrameWindow = TimeSpan.FromMilliseconds(33);

    private readonly Dictionary<ProcessIdentity, ProcessSample> _pendingUpserts = new();
    private readonly HashSet<ProcessIdentity> _pendingExits = [];
    private readonly Channel<ProcessDeltaBatch> _telemetryChannel;
    private readonly CancellationTokenSource _coalescerCts = new();
    private readonly Task _coalescerTask;
    private ulong _pendingSeq;
    private bool _hasPendingTelemetry;
    private int _disposeSignaled;

    public RuntimeGateway()
    {
        _telemetryChannel = Channel.CreateBounded<ProcessDeltaBatch>(new BoundedChannelOptions(TelemetryChannelCapacity)
        {
            SingleReader = true,
            SingleWriter = false,
            FullMode = BoundedChannelFullMode.DropOldest,
        });

        _coalescerTask = Task.Run(RunTelemetryCoalescerAsync);
    }

    public event EventHandler<ProcessDeltaBatch>? TelemetryDelta;

    public event EventHandler<RuntimeHealth>? RuntimeHealthChanged;

    public event EventHandler<CollectorWarning>? CollectorWarningRaised;

    public void Publish(TickOutcome outcome)
    {
        if (ShouldEmitTelemetryDelta(outcome))
        {
            TryQueueTelemetryDelta(outcome.Delta);
        }

        if (Volatile.Read(ref _disposeSignaled) == 1)
        {
            return;
        }

        RuntimeHealthChanged?.Invoke(this, outcome.Health);

        if (outcome.Warning is not null)
        {
            CollectorWarningRaised?.Invoke(this, outcome.Warning);
        }
    }

    public void PublishWarning(CollectorWarning warning)
    {
        if (Volatile.Read(ref _disposeSignaled) == 1)
        {
            return;
        }

        CollectorWarningRaised?.Invoke(this, warning);
    }

    public void Dispose()
    {
        if (Interlocked.Exchange(ref _disposeSignaled, 1) == 1)
        {
            return;
        }

        _telemetryChannel.Writer.TryComplete();
        _coalescerCts.Cancel();

        try
        {
            _coalescerTask.GetAwaiter().GetResult();
        }
        catch (OperationCanceledException)
        {
        }

        _coalescerCts.Dispose();
    }

    private void TryQueueTelemetryDelta(ProcessDeltaBatch delta)
    {
        if (Volatile.Read(ref _disposeSignaled) == 1)
        {
            return;
        }

        _ = _telemetryChannel.Writer.TryWrite(delta);
    }

    private async Task RunTelemetryCoalescerAsync()
    {
        ChannelReader<ProcessDeltaBatch> reader = _telemetryChannel.Reader;

        try
        {
            while (await reader.WaitToReadAsync(_coalescerCts.Token).ConfigureAwait(false))
            {
                DrainTelemetryQueue(reader);
                EmitMergedTelemetryIfPending();
                await Task.Delay(TelemetryFrameWindow, _coalescerCts.Token).ConfigureAwait(false);
            }
        }
        catch (OperationCanceledException) when (_coalescerCts.IsCancellationRequested)
        {
        }
        finally
        {
            DrainTelemetryQueue(reader);
            EmitMergedTelemetryIfPending();
        }
    }

    private void DrainTelemetryQueue(ChannelReader<ProcessDeltaBatch> reader)
    {
        while (reader.TryRead(out ProcessDeltaBatch? delta))
        {
            if (delta is null)
            {
                continue;
            }

            MergeDelta(delta);
        }
    }

    private void EmitMergedTelemetryIfPending()
    {
        if (!_hasPendingTelemetry)
        {
            return;
        }

        TelemetryDelta?.Invoke(this, FlushPendingDelta());
    }

    private void MergeDelta(ProcessDeltaBatch delta)
    {
        _hasPendingTelemetry = true;

        if (delta.Seq > _pendingSeq)
        {
            _pendingSeq = delta.Seq;
        }

        foreach (ProcessSample sample in delta.Upserts)
        {
            UpsertPending(sample);
        }

        foreach (ProcessIdentity identity in delta.Exits)
        {
            RegisterExit(identity);
        }
    }

    private static bool ShouldEmitTelemetryDelta(TickOutcome outcome)
    {
        return outcome.EmitTelemetryDelta || HasDeltaChanges(outcome.Delta);
    }

    private static bool HasDeltaChanges(ProcessDeltaBatch delta)
    {
        return delta.Upserts.Count > 0 || delta.Exits.Count > 0;
    }

    private void UpsertPending(ProcessSample sample)
    {
        ProcessIdentity identity = sample.Identity();
        _pendingExits.Remove(identity);
        _pendingUpserts[identity] = sample;
    }

    private void RegisterExit(ProcessIdentity identity)
    {
        _pendingUpserts.Remove(identity);
        _pendingExits.Add(identity);
    }

    private ProcessDeltaBatch FlushPendingDelta()
    {
        ProcessDeltaBatch delta = new()
        {
            Seq = _pendingSeq,
            Upserts = _pendingUpserts.Values.ToList(),
            Exits = _pendingExits.ToList(),
        };

        _pendingUpserts.Clear();
        _pendingExits.Clear();
        _hasPendingTelemetry = false;
        return delta;
    }
}
