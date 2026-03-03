using System;
using System.Threading;
using System.Threading.Tasks;
using BatCave.Core.Domain;
using CommunityToolkit.Mvvm.Input;

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
        if (ShouldSkipSelectedVisibleRowBinding(value))
        {
            return;
        }

        _isApplyingSelectedVisibleRowBinding = true;
        try
        {
            if (TryApplySelectionFromVisibleBinding(value))
            {
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
        if (!TrySyncSelectedRowFromTrackedRows(out ProcessIdentity identity))
        {
            return;
        }

        SelectedVisibleRow = ResolveVisibleSelectionAfterSort(identity);
        RaiseSelectedVisibleRowBindingProperty();
        ReassertSelectedVisibleRowBindingOnDispatcher(identity);
    }

    private void ReconcileSelectionAfterDelta()
    {
        if (!TrySyncSelectedRowFromTrackedRows(out ProcessIdentity identity))
        {
            return;
        }

        SelectedVisibleRow = TryGetVisibleRow(identity);
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
        return SelectedRow is not null && _allRows.ContainsKey(SelectedRow.Identity());
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

    private bool TryApplySelectionFromVisibleBinding(ProcessRowViewState? value)
    {
        if (value is null)
        {
            return false;
        }

        _ = SelectRowAsync(value.Sample, CancellationToken.None);
        return true;
    }

    private bool ShouldSkipSelectedVisibleRowBinding(ProcessRowViewState? value)
    {
        return _isApplyingSelectedVisibleRowBinding || ReferenceEquals(value, SelectedVisibleRow);
    }

    private void ReconcileVisibleSelectionAfterNullBinding()
    {
        if (!TryGetTrackedSelectedIdentityForBinding(out ProcessIdentity identity))
        {
            return;
        }

        if (TryRestoreVisibleSelection(identity))
        {
            return;
        }

        // Selected process is still tracked but hidden by filter/admin visibility; keep detail selection.
        SelectedVisibleRow = null;
    }

    private bool TryGetTrackedSelectedIdentityForBinding(out ProcessIdentity identity)
    {
        if (SelectedRow is null)
        {
            identity = default;
            return false;
        }

        identity = SelectedRow.Identity();
        if (_allRows.ContainsKey(identity))
        {
            return true;
        }

        ClearSelection();
        return false;
    }

    private bool TryRestoreVisibleSelection(ProcessIdentity identity)
    {
        ProcessRowViewState? restoredVisibleRow = TryGetVisibleRow(identity);
        if (restoredVisibleRow is not null)
        {
            RestoreVisibleSelection(restoredVisibleRow);
            return true;
        }

        if (_visibleRowStateByIdentity.TryGetValue(identity, out ProcessRowViewState? expectedVisibleRow)
            && ShouldShowRow(expectedVisibleRow))
        {
            // Sorting/virtualization can briefly detach the selected item from the view.
            RestoreVisibleSelection(expectedVisibleRow);
            return true;
        }

        return false;
    }

    private bool TrySyncSelectedRowFromTrackedRows(out ProcessIdentity identity)
    {
        if (SelectedRow is null)
        {
            SelectedVisibleRow = null;
            identity = default;
            return false;
        }

        identity = SelectedRow.Identity();
        if (!_allRows.TryGetValue(identity, out ProcessSample? updated))
        {
            ClearSelection();
            return false;
        }

        SelectedRow = updated;
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
    }
}
