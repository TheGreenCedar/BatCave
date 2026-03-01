using System;
using System.Collections.Generic;
using System.ComponentModel;
using System.Linq;
using System.Threading;
using BatCave.ViewModels;
using Microsoft.Extensions.DependencyInjection;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using ScottPlot.WinUI;

namespace BatCave;

public sealed partial class MainWindow : Window
{
    private bool _bootstrapped;

    public MainWindow()
    {
        ViewModel = App.Services.GetRequiredService<MonitoringShellViewModel>();
        InitializeComponent();
        ViewModel.AttachDispatcherQueue(DispatcherQueue);
        ViewModel.PropertyChanged += ViewModel_PropertyChanged;
        Activated += OnActivated;
        Closed += OnWindowClosed;
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
        RefreshMetricPlots();
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

    private void ViewModel_PropertyChanged(object? sender, PropertyChangedEventArgs e)
    {
        switch (e.PropertyName)
        {
            case nameof(MonitoringShellViewModel.CpuMetricTrendValues):
            case nameof(MonitoringShellViewModel.MemoryMetricTrendValues):
            case nameof(MonitoringShellViewModel.IoReadMetricTrendValues):
            case nameof(MonitoringShellViewModel.IoWriteMetricTrendValues):
            case nameof(MonitoringShellViewModel.NetworkMetricTrendValues):
            case nameof(MonitoringShellViewModel.ExpandedMetricTrendValues):
                RefreshMetricPlots();
                break;
        }
    }

    private void RefreshMetricPlots()
    {
        RenderMetricPlot(CpuChipPlot, ViewModel.CpuMetricTrendValues, lineWidth: 2f);
        RenderMetricPlot(MemoryChipPlot, ViewModel.MemoryMetricTrendValues, lineWidth: 2f);
        RenderMetricPlot(IoReadChipPlot, ViewModel.IoReadMetricTrendValues, lineWidth: 2f);
        RenderMetricPlot(IoWriteChipPlot, ViewModel.IoWriteMetricTrendValues, lineWidth: 2f);
        RenderMetricPlot(NetworkChipPlot, ViewModel.NetworkMetricTrendValues, lineWidth: 2f);
        RenderMetricPlot(ExpandedMetricPlot, ViewModel.ExpandedMetricTrendValues, lineWidth: 3f);
    }

    private static void RenderMetricPlot(WinUIPlot plotControl, IReadOnlyList<double> values, float lineWidth)
    {
        double[] series = values.Count > 0 ? values.ToArray() : [0d, 0d];

        plotControl.Plot.FigureBackground.Color = ScottPlot.Colors.Transparent;
        plotControl.Plot.DataBackground.Color = ScottPlot.Colors.Transparent;
        plotControl.Plot.Clear();
        var signal = plotControl.Plot.Add.Signal(series);
        signal.LineWidth = lineWidth;
        plotControl.Plot.Axes.Frameless();
        plotControl.Plot.Axes.Margins(0.02, 0.05);
        plotControl.Plot.HideGrid();
        plotControl.Plot.HideLegend();
        plotControl.Refresh();
    }

    private void OnWindowClosed(object sender, WindowEventArgs args)
    {
        ViewModel.PropertyChanged -= ViewModel_PropertyChanged;
        Closed -= OnWindowClosed;
    }
}
