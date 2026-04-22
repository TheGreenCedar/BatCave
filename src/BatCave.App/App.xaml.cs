using BatCave.App.Benchmarking;
using BatCave.App.Presentation;
using BatCave.Runtime.Benchmarking;
using BatCave.Runtime.Contracts;
using BatCave.Runtime.Operations;
using BatCave.Runtime.Persistence;
using BatCave.Runtime.Serialization;
using BatCave.Runtime.Store;
using Microsoft.Extensions.DependencyInjection;
using Microsoft.Extensions.Hosting;
using Microsoft.UI.Xaml;
using Serilog;
using System;
using System.IO;
using System.Linq;
using System.Threading;
using System.Threading.Tasks;

namespace BatCave.App;

public partial class App : Application
{
    private readonly object _shutdownSync = new();
    private IHost? _host;
    private Window? _window;
    private Task? _shutdownTask;

    public App()
    {
        InitializeComponent();
        UnhandledException += OnUnhandledException;
    }

    public static IServiceProvider Services
    {
        get
        {
            App app = (App)Current;
            return app._host?.Services
                   ?? throw new InvalidOperationException("Application host has not been initialized.");
        }
    }

    protected override async void OnLaunched(LaunchActivatedEventArgs args)
    {
        string[] commandLineArgs = [.. Environment.GetCommandLineArgs().Skip(1)];
        bool cliMode = IsCliMode(commandLineArgs);
        _host = CreateHost(registerRuntimeLoop: !cliMode);

        try
        {
            if (cliMode)
            {
                int exitCode = await _host.Services
                    .GetRequiredService<CliOperationsHost>()
                    .ExecuteAsync(commandLineArgs, CancellationToken.None);
                await ShutdownHostAsync();
                Environment.ExitCode = exitCode;
                Exit();
                return;
            }

            StartupGateStatus gateStatus = _host.Services.GetRequiredService<ILaunchPolicyGate>().Enforce();
            if (!gateStatus.Passed)
            {
                Log.Warning("startup_gate_blocked {ReasonCode} {ReasonMessage}", gateStatus.Reason?.Code, gateStatus.Reason?.Message);
                await ShutdownHostAsync();
                Environment.ExitCode = 2;
                Exit();
                return;
            }

            await _host.StartAsync();
            _window = _host.Services.GetRequiredService<MainWindow>();
            _window.Activate();
        }
        catch (Exception ex)
        {
            Log.Error(ex, "app_launch_failed");
            throw;
        }
    }

    private static bool IsCliMode(string[] args)
    {
        return args.Any(argument =>
            string.Equals(argument, "--print-gate-status", StringComparison.OrdinalIgnoreCase)
            || string.Equals(argument, "--print-runtime-health", StringComparison.OrdinalIgnoreCase)
            || string.Equals(argument, "--benchmark", StringComparison.OrdinalIgnoreCase)
            || string.Equals(argument, "--elevated-helper", StringComparison.OrdinalIgnoreCase));
    }

    private static IHost CreateHost(bool registerRuntimeLoop)
    {
        string logDirectory = Path.Combine(LocalJsonRuntimePersistenceStore.DefaultBaseDirectory(), "logs");
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
                services.AddBatCaveRuntime(registerHostedService: registerRuntimeLoop);
                services.AddSingleton<IWinUiBenchmarkRunner, ShellWinUiBenchmarkRunner>();
                services.AddSingleton<ShellViewModel>();
                services.AddSingleton<MainWindow>();
            })
            .Build();
    }

    internal static Task ShutdownServicesAsync()
    {
        return Current is App app ? app.ShutdownHostAsync() : Task.CompletedTask;
    }

    private Task ShutdownHostAsync()
    {
        lock (_shutdownSync)
        {
            if (_shutdownTask is not null)
            {
                return _shutdownTask;
            }

            IHost? host = _host;
            _host = null;
            _window = null;
            _shutdownTask = host is null ? Task.CompletedTask : ShutdownHostCoreAsync(host);
            return _shutdownTask;
        }
    }

    private static async Task ShutdownHostCoreAsync(IHost host)
    {
        try
        {
            await host.StopAsync(TimeSpan.FromSeconds(5)).ConfigureAwait(false);
        }
        finally
        {
            if (host is IAsyncDisposable asyncDisposable)
            {
                await asyncDisposable.DisposeAsync().ConfigureAwait(false);
            }
            else
            {
                host.Dispose();
            }

            await Log.CloseAndFlushAsync().ConfigureAwait(false);
        }
    }

    private void OnUnhandledException(object sender, Microsoft.UI.Xaml.UnhandledExceptionEventArgs e)
    {
        Log.Error(e.Exception, "unhandled_winui_exception");
    }
}
