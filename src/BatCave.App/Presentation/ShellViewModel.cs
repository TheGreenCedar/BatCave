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
    private const int TrendPointCount = 60;

    private readonly IRuntimeStore _runtimeStore;
    private readonly CancellationTokenSource _subscriptionCts = new();
    private readonly Dictionary<ProcessIdentity, ProcessSample> _previousSamplesByIdentity = [];
    private readonly Dictionary<ProcessIdentity, ProcessTrendSet> _processTrendsByIdentity = [];
    private readonly TrendBuffer _systemCpuTrend = new(TrendPointCount);
    private readonly TrendBuffer _systemKernelCpuTrend = new(TrendPointCount);
    private readonly TrendBuffer _systemMemoryTrend = new(TrendPointCount);
    private readonly TrendBuffer _systemDiskReadTrend = new(TrendPointCount);
    private readonly TrendBuffer _systemDiskWriteTrend = new(TrendPointCount);
    private readonly TrendBuffer _systemNetworkTrend = new(TrendPointCount);
    private readonly TrendBuffer _tickP95Trend = new(TrendPointCount);
    private readonly TrendBuffer _sortP95Trend = new(TrendPointCount);
    private readonly TrendBuffer _jitterP95Trend = new(TrendPointCount);
    private readonly List<TrendBuffer> _logicalCpuTrends = [];
    private DispatcherQueue? _dispatcherQueue;
    private Task? _subscriptionTask;
    private CancellationTokenSource? _filterDebounceCts;
    private RuntimeSnapshot _snapshot = new();
    private RuntimeViewState _viewState = new();
    private ProcessRowViewModel? _selectedRow;
    private ProcessQuickFilter _quickFilter = ProcessQuickFilter.All;
    private ShellWorkflowMode _activeWorkflow = ShellWorkflowMode.Overview;
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
    private double[] _cpuTrendValues = [];
    private double[] _memoryTrendValues = [];
    private double[] _systemCpuTrendValues = [];
    private double[] _systemKernelCpuTrendValues = [];
    private double[] _systemMemoryTrendValues = [];
    private double[] _systemDiskReadTrendValues = [];
    private double[] _systemDiskWriteTrendValues = [];
    private double[] _systemNetworkTrendValues = [];
    private double[] _diskTrendValues = [];
    private double[] _otherIoTrendValues = [];
    private double[] _tickP95TrendValues = [];
    private double[] _sortP95TrendValues = [];
    private double[] _jitterP95TrendValues = [];
    private BenchmarkSummary? _latestBenchmarkSummary;

    public ShellViewModel(IRuntimeStore runtimeStore)
    {
        _runtimeStore = runtimeStore;
        _latestBenchmarkSummary = TryLoadLatestBenchmarkSummary();
        ApplySnapshot(_runtimeStore.GetSnapshot());
    }

    public ObservableCollection<ProcessRowViewModel> Rows { get; } = [];

    public ObservableCollection<string> ProcessTimelineItems { get; } = [];

    public ObservableCollection<LogicalCpuChartViewModel> LogicalCpuCharts { get; } = [];

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
                RefreshSelectionProperties();
                RefreshTrendValues();
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

    public ShellWorkflowMode ActiveWorkflow
    {
        get => _activeWorkflow;
        private set
        {
            if (SetProperty(ref _activeWorkflow, value))
            {
                RefreshWorkflowProperties();
            }
        }
    }

    public Visibility ProcessWorkspaceVisibility => ActiveWorkflow is ShellWorkflowMode.Overview or ShellWorkflowMode.Processes
        ? Visibility.Visible
        : Visibility.Collapsed;

    public Visibility InspectorWorkspaceVisibility => ProcessWorkspaceVisibility;

    public Visibility HealthWorkspaceVisibility => ActiveWorkflow == ShellWorkflowMode.Health ? Visibility.Visible : Visibility.Collapsed;

    public Visibility ValidationWorkspaceVisibility => ActiveWorkflow == ShellWorkflowMode.Validation ? Visibility.Visible : Visibility.Collapsed;

    public string WorkflowStatusText => ActiveWorkflow switch
    {
        ShellWorkflowMode.Processes => "Process table and inspector",
        ShellWorkflowMode.Health => "Runtime health and collector confidence",
        ShellWorkflowMode.Validation => "Benchmark and handoff evidence",
        _ => "Attention cockpit and live triage",
    };

    public string TriageHeadlineText => HasWarning
        ? "Runtime needs attention"
        : _snapshot.Health.DegradeMode
            ? "Runtime confidence is degraded"
            : "Attention cockpit";

    public string TriageSummaryText
    {
        get
        {
            int limitedCount = _viewState.Rows.Count(static row => row.AccessState != AccessState.Full);
            int activeIoCount = _viewState.Rows.Count(static row => row.DiskBps + row.OtherIoBps > 0UL);
            return $"{Rows.Count:n0} visible | {limitedCount:n0} limited access | {activeIoCount:n0} active I/O";
        }
    }

    public string TriageAttentionTitle => FormatTriageTitle(TopAttentionProcess(), "No attention target");

    public string TriageAttentionDetail => FormatTriageDetail(TopAttentionProcess(), "Sort by Attention after the runtime warms up.");

    public string TriageCpuTitle => FormatTriageTitle(TopCpuProcess(), "No CPU spike");

    public string TriageCpuDetail => FormatTriageDetail(TopCpuProcess(), "High CPU processes will surface here.");

    public string TriageMemoryTitle => FormatTriageTitle(TopMemoryProcess(), "No memory leader");

    public string TriageMemoryDetail => FormatTriageDetail(TopMemoryProcess(), "Largest working set appears here.");

    public string TriageIoAccessTitle => FormatTriageTitle(TopIoOrLimitedProcess(), "I/O and access quiet");

    public string TriageIoAccessDetail => FormatTriageDetail(TopIoOrLimitedProcess(), "Active I/O or limited access appears here.");

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

    public double[] SystemCpuTrendValues
    {
        get => _systemCpuTrendValues;
        private set => SetProperty(ref _systemCpuTrendValues, value);
    }

    public double[] SystemKernelCpuTrendValues
    {
        get => _systemKernelCpuTrendValues;
        private set => SetProperty(ref _systemKernelCpuTrendValues, value);
    }

    public double[] SystemMemoryTrendValues
    {
        get => _systemMemoryTrendValues;
        private set => SetProperty(ref _systemMemoryTrendValues, value);
    }

    public double[] SystemDiskReadTrendValues
    {
        get => _systemDiskReadTrendValues;
        private set => SetProperty(ref _systemDiskReadTrendValues, value);
    }

    public double[] SystemDiskWriteTrendValues
    {
        get => _systemDiskWriteTrendValues;
        private set => SetProperty(ref _systemDiskWriteTrendValues, value);
    }

    public double[] SystemNetworkTrendValues
    {
        get => _systemNetworkTrendValues;
        private set => SetProperty(ref _systemNetworkTrendValues, value);
    }

    public double[] DiskTrendValues
    {
        get => _diskTrendValues;
        private set => SetProperty(ref _diskTrendValues, value);
    }

    public double[] OtherIoTrendValues
    {
        get => _otherIoTrendValues;
        private set => SetProperty(ref _otherIoTrendValues, value);
    }

    public double[] TickP95TrendValues
    {
        get => _tickP95TrendValues;
        private set => SetProperty(ref _tickP95TrendValues, value);
    }

    public double[] SortP95TrendValues
    {
        get => _sortP95TrendValues;
        private set => SetProperty(ref _sortP95TrendValues, value);
    }

    public double[] JitterP95TrendValues
    {
        get => _jitterP95TrendValues;
        private set => SetProperty(ref _jitterP95TrendValues, value);
    }

    public string SystemCpuChartText => $"CPU {CpuText}";

    public string SystemKernelCpuChartText => $"Kernel {FormatNullablePercent(_snapshot.System.KernelCpuPct)}";

    public string SystemMemoryChartText => $"Memory {MemoryText}";

    public string SystemDiskReadChartText => $"Read {FormatNullableRate(_snapshot.System.DiskReadBps)}";

    public string SystemDiskWriteChartText => $"Write {FormatNullableRate(_snapshot.System.DiskWriteBps)}";

    public string SystemNetworkChartText => $"Network {FormatNetworkRate(_snapshot.System)}";

    public string LogicalCpuChartText => LogicalCpuCharts.Count == 0
        ? $"Logical processors: {_snapshot.System.LogicalProcessorCount:n0}"
        : $"{LogicalCpuCharts.Count:n0} logical processors";

    public string TickP95ChartText => $"tick p95 {TickText}";

    public string SortP95ChartText => $"sort p95 {SortText}";

    public string JitterP95ChartText => $"jitter p95 {_snapshot.Health.JitterP95Ms:0.0} ms";

    public string TrendScopeText => SelectedRow is null
        ? "System overview trend"
        : $"Selected process trend: {SelectedRow.Name} (PID {SelectedRow.PidText})";

    public string CpuTrendLabelText => SelectedRow is null ? "SYSTEM CPU TREND" : "PROCESS CPU TREND";

    public string MemoryTrendLabelText => SelectedRow is null ? "SYSTEM MEMORY TREND" : "PROCESS MEMORY TREND";

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

    public string ProcessStoryText => SelectedRow is null
        ? $"System view is tracking {_snapshot.TotalProcessCount:n0} processes with {Rows.Count:n0} currently visible."
        : $"{SelectedRow.AttentionSummaryText} {SelectedRow.LastMeaningfulChangeText} Access: {SelectedRow.AccessStateText}.";

    public string HealthHeadlineText => RuntimeActionTitle;

    public string HealthSummaryText => RuntimeActionDetail;

    public string HealthDiagnosticsText =>
        $"{RuntimePerfText} | {RuntimeBudgetText} | loop {(_snapshot.Health.RuntimeLoopRunning ? "running" : "stopped")}";

    public string AccessSummaryText => AdminModeRequested
        ? AdminModeEnabled ? "Elevated access is active." : "Admin mode is requested but elevated access is inactive."
        : "Standard access mode is active.";

    public string LocalDataSummaryText => "Settings, warm cache, benchmark artifacts, and logs stay local under the BatCave workspace and LocalAppData paths.";

    public string RuntimeActionTitle
    {
        get
        {
            if (!string.IsNullOrWhiteSpace(WarningText))
            {
                return "Investigate collector warning";
            }

            if (AdminModeRequested && !AdminModeEnabled)
            {
                return "Retry elevated visibility";
            }

            if (_snapshot.Health.DegradeMode)
            {
                return "Run validation before trusting trends";
            }

            return IsPaused ? "Resume live telemetry" : "Runtime is healthy";
        }
    }

    public string RuntimeActionDetail
    {
        get
        {
            if (!string.IsNullOrWhiteSpace(WarningText))
            {
                return WarningText;
            }

            if (AdminModeRequested && !AdminModeEnabled)
            {
                return "Admin mode was requested but the effective collector is standard access.";
            }

            if (_snapshot.Health.DegradeMode)
            {
                return RuntimePerfText;
            }

            return IsPaused
                ? "The runtime loop is paused; resume when you want fresh samples."
                : "No warnings, degrade mode, or admin fallback are active.";
        }
    }

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

    public string BenchmarkEvidenceText => _latestBenchmarkSummary is null
        ? "Run the core benchmark to create a local artifact before claiming performance health."
        : $"Latest local artifact says budget {PassFail(_latestBenchmarkSummary.BudgetPassed)} and strict gate {PassFail(_latestBenchmarkSummary.StrictPassed)}.";

    public string ValidationHeadlineText => _latestBenchmarkSummary is null
        ? "No benchmark evidence yet"
        : $"{_latestBenchmarkSummary.Host} benchmark evidence is available";

    public string ValidationSummaryText => _latestBenchmarkSummary is null
        ? "Run the benchmark command, then rerun validation before treating the app as ready."
        : $"{BenchmarkStatusText} {BenchmarkBudgetText}";

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
        RefreshCockpitProperties();
    }

    [RelayCommand(CanExecute = nameof(HasSelectedRow))]
    private void ClearSelection()
    {
        SelectedRow = null;
    }

    public void SelectWorkflow(string? mode)
    {
        ActiveWorkflow = Enum.TryParse(mode, ignoreCase: true, out ShellWorkflowMode parsed)
            ? parsed
            : ShellWorkflowMode.Overview;
    }

    public void FilterToSelectedProcess()
    {
        if (SelectedRow is null)
        {
            return;
        }

        _quickFilter = ProcessQuickFilter.All;
        OnPropertyChanged(nameof(ActiveQuickFilterText));
        FilterText = SelectedRow.Name;
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

    private void RefreshWorkflowProperties()
    {
        OnPropertyChanged(nameof(ProcessWorkspaceVisibility));
        OnPropertyChanged(nameof(InspectorWorkspaceVisibility));
        OnPropertyChanged(nameof(HealthWorkspaceVisibility));
        OnPropertyChanged(nameof(ValidationWorkspaceVisibility));
        OnPropertyChanged(nameof(WorkflowStatusText));
    }

    private void RefreshSelectionProperties()
    {
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
        OnPropertyChanged(nameof(ProcessStoryText));
        OnPropertyChanged(nameof(CopyDetailsText));
        OnPropertyChanged(nameof(HasSelectedRow));
        OnPropertyChanged(nameof(ClearSelectionVisibility));
        OnPropertyChanged(nameof(CopyDetailsVisibility));
        OnPropertyChanged(nameof(TrendScopeText));
        OnPropertyChanged(nameof(CpuTrendLabelText));
        OnPropertyChanged(nameof(MemoryTrendLabelText));
        OnPropertyChanged(nameof(SystemCpuChartText));
        OnPropertyChanged(nameof(SystemKernelCpuChartText));
        OnPropertyChanged(nameof(SystemMemoryChartText));
        OnPropertyChanged(nameof(SystemDiskReadChartText));
        OnPropertyChanged(nameof(SystemDiskWriteChartText));
        OnPropertyChanged(nameof(SystemNetworkChartText));
        OnPropertyChanged(nameof(LogicalCpuChartText));
        OnPropertyChanged(nameof(TickP95ChartText));
        OnPropertyChanged(nameof(SortP95ChartText));
        OnPropertyChanged(nameof(JitterP95ChartText));
        RefreshTimelineItems();
        ClearSelectionCommand.NotifyCanExecuteChanged();
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
        OnPropertyChanged(nameof(SystemCpuChartText));
        OnPropertyChanged(nameof(SystemKernelCpuChartText));
        OnPropertyChanged(nameof(SystemMemoryChartText));
        OnPropertyChanged(nameof(SystemDiskReadChartText));
        OnPropertyChanged(nameof(SystemDiskWriteChartText));
        OnPropertyChanged(nameof(SystemNetworkChartText));
        OnPropertyChanged(nameof(LogicalCpuChartText));
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
        AppendTrend(snapshot, viewState.Rows);
        RefreshTrendValues();
        RefreshProcessTextProperties();
        RefreshSelectionProperties();
        RefreshCockpitProperties();
        RefreshWorkflowProperties();
    }

    private void ApplyProcessRows(IReadOnlyList<ProcessSample> rows, ProcessIdentity? selectedIdentity)
    {
        IReadOnlyList<ProcessSample> visibleRows = ApplyQuickFilter(rows);
        Dictionary<ProcessIdentity, ProcessRowViewModel> existing = Rows.ToDictionary(static row => row.Identity);
        List<ProcessRowViewModel> desiredRows = new(visibleRows.Count);
        for (int index = 0; index < visibleRows.Count; index++)
        {
            ProcessSample sample = visibleRows[index];
            ProcessIdentity identity = sample.Identity();
            _previousSamplesByIdentity.TryGetValue(identity, out ProcessSample? previousSample);
            bool isNew = previousSample is null && SnapshotSeq > 0;
            ProcessRowViewModel viewRow;
            if (existing.TryGetValue(identity, out ProcessRowViewModel? current))
            {
                bool hasSameDisplayState = current.HasSameDisplayState(sample);
                if (hasSameDisplayState)
                {
                    current.UpdateSample(sample);
                }
                else
                {
                    current.Update(sample, previousSample, isNew);
                }

                viewRow = current;
            }
            else
            {
                viewRow = new ProcessRowViewModel(sample, previousSample, isNew);
            }

            desiredRows.Add(viewRow);
        }

        SyncRows(desiredRows);

        foreach (ProcessSample sample in rows)
        {
            _previousSamplesByIdentity[sample.Identity()] = sample;
        }

        ProcessRowViewModel? previousSelectedRow = SelectedRow;
        ProcessRowViewModel? nextSelectedRow = selectedIdentity.HasValue
            ? Rows.FirstOrDefault(row => row.Identity.Equals(selectedIdentity.Value))
            : null;
        SelectedRow = nextSelectedRow;
        if (ReferenceEquals(previousSelectedRow, nextSelectedRow) && nextSelectedRow is not null)
        {
            _viewState = _viewState with
            {
                SelectedIdentity = nextSelectedRow.Identity,
                SelectedProcess = nextSelectedRow.Sample,
            };
        }
    }

    private void SyncRows(IReadOnlyList<ProcessRowViewModel> desiredRows)
    {
        for (int targetIndex = 0; targetIndex < desiredRows.Count; targetIndex++)
        {
            ProcessRowViewModel desiredRow = desiredRows[targetIndex];
            if (targetIndex < Rows.Count && ReferenceEquals(Rows[targetIndex], desiredRow))
            {
                continue;
            }

            int currentIndex = Rows.IndexOf(desiredRow);
            if (currentIndex >= 0)
            {
                Rows.Move(currentIndex, targetIndex);
            }
            else
            {
                Rows.Insert(targetIndex, desiredRow);
            }
        }

        while (Rows.Count > desiredRows.Count)
        {
            Rows.RemoveAt(Rows.Count - 1);
        }
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

    private void RefreshCockpitProperties()
    {
        OnPropertyChanged(nameof(TriageHeadlineText));
        OnPropertyChanged(nameof(TriageSummaryText));
        OnPropertyChanged(nameof(TriageAttentionTitle));
        OnPropertyChanged(nameof(TriageAttentionDetail));
        OnPropertyChanged(nameof(TriageCpuTitle));
        OnPropertyChanged(nameof(TriageCpuDetail));
        OnPropertyChanged(nameof(TriageMemoryTitle));
        OnPropertyChanged(nameof(TriageMemoryDetail));
        OnPropertyChanged(nameof(TriageIoAccessTitle));
        OnPropertyChanged(nameof(TriageIoAccessDetail));
        OnPropertyChanged(nameof(RuntimeActionTitle));
        OnPropertyChanged(nameof(RuntimeActionDetail));
        OnPropertyChanged(nameof(HealthHeadlineText));
        OnPropertyChanged(nameof(HealthSummaryText));
        OnPropertyChanged(nameof(HealthDiagnosticsText));
        OnPropertyChanged(nameof(AccessSummaryText));
        OnPropertyChanged(nameof(LocalDataSummaryText));
        OnPropertyChanged(nameof(BenchmarkEvidenceText));
        OnPropertyChanged(nameof(ValidationHeadlineText));
        OnPropertyChanged(nameof(ValidationSummaryText));
    }

    private void RefreshTimelineItems()
    {
        ProcessTimelineItems.Clear();
        if (SelectedRow is { } selectedRow)
        {
            ProcessTimelineItems.Add(selectedRow.AttentionSummaryText);
            ProcessTimelineItems.Add(selectedRow.LastMeaningfulChangeText);
            ProcessTimelineItems.Add($"Started {selectedRow.StartTimeText}; parent PID {selectedRow.ParentPidText}.");
            ProcessTimelineItems.Add($"Access {selectedRow.AccessStateText}; handles {selectedRow.HandlesText}; threads {selectedRow.ThreadsText}.");
            ProcessTimelineItems.Add($"Current load: {selectedRow.CpuText} CPU, {selectedRow.MemoryText} memory, {selectedRow.DiskText} disk, {selectedRow.OtherIoText} other I/O.");
            return;
        }

        ProcessTimelineItems.Add($"Tracking {_snapshot.TotalProcessCount:n0} processes with {Rows.Count:n0} visible in the current view.");
        ProcessTimelineItems.Add(RuntimeConfidenceText);
        ProcessTimelineItems.Add(RuntimePerfText);
        ProcessTimelineItems.Add($"System load: CPU {CpuText}, memory {MemoryText}, disk {FormatSystemDisk(_snapshot.System)}, other I/O {FormatNullableRate(_snapshot.System.OtherIoBps)}.");
        ProcessTimelineItems.Add(AccessSummaryText);
    }

    private void AppendTrend(RuntimeSnapshot snapshot, IReadOnlyList<ProcessSample> rows)
    {
        _systemCpuTrend.Append(snapshot.System.CpuPct.GetValueOrDefault());
        _systemKernelCpuTrend.Append(snapshot.System.KernelCpuPct.GetValueOrDefault());
        ulong totalMemoryBytes = snapshot.System.MemoryTotalBytes.GetValueOrDefault();
        _systemMemoryTrend.Append(totalMemoryBytes == 0
            ? 0
            : snapshot.System.MemoryUsedBytes.GetValueOrDefault() * 100d / totalMemoryBytes);
        _systemDiskReadTrend.Append(snapshot.System.DiskReadBps.GetValueOrDefault());
        _systemDiskWriteTrend.Append(snapshot.System.DiskWriteBps.GetValueOrDefault());
        _systemNetworkTrend.Append(ResolveNetworkBytes(snapshot.System));
        SyncLogicalCpuTrends(snapshot.System.LogicalCpuPct);
        _tickP95Trend.Append(snapshot.Health.TickP95Ms);
        _sortP95Trend.Append(snapshot.Health.SortP95Ms);
        _jitterP95Trend.Append(snapshot.Health.JitterP95Ms);

        HashSet<ProcessIdentity> activeIdentities = [];
        foreach (ProcessSample sample in rows)
        {
            ProcessIdentity identity = sample.Identity();
            activeIdentities.Add(identity);
            if (!_processTrendsByIdentity.TryGetValue(identity, out ProcessTrendSet? trends))
            {
                trends = new ProcessTrendSet(TrendPointCount);
                _processTrendsByIdentity[identity] = trends;
            }

            trends.Cpu.Append(sample.CpuPct);
            trends.Memory.Append((double)sample.MemoryBytes);
            trends.Disk.Append(sample.DiskBps);
            trends.OtherIo.Append(sample.OtherIoBps);
        }

        foreach (ProcessIdentity identity in _processTrendsByIdentity.Keys.ToArray())
        {
            if (!activeIdentities.Contains(identity))
            {
                _processTrendsByIdentity.Remove(identity);
            }
        }
    }

    private void RefreshTrendValues()
    {
        SystemCpuTrendValues = _systemCpuTrend.ToArray();
        SystemKernelCpuTrendValues = _systemKernelCpuTrend.ToArray();
        SystemMemoryTrendValues = _systemMemoryTrend.ToArray();
        SystemDiskReadTrendValues = _systemDiskReadTrend.ToArray();
        SystemDiskWriteTrendValues = _systemDiskWriteTrend.ToArray();
        SystemNetworkTrendValues = _systemNetworkTrend.ToArray();
        RefreshLogicalCpuChartValues();
        TickP95TrendValues = _tickP95Trend.ToArray();
        SortP95TrendValues = _sortP95Trend.ToArray();
        JitterP95TrendValues = _jitterP95Trend.ToArray();

        if (SelectedRow is { } selectedRow)
        {
            if (_processTrendsByIdentity.TryGetValue(selectedRow.Identity, out ProcessTrendSet? trends))
            {
                CpuTrendValues = trends.Cpu.ToArray();
                MemoryTrendValues = trends.Memory.ToArray();
                DiskTrendValues = trends.Disk.ToArray();
                OtherIoTrendValues = trends.OtherIo.ToArray();
            }
            else
            {
                CpuTrendValues = [selectedRow.Sample.CpuPct];
                MemoryTrendValues = [(double)selectedRow.Sample.MemoryBytes];
                DiskTrendValues = [selectedRow.Sample.DiskBps];
                OtherIoTrendValues = [selectedRow.Sample.OtherIoBps];
            }
        }
        else
        {
            CpuTrendValues = _systemCpuTrend.ToArray();
            MemoryTrendValues = _systemMemoryTrend.ToArray();
            DiskTrendValues = _systemDiskReadTrend
                .ToArray()
                .Zip(_systemDiskWriteTrend.ToArray(), static (read, write) => read + write)
                .ToArray();
            OtherIoTrendValues = _systemNetworkTrend.ToArray();
        }

        OnPropertyChanged(nameof(TrendScopeText));
        OnPropertyChanged(nameof(CpuTrendLabelText));
        OnPropertyChanged(nameof(MemoryTrendLabelText));
    }

    private void SyncLogicalCpuTrends(IReadOnlyList<double> logicalCpuPct)
    {
        int count = logicalCpuPct.Count;
        while (_logicalCpuTrends.Count < count)
        {
            _logicalCpuTrends.Add(new TrendBuffer(TrendPointCount));
        }

        while (_logicalCpuTrends.Count > count)
        {
            _logicalCpuTrends.RemoveAt(_logicalCpuTrends.Count - 1);
        }

        while (LogicalCpuCharts.Count < count)
        {
            int index = LogicalCpuCharts.Count;
            LogicalCpuCharts.Add(new LogicalCpuChartViewModel($"CPU {index}", []));
        }

        while (LogicalCpuCharts.Count > count)
        {
            LogicalCpuCharts.RemoveAt(LogicalCpuCharts.Count - 1);
        }

        for (int index = 0; index < count; index++)
        {
            _logicalCpuTrends[index].Append(logicalCpuPct[index]);
        }
    }

    private void RefreshLogicalCpuChartValues()
    {
        for (int index = 0; index < LogicalCpuCharts.Count && index < _logicalCpuTrends.Count; index++)
        {
            LogicalCpuCharts[index].Values = _logicalCpuTrends[index].ToArray();
        }
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

    private static string FormatNullablePercent(double? value)
    {
        return value.HasValue ? $"{value.Value:0.0}%" : "warming";
    }

    private static string FormatNetworkRate(SystemMetricsSnapshot snapshot)
    {
        ulong bytesPerSecond = ResolveNetworkBytes(snapshot);
        if (bytesPerSecond == 0UL && !snapshot.NetworkBytesBps.HasValue && !snapshot.OtherIoBps.HasValue)
        {
            return "n/a";
        }

        return FormatBitsRateFromBytes(bytesPerSecond);
    }

    private static ulong ResolveNetworkBytes(SystemMetricsSnapshot snapshot)
    {
        return snapshot.NetworkBytesBps ?? snapshot.OtherIoBps ?? 0UL;
    }

    private static string FormatBitsRateFromBytes(ulong bytesPerSecond)
    {
        double bitsPerSecond = bytesPerSecond * 8d;
        string[] units = ["bps", "Kbps", "Mbps", "Gbps", "Tbps"];
        int unit = 0;
        while (bitsPerSecond >= 1000d && unit < units.Length - 1)
        {
            bitsPerSecond /= 1000d;
            unit++;
        }

        return bitsPerSecond < 10d && unit > 0
            ? $"{bitsPerSecond:0.0} {units[unit]}"
            : $"{bitsPerSecond:0} {units[unit]}";
    }

    private ProcessSample? TopAttentionProcess()
    {
        return _viewState.Rows
            .OrderByDescending(ProcessAttention.Score)
            .ThenBy(static row => row.Name, StringComparer.OrdinalIgnoreCase)
            .FirstOrDefault();
    }

    private ProcessSample? TopCpuProcess()
    {
        return _viewState.Rows
            .OrderByDescending(static row => row.CpuPct)
            .ThenBy(static row => row.Name, StringComparer.OrdinalIgnoreCase)
            .FirstOrDefault();
    }

    private ProcessSample? TopMemoryProcess()
    {
        return _viewState.Rows
            .OrderByDescending(static row => row.MemoryBytes)
            .ThenBy(static row => row.Name, StringComparer.OrdinalIgnoreCase)
            .FirstOrDefault();
    }

    private ProcessSample? TopIoOrLimitedProcess()
    {
        return _viewState.Rows
            .OrderByDescending(static row => row.AccessState != AccessState.Full)
            .ThenByDescending(static row => row.DiskBps + row.OtherIoBps)
            .ThenBy(static row => row.Name, StringComparer.OrdinalIgnoreCase)
            .FirstOrDefault();
    }

    private static string FormatTriageTitle(ProcessSample? sample, string fallback)
    {
        return sample is null ? fallback : $"{sample.Name} ({sample.Pid})";
    }

    private static string FormatTriageDetail(ProcessSample? sample, string fallback)
    {
        if (sample is null)
        {
            return fallback;
        }

        ulong ioBytes = ulong.MaxValue - sample.DiskBps < sample.OtherIoBps
            ? ulong.MaxValue
            : sample.DiskBps + sample.OtherIoBps;
        return $"{sample.CpuPct:0.0}% CPU | {ProcessRowViewModel.FormatBytes(sample.MemoryBytes)} | {ProcessRowViewModel.FormatRate(ioBytes)} | {ProcessAttention.Label(sample, isNew: false)}";
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

    private sealed class ProcessTrendSet
    {
        public ProcessTrendSet(int capacity)
        {
            Cpu = new TrendBuffer(capacity);
            Memory = new TrendBuffer(capacity);
            Disk = new TrendBuffer(capacity);
            OtherIo = new TrendBuffer(capacity);
        }

        public TrendBuffer Cpu { get; }

        public TrendBuffer Memory { get; }

        public TrendBuffer Disk { get; }

        public TrendBuffer OtherIo { get; }
    }

    private sealed class TrendBuffer
    {
        private readonly int _capacity;
        private readonly Queue<double> _values = [];

        public TrendBuffer(int capacity)
        {
            _capacity = capacity;
        }

        public void Append(double value)
        {
            if (double.IsNaN(value) || double.IsInfinity(value) || value < 0)
            {
                value = 0;
            }

            if (_values.Count == _capacity)
            {
                _values.Dequeue();
            }

            _values.Enqueue(value);
        }

        public double[] ToArray() => _values.ToArray();
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

public enum ShellWorkflowMode
{
    Overview,
    Processes,
    Health,
    Validation,
}

public sealed partial class LogicalCpuChartViewModel(string title, double[] values) : ObservableObject
{
    private double[] _values = values;

    public string Title { get; } = title;

    public double[] Values
    {
        get => _values;
        set => SetProperty(ref _values, value);
    }
}
