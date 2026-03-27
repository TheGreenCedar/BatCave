using BatCave.Layouts;
using BatCave.ViewModels;
using Microsoft.Extensions.DependencyInjection;
using Microsoft.UI;
using Microsoft.UI.Windowing;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Input;
using System;
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
    private long _selectionSettleProbeStartedAt;
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
        GlobalResourceListView.SelectedItem = ViewModel.SelectedGlobalResource;
        QueueCompactProcessInitialScrollIfNeeded();
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

    private void CompactProcessSortHeader_Click(object sender, RoutedEventArgs e)
    {
        DispatcherQueue.TryEnqueue(ScrollCompactProcessListToTop);
    }

    private void VisibleRows_CollectionChanged(object? sender, NotifyCollectionChangedEventArgs e)
    {
        QueueCompactProcessInitialScrollIfNeeded();
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
                DispatcherQueue.TryEnqueue(() =>
                {
                    if (!ReferenceEquals(GlobalResourceListView.SelectedItem, ViewModel.SelectedGlobalResource))
                    {
                        GlobalResourceListView.SelectedItem = ViewModel.SelectedGlobalResource;
                    }
                });
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
        if (ViewModel.VisibleRows is INotifyCollectionChanged visibleRows)
        {
            visibleRows.CollectionChanged -= VisibleRows_CollectionChanged;
        }

        ViewModel.GlobalCpuLogicalProcessorRows.CollectionChanged -= GlobalCpuLogicalProcessorRows_CollectionChanged;
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
