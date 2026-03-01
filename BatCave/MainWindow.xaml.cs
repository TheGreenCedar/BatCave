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
    private bool _restoringSelection;

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
        if (_restoringSelection || sender is not ListView listView)
        {
            return;
        }

        if (listView.SelectedItem is ProcessRowViewState selected)
        {
            await ViewModel.ToggleSelectionAsync(selected.Sample, CancellationToken.None);
            return;
        }

        // Virtualization can emit a transient null selection while the selected row container is recycled.
        // Keep list selection visuals aligned with ViewModel state when a selected row is still tracked.
        if (ViewModel.SelectedVisibleRow is not null)
        {
            _restoringSelection = true;
            try
            {
                listView.SelectedItem = ViewModel.SelectedVisibleRow;
            }
            finally
            {
                _restoringSelection = false;
            }
        }
    }

    private void ClearSelection_Click(object sender, RoutedEventArgs e)
    {
        ViewModel.ClearSelection();
    }

    private void MetricChip_Click(object sender, RoutedEventArgs e)
    {
        if (sender is not FrameworkElement { Tag: string tag } || !Enum.TryParse(tag, out DetailMetricFocus focus))
        {
            return;
        }

        ViewModel.MetricFocus = focus;
    }
}
