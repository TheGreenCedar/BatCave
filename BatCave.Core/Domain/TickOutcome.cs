using BatCave.Core.Domain;

namespace BatCave.Core.Domain;

public sealed record TickOutcome
{
    public ProcessDeltaBatch Delta { get; init; } = new();

    public RuntimeHealth Health { get; init; } = new();

    public CollectorWarning? Warning { get; init; }

    public bool EmitTelemetryDelta { get; init; }
}
