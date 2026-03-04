using BatCave.Charts;
using BatCave.Core.Domain;
using System;
using System.Collections.Generic;
using System.Diagnostics;
using Windows.Foundation;

namespace BatCave.ViewModels;

public partial class MonitoringShellViewModel
{
    private void OnTelemetryDelta(object? sender, ProcessDeltaBatch delta)
    {
        RunOnUiThread(() => ApplyTelemetryDelta(delta));
    }

    private void ApplyTelemetryDelta(ProcessDeltaBatch delta)
    {
        long startedAt = Stopwatch.GetTimestamp();
        bool refreshFilter = ApplyUpserts(delta.Upserts);
        refreshFilter |= ApplyExits(delta.Exits);
        FinalizeDeltaRefresh(delta, refreshFilter);
        RecordTimingProbe(
            InteractionProbe.UiBatch,
            Stopwatch.GetTimestamp() - startedAt);
    }

    private bool ApplyUpserts(IReadOnlyList<ProcessSample> upserts)
    {
        bool refreshFilter = false;
        List<ProcessIdentity>? changedProjectionRows = null;

        foreach (ProcessSample upsert in upserts)
        {
            ProcessIdentity identity = upsert.Identity();
            bool hadPrevious = _allRows.TryGetValue(identity, out ProcessSample? previous);
            TrackUpsert(identity, upsert, hadPrevious ? previous : null);

            ProcessRowViewState rowState = GetOrCreateVisibleRowState(upsert);
            bool projectionChanged = UpdateVisibleRowForUpsert(identity, rowState, upsert, hadPrevious ? previous : null);
            if (projectionChanged)
            {
                changedProjectionRows ??= [];
                changedProjectionRows.Add(identity);
            }
        }

        if (changedProjectionRows is { Count: > 0 })
        {
            _rowViewSource.Edit(updater =>
            {
                foreach (ProcessIdentity changedIdentity in changedProjectionRows)
                {
                    updater.Refresh(changedIdentity);
                }
            });
        }

        return refreshFilter;
    }

    private bool ApplyExits(IReadOnlyList<ProcessIdentity> exits)
    {
        bool refreshFilter = false;

        foreach (ProcessIdentity exit in exits)
        {
            refreshFilter |= RemoveTrackedIdentity(exit);
        }

        return refreshFilter;
    }

    private void FinalizeDeltaRefresh(ProcessDeltaBatch delta, bool refreshFilter)
    {
        _summarySeq = Math.Max(_summarySeq, delta.Seq);
        if (delta.Upserts.Count == 0)
        {
            _summaryTsMs = UnixNowMs();
        }

        ClampSummary();
        UpdateGlobalSummaryHistory();
        AppendHeartbeatSamplesIfNeeded(delta.Seq);
        RefreshVisibleRows(refreshFilter);
        ReconcileSelectionAfterDelta();
    }

    private void LoadSnapshot(IReadOnlyList<ProcessSample> rows)
    {
        _allRows.Clear();
        _metricHistory.Clear();
        _metricHistoryLastSeq.Clear();
        _visibleRowStateByIdentity.Clear();
        _rowViewSource.Edit(updater => updater.Clear());

        foreach (ProcessSample row in rows)
        {
            AddSnapshotRow(row);
        }

        PruneMetadataCache();

        ResetSummaryFromRows(_allRows.Values);
        _globalHistory.Reset();
        UpdateGlobalSummaryHistory();

        RefreshVisibleRows(refreshFilter: true);
        ReconcileSelectionAfterDelta();
    }

    private void AddSnapshotRow(ProcessSample row)
    {
        ProcessIdentity identity = row.Identity();
        _allRows[identity] = row;

        MetricHistoryBuffer history = new(HistoryLimit);
        history.Append(row);
        _metricHistory[identity] = history;
        _metricHistoryLastSeq[identity] = row.Seq;

        IReadOnlyList<double> cpuTrend = GetRowCpuTrendSource(identity, row);
        ProcessRowViewState rowState = new(row, BuildRowCpuTrendGeometry(cpuTrend));
        rowState.UpdateCpuTrendValues(cpuTrend, RowMiniTrendVisiblePointCount);
        _visibleRowStateByIdentity[identity] = rowState;
        _rowViewSource.Edit(updater => updater.AddOrUpdate(rowState));
    }

    private void PruneMetadataCache()
    {
        List<ProcessIdentity> staleIdentities = [];
        foreach (ProcessIdentity cachedIdentity in _metadataCache.Keys)
        {
            if (!_allRows.ContainsKey(cachedIdentity))
            {
                staleIdentities.Add(cachedIdentity);
            }
        }

        foreach (ProcessIdentity staleIdentity in staleIdentities)
        {
            RemoveIdentityStateFromCaches(staleIdentity);
        }
    }

    private void RefreshVisibleRows(bool refreshFilter)
    {
        if (refreshFilter)
        {
            ApplyCanonicalFilter();
        }

        SelectedVisibleRow = ResolveSelectedVisibleRow();
        RefreshDetailMetrics();
    }

    private ProcessRowViewState GetOrCreateVisibleRowState(ProcessSample sample)
    {
        ProcessIdentity identity = sample.Identity();
        if (_visibleRowStateByIdentity.TryGetValue(identity, out ProcessRowViewState? existing))
        {
            return existing;
        }

        IReadOnlyList<double> cpuTrend = GetRowCpuTrendSource(identity, sample);
        ProcessRowViewState state = new(sample, BuildRowCpuTrendGeometry(cpuTrend));
        state.UpdateCpuTrendValues(cpuTrend, RowMiniTrendVisiblePointCount);
        _visibleRowStateByIdentity[identity] = state;
        _rowViewSource.Edit(updater => updater.AddOrUpdate(state));
        return state;
    }

    private IReadOnlyList<double> GetRowCpuTrendSource(ProcessIdentity identity, ProcessSample sample)
    {
        if (_metricHistory.TryGetValue(identity, out MetricHistoryBuffer? history))
        {
            return history.Cpu;
        }

        MetricHistoryBuffer fallback = new(HistoryLimit);
        fallback.Append(sample);
        return fallback.Cpu;
    }

    private static IReadOnlyList<Point> BuildRowCpuTrendGeometry(IReadOnlyList<double> values)
    {
        return SparklineMath.BuildPointsWithFallback(values, RowSparklineWidth, RowSparklineHeight);
    }

    private static bool ShouldReplaceVisibleRow(ProcessSample current, ProcessSample next)
    {
        return current.Name != next.Name
            || current.CpuPct != next.CpuPct
            || current.RssBytes != next.RssBytes
            || current.IoReadBps != next.IoReadBps
            || current.IoWriteBps != next.IoWriteBps
            || current.OtherIoBps != next.OtherIoBps
            || current.Threads != next.Threads
            || current.Handles != next.Handles
            || current.AccessState != next.AccessState;
    }

    private static bool ShouldRefreshRowSparkline(ProcessSample? previous, ProcessSample current)
    {
        if (previous is null)
        {
            return true;
        }

        return ProcessRowViewState.IsCpuSortBucketChanged(previous.CpuPct, current.CpuPct)
               || current.Seq % RowSparklineStride == 0;
    }

    private void AppendHeartbeatSamplesIfNeeded(ulong seq)
    {
        AppendSelectedHeartbeatSample(seq);
        if (IsTableHeartbeatDue(seq))
        {
            AppendTableHeartbeatSamples(seq);
        }
    }

    private void AppendSelectedHeartbeatSample(ulong seq)
    {
        if (SelectedRow is null)
        {
            return;
        }

        ProcessIdentity selectedIdentity = SelectedRow.Identity();
        if (_allRows.TryGetValue(selectedIdentity, out ProcessSample? selectedSample) && selectedSample is not null)
        {
            _ = AppendHeartbeatForIdentity(selectedIdentity, selectedSample, seq);
        }
    }

    private static bool IsTableHeartbeatDue(ulong seq)
    {
        return seq % RowSparklineStride == 0;
    }

    private void AppendTableHeartbeatSamples(ulong seq)
    {
        int visibleCount = VisibleRows.Count;
        if (visibleCount <= 0)
        {
            return;
        }

        int rowsToRefresh = Math.Min(visibleCount, MaxHeartbeatSparklineRowsPerFrame);
        int startIndex = _heartbeatVisibleRowCursor % visibleCount;
        _heartbeatVisibleRowCursor = (startIndex + rowsToRefresh) % Math.Max(1, visibleCount);

        for (int offset = 0; offset < rowsToRefresh; offset++)
        {
            int rowIndex = (startIndex + offset) % visibleCount;
            if (VisibleRows[rowIndex] is not ProcessRowViewState rowState)
            {
                continue;
            }

            ProcessIdentity identity = rowState.Identity;
            if (!_allRows.TryGetValue(identity, out ProcessSample? sample) || sample is null)
            {
                continue;
            }

            if (!ShouldShowSample(sample))
            {
                continue;
            }

            if (AppendHeartbeatForIdentity(identity, sample, seq))
            {
                IReadOnlyList<double> cpuTrend = GetRowCpuTrendSource(identity, sample);
                rowState.UpdateCpuTrendGeometry(BuildRowCpuTrendGeometry(cpuTrend));
                rowState.UpdateCpuTrendValues(cpuTrend, RowMiniTrendVisiblePointCount);
            }
        }
    }

    private bool AppendHeartbeatForIdentity(ProcessIdentity identity, ProcessSample sample, ulong seq)
    {
        ulong lastSeq = _metricHistoryLastSeq.TryGetValue(identity, out ulong value) ? value : 0;
        if (lastSeq >= seq)
        {
            return false;
        }

        if (!_metricHistory.TryGetValue(identity, out MetricHistoryBuffer? history))
        {
            history = new MetricHistoryBuffer(HistoryLimit);
            _metricHistory[identity] = history;
        }

        ProcessSample heartbeat = sample with
        {
            Seq = seq,
            TsMs = _summaryTsMs,
        };

        history.Append(heartbeat);
        _metricHistoryLastSeq[identity] = seq;
        return true;
    }

    private void TrackUpsert(ProcessIdentity identity, ProcessSample upsert, ProcessSample? previous)
    {
        if (previous is not null)
        {
            ApplySummaryDelta(previous, -1d);
        }

        _allRows[identity] = upsert;
        ApplySummaryDelta(upsert, 1d);
        _summarySeq = Math.Max(_summarySeq, upsert.Seq);
        _summaryTsMs = Math.Max(_summaryTsMs, upsert.TsMs);

        MetricHistoryBuffer history = GetOrCreateMetricHistory(identity);
        history.Append(upsert);
        _metricHistoryLastSeq[identity] = upsert.Seq;
    }

    private MetricHistoryBuffer GetOrCreateMetricHistory(ProcessIdentity identity)
    {
        if (_metricHistory.TryGetValue(identity, out MetricHistoryBuffer? history))
        {
            return history;
        }

        MetricHistoryBuffer created = new(HistoryLimit);
        _metricHistory[identity] = created;
        return created;
    }

    private bool UpdateVisibleRowForUpsert(
        ProcessIdentity identity,
        ProcessRowViewState rowState,
        ProcessSample upsert,
        ProcessSample? previous)
    {
        bool projectionChanged = false;
        if (ShouldReplaceVisibleRow(rowState.Sample, upsert))
        {
            rowState.UpdateSample(upsert);
            projectionChanged = true;
        }

        if (ShouldRefreshRowSparkline(previous, upsert) && ShouldShowSample(upsert))
        {
            IReadOnlyList<double> cpuTrend = GetRowCpuTrendSource(identity, upsert);
            rowState.UpdateCpuTrendGeometry(BuildRowCpuTrendGeometry(cpuTrend));
            rowState.UpdateCpuTrendValues(cpuTrend, RowMiniTrendVisiblePointCount);
        }

        return projectionChanged;
    }

    private bool RemoveTrackedIdentity(ProcessIdentity identity)
    {
        if (_allRows.Remove(identity, out ProcessSample? previous))
        {
            ApplySummaryDelta(previous, -1d);
        }

        RemoveIdentityStateFromCaches(identity);
        return RemoveVisibleRowState(identity);
    }

    private void RemoveIdentityStateFromCaches(ProcessIdentity identity)
    {
        _metadataCache.Remove(identity);
        _metricHistory.Remove(identity);
        _metricHistoryLastSeq.Remove(identity);
    }

    private bool RemoveVisibleRowState(ProcessIdentity identity)
    {
        if (_visibleRowStateByIdentity.Remove(identity, out _))
        {
            _rowViewSource.Edit(updater => updater.RemoveKey(identity));
            return true;
        }

        return false;
    }

    private ProcessRowViewState? ResolveSelectedVisibleRow()
    {
        return SelectedRow is null ? null : TryGetVisibleRow(SelectedRow.Identity());
    }
}
