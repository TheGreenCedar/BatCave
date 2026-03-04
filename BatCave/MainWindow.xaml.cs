using BatCave.Controls;
using BatCave.ViewModels;
using Microsoft.Extensions.DependencyInjection;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Input;
using Microsoft.UI.Xaml.Media;
using System;
using System.Collections.Generic;
using System.ComponentModel;
using System.Diagnostics;
using System.Threading;
using Windows.UI;

namespace BatCave;

public sealed partial class MainWindow : Window
{
    private const double WideMetricTrendBreakpoint = 1200;
    private const double WideMetricSidebarWidth = 248;
    private static readonly Brush CpuTrendStrokeBrush = CreateBrush(0xFF, 0x0B, 0x84, 0xD8);
    private static readonly Brush CpuTrendFillBrush = CreateBrush(0x33, 0x0B, 0x84, 0xD8);
    private static readonly Brush MemoryTrendStrokeBrush = CreateBrush(0xFF, 0x25, 0x63, 0xEB);
    private static readonly Brush MemoryTrendFillBrush = CreateBrush(0x33, 0x25, 0x63, 0xEB);
    private static readonly Brush IoReadTrendStrokeBrush = CreateBrush(0xFF, 0x6A, 0x9F, 0x2A);
    private static readonly Brush IoReadTrendFillBrush = CreateBrush(0x33, 0x6A, 0x9F, 0x2A);
    private static readonly Brush IoWriteTrendStrokeBrush = CreateBrush(0xFF, 0xD0, 0x7A, 0x00);
    private static readonly Brush IoWriteTrendFillBrush = CreateBrush(0x33, 0xD0, 0x7A, 0x00);
    private static readonly Brush OtherIoTrendStrokeBrush = CreateBrush(0xFF, 0xD1, 0x34, 0x38);
    private static readonly Brush OtherIoTrendFillBrush = CreateBrush(0x33, 0xD1, 0x34, 0x38);
    private static readonly Brush MetricGridBrush = CreateBrush(0x4C, 0xA0, 0xA8, 0xB8);

    private bool _bootstrapped;
    private bool _metricPlotRefreshQueued;
    private bool _syncingSelectionVisual;
    private bool _selectionSyncQueued;
    private bool _selectionSyncSecondPass;
    private long _selectionSettleProbeStartedAt;
    private MetricPlotDirtyFlags _dirtyMetricPlots = MetricPlotDirtyFlags.All;

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
        SyncAdminToggleState();
        ConfigureMetricChartModes();
        GlobalResourceListView.SelectedItem = ViewModel.SelectedGlobalResource;
        _dirtyMetricPlots = MetricPlotDirtyFlags.All;
        ScheduleMetricPlotRefresh();
        ApplyMetricTrendLayoutForWindowWidth(GetWindowWidth());
    }

    private async void AdminModeToggle_Toggled(object sender, RoutedEventArgs e)
    {
        if (sender is not ToggleSwitch toggle)
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

    private async void RetryBootstrap_Click(object sender, RoutedEventArgs e)
    {
        await ViewModel.RetryBootstrapAsync(CancellationToken.None);
    }

    private void FocusFilterAccelerator_Invoked(KeyboardAccelerator sender, KeyboardAcceleratorInvokedEventArgs args)
    {
        args.Handled = true;
        FilterTextBox.Focus(FocusState.Programmatic);
        FilterTextBox.SelectAll();
    }

    private async void RetryAccelerator_Invoked(KeyboardAccelerator sender, KeyboardAcceleratorInvokedEventArgs args)
    {
        args.Handled = true;
        if (RetryBootstrapButton.Visibility == Visibility.Visible && RetryBootstrapButton.IsEnabled)
        {
            await ViewModel.RetryBootstrapAsync(CancellationToken.None);
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
            case nameof(MonitoringShellViewModel.SelectedVisibleRowBinding):
                QueueSelectionVisualSync();
                break;
            case nameof(MonitoringShellViewModel.CurrentSortColumn):
            case nameof(MonitoringShellViewModel.CurrentSortDirection):
                QueueSelectionVisualSync(includeSecondPass: true);
                break;
        }
    }

    private void QueueSelectionVisualSync(bool includeSecondPass = false)
    {
        if (includeSecondPass)
        {
            _selectionSyncSecondPass = true;
            BeginSelectionSettleProbeIfNeeded();
        }

        if (_selectionSyncQueued)
        {
            return;
        }

        _selectionSyncQueued = true;
        DispatcherQueue.TryEnqueue(() =>
        {
            _selectionSyncQueued = false;
            _ = TrySyncSelectionVisual();

            if (_selectionSyncSecondPass)
            {
                _selectionSyncSecondPass = false;
                DispatcherQueue.TryEnqueue(() =>
                {
                    _ = TrySyncSelectionVisual();
                    CompleteSelectionSettleProbeIfPending();
                });
                return;
            }

            CompleteSelectionSettleProbeIfPending();
        });
    }

    private bool CanInteractWithAdminToggle()
    {
        return ViewModel.IsLive && !ViewModel.AdminModePending;
    }

    private void SyncAdminToggleState()
    {
        AdminModeToggle.IsEnabled = CanInteractWithAdminToggle();
    }

    private async void ProcessListView_SelectionChanged(object sender, SelectionChangedEventArgs e)
    {
        if (_syncingSelectionVisual || sender is not ListView listView)
        {
            return;
        }

        if (listView.SelectedItem is ProcessRowViewState selected)
        {
            BeginSelectionSettleProbeIfNeeded();
            await ViewModel.SelectRowAsync(selected.Sample, CancellationToken.None);
            QueueSelectionVisualSync();
            return;
        }

        if (ViewModel.SelectedVisibleRowBinding is not null)
        {
            // Ignore transient null churn from virtualization/sort transitions.
            BeginSelectionSettleProbeIfNeeded();
            QueueSelectionVisualSync(includeSecondPass: true);
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
            CpuTrendStrokeBrush,
            CpuTrendFillBrush,
            strokeThickness: 1.4);
        ConfigureMetricChart(
            MemoryChipPlot,
            MetricTrendScaleMode.MemoryBytes,
            showGrid: false,
            MemoryTrendStrokeBrush,
            MemoryTrendFillBrush,
            strokeThickness: 1.4);
        ConfigureMetricChart(
            IoReadChipPlot,
            MetricTrendScaleMode.IoRate,
            showGrid: false,
            IoReadTrendStrokeBrush,
            IoReadTrendFillBrush,
            strokeThickness: 1.4);
        ConfigureMetricChart(
            IoWriteChipPlot,
            MetricTrendScaleMode.IoRate,
            showGrid: false,
            IoWriteTrendStrokeBrush,
            IoWriteTrendFillBrush,
            strokeThickness: 1.4);
        ConfigureMetricChart(
            OtherIoChipPlot,
            MetricTrendScaleMode.IoRate,
            showGrid: false,
            OtherIoTrendStrokeBrush,
            OtherIoTrendFillBrush,
            strokeThickness: 1.4);
        ConfigureMetricChart(
            ExpandedMetricPlot,
            MetricTrendScaleMode.CpuPercent,
            showGrid: true,
            CpuTrendStrokeBrush,
            CpuTrendFillBrush,
            strokeThickness: 1.8);
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
        Brush fillBrush,
        double strokeThickness)
    {
        chart.ScaleMode = scaleMode;
        chart.ShowGrid = showGrid;
        chart.GridBrush = MetricGridBrush;
        chart.StrokeBrush = strokeBrush;
        chart.FillBrush = fillBrush;
        chart.StrokeThickness = strokeThickness;
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
            DetailMetricFocus.Memory => MemoryTrendStrokeBrush,
            DetailMetricFocus.IoRead => IoReadTrendStrokeBrush,
            DetailMetricFocus.IoWrite => IoWriteTrendStrokeBrush,
            DetailMetricFocus.OtherIo => OtherIoTrendStrokeBrush,
            _ => CpuTrendStrokeBrush,
        };
    }

    private Brush ResolveExpandedFillBrush(DetailMetricFocus metricFocus)
    {
        return metricFocus switch
        {
            DetailMetricFocus.Memory => MemoryTrendFillBrush,
            DetailMetricFocus.IoRead => IoReadTrendFillBrush,
            DetailMetricFocus.IoWrite => IoWriteTrendFillBrush,
            DetailMetricFocus.OtherIo => OtherIoTrendFillBrush,
            _ => CpuTrendFillBrush,
        };
    }

    private static Brush CreateBrush(byte a, byte r, byte g, byte b)
    {
        return new SolidColorBrush(Color.FromArgb(a, r, g, b));
    }

    private void OnWindowClosed(object sender, WindowEventArgs args)
    {
        ViewModel.PropertyChanged -= ViewModel_PropertyChanged;
        SizeChanged -= OnWindowSizeChanged;
        Closed -= OnWindowClosed;
    }

    private bool TrySyncSelectionVisual()
    {
        if (_syncingSelectionVisual)
        {
            return false;
        }

        _syncingSelectionVisual = true;
        try
        {
            SyncProcessListSelection();
        }
        finally
        {
            _syncingSelectionVisual = false;
        }

        return true;
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

    private void SyncProcessListSelection()
    {
        if (!ReferenceEquals(ProcessListView.SelectedItem, ViewModel.SelectedVisibleRowBinding))
        {
            ProcessListView.SelectedItem = ViewModel.SelectedVisibleRowBinding;
        }
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
