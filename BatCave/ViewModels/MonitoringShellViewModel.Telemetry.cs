using System;
using System.Collections.Generic;
using System.Linq;
using BatCave.Charts;
using BatCave.Core.Domain;

namespace BatCave.ViewModels;

public partial class MonitoringShellViewModel
{
    private void OnTelemetryDelta(object? sender, ProcessDeltaBatch delta)
    {
        RunOnUiThread(() =>
        {
            ApplyUpserts(delta.Upserts);
            ApplyExits(delta.Exits);
            FinalizeDeltaRefresh(delta);
        });
    }

    private void ApplyUpserts(IReadOnlyList<ProcessSample> upserts)
    {
        foreach (ProcessSample upsert in upserts)
        {
            ProcessIdentity identity = upsert.Identity();
            if (_allRows.TryGetValue(identity, out ProcessSample? previous))
            {
                ApplySummaryDelta(previous, -1d);
            }

            _allRows[identity] = upsert;
            ApplySummaryDelta(upsert, 1d);
            _summarySeq = Math.Max(_summarySeq, upsert.Seq);
            _summaryTsMs = Math.Max(_summaryTsMs, upsert.TsMs);

            if (!_metricHistory.TryGetValue(identity, out MetricHistoryBuffer? history))
            {
                history = new MetricHistoryBuffer(HistoryLimit);
                _metricHistory[identity] = history;
            }

            history.Append(upsert);

            ProcessRowViewState rowState = GetOrCreateVisibleRowState(upsert);
            if (ShouldReplaceVisibleRow(rowState.Sample, upsert))
            {
                rowState.UpdateSample(upsert);
            }

            rowState.UpdateCpuTrendPoints(BuildRowCpuTrendPoints(identity, upsert));
        }
    }

    private void ApplyExits(IReadOnlyList<ProcessIdentity> exits)
    {
        foreach (ProcessIdentity exit in exits)
        {
            if (_allRows.Remove(exit, out ProcessSample? previous))
            {
                ApplySummaryDelta(previous, -1d);
            }

            _metadataCache.Remove(exit);
            _metricHistory.Remove(exit);
            if (_visibleRowStateByIdentity.Remove(exit, out ProcessRowViewState? rowState))
            {
                _rowViewSource.Remove(rowState);
            }
        }
    }

    private void FinalizeDeltaRefresh(ProcessDeltaBatch delta)
    {
        _summarySeq = Math.Max(_summarySeq, delta.Seq);
        if (delta.Upserts.Count == 0)
        {
            _summaryTsMs = UnixNowMs();
        }

        ClampSummary();
        UpdateGlobalSummaryHistory();
        RefreshVisibleRows();
        ReconcileSelectionAfterDelta();
    }

    private void LoadSnapshot(IReadOnlyList<ProcessSample> rows)
    {
        _allRows.Clear();
        _metricHistory.Clear();
        _visibleRowStateByIdentity.Clear();
        _rowViewSource.Clear();

        foreach (ProcessSample row in rows)
        {
            ProcessIdentity identity = row.Identity();
            _allRows[identity] = row;

            MetricHistoryBuffer history = new(HistoryLimit);
            history.Append(row);
            _metricHistory[identity] = history;

            ProcessRowViewState rowState = new(row, BuildRowCpuTrendPoints(identity, row));
            _visibleRowStateByIdentity[identity] = rowState;
            _rowViewSource.Add(rowState);
        }

        PruneMetadataCache();

        ResetSummaryFromRows(_allRows.Values);
        _globalHistory.Reset();
        UpdateGlobalSummaryHistory();

        RefreshVisibleRows();
        ReconcileSelectionAfterDelta();
    }

    private void PruneMetadataCache()
    {
        HashSet<ProcessIdentity> validIdentities = _allRows.Keys.ToHashSet();
        foreach (ProcessIdentity cachedIdentity in _metadataCache.Keys.ToList())
        {
            if (!validIdentities.Contains(cachedIdentity))
            {
                _metadataCache.Remove(cachedIdentity);
            }
        }
    }

    private void RefreshVisibleRows()
    {
        VisibleRows.RefreshFilter();
        SelectedVisibleRow = SelectedRow is null ? null : TryGetVisibleRow(SelectedRow.Identity());
        RefreshDetailMetrics();
    }

    private ProcessRowViewState GetOrCreateVisibleRowState(ProcessSample sample)
    {
        ProcessIdentity identity = sample.Identity();
        if (_visibleRowStateByIdentity.TryGetValue(identity, out ProcessRowViewState? existing))
        {
            return existing;
        }

        ProcessRowViewState state = new(sample, BuildRowCpuTrendPoints(identity, sample));
        _visibleRowStateByIdentity[identity] = state;
        _rowViewSource.Add(state);
        return state;
    }

    private string BuildRowCpuTrendPoints(ProcessIdentity identity, ProcessSample sample)
    {
        IReadOnlyList<double> values = _metricHistory.TryGetValue(identity, out MetricHistoryBuffer? history)
            ? history.Cpu
            : MetricHistoryBuffer.Singleton(sample.CpuPct);

        return SparklineMath.BuildPointString(values, RowSparklineWidth, RowSparklineHeight);
    }

    private static bool ShouldReplaceVisibleRow(ProcessSample current, ProcessSample next)
    {
        return current.Name != next.Name
            || current.CpuPct != next.CpuPct
            || current.RssBytes != next.RssBytes
            || current.IoReadBps != next.IoReadBps
            || current.IoWriteBps != next.IoWriteBps
            || current.NetBps != next.NetBps
            || current.Threads != next.Threads
            || current.Handles != next.Handles
            || current.AccessState != next.AccessState;
    }
}
