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
            if (SelectedRow is not null && _allRows.ContainsKey(SelectedRow.Identity()))
            {
                return;
            }

            ClearSelection();
            return;
        }

        if (SelectedRow?.Identity() == row.Identity())
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
        SelectedRow = row;
        SelectedVisibleRow = TryGetVisibleRow(identity);
        MetadataError = null;
        SelectedMetadata = null;

        long requestVersion = Interlocked.Increment(ref _metadataRequestVersion);

        if (_metadataCache.TryGetValue(identity, out ProcessMetadata? cached))
        {
            SelectedMetadata = cached;
            IsMetadataLoading = false;
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

            if (SelectedRow is null)
            {
                return;
            }

            ProcessIdentity identity = SelectedRow.Identity();
            if (!_allRows.ContainsKey(identity))
            {
                ClearSelection();
                return;
            }

            ProcessRowViewState? restoredVisibleRow = TryGetVisibleRow(identity);
            if (restoredVisibleRow is not null)
            {
                RestoreVisibleSelection(restoredVisibleRow);
                return;
            }

            if (_visibleRowStateByIdentity.TryGetValue(identity, out ProcessRowViewState? expectedVisibleRow)
                && ShouldShowRow(expectedVisibleRow))
            {
                // Sorting/virtualization can briefly detach the selected item from the view.
                RestoreVisibleSelection(expectedVisibleRow);
                return;
            }

            // Selected process is still tracked but hidden by filter/admin visibility; keep detail selection.
            SelectedVisibleRow = null;
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
            OnPropertyChanged(nameof(SelectedVisibleRowBinding));
            return;
        }

        SelectedVisibleRow = row;
    }

    private void ReassertSelectionAfterSort()
    {
        if (SelectedRow is null)
        {
            SelectedVisibleRow = null;
            return;
        }

        ProcessIdentity identity = SelectedRow.Identity();
        if (!_allRows.TryGetValue(identity, out ProcessSample? updated))
        {
            ClearSelection();
            return;
        }

        SelectedRow = updated;
        ProcessRowViewState? visibleRow = _visibleRowStateByIdentity.TryGetValue(identity, out ProcessRowViewState? rowState)
            && ShouldShowRow(rowState)
            ? rowState
            : null;

        SelectedVisibleRow = visibleRow;
        OnPropertyChanged(nameof(SelectedVisibleRowBinding));

        _dispatcherQueue?.TryEnqueue(() =>
        {
            if (SelectedRow?.Identity() == identity)
            {
                OnPropertyChanged(nameof(SelectedVisibleRowBinding));
            }
        });
    }

    private void ReconcileSelectionAfterDelta()
    {
        if (SelectedRow is null)
        {
            SelectedVisibleRow = null;
            return;
        }

        ProcessIdentity identity = SelectedRow.Identity();
        if (_allRows.TryGetValue(identity, out ProcessSample? updated))
        {
            SelectedRow = updated;
            SelectedVisibleRow = TryGetVisibleRow(identity);
            return;
        }

        ClearSelection();
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
