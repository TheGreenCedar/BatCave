using BatCave.Core.Domain;
using System;
using System.Collections.Generic;
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
    private static readonly TimeSpan TelemetryFrameWindow = TimeSpan.FromMilliseconds(33);

    private readonly IRuntimeHealthService _runtimeHealthService;
    private readonly object _telemetrySync = new();
    private readonly Dictionary<ProcessIdentity, ProcessSample> _pendingUpserts = new();
    private readonly HashSet<ProcessIdentity> _pendingExits = [];
    private readonly Channel<bool> _telemetrySignalChannel;
    private readonly CancellationTokenSource _coalescerCts = new();
    private readonly Task _coalescerTask;
    private ulong _pendingSeq;
    private bool _hasPendingTelemetry;
    private int _disposeSignaled;

    public RuntimeGateway(IRuntimeHealthService runtimeHealthService)
    {
        _runtimeHealthService = runtimeHealthService;
        _telemetrySignalChannel = Channel.CreateBounded<bool>(new BoundedChannelOptions(1)
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
        if (Volatile.Read(ref _disposeSignaled) == 1)
        {
            return;
        }

        if (ShouldEmitTelemetryDelta(outcome))
        {
            TryQueueTelemetryDelta(outcome.Delta);
        }

        _runtimeHealthService.ReportHealth(outcome.Health);
        RuntimeHealthChanged?.Invoke(this, outcome.Health);

        if (outcome.Warning is not null)
        {
            _runtimeHealthService.ReportWarning(outcome.Warning);
            CollectorWarningRaised?.Invoke(this, outcome.Warning);
        }
    }

    public void PublishWarning(CollectorWarning warning)
    {
        if (Volatile.Read(ref _disposeSignaled) == 1)
        {
            return;
        }

        _runtimeHealthService.ReportWarning(warning);
        CollectorWarningRaised?.Invoke(this, warning);
    }

    public void Dispose()
    {
        if (Interlocked.Exchange(ref _disposeSignaled, 1) == 1)
        {
            return;
        }

        _telemetrySignalChannel.Writer.TryComplete();
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

        lock (_telemetrySync)
        {
            MergeDelta(delta);
        }

        _ = _telemetrySignalChannel.Writer.TryWrite(true);
    }

    private async Task RunTelemetryCoalescerAsync()
    {
        ChannelReader<bool> reader = _telemetrySignalChannel.Reader;

        try
        {
            while (await reader.WaitToReadAsync(_coalescerCts.Token).ConfigureAwait(false))
            {
                DrainTelemetrySignals(reader);
                await Task.Delay(TelemetryFrameWindow, _coalescerCts.Token).ConfigureAwait(false);
                EmitMergedTelemetryIfPending();
            }
        }
        catch (OperationCanceledException) when (_coalescerCts.IsCancellationRequested)
        {
        }
        finally
        {
            DrainTelemetrySignals(reader);
            EmitMergedTelemetryIfPending();
        }
    }

    private static void DrainTelemetrySignals(ChannelReader<bool> reader)
    {
        while (reader.TryRead(out _))
        {
        }
    }

    private void EmitMergedTelemetryIfPending()
    {
        ProcessDeltaBatch? delta = FlushPendingDeltaOrNull();
        if (delta is null)
        {
            return;
        }

        TelemetryDelta?.Invoke(this, delta);
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

    private ProcessDeltaBatch? FlushPendingDeltaOrNull()
    {
        lock (_telemetrySync)
        {
            if (!_hasPendingTelemetry)
            {
                return null;
            }

            ProcessDeltaBatch delta = new()
            {
                Seq = _pendingSeq,
                Upserts = [.. _pendingUpserts.Values],
                Exits = [.. _pendingExits],
            };

            _pendingUpserts.Clear();
            _pendingExits.Clear();
            _hasPendingTelemetry = false;
            return delta;
        }
    }
}
