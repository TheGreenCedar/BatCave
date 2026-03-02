using BatCave.Core.Abstractions;
using BatCave.Core.Domain;

namespace BatCave.Core.Pipeline;

public sealed class DeltaTelemetryPipeline : ITelemetryPipeline
{
    private const ulong HeartbeatIntervalTicks = 8;

    private readonly Dictionary<ProcessIdentity, ProcessSample> _previous = new();
    private readonly Dictionary<ProcessIdentity, ulong> _lastEmittedSeq = new();
    private readonly HashSet<ProcessIdentity> _seenThisTick = [];
    private readonly List<ProcessIdentity> _staleIdentities = [];

    public void SeedFromWarmCache(IReadOnlyList<ProcessSample> rows)
    {
        _previous.Clear();
        _lastEmittedSeq.Clear();

        foreach (ProcessSample sample in rows)
        {
            _previous[sample.Identity()] = sample;
        }
    }

    public ProcessDeltaBatch ApplyRaw(ulong seq, IReadOnlyList<ProcessSample> raw)
    {
        _seenThisTick.Clear();
        _staleIdentities.Clear();
        List<ProcessSample> upserts = new(raw.Count);

        foreach (ProcessSample sample in raw)
        {
            ProcessIdentity identity = sample.Identity();
            _seenThisTick.Add(identity);

            bool changed = !_previous.TryGetValue(identity, out ProcessSample? previous) || !EquivalentSample(previous, sample);
            ulong previousEmitSeq = _lastEmittedSeq.TryGetValue(identity, out ulong emitSeq) ? emitSeq : 0;
            bool dueForHeartbeat = seq >= previousEmitSeq && seq - previousEmitSeq >= HeartbeatIntervalTicks;

            if (changed || dueForHeartbeat)
            {
                upserts.Add(sample);
                _lastEmittedSeq[identity] = seq;
            }

            _previous[identity] = sample;
        }

        foreach (ProcessIdentity identity in _previous.Keys)
        {
            if (!_seenThisTick.Contains(identity))
            {
                _staleIdentities.Add(identity);
            }
        }

        List<ProcessIdentity> exits = new(_staleIdentities.Count);
        foreach (ProcessIdentity staleIdentity in _staleIdentities)
        {
            _previous.Remove(staleIdentity);
            _lastEmittedSeq.Remove(staleIdentity);
            exits.Add(staleIdentity);
        }

        return new ProcessDeltaBatch
        {
            Seq = seq,
            Upserts = upserts,
            Exits = exits,
        };
    }

    private static bool EquivalentSample(ProcessSample left, ProcessSample right)
    {
        return CpuEquivalent(left.CpuPct, right.CpuPct)
               && left.RssBytes == right.RssBytes
               && left.PrivateBytes == right.PrivateBytes
               && left.IoReadBps == right.IoReadBps
               && left.IoWriteBps == right.IoWriteBps
               && left.NetBps == right.NetBps
               && left.Threads == right.Threads
               && left.Handles == right.Handles
               && left.AccessState == right.AccessState
               && string.Equals(left.Name, right.Name, StringComparison.Ordinal);
    }

    private static bool CpuEquivalent(double left, double right)
    {
        return Math.Abs(left - right) < 0.0001;
    }
}
