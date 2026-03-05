using BatCave.Core.Operations;
using BatCave.Core.Persistence;
using BatCave.Core.Runtime;
using BatCave.Core.Serialization;
using BatCave.Hosting;
using BatCave.Services;
using Microsoft.Extensions.DependencyInjection;
using Microsoft.Extensions.Hosting;
using Microsoft.Extensions.Logging;
using Microsoft.UI.Xaml;
using Serilog;
using System;
using System.Collections.Generic;
using System.IO;
using System.Linq;
using System.Text.Json;
using System.Threading;
using System.Threading.Tasks;

namespace BatCave;

public partial class App : Application
{
    private static readonly HashSet<string> CliModeFlags = new(StringComparer.OrdinalIgnoreCase)
    {
        "--print-gate-status",
        "--benchmark",
        "--elevated-helper",
        "--print-runtime-health",
    };

    private IHost? _host;
    private Window? _window;
    private bool _hostStopped;

    public App()
    {
        InitializeComponent();
        UnhandledException += OnUnhandledException;
        AppDomain.CurrentDomain.UnhandledException += (_, eventArgs) => OnCurrentDomainUnhandledException(eventArgs);
        TaskScheduler.UnobservedTaskException += (_, eventArgs) => OnUnobservedTaskException(eventArgs);
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

        _host = CreateHost(CreateRuntimeHostOptions(cliMode));
        await _host.StartAsync();

        if (await TryRunCliModeAsync(commandLineArgs, cliMode))
        {
            return;
        }

        ActivateMainWindow();
    }

    private static RuntimeHostOptions CreateRuntimeHostOptions(bool cliMode)
    {
        RuntimeHostOptions options = new()
        {
            EnableRuntimeLoop = !cliMode,
            DefaultAdminMode = true,
            DefaultSortColumn = Core.Domain.SortColumn.CpuPct,
            DefaultSortDirection = Core.Domain.SortDirection.Desc,
            DefaultFilterText = string.Empty,
            DefaultMetricTrendWindowSeconds = 60,
        };

        return RuntimeHostOptionsValidator.Validate(options);
    }

    private static IHost CreateHost(RuntimeHostOptions runtimeHostOptions)
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
                services.AddBatCaveRuntimeServices(runtimeHostOptions);
                services.AddBatCaveUiServices();
            })
            .Build();
    }

    private async Task<bool> TryRunCliModeAsync(string[] commandLineArgs, bool cliMode)
    {
        if (!cliMode || _host is null)
        {
            return false;
        }

        ICliOperationsHost cliOperationsHost = _host.Services.GetRequiredService<ICliOperationsHost>();
        int exitCode;
        if (commandLineArgs.Any(argument => string.Equals(argument, "--print-runtime-health", StringComparison.OrdinalIgnoreCase)))
        {
            RuntimeHealthSnapshot snapshot = _host.Services.GetRequiredService<IRuntimeHealthService>().Snapshot();
            Console.WriteLine(JsonSerializer.Serialize(snapshot, JsonDefaults.SnakeCase));
            exitCode = 0;
        }
        else
        {
            exitCode = WinUiBenchmarkCliRunner.IsBenchmarkCommand(commandLineArgs)
                ? await WinUiBenchmarkCliRunner.ExecuteAsync(_host.Services, commandLineArgs, CancellationToken.None)
                : await cliOperationsHost.ExecuteAsync(commandLineArgs, CancellationToken.None);
        }

        await ShutdownHostAsync();
        Environment.ExitCode = exitCode;
        Exit();
        return true;
    }

    private static bool IsCliMode(string[] args)
    {
        return args.Any(CliModeFlags.Contains);
    }

    private void ActivateMainWindow()
    {
        if (_host is null)
        {
            throw new InvalidOperationException("Application host has not been initialized.");
        }

        _window = _host.Services.GetRequiredService<MainWindow>();
        _window.Closed += OnWindowClosed;
        _window.Activate();
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

        IHost? host = _host;
        _host = null;
        if (host is not null)
        {
            await host.StopAsync();
            host.Dispose();
        }

        Log.CloseAndFlush();
    }

    private void OnUnhandledException(object sender, Microsoft.UI.Xaml.UnhandledExceptionEventArgs e)
    {
        ILogger<App>? logger = _host?.Services.GetService<ILogger<App>>();
        if (logger is not null)
        {
            logger.LogError(e.Exception, "Unhandled UI exception: {Message}", e.Message);
            return;
        }

        Log.Error(e.Exception, "Unhandled UI exception: {Message}", e.Message);
    }

    private void OnCurrentDomainUnhandledException(System.UnhandledExceptionEventArgs e)
    {
        ILogger<App>? logger = _host?.Services.GetService<ILogger<App>>();
        if (e.ExceptionObject is Exception exception)
        {
            if (logger is not null)
            {
                logger.LogCritical(exception, "Unhandled domain exception. IsTerminating={IsTerminating}", e.IsTerminating);
            }
            else
            {
                Log.Fatal(exception, "Unhandled domain exception. IsTerminating={IsTerminating}", e.IsTerminating);
            }

            return;
        }

        if (logger is not null)
        {
            logger.LogCritical(
                "Unhandled domain exception object of type {Type}. IsTerminating={IsTerminating}",
                e.ExceptionObject?.GetType().FullName ?? "unknown",
                e.IsTerminating);
        }
        else
        {
            Log.Fatal(
                "Unhandled domain exception object of type {Type}. IsTerminating={IsTerminating}",
                e.ExceptionObject?.GetType().FullName ?? "unknown",
                e.IsTerminating);
        }
    }

    private void OnUnobservedTaskException(UnobservedTaskExceptionEventArgs e)
    {
        ILogger<App>? logger = _host?.Services.GetService<ILogger<App>>();
        if (logger is not null)
        {
            logger.LogError(e.Exception, "Unobserved task exception.");
        }
        else
        {
            Log.Error(e.Exception, "Unobserved task exception.");
        }

        e.SetObserved();
    }
}
