using BatCave.Layouts;
using BatCave.Controls;
using BatCave.Converters;
using BatCave.ViewModels;
using Microsoft.Extensions.DependencyInjection;
using Microsoft.UI;
using Microsoft.UI.Windowing;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Input;
using Microsoft.UI.Xaml.Media;
using System;
using System.Collections.Generic;
using System.Collections.Specialized;
using System.ComponentModel;
using System.Diagnostics;
using System.IO;
using System.Threading;
using WinRT.Interop;
using Windows.UI;

namespace BatCave;

public sealed partial class MainWindow : Window
{
    private readonly AppWindow _appWindow;
    private bool _bootstrapped;
    private bool _titleBarConfigured;
    private bool _compactProcessInitialScrollPending = true;
    private bool _compactProcessSortPending;
    private bool _compactProcessSortFinalizeQueued;
    private bool _compactProcessSelectionRestoreQueued;
    private bool _syncingCompactProcessSelection;
    private bool _globalResourceSelectionRestoreQueued;
    private long _selectionSettleProbeStartedAt;
    private double? _compactProcessSortRestoreOffset;
    private ScrollViewer? _compactProcessListScrollViewer;
    private readonly ToolTip _inspectorChartToolTip = new();
    private Border? _activeInspectorChartOverlay;
    private bool _logicalCpuGridLayoutQueued;
    private int _logicalCpuGridLastCount = -1;
    private double _logicalCpuGridLastWidth = -1;
    public MainWindow()
    {
        ViewModel = App.Services.GetRequiredService<MonitoringShellViewModel>();
        InitializeComponent();
        IntPtr windowHandle = WindowNative.GetWindowHandle(this);
        WindowId windowId = Win32Interop.GetWindowIdFromWindow(windowHandle);
        _appWindow = AppWindow.GetFromWindowId(windowId);
        TryApplyWindowIcon();
        ViewModel.AttachDispatcherQueue(DispatcherQueue);
        ViewModel.PropertyChanged += ViewModel_PropertyChanged;
        if (ViewModel.VisibleRows is INotifyCollectionChanged visibleRows)
        {
            visibleRows.CollectionChanged += VisibleRows_CollectionChanged;
        }

        ViewModel.GlobalCpuLogicalProcessorRows.CollectionChanged += GlobalCpuLogicalProcessorRows_CollectionChanged;
        Activated += OnActivated;
        Closed += OnWindowClosed;
    }
    public MonitoringShellViewModel ViewModel { get; }

    private void TryApplyWindowIcon()
    {
        string iconPath = System.IO.Path.Combine(AppContext.BaseDirectory, "Assets", "BatCaveLogo.ico");
        if (!File.Exists(iconPath))
        {
            return;
        }

        _appWindow.SetIcon(iconPath);
    }

    private void ConfigureTitleBar()
    {
        ExtendsContentIntoTitleBar = true;
        SetTitleBar(TitleBarDragRegion);

        ApplyTitleBarButtonColors();
    }

    private void ShellRoot_ActualThemeChanged(FrameworkElement sender, object args)
    {
        ApplyTitleBarButtonColors();
        ApplyMotionPreferences();
    }

    private void ApplyTitleBarButtonColors()
    {
        if (!AppWindowTitleBar.IsCustomizationSupported())
        {
            return;
        }

        Color headerColor = ResolveThemeColor("BatCaveHeaderColor", new Color { A = 0xFF, R = 0x0A, G = 0x12, B = 0x20 });
        Color primaryTextColor = ResolveThemeColor("BatCaveTextPrimaryColor", Colors.White);
        Color secondaryTextColor = ResolveThemeColor("BatCaveTextSecondaryColor", new Color { A = 0xFF, R = 0xCB, G = 0xD5, B = 0xE1 });
        Color hoverSurfaceColor = ResolveThemeColor("BatCavePanelAltColor", new Color { A = 0xFF, R = 0x1A, G = 0x29, B = 0x40 });
        Color pressedSurfaceColor = ResolveThemeColor("BatCavePanelColor", new Color { A = 0xFF, R = 0x14, G = 0x20, B = 0x33 });
        AppWindowTitleBar titleBar = _appWindow.TitleBar;

        titleBar.BackgroundColor = headerColor;
        titleBar.ForegroundColor = primaryTextColor;
        titleBar.InactiveBackgroundColor = headerColor;
        titleBar.InactiveForegroundColor = secondaryTextColor;
        titleBar.ButtonBackgroundColor = headerColor;
        titleBar.ButtonInactiveBackgroundColor = headerColor;
        titleBar.ButtonForegroundColor = primaryTextColor;
        titleBar.ButtonInactiveForegroundColor = secondaryTextColor;
        titleBar.ButtonHoverBackgroundColor = hoverSurfaceColor;
        titleBar.ButtonHoverForegroundColor = primaryTextColor;
        titleBar.ButtonPressedBackgroundColor = pressedSurfaceColor;
        titleBar.ButtonPressedForegroundColor = primaryTextColor;
    }

    private static Color ResolveThemeColor(string resourceKey, Color fallback)
    {
        if (Application.Current.Resources.TryGetValue(resourceKey, out object? resource) &&
            resource is Color color)
        {
            return color;
        }

        return fallback;
    }

    private async void OnActivated(object sender, WindowActivatedEventArgs args)
    {
        ApplyMotionPreferences();

        if (!_titleBarConfigured)
        {
            ConfigureTitleBar();
            ShellRoot.ActualThemeChanged += ShellRoot_ActualThemeChanged;
            _titleBarConfigured = true;
        }

        if (_bootstrapped)
        {
            return;
        }

        _bootstrapped = true;
        await ViewModel.BootstrapAsync(CancellationToken.None);
        SyncAdminToggleState();
        SyncCompactProcessSelection();
        GlobalResourceListView.SelectedItem = ViewModel.SelectedGlobalResource;
        QueueCompactProcessInitialScrollIfNeeded();
        ScheduleLogicalCpuGridLayout();
    }

    private void ApplyMotionPreferences()
    {
        Windows.UI.ViewManagement.UISettings uiSettings = new Windows.UI.ViewManagement.UISettings();
        bool animationsEnabled = uiSettings.AnimationsEnabled;

        Application.Current.Resources["BatCaveInteractivePointerOverOpacity"] = animationsEnabled ? 0.94d : 1d;
        Application.Current.Resources["BatCaveInteractivePressedOpacity"] = animationsEnabled ? 0.84d : 1d;
        Application.Current.Resources["BatCaveInteractiveFastDuration"] = animationsEnabled
            ? new Duration(TimeSpan.FromMilliseconds(100))
            : new Duration(TimeSpan.Zero);
        Application.Current.Resources["BatCaveInteractivePressedDuration"] = animationsEnabled
            ? new Duration(TimeSpan.FromMilliseconds(50))
            : new Duration(TimeSpan.Zero);
        Application.Current.Resources["BatCaveChartSmoothTransitionsEnabled"] = animationsEnabled;
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
            case nameof(MonitoringShellViewModel.SelectedVisibleRowBinding):
                if (!_compactProcessSortPending)
                {
                    SyncCompactProcessSelection();
                }

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
        if (_syncingCompactProcessSelection)
        {
            CompleteSelectionSettleProbeIfPending();
            return;
        }

        if (sender is not ListView listView)
        {
            return;
        }

        if (listView.SelectedItem is ProcessRowViewState selected)
        {
            BeginSelectionSettleProbeIfNeeded();
            if (!ReferenceEquals(ViewModel.SelectedVisibleRowBinding, selected))
            {
                _ = ViewModel.SelectRowAsync(selected.Sample, CancellationToken.None);
            }

            DispatcherQueue.TryEnqueue(CompleteSelectionSettleProbeIfPending);
            return;
        }

        if (ViewModel.SelectedVisibleRowBinding is not null && ViewModel.VisibleRows.Count > 0)
        {
            // Ignore transient null churn from virtualization/sort transitions.
            BeginSelectionSettleProbeIfNeeded();
            if (!_compactProcessSortPending)
            {
                QueueCompactProcessSelectionRestore();
            }

            DispatcherQueue.TryEnqueue(CompleteSelectionSettleProbeIfPending);
            return;
        }

        CompleteSelectionSettleProbeIfPending();
    }

    private void CompactProcessSortHeader_Click(object sender, RoutedEventArgs e)
    {
        _compactProcessSortPending = true;
        _compactProcessSortRestoreOffset = TryGetCompactProcessScrollOffset();
        DispatcherQueue.TryEnqueue(CompleteCompactProcessSortInteraction);
    }

    private void VisibleRows_CollectionChanged(object? sender, NotifyCollectionChangedEventArgs e)
    {
        QueueCompactProcessInitialScrollIfNeeded();
        if (_compactProcessSortPending)
        {
            return;
        }

        if (ViewModel.SelectedVisibleRowBinding is not null && CompactProcessListView.SelectedItem is null)
        {
            QueueCompactProcessSelectionRestore();
        }
    }

    private void QueueCompactProcessInitialScrollIfNeeded()
    {
        if (!_compactProcessInitialScrollPending || ViewModel.VisibleRows.Count <= 0)
        {
            return;
        }

        _compactProcessInitialScrollPending = false;
        DispatcherQueue.TryEnqueue(ScrollCompactProcessListToTop);
    }

    private void ScrollCompactProcessListToTop()
    {
        if (ViewModel.VisibleRows.Count <= 0)
        {
            return;
        }

        CompactProcessListView.ScrollIntoView(ViewModel.VisibleRows[0], ScrollIntoViewAlignment.Leading);
    }

    private void QueueCompactProcessSelectionRestore()
    {
        if (_compactProcessSelectionRestoreQueued)
        {
            return;
        }

        _compactProcessSelectionRestoreQueued = true;
        DispatcherQueue.TryEnqueue(RestoreCompactProcessSelectionIfNeeded);
    }

    private void RestoreCompactProcessSelectionIfNeeded()
    {
        _compactProcessSelectionRestoreQueued = false;
        if (_compactProcessSortPending)
        {
            return;
        }

        SyncCompactProcessSelection();
    }

    private void CompleteCompactProcessSortInteraction()
    {
        if (!_compactProcessSortPending)
        {
            return;
        }

        SyncCompactProcessSelection();
        CompactProcessListView.LayoutUpdated -= CompactProcessListView_LayoutUpdated;
        CompactProcessListView.LayoutUpdated += CompactProcessListView_LayoutUpdated;
    }

    private void FinalizeCompactProcessSortInteraction()
    {
        _compactProcessSortFinalizeQueued = false;
        RestoreCompactProcessScrollOffsetIfNeeded();
        _compactProcessSortPending = false;
    }

    private void CompactProcessListView_LayoutUpdated(object? sender, object e)
    {
        CompactProcessListView.LayoutUpdated -= CompactProcessListView_LayoutUpdated;
        if (_compactProcessSortFinalizeQueued)
        {
            return;
        }

        _compactProcessSortFinalizeQueued = true;
        DispatcherQueue.TryEnqueue(FinalizeCompactProcessSortInteraction);
    }

    private void SyncCompactProcessSelection()
    {
        ProcessRowViewState? selectedVisibleRow = ViewModel.SelectedVisibleRowBinding;
        if (selectedVisibleRow is not null && !IsVisibleProcessRow(selectedVisibleRow))
        {
            selectedVisibleRow = null;
        }

        if (ReferenceEquals(CompactProcessListView.SelectedItem, selectedVisibleRow))
        {
            return;
        }

        try
        {
            _syncingCompactProcessSelection = true;
            CompactProcessListView.SelectedItem = selectedVisibleRow;
        }
        finally
        {
            _syncingCompactProcessSelection = false;
        }
    }

    private bool IsVisibleProcessRow(ProcessRowViewState row)
    {
        foreach (ProcessRowViewState visibleRow in ViewModel.VisibleRows)
        {
            if (ReferenceEquals(visibleRow, row))
            {
                return true;
            }
        }

        return false;
    }

    private double? TryGetCompactProcessScrollOffset()
    {
        ScrollViewer? scrollViewer = GetCompactProcessListScrollViewer();
        return scrollViewer?.VerticalOffset;
    }

    private void RestoreCompactProcessScrollOffsetIfNeeded()
    {
        if (_compactProcessSortRestoreOffset is not double verticalOffset)
        {
            return;
        }

        _compactProcessSortRestoreOffset = null;
        ScrollViewer? scrollViewer = GetCompactProcessListScrollViewer();
        scrollViewer?.ChangeView(null, verticalOffset, null, disableAnimation: true);
    }

    private ScrollViewer? GetCompactProcessListScrollViewer()
    {
        return _compactProcessListScrollViewer ??= FindDescendant<ScrollViewer>(CompactProcessListView);
    }

    private static T? FindDescendant<T>(DependencyObject? root) where T : DependencyObject
    {
        if (root is null)
        {
            return null;
        }

        int childCount = VisualTreeHelper.GetChildrenCount(root);
        for (int index = 0; index < childCount; index++)
        {
            DependencyObject child = VisualTreeHelper.GetChild(root, index);
            if (child is T target)
            {
                return target;
            }

            T? nested = FindDescendant<T>(child);
            if (nested is not null)
            {
                return nested;
            }
        }

        return null;
    }

    private void GlobalResourceListView_SelectionChanged(object sender, SelectionChangedEventArgs e)
    {
        if (sender is not ListView listView)
        {
            return;
        }

        try
        {
            if (listView.SelectedItem is GlobalResourceRowViewState selected)
            {
                if (!ReferenceEquals(ViewModel.SelectedGlobalResource, selected))
                {
                    ViewModel.SelectedGlobalResource = selected;
                }

                return;
            }

            if (ViewModel.SelectedGlobalResource is not null && ViewModel.GlobalResourceRows.Count > 0)
            {
                // Ignore transient null churn while the ListView rebinds during per-tick row refreshes.
                QueueGlobalResourceSelectionRestore();
                return;
            }

            if (ViewModel.SelectedGlobalResource is not null)
            {
                ViewModel.SelectedGlobalResource = null;
            }
        }
        catch (Exception ex)
        {
            Debug.WriteLine($"[GlobalSelection] Failed to apply selection from list view. {ex}");
            ViewModel.SelectedGlobalResource = null;
        }
    }

    private void GlobalCpuLogicalGridHost_SizeChanged(object sender, SizeChangedEventArgs e)
    {
        ScheduleLogicalCpuGridLayout();
    }

    private void GlobalCpuLogicalProcessorRows_CollectionChanged(object? sender, NotifyCollectionChangedEventArgs e)
    {
        ScheduleLogicalCpuGridLayout();
    }

    private void InspectorChartOverlay_PointerMoved(object sender, PointerRoutedEventArgs e)
    {
        if (sender is not Border overlay)
        {
            return;
        }

        string? tooltipText = TryBuildInspectorChartTooltip(overlay, e);
        if (string.IsNullOrWhiteSpace(tooltipText))
        {
            CloseInspectorChartTooltip();
            return;
        }

        if (!ReferenceEquals(_activeInspectorChartOverlay, overlay))
        {
            if (_activeInspectorChartOverlay is not null)
            {
                ToolTipService.SetToolTip(_activeInspectorChartOverlay, null);
            }

            _activeInspectorChartOverlay = overlay;
            ToolTipService.SetToolTip(overlay, _inspectorChartToolTip);
        }

        _inspectorChartToolTip.Content = tooltipText;
        _inspectorChartToolTip.IsOpen = true;
    }

    private void InspectorChartOverlay_PointerExited(object sender, PointerRoutedEventArgs e)
    {
        CloseInspectorChartTooltip();
    }

    private string? TryBuildInspectorChartTooltip(Border overlay, PointerRoutedEventArgs e)
    {
        if (overlay.ActualWidth <= 0d)
        {
            return null;
        }

        if (!TryResolveInspectorChartPayload(
                overlay.Tag as string,
                out IReadOnlyList<double>? values,
                out MetricTrendScaleMode scaleMode,
                out string primaryLabel,
                out IReadOnlyList<double>? secondaryValues,
                out string? secondaryLabel))
        {
            return null;
        }

        if (values is null || values.Count == 0)
        {
            return null;
        }

        double width = Math.Max(overlay.ActualWidth, 1d);
        double x = e.GetCurrentPoint(overlay).Position.X;
        double normalized = Math.Clamp(x / width, 0d, 1d);
        int sampleIndex = Math.Clamp((int)Math.Round(normalized * (values.Count - 1)), 0, values.Count - 1);
        double primaryValue = values[sampleIndex];
        if (!double.IsFinite(primaryValue))
        {
            return null;
        }

        string tooltipText = $"{primaryLabel}: {FormatChartValue(scaleMode, primaryValue)}";
        if (secondaryValues is not null &&
            secondaryValues.Count > sampleIndex &&
            double.IsFinite(secondaryValues[sampleIndex]))
        {
            tooltipText = $"{tooltipText}{Environment.NewLine}{secondaryLabel}: {FormatChartValue(scaleMode, secondaryValues[sampleIndex])}";
        }

        return tooltipText;
    }

    private bool TryResolveInspectorChartPayload(
        string? tag,
        out IReadOnlyList<double>? values,
        out MetricTrendScaleMode scaleMode,
        out string primaryLabel,
        out IReadOnlyList<double>? secondaryValues,
        out string? secondaryLabel)
    {
        values = null;
        secondaryValues = null;
        secondaryLabel = null;
        scaleMode = MetricTrendScaleMode.CpuPercent;
        primaryLabel = string.Empty;

        switch (tag)
        {
            case "SystemPrimary":
                values = ViewModel.GlobalPrimaryTrendValues;
                scaleMode = ViewModel.GlobalPrimaryScaleMode;
                primaryLabel = $"System {ViewModel.GlobalPrimaryChartTitle}";
                if (ViewModel.GlobalShowSecondaryOverlay && ViewModel.GlobalSecondaryTrendValues.Length > 0)
                {
                    secondaryValues = ViewModel.GlobalSecondaryTrendValues;
                    secondaryLabel = ResolveOverlayLabel(scaleMode);
                }

                return true;
            case "SystemAuxiliary":
                values = ViewModel.GlobalAuxiliaryTrendValues;
                scaleMode = ViewModel.GlobalAuxiliaryScaleMode;
                primaryLabel = $"System {ViewModel.GlobalAuxiliaryChartTitle}";
                return true;
            case "ProcessPrimary":
                values = ViewModel.GlobalPrimaryTrendValues;
                scaleMode = ViewModel.GlobalPrimaryScaleMode;
                primaryLabel = $"Process {ViewModel.GlobalPrimaryChartTitle}";
                if (ViewModel.GlobalShowSecondaryOverlay && ViewModel.GlobalSecondaryTrendValues.Length > 0)
                {
                    secondaryValues = ViewModel.GlobalSecondaryTrendValues;
                    secondaryLabel = ResolveOverlayLabel(scaleMode);
                }

                return true;
            default:
                return false;
        }
    }

    private static string ResolveOverlayLabel(MetricTrendScaleMode scaleMode)
    {
        return scaleMode switch
        {
            MetricTrendScaleMode.CpuPercent => "Kernel",
            MetricTrendScaleMode.BitsRate => "Receive",
            _ => "Overlay",
        };
    }

    private static string FormatChartValue(MetricTrendScaleMode scaleMode, double value)
    {
        return scaleMode switch
        {
            MetricTrendScaleMode.CpuPercent => $"{Math.Max(0d, value):F1}%",
            MetricTrendScaleMode.MemoryBytes => ValueFormat.FormatBytes(ClampToUlong(value)),
            MetricTrendScaleMode.IoRate => ValueFormat.FormatRate(ClampToUlong(value)),
            MetricTrendScaleMode.BitsRate => ValueFormat.FormatBitsRate(Math.Max(0d, value)),
            _ => value.ToString("F1"),
        };
    }

    private static ulong ClampToUlong(double value)
    {
        if (!double.IsFinite(value) || value <= 0d)
        {
            return 0UL;
        }

        return value >= ulong.MaxValue ? ulong.MaxValue : (ulong)Math.Round(value);
    }

    private void CloseInspectorChartTooltip()
    {
        _inspectorChartToolTip.IsOpen = false;
        if (_activeInspectorChartOverlay is not null)
        {
            ToolTipService.SetToolTip(_activeInspectorChartOverlay, null);
            _activeInspectorChartOverlay = null;
        }
    }

    private void ScheduleLogicalCpuGridLayout()
    {
        if (_logicalCpuGridLayoutQueued)
        {
            return;
        }

        _logicalCpuGridLayoutQueued = true;
        DispatcherQueue.TryEnqueue(ApplyQueuedLogicalCpuGridLayout);
    }

    private void QueueGlobalResourceSelectionRestore()
    {
        if (_globalResourceSelectionRestoreQueued)
        {
            return;
        }

        _globalResourceSelectionRestoreQueued = true;
        DispatcherQueue.TryEnqueue(RestoreGlobalResourceSelectionIfNeeded);
    }

    private void RestoreGlobalResourceSelectionIfNeeded()
    {
        _globalResourceSelectionRestoreQueued = false;

        if (ViewModel.SelectedGlobalResource is null || ViewModel.GlobalResourceRows.Count == 0)
        {
            return;
        }

        if (!ReferenceEquals(GlobalResourceListView.SelectedItem, ViewModel.SelectedGlobalResource))
        {
            GlobalResourceListView.SelectedItem = ViewModel.SelectedGlobalResource;
        }
    }

    private void ApplyQueuedLogicalCpuGridLayout()
    {
        _logicalCpuGridLayoutQueued = false;
        ApplyLogicalCpuGridLayout();
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

        double scrollerWidth = GlobalCpuLogicalGridScroller.ActualWidth;
        double hostWidth = GlobalCpuLogicalGridHost.ActualWidth;
        double availableWidth = scrollerWidth > 1d
            ? scrollerWidth
            : Math.Max(0d, hostWidth - 24d);
        if (availableWidth < 1)
        {
            return;
        }

        if (_logicalCpuGridLastCount == logicalProcessorCount &&
            Math.Abs(_logicalCpuGridLastWidth - availableWidth) < 0.5)
        {
            return;
        }

        LogicalCpuGridLayoutResult layout = LogicalCpuGridLayout.Resolve(
            logicalProcessorCount,
            availableWidth,
            double.PositiveInfinity);

        GlobalCpuLogicalUniformLayout.MaximumRowsOrColumns = layout.Columns;
        GlobalCpuLogicalUniformLayout.MinColumnSpacing = LogicalCpuGridLayout.TileItemMargin * 2d;
        GlobalCpuLogicalUniformLayout.MinRowSpacing = LogicalCpuGridLayout.TileItemMargin * 2d;
        GlobalCpuLogicalUniformLayout.MinItemWidth = layout.ItemWidth;
        GlobalCpuLogicalUniformLayout.MinItemHeight = layout.ItemHeight;

        _logicalCpuGridLastCount = logicalProcessorCount;
        _logicalCpuGridLastWidth = availableWidth;
    }

    private void OnWindowClosed(object sender, WindowEventArgs args)
    {
        if (_titleBarConfigured)
        {
            ShellRoot.ActualThemeChanged -= ShellRoot_ActualThemeChanged;
        }

        ViewModel.PropertyChanged -= ViewModel_PropertyChanged;
        CompactProcessListView.LayoutUpdated -= CompactProcessListView_LayoutUpdated;
        if (ViewModel.VisibleRows is INotifyCollectionChanged visibleRows)
        {
            visibleRows.CollectionChanged -= VisibleRows_CollectionChanged;
        }

        ViewModel.GlobalCpuLogicalProcessorRows.CollectionChanged -= GlobalCpuLogicalProcessorRows_CollectionChanged;
        Closed -= OnWindowClosed;
        CloseInspectorChartTooltip();
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
