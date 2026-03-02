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

        if (outcome.EmitTelemetryDelta)
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

        for (int index = 0; index < delta.Upserts.Count; index++)
        {
            ProcessSample sample = delta.Upserts[index];
            ProcessIdentity identity = sample.Identity();
            _pendingExits.Remove(identity);
            _pendingUpserts[identity] = sample;
        }

        for (int index = 0; index < delta.Exits.Count; index++)
        {
            ProcessIdentity identity = delta.Exits[index];
            _pendingUpserts.Remove(identity);
            _pendingExits.Add(identity);
        }
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
