using BatCave.Layouts;
using BatCave.ViewModels;
using Microsoft.Extensions.DependencyInjection;
using Microsoft.UI;
using Microsoft.UI.Windowing;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Input;
using Microsoft.UI.Xaml.Markup;
using Microsoft.UI.Xaml.Media;
using Microsoft.UI.Xaml.Media.Animation;
using System;
using System.Collections.Generic;
using System.Collections.Specialized;
using System.ComponentModel;
using System.Diagnostics;
using System.IO;
using System.Threading;
using WinRT.Interop;
using Windows.Foundation;
using XamlPath = Microsoft.UI.Xaml.Shapes.Path;

namespace BatCave;

public sealed partial class MainWindow : Window
{
    private bool _bootstrapped;
    private bool _compactProcessInitialScrollPending = true;
    private long _selectionSettleProbeStartedAt;
    private bool _logicalCpuGridLayoutQueued;
    private int _logicalCpuGridLastCount = -1;
    private double _logicalCpuGridLastWidth = -1;
    private readonly List<Storyboard> _headerDecorationStoryboards = [];
    private const double HeaderBatBaseWidth = 100d;
    private const double HeaderBatBaseHeight = 48d;
    private const string HeaderBatGlidePathData = "M4,24 C10,16 16,12 24,12 C30,12 38,15 48,21 L50,24 L52,21 C62,15 70,12 76,12 C84,12 90,16 96,24 L86,23 L78,32 L67,27 L58,39 L50,31 L42,39 L33,27 L22,32 L14,23 Z";
    private const string HeaderBatSweepPathData = "M6,26 C16,14 24,8 34,8 C42,8 48,14 50,20 L52,20 C54,14 60,8 68,8 C78,8 86,14 96,26 L84,24 L76,34 L64,28 L56,40 L50,34 L44,40 L36,28 L24,34 L16,24 Z";
    private const string HeaderBatDartPathData = "M10,24 C18,18 26,14 34,14 C40,14 45,17 49,21 L50,24 L51,21 C55,17 60,14 66,14 C74,14 82,18 90,24 L80,23 L72,31 L62,27 L56,37 L50,30 L44,37 L38,27 L28,31 L20,23 Z";
    private static readonly IReadOnlyList<HeaderBatSpec> HeaderBatPattern =
    [
        new(HeaderBatVariant.Glide, HeaderBatSide.Left, 0.02, 8, 0.98, 0.25, HeaderBatMotion.TowardCave, "BatCavePrimaryBrush", 0.00, -3d),
        new(HeaderBatVariant.Dart, HeaderBatSide.Left, 0.08, 22, 0.58, 0.16, HeaderBatMotion.TowardCave, "BatCaveAccentBrush", 0.06, -1d),
        new(HeaderBatVariant.Sweep, HeaderBatSide.Left, 0.12, -2, 0.86, 0.21, HeaderBatMotion.TowardCave, "BatCaveSecondaryBrush", 0.10, 2d),
        new(HeaderBatVariant.Glide, HeaderBatSide.Left, 0.18, 12, 0.66, 0.18, HeaderBatMotion.AwayFromCave, "BatCavePrimaryBrush", 0.14, 1d),
        new(HeaderBatVariant.Dart, HeaderBatSide.Left, 0.22, 18, 0.74, 0.19, HeaderBatMotion.AwayFromCave, "BatCaveAccentBrush", 0.18, -2d),
        new(HeaderBatVariant.Glide, HeaderBatSide.Left, 0.34, 28, 0.62, 0.17, HeaderBatMotion.AwayFromCave, "BatCavePrimaryBrush", 0.28, 1d),
        new(HeaderBatVariant.Sweep, HeaderBatSide.Left, 0.42, 2, 0.54, 0.16, HeaderBatMotion.TowardCave, "BatCaveSecondaryBrush", 0.32, -2d),
        new(HeaderBatVariant.Sweep, HeaderBatSide.Left, 0.50, 10, 0.70, 0.18, HeaderBatMotion.TowardCave, "BatCaveSecondaryBrush", 0.36, -1d),
        new(HeaderBatVariant.Dart, HeaderBatSide.Left, 0.58, 30, 0.46, 0.14, HeaderBatMotion.AwayFromCave, "BatCaveAccentBrush", 0.40, 1d),
        new(HeaderBatVariant.Dart, HeaderBatSide.Left, 0.68, -6, 0.56, 0.15, HeaderBatMotion.TowardCave, "BatCaveAccentBrush", 0.44, 2d),
        new(HeaderBatVariant.Glide, HeaderBatSide.Left, 0.82, 24, 0.50, 0.14, HeaderBatMotion.AwayFromCave, "BatCavePrimaryBrush", 0.54, -1d),
        new(HeaderBatVariant.Sweep, HeaderBatSide.Left, 0.88, 14, 0.40, 0.12, HeaderBatMotion.TowardCave, "BatCaveSecondaryBrush", 0.58, 1d),
        new(HeaderBatVariant.Sweep, HeaderBatSide.Left, 0.92, 4, 0.44, 0.13, HeaderBatMotion.TowardCave, "BatCaveSecondaryBrush", 0.62, 1d),
        new(HeaderBatVariant.Glide, HeaderBatSide.Right, 0.02, 6, 0.96, 0.24, HeaderBatMotion.AwayFromCave, "BatCavePrimaryBrush", 0.06, 3d),
        new(HeaderBatVariant.Dart, HeaderBatSide.Right, 0.08, 20, 0.58, 0.16, HeaderBatMotion.AwayFromCave, "BatCaveAccentBrush", 0.12, 1d),
        new(HeaderBatVariant.Sweep, HeaderBatSide.Right, 0.14, -4, 0.84, 0.21, HeaderBatMotion.TowardCave, "BatCaveSecondaryBrush", 0.16, -2d),
        new(HeaderBatVariant.Glide, HeaderBatSide.Right, 0.20, 10, 0.66, 0.18, HeaderBatMotion.TowardCave, "BatCavePrimaryBrush", 0.20, -1d),
        new(HeaderBatVariant.Dart, HeaderBatSide.Right, 0.24, 16, 0.72, 0.19, HeaderBatMotion.AwayFromCave, "BatCaveAccentBrush", 0.24, 2d),
        new(HeaderBatVariant.Glide, HeaderBatSide.Right, 0.36, 26, 0.60, 0.17, HeaderBatMotion.TowardCave, "BatCavePrimaryBrush", 0.34, -1d),
        new(HeaderBatVariant.Sweep, HeaderBatSide.Right, 0.44, 0, 0.54, 0.16, HeaderBatMotion.AwayFromCave, "BatCaveSecondaryBrush", 0.38, 2d),
        new(HeaderBatVariant.Sweep, HeaderBatSide.Right, 0.52, 12, 0.68, 0.18, HeaderBatMotion.AwayFromCave, "BatCaveSecondaryBrush", 0.42, 1d),
        new(HeaderBatVariant.Dart, HeaderBatSide.Right, 0.60, 28, 0.46, 0.14, HeaderBatMotion.TowardCave, "BatCaveAccentBrush", 0.48, -1d),
        new(HeaderBatVariant.Dart, HeaderBatSide.Right, 0.70, -8, 0.54, 0.15, HeaderBatMotion.TowardCave, "BatCaveAccentBrush", 0.52, -2d),
        new(HeaderBatVariant.Glide, HeaderBatSide.Right, 0.84, 22, 0.48, 0.14, HeaderBatMotion.TowardCave, "BatCavePrimaryBrush", 0.60, 1d),
        new(HeaderBatVariant.Sweep, HeaderBatSide.Right, 0.90, 12, 0.40, 0.12, HeaderBatMotion.AwayFromCave, "BatCaveSecondaryBrush", 0.66, -1d),
        new(HeaderBatVariant.Sweep, HeaderBatSide.Right, 0.94, 2, 0.42, 0.13, HeaderBatMotion.AwayFromCave, "BatCaveSecondaryBrush", 0.70, -1d),
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
        ResetHeaderDecorationAnimations();

        if (width <= 0 || height <= 0)
        {
            HeaderDecorationCanvas.Children.Clear();
            return;
        }

        HeaderDecorationCanvas.Children.Clear();
        HeaderDecorationCanvas.Clip = new RectangleGeometry { Rect = new Rect(0, 0, width, height) };
        if (width < 220d)
        {
            return;
        }

        double caveWidth = HeaderControlsInline.Visibility == Visibility.Visible && HeaderControlsInline.ActualWidth > 0
            ? HeaderControlsInline.ActualWidth
            : Math.Min(280d, width * 0.42d);
        double caveHalfWidth = caveWidth / 2d;
        double caveCenter = width / 2d;
        double leftStart = 12d;
        double rightStart = Math.Min(width - 12d, caveCenter + caveHalfWidth + 12d);
        double leftSpan = Math.Max(42d, caveCenter - caveHalfWidth - leftStart - 12d);
        double rightSpan = Math.Max(42d, width - rightStart - 12d);
        int batCount = width < 640d ? 8 : width < 920d ? 14 : HeaderBatPattern.Count;

        for (int index = 0; index < batCount; index++)
        {
            HeaderBatSpec spec = HeaderBatPattern[index];
            double batWidth = HeaderBatBaseWidth * spec.Scale;
            double batHeight = HeaderBatBaseHeight * spec.Scale;
            double availableSpan = spec.Side == HeaderBatSide.Left ? leftSpan : rightSpan;
            double startX = spec.Side == HeaderBatSide.Left
                ? leftStart + (availableSpan - batWidth) * spec.Lane
                : rightStart + (availableSpan - batWidth) * spec.Lane;
            double top = Math.Clamp(spec.TopOffset, -6d, Math.Max(-6d, height - batHeight - 4d));

            XamlPath bat = CreateHeaderBatPath(spec, batWidth, batHeight);
            Canvas.SetLeft(bat, startX);
            Canvas.SetTop(bat, top);
            HeaderDecorationCanvas.Children.Add(bat);
            StartHeaderBatAnimation(bat, spec);
        }
    }

    private static XamlPath CreateHeaderBatPath(HeaderBatSpec spec, double width, double height)
    {
        XamlPath bat = (XamlPath)XamlReader.Load(
            $"<Path xmlns=\"http://schemas.microsoft.com/winfx/2006/xaml/presentation\" Data=\"{ResolveHeaderBatPathData(spec.Variant)}\" Stretch=\"Fill\" />");
        bat.Width = width;
        bat.Height = height;
        bat.Fill = (Brush)Application.Current.Resources[spec.BrushResourceKey];
        bat.Opacity = spec.Opacity;
        bat.IsHitTestVisible = false;
        return bat;
    }

    private void StartHeaderBatAnimation(XamlPath bat, HeaderBatSpec spec)
    {
        double direction = spec.Side == HeaderBatSide.Left ? 1d : -1d;
        if (spec.MotionDirection == HeaderBatMotion.AwayFromCave)
        {
            direction *= -1d;
        }

        double drift = direction * (8d + (spec.Scale * 10d));
        TimeSpan duration = TimeSpan.FromSeconds(4.2d + (1d - spec.Scale) * 1.6d);
        TranslateTransform translate = new();
        bat.RenderTransform = translate;

        DoubleAnimation slideAnimation = new()
        {
            From = 0d,
            To = drift,
            Duration = duration,
            AutoReverse = true,
            RepeatBehavior = RepeatBehavior.Forever,
            BeginTime = TimeSpan.FromSeconds(spec.PhaseSeconds),
            EnableDependentAnimation = true,
            EasingFunction = new SineEase { EasingMode = EasingMode.EaseInOut },
        };

        DoubleAnimation opacityAnimation = new()
        {
            From = Math.Max(0.05d, spec.Opacity - 0.06d),
            To = Math.Min(0.4d, spec.Opacity + 0.05d),
            Duration = duration,
            AutoReverse = true,
            RepeatBehavior = RepeatBehavior.Forever,
            BeginTime = TimeSpan.FromSeconds(spec.PhaseSeconds),
        };

        Storyboard storyboard = new();
        Storyboard.SetTarget(slideAnimation, translate);
        Storyboard.SetTargetProperty(slideAnimation, "X");

        DoubleAnimation liftAnimation = new()
        {
            From = 0d,
            To = spec.VerticalDrift,
            Duration = TimeSpan.FromSeconds(duration.TotalSeconds + 0.8d),
            AutoReverse = true,
            RepeatBehavior = RepeatBehavior.Forever,
            BeginTime = TimeSpan.FromSeconds(spec.PhaseSeconds * 0.75d),
            EnableDependentAnimation = true,
            EasingFunction = new SineEase { EasingMode = EasingMode.EaseInOut },
        };

        Storyboard.SetTarget(liftAnimation, translate);
        Storyboard.SetTargetProperty(liftAnimation, "Y");

        Storyboard.SetTarget(opacityAnimation, bat);
        Storyboard.SetTargetProperty(opacityAnimation, "Opacity");
        storyboard.Children.Add(slideAnimation);
        storyboard.Children.Add(liftAnimation);
        storyboard.Children.Add(opacityAnimation);
        storyboard.Begin();
        _headerDecorationStoryboards.Add(storyboard);
    }

    private static string ResolveHeaderBatPathData(HeaderBatVariant variant)
    {
        return variant switch
        {
            HeaderBatVariant.Glide => HeaderBatGlidePathData,
            HeaderBatVariant.Sweep => HeaderBatSweepPathData,
            _ => HeaderBatDartPathData,
        };
    }

    private void ResetHeaderDecorationAnimations()
    {
        foreach (Storyboard storyboard in _headerDecorationStoryboards)
        {
            storyboard.Stop();
        }

        _headerDecorationStoryboards.Clear();
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

    private readonly record struct HeaderBatSpec(
        HeaderBatVariant Variant,
        HeaderBatSide Side,
        double Lane,
        double TopOffset,
        double Scale,
        double Opacity,
        HeaderBatMotion MotionDirection,
        string BrushResourceKey,
        double PhaseSeconds,
        double VerticalDrift);

    private enum HeaderBatVariant
    {
        Glide,
        Sweep,
        Dart,
    }

    private enum HeaderBatSide
    {
        Left,
        Right,
    }

    private enum HeaderBatMotion
    {
        TowardCave,
        AwayFromCave,
    }

    private void OnWindowClosed(object sender, WindowEventArgs args)
    {
        ResetHeaderDecorationAnimations();
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
