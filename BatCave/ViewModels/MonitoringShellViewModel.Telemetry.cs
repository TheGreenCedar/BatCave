using System;
using System.Collections.Generic;
using BatCave.Charts;
using BatCave.Core.Domain;

namespace BatCave.ViewModels;

public partial class MonitoringShellViewModel
{
    private void OnTelemetryDelta(object? sender, ProcessDeltaBatch delta)
    {
        RunOnUiThread(() =>
        {
            bool refreshFilter = ApplyUpserts(delta.Upserts);
            refreshFilter |= ApplyExits(delta.Exits);
            FinalizeDeltaRefresh(delta, refreshFilter);
        });
    }

    private bool ApplyUpserts(IReadOnlyList<ProcessSample> upserts)
    {
        bool refreshFilter = false;

        foreach (ProcessSample upsert in upserts)
        {
            ProcessIdentity identity = upsert.Identity();
            bool hadPrevious = _allRows.TryGetValue(identity, out ProcessSample? previous);
            if (hadPrevious && previous is not null)
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
            _metricHistoryLastSeq[identity] = upsert.Seq;

            ProcessRowViewState rowState = GetOrCreateVisibleRowState(upsert, out bool created);
            ProcessSample priorSample = rowState.Sample;
            bool shouldRefreshForVisibility = created;
            if (ShouldReplaceVisibleRow(rowState.Sample, upsert))
            {
                rowState.UpdateSample(upsert);
                shouldRefreshForVisibility |= RequiresVisibleRefresh(priorSample, upsert);
            }

            if (ShouldRefreshRowSparkline(hadPrevious ? previous : null, upsert))
            {
                rowState.UpdateCpuTrendPoints(BuildRowCpuTrendPoints(identity, upsert));
            }

            refreshFilter |= shouldRefreshForVisibility;
        }

        return refreshFilter;
    }

    private bool ApplyExits(IReadOnlyList<ProcessIdentity> exits)
    {
        bool refreshFilter = false;

        foreach (ProcessIdentity exit in exits)
        {
            if (_allRows.Remove(exit, out ProcessSample? previous))
            {
                ApplySummaryDelta(previous, -1d);
            }

            _metadataCache.Remove(exit);
            _metricHistory.Remove(exit);
            _metricHistoryLastSeq.Remove(exit);
            if (_visibleRowStateByIdentity.Remove(exit, out ProcessRowViewState? rowState))
            {
                _rowViewSource.Remove(rowState);
                refreshFilter = true;
            }
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
        _rowViewSource.Clear();

        foreach (ProcessSample row in rows)
        {
            ProcessIdentity identity = row.Identity();
            _allRows[identity] = row;

            MetricHistoryBuffer history = new(HistoryLimit);
            history.Append(row);
            _metricHistory[identity] = history;
            _metricHistoryLastSeq[identity] = row.Seq;

            ProcessRowViewState rowState = new(row, BuildRowCpuTrendPoints(identity, row));
            _visibleRowStateByIdentity[identity] = rowState;
            _rowViewSource.Add(rowState);
        }

        PruneMetadataCache();

        ResetSummaryFromRows(_allRows.Values);
        _globalHistory.Reset();
        UpdateGlobalSummaryHistory();

        RefreshVisibleRows(refreshFilter: true);
        ReconcileSelectionAfterDelta();
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
            _metadataCache.Remove(staleIdentity);
        }
    }

    private void RefreshVisibleRows(bool refreshFilter)
    {
        if (refreshFilter)
        {
            VisibleRows.RefreshFilter();
        }

        SelectedVisibleRow = SelectedRow is null ? null : TryGetVisibleRow(SelectedRow.Identity());
        RefreshDetailMetrics();
    }

    private ProcessRowViewState GetOrCreateVisibleRowState(ProcessSample sample, out bool created)
    {
        ProcessIdentity identity = sample.Identity();
        if (_visibleRowStateByIdentity.TryGetValue(identity, out ProcessRowViewState? existing))
        {
            created = false;
            return existing;
        }

        ProcessRowViewState state = new(sample, BuildRowCpuTrendPoints(identity, sample));
        _visibleRowStateByIdentity[identity] = state;
        _rowViewSource.Add(state);
        created = true;
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

    private bool RequiresVisibleRefresh(ProcessSample previous, ProcessSample current)
    {
        bool wasVisible = ShouldShowSample(previous);
        bool isVisible = ShouldShowSample(current);
        if (wasVisible != isVisible)
        {
            return true;
        }

        return IsSortKeyChanged(previous, current);
    }

    private bool IsSortKeyChanged(ProcessSample previous, ProcessSample current)
    {
        return CurrentSortColumn switch
        {
            SortColumn.Pid => previous.Pid != current.Pid,
            SortColumn.Name => !string.Equals(previous.Name, current.Name, StringComparison.Ordinal),
            SortColumn.CpuPct => ProcessRowViewState.IsCpuSortBucketChanged(previous.CpuPct, current.CpuPct),
            SortColumn.RssBytes => previous.RssBytes != current.RssBytes,
            SortColumn.IoReadBps => previous.IoReadBps != current.IoReadBps,
            SortColumn.IoWriteBps => previous.IoWriteBps != current.IoWriteBps,
            SortColumn.NetBps => previous.NetBps != current.NetBps,
            SortColumn.Threads => previous.Threads != current.Threads,
            SortColumn.Handles => previous.Handles != current.Handles,
            SortColumn.StartTimeMs => previous.StartTimeMs != current.StartTimeMs,
            _ => false,
        };
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
        if (SelectedRow is null)
        {
            AppendTableHeartbeatSamplesIfDue(seq);
            return;
        }

        ProcessIdentity selectedIdentity = SelectedRow.Identity();
        if (_allRows.TryGetValue(selectedIdentity, out ProcessSample? selectedSample) && selectedSample is not null)
        {
            _ = AppendHeartbeatForIdentity(selectedIdentity, selectedSample, seq);
        }

        AppendTableHeartbeatSamplesIfDue(seq);
    }

    private void AppendTableHeartbeatSamplesIfDue(ulong seq)
    {
        if (seq % RowSparklineStride != 0)
        {
            return;
        }

        foreach ((ProcessIdentity identity, ProcessRowViewState rowState) in _visibleRowStateByIdentity)
        {
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
                rowState.UpdateCpuTrendPoints(BuildRowCpuTrendPoints(identity, sample));
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
}
