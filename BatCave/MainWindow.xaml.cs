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
    private const double WideMetricTrendBreakpoint = 1200;
    private const double WideMetricSidebarWidth = 248;

    private bool _bootstrapped;
    private bool _syncingSelectionVisual;

    public MainWindow()
    {
        ViewModel = App.Services.GetRequiredService<MonitoringShellViewModel>();
        InitializeComponent();
        ViewModel.AttachDispatcherQueue(DispatcherQueue);
        ViewModel.PropertyChanged += ViewModel_PropertyChanged;
        Activated += OnActivated;
        SizeChanged += OnWindowSizeChanged;
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
        ApplyMetricTrendLayoutForWindowWidth(GetWindowWidth());
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
            case nameof(MonitoringShellViewModel.SelectedVisibleRowBinding):
                SyncSelectionVisual();
                break;
            case nameof(MonitoringShellViewModel.CurrentSortColumn):
            case nameof(MonitoringShellViewModel.CurrentSortDirection):
                SyncSelectionVisual(deferSecondPass: true);
                break;
        }
    }

    private void SyncSelectionVisual(bool deferSecondPass = false)
    {
        if (_syncingSelectionVisual)
        {
            return;
        }

        _syncingSelectionVisual = true;
        try
        {
            if (!ReferenceEquals(ProcessListView.SelectedItem, ViewModel.SelectedVisibleRowBinding))
            {
                ProcessListView.SelectedItem = ViewModel.SelectedVisibleRowBinding;
            }
        }
        finally
        {
            _syncingSelectionVisual = false;
        }

        if (!deferSecondPass)
        {
            return;
        }

        DispatcherQueue.TryEnqueue(() =>
        {
            if (!_syncingSelectionVisual
                && !ReferenceEquals(ProcessListView.SelectedItem, ViewModel.SelectedVisibleRowBinding))
            {
                _syncingSelectionVisual = true;
                try
                {
                    ProcessListView.SelectedItem = ViewModel.SelectedVisibleRowBinding;
                }
                finally
                {
                    _syncingSelectionVisual = false;
                }
            }
        });
    }

    private async void ProcessListView_SelectionChanged(object sender, SelectionChangedEventArgs e)
    {
        if (_syncingSelectionVisual || sender is not ListView listView)
        {
            return;
        }

        if (listView.SelectedItem is ProcessRowViewState selected)
        {
            await ViewModel.SelectRowAsync(selected.Sample, CancellationToken.None);
            return;
        }

        if (ViewModel.SelectedVisibleRowBinding is not null)
        {
            // Ignore transient null churn from virtualization/sort transitions.
            SyncSelectionVisual(deferSecondPass: true);
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

    private void OnWindowSizeChanged(object sender, WindowSizeChangedEventArgs args)
    {
        ApplyMetricTrendLayoutForWindowWidth(args.Size.Width);
    }

    private double GetWindowWidth()
    {
        if (Content is FrameworkElement root && root.ActualWidth > 0)
        {
            return root.ActualWidth;
        }

        return AppWindow.Size.Width;
    }

    private void ApplyMetricTrendLayoutForWindowWidth(double windowWidth)
    {
        bool isWide = windowWidth >= WideMetricTrendBreakpoint;

        MetricSidebarColumn.Width = isWide
            ? new GridLength(WideMetricSidebarWidth)
            : new GridLength(1, GridUnitType.Star);
        MetricMainColumn.Width = isWide
            ? new GridLength(1, GridUnitType.Star)
            : new GridLength(0);
        MetricMainRow.Height = isWide
            ? new GridLength(0)
            : GridLength.Auto;

        Grid.SetRow(MetricMainHost, isWide ? 0 : 1);
        Grid.SetColumn(MetricMainHost, isWide ? 1 : 0);
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
        SizeChanged -= OnWindowSizeChanged;
        Closed -= OnWindowClosed;
    }
}
