using BatCave.Core.Domain;
using System;
using System.Collections.Generic;
using System.Linq;

namespace BatCave.Rendering;

public sealed class TelemetryDeltaAccumulator
{
    private readonly object _gate = new();
    private readonly Dictionary<ProcessIdentity, ProcessSample> _pendingUpserts = new();
    private readonly HashSet<ProcessIdentity> _pendingExits = [];
    private ulong _pendingSeq;
    private int _pendingBatchCount;
    private int _peakBatchDepth;

    public int PendingBatchCount
    {
        get
        {
            lock (_gate)
            {
                return _pendingBatchCount;
            }
        }
    }

    public int Enqueue(ProcessDeltaBatch delta)
    {
        lock (_gate)
        {
            MergeDeltaNoLock(delta);
            _pendingBatchCount++;
            _peakBatchDepth = Math.Max(_peakBatchDepth, _pendingBatchCount);

            return _pendingBatchCount;
        }
    }

    public bool TryDrain(out ProcessDeltaBatch mergedDelta, out int queueDepth)
    {
        lock (_gate)
        {
            if (_pendingBatchCount == 0)
            {
                mergedDelta = new ProcessDeltaBatch();
                queueDepth = 0;
                return false;
            }

            queueDepth = _peakBatchDepth;
            mergedDelta = new ProcessDeltaBatch
            {
                Seq = _pendingSeq,
                Upserts = _pendingUpserts.Values.ToList(),
                Exits = _pendingExits.ToList(),
            };

            ResetNoLock();
            return true;
        }
    }

    private void MergeDeltaNoLock(ProcessDeltaBatch delta)
    {
        if (delta.Seq > _pendingSeq)
        {
            _pendingSeq = delta.Seq;
        }

        ApplyUpsertsNoLock(delta.Upserts);
        ApplyExitsNoLock(delta.Exits);
    }

    private void ApplyUpsertsNoLock(IReadOnlyList<ProcessSample> upserts)
    {
        foreach (ProcessSample sample in upserts)
        {
            ProcessIdentity identity = sample.Identity();
            _pendingExits.Remove(identity);
            _pendingUpserts[identity] = sample;
        }
    }

    private void ApplyExitsNoLock(IReadOnlyList<ProcessIdentity> exits)
    {
        foreach (ProcessIdentity identity in exits)
        {
            _pendingUpserts.Remove(identity);
            _pendingExits.Add(identity);
        }
    }

    private void ResetNoLock()
    {
        _pendingUpserts.Clear();
        _pendingExits.Clear();
        _pendingSeq = 0;
        _pendingBatchCount = 0;
        _peakBatchDepth = 0;
    }
}
