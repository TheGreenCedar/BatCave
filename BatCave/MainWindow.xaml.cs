using BatCave.Layouts;
using BatCave.ViewModels;
using Microsoft.Extensions.DependencyInjection;
using Microsoft.UI;
using Microsoft.UI.Windowing;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Input;
using Microsoft.UI.Xaml.Media;
using Microsoft.UI.Xaml.Shapes;
using System;
using System.Collections.Generic;
using System.Collections.Specialized;
using System.ComponentModel;
using System.Diagnostics;
using System.IO;
using System.Threading;
using WinRT.Interop;
using Windows.Foundation;

namespace BatCave;

public sealed partial class MainWindow : Window
{

    private bool _bootstrapped;
    private bool _compactProcessInitialScrollPending = true;
    private long _selectionSettleProbeStartedAt;
    private bool _logicalCpuGridLayoutQueued;
    private int _logicalCpuGridLastCount = -1;
    private double _logicalCpuGridLastWidth = -1;
    private const double HeaderDecorationStride = 388;
    private static readonly IReadOnlyList<HeaderDecorationSpec> HeaderDecorationPattern =
    [
        new(false, -20, -18, 72, "BatCavePrimaryBrush", 0),
        new(true, 84, -6, 20, "BatCaveAccentBrush", 0),
        new(false, 128, 12, 28, "BatCaveSecondaryBrush", -16),
        new(true, 182, 22, 14, "BatCavePrimaryBrush", 0),
        new(false, 216, -10, 58, "BatCaveAccentBrush", 14),
        new(false, 272, 12, 24, "BatCavePrimaryBrush", 22),
        new(true, 318, -4, 46, "BatCaveSecondaryBrush", 0),
    ];

    public MainWindow()
    {
        ViewModel = App.Services.GetRequiredService<MonitoringShellViewModel>();
        InitializeComponent();
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

        IntPtr windowHandle = WindowNative.GetWindowHandle(this);
        WindowId windowId = Win32Interop.GetWindowIdFromWindow(windowHandle);
        AppWindow appWindow = AppWindow.GetFromWindowId(windowId);
        appWindow.SetIcon(iconPath);
    }

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

    private void HeaderRegion_SizeChanged(object sender, SizeChangedEventArgs e)
    {
        RebuildHeaderDecorations(e.NewSize.Width, e.NewSize.Height);
    }

    private void RebuildHeaderDecorations(double width, double height)
    {
        if (width <= 0 || height <= 0)
        {
            return;
        }

        HeaderDecorationCanvas.Children.Clear();
        HeaderDecorationCanvas.Clip = new RectangleGeometry { Rect = new Rect(0, 0, width, height) };

        for (double offset = 0; offset < width + HeaderDecorationStride; offset += HeaderDecorationStride)
        {
            foreach (HeaderDecorationSpec spec in HeaderDecorationPattern)
            {
                Shape shape = spec.IsEllipse ? new Ellipse() : new Rectangle();
                shape.Width = spec.Size;
                shape.Height = spec.Size;
                shape.Fill = (Brush)Application.Current.Resources[spec.BrushResourceKey];

                if (Math.Abs(spec.Angle) > double.Epsilon)
                {
                    shape.RenderTransform = new RotateTransform
                    {
                        Angle = spec.Angle,
                        CenterX = spec.Size / 2,
                        CenterY = spec.Size / 2,
                    };
                }

                Canvas.SetLeft(shape, spec.Left + offset);
                Canvas.SetTop(shape, spec.Top);
                HeaderDecorationCanvas.Children.Add(shape);
            }
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

    private readonly record struct HeaderDecorationSpec(bool IsEllipse, double Left, double Top, double Size, string BrushResourceKey, double Angle);

    private void OnWindowClosed(object sender, WindowEventArgs args)
    {
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
