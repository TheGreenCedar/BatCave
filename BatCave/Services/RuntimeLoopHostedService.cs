using BatCave.Core.Abstractions;
using BatCave.Core.Domain;
using BatCave.Core.Runtime;
using Microsoft.Extensions.Hosting;
using Microsoft.Extensions.Logging;
using System;
using System.Threading;
using System.Threading.Tasks;

namespace BatCave.Services;

public sealed class RuntimeLoopHostedService : IHostedService, IDisposable
{
    private readonly IRuntimeLoopController _runtimeLoopController;
    private readonly IRuntimeEventGateway _runtimeEventGateway;
    private readonly ILaunchPolicyGate _launchPolicyGate;
    private readonly RuntimeHostOptions _runtimeHostOptions;
    private readonly IRuntimeHealthService _runtimeHealthService;
    private readonly ILogger<RuntimeLoopHostedService> _logger;

    private bool _eventsWired;
    private bool _started;

    public RuntimeLoopHostedService(
        IRuntimeLoopController runtimeLoopController,
        IRuntimeEventGateway runtimeEventGateway,
        ILaunchPolicyGate launchPolicyGate,
        RuntimeHostOptions runtimeHostOptions,
        IRuntimeHealthService runtimeHealthService,
        ILogger<RuntimeLoopHostedService> logger)
    {
        _runtimeLoopController = runtimeLoopController;
        _runtimeEventGateway = runtimeEventGateway;
        _launchPolicyGate = launchPolicyGate;
        _runtimeHostOptions = runtimeHostOptions;
        _runtimeHealthService = runtimeHealthService;
        _logger = logger;
    }

    public Task StartAsync(CancellationToken cancellationToken)
    {
        if (cancellationToken.IsCancellationRequested)
        {
            return Task.FromCanceled(cancellationToken);
        }

        if (!_runtimeHostOptions.EnableRuntimeLoop)
        {
            _runtimeHealthService.ReportRuntimeLoopState(
                enabled: false,
                running: false,
                startupBlocked: false,
                statusSummary: "Runtime loop disabled for this host mode.");
            return Task.CompletedTask;
        }

        StartupGateStatus startupGateStatus = _launchPolicyGate.Enforce();
        if (!startupGateStatus.Passed)
        {
            string reason = FormatBlockReason(startupGateStatus.Reason);
            _runtimeHealthService.ReportRuntimeLoopState(
                enabled: true,
                running: false,
                startupBlocked: true,
                statusSummary: $"Runtime loop blocked: {reason}");
            _logger.LogWarning("runtime_loop_start_blocked reason={Reason}", reason);
            return Task.CompletedTask;
        }

        EnsureEventsWired();
        _runtimeLoopController.Start(_runtimeLoopController.CurrentGeneration);
        _started = true;

        _runtimeHealthService.ReportRuntimeLoopState(
            enabled: true,
            running: true,
            startupBlocked: false,
            statusSummary: "Runtime loop active.");

        _logger.LogInformation("runtime_loop_started generation={Generation}", _runtimeLoopController.CurrentGeneration);
        return Task.CompletedTask;
    }

    public Task StopAsync(CancellationToken cancellationToken)
    {
        if (cancellationToken.IsCancellationRequested)
        {
            return Task.FromCanceled(cancellationToken);
        }

        if (_started)
        {
            _runtimeLoopController.StopAndAdvanceGeneration();
            _started = false;
            _logger.LogInformation("runtime_loop_stopped");
        }

        _runtimeHealthService.ReportRuntimeLoopState(
            enabled: _runtimeHostOptions.EnableRuntimeLoop,
            running: false,
            startupBlocked: false,
            statusSummary: _runtimeHostOptions.EnableRuntimeLoop
                ? "Runtime loop stopped."
                : "Runtime loop disabled for this host mode.");

        return Task.CompletedTask;
    }

    public void Dispose()
    {
        if (!_eventsWired)
        {
            return;
        }

        _runtimeLoopController.TickCompleted -= OnTickCompleted;
        _runtimeLoopController.TickFaulted -= OnTickFaulted;
        _eventsWired = false;
    }

    private void EnsureEventsWired()
    {
        if (_eventsWired)
        {
            return;
        }

        _runtimeLoopController.TickCompleted += OnTickCompleted;
        _runtimeLoopController.TickFaulted += OnTickFaulted;
        _eventsWired = true;
    }

    private void OnTickCompleted(object? sender, TickOutcome outcome)
    {
        _runtimeEventGateway.Publish(outcome);
    }

    private void OnTickFaulted(object? sender, TickFaultedEventArgs fault)
    {
        _runtimeEventGateway.PublishWarning(new CollectorWarning
        {
            Seq = 0,
            Message = $"runtime loop fault ({fault.ExceptionType}): {fault.Message}. retry in {fault.DelayMs} ms (streak {fault.ConsecutiveFaults}, generation {fault.Generation})",
        });
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
