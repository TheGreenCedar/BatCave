using BatCave.App.Presentation;
using BatCave.Runtime.Contracts;
using Microsoft.Extensions.DependencyInjection;
using Microsoft.UI.Dispatching;
using Microsoft.UI;
using Microsoft.UI.Text;
using Microsoft.UI.Windowing;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Input;
using Microsoft.UI.Xaml.Media;
using System;
using System.ComponentModel;
using System.IO;
using Windows.ApplicationModel.DataTransfer;
using Windows.Graphics;
using WinRT.Interop;

namespace BatCave.App;

public sealed partial class MainWindow : Window
{
    private const double WideLayoutMinWidth = 1100;
    private const double CompactProcessListMaxWidth = 1280;
    private const double InspectorPaneWidth = 420;
    private const double WorkspaceHorizontalPadding = 32;
    private const double NarrowProcessPaneMaxHeight = 360;
    private const int PreferredInitialWindowWidth = 1600;
    private const int PreferredInitialWindowHeight = 1000;
    private AppWindow? _appWindow;
    private DispatcherQueueTimer? _responsiveLayoutTimer;
    private double _lastResponsiveLayoutWidth = double.NaN;

    public MainWindow()
    {
        ViewModel = App.Services.GetRequiredService<ShellViewModel>();
        InitializeComponent();
        ViewModel.PropertyChanged += OnViewModelPropertyChanged;
        TryApplyWindowIcon();
        Closed += OnClosed;
    }

    public ShellViewModel ViewModel { get; }

    private void Root_Loaded(object sender, RoutedEventArgs e)
    {
        AttachAppWindowEvents();
        EnsureUsableInitialWindowSize();
        StartResponsiveLayoutWatcher();
        ViewModel.AttachDispatcherQueue(DispatcherQueue);
        ViewModel.Start();
        WorkflowNav.SelectedItem = OverviewNavItem;
        UpdateSortHeaderVisualState();
        ApplyResponsiveLayoutIfChanged(GetResponsiveWidth(Root.ActualWidth));
    }

    private void Root_SizeChanged(object sender, SizeChangedEventArgs e)
    {
        ApplyResponsiveLayoutIfChanged(GetResponsiveWidth(e.NewSize.Width));
    }

    private async void OnClosed(object sender, WindowEventArgs args)
    {
        ViewModel.PropertyChanged -= OnViewModelPropertyChanged;
        if (_appWindow is not null)
        {
            _appWindow.Changed -= AppWindow_Changed;
        }

        _responsiveLayoutTimer?.Stop();
        try
        {
            await ViewModel.DisposeAsync();
        }
        finally
        {
            await App.ShutdownServicesAsync();
        }
    }

    private void OnViewModelPropertyChanged(object? sender, PropertyChangedEventArgs e)
    {
        if (e.PropertyName is nameof(ShellViewModel.CurrentSortColumn)
            or nameof(ShellViewModel.CurrentSortDirection)
            or nameof(ShellViewModel.ProcessSortText))
        {
            UpdateSortHeaderVisualState();
        }
    }

    private void TryApplyWindowIcon()
    {
        string iconPath = Path.Combine(AppContext.BaseDirectory, "Assets", "BatCaveLogo.ico");
        if (!File.Exists(iconPath))
        {
            return;
        }

        GetAppWindow().SetIcon(iconPath);
    }

    private void AttachAppWindowEvents()
    {
        AppWindow appWindow = GetAppWindow();
        appWindow.Changed -= AppWindow_Changed;
        appWindow.Changed += AppWindow_Changed;
    }

    private void EnsureUsableInitialWindowSize()
    {
        AppWindow appWindow = GetAppWindow();
        DisplayArea displayArea = DisplayArea.GetFromWindowId(appWindow.Id, DisplayAreaFallback.Nearest);
        int workAreaWidth = displayArea.WorkArea.Width > 0 ? displayArea.WorkArea.Width : PreferredInitialWindowWidth;
        int workAreaHeight = displayArea.WorkArea.Height > 0 ? displayArea.WorkArea.Height : PreferredInitialWindowHeight;
        int targetWidth = Math.Min(PreferredInitialWindowWidth, workAreaWidth);
        int targetHeight = Math.Min(PreferredInitialWindowHeight, workAreaHeight);
        if (appWindow.Size.Width < Math.Min(WideLayoutMinWidth, targetWidth) || appWindow.Size.Height < 720)
        {
            appWindow.Resize(new SizeInt32(targetWidth, targetHeight));
        }
    }

    private AppWindow GetAppWindow()
    {
        if (_appWindow is not null)
        {
            return _appWindow;
        }

        IntPtr windowHandle = WindowNative.GetWindowHandle(this);
        WindowId windowId = Win32Interop.GetWindowIdFromWindow(windowHandle);
        _appWindow = AppWindow.GetFromWindowId(windowId);
        return _appWindow;
    }

    private double GetResponsiveWidth(double fallbackWidth)
    {
        if (fallbackWidth > 0)
        {
            return fallbackWidth;
        }

        AppWindow appWindow = GetAppWindow();
        return appWindow.Size.Width > 0 ? appWindow.Size.Width : fallbackWidth;
    }

    private void AppWindow_Changed(AppWindow sender, AppWindowChangedEventArgs args)
    {
        if (args.DidSizeChange)
        {
            ApplyResponsiveLayoutIfChanged(GetResponsiveWidth(Root.ActualWidth));
        }
    }

    private void StartResponsiveLayoutWatcher()
    {
        if (_responsiveLayoutTimer is not null)
        {
            return;
        }

        _responsiveLayoutTimer = DispatcherQueue.CreateTimer();
        _responsiveLayoutTimer.Interval = TimeSpan.FromMilliseconds(250);
        _responsiveLayoutTimer.Tick += (_, _) => ApplyResponsiveLayoutIfChanged(GetResponsiveWidth(Root.ActualWidth));
        _responsiveLayoutTimer.Start();
    }

    private async void AdminToggle_Toggled(object sender, RoutedEventArgs e)
    {
        if (sender is ToggleSwitch toggle && toggle.IsOn != ViewModel.AdminModeRequested)
        {
            await ViewModel.SetAdminModeAsync(toggle.IsOn);
        }
    }

    private async void SortButton_Click(object sender, RoutedEventArgs e)
    {
        if (sender is Button { Tag: string tag } && ViewModel.SortCommand.CanExecute(tag))
        {
            await ViewModel.SortCommand.ExecuteAsync(tag);
        }
    }

    private async void SortMenuItem_Click(object sender, RoutedEventArgs e)
    {
        if (sender is MenuFlyoutItem { Tag: string tag } && ViewModel.SortCommand.CanExecute(tag))
        {
            await ViewModel.SortCommand.ExecuteAsync(tag);
        }
    }

    private void UpdateSortHeaderVisualState()
    {
        SortColumn currentColumn = ViewModel.CurrentSortColumn;
        SortDirection currentDirection = ViewModel.CurrentSortDirection;
        ApplySortHeaderVisualState(NameSortButton, SortColumn.Name, "Name", currentColumn, currentDirection);
        ApplySortHeaderVisualState(AttentionSortButton, SortColumn.Attention, "Attention", currentColumn, currentDirection);
        ApplySortHeaderVisualState(CpuSortButton, SortColumn.CpuPct, "CPU", currentColumn, currentDirection);
        ApplySortHeaderVisualState(MemorySortButton, SortColumn.MemoryBytes, "Memory", currentColumn, currentDirection);
        ApplySortHeaderVisualState(DiskSortButton, SortColumn.DiskBps, "Disk", currentColumn, currentDirection);
        ApplySortHeaderVisualState(OtherIoSortButton, SortColumn.OtherIoBps, "Other I/O", currentColumn, currentDirection);
        ApplySortHeaderVisualState(PidSortButton, SortColumn.Pid, "PID", currentColumn, currentDirection);
    }

    private static void ApplySortHeaderVisualState(
        Button button,
        SortColumn headerColumn,
        string label,
        SortColumn currentColumn,
        SortDirection currentDirection)
    {
        bool isActive = headerColumn == currentColumn;
        string directionMarker = currentDirection == SortDirection.Asc ? "↑" : "↓";
        button.Content = isActive ? $"{label} {directionMarker}" : label;
        button.FontWeight = isActive ? FontWeights.SemiBold : FontWeights.Normal;
        button.Opacity = isActive ? 1 : 0.82;
        button.BorderThickness = isActive ? new Thickness(2) : new Thickness(1);
        button.Background = GetBrush(isActive ? "BatCavePrimaryBrush" : "BatCaveSurfaceBrush");
        button.BorderBrush = GetBrush(isActive ? "BatCavePrimaryBrush" : "BatCaveBorderBrush");
        button.Foreground = GetBrush(isActive ? "BatCaveSurfaceBrush" : "BatCaveTextPrimaryBrush");
    }

    private static Brush? GetBrush(string resourceKey)
    {
        return Application.Current.Resources.TryGetValue(resourceKey, out object value)
            ? value as Brush
            : null;
    }

    private void FocusFilter_Invoked(KeyboardAccelerator sender, KeyboardAcceleratorInvokedEventArgs args)
    {
        args.Handled = true;
        FilterBox.Focus(FocusState.Programmatic);
        FilterBox.SelectAll();
    }

    private async void Refresh_Invoked(KeyboardAccelerator sender, KeyboardAcceleratorInvokedEventArgs args)
    {
        args.Handled = true;
        if (ViewModel.RefreshCommand.CanExecute(null))
        {
            await ViewModel.RefreshCommand.ExecuteAsync(null);
        }
    }

    private async void TogglePause_Invoked(KeyboardAccelerator sender, KeyboardAcceleratorInvokedEventArgs args)
    {
        args.Handled = true;
        if (ViewModel.IsPaused)
        {
            await ViewModel.ResumeCommand.ExecuteAsync(null);
        }
        else
        {
            await ViewModel.PauseCommand.ExecuteAsync(null);
        }
    }

    private void ClearSelection_Invoked(KeyboardAccelerator sender, KeyboardAcceleratorInvokedEventArgs args)
    {
        args.Handled = true;
        if (ViewModel.ClearSelectionCommand.CanExecute(null))
        {
            ViewModel.ClearSelectionCommand.Execute(null);
        }
    }

    private void CopyDetails_Click(object sender, RoutedEventArgs e)
    {
        CopyTextToClipboard(ViewModel.CopyDetailsText);
    }

    private void WorkflowNav_SelectionChanged(NavigationView sender, NavigationViewSelectionChangedEventArgs args)
    {
        if (args.SelectedItem is NavigationViewItem { Tag: string tag })
        {
            ViewModel.SelectWorkflow(tag);
            ApplyResponsiveLayoutIfChanged(GetResponsiveWidth(Root.ActualWidth));
        }
    }

    private void ProcessRow_RightTapped(object sender, RightTappedRoutedEventArgs e)
    {
        if (sender is FrameworkElement { DataContext: ProcessRowViewModel row })
        {
            ViewModel.SelectedRow = row;
        }
    }

    private void ProcessRowMenuItem_Click(object sender, RoutedEventArgs e)
    {
        if (sender is not MenuFlyoutItem { Tag: string action })
        {
            return;
        }

        switch (action)
        {
            case "CopyDetails":
                CopyTextToClipboard(ViewModel.CopyDetailsText);
                break;
            case "CopyPid":
                CopyTextToClipboard(ViewModel.SelectedRow?.PidText ?? string.Empty);
                break;
            case "FilterToProcess":
                ViewModel.FilterToSelectedProcess();
                break;
            case "ClearSelection":
                if (ViewModel.ClearSelectionCommand.CanExecute(null))
                {
                    ViewModel.ClearSelectionCommand.Execute(null);
                }

                break;
        }
    }

    private static void CopyTextToClipboard(string text)
    {
        if (string.IsNullOrWhiteSpace(text))
        {
            return;
        }

        DataPackage package = new();
        package.SetText(text);
        Clipboard.SetContent(package);
    }

    private void ApplyResponsiveLayoutIfChanged(double width)
    {
        if (Math.Abs(width - _lastResponsiveLayoutWidth) < 0.5)
        {
            return;
        }

        ApplyResponsiveLayout(width);
    }

    private void ApplyResponsiveLayout(double width)
    {
        _lastResponsiveLayoutWidth = width;
        double layoutWidth = Root.ActualWidth > 0 ? Root.ActualWidth : width;
        bool narrow = width < WideLayoutMinWidth;
        bool compactProcesses = width < CompactProcessListMaxWidth;
        WorkspaceGrid.ColumnSpacing = narrow ? 0 : 14;
        WorkspaceGrid.RowSpacing = narrow ? 14 : 0;
        ProcessColumn.Width = new GridLength(1, GridUnitType.Star);
        ProcessColumn.MaxWidth = double.PositiveInfinity;
        ProcessPane.Width = double.NaN;
        ProcessPane.MaxWidth = double.PositiveInfinity;
        ProcessPane.MaxHeight = narrow ? NarrowProcessPaneMaxHeight : double.PositiveInfinity;
        double desktopProcessTableWidth = narrow
            ? Math.Max(320, layoutWidth - WorkspaceHorizontalPadding)
            : double.NaN;
        DesktopProcessTable.Width = compactProcesses ? double.NaN : desktopProcessTableWidth;
        DesktopProcessTable.Visibility = compactProcesses ? Visibility.Collapsed : Visibility.Visible;
        CompactProcessSurface.Visibility = compactProcesses ? Visibility.Visible : Visibility.Collapsed;
        CompactProcessList.Visibility = compactProcesses ? Visibility.Visible : Visibility.Collapsed;
        InspectorPane.Width = narrow ? double.NaN : InspectorPaneWidth;
        InspectorColumn.Width = narrow ? new GridLength(0) : new GridLength(InspectorPaneWidth);
        InspectorMetricsSecondColumn.Width = narrow ? new GridLength(0) : new GridLength(1, GridUnitType.Star);
        Grid.SetColumn(MemoryMetricCard, narrow ? 0 : 1);
        Grid.SetRow(MemoryMetricCard, narrow ? 1 : 0);
        Grid.SetColumn(DiskMetricCard, 0);
        Grid.SetRow(DiskMetricCard, narrow ? 2 : 1);
        Grid.SetColumn(OtherIoMetricCard, narrow ? 0 : 1);
        Grid.SetRow(OtherIoMetricCard, narrow ? 3 : 1);
        Grid.SetColumnSpan(ThreadsMetricCard, narrow ? 1 : 2);
        Grid.SetRow(ThreadsMetricCard, narrow ? 4 : 2);
        ProcessWorkspaceRow.Height = narrow ? GridLength.Auto : new GridLength(1, GridUnitType.Star);
        InspectorWorkspaceRow.Height = narrow ? new GridLength(1, GridUnitType.Star) : GridLength.Auto;
        Grid.SetRow(InspectorPane, narrow ? 1 : 0);
        Grid.SetColumn(InspectorPane, narrow ? 0 : 1);
        ScrollSelectedRowIntoView();
    }

    private void ProcessList_SelectionChanged(object sender, SelectionChangedEventArgs e)
    {
        ScrollSelectedRowIntoView();
    }

    private void ScrollSelectedRowIntoView()
    {
        ProcessRowViewModel? selectedRow = ViewModel.SelectedRow;
        if (selectedRow is null)
        {
            return;
        }

        ProcessList.ScrollIntoView(selectedRow, ScrollIntoViewAlignment.Leading);
        CompactProcessList.ScrollIntoView(selectedRow, ScrollIntoViewAlignment.Leading);
    }
}
