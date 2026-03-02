using System;
using System.Collections.Generic;
using System.Linq;
using BatCave.Core.Domain;

namespace BatCave.Services;

public interface IRuntimeEventGateway
{
    event EventHandler<ProcessDeltaBatch>? TelemetryDelta;

    event EventHandler<RuntimeHealth>? RuntimeHealthChanged;

    event EventHandler<CollectorWarning>? CollectorWarningRaised;

    void Publish(TickOutcome outcome);

    void PublishWarning(CollectorWarning warning);
}

public sealed class RuntimeGateway : IRuntimeEventGateway
{
    private readonly Dictionary<ProcessIdentity, ProcessSample> _pendingUpserts = new();
    private readonly HashSet<ProcessIdentity> _pendingExits = [];
    private ulong _pendingSeq;

    public event EventHandler<ProcessDeltaBatch>? TelemetryDelta;

    public event EventHandler<RuntimeHealth>? RuntimeHealthChanged;

    public event EventHandler<CollectorWarning>? CollectorWarningRaised;

    public void Publish(TickOutcome outcome)
    {
        MergeDelta(outcome.Delta);
        if (ShouldEmitTelemetryDelta(outcome))
        {
            TelemetryDelta?.Invoke(this, FlushPendingDelta());
        }

        RuntimeHealthChanged?.Invoke(this, outcome.Health);

        if (outcome.Warning is not null)
        {
            CollectorWarningRaised?.Invoke(this, outcome.Warning);
        }
    }

    public void PublishWarning(CollectorWarning warning)
    {
        CollectorWarningRaised?.Invoke(this, warning);
    }

    private void MergeDelta(ProcessDeltaBatch delta)
    {
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
        return delta;
    }
}
