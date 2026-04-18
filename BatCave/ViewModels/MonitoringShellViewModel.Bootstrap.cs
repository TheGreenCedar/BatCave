using BatCave.Core.Abstractions;
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
        if (_disposed)
        {
            return Task.CompletedTask;
        }

        if (IsStartupError && _startupFailureRecoveryAction is not null)
        {
            IsLoading = false;
            return Task.CompletedTask;
        }

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

    public async Task RetryBootstrapAsync(CancellationToken ct)
    {
        Func<CancellationToken, Task>? recoveryAction = _startupFailureRecoveryAction;
        if (recoveryAction is not null)
        {
            _startupFailureRecoveryAction = null;
            InitializeBootstrapState();

            try
            {
                await recoveryAction(ct).ConfigureAwait(false);
            }
            catch (Exception ex)
            {
                PresentStartupFailure(ex, recoveryAction);
                return;
            }
        }

        await BootstrapAsync(ct).ConfigureAwait(false);
    }

    [RelayCommand]
    private Task RetryBootstrapRequestedAsync()
    {
        return RetryBootstrapAsync(CancellationToken.None);
    }

    public void PresentStartupFailure(Exception ex, Func<CancellationToken, Task>? recoveryAction = null)
    {
        if (_disposed)
        {
            return;
        }

        _startupFailureRecoveryAction = recoveryAction;
        ApplyStartupFailureState(ex);
    }

    public async Task ToggleAdminModeAsync(bool nextAdminMode, CancellationToken ct)
    {
        if (_disposed)
        {
            return;
        }

        if (AdminModePending || nextAdminMode == AdminModeEnabled)
        {
            return;
        }

        AdminModePending = true;
        AdminModeError = null;
        bool loopStopped = false;

        try
        {
            await _runtimeLoopService.StopAndAdvanceGenerationAsync(ct);
            loopStopped = true;
            CollectorActivationResult activation = await _runtime.RestartAsync(nextAdminMode, ct);

            if (_disposed)
            {
                return;
            }

            AdminModeEnabled = activation.EffectiveAdminMode;
            RefreshRuntimeSnapshot();
            ApplyCollectorWarning(new CollectorWarning
            {
                Seq = 0,
                Message = activation.Warning ?? string.Empty,
            });
        }
        catch (Exception ex)
        {
            if (_disposed)
            {
                return;
            }

            AdminModeError = ex.Message;
            AdminModeEnabled = _runtime.IsAdminMode();
        }
        finally
        {
            if (!_disposed)
            {
                if (loopStopped)
                {
                    _runtimeLoopService.Start(_runtimeLoopService.CurrentGeneration);
                }

                AdminModePending = false;
            }
        }
    }

    private void OnRuntimeHealthChanged(object? sender, RuntimeHealth health)
    {
        if (_disposed)
        {
            return;
        }

        QueuePendingRuntimeHealth(health);
    }

    private void OnCollectorWarningRaised(object? sender, CollectorWarning warning)
    {
        if (_disposed)
        {
            return;
        }

        QueuePendingCollectorWarning(warning);
    }

    private void ApplyCollectorWarning(CollectorWarning warning)
    {
        if (_disposed)
        {
            return;
        }

        if (string.IsNullOrWhiteSpace(warning.Message))
        {
            return;
        }

        RuntimeHealth runtimeHealth = _runtime.GetRuntimeHealth();
        _latestWarningSeq = warning.Seq > 0 ? warning.Seq : runtimeHealth.Seq;
        _latestWarningSummary = warning.Message;
        AdminModeError = warning.Message;
        RuntimeHealthStatus = BuildRuntimeHealthStatus(runtimeHealth);
        SetRuntimeStatusPresentation(RuntimeStatusTone.Warning, "Collector Warning", warning.Message);
    }

    private void RefreshRuntimeSnapshot()
    {
        if (_disposed)
        {
            return;
        }

        QueryResponse snapshot = _runtime.GetSnapshot();
        RuntimeHealth health = _runtime.GetRuntimeHealth();
        LoadSnapshot(snapshot.Rows);
        ApplyRuntimeHealth(health);
    }

    private void InitializeBootstrapState()
    {
        if (_disposed)
        {
            return;
        }

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
        SetRuntimeStatusPresentation(RuntimeStatusTone.Info, "Starting Runtime", "Starting monitoring services.");
    }

    private void ApplyBlockedStartupState(StartupGateStatus startupGateStatus)
    {
        if (_disposed)
        {
            return;
        }

        IsLoading = false;
        IsStartupError = false;
        IsBlocked = true;
        IsLive = false;
        ResetWarningState();
        StartupErrorMessage = string.Empty;
        StartupGateStatus = startupGateStatus;
        BlockedReasonMessage = FormatBlockReason(startupGateStatus.Reason);
        ShellHeadline = "Startup Blocked";
        ShellBody = BlockedReasonMessage;
        RuntimeHealthStatus = "Runtime health unavailable.";
        SetRuntimeStatusPresentation(RuntimeStatusTone.Error, "Startup Blocked", BlockedReasonMessage);
    }

    private void ApplyStartupFailureState(Exception ex)
    {
        if (_disposed)
        {
            return;
        }

        IsLoading = false;
        IsBlocked = false;
        IsStartupError = true;
        IsLive = false;
        ResetWarningState();
        BlockedReasonMessage = string.Empty;
        StartupGateStatus = new StartupGateStatus();
        StartupErrorMessage = ex.Message;
        ShellHeadline = "Startup Incomplete";
        ShellBody = ex.Message;
        RuntimeHealthStatus = _runtimeHealthService.Snapshot().StatusSummary;
        SetRuntimeStatusPresentation(RuntimeStatusTone.Error, "Startup Error", ex.Message);
    }

    private void ApplyRuntimeHealth(RuntimeHealth health)
    {
        if (_disposed)
        {
            return;
        }

        MaybeClearStaleWarning(health);
        RuntimeHealthStatus = BuildRuntimeHealthStatus(health);
        ApplyRuntimeStatusPresentation(health);
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

    private void ApplyRuntimeStatusPresentation(RuntimeHealth health)
    {
        if (!string.IsNullOrWhiteSpace(_latestWarningSummary))
        {
            SetRuntimeStatusPresentation(RuntimeStatusTone.Warning, "Collector Warning", _latestWarningSummary!);
            return;
        }

        if (health.DroppedTicks > 0)
        {
            SetRuntimeStatusPresentation(
                RuntimeStatusTone.Error,
                "Dropped Samples Detected",
                "Runtime loop dropped samples. Review runtime detail for counts and jitter.");
            return;
        }


        SetRuntimeStatusPresentation(
            RuntimeStatusTone.Success,
            "Runtime Healthy",
            "No active collector warnings. Review runtime detail for current loop metrics.",
            isVisible: false);
    }
}
