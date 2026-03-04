using BatCave.ViewModels;

namespace BatCave.Tests.ViewModels;

public sealed class InteractionProbeRecorderTests
{
    [Fact]
    public void Record_WhenCapacityExceeded_OverwritesOldestSamplesPerProbeKind()
    {
        InteractionProbeRecorder recorder = new(sampleCapacity: 3);

        RecordSamples(recorder, InteractionProbeRecorder.ProbeType.FilterApply, [100, 1, 2, 3]);
        RecordSamples(recorder, InteractionProbeRecorder.ProbeType.SortComplete, [200, 4, 5, 6]);
        RecordSamples(recorder, InteractionProbeRecorder.ProbeType.SelectionSettle, [300, 7, 8, 9]);
        RecordSamples(recorder, InteractionProbeRecorder.ProbeType.UiBatch, [400, 10, 11, 12]);
        RecordSamples(recorder, InteractionProbeRecorder.ProbeType.PlotRefresh, [500, 13, 14, 15]);

        Assert.Equal(3, recorder.GetP95(InteractionProbeRecorder.ProbeType.FilterApply));
        Assert.Equal(6, recorder.GetP95(InteractionProbeRecorder.ProbeType.SortComplete));
        Assert.Equal(9, recorder.GetP95(InteractionProbeRecorder.ProbeType.SelectionSettle));
        Assert.Equal(12, recorder.GetP95(InteractionProbeRecorder.ProbeType.UiBatch));
        Assert.Equal(15, recorder.GetP95(InteractionProbeRecorder.ProbeType.PlotRefresh));
    }

    [Fact]
    public void SnapshotP95_ComputesIndependentProbeKindPercentiles()
    {
        InteractionProbeRecorder recorder = new(sampleCapacity: 20);

        RecordSamples(recorder, InteractionProbeRecorder.ProbeType.FilterApply, Enumerable.Range(1, 20).Select(static v => (double)v));
        RecordSamples(recorder, InteractionProbeRecorder.ProbeType.SortComplete, Enumerable.Range(1, 20).Select(static v => (double)(v * 10)));
        RecordSamples(recorder, InteractionProbeRecorder.ProbeType.SelectionSettle, [9, 1, 5, 7, 3, 11, 13, 15, 17, 19]);
        RecordSamples(recorder, InteractionProbeRecorder.ProbeType.UiBatch, [42]);
        RecordSamples(recorder, InteractionProbeRecorder.ProbeType.PlotRefresh, [8, 8, 8, 8]);

        InteractionProbeP95Snapshot snapshot = recorder.SnapshotP95();

        Assert.Equal(19, snapshot.FilterApplyMs);
        Assert.Equal(190, snapshot.SortCompleteMs);
        Assert.Equal(19, snapshot.SelectionSettleMs);
        Assert.Equal(42, snapshot.UiBatchMs);
        Assert.Equal(8, snapshot.PlotRefreshMs);
    }

    private static void RecordSamples(
        InteractionProbeRecorder recorder,
        InteractionProbeRecorder.ProbeType probeType,
        IEnumerable<double> samples)
    {
        foreach (double sample in samples)
        {
            recorder.Record(probeType, sample);
        }
    }
}
