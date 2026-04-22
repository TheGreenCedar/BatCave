using BatCave.Runtime.Contracts;

namespace BatCave.Runtime.Presentation;

public sealed record RuntimeViewState
{
    public RuntimeSnapshot Snapshot { get; init; } = new();
    public IReadOnlyList<ProcessSample> Rows { get; init; } = Array.Empty<ProcessSample>();
    public ProcessIdentity? SelectedIdentity { get; init; }
    public ProcessSample? SelectedProcess { get; init; }
    public RuntimeWarning? ActiveWarning { get; init; }
    public string? HealthBanner { get; init; }

    public bool HasHealthBanner => !string.IsNullOrWhiteSpace(HealthBanner);
}

public static class RuntimeViewReducer
{
    public static RuntimeViewState Reduce(RuntimeViewState? previous, RuntimeSnapshot snapshot)
    {
        ProcessSample? selected = ResolveSelection(previous?.SelectedIdentity, snapshot.Rows);
        RuntimeWarning? activeWarning = snapshot.Warnings.LastOrDefault();
        string? healthBanner = activeWarning?.Message;
        if (string.IsNullOrWhiteSpace(healthBanner) && snapshot.Health.DegradeMode)
        {
            healthBanner = snapshot.Health.StatusSummary;
        }

        return new RuntimeViewState
        {
            Snapshot = snapshot,
            Rows = Freeze(snapshot.Rows),
            SelectedIdentity = selected?.Identity(),
            SelectedProcess = selected,
            ActiveWarning = activeWarning,
            HealthBanner = healthBanner,
        };
    }

    private static ProcessSample? ResolveSelection(ProcessIdentity? selectedIdentity, IReadOnlyList<ProcessSample> rows)
    {
        if (selectedIdentity.HasValue)
        {
            ProcessSample? previous = rows.FirstOrDefault(row => row.Identity().Equals(selectedIdentity.Value));
            if (previous is not null)
            {
                return previous;
            }
        }

        return null;
    }

    private static IReadOnlyList<T> Freeze<T>(IEnumerable<T> values) => Array.AsReadOnly(values.ToArray());
}
