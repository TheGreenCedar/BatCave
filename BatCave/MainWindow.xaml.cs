using BatCave.Layouts;
using BatCave.ViewModels;
using Microsoft.Extensions.DependencyInjection;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Input;
using System;
using System.Collections.Specialized;
using System.ComponentModel;
using System.Diagnostics;
using System.Threading;

namespace BatCave;

public sealed partial class MainWindow : Window
{

    private bool _bootstrapped;
    private long _selectionSettleProbeStartedAt;
    private bool _logicalCpuGridLayoutQueued;
    private int _logicalCpuGridLastCount = -1;
    private double _logicalCpuGridLastWidth = -1;
    private double _logicalCpuGridLastHeight = -1;
    private double _logicalCpuTileWidth = LogicalCpuGridLayout.TileTargetWidth;
    private double _logicalCpuTileHeight = LogicalCpuGridLayout.TileMinHeight;
    private double _logicalCpuTileChartHeight = LogicalCpuGridLayout.TileMinChartHeight;
    private double _lastChartSizingWindowHeight = -1;

    public MainWindow()
    {
        ViewModel = App.Services.GetRequiredService<MonitoringShellViewModel>();
        InitializeComponent();
        ViewModel.AttachDispatcherQueue(DispatcherQueue);
        ViewModel.PropertyChanged += ViewModel_PropertyChanged;
        ViewModel.GlobalCpuLogicalProcessorRows.CollectionChanged += GlobalCpuLogicalProcessorRows_CollectionChanged;
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
        SyncAdminToggleState();
        GlobalResourceListView.SelectedItem = ViewModel.SelectedGlobalResource;
        ScheduleLogicalCpuGridLayout();
    }

    private async void AdminModeToggle_Toggled(object sender, RoutedEventArgs e)
    {
        if (sender is not ToggleSwitch toggle)
        {
            return;
        }

        if (toggle.Visibility != Visibility.Visible)
        {
            return;
        }

        if (!CanInteractWithAdminToggle())
        {
            if (toggle.IsOn != ViewModel.AdminModeEnabled)
            {
                toggle.IsOn = ViewModel.AdminModeEnabled;
            }

            return;
        }

        try
        {
            await ViewModel.ToggleAdminModeAsync(toggle.IsOn, CancellationToken.None);
        }
        finally
        {
            SyncAdminToggleState();
        }
    }

    private void FocusFilterAccelerator_Invoked(KeyboardAccelerator sender, KeyboardAcceleratorInvokedEventArgs args)
    {
        args.Handled = true;
        TextBox target = HeaderControlsPhone.Visibility == Visibility.Visible
            ? FilterTextBoxPhone
            : FilterTextBox;
        target.Focus(FocusState.Programmatic);
        target.SelectAll();
    }

    private async void RetryAccelerator_Invoked(KeyboardAccelerator sender, KeyboardAcceleratorInvokedEventArgs args)
    {
        args.Handled = true;
        if (ViewModel.StartupErrorVisibility == Visibility.Visible
            && ViewModel.RetryBootstrapRequestedCommand.CanExecute(null))
        {
            await ViewModel.RetryBootstrapRequestedCommand.ExecuteAsync(null);
        }
    }

    private void ClearSelectionAccelerator_Invoked(KeyboardAccelerator sender, KeyboardAcceleratorInvokedEventArgs args)
    {
        args.Handled = true;
        if (ViewModel.ClearSelectionRequestedCommand.CanExecute(null))
        {
            ViewModel.ClearSelectionRequestedCommand.Execute(null);
        }
    }

    private void ViewModel_PropertyChanged(object? sender, PropertyChangedEventArgs e)
    {
        if (e.PropertyName is null)
        {
            return;
        }

        switch (e.PropertyName)
        {
            case nameof(MonitoringShellViewModel.IsLive):
            case nameof(MonitoringShellViewModel.AdminModePending):
            case nameof(MonitoringShellViewModel.AdminModeEnabled):
                SyncAdminToggleState();
                break;
            case nameof(MonitoringShellViewModel.GlobalCpuLogicalGridVisibility):
                ScheduleLogicalCpuGridLayout();
                break;
            case nameof(MonitoringShellViewModel.SelectedGlobalResource):
                if (!ReferenceEquals(GlobalResourceListView.SelectedItem, ViewModel.SelectedGlobalResource))
                {
                    GlobalResourceListView.SelectedItem = ViewModel.SelectedGlobalResource;
                }

                break;
        }
    }

    private bool CanInteractWithAdminToggle()
    {
        return ViewModel.IsLive && !ViewModel.AdminModePending;
    }

    private void SyncAdminToggleState()
    {
        bool canInteract = CanInteractWithAdminToggle();
        AdminModeToggle.IsEnabled = canInteract;
        AdminModeTogglePhone.IsEnabled = canInteract;
    }

    private void ProcessListView_SelectionChanged(object sender, SelectionChangedEventArgs e)
    {
        if (sender is not ListView listView)
        {
            return;
        }

        if (listView.SelectedItem is ProcessRowViewState selected)
        {
            BeginSelectionSettleProbeIfNeeded();
            _ = ViewModel.SelectRowAsync(selected.Sample, CancellationToken.None);
            DispatcherQueue.TryEnqueue(CompleteSelectionSettleProbeIfPending);
            return;
        }

        if (ViewModel.SelectedVisibleRowBinding is not null)
        {
            // Ignore transient null churn from virtualization/sort transitions.
            BeginSelectionSettleProbeIfNeeded();
            ViewModel.SelectedVisibleRowBinding = null;
            DispatcherQueue.TryEnqueue(CompleteSelectionSettleProbeIfPending);
            return;
        }

        CompleteSelectionSettleProbeIfPending();
    }

    private void GlobalResourceListView_SelectionChanged(object sender, SelectionChangedEventArgs e)
    {
        if (sender is not ListView listView)
        {
            return;
        }

        try
        {
            GlobalResourceRowViewState? selected = listView.SelectedItem as GlobalResourceRowViewState;
            if (!ReferenceEquals(ViewModel.SelectedGlobalResource, selected))
            {
                ViewModel.SelectedGlobalResource = selected;
            }
        }
        catch (Exception ex)
        {
            Debug.WriteLine($"[GlobalSelection] Failed to apply selection from list view. {ex}");
            ViewModel.SelectedGlobalResource = null;
        }
    }

    private void OnWindowSizeChanged(object sender, WindowSizeChangedEventArgs args)
    {
        ScheduleLogicalCpuGridLayout();
        ApplyInspectorChartSizing(args.Size.Height);
    }

    private void GlobalCpuLogicalRepeater_Loaded(object sender, RoutedEventArgs e)
    {
        ScheduleLogicalCpuGridLayout();
    }

    private void GlobalCpuLogicalGridHost_SizeChanged(object sender, SizeChangedEventArgs e)
    {
        ScheduleLogicalCpuGridLayout();
    }

    private void GlobalCpuLogicalProcessorRows_CollectionChanged(object? sender, NotifyCollectionChangedEventArgs e)
    {
        ScheduleLogicalCpuGridLayout();
    }

    private void GlobalCpuLogicalRepeater_ElementPrepared(ItemsRepeater sender, ItemsRepeaterElementPreparedEventArgs args)
    {
        if (args.Element is not FrameworkElement element)
        {
            return;
        }

        ApplyLogicalCpuTileSize(element);
    }

    private void ScheduleLogicalCpuGridLayout()
    {
        if (_logicalCpuGridLayoutQueued)
        {
            return;
        }

        _logicalCpuGridLayoutQueued = true;
        DispatcherQueue.TryEnqueue(() =>
        {
            _logicalCpuGridLayoutQueued = false;
            ApplyLogicalCpuGridLayout();
        });
    }

    private void ApplyLogicalCpuGridLayout()
    {
        if (GlobalCpuLogicalGridHost.Visibility != Visibility.Visible)
        {
            return;
        }

        if (GlobalCpuLogicalUniformLayout is null)
        {
            return;
        }

        int logicalProcessorCount = ViewModel.GlobalCpuLogicalProcessorRows.Count;
        if (logicalProcessorCount <= 0)
        {
            return;
        }

        double hostWidth = GlobalCpuLogicalGridHost.ActualWidth;
        double availableWidth = Math.Max(0d, hostWidth - 24d);
        if (availableWidth < 1)
        {
            return;
        }

        double hostHeight = Math.Max(0d, GlobalCpuLogicalGridHost.ActualHeight - 24d);
        double availableHeight = hostHeight > 1d ? hostHeight : double.PositiveInfinity;
        double layoutHeightSentinel = double.IsFinite(availableHeight) ? availableHeight : -1d;

        if (_logicalCpuGridLastCount == logicalProcessorCount &&
            Math.Abs(_logicalCpuGridLastWidth - availableWidth) < 0.5 &&
            Math.Abs(_logicalCpuGridLastHeight - layoutHeightSentinel) < 0.5)
        {
            return;
        }

        LogicalCpuGridLayoutResult layout = LogicalCpuGridLayout.Resolve(
            logicalProcessorCount,
            availableWidth,
            availableHeight);

        GlobalCpuLogicalUniformLayout.MaximumRowsOrColumns = layout.Columns;
        GlobalCpuLogicalUniformLayout.MinColumnSpacing = LogicalCpuGridLayout.TileItemMargin * 2d;
        GlobalCpuLogicalUniformLayout.MinRowSpacing = LogicalCpuGridLayout.TileItemMargin * 2d;
        GlobalCpuLogicalUniformLayout.MinItemWidth = layout.ItemWidth;
        GlobalCpuLogicalUniformLayout.MinItemHeight = layout.ItemHeight;

        _logicalCpuTileWidth = layout.ItemWidth;
        _logicalCpuTileHeight = layout.ItemHeight;
        _logicalCpuTileChartHeight = layout.ChartHeight;
        ApplyLogicalCpuTileSizeToRealizedElements();
        GlobalCpuLogicalRepeater.InvalidateMeasure();

        _logicalCpuGridLastCount = logicalProcessorCount;
        _logicalCpuGridLastWidth = availableWidth;
        _logicalCpuGridLastHeight = layoutHeightSentinel;
    }

    private void ApplyLogicalCpuTileSizeToRealizedElements()
    {
        int count = ViewModel.GlobalCpuLogicalProcessorRows.Count;
        for (int index = 0; index < count; index++)
        {
            if (GlobalCpuLogicalRepeater.TryGetElement(index) is not FrameworkElement element)
            {
                continue;
            }

            ApplyLogicalCpuTileSize(element);
        }
    }

    private void ApplyLogicalCpuTileSize(FrameworkElement element)
    {
        element.Width = _logicalCpuTileWidth;
        element.Height = _logicalCpuTileHeight;
        if (element.FindName("LogicalCpuTrendChart") is FrameworkElement trendChart)
        {
            trendChart.Height = _logicalCpuTileChartHeight;
        }
    }

    private void ApplyInspectorChartSizing(double windowHeight)
    {
        if (windowHeight <= 0)
        {
            return;
        }

        if (Math.Abs(_lastChartSizingWindowHeight - windowHeight) < 0.5d)
        {
            return;
        }

        _lastChartSizingWindowHeight = windowHeight;

        double inspectorVerticalBudget = Math.Max(windowHeight - 330d, 320d);
        double systemPrimaryHeight = Clamp(inspectorVerticalBudget * 0.58d, 240d, 560d);
        double processPrimaryHeight = Clamp(inspectorVerticalBudget * 0.64d, 260d, 600d);
        double auxiliaryHeight = Clamp(systemPrimaryHeight * 0.24d, 72d, 160d);
        double placeholderHeight = Clamp(processPrimaryHeight * 0.44d, 120d, 240d);

        SystemPrimaryTrendChart.Height = systemPrimaryHeight;
        SystemAuxTrendChart.Height = auxiliaryHeight;
        ProcessPrimaryTrendChart.Height = processPrimaryHeight;
        ProcessLogicalPlaceholder.MinHeight = placeholderHeight;
    }

    private static double Clamp(double value, double min, double max)
    {
        return Math.Min(max, Math.Max(min, value));
    }


    private void OnWindowClosed(object sender, WindowEventArgs args)
    {
        ViewModel.PropertyChanged -= ViewModel_PropertyChanged;
        ViewModel.GlobalCpuLogicalProcessorRows.CollectionChanged -= GlobalCpuLogicalProcessorRows_CollectionChanged;
        SizeChanged -= OnWindowSizeChanged;
        Closed -= OnWindowClosed;
    }

    private void BeginSelectionSettleProbeIfNeeded()
    {
        if (_selectionSettleProbeStartedAt <= 0)
        {
            _selectionSettleProbeStartedAt = Stopwatch.GetTimestamp();
        }
    }

    private void CompleteSelectionSettleProbeIfPending()
    {
        if (_selectionSettleProbeStartedAt <= 0)
        {
            return;
        }

        long startedAt = _selectionSettleProbeStartedAt;
        _selectionSettleProbeStartedAt = 0;
        ViewModel.RecordSelectionSettleProbe(Stopwatch.GetTimestamp() - startedAt);
    }
}

