using BatCave.Runtime.Benchmarking;
using BatCave.Runtime.Contracts;
using BatCave.Runtime.Presentation;
using BatCave.Runtime.Serialization;
using CommunityToolkit.Mvvm.ComponentModel;
using CommunityToolkit.Mvvm.Input;
using Microsoft.UI.Dispatching;
using Microsoft.UI.Xaml;
using System.Collections.ObjectModel;
using System.Globalization;
using System.Text.Json;

namespace BatCave.App.Presentation;

public sealed partial class ShellViewModel : ObservableObject, IAsyncDisposable
{
    private const int FilterDebounceMs = 150;

    private readonly IRuntimeStore _runtimeStore;
    private readonly CancellationTokenSource _subscriptionCts = new();
    private readonly Dictionary<ProcessIdentity, ProcessSample> _previousSamplesByIdentity = [];
    private DispatcherQueue? _dispatcherQueue;
    private Task? _subscriptionTask;
    private CancellationTokenSource? _filterDebounceCts;
    private RuntimeSnapshot _snapshot = new();
    private RuntimeViewState _viewState = new();
    private ProcessRowViewModel? _selectedRow;
    private ProcessQuickFilter _quickFilter = ProcessQuickFilter.All;
    private string _filterText = string.Empty;
    private string _statusText = "Runtime starting.";
    private string _processCountText = "0 processes";
    private string _filterMatchText = "0 shown";
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
    private BenchmarkSummary? _latestBenchmarkSummary;
    private int _trendIndex;

    public ShellViewModel(IRuntimeStore runtimeStore)
    {
        _runtimeStore = runtimeStore;
        _latestBenchmarkSummary = TryLoadLatestBenchmarkSummary();
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
                OnPropertyChanged(nameof(InspectorParentPidText));
                OnPropertyChanged(nameof(InspectorStartTimeText));
                OnPropertyChanged(nameof(InspectorAccessStateText));
                OnPropertyChanged(nameof(InspectorPrivateMemoryText));
                OnPropertyChanged(nameof(InspectorHandlesText));
                OnPropertyChanged(nameof(InspectorAttentionText));
                OnPropertyChanged(nameof(InspectorLastChangeText));
                OnPropertyChanged(nameof(CopyDetailsText));
                OnPropertyChanged(nameof(HasSelectedRow));
                OnPropertyChanged(nameof(ClearSelectionVisibility));
                OnPropertyChanged(nameof(CopyDetailsVisibility));
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
                OnPropertyChanged(nameof(ClearFilterVisibility));
                QueueFilterUpdate(value);
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

    public string FilterMatchText
    {
        get => _filterMatchText;
        private set => SetProperty(ref _filterMatchText, value);
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

    public string RuntimeConfidenceText => _snapshot.Health.DegradeMode ? "Runtime confidence: degraded" : $"Runtime confidence: {_snapshot.Health.StatusSummary}";

    public string RuntimePerfText =>
        $"tick p95 {TickText} | sort p95 {SortText} | jitter p95 {_snapshot.Health.JitterP95Ms:0.0} ms | dropped {_snapshot.Health.DroppedTicks:n0}";

    public string RuntimeBudgetText =>
        $"app CPU {_snapshot.Health.AppCpuPct:0.0}% | RSS {ProcessRowViewModel.FormatBytes(_snapshot.Health.AppRssBytes)}";

    public string FooterCpuText => $"CPU {CpuText}";

    public string FooterTickText => $"tick p95 {TickText}";

    public string FooterSortP95Text => $"sort p95 {SortText}";

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

    public Visibility CopyDetailsVisibility => HasSelectedRow ? Visibility.Visible : Visibility.Collapsed;

    public Visibility ClearFilterVisibility => string.IsNullOrWhiteSpace(FilterText) ? Visibility.Collapsed : Visibility.Visible;

    public Visibility EmptyStateVisibility => Rows.Count == 0 ? Visibility.Visible : Visibility.Collapsed;

    public string EmptyStateText
    {
        get
        {
            if (_snapshot.Seq == 0 && !_snapshot.System.IsReady)
            {
                return "Warming process telemetry.";
            }

            if (!string.IsNullOrWhiteSpace(FilterText))
            {
                return $"No process matches \"{FilterText}\".";
            }

            return _quickFilter == ProcessQuickFilter.All
                ? "No processes available."
                : $"No processes match {QuickFilterLabel(_quickFilter)}.";
        }
    }

    public string ActiveQuickFilterText => $"View: {QuickFilterLabel(_quickFilter)}";

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

    public string InspectorParentPidText => SelectedRow?.ParentPidText ?? "n/a";

    public string InspectorStartTimeText => SelectedRow?.StartTimeText ?? "n/a";

    public string InspectorAccessStateText => SelectedRow?.AccessStateText ?? "System";

    public string InspectorPrivateMemoryText => SelectedRow?.PrivateMemoryText ?? "n/a";

    public string InspectorHandlesText => SelectedRow?.HandlesText ?? "n/a";

    public string InspectorAttentionText => SelectedRow?.AttentionSummaryText ?? "System overview";

    public string InspectorLastChangeText => SelectedRow?.LastMeaningfulChangeText ?? "System-level metrics update every runtime tick.";

    public string CopyDetailsText => SelectedRow?.ToClipboardText() ?? string.Empty;

    public string BenchmarkStatusText => _latestBenchmarkSummary is null
        ? "No benchmark artifact found."
        : $"{_latestBenchmarkSummary.Host} benchmark: tick p95 {_latestBenchmarkSummary.TickP95Ms:0.0} ms, sort p95 {_latestBenchmarkSummary.SortP95Ms:0.0} ms.";

    public string BenchmarkBudgetText => _latestBenchmarkSummary is null
        ? "Run a benchmark to populate artifacts/benchmarks."
        : $"budget {PassFail(_latestBenchmarkSummary.BudgetPassed)} | strict {PassFail(_latestBenchmarkSummary.StrictPassed)} | app CPU {_latestBenchmarkSummary.AvgAppCpuPct:0.0}% | RSS {ProcessRowViewModel.FormatBytes(_latestBenchmarkSummary.AvgAppRssBytes)}";

    public string BenchmarkCommandText =>
        "powershell -NoProfile -ExecutionPolicy Bypass -File scripts/run-benchmark.ps1 -BenchmarkHost core -Platform x64 -Ticks 120 -SleepMs 1000";

    public string ValidationCommandText =>
        "powershell -NoProfile -ExecutionPolicy Bypass -File scripts/validate-winui.ps1 -Platform x64";

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

        _filterDebounceCts?.Cancel();
        _filterDebounceCts?.Dispose();
        _filterDebounceCts = null;
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

    [RelayCommand]
    private void ClearFilter()
    {
        FilterText = string.Empty;
    }

    [RelayCommand]
    private void ApplyQuickFilter(string? mode)
    {
        _quickFilter = ParseQuickFilter(mode);
        OnPropertyChanged(nameof(ActiveQuickFilterText));
        ApplyProcessRows(_viewState.Rows, _viewState.SelectedIdentity);
        RefreshProcessTextProperties();
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

    private void QueueFilterUpdate(string value)
    {
        _filterDebounceCts?.Cancel();
        _filterDebounceCts?.Dispose();
        CancellationTokenSource cts = new();
        _filterDebounceCts = cts;
        _ = ApplyFilterAfterDelayAsync(value, cts.Token);
    }

    private async Task ApplyFilterAfterDelayAsync(string value, CancellationToken ct)
    {
        try
        {
            await Task.Delay(FilterDebounceMs, ct).ConfigureAwait(false);
            await SetQueryAsync(_snapshot.Settings.Query with { FilterText = value }).ConfigureAwait(false);
        }
        catch (OperationCanceledException)
        {
        }
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
        CpuText = snapshot.System.CpuPct.HasValue ? $"{snapshot.System.CpuPct.Value:0.0}%" : "warming";
        MemoryText = FormatMemory(snapshot.System);
        TickText = $"{snapshot.Health.TickP95Ms:0.0} ms";
        SortText = $"{snapshot.Health.SortP95Ms:0.0} ms";
        OnPropertyChanged(nameof(RuntimeConfidenceText));
        OnPropertyChanged(nameof(RuntimePerfText));
        OnPropertyChanged(nameof(RuntimeBudgetText));
        OnPropertyChanged(nameof(FooterCpuText));
        OnPropertyChanged(nameof(FooterTickText));
        OnPropertyChanged(nameof(FooterSortP95Text));
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
            OnPropertyChanged(nameof(ClearFilterVisibility));
        }

        ApplyProcessRows(viewState.Rows, selectedIdentity);
        AppendTrend(snapshot);
        RefreshProcessTextProperties();
        OnPropertyChanged(nameof(InspectorCpuText));
        OnPropertyChanged(nameof(InspectorMemoryText));
        OnPropertyChanged(nameof(InspectorDiskText));
        OnPropertyChanged(nameof(InspectorOtherIoText));
        OnPropertyChanged(nameof(InspectorThreadText));
    }

    private void ApplyProcessRows(IReadOnlyList<ProcessSample> rows, ProcessIdentity? selectedIdentity)
    {
        IReadOnlyList<ProcessSample> visibleRows = ApplyQuickFilter(rows);
        Dictionary<ProcessIdentity, ProcessRowViewModel> existing = Rows.ToDictionary(static row => row.Identity);
        for (int index = 0; index < visibleRows.Count; index++)
        {
            ProcessSample sample = visibleRows[index];
            ProcessIdentity identity = sample.Identity();
            _previousSamplesByIdentity.TryGetValue(identity, out ProcessSample? previousSample);
            bool isNew = previousSample is null && SnapshotSeq > 0;
            ProcessRowViewModel viewRow = existing.TryGetValue(identity, out ProcessRowViewModel? current)
                && current.HasSameDisplayState(sample)
                    ? current
                    : new ProcessRowViewModel(sample, previousSample, isNew);

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

        while (Rows.Count > visibleRows.Count)
        {
            Rows.RemoveAt(Rows.Count - 1);
        }

        foreach (ProcessSample sample in rows)
        {
            _previousSamplesByIdentity[sample.Identity()] = sample;
        }

        SelectedRow = selectedIdentity.HasValue
            ? Rows.FirstOrDefault(row => row.Identity.Equals(selectedIdentity.Value))
            : null;
    }

    private IReadOnlyList<ProcessSample> ApplyQuickFilter(IReadOnlyList<ProcessSample> rows)
    {
        IEnumerable<ProcessSample> filtered = _quickFilter switch
        {
            ProcessQuickFilter.HighCpu => rows.Where(static row => row.CpuPct >= 1d),
            ProcessQuickFilter.HighMemory => rows.Where(static row => row.MemoryBytes >= ProcessAttention.MemoryHeavyThresholdBytes),
            ProcessQuickFilter.ActiveIo => rows.Where(static row => row.DiskBps + row.OtherIoBps > 0UL),
            ProcessQuickFilter.LimitedAccess => rows.Where(static row => row.AccessState != AccessState.Full),
            _ => rows,
        };

        return filtered.ToArray();
    }

    private void RefreshProcessTextProperties()
    {
        int queriedCount = _viewState.Rows.Count;
        int visibleCount = Rows.Count;
        ProcessCountText = _snapshot.TotalProcessCount == queriedCount && _quickFilter == ProcessQuickFilter.All
            ? $"{_snapshot.TotalProcessCount:n0} processes"
            : $"{visibleCount:n0} shown of {_snapshot.TotalProcessCount:n0} processes";
        FilterMatchText = string.IsNullOrWhiteSpace(FilterText) && _quickFilter == ProcessQuickFilter.All
            ? $"{visibleCount:n0} shown"
            : $"{visibleCount:n0} matches";
        OnPropertyChanged(nameof(EmptyStateVisibility));
        OnPropertyChanged(nameof(EmptyStateText));
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
            SortColumn.Attention => "Attention",
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

    private static ProcessQuickFilter ParseQuickFilter(string? mode)
    {
        return Enum.TryParse(mode, ignoreCase: true, out ProcessQuickFilter parsed)
            ? parsed
            : ProcessQuickFilter.All;
    }

    private static string QuickFilterLabel(ProcessQuickFilter mode)
    {
        return mode switch
        {
            ProcessQuickFilter.HighCpu => "High CPU",
            ProcessQuickFilter.HighMemory => "High Memory",
            ProcessQuickFilter.ActiveIo => "Active I/O",
            ProcessQuickFilter.LimitedAccess => "Limited Access",
            _ => "All Processes",
        };
    }

    private static string PassFail(bool passed) => passed ? "passed" : "failed";

    private static BenchmarkSummary? TryLoadLatestBenchmarkSummary()
    {
        DirectoryInfo? directory = new(AppContext.BaseDirectory);
        while (directory is not null)
        {
            if (File.Exists(Path.Combine(directory.FullName, "BatCave.slnx")))
            {
                string benchmarkDirectory = Path.Combine(directory.FullName, "artifacts", "benchmarks");
                if (!Directory.Exists(benchmarkDirectory))
                {
                    return null;
                }

                FileInfo? latest = new DirectoryInfo(benchmarkDirectory)
                    .EnumerateFiles("*.json")
                    .OrderByDescending(static file => file.LastWriteTimeUtc)
                    .FirstOrDefault();
                if (latest is null)
                {
                    return null;
                }

                try
                {
                    return JsonSerializer.Deserialize<BenchmarkSummary>(File.ReadAllText(latest.FullName), JsonDefaults.SnakeCase);
                }
                catch
                {
                    return null;
                }
            }

            directory = directory.Parent;
        }

        return null;
    }
}

internal enum ProcessQuickFilter
{
    All,
    HighCpu,
    HighMemory,
    ActiveIo,
    LimitedAccess,
}
