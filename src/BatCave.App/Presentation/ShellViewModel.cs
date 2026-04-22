using BatCave.Runtime.Contracts;
using BatCave.Runtime.Presentation;
using CommunityToolkit.Mvvm.ComponentModel;
using CommunityToolkit.Mvvm.Input;
using Microsoft.UI.Dispatching;
using Microsoft.UI.Xaml;
using System.Collections.ObjectModel;

namespace BatCave.App.Presentation;

public sealed partial class ShellViewModel : ObservableObject, IAsyncDisposable
{
    private readonly IRuntimeStore _runtimeStore;
    private readonly CancellationTokenSource _subscriptionCts = new();
    private DispatcherQueue? _dispatcherQueue;
    private Task? _subscriptionTask;
    private RuntimeSnapshot _snapshot = new();
    private RuntimeViewState _viewState = new();
    private ProcessRowViewModel? _selectedRow;
    private string _filterText = string.Empty;
    private string _statusText = "Runtime starting.";
    private string _processCountText = "0 processes";
    private string _cpuText = "n/a";
    private string _memoryText = "n/a";
    private string _tickText = "0.0 ms";
    private string _sortText = "0.0 ms";
    private string _warningText = string.Empty;
    private bool _adminModeRequested;
    private bool _adminModeEnabled;
    private bool _isPaused;
    private double[] _cpuTrendValues = new double[60];
    private double[] _memoryTrendValues = new double[60];
    private int _trendIndex;

    public ShellViewModel(IRuntimeStore runtimeStore)
    {
        _runtimeStore = runtimeStore;
        ApplySnapshot(_runtimeStore.GetSnapshot());
    }

    public ObservableCollection<ProcessRowViewModel> Rows { get; } = [];

    internal ulong SnapshotSeq { get; private set; }

    public ProcessRowViewModel? SelectedRow
    {
        get => _selectedRow;
        set
        {
            if (SetProperty(ref _selectedRow, value))
            {
                _viewState = _viewState with
                {
                    SelectedIdentity = value?.Identity,
                    SelectedProcess = value?.Sample,
                };
                OnPropertyChanged(nameof(InspectorTitle));
                OnPropertyChanged(nameof(InspectorSubtitle));
                OnPropertyChanged(nameof(InspectorCpuText));
                OnPropertyChanged(nameof(InspectorMemoryText));
                OnPropertyChanged(nameof(InspectorDiskText));
                OnPropertyChanged(nameof(InspectorOtherIoText));
                OnPropertyChanged(nameof(InspectorThreadText));
                OnPropertyChanged(nameof(HasSelectedRow));
                OnPropertyChanged(nameof(ClearSelectionVisibility));
                ClearSelectionCommand.NotifyCanExecuteChanged();
            }
        }
    }

    public string FilterText
    {
        get => _filterText;
        set
        {
            if (SetProperty(ref _filterText, value))
            {
                _ = SetQueryAsync(_snapshot.Settings.Query with { FilterText = value });
            }
        }
    }

    public string StatusText
    {
        get => _statusText;
        private set => SetProperty(ref _statusText, value);
    }

    public string ProcessCountText
    {
        get => _processCountText;
        private set => SetProperty(ref _processCountText, value);
    }

    public string CpuText
    {
        get => _cpuText;
        private set => SetProperty(ref _cpuText, value);
    }

    public string MemoryText
    {
        get => _memoryText;
        private set => SetProperty(ref _memoryText, value);
    }

    public string TickText
    {
        get => _tickText;
        private set => SetProperty(ref _tickText, value);
    }

    public string SortText
    {
        get => _sortText;
        private set => SetProperty(ref _sortText, value);
    }

    public string ProcessSortText => $"Sort: {FormatSortColumn(_snapshot.Settings.Query.SortColumn)} {FormatSortDirection(_snapshot.Settings.Query.SortDirection)}";

    public SortColumn CurrentSortColumn => _snapshot.Settings.Query.SortColumn;

    public SortDirection CurrentSortDirection => _snapshot.Settings.Query.SortDirection;

    public string WarningText
    {
        get => _warningText;
        private set
        {
            if (SetProperty(ref _warningText, value))
            {
                OnPropertyChanged(nameof(HasWarning));
                OnPropertyChanged(nameof(WarningVisibility));
            }
        }
    }

    public bool HasWarning => !string.IsNullOrWhiteSpace(WarningText);

    public Visibility WarningVisibility => string.IsNullOrWhiteSpace(WarningText) ? Visibility.Collapsed : Visibility.Visible;

    public bool AdminModeEnabled
    {
        get => _adminModeEnabled;
        private set
        {
            if (SetProperty(ref _adminModeEnabled, value))
            {
                RefreshAdminStatusProperties();
            }
        }
    }

    public bool AdminModeRequested
    {
        get => _adminModeRequested;
        private set
        {
            if (SetProperty(ref _adminModeRequested, value))
            {
                RefreshAdminStatusProperties();
            }
        }
    }

    public string AdminStatusText => AdminModeRequested
        ? AdminModeEnabled ? "Admin requested; elevated access active" : "Admin requested; standard access active"
        : "Standard access";

    public bool CanRetryAdminMode => AdminModeRequested && !AdminModeEnabled;

    public Visibility RetryAdminModeVisibility => CanRetryAdminMode ? Visibility.Visible : Visibility.Collapsed;

    public bool HasSelectedRow => SelectedRow is not null;

    public Visibility ClearSelectionVisibility => HasSelectedRow ? Visibility.Visible : Visibility.Collapsed;

    public bool IsPaused
    {
        get => _isPaused;
        private set
        {
            if (SetProperty(ref _isPaused, value))
            {
                RefreshCommandStates();
            }
        }
    }

    public double[] CpuTrendValues
    {
        get => _cpuTrendValues;
        private set => SetProperty(ref _cpuTrendValues, value);
    }

    public double[] MemoryTrendValues
    {
        get => _memoryTrendValues;
        private set => SetProperty(ref _memoryTrendValues, value);
    }

    public string InspectorTitle => SelectedRow?.Name ?? "System Overview";

    public string InspectorSubtitle => SelectedRow is null
        ? "Live system telemetry and runtime health"
        : $"PID {SelectedRow.PidText}";

    public string InspectorCpuText => SelectedRow?.CpuText ?? CpuText;

    public string InspectorMemoryText => SelectedRow?.MemoryText ?? MemoryText;

    public string InspectorDiskText => SelectedRow?.DiskText ?? FormatSystemDisk(_snapshot.System);

    public string InspectorOtherIoText => SelectedRow?.OtherIoText ?? FormatNullableRate(_snapshot.System.OtherIoBps);

    public string InspectorThreadText => SelectedRow?.ThreadsText ?? _snapshot.System.LogicalProcessorCount.ToString(System.Globalization.CultureInfo.InvariantCulture);

    public void AttachDispatcherQueue(DispatcherQueue dispatcherQueue)
    {
        _dispatcherQueue = dispatcherQueue;
    }

    public void Start()
    {
        if (_subscriptionTask is { IsCompleted: false })
        {
            return;
        }

        _subscriptionTask = Task.Run(SubscribeAsync);
    }

    public async ValueTask DisposeAsync()
    {
        _subscriptionCts.Cancel();
        if (_subscriptionTask is not null)
        {
            try
            {
                await _subscriptionTask.ConfigureAwait(false);
            }
            catch (OperationCanceledException)
            {
            }
        }

        _subscriptionCts.Dispose();
    }

    [RelayCommand]
    private Task RefreshAsync()
    {
        return _runtimeStore.ExecuteAsync(new RefreshNowCommand(), CancellationToken.None);
    }

    [RelayCommand(CanExecute = nameof(CanPause))]
    private Task PauseAsync()
    {
        return _runtimeStore.ExecuteAsync(new PauseRuntimeCommand(), CancellationToken.None);
    }

    [RelayCommand(CanExecute = nameof(CanResume))]
    private Task ResumeAsync()
    {
        return _runtimeStore.ExecuteAsync(new ResumeRuntimeCommand(), CancellationToken.None);
    }

    [RelayCommand]
    private Task SortAsync(string? columnName)
    {
        if (!Enum.TryParse(columnName, out SortColumn column))
        {
            column = SortColumn.CpuPct;
        }

        RuntimeQuery current = _snapshot.Settings.Query;
        SortDirection nextDirection = current.SortColumn == column && current.SortDirection == SortDirection.Desc
            ? SortDirection.Asc
            : SortDirection.Desc;
        return SetQueryAsync(current with { SortColumn = column, SortDirection = nextDirection });
    }

    public Task SetAdminModeAsync(bool enabled)
    {
        return _runtimeStore.ExecuteAsync(new SetAdminModeCommand(enabled), CancellationToken.None);
    }

    [RelayCommand(CanExecute = nameof(CanRetryAdminMode))]
    private Task RetryAdminModeAsync()
    {
        return SetAdminModeAsync(true);
    }

    [RelayCommand(CanExecute = nameof(HasSelectedRow))]
    private void ClearSelection()
    {
        SelectedRow = null;
    }

    private bool CanPause() => !IsPaused;

    private bool CanResume() => IsPaused;

    private void RefreshCommandStates()
    {
        PauseCommand.NotifyCanExecuteChanged();
        ResumeCommand.NotifyCanExecuteChanged();
    }

    private void RefreshAdminStatusProperties()
    {
        OnPropertyChanged(nameof(AdminStatusText));
        OnPropertyChanged(nameof(CanRetryAdminMode));
        OnPropertyChanged(nameof(RetryAdminModeVisibility));
        RetryAdminModeCommand.NotifyCanExecuteChanged();
    }

    private Task SetQueryAsync(RuntimeQuery query)
    {
        return _runtimeStore.ExecuteAsync(new SetProcessQueryCommand(query), CancellationToken.None);
    }

    private async Task SubscribeAsync()
    {
        await foreach (RuntimeDelta delta in _runtimeStore.SubscribeAsync(_subscriptionCts.Token).ConfigureAwait(false))
        {
            DispatchApply(delta.Snapshot);
        }
    }

    private void DispatchApply(RuntimeSnapshot snapshot)
    {
        DispatcherQueue? dispatcherQueue = _dispatcherQueue;
        if (dispatcherQueue is null || dispatcherQueue.HasThreadAccess)
        {
            ApplySnapshot(snapshot);
            return;
        }

        _ = dispatcherQueue.TryEnqueue(() => ApplySnapshot(snapshot));
    }

    private void ApplySnapshot(RuntimeSnapshot snapshot)
    {
        RuntimeViewState viewState = RuntimeViewReducer.Reduce(_viewState, snapshot);
        _viewState = viewState;
        ProcessIdentity? selectedIdentity = viewState.SelectedIdentity;
        _snapshot = snapshot;
        SnapshotSeq = snapshot.Seq;
        StatusText = snapshot.Health.StatusSummary;
        ProcessCountText = $"{snapshot.TotalProcessCount:n0} processes";
        CpuText = snapshot.System.CpuPct.HasValue ? $"{snapshot.System.CpuPct.Value:0.0}%" : "warming";
        MemoryText = FormatMemory(snapshot.System);
        TickText = $"{snapshot.Health.TickP95Ms:0.0} ms";
        SortText = $"{snapshot.Health.SortP95Ms:0.0} ms";
        OnPropertyChanged(nameof(ProcessSortText));
        OnPropertyChanged(nameof(CurrentSortColumn));
        OnPropertyChanged(nameof(CurrentSortDirection));
        WarningText = viewState.HealthBanner ?? string.Empty;
        AdminModeRequested = snapshot.Settings.AdminModeRequested;
        AdminModeEnabled = snapshot.Settings.AdminModeEnabled;
        IsPaused = snapshot.Settings.Paused;
        if (!string.Equals(FilterText, snapshot.Settings.Query.FilterText, StringComparison.Ordinal))
        {
            _filterText = snapshot.Settings.Query.FilterText;
            OnPropertyChanged(nameof(FilterText));
        }

        ApplyProcessRows(viewState.Rows, selectedIdentity);
        AppendTrend(snapshot);
        OnPropertyChanged(nameof(InspectorCpuText));
        OnPropertyChanged(nameof(InspectorMemoryText));
        OnPropertyChanged(nameof(InspectorDiskText));
        OnPropertyChanged(nameof(InspectorOtherIoText));
        OnPropertyChanged(nameof(InspectorThreadText));
    }

    private void ApplyProcessRows(IReadOnlyList<ProcessSample> rows, ProcessIdentity? selectedIdentity)
    {
        Dictionary<ProcessIdentity, ProcessRowViewModel> existing = Rows.ToDictionary(static row => row.Identity);
        for (int index = 0; index < rows.Count; index++)
        {
            ProcessSample sample = rows[index];
            ProcessIdentity identity = sample.Identity();
            ProcessRowViewModel viewRow = existing.TryGetValue(identity, out ProcessRowViewModel? current)
                && current.HasSameDisplayState(sample)
                    ? current
                    : new ProcessRowViewModel(sample);

            if (index < Rows.Count)
            {
                if (!ReferenceEquals(Rows[index], viewRow))
                {
                    Rows[index] = viewRow;
                }
            }
            else
            {
                Rows.Add(viewRow);
            }
        }

        while (Rows.Count > rows.Count)
        {
            Rows.RemoveAt(Rows.Count - 1);
        }

        SelectedRow = selectedIdentity.HasValue
            ? Rows.FirstOrDefault(row => row.Identity.Equals(selectedIdentity.Value))
            : null;
    }

    private void AppendTrend(RuntimeSnapshot snapshot)
    {
        double[] cpu = (double[])CpuTrendValues.Clone();
        double[] memory = (double[])MemoryTrendValues.Clone();
        cpu[_trendIndex] = snapshot.System.CpuPct.GetValueOrDefault();
        ulong totalMemoryBytes = snapshot.System.MemoryTotalBytes.GetValueOrDefault();
        memory[_trendIndex] = totalMemoryBytes == 0
            ? 0
            : snapshot.System.MemoryUsedBytes.GetValueOrDefault() * 100d / totalMemoryBytes;
        _trendIndex = (_trendIndex + 1) % cpu.Length;
        CpuTrendValues = RotateTrend(cpu, _trendIndex);
        MemoryTrendValues = RotateTrend(memory, _trendIndex);
    }

    private static double[] RotateTrend(double[] values, int start)
    {
        double[] rotated = new double[values.Length];
        for (int index = 0; index < values.Length; index++)
        {
            rotated[index] = values[(start + index) % values.Length];
        }

        return rotated;
    }

    private static string FormatMemory(SystemMetricsSnapshot snapshot)
    {
        if (!snapshot.MemoryUsedBytes.HasValue)
        {
            return "n/a";
        }

        if (!snapshot.MemoryTotalBytes.HasValue || snapshot.MemoryTotalBytes.Value == 0)
        {
            return ProcessRowViewModel.FormatBytes(snapshot.MemoryUsedBytes.Value);
        }

        double pct = snapshot.MemoryUsedBytes.Value * 100d / snapshot.MemoryTotalBytes.Value;
        return $"{pct:0.0}%  {ProcessRowViewModel.FormatBytes(snapshot.MemoryUsedBytes.Value)}";
    }

    private static string FormatSystemDisk(SystemMetricsSnapshot snapshot)
    {
        if (!snapshot.DiskReadBps.HasValue && !snapshot.DiskWriteBps.HasValue)
        {
            return "n/a";
        }

        ulong read = snapshot.DiskReadBps.GetValueOrDefault();
        ulong write = snapshot.DiskWriteBps.GetValueOrDefault();
        ulong total = ulong.MaxValue - read < write ? ulong.MaxValue : read + write;
        return ProcessRowViewModel.FormatRate(total);
    }

    private static string FormatNullableRate(ulong? bytesPerSecond)
    {
        return bytesPerSecond.HasValue ? ProcessRowViewModel.FormatRate(bytesPerSecond.Value) : "n/a";
    }

    private static string FormatSortColumn(SortColumn column)
    {
        return column switch
        {
            SortColumn.CpuPct => "CPU",
            SortColumn.MemoryBytes => "Memory",
            SortColumn.DiskBps => "Disk",
            SortColumn.OtherIoBps => "Other I/O",
            SortColumn.StartTimeMs => "Start time",
            _ => column.ToString(),
        };
    }

    private static string FormatSortDirection(SortDirection direction)
    {
        return direction == SortDirection.Asc ? "ascending" : "descending";
    }
}
