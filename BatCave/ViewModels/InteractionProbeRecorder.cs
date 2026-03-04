using System;

namespace BatCave.ViewModels;

public partial class MonitoringShellViewModel
{
    private readonly InteractionProbeRecorder _interactionProbeRecorder = new();

    partial void RecordInteractionProbeUnsmoothed(InteractionProbe probe, double sampleMs)
    {
        _interactionProbeRecorder.Record(ResolveProbeType(probe), sampleMs);
    }

    private static InteractionProbeRecorder.ProbeType ResolveProbeType(InteractionProbe probe)
    {
        return probe switch
        {
            InteractionProbe.FilterApply => InteractionProbeRecorder.ProbeType.FilterApply,
            InteractionProbe.SortComplete => InteractionProbeRecorder.ProbeType.SortComplete,
            InteractionProbe.SelectionSettle => InteractionProbeRecorder.ProbeType.SelectionSettle,
            InteractionProbe.UiBatch => InteractionProbeRecorder.ProbeType.UiBatch,
            InteractionProbe.PlotRefresh => InteractionProbeRecorder.ProbeType.PlotRefresh,
            _ => InteractionProbeRecorder.ProbeType.FilterApply,
        };
    }

    internal InteractionProbeP95Snapshot SnapshotInteractionProbeP95()
    {
        return _interactionProbeRecorder.SnapshotP95();
    }

    internal void ResetInteractionProbeRecorder()
    {
        _interactionProbeRecorder.Reset();
    }
}

internal sealed class InteractionProbeRecorder
{
    private const int DefaultSampleCapacity = 64;

    private readonly RingBuffer _filterApply;
    private readonly RingBuffer _sortComplete;
    private readonly RingBuffer _selectionSettle;
    private readonly RingBuffer _uiBatch;
    private readonly RingBuffer _plotRefresh;

    public InteractionProbeRecorder(int sampleCapacity = DefaultSampleCapacity)
    {
        int capacity = Math.Max(1, sampleCapacity);
        _filterApply = new RingBuffer(capacity);
        _sortComplete = new RingBuffer(capacity);
        _selectionSettle = new RingBuffer(capacity);
        _uiBatch = new RingBuffer(capacity);
        _plotRefresh = new RingBuffer(capacity);
    }

    internal enum ProbeType
    {
        FilterApply,
        SortComplete,
        SelectionSettle,
        UiBatch,
        PlotRefresh,
    }

    public void Record(ProbeType probeType, double sampleMs)
    {
        if (sampleMs <= 0 || double.IsNaN(sampleMs) || double.IsInfinity(sampleMs))
        {
            return;
        }

        ResolveBuffer(probeType).Add(sampleMs);
    }

    public double GetP95(ProbeType probeType)
    {
        return ResolveBuffer(probeType).GetP95();
    }

    public InteractionProbeP95Snapshot SnapshotP95()
    {
        return new InteractionProbeP95Snapshot(
            GetP95(ProbeType.FilterApply),
            GetP95(ProbeType.SortComplete),
            GetP95(ProbeType.SelectionSettle),
            GetP95(ProbeType.UiBatch),
            GetP95(ProbeType.PlotRefresh));
    }

    public void Reset()
    {
        _filterApply.Clear();
        _sortComplete.Clear();
        _selectionSettle.Clear();
        _uiBatch.Clear();
        _plotRefresh.Clear();
    }

    private RingBuffer ResolveBuffer(ProbeType probeType)
    {
        return probeType switch
        {
            ProbeType.FilterApply => _filterApply,
            ProbeType.SortComplete => _sortComplete,
            ProbeType.SelectionSettle => _selectionSettle,
            ProbeType.UiBatch => _uiBatch,
            ProbeType.PlotRefresh => _plotRefresh,
            _ => _filterApply,
        };
    }

    private sealed class RingBuffer
    {
        private readonly double[] _buffer;
        private double[] _scratch;
        private int _start;
        private int _count;

        public RingBuffer(int capacity)
        {
            _buffer = new double[capacity];
            _scratch = new double[capacity];
        }

        public void Add(double value)
        {
            if (_count < _buffer.Length)
            {
                _buffer[(_start + _count) % _buffer.Length] = value;
                _count++;
                return;
            }

            _buffer[_start] = value;
            _start = (_start + 1) % _buffer.Length;
        }

        public double GetP95()
        {
            if (_count == 0)
            {
                return 0;
            }

            if (_scratch.Length < _count)
            {
                _scratch = new double[_count];
            }

            for (int index = 0; index < _count; index++)
            {
                _scratch[index] = _buffer[(_start + index) % _buffer.Length];
            }

            Array.Sort(_scratch, 0, _count);
            int percentileIndex = Math.Min(
                _count - 1,
                Math.Max(0, (int)Math.Ceiling(_count * 0.95d) - 1));

            return _scratch[percentileIndex];
        }

        public void Clear()
        {
            _start = 0;
            _count = 0;
        }
    }
}

internal readonly record struct InteractionProbeP95Snapshot(
    double FilterApplyMs,
    double SortCompleteMs,
    double SelectionSettleMs,
    double UiBatchMs,
    double PlotRefreshMs);
