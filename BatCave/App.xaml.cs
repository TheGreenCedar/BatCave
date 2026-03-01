using System;
using System.Linq;
using System.Threading;
using System.Threading.Tasks;
using BatCave.Core.Operations;
using Microsoft.Extensions.DependencyInjection;
using Microsoft.Extensions.Hosting;
using Microsoft.UI.Xaml;

namespace BatCave;

public partial class App : Application
{
    private readonly IHost _host;
    private Window? _window;
    private bool _hostStopped;

    public App()
    {
        InitializeComponent();
        _host = CreateHost();
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

        _window = _host.Services.GetRequiredService<MainWindow>();
        _window.Closed += OnWindowClosed;
        _window.Activate();
    }

    private static IHost CreateHost()
    {
        return Host.CreateDefaultBuilder()
            .ConfigureServices(services =>
            {
                services.AddSingleton<ICliOperationsHost, CliOperationsHost>();
                services.AddSingleton<MainWindow>();
            })
            .Build();
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
        await _host.StopAsync();
        _host.Dispose();
    }
}
