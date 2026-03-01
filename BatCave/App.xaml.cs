using System;
using System.IO;
using System.Linq;
using System.Threading;
using System.Threading.Tasks;
using BatCave.Core.Abstractions;
using BatCave.Core.Collector;
using BatCave.Core.Domain;
using BatCave.Core.Metadata;
using BatCave.Core.Operations;
using BatCave.Core.Persistence;
using BatCave.Core.Pipeline;
using BatCave.Core.Policy;
using BatCave.Core.Runtime;
using BatCave.Core.Sort;
using BatCave.Core.State;
using BatCave.Services;
using BatCave.ViewModels;
using Microsoft.Extensions.DependencyInjection;
using Microsoft.Extensions.Hosting;
using Microsoft.UI.Xaml;
using Serilog;

namespace BatCave;

public partial class App : Application
{
    private readonly IHost _host;
    private Window? _window;
    private bool _hostStopped;
    private bool _runtimeLoopWired;

    public App()
    {
        InitializeComponent();
        _host = CreateHost();
    }

    public static IServiceProvider Services
    {
        get
        {
            return ((App)Current)._host.Services;
        }
    }

    protected override async void OnLaunched(LaunchActivatedEventArgs args)
    {
        await _host.StartAsync();

        string[] commandLineArgs = Environment.GetCommandLineArgs().Skip(1).ToArray();
        ICliOperationsHost cliOperationsHost = _host.Services.GetRequiredService<ICliOperationsHost>();

        if (cliOperationsHost.IsCliMode(commandLineArgs))
        {
            int exitCode = await cliOperationsHost.ExecuteAsync(commandLineArgs, CancellationToken.None);
            await ShutdownHostAsync();
            Environment.Exit(exitCode);
            return;
        }

        StartRuntimeLoopIfAllowed();

        _window = _host.Services.GetRequiredService<MainWindow>();
        _window.Closed += OnWindowClosed;
        _window.Activate();
    }

    private static IHost CreateHost()
    {
        string logDirectory = Path.Combine(LocalJsonPersistenceStore.DefaultBaseDirectory(), "logs");
        Directory.CreateDirectory(logDirectory);

        Log.Logger = new LoggerConfiguration()
            .MinimumLevel.Information()
            .Enrich.FromLogContext()
            .WriteTo.File(
                path: Path.Combine(logDirectory, "monitor-.log"),
                rollingInterval: RollingInterval.Day,
                retainedFileCountLimit: 14,
                shared: true)
            .CreateLogger();

        return Host.CreateDefaultBuilder()
            .UseSerilog()
            .ConfigureServices(services =>
            {
                services.AddSingleton<ICliOperationsHost, CliOperationsHost>();
                services.AddSingleton<ILaunchPolicyGate, WindowsLaunchPolicyGate>();

                services.AddSingleton<IProcessCollectorFactory, DefaultProcessCollectorFactory>();
                services.AddSingleton<ITelemetryPipeline, DeltaTelemetryPipeline>();
                services.AddSingleton<IStateStore, InMemoryStateStore>();
                services.AddSingleton<ISortIndexEngine, IncrementalSortIndexEngine>();
                services.AddSingleton<IPersistenceStore, LocalJsonPersistenceStore>();
                services.AddSingleton<IProcessMetadataProvider, ProcessMetadataProvider>();

                services.AddSingleton<MonitoringRuntime>();
                services.AddSingleton(TimeProvider.System);
                services.AddSingleton<RuntimeLoopService>();
                services.AddSingleton<IRuntimeEventGateway, RuntimeGateway>();

                services.AddSingleton<MonitoringShellViewModel>();
                services.AddSingleton<MainWindow>();
            })
            .Build();
    }

    private void StartRuntimeLoopIfAllowed()
    {
        if (_runtimeLoopWired)
        {
            return;
        }

        StartupGateStatus status = _host.Services.GetRequiredService<ILaunchPolicyGate>().Enforce();
        if (!status.Passed)
        {
            return;
        }

        RuntimeLoopService runtimeLoopService = _host.Services.GetRequiredService<RuntimeLoopService>();
        IRuntimeEventGateway runtimeEventGateway = _host.Services.GetRequiredService<IRuntimeEventGateway>();

        runtimeLoopService.TickCompleted += (_, outcome) => runtimeEventGateway.Publish(outcome);
        runtimeLoopService.Start(runtimeLoopService.CurrentGeneration);

        _runtimeLoopWired = true;
    }

    private async void OnWindowClosed(object sender, WindowEventArgs args)
    {
        await ShutdownHostAsync();
    }

    private async Task ShutdownHostAsync()
    {
        if (_hostStopped)
        {
            return;
        }

        _hostStopped = true;

        RuntimeLoopService? runtimeLoopService = _host.Services.GetService<RuntimeLoopService>();
        runtimeLoopService?.StopAndAdvanceGeneration();

        await _host.StopAsync();
        _host.Dispose();
        Log.CloseAndFlush();
    }
}
