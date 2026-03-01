using System;
using System.Collections.Generic;
using System.Collections.ObjectModel;
using System.Linq;
using System.Threading;
using System.Threading.Tasks;
using BatCave.Core.Abstractions;
using BatCave.Core.Domain;
using BatCave.Core.Runtime;
using BatCave.Services;
using CommunityToolkit.Mvvm.ComponentModel;
using Microsoft.UI.Dispatching;
using Microsoft.UI.Xaml;

namespace BatCave.ViewModels;

public enum DetailMetricFocus
{
    Cpu,
    Memory,
    Io,
    Network,
}

public class MonitoringShellViewModel : ObservableObject
{
    private const int FilterDebounceMs = 160;

    private readonly ILaunchPolicyGate _launchPolicyGate;
    private readonly MonitoringRuntime _runtime;
    private readonly RuntimeLoopService _runtimeLoopService;
    private readonly IRuntimeEventGateway _runtimeEventGateway;
    private readonly IProcessMetadataProvider _metadataProvider;

    private readonly Dictionary<ProcessIdentity, ProcessSample> _allRows = new();
    private readonly Dictionary<ProcessIdentity, ProcessMetadata?> _metadataCache = new();

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
    private ProcessMetadata? _selectedMetadata;
    private bool _isMetadataLoading;
    private string? _metadataError;
    private DetailMetricFocus _metricFocus = DetailMetricFocus.Cpu;
    private long _metadataRequestVersion;

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
    }

    public ObservableCollection<ProcessSample> VisibleRows { get; } = [];

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
                RaiseDetailProperties();
            }
        }
    }

    public ProcessMetadata? SelectedMetadata
    {
        get => _selectedMetadata;
        private set
        {
            if (SetProperty(ref _selectedMetadata, value))
            {
                RaiseDetailProperties();
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
                RaiseDetailProperties();
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
                RaiseDetailProperties();
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
                RaiseDetailProperties();
            }
        }
    }

    public bool HasSelection => SelectedRow is not null;

    public string DetailTitle =>
        SelectedRow is null
            ? "Global Summary"
            : $"{SelectedRow.Name} ({SelectedRow.Pid})";

    public string DetailMetricValue
    {
        get
        {
            if (SelectedRow is null)
            {
                return $"Visible rows: {VisibleRows.Count}";
            }

            return MetricFocus switch
            {
                DetailMetricFocus.Cpu => $"{SelectedRow.CpuPct:F1}% CPU",
                DetailMetricFocus.Memory => $"{FormatBytes(SelectedRow.RssBytes)} RSS",
                DetailMetricFocus.Io => $"{FormatRate(SelectedRow.IoReadBps)} read, {FormatRate(SelectedRow.IoWriteBps)} write",
                DetailMetricFocus.Network => $"{FormatRate(SelectedRow.NetBps)} net",
                _ => $"{SelectedRow.CpuPct:F1}% CPU",
            };
        }
    }

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
        RefreshVisibleRows();
    }

    public async Task ToggleSelectionAsync(ProcessSample? row, CancellationToken ct)
    {
        if (row is null)
        {
            ClearSelection();
            return;
        }

        if (SelectedRow?.Identity() == row.Identity())
        {
            ClearSelection();
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
        SelectedMetadata = null;
        IsMetadataLoading = false;
        MetadataError = null;
    }

    private void OnTelemetryDelta(object? sender, ProcessDeltaBatch delta)
    {
        RunOnUiThread(() =>
        {
            foreach (ProcessSample upsert in delta.Upserts)
            {
                _allRows[upsert.Identity()] = upsert;
            }

            foreach (ProcessIdentity exit in delta.Exits)
            {
                _allRows.Remove(exit);
                _metadataCache.Remove(exit);
            }

            ReconcileSelectionAfterDelta();
            RefreshVisibleRows();
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
        foreach (ProcessSample row in rows)
        {
            _allRows[row.Identity()] = row;
        }

        HashSet<ProcessIdentity> validIdentities = _allRows.Keys.ToHashSet();
        foreach (ProcessIdentity cachedIdentity in _metadataCache.Keys.ToList())
        {
            if (!validIdentities.Contains(cachedIdentity))
            {
                _metadataCache.Remove(cachedIdentity);
            }
        }

        ReconcileSelectionAfterDelta();
        RefreshVisibleRows();
    }

    private void ReconcileSelectionAfterDelta()
    {
        if (SelectedRow is null)
        {
            return;
        }

        ProcessIdentity identity = SelectedRow.Identity();
        if (_allRows.TryGetValue(identity, out ProcessSample? updated))
        {
            SelectedRow = updated;
            return;
        }

        ClearSelection();
    }

    private void RefreshVisibleRows()
    {
        IEnumerable<ProcessSample> rows = _allRows.Values;

        if (!AdminModeEnabled)
        {
            rows = rows.Where(row => row.AccessState != AccessState.Denied);
        }

        if (AdminEnabledOnlyFilter)
        {
            rows = rows.Where(row => row.AccessState == AccessState.Full);
        }

        string needle = FilterText.Trim().ToLowerInvariant();
        if (!string.IsNullOrWhiteSpace(needle))
        {
            rows = rows.Where(row =>
                row.Name.Contains(needle, StringComparison.OrdinalIgnoreCase)
                || row.Pid.ToString().Contains(needle, StringComparison.OrdinalIgnoreCase));
        }

        List<ProcessSample> next = OrderRows(rows).ToList();

        VisibleRows.Clear();
        foreach (ProcessSample row in next)
        {
            VisibleRows.Add(row);
        }

        RaiseDetailProperties();
    }

    private IOrderedEnumerable<ProcessSample> OrderRows(IEnumerable<ProcessSample> rows)
    {
        return (CurrentSortColumn, CurrentSortDirection) switch
        {
            (SortColumn.Pid, SortDirection.Asc) => rows.OrderBy(row => row.Pid).ThenBy(row => row.StartTimeMs),
            (SortColumn.Pid, SortDirection.Desc) => rows.OrderByDescending(row => row.Pid).ThenBy(row => row.StartTimeMs),
            (SortColumn.Name, SortDirection.Asc) => rows.OrderBy(row => row.Name, StringComparer.OrdinalIgnoreCase).ThenBy(row => row.Pid).ThenBy(row => row.StartTimeMs),
            (SortColumn.Name, SortDirection.Desc) => rows.OrderByDescending(row => row.Name, StringComparer.OrdinalIgnoreCase).ThenBy(row => row.Pid).ThenBy(row => row.StartTimeMs),
            (SortColumn.CpuPct, SortDirection.Asc) => rows.OrderBy(row => row.CpuPct).ThenBy(row => row.Pid).ThenBy(row => row.StartTimeMs),
            (SortColumn.CpuPct, SortDirection.Desc) => rows.OrderByDescending(row => row.CpuPct).ThenBy(row => row.Pid).ThenBy(row => row.StartTimeMs),
            (SortColumn.RssBytes, SortDirection.Asc) => rows.OrderBy(row => row.RssBytes).ThenBy(row => row.Pid).ThenBy(row => row.StartTimeMs),
            (SortColumn.RssBytes, SortDirection.Desc) => rows.OrderByDescending(row => row.RssBytes).ThenBy(row => row.Pid).ThenBy(row => row.StartTimeMs),
            (SortColumn.IoReadBps, SortDirection.Asc) => rows.OrderBy(row => row.IoReadBps).ThenBy(row => row.Pid).ThenBy(row => row.StartTimeMs),
            (SortColumn.IoReadBps, SortDirection.Desc) => rows.OrderByDescending(row => row.IoReadBps).ThenBy(row => row.Pid).ThenBy(row => row.StartTimeMs),
            (SortColumn.IoWriteBps, SortDirection.Asc) => rows.OrderBy(row => row.IoWriteBps).ThenBy(row => row.Pid).ThenBy(row => row.StartTimeMs),
            (SortColumn.IoWriteBps, SortDirection.Desc) => rows.OrderByDescending(row => row.IoWriteBps).ThenBy(row => row.Pid).ThenBy(row => row.StartTimeMs),
            (SortColumn.NetBps, SortDirection.Asc) => rows.OrderBy(row => row.NetBps).ThenBy(row => row.Pid).ThenBy(row => row.StartTimeMs),
            (SortColumn.NetBps, SortDirection.Desc) => rows.OrderByDescending(row => row.NetBps).ThenBy(row => row.Pid).ThenBy(row => row.StartTimeMs),
            (SortColumn.Threads, SortDirection.Asc) => rows.OrderBy(row => row.Threads).ThenBy(row => row.Pid).ThenBy(row => row.StartTimeMs),
            (SortColumn.Threads, SortDirection.Desc) => rows.OrderByDescending(row => row.Threads).ThenBy(row => row.Pid).ThenBy(row => row.StartTimeMs),
            (SortColumn.Handles, SortDirection.Asc) => rows.OrderBy(row => row.Handles).ThenBy(row => row.Pid).ThenBy(row => row.StartTimeMs),
            (SortColumn.Handles, SortDirection.Desc) => rows.OrderByDescending(row => row.Handles).ThenBy(row => row.Pid).ThenBy(row => row.StartTimeMs),
            (SortColumn.StartTimeMs, SortDirection.Asc) => rows.OrderBy(row => row.StartTimeMs).ThenBy(row => row.Pid),
            (SortColumn.StartTimeMs, SortDirection.Desc) => rows.OrderByDescending(row => row.StartTimeMs).ThenBy(row => row.Pid),
            _ => rows.OrderByDescending(row => row.CpuPct).ThenBy(row => row.Pid).ThenBy(row => row.StartTimeMs),
        };
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

    private void RaiseDetailProperties()
    {
        OnPropertyChanged(nameof(DetailTitle));
        OnPropertyChanged(nameof(DetailMetricValue));
        OnPropertyChanged(nameof(MetadataStatus));
        OnPropertyChanged(nameof(MetadataParentPid));
        OnPropertyChanged(nameof(MetadataCommandLine));
        OnPropertyChanged(nameof(MetadataExecutablePath));
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

    private static string FormatBytes(ulong value)
    {
        const double kb = 1024d;
        const double mb = kb * 1024d;
        const double gb = mb * 1024d;

        if (value >= gb)
        {
            return $"{value / gb:F2} GB";
        }

        if (value >= mb)
        {
            return $"{value / mb:F1} MB";
        }

        if (value >= kb)
        {
            return $"{value / kb:F1} KB";
        }

        return $"{value} B";
    }

    private static string FormatRate(ulong value)
    {
        return $"{FormatBytes(value)}/s";
    }

    private void RaiseStateVisibilityProperties()
    {
        OnPropertyChanged(nameof(LoadingVisibility));
        OnPropertyChanged(nameof(BlockedVisibility));
        OnPropertyChanged(nameof(StartupErrorVisibility));
        OnPropertyChanged(nameof(LiveVisibility));
    }
}
