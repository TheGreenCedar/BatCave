using BatCave.Core.Domain;
using CommunityToolkit.Mvvm.Input;
using System;
using System.Threading;
using System.Threading.Tasks;

namespace BatCave.ViewModels;

public partial class MonitoringShellViewModel
{
    [RelayCommand]
    private void ClearSelectionRequested()
    {
        ClearSelection();
    }

    public async Task ToggleSelectionAsync(ProcessSample? row, CancellationToken ct)
    {
        if (row is null)
        {
            if (ShouldPreserveSelectionOnNullToggle())
            {
                return;
            }

            ClearSelection();
            return;
        }

        if (IsSameAsCurrentSelection(row))
        {
            return;
        }

        await SelectRowAsync(row, ct);
    }

    public async Task SelectRowAsync(ProcessSample? row, CancellationToken ct)
    {
        if (row is null)
        {
            ClearSelection();
            return;
        }

        ProcessIdentity identity = row.Identity();
        PrepareSelectionState(row, identity);
        long requestVersion = Interlocked.Increment(ref _metadataRequestVersion);

        if (TryApplyCachedMetadata(identity))
        {
            return;
        }

        IsMetadataLoading = true;

        try
        {
            ProcessMetadata? metadata = await _metadataProvider.GetAsync(row.Pid, row.StartTimeMs, ct);
            RunOnUiThread(() => CompleteMetadataRequest(identity, requestVersion, metadata, error: null));
        }
        catch (Exception ex)
        {
            RunOnUiThread(() => CompleteMetadataRequest(identity, requestVersion, metadata: null, error: ex.Message));
        }
    }

    public void ClearSelection()
    {
        Interlocked.Increment(ref _metadataRequestVersion);
        SelectedRow = null;
        SelectedVisibleRow = null;
        SelectedMetadata = null;
        IsMetadataLoading = false;
        MetadataError = null;
    }

    private void ApplySelectedVisibleRowBinding(ProcessRowViewState? value)
    {
        if (_isApplyingSelectedVisibleRowBinding || ReferenceEquals(value, SelectedVisibleRow))
        {
            return;
        }

        _isApplyingSelectedVisibleRowBinding = true;
        try
        {
            if (value is not null)
            {
                _ = SelectRowAsync(value.Sample, CancellationToken.None);
                return;
            }

            ReconcileVisibleSelectionAfterNullBinding();
        }
        finally
        {
            _isApplyingSelectedVisibleRowBinding = false;
        }
    }

    private void RestoreVisibleSelection(ProcessRowViewState row)
    {
        // If reference is unchanged we still need to notify binding so ListView re-applies selection visuals.
        if (ReferenceEquals(SelectedVisibleRow, row))
        {
            RaiseSelectedVisibleRowBindingProperty();
            return;
        }

        SelectedVisibleRow = row;
    }

    private void ReassertSelectionAfterSort()
    {
        if (!TrySyncSelectedVisibleRowFromTrackedRows(ResolveVisibleSelectionAfterSort, out ProcessIdentity identity))
        {
            return;
        }

        RaiseSelectedVisibleRowBindingProperty();
        ReassertSelectedVisibleRowBindingOnDispatcher(identity);
    }

    private void ReconcileSelectionAfterDelta()
    {
        if (!TrySyncSelectedVisibleRowFromTrackedRows(TryGetVisibleRow, out _))
        {
            return;
        }
    }

    private ProcessRowViewState? TryGetVisibleRow(ProcessIdentity identity)
    {
        if (!_visibleRowStateByIdentity.TryGetValue(identity, out ProcessRowViewState? row))
        {
            return null;
        }

        return ShouldShowSample(row.Sample)
            ? row
            : null;
    }

    private bool IsSameAsCurrentSelection(ProcessSample row)
    {
        return SelectedRow?.Identity() == row.Identity();
    }

    private bool ShouldPreserveSelectionOnNullToggle()
    {
        if (SelectedRow is null)
        {
            return false;
        }

        return _allRows.TryGetValue(SelectedRow.Identity(), out _);
    }

    private void PrepareSelectionState(ProcessSample row, ProcessIdentity identity)
    {
        SelectedRow = row;
        SelectedVisibleRow = TryGetVisibleRow(identity);
        MetadataError = null;
        SelectedMetadata = null;
    }

    private bool TryApplyCachedMetadata(ProcessIdentity identity)
    {
        if (!_metadataCache.TryGetValue(identity, out ProcessMetadata? cached))
        {
            return false;
        }

        SelectedMetadata = cached;
        IsMetadataLoading = false;
        return true;
    }

    private void ReconcileVisibleSelectionAfterNullBinding()
    {
        if (!TryGetTrackedSelectedIdentity(out ProcessIdentity identity))
        {
            return;
        }

        ProcessRowViewState? restoredVisibleRow = TryResolveVisibleSelection(identity);
        if (restoredVisibleRow is not null)
        {
            RestoreVisibleSelection(restoredVisibleRow);
            return;
        }

        // Selected process is still tracked but hidden by filter/admin visibility; keep detail selection.
        SelectedVisibleRow = null;
    }

    private bool TryGetTrackedSelectedIdentity(out ProcessIdentity identity)
    {
        return TryGetTrackedSelection(out identity, out _);
    }

    private bool TryGetTrackedSelection(out ProcessIdentity identity, out ProcessSample selected)
    {
        if (SelectedRow is not ProcessSample currentSelection)
        {
            identity = default;
            selected = default!;
            return false;
        }

        identity = currentSelection.Identity();
        if (_allRows.TryGetValue(identity, out selected!))
        {
            return true;
        }

        ClearSelection();
        selected = default!;
        return false;
    }

    private ProcessRowViewState? TryResolveVisibleSelection(ProcessIdentity identity)
    {
        ProcessRowViewState? restoredVisibleRow = TryGetVisibleRow(identity);
        if (restoredVisibleRow is not null)
        {
            return restoredVisibleRow;
        }

        if (_visibleRowStateByIdentity.TryGetValue(identity, out ProcessRowViewState? expectedVisibleRow)
            && ShouldShowRow(expectedVisibleRow))
        {
            // Sorting/virtualization can briefly detach the selected item from the view.
            return expectedVisibleRow;
        }

        return null;
    }

    private bool TrySyncSelectedRowFromTrackedRows(out ProcessIdentity identity)
    {
        if (SelectedRow is null)
        {
            SelectedVisibleRow = null;
            identity = default;
            return false;
        }

        if (!TryGetTrackedSelection(out identity, out ProcessSample updated))
        {
            return false;
        }

        SelectedRow = updated;
        return true;
    }

    private bool TrySyncSelectedVisibleRowFromTrackedRows(
        Func<ProcessIdentity, ProcessRowViewState?> resolveVisibleSelection,
        out ProcessIdentity identity)
    {
        if (!TrySyncSelectedRowFromTrackedRows(out identity))
        {
            return false;
        }

        SelectedVisibleRow = resolveVisibleSelection(identity);
        return true;
    }

    private ProcessRowViewState? ResolveVisibleSelectionAfterSort(ProcessIdentity identity)
    {
        return _visibleRowStateByIdentity.TryGetValue(identity, out ProcessRowViewState? rowState)
               && ShouldShowRow(rowState)
            ? rowState
            : null;
    }

    private void ReassertSelectedVisibleRowBindingOnDispatcher(ProcessIdentity identity)
    {
        _dispatcherQueue?.TryEnqueue(() =>
        {
            if (SelectedRow?.Identity() == identity)
            {
                RaiseSelectedVisibleRowBindingProperty();
            }
        });
    }

    private bool IsCurrentMetadataRequest(long requestVersion, ProcessIdentity identity)
    {
        return requestVersion == _metadataRequestVersion && SelectedRow?.Identity() == identity;
    }

    private void CompleteMetadataRequest(ProcessIdentity identity, long requestVersion, ProcessMetadata? metadata, string? error)
    {
        if (!IsCurrentMetadataRequest(requestVersion, identity))
        {
            return;
        }

        if (string.IsNullOrWhiteSpace(error))
        {
            _metadataCache[identity] = metadata;
            SelectedMetadata = metadata;
            MetadataError = null;
        }
        else
        {
            SelectedMetadata = null;
            MetadataError = error;
        }

        IsMetadataLoading = false;
        QueueGlobalDetailStateRefresh();
    }
}
