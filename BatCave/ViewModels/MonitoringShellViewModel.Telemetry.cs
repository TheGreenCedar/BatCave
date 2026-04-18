using BatCave.Core.Domain;
using System;
using System.Collections.Generic;
using System.Diagnostics;
using System.Threading;

namespace BatCave.ViewModels;

public partial class MonitoringShellViewModel
{
    private void OnTelemetryDelta(object? sender, ProcessDeltaBatch delta)
    {
        if (_disposed)
        {
            return;
        }

        QueuePendingTelemetryDelta(delta);
    }

    private void ApplyTelemetryDelta(ProcessDeltaBatch delta)
    {
        if (_disposed)
        {
            return;
        }

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
        if (_disposed)
        {
            return false;
        }

        bool refreshFilter = false;
        List<ProcessIdentity>? changedProjectionRows = null;
        string filterNeedle = FilterText.Trim();
        SortColumn currentSortColumn = CurrentSortColumn;
        bool adminModeEnabled = AdminModeEnabled;
        bool adminEnabledOnlyFilter = AdminEnabledOnlyFilter;

        foreach (ProcessSample upsert in upserts)
        {
            ProcessIdentity identity = upsert.Identity();
            bool hadPrevious = _allRows.TryGetValue(identity, out ProcessSample? previous);
            TrackUpsert(identity, upsert, hadPrevious ? previous : null);

            ProcessRowViewState rowState = GetOrCreateVisibleRowState(upsert);
            bool projectionChanged = UpdateVisibleRowForUpsert(
                rowState,
                upsert,
                currentSortColumn,
                filterNeedle,
                adminModeEnabled,
                adminEnabledOnlyFilter);
            if (projectionChanged)
            {
                changedProjectionRows ??= [];
                changedProjectionRows.Add(identity);
            }

            refreshFilter |= DidVisibilityMembershipChange(
                hadPrevious ? previous : null,
                upsert,
                filterNeedle,
                adminModeEnabled,
                adminEnabledOnlyFilter);
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

    private static bool DidVisibilityMembershipChange(
        ProcessSample? previous,
        ProcessSample current,
        string filterNeedle,
        bool adminModeEnabled,
        bool adminEnabledOnlyFilter)
    {
        if (previous is null)
        {
            return false;
        }

        return IsVisibleForCurrentRowShaping(previous, filterNeedle, adminModeEnabled, adminEnabledOnlyFilter)
               != IsVisibleForCurrentRowShaping(current, filterNeedle, adminModeEnabled, adminEnabledOnlyFilter);
    }

    private bool ApplyExits(IReadOnlyList<ProcessIdentity> exits)
    {
        if (_disposed)
        {
            return false;
        }

        bool refreshFilter = false;

        foreach (ProcessIdentity exit in exits)
        {
            refreshFilter |= RemoveTrackedIdentity(exit);
        }

        return refreshFilter;
    }

    private void FinalizeDeltaRefresh(ProcessDeltaBatch delta, bool refreshFilter)
    {
        if (_disposed)
        {
            return;
        }

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
        RefreshInspectorAfterTelemetryDelta();
    }

    private void QueuePendingTelemetryDelta(ProcessDeltaBatch delta)
    {
        lock (_pendingUiEventSync)
        {
            MergePendingTelemetry(delta);
        }

        QueuePendingUiEventDrain();
    }

    private void QueuePendingRuntimeHealth(RuntimeHealth health)
    {
        lock (_pendingUiEventSync)
        {
            _pendingRuntimeHealth = health;
        }

        QueuePendingUiEventDrain();
    }

    private void QueuePendingCollectorWarning(CollectorWarning warning)
    {
        lock (_pendingUiEventSync)
        {
            _pendingCollectorWarnings.Add(warning);
        }

        QueuePendingUiEventDrain();
    }

    private void QueuePendingUiEventDrain()
    {
        if (Interlocked.Exchange(ref _pendingUiEventDrainQueued, 1) == 1)
        {
            return;
        }

        RunDispatcherHandlerOnUiThread(DrainPendingUiEvents);
    }

    private void DrainPendingUiEvents()
    {
        while (true)
        {
            ProcessDeltaBatch? delta;
            RuntimeHealth? runtimeHealth;
            CollectorWarning[]? warnings;

            lock (_pendingUiEventSync)
            {
                if (!_hasPendingTelemetry
                    && _pendingRuntimeHealth is null
                    && _pendingCollectorWarnings.Count == 0)
                {
                    Interlocked.Exchange(ref _pendingUiEventDrainQueued, 0);
                    return;
                }

                delta = _hasPendingTelemetry ? FlushPendingTelemetry() : null;
                runtimeHealth = _pendingRuntimeHealth;
                _pendingRuntimeHealth = null;
                warnings = _pendingCollectorWarnings.Count > 0
                    ? [.. _pendingCollectorWarnings]
                    : null;
                _pendingCollectorWarnings.Clear();
            }

            if (delta is not null)
            {
                ApplyTelemetryDelta(delta);
            }

            if (runtimeHealth is not null)
            {
                ApplyRuntimeHealth(runtimeHealth);
            }

            if (warnings is null)
            {
                continue;
            }

            foreach (CollectorWarning warning in warnings)
            {
                ApplyCollectorWarning(warning);
            }
        }
    }

    private void MergePendingTelemetry(ProcessDeltaBatch delta)
    {
        _hasPendingTelemetry = true;

        if (delta.Seq > _pendingTelemetrySeq)
        {
            _pendingTelemetrySeq = delta.Seq;
        }

        foreach (ProcessSample sample in delta.Upserts)
        {
            ProcessIdentity identity = sample.Identity();
            _pendingTelemetryExits.Remove(identity);
            _pendingTelemetryUpserts[identity] = sample;
        }

        foreach (ProcessIdentity identity in delta.Exits)
        {
            _pendingTelemetryUpserts.Remove(identity);
            _pendingTelemetryExits.Add(identity);
        }
    }

    private ProcessDeltaBatch FlushPendingTelemetry()
    {
        ProcessDeltaBatch delta = new()
        {
            Seq = _pendingTelemetrySeq,
            Upserts = [.. _pendingTelemetryUpserts.Values],
            Exits = [.. _pendingTelemetryExits],
        };

        _pendingTelemetrySeq = 0;
        _pendingTelemetryUpserts.Clear();
        _pendingTelemetryExits.Clear();
        _hasPendingTelemetry = false;
        return delta;
    }

    private void LoadSnapshot(IReadOnlyList<ProcessSample> rows)
    {
        if (_disposed)
        {
            return;
        }

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
        if (_disposed)
        {
            return;
        }

        ProcessIdentity identity = row.Identity();
        _allRows[identity] = row;

        MetricHistoryBuffer history = new(HistoryLimit);
        history.Append(row);
        _metricHistory[identity] = history;
        _metricHistoryLastSeq[identity] = row.Seq;

        ProcessRowViewState rowState = new(row, []);
        _visibleRowStateByIdentity[identity] = rowState;
        _rowViewSource.Edit(updater => updater.AddOrUpdate(rowState));
    }

    private void PruneMetadataCache()
    {
        if (_disposed)
        {
            return;
        }

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
        if (_disposed)
        {
            return;
        }

        if (refreshFilter)
        {
            ApplyCanonicalFilter();
        }

        SelectedVisibleRow = ResolveSelectedVisibleRow();
        RaiseCompactTotalsProperties();
    }

    private void RefreshInspectorAfterTelemetryDelta()
    {
        RefreshDetailMetrics();

        if (SelectedRow is null)
        {
            return;
        }

        BuildAndAppendResourceRows(BuildGlobalResourceDescriptors(_latestGlobalMetricsSample));
        RefreshGlobalDetailState();
    }

    private ProcessRowViewState GetOrCreateVisibleRowState(ProcessSample sample)
    {
        if (_disposed)
        {
            return new ProcessRowViewState(sample, []);
        }

        ProcessIdentity identity = sample.Identity();
        if (_visibleRowStateByIdentity.TryGetValue(identity, out ProcessRowViewState? existing))
        {
            return existing;
        }

        ProcessRowViewState state = new(sample, []);
        _visibleRowStateByIdentity[identity] = state;
        _rowViewSource.Edit(updater => updater.AddOrUpdate(state));
        return state;
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

    private void AppendHeartbeatSamplesIfNeeded(ulong seq)
    {
        AppendSelectedHeartbeatSample(seq);
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

    private bool AppendHeartbeatForIdentity(ProcessIdentity identity, ProcessSample sample, ulong seq)
    {
        if (_disposed)
        {
            return false;
        }

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
        if (_disposed)
        {
            return;
        }

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

    private static bool UpdateVisibleRowForUpsert(
        ProcessRowViewState rowState,
        ProcessSample upsert,
        SortColumn currentSortColumn,
        string filterNeedle,
        bool adminModeEnabled,
        bool adminEnabledOnlyFilter)
    {
        ProcessSample previous = rowState.Sample;
        bool displayProjectionChanged = false;
        if (ShouldReplaceVisibleRow(previous, upsert))
        {
            rowState.UpdateSample(upsert);
            displayProjectionChanged = true;
        }

        if (!displayProjectionChanged)
        {
            return false;
        }

        return ShouldRefreshVisibleRowShape(
            previous,
            upsert,
            currentSortColumn,
            filterNeedle,
            adminModeEnabled,
            adminEnabledOnlyFilter);
    }

    private static bool ShouldRefreshVisibleRowShape(
        ProcessSample previous,
        ProcessSample current,
        SortColumn currentSortColumn,
        string filterNeedle,
        bool adminModeEnabled,
        bool adminEnabledOnlyFilter)
    {
        bool wasVisible = IsVisibleForCurrentRowShaping(previous, filterNeedle, adminModeEnabled, adminEnabledOnlyFilter);
        bool isVisible = IsVisibleForCurrentRowShaping(current, filterNeedle, adminModeEnabled, adminEnabledOnlyFilter);

        if (wasVisible != isVisible)
        {
            return true;
        }

        if (!wasVisible)
        {
            return false;
        }

        return IsCurrentSortKeyChanged(previous, current, currentSortColumn);
    }

    private static bool IsVisibleForCurrentRowShaping(
        ProcessSample sample,
        string filterNeedle,
        bool adminModeEnabled,
        bool adminEnabledOnlyFilter)
    {
        if (IsFilteredOutByAdminVisibility(sample, adminModeEnabled, adminEnabledOnlyFilter))
        {
            return false;
        }

        if (string.IsNullOrWhiteSpace(filterNeedle))
        {
            return true;
        }

        return sample.Name.Contains(filterNeedle, StringComparison.OrdinalIgnoreCase)
            || sample.Pid.ToString().Contains(filterNeedle, StringComparison.OrdinalIgnoreCase);
    }

    private static bool IsFilteredOutByAdminVisibility(
        ProcessSample sample,
        bool adminModeEnabled,
        bool adminEnabledOnlyFilter)
    {
        if (!adminModeEnabled && sample.AccessState == AccessState.Denied)
        {
            return true;
        }

        return adminEnabledOnlyFilter && sample.AccessState != AccessState.Full;
    }

    private static bool IsCurrentSortKeyChanged(
        ProcessSample previous,
        ProcessSample current,
        SortColumn currentSortColumn)
    {
        return currentSortColumn switch
        {
            SortColumn.Name => !string.Equals(previous.Name, current.Name, StringComparison.Ordinal),
            SortColumn.CpuPct => ProcessRowViewState.IsCpuSortBucketChanged(previous.CpuPct, current.CpuPct),
            SortColumn.RssBytes => previous.RssBytes != current.RssBytes,
            SortColumn.IoReadBps => previous.IoReadBps != current.IoReadBps,
            SortColumn.IoWriteBps => previous.IoWriteBps != current.IoWriteBps,
            SortColumn.OtherIoBps => previous.OtherIoBps != current.OtherIoBps,
            SortColumn.DiskBps => previous.IoReadBps != current.IoReadBps || previous.IoWriteBps != current.IoWriteBps,
            SortColumn.Threads => previous.Threads != current.Threads,
            SortColumn.Handles => previous.Handles != current.Handles,
            _ => false,
        };
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
        if (_disposed)
        {
            return;
        }

        _metadataCache.Remove(identity);
        _metricHistory.Remove(identity);
        _metricHistoryLastSeq.Remove(identity);
    }

    private bool RemoveVisibleRowState(ProcessIdentity identity)
    {
        if (_disposed)
        {
            return false;
        }

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
