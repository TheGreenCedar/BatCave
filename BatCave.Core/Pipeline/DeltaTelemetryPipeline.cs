using BatCave.Core.Abstractions;
using BatCave.Core.Domain;

namespace BatCave.Core.Pipeline;

public sealed class DeltaTelemetryPipeline : ITelemetryPipeline
{
    private const ulong HeartbeatIntervalTicks = 4;

    private readonly Dictionary<ProcessIdentity, ProcessSample> _previous = new();
    private readonly Dictionary<ProcessIdentity, ulong> _lastEmittedSeq = new();

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
        HashSet<ProcessIdentity> current = new(raw.Count);
        Dictionary<ProcessIdentity, ProcessSample> nextPrevious = new(raw.Count);
        Dictionary<ProcessIdentity, ulong> nextLastEmittedSeq = new(raw.Count);
        List<ProcessSample> upserts = new(raw.Count);

        foreach (ProcessSample sample in raw)
        {
            ProcessIdentity identity = sample.Identity();
            bool changed = !_previous.TryGetValue(identity, out ProcessSample? previous) || !EquivalentSample(previous, sample);
            ulong previousEmitSeq = _lastEmittedSeq.TryGetValue(identity, out ulong emitSeq) ? emitSeq : 0;
            bool dueForHeartbeat = seq >= previousEmitSeq && seq - previousEmitSeq >= HeartbeatIntervalTicks;

            if (changed || dueForHeartbeat)
            {
                upserts.Add(sample);
                nextLastEmittedSeq[identity] = seq;
            }
            else
            {
                nextLastEmittedSeq[identity] = previousEmitSeq;
            }

            current.Add(identity);
            nextPrevious[identity] = sample;
        }

        List<ProcessIdentity> exits = _previous.Keys
            .Where(identity => !current.Contains(identity))
            .ToList();

        _previous.Clear();
        foreach ((ProcessIdentity identity, ProcessSample sample) in nextPrevious)
        {
            _previous[identity] = sample;
        }

        _lastEmittedSeq.Clear();
        foreach ((ProcessIdentity identity, ulong emittedSeq) in nextLastEmittedSeq)
        {
            _lastEmittedSeq[identity] = emittedSeq;
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
