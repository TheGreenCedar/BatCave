using System;
using System.Threading;
using System.Threading.Tasks;
using BatCave.Core.Abstractions;
using BatCave.Core.Domain;
using CommunityToolkit.Mvvm.ComponentModel;

namespace BatCave.ViewModels;

public class MonitoringShellViewModel : ObservableObject
{
    private readonly ILaunchPolicyGate _launchPolicyGate;

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
    private string _runtimeHealthStatus = "Runtime health unavailable.";
    private StartupGateStatus _startupGateStatus = new();

    public MonitoringShellViewModel(ILaunchPolicyGate launchPolicyGate)
    {
        _launchPolicyGate = launchPolicyGate;
    }

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
        set => SetProperty(ref _filterText, value);
    }

    public bool AdminModeEnabled
    {
        get => _adminModeEnabled;
        set => SetProperty(ref _adminModeEnabled, value);
    }

    public bool AdminEnabledOnlyFilter
    {
        get => _adminEnabledOnlyFilter;
        set => SetProperty(ref _adminEnabledOnlyFilter, value);
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

            IsLive = true;
            BlockedReasonMessage = string.Empty;
            ShellHeadline = "Live monitor shell ready";
            ShellBody = "Runtime and table wiring will be completed in upcoming tasks.";
            RuntimeHealthStatus = "Runtime startup passed. Monitoring runtime initialization will continue in the next task.";
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
