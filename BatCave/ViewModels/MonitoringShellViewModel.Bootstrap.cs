using BatCave.Core.Domain;
using BatCave.Services;
using CommunityToolkit.Mvvm.Input;
using System;
using System.Threading;
using System.Threading.Tasks;

namespace BatCave.ViewModels;

public partial class MonitoringShellViewModel
{
    private const ulong WarningClearAfterTicks = 8;
    private string? _latestWarningSummary;
    private ulong _latestWarningSeq;

    public Task BootstrapAsync(CancellationToken ct)
    {
        InitializeBootstrapState();

        try
        {
            StartupGateStatus = _launchPolicyGate.Enforce();
            if (!StartupGateStatus.Passed)
            {
                ApplyBlockedStartupState(StartupGateStatus);
                return Task.CompletedTask;
            }

            _filterText = _runtime.CurrentFilterText;
            OnPropertyChanged(nameof(FilterText));

            CurrentSortColumn = _runtime.CurrentSortColumn;
            CurrentSortDirection = _runtime.CurrentSortDirection;
            ApplyCanonicalShaping();
            AdminModeEnabled = _runtime.IsAdminMode();
            MetricTrendWindowSeconds = _runtime.CurrentMetricTrendWindowSeconds;

            RefreshRuntimeSnapshot();

            IsLive = true;
            BlockedReasonMessage = string.Empty;
            ShellHeadline = "Live monitor shell ready";
            ShellBody = "Runtime loop and admin restart paths are active.";
        }
        catch (Exception ex)
        {
            ApplyStartupFailureState(ex);
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

    [RelayCommand]
    private Task RetryBootstrapRequestedAsync()
    {
        return RetryBootstrapAsync(CancellationToken.None);
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
            RefreshRuntimeSnapshot();
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

    private void OnRuntimeHealthChanged(object? sender, RuntimeHealth health)
    {
        RunOnUiThread(() => ApplyRuntimeHealth(health));
    }

    private void OnCollectorWarningRaised(object? sender, CollectorWarning warning)
    {
        RunOnUiThread(() =>
        {
            RuntimeHealth runtimeHealth = _runtime.GetRuntimeHealth();
            _latestWarningSeq = warning.Seq > 0 ? warning.Seq : runtimeHealth.Seq;
            _latestWarningSummary = warning.Message;
            AdminModeError = warning.Message;
            RuntimeHealthStatus = BuildRuntimeHealthStatus(runtimeHealth);
        });
    }

    private void RefreshRuntimeSnapshot()
    {
        QueryResponse snapshot = _runtime.GetSnapshot();
        RuntimeHealth health = _runtime.GetRuntimeHealth();
        LoadSnapshot(snapshot.Rows);
        ApplyRuntimeHealth(health);
    }

    private void InitializeBootstrapState()
    {
        RuntimeHealthSnapshot healthSnapshot = _runtimeHealthService.Snapshot();
        IsLoading = true;
        IsStartupError = false;
        IsBlocked = false;
        IsLive = false;
        EnsureGlobalMetricsSamplingStarted();
        ResetWarningState();
        StartupErrorMessage = string.Empty;
        ShellHeadline = "Initializing monitor runtime...";
        ShellBody = "Starting monitoring services.";
        RuntimeHealthStatus = healthSnapshot.StatusSummary;
    }

    private void ApplyBlockedStartupState(StartupGateStatus startupGateStatus)
    {
        IsBlocked = true;
        ResetWarningState();
        BlockedReasonMessage = FormatBlockReason(startupGateStatus.Reason);
        ShellHeadline = "Startup Blocked";
        ShellBody = BlockedReasonMessage;
        RuntimeHealthStatus = "Runtime health unavailable.";
    }

    private void ApplyStartupFailureState(Exception ex)
    {
        IsStartupError = true;
        ResetWarningState();
        StartupErrorMessage = ex.Message;
        ShellHeadline = "Startup Incomplete";
        ShellBody = ex.Message;
    }

    private void ApplyRuntimeHealth(RuntimeHealth health)
    {
        MaybeClearStaleWarning(health);
        RuntimeHealthStatus = BuildRuntimeHealthStatus(health);
    }

    private string BuildRuntimeHealthStatus(RuntimeHealth health)
    {
        string status =
            $"seq {health.Seq}, jitter p95 {health.JitterP95Ms:F0} ms, dropped {health.DroppedTicks}, degrade {(health.DegradeMode ? "ON" : "OFF")}";

        if (!string.IsNullOrWhiteSpace(_latestWarningSummary))
        {
            status += $", last warning: {_latestWarningSummary}";
        }

        return status;
    }

    private void MaybeClearStaleWarning(RuntimeHealth health)
    {
        if (string.IsNullOrWhiteSpace(_latestWarningSummary)
            || health.Seq <= _latestWarningSeq
            || health.Seq - _latestWarningSeq < WarningClearAfterTicks)
        {
            return;
        }

        ResetWarningState();
        AdminModeError = null;
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

    private void ResetWarningState()
    {
        _latestWarningSummary = null;
        _latestWarningSeq = 0;
    }
}
