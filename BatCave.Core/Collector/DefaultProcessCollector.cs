using BatCave.Core.Abstractions;
using BatCave.Core.Domain;

namespace BatCave.Core.Collector;

public sealed class DefaultProcessCollector : IProcessCollector
{
    private readonly WindowsProcessCollector _collector;

    public DefaultProcessCollector()
    {
        _collector = new WindowsProcessCollector();
    }

    public IReadOnlyList<ProcessSample> CollectTick(ulong seq)
    {
        return _collector.CollectTick(seq);
    }

    public string? TakeWarning()
    {
        return _collector.TakeWarning();
    }
}
