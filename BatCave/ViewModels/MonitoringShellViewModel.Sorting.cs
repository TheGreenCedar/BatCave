using System;
using System.Threading;
using System.Threading.Tasks;
using BatCave.Core.Domain;
using CommunityToolkit.Mvvm.Input;
using SortDescription = CommunityToolkit.WinUI.Collections.SortDescription;

namespace BatCave.ViewModels;

public partial class MonitoringShellViewModel
{
    public void ChangeSort(SortColumn column)
    {
        SortDirection nextDirection = CurrentSortColumn == column && CurrentSortDirection == SortDirection.Desc
            ? SortDirection.Asc
            : SortDirection.Desc;

        CurrentSortColumn = column;
        CurrentSortDirection = nextDirection;

        _runtime.SetSort(CurrentSortColumn, CurrentSortDirection);
        ApplySortDescriptions();
        ReassertSelectionAfterSort();
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

    private bool ShouldShowSample(ProcessSample sample)
    {
        if (!AdminModeEnabled && sample.AccessState == AccessState.Denied)
        {
            return false;
        }

        if (AdminEnabledOnlyFilter && sample.AccessState != AccessState.Full)
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

    private void ApplySortDescriptions()
    {
        string primarySortKey = CurrentSortColumn switch
        {
            SortColumn.Pid => nameof(ProcessRowViewState.Pid),
            SortColumn.Name => nameof(ProcessRowViewState.Name),
            SortColumn.CpuPct => nameof(ProcessRowViewState.CpuSortBucket),
            SortColumn.RssBytes => nameof(ProcessRowViewState.RssBytes),
            SortColumn.IoReadBps => nameof(ProcessRowViewState.IoReadBps),
            SortColumn.IoWriteBps => nameof(ProcessRowViewState.IoWriteBps),
            SortColumn.NetBps => nameof(ProcessRowViewState.NetBps),
            SortColumn.Threads => nameof(ProcessRowViewState.Threads),
            SortColumn.Handles => nameof(ProcessRowViewState.Handles),
            SortColumn.StartTimeMs => nameof(ProcessRowViewState.StartTimeMs),
            _ => nameof(ProcessRowViewState.CpuSortBucket),
        };

        CommunityToolkit.WinUI.Collections.SortDirection direction = CurrentSortDirection == SortDirection.Asc
            ? CommunityToolkit.WinUI.Collections.SortDirection.Ascending
            : CommunityToolkit.WinUI.Collections.SortDirection.Descending;

        VisibleRows.SortDescriptions.Clear();
        VisibleRows.SortDescriptions.Add(new SortDescription(primarySortKey, direction));
        AddSortTieBreakers(primarySortKey);
    }

    private void AddSortTieBreakers(string primarySortKey)
    {
        if (!string.Equals(primarySortKey, nameof(ProcessRowViewState.Pid), StringComparison.Ordinal))
        {
            VisibleRows.SortDescriptions.Add(
                new SortDescription(
                    nameof(ProcessRowViewState.Pid),
                    CommunityToolkit.WinUI.Collections.SortDirection.Ascending));
        }

        if (!string.Equals(primarySortKey, nameof(ProcessRowViewState.StartTimeMs), StringComparison.Ordinal))
        {
            VisibleRows.SortDescriptions.Add(
                new SortDescription(
                    nameof(ProcessRowViewState.StartTimeMs),
                    CommunityToolkit.WinUI.Collections.SortDirection.Ascending));
        }
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

            _runtime.SetFilter(filterText);
            RunOnUiThread(() => RefreshVisibleRows(refreshFilter: true));
        }
        catch (OperationCanceledException)
        {
            // no-op
        }
    }

    private void RunOnUiThread(Action action)
    {
        if (_dispatcherQueue is null || _dispatcherQueue.HasThreadAccess)
        {
            action();
            return;
        }

        _dispatcherQueue.TryEnqueue(() => action());
    }

    private string SortLabel(string text, SortColumn column)
    {
        if (CurrentSortColumn != column)
        {
            return text;
        }

        return CurrentSortDirection == SortDirection.Desc
            ? $"{text} ↓"
            : $"{text} ↑";
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
            nameof(NetSortLabel),
            nameof(ThreadsSortLabel),
            nameof(HandlesSortLabel));
    }
}
