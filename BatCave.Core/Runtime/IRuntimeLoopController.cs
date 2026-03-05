using BatCave.Core.Domain;

namespace BatCave.Core.Runtime;

public interface IRuntimeLoopController
{
    event EventHandler<TickOutcome>? TickCompleted;

    event EventHandler<TickFaultedEventArgs>? TickFaulted;

    long CurrentGeneration { get; }

    void Start(long generation);

    void StopAndAdvanceGeneration();

    Task StopAndAdvanceGenerationAsync(CancellationToken ct);
}
