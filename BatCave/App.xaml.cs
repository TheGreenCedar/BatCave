using BatCave.Core.Operations;
using BatCave.Core.Persistence;
using BatCave.Core.Runtime;
using BatCave.Core.Serialization;
using BatCave.Hosting;
using BatCave.Services;
using BatCave.ViewModels;
using Microsoft.Extensions.DependencyInjection;
using Microsoft.Extensions.Hosting;
using Microsoft.Extensions.Logging;
using Microsoft.UI.Text;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
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

        try
        {
            _host = CreateHost(BuildRuntimeHostOptions(cliMode));
            await _host.StartAsync();
        }
        catch (Exception ex) when (!cliMode)
        {
            RouteStartupFailureToShell(ex);
            return;
        }

        if (await TryRunCliModeAsync(commandLineArgs, cliMode))
        {
            return;
        }

        ActivateMainWindow();
    }

    private static RuntimeHostOptions BuildRuntimeHostOptions(bool cliMode)
    {
        return new RuntimeHostOptions
        {
            EnableRuntimeLoop = !cliMode,
            DefaultAdminMode = true,
            DeferAdminModeAtStartup = !cliMode,
            DefaultSortColumn = Core.Domain.SortColumn.CpuPct,
            DefaultSortDirection = Core.Domain.SortDirection.Desc,
            DefaultFilterText = string.Empty,
            DefaultMetricTrendWindowSeconds = 60,
        };
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

    private void RouteStartupFailureToShell(Exception ex)
    {
        if (_host is null)
        {
            Log.Error(ex, "Interactive startup failure occurred before the application host was created.");
            ShowHostConstructionFailureWindow(ex);
            return;
        }

        IHost host = _host;
        ILogger<App>? logger = host.Services.GetService<ILogger<App>>();
        if (logger is not null)
        {
            logger.LogError(ex, "Interactive startup failed before the monitor shell became live.");
        }
        else
        {
            Log.Error(ex, "Interactive startup failed before the monitor shell became live.");
        }

        MonitoringShellViewModel viewModel = host.Services.GetRequiredService<MonitoringShellViewModel>();
        viewModel.PresentStartupFailure(ex, RetryHostStartupAsync);
        ActivateMainWindow();
    }

    private void ShowHostConstructionFailureWindow(Exception ex)
    {
        Window fallbackWindow = _window ?? new Window();
        _window = fallbackWindow;

        TextBlock titleBlock = new()
        {
            Text = "Startup Error",
            FontSize = 24,
            FontWeight = FontWeights.SemiBold,
        };
        TextBlock messageBlock = new()
        {
            Text = "BatCave could not finish building the application host.",
            TextWrapping = TextWrapping.WrapWholeWords,
        };
        TextBlock detailBlock = new()
        {
            Text = ex.Message,
            TextWrapping = TextWrapping.WrapWholeWords,
        };
        Button retryButton = new()
        {
            Content = "Retry",
            HorizontalAlignment = HorizontalAlignment.Left,
        };
        retryButton.Click += async (_, _) =>
        {
            await RetryHostConstructionAsync(retryButton, titleBlock, messageBlock, detailBlock);
        };

        fallbackWindow.Content = new Grid
        {
            Padding = new Thickness(32),
            Children =
            {
                new StackPanel
                {
                    VerticalAlignment = VerticalAlignment.Center,
                    HorizontalAlignment = HorizontalAlignment.Center,
                    Spacing = 12,
                    MaxWidth = 720,
                    Children =
                    {
                        titleBlock,
                        messageBlock,
                        detailBlock,
                        retryButton,
                    },
                },
            },
        };
        fallbackWindow.Activate();
    }

    private async Task RetryHostConstructionAsync(
        Button retryButton,
        TextBlock titleBlock,
        TextBlock messageBlock,
        TextBlock detailBlock)
    {
        retryButton.IsEnabled = false;
        titleBlock.Text = "Retrying Startup";
        messageBlock.Text = "Rebuilding the application host.";

        try
        {
            _host = CreateHost(BuildRuntimeHostOptions(cliMode: false));
            await _host.StartAsync();

            Window? fallbackWindow = _window;
            _window = null;
            fallbackWindow?.Close();
            ActivateMainWindow();
        }
        catch (Exception ex)
        {
            if (_host is not null)
            {
                _host.Dispose();
                _host = null;
            }

            Log.Error(ex, "Interactive startup retry failed before the application host was created.");
            titleBlock.Text = "Startup Error";
            messageBlock.Text = "BatCave could not finish building the application host.";
            detailBlock.Text = ex.Message;
            retryButton.IsEnabled = true;
        }
    }

    private Task RetryHostStartupAsync(CancellationToken ct)
    {
        if (_host is null)
        {
            throw new InvalidOperationException("Application host is unavailable for startup retry.");
        }

        return _host.StartAsync(ct);
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
