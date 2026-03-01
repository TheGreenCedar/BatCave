using System;
using BatCave.Core.Domain;

namespace BatCave.Services;

public interface IRuntimeEventGateway
{
    event EventHandler<ProcessDeltaBatch>? TelemetryDelta;

    event EventHandler<RuntimeHealth>? RuntimeHealthChanged;

    event EventHandler<CollectorWarning>? CollectorWarningRaised;

    void Publish(TickOutcome outcome);
}

public sealed class RuntimeGateway : IRuntimeEventGateway
{
    public event EventHandler<ProcessDeltaBatch>? TelemetryDelta;

    public event EventHandler<RuntimeHealth>? RuntimeHealthChanged;

    public event EventHandler<CollectorWarning>? CollectorWarningRaised;

    public void Publish(TickOutcome outcome)
    {
        if (outcome.EmitTelemetryDelta)
        {
            TelemetryDelta?.Invoke(this, outcome.Delta);
        }

        RuntimeHealthChanged?.Invoke(this, outcome.Health);

        if (outcome.Warning is not null)
        {
            CollectorWarningRaised?.Invoke(this, outcome.Warning);
        }
    }
}
