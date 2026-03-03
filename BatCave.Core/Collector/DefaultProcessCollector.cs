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
    private readonly Queue<string> _pendingWarnings = [];
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
        if (_bridge is null)
        {
            return _local.CollectTick(seq);
        }

        CaptureBridgeWarning(_bridge);
        BridgePollResult pollResult = _bridge.PollRows();
        CaptureBridgeWarning(_bridge);

        return pollResult.State == BridgePollState.Rows
            ? StampRowsWithTick(pollResult.Rows, seq)
            : CollectFromLocalAfterBridgeState(pollResult, seq);
    }

    private IReadOnlyList<ProcessSample> CollectFromLocalAfterBridgeState(BridgePollResult pollResult, ulong seq)
    {
        if (pollResult.State == BridgePollState.Faulted)
        {
            if (!string.IsNullOrWhiteSpace(pollResult.Reason))
            {
                _pendingWarnings.Enqueue($"elevated_bridge_faulted: {pollResult.Reason}");
            }

            _bridge?.Dispose();
            _bridge = null;
        }

        return _local.CollectTick(seq);
    }

    private static IReadOnlyList<ProcessSample> StampRowsWithTick(IReadOnlyList<ProcessSample> rows, ulong seq)
    {
        ulong timestamp = (ulong)DateTimeOffset.UtcNow.ToUnixTimeMilliseconds();
        return rows.Select(row => row with
        {
            Seq = seq,
            TsMs = timestamp,
        }).ToList();
    }

    public string? TakeWarning()
    {
        return _pendingWarnings.TryDequeue(out string? warning)
            ? warning
            : _local.TakeWarning();
    }

    public void Dispose()
    {
        _bridge?.Dispose();
    }

    private void CaptureBridgeWarning(ElevatedBridgeClient bridge)
    {
        string? warning = bridge.TakeWarning();
        if (!string.IsNullOrWhiteSpace(warning))
        {
            _pendingWarnings.Enqueue(warning);
        }
    }
}
