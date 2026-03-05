using BatCave.Controls;
using BatCave.Layouts;
using BatCave.Styling;
using BatCave.ViewModels;
using Microsoft.Extensions.DependencyInjection;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Input;
using Microsoft.UI.Xaml.Media;
using System;
using System.Collections.Generic;
using System.Collections.Specialized;
using System.ComponentModel;
using System.Diagnostics;
using System.Threading;
using Windows.UI;

namespace BatCave;

public sealed partial class MainWindow : Window
{
    private const double WideMetricSidebarWidth = 248;
    private const double LogicalCpuTileTargetWidth = 170;
    private const double LogicalCpuTileTargetChartHeight = 120;
    private const double LogicalCpuTileLabelReserve = 20;
    private const double LogicalCpuTileMinWidth = 56;
    private const double LogicalCpuTileMinHeight = 28;
    private const double LogicalCpuTileItemMargin = 2;

    private bool _bootstrapped;
    private bool _metricPlotRefreshQueued;
    private long _selectionSettleProbeStartedAt;
    private MetricPlotDirtyFlags _dirtyMetricPlots = MetricPlotDirtyFlags.All;
    private ShellAdaptiveMode? _appliedShellAdaptiveMode;
    private bool _logicalCpuGridLayoutQueued;
    private int _logicalCpuGridLastCount = -1;
    private double _logicalCpuGridLastWidth = -1;
    private double _logicalCpuGridLastHeight = -1;
    private readonly Brush _cpuTrendStrokeBrush;
    private readonly Brush _cpuTrendFillBrush;
    private readonly Brush _memoryTrendStrokeBrush;
    private readonly Brush _memoryTrendFillBrush;
    private readonly Brush _ioReadTrendStrokeBrush;
    private readonly Brush _ioReadTrendFillBrush;
    private readonly Brush _ioWriteTrendStrokeBrush;
    private readonly Brush _ioWriteTrendFillBrush;
    private readonly Brush _otherIoTrendStrokeBrush;
    private readonly Brush _otherIoTrendFillBrush;

    public MainWindow()
    {
        ViewModel = App.Services.GetRequiredService<MonitoringShellViewModel>();
        _cpuTrendStrokeBrush = AppThemeTokens.ResolveBrush("ChartCpuStrokeBrush", Color.FromArgb(0xFF, 0x0B, 0x84, 0xD8));
        _cpuTrendFillBrush = AppThemeTokens.ResolveBrush("ChartCpuFillBrush", Color.FromArgb(0x33, 0x0B, 0x84, 0xD8));
        _memoryTrendStrokeBrush = AppThemeTokens.ResolveBrush("ChartMemoryStrokeBrush", Color.FromArgb(0xFF, 0x25, 0x63, 0xEB));
        _memoryTrendFillBrush = AppThemeTokens.ResolveBrush("ChartMemoryFillBrush", Color.FromArgb(0x33, 0x25, 0x63, 0xEB));
        _ioReadTrendStrokeBrush = AppThemeTokens.ResolveBrush("ChartIoReadStrokeBrush", Color.FromArgb(0xFF, 0x6A, 0x9F, 0x2A));
        _ioReadTrendFillBrush = AppThemeTokens.ResolveBrush("ChartIoReadFillBrush", Color.FromArgb(0x33, 0x6A, 0x9F, 0x2A));
        _ioWriteTrendStrokeBrush = AppThemeTokens.ResolveBrush("ChartIoWriteStrokeBrush", Color.FromArgb(0xFF, 0xD0, 0x7A, 0x00));
        _ioWriteTrendFillBrush = AppThemeTokens.ResolveBrush("ChartIoWriteFillBrush", Color.FromArgb(0x33, 0xD0, 0x7A, 0x00));
        _otherIoTrendStrokeBrush = AppThemeTokens.ResolveBrush("ChartOtherIoStrokeBrush", Color.FromArgb(0xFF, 0xD1, 0x34, 0x38));
        _otherIoTrendFillBrush = AppThemeTokens.ResolveBrush("ChartOtherIoFillBrush", Color.FromArgb(0x33, 0xD1, 0x34, 0x38));
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
        ConfigureMetricChartModes();
        GlobalResourceListView.SelectedItem = ViewModel.SelectedGlobalResource;
        _dirtyMetricPlots = MetricPlotDirtyFlags.All;
        ScheduleMetricPlotRefresh();
        ApplyMetricTrendLayoutForWindowWidth(GetWindowWidth());
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

        if (TryMarkMetricPlotDirty(e.PropertyName))
        {
            ScheduleMetricPlotRefresh();
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

    private void ScheduleMetricPlotRefresh()
    {
        if (_dirtyMetricPlots == MetricPlotDirtyFlags.None)
        {
            return;
        }

        if (_metricPlotRefreshQueued)
        {
            return;
        }

        _metricPlotRefreshQueued = true;
        DispatcherQueue.TryEnqueue(() =>
        {
            _metricPlotRefreshQueued = false;
            RefreshMetricPlots();
            if (_dirtyMetricPlots != MetricPlotDirtyFlags.None)
            {
                ScheduleMetricPlotRefresh();
            }
        });
    }

    private void RefreshMetricPlots()
    {
        MetricPlotDirtyFlags dirty = _dirtyMetricPlots;
        if (dirty == MetricPlotDirtyFlags.None)
        {
            return;
        }

        _dirtyMetricPlots = MetricPlotDirtyFlags.None;

        long startedAt = Stopwatch.GetTimestamp();
        bool refreshedAny = false;
        int visiblePointCount = ViewModel.MetricTrendWindowSeconds;
        ApplyExpandedMetricStyle();
        foreach (MetricPlotDescriptor descriptor in EnumerateMetricPlotDescriptors())
        {
            refreshedAny |= RefreshMetricPlotIfDirty(
                dirty,
                descriptor.DirtyFlag,
                descriptor.Chart,
                descriptor.Values,
                visiblePointCount);
        }

        if (refreshedAny)
        {
            ViewModel.RecordPlotRefreshProbe(Stopwatch.GetTimestamp() - startedAt);
        }
    }

    private void OnWindowSizeChanged(object sender, WindowSizeChangedEventArgs args)
    {
        ApplyMetricTrendLayoutForWindowWidth(args.Size.Width);
        ScheduleLogicalCpuGridLayout();
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
        ShellAdaptiveMode adaptiveMode = ShellAdaptiveLayout.Resolve(windowWidth);
        if (_appliedShellAdaptiveMode == adaptiveMode)
        {
            return;
        }

        _appliedShellAdaptiveMode = adaptiveMode;
        bool isWide = adaptiveMode == ShellAdaptiveMode.Wide;
        ApplyMetricTrendColumnLayout(isWide);

        Grid.SetRow(MetricMainHost, isWide ? 0 : 1);
        Grid.SetColumn(MetricMainHost, isWide ? 1 : 0);
    }

    private static bool RefreshMetricPlotIfDirty(
        MetricPlotDirtyFlags dirty,
        MetricPlotDirtyFlags target,
        MetricTrendChart chart,
        IReadOnlyList<double> values,
        int visiblePointCount)
    {
        if ((dirty & target) == 0)
        {
            return false;
        }
        if (!ReferenceEquals(chart.Values, values))
        {
            chart.Values = values;
        }

        if (chart.VisiblePointCount != visiblePointCount)
        {
            chart.VisiblePointCount = visiblePointCount;
        }

        chart.RequestRender();
        return true;
    }

    private void ApplyExpandedMetricStyle()
    {
        ExpandedMetricPlot.ScaleMode = ResolveExpandedScaleMode(ViewModel.MetricFocus);
        ExpandedMetricPlot.StrokeBrush = ResolveExpandedStrokeBrush(ViewModel.MetricFocus);
        ExpandedMetricPlot.FillBrush = ResolveExpandedFillBrush(ViewModel.MetricFocus);
    }

    private void ConfigureMetricChartModes()
    {
        ConfigureMetricChart(
            CpuChipPlot,
            MetricTrendScaleMode.CpuPercent,
            showGrid: false,
            _cpuTrendStrokeBrush,
            _cpuTrendFillBrush);
        ConfigureMetricChart(
            MemoryChipPlot,
            MetricTrendScaleMode.MemoryBytes,
            showGrid: false,
            _memoryTrendStrokeBrush,
            _memoryTrendFillBrush);
        ConfigureMetricChart(
            IoReadChipPlot,
            MetricTrendScaleMode.IoRate,
            showGrid: false,
            _ioReadTrendStrokeBrush,
            _ioReadTrendFillBrush);
        ConfigureMetricChart(
            IoWriteChipPlot,
            MetricTrendScaleMode.IoRate,
            showGrid: false,
            _ioWriteTrendStrokeBrush,
            _ioWriteTrendFillBrush);
        ConfigureMetricChart(
            OtherIoChipPlot,
            MetricTrendScaleMode.IoRate,
            showGrid: false,
            _otherIoTrendStrokeBrush,
            _otherIoTrendFillBrush);
        ConfigureMetricChart(
            ExpandedMetricPlot,
            MetricTrendScaleMode.CpuPercent,
            showGrid: true,
            _cpuTrendStrokeBrush,
            _cpuTrendFillBrush);
    }

    private IEnumerable<MetricPlotDescriptor> EnumerateMetricPlotDescriptors()
    {
        yield return new MetricPlotDescriptor(MetricPlotDirtyFlags.Cpu, CpuChipPlot, ViewModel.CpuMetricTrendValues);
        yield return new MetricPlotDescriptor(MetricPlotDirtyFlags.Memory, MemoryChipPlot, ViewModel.MemoryMetricTrendValues);
        yield return new MetricPlotDescriptor(MetricPlotDirtyFlags.IoRead, IoReadChipPlot, ViewModel.IoReadMetricTrendValues);
        yield return new MetricPlotDescriptor(MetricPlotDirtyFlags.IoWrite, IoWriteChipPlot, ViewModel.IoWriteMetricTrendValues);
        yield return new MetricPlotDescriptor(MetricPlotDirtyFlags.OtherIo, OtherIoChipPlot, ViewModel.OtherIoMetricTrendValues);
        yield return new MetricPlotDescriptor(MetricPlotDirtyFlags.Expanded, ExpandedMetricPlot, ViewModel.ExpandedMetricTrendValues);
    }

    private static void ConfigureMetricChart(
        MetricTrendChart chart,
        MetricTrendScaleMode scaleMode,
        bool showGrid,
        Brush strokeBrush,
        Brush fillBrush)
    {
        chart.ScaleMode = scaleMode;
        chart.ShowGrid = showGrid;
        chart.StrokeBrush = strokeBrush;
        chart.FillBrush = fillBrush;
    }

    private static MetricTrendScaleMode ResolveExpandedScaleMode(DetailMetricFocus metricFocus)
    {
        return metricFocus switch
        {
            DetailMetricFocus.Memory => MetricTrendScaleMode.MemoryBytes,
            DetailMetricFocus.IoRead => MetricTrendScaleMode.IoRate,
            DetailMetricFocus.IoWrite => MetricTrendScaleMode.IoRate,
            DetailMetricFocus.OtherIo => MetricTrendScaleMode.IoRate,
            _ => MetricTrendScaleMode.CpuPercent,
        };
    }

    private Brush ResolveExpandedStrokeBrush(DetailMetricFocus metricFocus)
    {
        return metricFocus switch
        {
            DetailMetricFocus.Memory => _memoryTrendStrokeBrush,
            DetailMetricFocus.IoRead => _ioReadTrendStrokeBrush,
            DetailMetricFocus.IoWrite => _ioWriteTrendStrokeBrush,
            DetailMetricFocus.OtherIo => _otherIoTrendStrokeBrush,
            _ => _cpuTrendStrokeBrush,
        };
    }

    private Brush ResolveExpandedFillBrush(DetailMetricFocus metricFocus)
    {
        return metricFocus switch
        {
            DetailMetricFocus.Memory => _memoryTrendFillBrush,
            DetailMetricFocus.IoRead => _ioReadTrendFillBrush,
            DetailMetricFocus.IoWrite => _ioWriteTrendFillBrush,
            DetailMetricFocus.OtherIo => _otherIoTrendFillBrush,
            _ => _cpuTrendFillBrush,
        };
    }

    private void GlobalCpuLogicalGridView_Loaded(object sender, RoutedEventArgs e)
    {
        ScheduleLogicalCpuGridLayout();
    }

    private void GlobalCpuLogicalGridView_SizeChanged(object sender, SizeChangedEventArgs e)
    {
        ScheduleLogicalCpuGridLayout();
    }

    private void GlobalCpuLogicalProcessorRows_CollectionChanged(object? sender, NotifyCollectionChangedEventArgs e)
    {
        ScheduleLogicalCpuGridLayout();
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
        if (GlobalCpuLogicalGridView.Visibility != Visibility.Visible)
        {
            return;
        }

        if (GlobalCpuLogicalGridView.ItemsPanelRoot is not ItemsWrapGrid itemsWrapGrid)
        {
            return;
        }

        int logicalProcessorCount = ViewModel.GlobalCpuLogicalProcessorRows.Count;
        if (logicalProcessorCount <= 0)
        {
            return;
        }

        double availableWidth = Math.Max(0, GlobalCpuLogicalGridView.ActualWidth - 4);
        double availableHeight = Math.Max(0, GlobalCpuLogicalGridView.ActualHeight - 4);
        if (availableWidth < 1 || availableHeight < 1)
        {
            return;
        }

        if (_logicalCpuGridLastCount == logicalProcessorCount &&
            Math.Abs(_logicalCpuGridLastWidth - availableWidth) < 0.5 &&
            Math.Abs(_logicalCpuGridLastHeight - availableHeight) < 0.5)
        {
            return;
        }

        (int columns, double itemWidth, double itemHeight) = ResolveLogicalCpuGridLayout(
            logicalProcessorCount,
            availableWidth,
            availableHeight);

        itemsWrapGrid.MaximumRowsOrColumns = columns;
        itemsWrapGrid.ItemWidth = itemWidth;
        itemsWrapGrid.ItemHeight = itemHeight;

        _logicalCpuGridLastCount = logicalProcessorCount;
        _logicalCpuGridLastWidth = availableWidth;
        _logicalCpuGridLastHeight = availableHeight;
    }

    private static (int Columns, double ItemWidth, double ItemHeight) ResolveLogicalCpuGridLayout(
        int itemCount,
        double availableWidth,
        double availableHeight)
    {
        int bestColumns = 1;
        double bestItemWidth = Math.Max(LogicalCpuTileMinWidth, availableWidth);
        double bestItemHeight = Math.Max(LogicalCpuTileMinHeight, availableHeight / itemCount);
        double bestScore = double.NegativeInfinity;

        for (int columns = 1; columns <= itemCount; columns++)
        {
            int rows = (itemCount + columns - 1) / columns;
            double horizontalMargins = columns * LogicalCpuTileItemMargin * 2d;
            double verticalMargins = rows * LogicalCpuTileItemMargin * 2d;
            double itemWidth = (availableWidth - horizontalMargins) / columns;
            double itemHeight = (availableHeight - verticalMargins) / rows;
            double chartHeight = itemHeight - LogicalCpuTileLabelReserve;
            if (itemWidth <= 0 || chartHeight <= 0)
            {
                continue;
            }

            double widthFit = itemWidth / LogicalCpuTileTargetWidth;
            double heightFit = chartHeight / LogicalCpuTileTargetChartHeight;
            double score = Math.Min(widthFit, heightFit);

            if (score > bestScore)
            {
                bestScore = score;
                bestColumns = columns;
                bestItemWidth = itemWidth;
                bestItemHeight = itemHeight;
            }
        }

        bestItemWidth = Math.Max(LogicalCpuTileMinWidth, bestItemWidth);
        bestItemHeight = Math.Max(LogicalCpuTileMinHeight, bestItemHeight);
        return (bestColumns, bestItemWidth, bestItemHeight);
    }

    private void OnWindowClosed(object sender, WindowEventArgs args)
    {
        ViewModel.PropertyChanged -= ViewModel_PropertyChanged;
        ViewModel.GlobalCpuLogicalProcessorRows.CollectionChanged -= GlobalCpuLogicalProcessorRows_CollectionChanged;
        SizeChanged -= OnWindowSizeChanged;
        Closed -= OnWindowClosed;
    }

    private void ApplyMetricTrendColumnLayout(bool isWide)
    {
        MetricSidebarColumn.Width = isWide
            ? new GridLength(WideMetricSidebarWidth)
            : new GridLength(1, GridUnitType.Star);
        MetricMainColumn.Width = isWide
            ? new GridLength(1, GridUnitType.Star)
            : new GridLength(0);
        MetricSidebarRow.Height = isWide
            ? new GridLength(1, GridUnitType.Star)
            : GridLength.Auto;
        MetricMainRow.Height = isWide
            ? new GridLength(0)
            : GridLength.Auto;
    }

    private bool TryMarkMetricPlotDirty(string propertyName)
    {
        if (!TryResolveMetricPlotFlag(propertyName, out MetricPlotDirtyFlags dirtyFlag))
        {
            return false;
        }

        _dirtyMetricPlots |= dirtyFlag;
        return true;
    }

    private static bool TryResolveMetricPlotFlag(string propertyName, out MetricPlotDirtyFlags dirtyFlag)
    {
        dirtyFlag = propertyName switch
        {
            nameof(MonitoringShellViewModel.CpuMetricTrendValues) => MetricPlotDirtyFlags.Cpu,
            nameof(MonitoringShellViewModel.MemoryMetricTrendValues) => MetricPlotDirtyFlags.Memory,
            nameof(MonitoringShellViewModel.IoReadMetricTrendValues) => MetricPlotDirtyFlags.IoRead,
            nameof(MonitoringShellViewModel.IoWriteMetricTrendValues) => MetricPlotDirtyFlags.IoWrite,
            nameof(MonitoringShellViewModel.OtherIoMetricTrendValues) => MetricPlotDirtyFlags.OtherIo,
            nameof(MonitoringShellViewModel.ExpandedMetricTrendValues) => MetricPlotDirtyFlags.Expanded,
            nameof(MonitoringShellViewModel.MetricFocus) => MetricPlotDirtyFlags.Expanded,
            nameof(MonitoringShellViewModel.MetricTrendWindowSeconds) => MetricPlotDirtyFlags.All,
            _ => MetricPlotDirtyFlags.None,
        };

        return dirtyFlag != MetricPlotDirtyFlags.None;
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

    [Flags]
    private enum MetricPlotDirtyFlags
    {
        None = 0,
        Cpu = 1 << 0,
        Memory = 1 << 1,
        IoRead = 1 << 2,
        IoWrite = 1 << 3,
        OtherIo = 1 << 4,
        Expanded = 1 << 5,
        All = Cpu | Memory | IoRead | IoWrite | OtherIo | Expanded,
    }

    private readonly record struct MetricPlotDescriptor(
        MetricPlotDirtyFlags DirtyFlag,
        MetricTrendChart Chart,
        IReadOnlyList<double> Values);
}
