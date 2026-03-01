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

namespace BatCave.ViewModels;

public class MonitoringShellViewModel : ObservableObject
{
    private readonly ILaunchPolicyGate _launchPolicyGate;
    private readonly MonitoringRuntime _runtime;
    private readonly RuntimeLoopService _runtimeLoopService;
    private readonly IRuntimeEventGateway _runtimeEventGateway;
    private readonly IProcessMetadataProvider _metadataProvider;

    private readonly Dictionary<ProcessIdentity, ProcessSample> _allRows = new();
    private readonly Dictionary<ProcessIdentity, ProcessMetadata?> _metadataCache = new();

    private bool _isLoading = true;
    private bool _isBlocked;
    private bool _isStartupError;
    private bool _isLive;
    private string _shellHeadline = "Initializing monitor runtime...";
    private string _shellBody = "Starting monitoring services.";
    private string _blockedReasonMessage = string.Empty;
    private string _startupErrorMessage = string.Empty;
    private string _filterText = string.Empty;
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
        private set => SetProperty(ref _isLoading, value);
    }

    public bool IsBlocked
    {
        get => _isBlocked;
        private set => SetProperty(ref _isBlocked, value);
    }

    public bool IsStartupError
    {
        get => _isStartupError;
        private set => SetProperty(ref _isStartupError, value);
    }

    public bool IsLive
    {
        get => _isLive;
        private set => SetProperty(ref _isLive, value);
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
                _runtime.SetFilter(value);
                RefreshVisibleRows();
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
        private set => SetProperty(ref _adminModeError, value);
    }

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
        private set => SetProperty(ref _selectedRow, value);
    }

    public ProcessMetadata? SelectedMetadata
    {
        get => _selectedMetadata;
        private set => SetProperty(ref _selectedMetadata, value);
    }

    public bool IsMetadataLoading
    {
        get => _isMetadataLoading;
        private set => SetProperty(ref _isMetadataLoading, value);
    }

    public string? MetadataError
    {
        get => _metadataError;
        private set => SetProperty(ref _metadataError, value);
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

            if (!IsCurrentMetadataRequest(requestVersion, identity))
            {
                return;
            }

            _metadataCache[identity] = metadata;
            SelectedMetadata = metadata;
            MetadataError = null;
        }
        catch (Exception ex)
        {
            if (!IsCurrentMetadataRequest(requestVersion, identity))
            {
                return;
            }

            MetadataError = ex.Message;
            SelectedMetadata = null;
        }
        finally
        {
            if (IsCurrentMetadataRequest(requestVersion, identity))
            {
                IsMetadataLoading = false;
            }
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
    }

    private void OnRuntimeHealthChanged(object? sender, RuntimeHealth health)
    {
        ApplyRuntimeHealth(health);
    }

    private void OnCollectorWarningRaised(object? sender, CollectorWarning warning)
    {
        AdminModeError = warning.Message;
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
            rows = rows.Where(row => AdminModeEnabled && row.AccessState == AccessState.Full);
        }

        string needle = FilterText.Trim().ToLowerInvariant();
        if (!string.IsNullOrWhiteSpace(needle))
        {
            rows = rows.Where(row =>
                row.Name.Contains(needle, StringComparison.OrdinalIgnoreCase)
                || row.Pid.ToString().Contains(needle, StringComparison.OrdinalIgnoreCase));
        }

        List<ProcessSample> next = rows
            .OrderByDescending(row => row.CpuPct)
            .ThenBy(row => row.Pid)
            .ThenBy(row => row.StartTimeMs)
            .ToList();

        VisibleRows.Clear();
        foreach (ProcessSample row in next)
        {
            VisibleRows.Add(row);
        }
    }

    private bool IsCurrentMetadataRequest(long requestVersion, ProcessIdentity identity)
    {
        return requestVersion == _metadataRequestVersion && SelectedRow?.Identity() == identity;
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
}
