using System;
using System.Threading;
using BatCave.Core.Domain;
using BatCave.ViewModels;
using Microsoft.Extensions.DependencyInjection;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;

namespace BatCave;

public sealed partial class MainWindow : Window
{
    private bool _bootstrapped;

    public MainWindow()
    {
        ViewModel = App.Services.GetRequiredService<MonitoringShellViewModel>();
        InitializeComponent();
        ViewModel.AttachDispatcherQueue(DispatcherQueue);
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
        if (sender is ToggleSwitch toggle)
        {
            await ViewModel.ToggleAdminModeAsync(toggle.IsOn, CancellationToken.None);
        }
    }

    private async void RetryBootstrap_Click(object sender, RoutedEventArgs e)
    {
        await ViewModel.RetryBootstrapAsync(CancellationToken.None);
    }

    private void SortHeader_Click(object sender, RoutedEventArgs e)
    {
        if (sender is FrameworkElement { Tag: string tag } && Enum.TryParse(tag, out SortColumn column))
        {
            ViewModel.ChangeSort(column);
        }
    }

    private async void ProcessListView_SelectionChanged(object sender, SelectionChangedEventArgs e)
    {
        if (sender is ListView listView)
        {
            if (listView.SelectedItem is not ProcessSample selected)
            {
                return;
            }

            await ViewModel.ToggleSelectionAsync(selected, CancellationToken.None);
        }
    }

    private void ClearSelection_Click(object sender, RoutedEventArgs e)
    {
        ViewModel.ClearSelection();
    }

    private void MetricFocusCombo_SelectionChanged(object sender, SelectionChangedEventArgs e)
    {
        if (sender is not ComboBox comboBox || comboBox.SelectedIndex < 0)
        {
            return;
        }

        ViewModel.MetricFocus = comboBox.SelectedIndex switch
        {
            0 => DetailMetricFocus.Cpu,
            1 => DetailMetricFocus.Memory,
            2 => DetailMetricFocus.Io,
            3 => DetailMetricFocus.Network,
            _ => DetailMetricFocus.Cpu,
        };
    }
}
