using System;
using System.Threading;
using BatCave.ViewModels;
using Microsoft.Extensions.DependencyInjection;
using Microsoft.UI.Xaml;

namespace BatCave;

public sealed partial class MainWindow : Window
{
    private bool _bootstrapped;

    public MainWindow()
    {
        ViewModel = App.Services.GetRequiredService<MonitoringShellViewModel>();
        InitializeComponent();
        Activated += OnActivated;
    }

    public MonitoringShellViewModel ViewModel { get; }

    private async void OnActivated(object sender, WindowActivatedEventArgs args)
    {
        if (_bootstrapped)
        {
            return;
        }

        _bootstrapped = true;
        await ViewModel.BootstrapAsync(CancellationToken.None);
    }

    private async void AdminModeToggle_Toggled(object sender, RoutedEventArgs e)
    {
        await ViewModel.ToggleAdminModeAsync(AdminModeToggle.IsOn, CancellationToken.None);
    }
}
