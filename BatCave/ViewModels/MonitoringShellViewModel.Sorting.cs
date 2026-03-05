using BatCave.Core.Domain;
using CommunityToolkit.Mvvm.Input;
using System;
using System.Collections.Generic;
using System.Diagnostics;
using System.Threading;
using System.Threading.Tasks;

namespace BatCave.ViewModels;

public partial class MonitoringShellViewModel
{
    private const double InteractionProbeSmoothingFactor = 0.35;

    private double _filterApplyProbeMs;
    private double _sortCompleteProbeMs;
    private double _selectionSettleProbeMs;
    private double _uiBatchProbeMs;
    private double _plotRefreshProbeMs;
    private string _interactionTimingProbe = BuildInteractionTimingProbeText(0, 0, 0, 0, 0);

    public string InteractionTimingProbe
    {
        get => _interactionTimingProbe;
        private set => SetProperty(ref _interactionTimingProbe, value);
    }

    public void ChangeSort(SortColumn column)
    {
        long startedAt = Stopwatch.GetTimestamp();
        CurrentSortColumn = column;
        CurrentSortDirection = ResolveNextSortDirection(column);

        _runtime.SetSort(CurrentSortColumn, CurrentSortDirection);
        ApplyCanonicalSort();
        ReassertSelectionAfterSort();

        RecordTimingProbe(InteractionProbe.SortComplete, Stopwatch.GetTimestamp() - startedAt);
    }

    [RelayCommand]
    private void SortHeader(string? sortTag)
    {
        if (!Enum.TryParse(sortTag, out SortColumn column))
        {
            return;
        }

        ChangeSort(column);
    }

    private bool ShouldShowRow(object item)
    {
        if (item is not ProcessRowViewState row)
        {
            return false;
        }

        return ShouldShowSample(row.Sample);
    }

    private Func<ProcessRowViewState, bool> BuildVisibilityFilter()
    {
        string needle = FilterText.Trim();
        bool hasFilter = !string.IsNullOrWhiteSpace(needle);

        return row =>
        {
            if (IsFilteredByAdminVisibility(row.Sample))
            {
                return false;
            }

            if (!hasFilter)
            {
                return true;
            }

            ProcessSample sample = row.Sample;
            return sample.Name.Contains(needle, StringComparison.OrdinalIgnoreCase)
                   || sample.Pid.ToString().Contains(needle, StringComparison.OrdinalIgnoreCase);
        };
    }

    private IComparer<ProcessRowViewState> BuildSortComparer(SortColumn column, SortDirection direction)
    {
        return Comparer<ProcessRowViewState>.Create((left, right) =>
        {
            int primary = CompareRowsByColumn(left, right, column);
            if (primary != 0)
            {
                return direction == SortDirection.Asc ? primary : -primary;
            }

            int pidOrder = left.Pid.CompareTo(right.Pid);
            if (pidOrder != 0)
            {
                return pidOrder;
            }

            return left.StartTimeMs.CompareTo(right.StartTimeMs);
        });
    }

    private static int CompareRowsByColumn(ProcessRowViewState left, ProcessRowViewState right, SortColumn column)
    {
        return column switch
        {
            SortColumn.Pid => left.Pid.CompareTo(right.Pid),
            SortColumn.Name => string.Compare(left.Name, right.Name, StringComparison.Ordinal),
            SortColumn.CpuPct => left.CpuSortBucket.CompareTo(right.CpuSortBucket),
            SortColumn.RssBytes => left.RssBytes.CompareTo(right.RssBytes),
            SortColumn.IoReadBps => left.IoReadBps.CompareTo(right.IoReadBps),
            SortColumn.IoWriteBps => left.IoWriteBps.CompareTo(right.IoWriteBps),
            SortColumn.OtherIoBps => left.OtherIoBps.CompareTo(right.OtherIoBps),
            SortColumn.DiskBps => left.DiskBps.CompareTo(right.DiskBps),
            SortColumn.Threads => left.Threads.CompareTo(right.Threads),
            SortColumn.Handles => left.Handles.CompareTo(right.Handles),
            SortColumn.StartTimeMs => left.StartTimeMs.CompareTo(right.StartTimeMs),
            _ => 0,
        };
    }

    private bool ShouldShowSample(ProcessSample sample)
    {
        if (IsFilteredByAdminVisibility(sample))
        {
            return false;
        }

        string needle = FilterText.Trim();
        if (string.IsNullOrWhiteSpace(needle))
        {
            return true;
        }

        return sample.Name.Contains(needle, StringComparison.OrdinalIgnoreCase)
               || sample.Pid.ToString().Contains(needle, StringComparison.OrdinalIgnoreCase);
    }

    private void ApplyCanonicalShaping()
    {
        ApplyCanonicalSort();
        ApplyCanonicalFilter();
    }

    private void ApplyCanonicalSort()
    {
        _rowSorter.OnNext(BuildSortComparer(CurrentSortColumn, CurrentSortDirection));
    }

    private void ApplyCanonicalFilter()
    {
        _rowFilter.OnNext(BuildVisibilityFilter());
    }

    private void ScheduleFilterApply(string filterText)
    {
        _filterDebounceCts?.Cancel();
        _filterDebounceCts?.Dispose();

        CancellationTokenSource cts = new();
        _filterDebounceCts = cts;
        _ = ApplyFilterAfterDelayAsync(filterText, cts.Token);
    }

    private async Task ApplyFilterAfterDelayAsync(string filterText, CancellationToken ct)
    {
        try
        {
            await Task.Delay(FilterDebounceMs, ct);
            if (ct.IsCancellationRequested)
            {
                return;
            }

            long probeStartedAt = Stopwatch.GetTimestamp();
            _runtime.SetFilter(filterText);
            RunOnUiThread(() =>
            {
                RefreshVisibleRows(refreshFilter: true);
                RecordTimingProbe(InteractionProbe.FilterApply, Stopwatch.GetTimestamp() - probeStartedAt);
            });
        }
        catch (OperationCanceledException)
        {
            // no-op
        }
    }

    internal void RecordSelectionSettleProbe(long elapsedTicks)
    {
        RecordTimingProbe(InteractionProbe.SelectionSettle, elapsedTicks);
    }

    internal void RecordPlotRefreshProbe(long elapsedTicks)
    {
        RecordTimingProbe(InteractionProbe.PlotRefresh, elapsedTicks);
    }

    private void RunOnUiThread(Action action)
    {
        var dispatcherQueue = _dispatcherQueue;
        if (dispatcherQueue is null || dispatcherQueue.HasThreadAccess)
        {
            action();
            return;
        }

        dispatcherQueue.TryEnqueue(() => action());
    }

    private string SortLabel(string text, SortColumn column)
    {
        if (CurrentSortColumn != column)
        {
            return text;
        }

        return $"{text} {SortDirectionSuffix(CurrentSortDirection)}";
    }

    private void RaiseSortHeaderLabels()
    {
        RaiseProperties(
            nameof(NameSortLabel),
            nameof(PidSortLabel),
            nameof(CpuSortLabel),
            nameof(MemorySortLabel),
            nameof(IoReadSortLabel),
            nameof(IoWriteSortLabel),
            nameof(OtherIoSortLabel),
            nameof(ThreadsSortLabel),
            nameof(HandlesSortLabel));
        RaiseCompactTableProperties();
    }

    private SortDirection ResolveNextSortDirection(SortColumn column)
    {
        return CurrentSortColumn == column && CurrentSortDirection == SortDirection.Desc
            ? SortDirection.Asc
            : SortDirection.Desc;
    }

    private bool IsFilteredByAdminVisibility(ProcessSample sample)
    {
        if (!AdminModeEnabled && sample.AccessState == AccessState.Denied)
        {
            return true;
        }

        return AdminEnabledOnlyFilter && sample.AccessState != AccessState.Full;
    }

    private static string SortDirectionSuffix(SortDirection direction)
    {
        return direction == SortDirection.Desc ? "↓" : "↑";
    }

    private void RecordTimingProbe(
        InteractionProbe probe,
        long elapsedTicks)
    {
        if (elapsedTicks <= 0)
        {
            return;
        }

        double sampleMs = elapsedTicks * 1000d / Stopwatch.Frequency;
        RecordInteractionProbeUnsmoothed(probe, sampleMs);
        if (!TryUpdateProbe(probe, sampleMs))
        {
            return;
        }

        InteractionTimingProbe = BuildInteractionTimingProbeText(
            _filterApplyProbeMs,
            _sortCompleteProbeMs,
            _selectionSettleProbeMs,
            _uiBatchProbeMs,
            _plotRefreshProbeMs);
    }

    private bool TryUpdateProbe(InteractionProbe probe, double sampleMs)
    {
        switch (probe)
        {
            case InteractionProbe.FilterApply:
                _filterApplyProbeMs = SmoothInteractionProbeSample(_filterApplyProbeMs, sampleMs);
                return true;
            case InteractionProbe.SortComplete:
                _sortCompleteProbeMs = SmoothInteractionProbeSample(_sortCompleteProbeMs, sampleMs);
                return true;
            case InteractionProbe.SelectionSettle:
                _selectionSettleProbeMs = SmoothInteractionProbeSample(_selectionSettleProbeMs, sampleMs);
                return true;
            case InteractionProbe.UiBatch:
                _uiBatchProbeMs = SmoothInteractionProbeSample(_uiBatchProbeMs, sampleMs);
                return true;
            case InteractionProbe.PlotRefresh:
                _plotRefreshProbeMs = SmoothInteractionProbeSample(_plotRefreshProbeMs, sampleMs);
                return true;
            default:
                return false;
        }
    }

    private static double SmoothInteractionProbeSample(double currentValue, double sample)
    {
        if (currentValue <= 0)
        {
            return sample;
        }

        return currentValue + ((sample - currentValue) * InteractionProbeSmoothingFactor);
    }

    private static string BuildInteractionTimingProbeText(
        double filterApplyMs,
        double sortCompleteMs,
        double selectionSettleMs,
        double uiBatchMs,
        double plotRefreshMs)
    {
        return $"Probe ms (smoothed): filter {FormatProbeMs(filterApplyMs)} | sort {FormatProbeMs(sortCompleteMs)} | selection {FormatProbeMs(selectionSettleMs)} | batch {FormatProbeMs(uiBatchMs)} | plot {FormatProbeMs(plotRefreshMs)}";
    }

    private static string FormatProbeMs(double value)
    {
        return value <= 0 ? "--" : value.ToString("F1");
    }

    partial void RecordInteractionProbeUnsmoothed(InteractionProbe probe, double sampleMs);

    private enum InteractionProbe
    {
        FilterApply,
        SortComplete,
        SelectionSettle,
        UiBatch,
        PlotRefresh,
    }
}
