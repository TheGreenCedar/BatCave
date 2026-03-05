using BatCave.Core.Abstractions;
using BatCave.Core.Domain;

namespace BatCave.Core.Collector;

public sealed class DefaultProcessCollectorFactory : IProcessCollectorFactory
{
    public async ValueTask<CollectorActivationResult> CreateAsync(bool adminMode, CancellationToken ct)
    {
        if (!adminMode)
        {
            return new CollectorActivationResult(
                Collector: new DefaultProcessCollector(),
                EffectiveAdminMode: false,
                Warning: null);
        }

        try
        {
            ElevatedBridgeClient bridge = await ElevatedBridgeClient.LaunchAsync(ct).ConfigureAwait(false);
            return new CollectorActivationResult(
                Collector: new DefaultProcessCollector(bridge),
                EffectiveAdminMode: true,
                Warning: null);
        }
        catch (OperationCanceledException)
        {
            throw;
        }
        catch (Exception ex)
        {
            return new CollectorActivationResult(
                Collector: new DefaultProcessCollector(),
                EffectiveAdminMode: false,
                Warning:
                    $"admin_mode_start_failed requested_admin_mode=true fallback_admin_mode=false error={ex.GetType().Name}: {ex.Message}");
        }
    }
}

public sealed class DefaultProcessCollector : IProcessCollector, IDisposable
{
    private readonly WindowsProcessCollector _local = new();
    private readonly Queue<string> _pendingWarnings = [];
    private ElevatedBridgeClient? _bridge;

    public DefaultProcessCollector()
    {
    }

    internal DefaultProcessCollector(ElevatedBridgeClient bridge)
    {
        _bridge = bridge;
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

    private static List<ProcessSample> StampRowsWithTick(List<ProcessSample> rows, ulong seq)
    {
        ulong timestamp = (ulong)DateTimeOffset.UtcNow.ToUnixTimeMilliseconds();
        return
        [
            .. rows.Select(row => row with
        {
            Seq = seq,
            TsMs = timestamp,
        }),
        ];
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
