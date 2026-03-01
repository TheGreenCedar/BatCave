using System;
using System.Collections.Generic;
using System.Collections.ObjectModel;
using System.Linq;
using System.Threading;
using System.Threading.Tasks;
using BatCave.Charts;
using BatCave.Converters;
using BatCave.Core.Abstractions;
using BatCave.Core.Domain;
using BatCave.Core.Runtime;
using BatCave.Services;
using CommunityToolkit.Mvvm.ComponentModel;
using CommunityToolkit.Mvvm.Input;
using Microsoft.UI.Dispatching;
using Microsoft.UI.Xaml;
using AdvancedCollectionView = CommunityToolkit.WinUI.Collections.AdvancedCollectionView;
using SortDescription = CommunityToolkit.WinUI.Collections.SortDescription;

namespace BatCave.ViewModels;

public enum DetailMetricFocus
{
    Cpu,
    Memory,
    IoRead,
    IoWrite,
    Network,
}

public partial class MonitoringShellViewModel : ObservableObject
{
    private const int FilterDebounceMs = 160;
    private const int HistoryLimit = 120;
    private const double RowSparklineWidth = 96;
    private const double RowSparklineHeight = 22;

    private readonly ILaunchPolicyGate _launchPolicyGate;
    private readonly MonitoringRuntime _runtime;
    private readonly RuntimeLoopService _runtimeLoopService;
    private readonly IRuntimeEventGateway _runtimeEventGateway;
    private readonly IProcessMetadataProvider _metadataProvider;

    private readonly Dictionary<ProcessIdentity, ProcessSample> _allRows = new();
    private readonly Dictionary<ProcessIdentity, ProcessMetadata?> _metadataCache = new();
    private readonly Dictionary<ProcessIdentity, MetricHistoryBuffer> _metricHistory = new();
    private readonly Dictionary<ProcessIdentity, ProcessRowViewState> _visibleRowStateByIdentity = new();
    private readonly ObservableCollection<ProcessRowViewState> _rowViewSource = [];
    private readonly MetricHistoryBuffer _globalHistory = new(HistoryLimit);

    private DispatcherQueue? _dispatcherQueue;
    private CancellationTokenSource? _filterDebounceCts;

    private bool _isLoading = true;
    private bool _isBlocked;
    private bool _isStartupError;
    private bool _isLive;
    private string _shellHeadline = "Initializing monitor runtime...";
    private string _shellBody = "Starting monitoring services.";
    private string _blockedReasonMessage = string.Empty;
    private string _startupErrorMessage = string.Empty;
    private string _filterText = string.Empty;
    private SortColumn _currentSortColumn = SortColumn.CpuPct;
    private SortDirection _currentSortDirection = SortDirection.Desc;
    private bool _adminModeEnabled;
    private bool _adminEnabledOnlyFilter;
    private bool _adminModePending;
    private string? _adminModeError;
    private string _runtimeHealthStatus = "Runtime health unavailable.";
    private StartupGateStatus _startupGateStatus = new();
    private ProcessSample? _selectedRow;
    private ProcessRowViewState? _selectedVisibleRow;
    private ProcessMetadata? _selectedMetadata;
    private bool _isApplyingSelectedVisibleRowBinding;
    private bool _isMetadataLoading;
    private string? _metadataError;
    private DetailMetricFocus _metricFocus = DetailMetricFocus.Cpu;
    private long _metadataRequestVersion;

    private ulong _summarySeq;
    private ulong _summaryTsMs;
    private double _summaryCpuPct;
    private double _summaryRssBytes;
    private double _summaryPrivateBytes;
    private double _summaryIoReadBps;
    private double _summaryIoWriteBps;
    private double _summaryNetBps;
    private double _summaryThreads;
    private double _summaryHandles;
    private ProcessSample _globalSummaryRow = CreateEmptyGlobalSummary();

    private double[] _cpuMetricTrendValues = [];
    private double[] _memoryMetricTrendValues = [];
    private double[] _ioReadMetricTrendValues = [];
    private double[] _ioWriteMetricTrendValues = [];
    private double[] _networkMetricTrendValues = [];
    private double[] _expandedMetricTrendValues = [];

    private string _cpuMetricChipValue = "0.00%";
    private string _memoryMetricChipValue = "0 B";
    private string _ioReadMetricChipValue = "0 B/s";
    private string _ioWriteMetricChipValue = "0 B/s";
    private string _networkMetricChipValue = "0 B/s";
    private string _expandedMetricTitle = "CPU Trend";
    private string _expandedMetricValue = "0.0% CPU";

    public MonitoringShellViewModel(
        ILaunchPolicyGate launchPolicyGate,
        MonitoringRuntime runtime,
        RuntimeLoopService runtimeLoopService,
        IRuntimeEventGateway runtimeEventGateway,
        IProcessMetadataProvider metadataProvider)
    {
        _launchPolicyGate = launchPolicyGate;
        _runtime = runtime;
        _runtimeLoopService = runtimeLoopService;
        _runtimeEventGateway = runtimeEventGateway;
        _metadataProvider = metadataProvider;

        _runtimeEventGateway.TelemetryDelta += OnTelemetryDelta;
        _runtimeEventGateway.RuntimeHealthChanged += OnRuntimeHealthChanged;
        _runtimeEventGateway.CollectorWarningRaised += OnCollectorWarningRaised;

        VisibleRows = new AdvancedCollectionView(_rowViewSource, true);
        VisibleRows.Filter = ShouldShowRow;
        ApplySortDescriptions();
    }

    public AdvancedCollectionView VisibleRows { get; }

    public bool IsLoading
    {
        get => _isLoading;
        private set
        {
            if (SetProperty(ref _isLoading, value))
            {
                RaiseStateVisibilityProperties();
            }
        }
    }

    public bool IsBlocked
    {
        get => _isBlocked;
        private set
        {
            if (SetProperty(ref _isBlocked, value))
            {
                RaiseStateVisibilityProperties();
            }
        }
    }

    public bool IsStartupError
    {
        get => _isStartupError;
        private set
        {
            if (SetProperty(ref _isStartupError, value))
            {
                RaiseStateVisibilityProperties();
            }
        }
    }

    public bool IsLive
    {
        get => _isLive;
        private set
        {
            if (SetProperty(ref _isLive, value))
            {
                RaiseStateVisibilityProperties();
            }
        }
    }

    public string ShellHeadline
    {
        get => _shellHeadline;
        private set => SetProperty(ref _shellHeadline, value);
    }

    public string ShellBody
    {
        get => _shellBody;
        private set => SetProperty(ref _shellBody, value);
    }

    public string BlockedReasonMessage
    {
        get => _blockedReasonMessage;
        private set => SetProperty(ref _blockedReasonMessage, value);
    }

    public string StartupErrorMessage
    {
        get => _startupErrorMessage;
        private set => SetProperty(ref _startupErrorMessage, value);
    }

    public string FilterText
    {
        get => _filterText;
        set
        {
            if (SetProperty(ref _filterText, value))
            {
                ScheduleFilterApply(value);
            }
        }
    }

    public SortColumn CurrentSortColumn
    {
        get => _currentSortColumn;
        private set
        {
            if (SetProperty(ref _currentSortColumn, value))
            {
                RaiseSortHeaderLabels();
            }
        }
    }

    public SortDirection CurrentSortDirection
    {
        get => _currentSortDirection;
        private set
        {
            if (SetProperty(ref _currentSortDirection, value))
            {
                RaiseSortHeaderLabels();
            }
        }
    }

    public bool AdminModeEnabled
    {
        get => _adminModeEnabled;
        private set
        {
            if (SetProperty(ref _adminModeEnabled, value))
            {
                if (!value)
                {
                    AdminEnabledOnlyFilter = false;
                }

                RefreshVisibleRows();
            }
        }
    }

    public bool AdminEnabledOnlyFilter
    {
        get => _adminEnabledOnlyFilter;
        set
        {
            if (!AdminModeEnabled && value)
            {
                value = false;
            }

            if (SetProperty(ref _adminEnabledOnlyFilter, value))
            {
                RefreshVisibleRows();
            }
        }
    }

    public bool AdminModePending
    {
        get => _adminModePending;
        private set => SetProperty(ref _adminModePending, value);
    }

    public string? AdminModeError
    {
        get => _adminModeError;
        private set
        {
            if (SetProperty(ref _adminModeError, value))
            {
                OnPropertyChanged(nameof(HasAdminModeError));
                OnPropertyChanged(nameof(AdminErrorVisibility));
            }
        }
    }

    public bool HasAdminModeError => !string.IsNullOrWhiteSpace(AdminModeError);

    public Visibility LoadingVisibility => IsLoading ? Visibility.Visible : Visibility.Collapsed;

    public Visibility BlockedVisibility => IsBlocked ? Visibility.Visible : Visibility.Collapsed;

    public Visibility StartupErrorVisibility => IsStartupError ? Visibility.Visible : Visibility.Collapsed;

    public Visibility LiveVisibility => IsLive ? Visibility.Visible : Visibility.Collapsed;

    public Visibility AdminErrorVisibility => HasAdminModeError ? Visibility.Visible : Visibility.Collapsed;

    public string RuntimeHealthStatus
    {
        get => _runtimeHealthStatus;
        private set => SetProperty(ref _runtimeHealthStatus, value);
    }

    public StartupGateStatus StartupGateStatus
    {
        get => _startupGateStatus;
        private set => SetProperty(ref _startupGateStatus, value);
    }

    public ProcessSample? SelectedRow
    {
        get => _selectedRow;
        private set
        {
            if (SetProperty(ref _selectedRow, value))
            {
                OnPropertyChanged(nameof(HasSelection));
                OnPropertyChanged(nameof(DetailTitle));
                RaiseMetadataProperties();
                RefreshDetailMetrics();
            }
        }
    }

    public ProcessRowViewState? SelectedVisibleRow
    {
        get => _selectedVisibleRow;
        private set
        {
            if (SetProperty(ref _selectedVisibleRow, value))
            {
                OnPropertyChanged(nameof(SelectedVisibleRowBinding));
            }
        }
    }

    public ProcessRowViewState? SelectedVisibleRowBinding
    {
        get => SelectedVisibleRow;
        set => ApplySelectedVisibleRowBinding(value);
    }

    public ProcessMetadata? SelectedMetadata
    {
        get => _selectedMetadata;
        private set
        {
            if (SetProperty(ref _selectedMetadata, value))
            {
                RaiseMetadataProperties();
            }
        }
    }

    public bool IsMetadataLoading
    {
        get => _isMetadataLoading;
        private set
        {
            if (SetProperty(ref _isMetadataLoading, value))
            {
                RaiseMetadataProperties();
            }
        }
    }

    public string? MetadataError
    {
        get => _metadataError;
        private set
        {
            if (SetProperty(ref _metadataError, value))
            {
                RaiseMetadataProperties();
            }
        }
    }

    public DetailMetricFocus MetricFocus
    {
        get => _metricFocus;
        set
        {
            if (SetProperty(ref _metricFocus, value))
            {
                RaiseMetricFocusProperties();
                RefreshDetailMetrics();
            }
        }
    }

    public bool IsCpuMetricFocused => MetricFocus == DetailMetricFocus.Cpu;

    public bool IsMemoryMetricFocused => MetricFocus == DetailMetricFocus.Memory;

    public bool IsIoReadMetricFocused => MetricFocus == DetailMetricFocus.IoRead;

    public bool IsIoWriteMetricFocused => MetricFocus == DetailMetricFocus.IoWrite;

    public bool IsNetworkMetricFocused => MetricFocus == DetailMetricFocus.Network;

    public bool HasSelection => SelectedRow is not null;

    public string DetailTitle =>
        SelectedRow is null
            ? _globalSummaryRow.Name
            : $"{SelectedRow.Name} ({SelectedRow.Pid})";

    public string DetailMetricValue => ExpandedMetricValue;

    public string MetadataStatus
    {
        get
        {
            if (SelectedRow is null)
            {
                return "Select a process to load metadata.";
            }

            if (IsMetadataLoading)
            {
                return "Loading metadata...";
            }

            if (!string.IsNullOrWhiteSpace(MetadataError))
            {
                return $"Metadata error: {MetadataError}";
            }

            if (SelectedMetadata is null)
            {
                return "Metadata unavailable for this process identity.";
            }

            return "Metadata loaded.";
        }
    }

    public string MetadataParentPid =>
        SelectedMetadata is null
            ? "n/a"
            : SelectedMetadata.ParentPid.ToString();

    public string MetadataCommandLine =>
        string.IsNullOrWhiteSpace(SelectedMetadata?.CommandLine)
            ? "n/a"
            : SelectedMetadata.CommandLine!;

    public string MetadataExecutablePath =>
        string.IsNullOrWhiteSpace(SelectedMetadata?.ExecutablePath)
            ? "n/a"
            : SelectedMetadata.ExecutablePath!;

    public string NameSortLabel => SortLabel("Name", SortColumn.Name);

    public string PidSortLabel => SortLabel("PID", SortColumn.Pid);

    public string CpuSortLabel => SortLabel("CPU", SortColumn.CpuPct);

    public string MemorySortLabel => SortLabel("Memory", SortColumn.RssBytes);

    public string IoReadSortLabel => SortLabel("IO Read", SortColumn.IoReadBps);

    public string IoWriteSortLabel => SortLabel("IO Write", SortColumn.IoWriteBps);

    public string NetSortLabel => SortLabel("Net", SortColumn.NetBps);

    public string ThreadsSortLabel => SortLabel("Threads", SortColumn.Threads);

    public string HandlesSortLabel => SortLabel("Handles", SortColumn.Handles);

    public double[] CpuMetricTrendValues
    {
        get => _cpuMetricTrendValues;
        private set => SetProperty(ref _cpuMetricTrendValues, value);
    }

    public double[] MemoryMetricTrendValues
    {
        get => _memoryMetricTrendValues;
        private set => SetProperty(ref _memoryMetricTrendValues, value);
    }

    public double[] IoReadMetricTrendValues
    {
        get => _ioReadMetricTrendValues;
        private set => SetProperty(ref _ioReadMetricTrendValues, value);
    }

    public double[] IoWriteMetricTrendValues
    {
        get => _ioWriteMetricTrendValues;
        private set => SetProperty(ref _ioWriteMetricTrendValues, value);
    }

    public double[] NetworkMetricTrendValues
    {
        get => _networkMetricTrendValues;
        private set => SetProperty(ref _networkMetricTrendValues, value);
    }

    public double[] ExpandedMetricTrendValues
    {
        get => _expandedMetricTrendValues;
        private set => SetProperty(ref _expandedMetricTrendValues, value);
    }

    public string CpuMetricChipValue
    {
        get => _cpuMetricChipValue;
        private set => SetProperty(ref _cpuMetricChipValue, value);
    }

    public string MemoryMetricChipValue
    {
        get => _memoryMetricChipValue;
        private set => SetProperty(ref _memoryMetricChipValue, value);
    }

    public string IoReadMetricChipValue
    {
        get => _ioReadMetricChipValue;
        private set => SetProperty(ref _ioReadMetricChipValue, value);
    }

    public string IoWriteMetricChipValue
    {
        get => _ioWriteMetricChipValue;
        private set => SetProperty(ref _ioWriteMetricChipValue, value);
    }

    public string NetworkMetricChipValue
    {
        get => _networkMetricChipValue;
        private set => SetProperty(ref _networkMetricChipValue, value);
    }

    public string ExpandedMetricTitle
    {
        get => _expandedMetricTitle;
        private set => SetProperty(ref _expandedMetricTitle, value);
    }

    public string ExpandedMetricValue
    {
        get => _expandedMetricValue;
        private set
        {
            if (SetProperty(ref _expandedMetricValue, value))
            {
                OnPropertyChanged(nameof(DetailMetricValue));
            }
        }
    }

    public void AttachDispatcherQueue(DispatcherQueue dispatcherQueue)
    {
        _dispatcherQueue = dispatcherQueue;
    }

    public Task BootstrapAsync(CancellationToken ct)
    {
        IsLoading = true;
        IsStartupError = false;
        IsBlocked = false;
        IsLive = false;
        StartupErrorMessage = string.Empty;
        ShellHeadline = "Initializing monitor runtime...";
        ShellBody = "Starting monitoring services.";

        try
        {
            StartupGateStatus = _launchPolicyGate.Enforce();
            if (!StartupGateStatus.Passed)
            {
                IsBlocked = true;
                BlockedReasonMessage = FormatBlockReason(StartupGateStatus.Reason);
                ShellHeadline = "Startup Blocked";
                ShellBody = BlockedReasonMessage;
                RuntimeHealthStatus = "Runtime health unavailable.";
                return Task.CompletedTask;
            }

            _filterText = _runtime.CurrentFilterText;
            OnPropertyChanged(nameof(FilterText));

            CurrentSortColumn = _runtime.CurrentSortColumn;
            CurrentSortDirection = _runtime.CurrentSortDirection;
            ApplySortDescriptions();
            AdminModeEnabled = _runtime.IsAdminMode();

            QueryResponse snapshot = _runtime.GetSnapshot();
            RuntimeHealth health = _runtime.GetRuntimeHealth();

            LoadSnapshot(snapshot.Rows);
            ApplyRuntimeHealth(health);

            IsLive = true;
            BlockedReasonMessage = string.Empty;
            ShellHeadline = "Live monitor shell ready";
            ShellBody = "Runtime loop and admin restart paths are active.";
        }
        catch (Exception ex)
        {
            IsStartupError = true;
            StartupErrorMessage = ex.Message;
            ShellHeadline = "Startup Incomplete";
            ShellBody = ex.Message;
        }
        finally
        {
            IsLoading = false;
        }

        return Task.CompletedTask;
    }

    public Task RetryBootstrapAsync(CancellationToken ct)
    {
        return BootstrapAsync(ct);
    }

    public async Task ToggleAdminModeAsync(bool nextAdminMode, CancellationToken ct)
    {
        if (AdminModePending || nextAdminMode == AdminModeEnabled)
        {
            return;
        }

        AdminModePending = true;
        AdminModeError = null;

        try
        {
            _runtimeLoopService.StopAndAdvanceGeneration();
            await _runtime.RestartAsync(nextAdminMode, ct);
            _runtimeLoopService.Start(_runtimeLoopService.CurrentGeneration);

            AdminModeEnabled = _runtime.IsAdminMode();

            QueryResponse snapshot = _runtime.GetSnapshot();
            RuntimeHealth health = _runtime.GetRuntimeHealth();
            LoadSnapshot(snapshot.Rows);
            ApplyRuntimeHealth(health);
        }
        catch (Exception ex)
        {
            AdminModeError = ex.Message;
            AdminModeEnabled = _runtime.IsAdminMode();
        }
        finally
        {
            AdminModePending = false;
        }
    }

    public void ChangeSort(SortColumn column)
    {
        SortDirection nextDirection = CurrentSortColumn == column && CurrentSortDirection == SortDirection.Desc
            ? SortDirection.Asc
            : SortDirection.Desc;

        CurrentSortColumn = column;
        CurrentSortDirection = nextDirection;

        _runtime.SetSort(CurrentSortColumn, CurrentSortDirection);
        ApplySortDescriptions();
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

    [RelayCommand]
    private void MetricFocusSelected(string? focusTag)
    {
        if (!Enum.TryParse(focusTag, out DetailMetricFocus focus))
        {
            return;
        }

        MetricFocus = focus;
    }

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

            RunOnUiThread(() =>
            {
                if (!IsCurrentMetadataRequest(requestVersion, identity))
                {
                    return;
                }

                _metadataCache[identity] = metadata;
                SelectedMetadata = metadata;
                MetadataError = null;
                IsMetadataLoading = false;
            });
        }
        catch (Exception ex)
        {
            RunOnUiThread(() =>
            {
                if (!IsCurrentMetadataRequest(requestVersion, identity))
                {
                    return;
                }

                MetadataError = ex.Message;
                SelectedMetadata = null;
                IsMetadataLoading = false;
            });
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
        if (_isApplyingSelectedVisibleRowBinding)
        {
            return;
        }

        if (ReferenceEquals(value, SelectedVisibleRow))
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

    private void OnTelemetryDelta(object? sender, ProcessDeltaBatch delta)
    {
        RunOnUiThread(() =>
        {
            foreach (ProcessSample upsert in delta.Upserts)
            {
                ProcessIdentity identity = upsert.Identity();
                if (_allRows.TryGetValue(identity, out ProcessSample? previous))
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

                ProcessRowViewState rowState = GetOrCreateVisibleRowState(upsert);
                if (ShouldReplaceVisibleRow(rowState.Sample, upsert))
                {
                    rowState.UpdateSample(upsert);
                }

                rowState.UpdateCpuTrendPoints(BuildRowCpuTrendPoints(identity, upsert));
            }

            foreach (ProcessIdentity exit in delta.Exits)
            {
                if (_allRows.Remove(exit, out ProcessSample? previous))
                {
                    ApplySummaryDelta(previous, -1d);
                }

                _metadataCache.Remove(exit);
                _metricHistory.Remove(exit);
                if (_visibleRowStateByIdentity.Remove(exit, out ProcessRowViewState? rowState))
                {
                    _rowViewSource.Remove(rowState);
                }
            }

            _summarySeq = Math.Max(_summarySeq, delta.Seq);
            if (delta.Upserts.Count == 0)
            {
                _summaryTsMs = UnixNowMs();
            }

            ClampSummary();
            UpdateGlobalSummaryHistory();
            RefreshVisibleRows();
            ReconcileSelectionAfterDelta();
        });
    }

    private void OnRuntimeHealthChanged(object? sender, RuntimeHealth health)
    {
        RunOnUiThread(() => ApplyRuntimeHealth(health));
    }

    private void OnCollectorWarningRaised(object? sender, CollectorWarning warning)
    {
        RunOnUiThread(() => AdminModeError = warning.Message);
    }

    private void ApplyRuntimeHealth(RuntimeHealth health)
    {
        RuntimeHealthStatus =
            $"seq {health.Seq}, jitter p95 {health.JitterP95Ms:F0} ms, dropped {health.DroppedTicks}, degrade {(health.DegradeMode ? "ON" : "OFF")}";
    }

    private void LoadSnapshot(IReadOnlyList<ProcessSample> rows)
    {
        _allRows.Clear();
        _metricHistory.Clear();
        _visibleRowStateByIdentity.Clear();
        _rowViewSource.Clear();
        foreach (ProcessSample row in rows)
        {
            ProcessIdentity identity = row.Identity();
            _allRows[identity] = row;

            MetricHistoryBuffer history = new(HistoryLimit);
            history.Append(row);
            _metricHistory[identity] = history;

            ProcessRowViewState rowState = new(row, BuildRowCpuTrendPoints(identity, row));
            _visibleRowStateByIdentity[identity] = rowState;
            _rowViewSource.Add(rowState);
        }

        HashSet<ProcessIdentity> validIdentities = _allRows.Keys.ToHashSet();
        foreach (ProcessIdentity cachedIdentity in _metadataCache.Keys.ToList())
        {
            if (!validIdentities.Contains(cachedIdentity))
            {
                _metadataCache.Remove(cachedIdentity);
            }
        }

        ResetSummaryFromRows(_allRows.Values);
        _globalHistory.Reset();
        UpdateGlobalSummaryHistory();

        RefreshVisibleRows();
        ReconcileSelectionAfterDelta();
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

    private void RefreshVisibleRows()
    {
        VisibleRows.RefreshFilter();
        SelectedVisibleRow = SelectedRow is null ? null : TryGetVisibleRow(SelectedRow.Identity());
        RefreshDetailMetrics();
    }

    private bool ShouldShowRow(object item)
    {
        if (item is not ProcessRowViewState row)
        {
            return false;
        }

        if (!AdminModeEnabled && row.AccessState == AccessState.Denied)
        {
            return false;
        }

        if (AdminEnabledOnlyFilter && row.AccessState != AccessState.Full)
        {
            return false;
        }

        string needle = FilterText.Trim();
        if (string.IsNullOrWhiteSpace(needle))
        {
            return true;
        }

        return row.Name.Contains(needle, StringComparison.OrdinalIgnoreCase)
               || row.Pid.ToString().Contains(needle, StringComparison.OrdinalIgnoreCase);
    }

    private ProcessRowViewState GetOrCreateVisibleRowState(ProcessSample sample)
    {
        ProcessIdentity identity = sample.Identity();
        if (_visibleRowStateByIdentity.TryGetValue(identity, out ProcessRowViewState? existing))
        {
            return existing;
        }

        ProcessRowViewState state = new(sample, BuildRowCpuTrendPoints(identity, sample));
        _visibleRowStateByIdentity[identity] = state;
        _rowViewSource.Add(state);
        return state;
    }

    private ProcessRowViewState? TryGetVisibleRow(ProcessIdentity identity)
    {
        foreach (ProcessRowViewState row in VisibleRows.OfType<ProcessRowViewState>())
        {
            if (row.Identity == identity)
            {
                return row;
            }
        }

        return null;
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

        if (!string.Equals(primarySortKey, nameof(ProcessRowViewState.Pid), StringComparison.Ordinal))
        {
            VisibleRows.SortDescriptions.Add(new SortDescription(nameof(ProcessRowViewState.Pid), CommunityToolkit.WinUI.Collections.SortDirection.Ascending));
        }

        if (!string.Equals(primarySortKey, nameof(ProcessRowViewState.StartTimeMs), StringComparison.Ordinal))
        {
            VisibleRows.SortDescriptions.Add(new SortDescription(nameof(ProcessRowViewState.StartTimeMs), CommunityToolkit.WinUI.Collections.SortDirection.Ascending));
        }
    }

    private bool IsCurrentMetadataRequest(long requestVersion, ProcessIdentity identity)
    {
        return requestVersion == _metadataRequestVersion && SelectedRow?.Identity() == identity;
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
            RunOnUiThread(RefreshVisibleRows);
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
        OnPropertyChanged(nameof(NameSortLabel));
        OnPropertyChanged(nameof(PidSortLabel));
        OnPropertyChanged(nameof(CpuSortLabel));
        OnPropertyChanged(nameof(MemorySortLabel));
        OnPropertyChanged(nameof(IoReadSortLabel));
        OnPropertyChanged(nameof(IoWriteSortLabel));
        OnPropertyChanged(nameof(NetSortLabel));
        OnPropertyChanged(nameof(ThreadsSortLabel));
        OnPropertyChanged(nameof(HandlesSortLabel));
    }

    private void RaiseMetricFocusProperties()
    {
        OnPropertyChanged(nameof(IsCpuMetricFocused));
        OnPropertyChanged(nameof(IsMemoryMetricFocused));
        OnPropertyChanged(nameof(IsIoReadMetricFocused));
        OnPropertyChanged(nameof(IsIoWriteMetricFocused));
        OnPropertyChanged(nameof(IsNetworkMetricFocused));
    }

    private void RaiseMetadataProperties()
    {
        OnPropertyChanged(nameof(MetadataStatus));
        OnPropertyChanged(nameof(MetadataParentPid));
        OnPropertyChanged(nameof(MetadataCommandLine));
        OnPropertyChanged(nameof(MetadataExecutablePath));
    }

    private void RefreshDetailMetrics()
    {
        ProcessSample detailSample = SelectedRow ?? _globalSummaryRow;
        MetricHistoryBuffer history = GetDetailHistory(detailSample);

        CpuMetricChipValue = $"{detailSample.CpuPct:F2}%";
        MemoryMetricChipValue = ValueFormat.FormatBytes(detailSample.RssBytes);
        IoReadMetricChipValue = ValueFormat.FormatRate(detailSample.IoReadBps);
        IoWriteMetricChipValue = ValueFormat.FormatRate(detailSample.IoWriteBps);
        NetworkMetricChipValue = ValueFormat.FormatRate(detailSample.NetBps);

        CpuMetricTrendValues = history.Cpu.ToArray();
        MemoryMetricTrendValues = history.Memory.ToArray();
        IoReadMetricTrendValues = history.IoRead.ToArray();
        IoWriteMetricTrendValues = history.IoWrite.ToArray();
        NetworkMetricTrendValues = history.Net.ToArray();

        switch (MetricFocus)
        {
            case DetailMetricFocus.Cpu:
                ExpandedMetricTitle = "CPU Trend";
                ExpandedMetricValue = $"{detailSample.CpuPct:F1}% CPU";
                ExpandedMetricTrendValues = history.Cpu.ToArray();
                break;
            case DetailMetricFocus.Memory:
                ExpandedMetricTitle = "Memory Trend";
                ExpandedMetricValue = $"{ValueFormat.FormatBytes(detailSample.RssBytes)} RSS";
                ExpandedMetricTrendValues = history.Memory.ToArray();
                break;
            case DetailMetricFocus.IoRead:
                ExpandedMetricTitle = "Disk Read Trend";
                ExpandedMetricValue = $"{ValueFormat.FormatRate(detailSample.IoReadBps)} read";
                ExpandedMetricTrendValues = history.IoRead.ToArray();
                break;
            case DetailMetricFocus.IoWrite:
                ExpandedMetricTitle = "Disk Write Trend";
                ExpandedMetricValue = $"{ValueFormat.FormatRate(detailSample.IoWriteBps)} write";
                ExpandedMetricTrendValues = history.IoWrite.ToArray();
                break;
            case DetailMetricFocus.Network:
                ExpandedMetricTitle = "Network Trend";
                ExpandedMetricValue = $"{ValueFormat.FormatRate(detailSample.NetBps)} net";
                ExpandedMetricTrendValues = history.Net.ToArray();
                break;
            default:
                ExpandedMetricTitle = "CPU Trend";
                ExpandedMetricValue = $"{detailSample.CpuPct:F1}% CPU";
                ExpandedMetricTrendValues = history.Cpu.ToArray();
                break;
        }
    }

    private MetricHistoryBuffer GetDetailHistory(ProcessSample detailSample)
    {
        if (SelectedRow is null)
        {
            return _globalHistory;
        }

        if (_metricHistory.TryGetValue(detailSample.Identity(), out MetricHistoryBuffer? history))
        {
            return history;
        }

        MetricHistoryBuffer fallback = new(HistoryLimit);
        fallback.Append(detailSample);
        return fallback;
    }

    private void ResetSummaryFromRows(IEnumerable<ProcessSample> rows)
    {
        _summarySeq = 0;
        _summaryTsMs = UnixNowMs();
        _summaryCpuPct = 0;
        _summaryRssBytes = 0;
        _summaryPrivateBytes = 0;
        _summaryIoReadBps = 0;
        _summaryIoWriteBps = 0;
        _summaryNetBps = 0;
        _summaryThreads = 0;
        _summaryHandles = 0;

        foreach (ProcessSample row in rows)
        {
            ApplySummaryDelta(row, 1d);
            _summarySeq = Math.Max(_summarySeq, row.Seq);
            _summaryTsMs = Math.Max(_summaryTsMs, row.TsMs);
        }

        ClampSummary();
    }

    private void ApplySummaryDelta(ProcessSample sample, double multiplier)
    {
        _summaryCpuPct += sample.CpuPct * multiplier;
        _summaryRssBytes += sample.RssBytes * multiplier;
        _summaryPrivateBytes += sample.PrivateBytes * multiplier;
        _summaryIoReadBps += sample.IoReadBps * multiplier;
        _summaryIoWriteBps += sample.IoWriteBps * multiplier;
        _summaryNetBps += sample.NetBps * multiplier;
        _summaryThreads += sample.Threads * multiplier;
        _summaryHandles += sample.Handles * multiplier;
    }

    private void ClampSummary()
    {
        _summaryCpuPct = Math.Max(0d, _summaryCpuPct);
        _summaryRssBytes = Math.Max(0d, _summaryRssBytes);
        _summaryPrivateBytes = Math.Max(0d, _summaryPrivateBytes);
        _summaryIoReadBps = Math.Max(0d, _summaryIoReadBps);
        _summaryIoWriteBps = Math.Max(0d, _summaryIoWriteBps);
        _summaryNetBps = Math.Max(0d, _summaryNetBps);
        _summaryThreads = Math.Max(0d, _summaryThreads);
        _summaryHandles = Math.Max(0d, _summaryHandles);
    }

    private void UpdateGlobalSummaryHistory()
    {
        _globalSummaryRow = new ProcessSample
        {
            Seq = _summarySeq,
            TsMs = _summaryTsMs,
            Pid = 0,
            ParentPid = 0,
            StartTimeMs = 0,
            Name = "Global System Values",
            CpuPct = _summaryCpuPct,
            RssBytes = ClampToUlong(_summaryRssBytes),
            PrivateBytes = ClampToUlong(_summaryPrivateBytes),
            IoReadBps = ClampToUlong(_summaryIoReadBps),
            IoWriteBps = ClampToUlong(_summaryIoWriteBps),
            NetBps = ClampToUlong(_summaryNetBps),
            Threads = ClampToUInt(_summaryThreads),
            Handles = ClampToUInt(_summaryHandles),
            AccessState = AccessState.Full,
        };

        _globalHistory.Append(_globalSummaryRow);
        if (SelectedRow is null)
        {
            OnPropertyChanged(nameof(DetailTitle));
        }
    }

    private static ProcessSample CreateEmptyGlobalSummary()
    {
        return new ProcessSample
        {
            Seq = 0,
            TsMs = UnixNowMs(),
            Pid = 0,
            ParentPid = 0,
            StartTimeMs = 0,
            Name = "Global System Values",
            CpuPct = 0,
            RssBytes = 0,
            PrivateBytes = 0,
            IoReadBps = 0,
            IoWriteBps = 0,
            NetBps = 0,
            Threads = 0,
            Handles = 0,
            AccessState = AccessState.Full,
        };
    }

    private static ulong ClampToUlong(double value)
    {
        if (value <= 0)
        {
            return 0;
        }

        if (value >= ulong.MaxValue)
        {
            return ulong.MaxValue;
        }

        return (ulong)Math.Round(value);
    }

    private static uint ClampToUInt(double value)
    {
        if (value <= 0)
        {
            return 0;
        }

        if (value >= uint.MaxValue)
        {
            return uint.MaxValue;
        }

        return (uint)Math.Round(value);
    }

    private static ulong UnixNowMs()
    {
        long now = DateTimeOffset.UtcNow.ToUnixTimeMilliseconds();
        return now <= 0 ? 0UL : (ulong)now;
    }

    private static string FormatBlockReason(LaunchBlockReason? reason)
    {
        if (reason is null)
        {
            return "Unknown startup policy failure.";
        }

        return reason.Kind switch
        {
            LaunchBlockReasonKind.UnsupportedPlatform =>
                $"Unsupported platform: {reason.Os ?? "unknown"}. This build supports Windows 11 only.",
            LaunchBlockReasonKind.RequiresWindows11 =>
                $"Windows build {reason.DetectedBuild.GetValueOrDefault()} detected. Windows 11 build 22000+ is required.",
            _ => "Startup policy failed due to an unrecognized gate condition.",
        };
    }

    private void RaiseStateVisibilityProperties()
    {
        OnPropertyChanged(nameof(LoadingVisibility));
        OnPropertyChanged(nameof(BlockedVisibility));
        OnPropertyChanged(nameof(StartupErrorVisibility));
        OnPropertyChanged(nameof(LiveVisibility));
    }
}
