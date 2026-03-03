using System;
using System.Collections.Generic;
using System.ComponentModel;
using System.Diagnostics;
using System.Threading;
using BatCave.ViewModels;
using Microsoft.Extensions.DependencyInjection;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Input;
using ScottPlot.WinUI;

namespace BatCave;

public sealed partial class MainWindow : Window
{
    private const double WideMetricTrendBreakpoint = 1200;
    private const double WideMetricSidebarWidth = 248;
    private const int MetricPlotBufferCapacity = 128;
    private const double AutoScaleMaterialShiftRatio = 0.35;

    private bool _bootstrapped;
    private bool _metricPlotRefreshQueued;
    private bool _syncingSelectionVisual;
    private bool _selectionSyncQueued;
    private bool _selectionSyncSecondPass;
    private long _selectionSettleProbeStartedAt;
    private MetricPlotDirtyFlags _dirtyMetricPlots = MetricPlotDirtyFlags.All;
    private readonly Dictionary<WinUIPlot, MetricPlotState> _metricPlotStates = [];

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
        _dirtyMetricPlots = MetricPlotDirtyFlags.All;
        ScheduleMetricPlotRefresh();
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
        refreshedAny |= RefreshMetricPlotIfDirty(dirty, MetricPlotDirtyFlags.Cpu, CpuChipPlot, ViewModel.CpuMetricTrendValues, lineWidth: 2f);
        refreshedAny |= RefreshMetricPlotIfDirty(dirty, MetricPlotDirtyFlags.Memory, MemoryChipPlot, ViewModel.MemoryMetricTrendValues, lineWidth: 2f);
        refreshedAny |= RefreshMetricPlotIfDirty(dirty, MetricPlotDirtyFlags.IoRead, IoReadChipPlot, ViewModel.IoReadMetricTrendValues, lineWidth: 2f);
        refreshedAny |= RefreshMetricPlotIfDirty(dirty, MetricPlotDirtyFlags.IoWrite, IoWriteChipPlot, ViewModel.IoWriteMetricTrendValues, lineWidth: 2f);
        refreshedAny |= RefreshMetricPlotIfDirty(dirty, MetricPlotDirtyFlags.OtherIo, OtherIoChipPlot, ViewModel.OtherIoMetricTrendValues, lineWidth: 2f);
        refreshedAny |= RefreshMetricPlotIfDirty(dirty, MetricPlotDirtyFlags.Expanded, ExpandedMetricPlot, ViewModel.ExpandedMetricTrendValues, lineWidth: 3f);

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

    private bool RefreshMetricPlotIfDirty(
        MetricPlotDirtyFlags dirty,
        MetricPlotDirtyFlags target,
        WinUIPlot plotControl,
        IReadOnlyList<double> values,
        float lineWidth)
    {
        if ((dirty & target) == 0)
        {
            return false;
        }

        MetricPlotState state = EnsureMetricPlotState(plotControl, lineWidth);
        bool hasSeriesChanges = UpdateMetricPlotBuffer(
            state,
            values,
            out int activePointCount,
            out double minValue,
            out double maxValue);

        if (state.Signal.LineWidth != lineWidth)
        {
            state.Signal.LineWidth = lineWidth;
            hasSeriesChanges = true;
        }

        bool forceAutoScale = !ReferenceEquals(state.LastValueSource, values);
        state.LastValueSource = values;

        bool requiresRender = hasSeriesChanges || forceAutoScale || !state.HasScaledBounds;
        if (!requiresRender)
        {
            return false;
        }

        state.ActivePointCount = activePointCount;
        state.Signal.MinRenderIndex = 0;
        state.Signal.MaxRenderIndex = Math.Max(1, activePointCount - 1);
        if (ShouldAutoScale(state, activePointCount, minValue, maxValue, forceAutoScale))
        {
            plotControl.Plot.Axes.AutoScale();
            state.ScaledMin = minValue;
            state.ScaledMax = maxValue;
            state.ScaledPointCount = activePointCount;
            state.HasScaledBounds = true;
        }

        plotControl.Refresh();
        return true;
    }

    private static bool UpdateMetricPlotBuffer(
        MetricPlotState state,
        IReadOnlyList<double> values,
        out int activePointCount,
        out double minValue,
        out double maxValue)
    {
        activePointCount = values.Count > 0 ? Math.Max(2, Math.Min(values.Count, state.Buffer.Length)) : 2;
        bool changed = state.ActivePointCount != activePointCount;

        if (values.Count == 0)
        {
            minValue = 0d;
            maxValue = 0d;
            return SetConstantBufferValue(state, 0d, changed);
        }

        if (values.Count == 1)
        {
            double single = values[0];
            minValue = single;
            maxValue = single;
            return SetConstantBufferValue(state, single, changed);
        }

        int sourceStartIndex = values.Count > state.Buffer.Length ? values.Count - state.Buffer.Length : 0;
        minValue = double.PositiveInfinity;
        maxValue = double.NegativeInfinity;
        for (int index = 0; index < activePointCount; index++)
        {
            double next = values[sourceStartIndex + index];
            minValue = Math.Min(minValue, next);
            maxValue = Math.Max(maxValue, next);
            if (state.Buffer[index] == next)
            {
                continue;
            }

            state.Buffer[index] = next;
            changed = true;
        }

        return changed;
    }

    private static bool SetConstantBufferValue(MetricPlotState state, double value, bool changed)
    {
        if (state.Buffer[0] != value)
        {
            state.Buffer[0] = value;
            changed = true;
        }

        if (state.Buffer[1] != value)
        {
            state.Buffer[1] = value;
            changed = true;
        }

        return changed;
    }

    private static bool ShouldAutoScale(
        MetricPlotState state,
        int activePointCount,
        double minValue,
        double maxValue,
        bool forceAutoScale)
    {
        if (forceAutoScale || !state.HasScaledBounds)
        {
            return true;
        }

        if (minValue < state.ScaledMin || maxValue > state.ScaledMax)
        {
            return true;
        }

        double previousRange = Math.Max(1e-9, state.ScaledMax - state.ScaledMin);
        double shiftedLowRatio = (minValue - state.ScaledMin) / previousRange;
        double shiftedHighRatio = (state.ScaledMax - maxValue) / previousRange;

        if (shiftedLowRatio >= AutoScaleMaterialShiftRatio || shiftedHighRatio >= AutoScaleMaterialShiftRatio)
        {
            return true;
        }

        return state.ScaledPointCount != activePointCount && activePointCount <= 2;
    }

    private MetricPlotState EnsureMetricPlotState(WinUIPlot plotControl, float lineWidth)
    {
        if (_metricPlotStates.TryGetValue(plotControl, out MetricPlotState? existing))
        {
            return existing;
        }

        ConfigureMetricPlot(plotControl);
        double[] buffer = new double[MetricPlotBufferCapacity];
        ScottPlot.Plottables.Signal signal = plotControl.Plot.Add.Signal(buffer);
        signal.LineWidth = lineWidth;
        signal.MinRenderIndex = 0;
        signal.MaxRenderIndex = 1;

        MetricPlotState created = new(signal, buffer);
        _metricPlotStates[plotControl] = created;
        return created;
    }

    private static void ConfigureMetricPlot(WinUIPlot plotControl)
    {
        plotControl.Plot.FigureBackground.Color = ScottPlot.Colors.Transparent;
        plotControl.Plot.DataBackground.Color = ScottPlot.Colors.Transparent;
        plotControl.Plot.Axes.Frameless();
        plotControl.Plot.Axes.Margins(0.02, 0.05);
        plotControl.Plot.HideGrid();
        plotControl.Plot.HideLegend();
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

    private sealed class MetricPlotState
    {
        public MetricPlotState(ScottPlot.Plottables.Signal signal, double[] buffer)
        {
            Signal = signal;
            Buffer = buffer;
            ActivePointCount = 2;
            ScaledPointCount = 2;
        }

        public ScottPlot.Plottables.Signal Signal { get; }

        public double[] Buffer { get; }

        public int ActivePointCount { get; set; }

        public IReadOnlyList<double>? LastValueSource { get; set; }

        public bool HasScaledBounds { get; set; }

        public double ScaledMin { get; set; }

        public double ScaledMax { get; set; }

        public int ScaledPointCount { get; set; }
    }
}
