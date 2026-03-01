using BatCave.Core.Abstractions;
using BatCave.Core.Domain;

namespace BatCave.Core.Collector;

public sealed class DefaultProcessCollectorFactory : IProcessCollectorFactory
{
    public IProcessCollector Create(bool adminMode)
    {
        return new DefaultProcessCollector(adminMode);
    }
}

public sealed class DefaultProcessCollector : IProcessCollector, IDisposable
{
    private readonly WindowsProcessCollector _local;
    private ElevatedBridgeClient? _bridge;

    public DefaultProcessCollector(bool adminMode)
    {
        _local = new WindowsProcessCollector();
        if (adminMode)
        {
            _bridge = ElevatedBridgeClient.LaunchAsync(CancellationToken.None).GetAwaiter().GetResult();
        }
    }

    public IReadOnlyList<ProcessSample> CollectTick(ulong seq)
    {
        if (_bridge is not null)
        {
            BridgePollResult pollResult = _bridge.PollRows();
            switch (pollResult.State)
            {
                case BridgePollState.Rows:
                {
                    ulong timestamp = (ulong)DateTimeOffset.UtcNow.ToUnixTimeMilliseconds();
                    return pollResult.Rows.Select(row => row with
                    {
                        Seq = seq,
                        TsMs = timestamp,
                    }).ToList();
                }
                case BridgePollState.Pending:
                    return _local.CollectTick(seq);
                case BridgePollState.Faulted:
                    _bridge.Dispose();
                    _bridge = null;
                    return _local.CollectTick(seq);
            }
        }

        return _local.CollectTick(seq);
    }

    public string? TakeWarning()
    {
        return _local.TakeWarning();
    }

    public void Dispose()
    {
        _bridge?.Dispose();
    }
}
