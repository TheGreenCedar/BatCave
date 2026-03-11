using BatCave.Charts;
using BatCave.Converters;
using LiveChartsCore;
using LiveChartsCore.Defaults;
using LiveChartsCore.Measure;
using LiveChartsCore.SkiaSharpView;
using LiveChartsCore.SkiaSharpView.Painting;
using LiveChartsCore.SkiaSharpView.Painting.Effects;
using Microsoft.UI;
using Microsoft.UI.Dispatching;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Media;
using Microsoft.UI.Xaml.Media.Animation;
using SkiaSharp;
using System;
using System.Collections.ObjectModel;
using System.Collections.Generic;
using System.ComponentModel;
using System.Diagnostics;
using System.Threading;
using Windows.UI;
using LiveChartsCore.SkiaSharpView.WinUI;

namespace BatCave.Controls;

public sealed partial class MetricTrendChart : UserControl
{
    public static readonly DependencyProperty ValuesProperty = DependencyProperty.Register(
        nameof(Values),
        typeof(IReadOnlyList<double>),
        typeof(MetricTrendChart),
        new PropertyMetadata(Array.Empty<double>(), OnChartPropertyChanged));

    public static readonly DependencyProperty OverlayValuesProperty = DependencyProperty.Register(
        nameof(OverlayValues),
        typeof(IReadOnlyList<double>),
        typeof(MetricTrendChart),
        new PropertyMetadata(Array.Empty<double>(), OnChartPropertyChanged));

    public static readonly DependencyProperty VisiblePointCountProperty = DependencyProperty.Register(
        nameof(VisiblePointCount),
        typeof(int),
        typeof(MetricTrendChart),
        new PropertyMetadata(MetricTrendChartRenderPlanner.MinVisiblePointCount, OnChartPropertyChanged));

    public static readonly DependencyProperty ChartIdentityKeyProperty = DependencyProperty.Register(
        nameof(ChartIdentityKey),
        typeof(string),
        typeof(MetricTrendChart),
        new PropertyMetadata(string.Empty, OnChartPropertyChanged));

    public static readonly DependencyProperty ScaleModeProperty = DependencyProperty.Register(
        nameof(ScaleMode),
        typeof(MetricTrendScaleMode),
        typeof(MetricTrendChart),
        new PropertyMetadata(MetricTrendScaleMode.CpuPercent, OnChartPropertyChanged));

    public static readonly DependencyProperty DomainMaxOverrideProperty = DependencyProperty.Register(
        nameof(DomainMaxOverride),
        typeof(double),
        typeof(MetricTrendChart),
        new PropertyMetadata(double.NaN, OnChartPropertyChanged));

    public static readonly DependencyProperty ShowGridProperty = DependencyProperty.Register(
        nameof(ShowGrid),
        typeof(bool),
        typeof(MetricTrendChart),
        new PropertyMetadata(false, OnChartPropertyChanged));

    public static readonly DependencyProperty ShowAreaFillProperty = DependencyProperty.Register(
        nameof(ShowAreaFill),
        typeof(bool),
        typeof(MetricTrendChart),
        new PropertyMetadata(true, OnChartPropertyChanged));

    public static readonly DependencyProperty StrokeBrushProperty = DependencyProperty.Register(
        nameof(StrokeBrush),
        typeof(Brush),
        typeof(MetricTrendChart),
        new PropertyMetadata(null, OnChartPropertyChanged));

    public static readonly DependencyProperty FillBrushProperty = DependencyProperty.Register(
        nameof(FillBrush),
        typeof(Brush),
        typeof(MetricTrendChart),
        new PropertyMetadata(null, OnChartPropertyChanged));

    public static readonly DependencyProperty GridBrushProperty = DependencyProperty.Register(
        nameof(GridBrush),
        typeof(Brush),
        typeof(MetricTrendChart),
        new PropertyMetadata(null, OnChartPropertyChanged));

    public static readonly DependencyProperty OverlayStrokeBrushProperty = DependencyProperty.Register(
        nameof(OverlayStrokeBrush),
        typeof(Brush),
        typeof(MetricTrendChart),
        new PropertyMetadata(null, OnChartPropertyChanged));

    public static readonly DependencyProperty StrokeThicknessProperty = DependencyProperty.Register(
        nameof(StrokeThickness),
        typeof(double),
        typeof(MetricTrendChart),
        new PropertyMetadata(1.15d, OnChartPropertyChanged));

    public static readonly DependencyProperty OverlayStrokeThicknessProperty = DependencyProperty.Register(
        nameof(OverlayStrokeThickness),
        typeof(double),
        typeof(MetricTrendChart),
        new PropertyMetadata(0.85d, OnChartPropertyChanged));

    public static readonly DependencyProperty ShowOverlayProperty = DependencyProperty.Register(
        nameof(ShowOverlay),
        typeof(bool),
        typeof(MetricTrendChart),
        new PropertyMetadata(false, OnChartPropertyChanged));

    public static readonly DependencyProperty EnableSmoothPointTransitionsProperty = DependencyProperty.Register(
        nameof(EnableSmoothPointTransitions),
        typeof(bool),
        typeof(MetricTrendChart),
        new PropertyMetadata(true, OnChartPropertyChanged));

    public static readonly DependencyProperty PointTransitionDurationMsProperty = DependencyProperty.Register(
        nameof(PointTransitionDurationMs),
        typeof(int),
        typeof(MetricTrendChart),
        new PropertyMetadata(MetricTrendTransitionMath.DefaultDurationMs, OnChartPropertyChanged));

    private static readonly DashEffect OverlayDashEffect = new([2f, 2f], 0f);
    private readonly Color _defaultStrokeColor = Colors.DodgerBlue;
    private readonly Color _defaultFillColor = Color.FromArgb(38, 30, 144, 255);
    private readonly Color _defaultOverlayStrokeColor = Color.FromArgb(204, 76, 93, 112);
    private readonly Color _defaultGridColor = Color.FromArgb(36, 140, 148, 163);
    private readonly Axis _xAxis = new();
    private readonly Axis _yAxis = new();
    private readonly Axis[] _xAxes;
    private readonly Axis[] _yAxes;
    private ObservableCollection<ObservablePoint> _primaryPoints = [];
    private ObservableCollection<ObservablePoint> _overlayPoints = [];
    private LineSeries<ObservablePoint> _primarySeries = null!;
    private LineSeries<ObservablePoint> _overlaySeries = null!;
    private ISeries[] _singleSeries = null!;
    private ISeries[] _dualSeries = null!;
    private readonly Axis _transitionXAxis = new();
    private readonly Axis _transitionYAxis = new();
    private readonly Axis[] _transitionXAxes;
    private readonly Axis[] _transitionYAxes;
    private ObservableCollection<ObservablePoint> _transitionPrimaryPoints = [];
    private ObservableCollection<ObservablePoint> _transitionOverlayPoints = [];
    private LineSeries<ObservablePoint> _transitionPrimarySeries = null!;
    private LineSeries<ObservablePoint> _transitionOverlaySeries = null!;
    private ISeries[] _transitionSingleSeries = null!;
    private ISeries[] _transitionDualSeries = null!;

    private double _dynamicDomainMaxRaw;
    private int _renderQueued;
    private bool _hasTransitionSnapshot;
    private bool _pendingViewportSwitch;
    private bool _requiresTransitionReset;
    private bool _requiresSeriesRebuild;
    private INotifyPropertyChanged? _dataContextNotifier;
    private MetricTrendTransitionSnapshot _transitionSnapshot;
    private Storyboard? _viewportTransitionStoryboard;
    private DispatcherQueueTimer? _viewportTransitionCleanupTimer;

    public MetricTrendChart()
    {
        InitializeComponent();

        _xAxes = [_xAxis];
        _yAxes = [_yAxis];
        _transitionXAxes = [_transitionXAxis];
        _transitionYAxes = [_transitionYAxis];
        RebuildChartSeries();

        InitializeChartSurface(TrendChart, _xAxes, _yAxes, _singleSeries);
        InitializeChartSurface(TransitionChart, _transitionXAxes, _transitionYAxes, _transitionSingleSeries);

        Loaded += OnLoaded;
        Unloaded += OnUnloaded;
        DataContextChanged += OnDataContextChanged;
        PlotBorder.SizeChanged += PlotBorder_SizeChanged;
        ScheduleRender();
    }

    public IReadOnlyList<double> Values
    {
        get => (IReadOnlyList<double>)GetValue(ValuesProperty);
        set => SetValue(ValuesProperty, value ?? Array.Empty<double>());
    }

    public IReadOnlyList<double> OverlayValues
    {
        get => (IReadOnlyList<double>)GetValue(OverlayValuesProperty);
        set => SetValue(OverlayValuesProperty, value ?? Array.Empty<double>());
    }

    public int VisiblePointCount
    {
        get => (int)GetValue(VisiblePointCountProperty);
        set => SetValue(VisiblePointCountProperty, value);
    }

    public string ChartIdentityKey
    {
        get => (string)GetValue(ChartIdentityKeyProperty);
        set => SetValue(ChartIdentityKeyProperty, value ?? string.Empty);
    }

    public MetricTrendScaleMode ScaleMode
    {
        get => (MetricTrendScaleMode)GetValue(ScaleModeProperty);
        set => SetValue(ScaleModeProperty, value);
    }

    public double DomainMaxOverride
    {
        get => (double)GetValue(DomainMaxOverrideProperty);
        set => SetValue(DomainMaxOverrideProperty, value);
    }

    public bool ShowGrid
    {
        get => (bool)GetValue(ShowGridProperty);
        set => SetValue(ShowGridProperty, value);
    }

    public bool ShowAreaFill
    {
        get => (bool)GetValue(ShowAreaFillProperty);
        set => SetValue(ShowAreaFillProperty, value);
    }

    public Brush StrokeBrush
    {
        get => (Brush?)GetValue(StrokeBrushProperty) ?? new SolidColorBrush(_defaultStrokeColor);
        set => SetValue(StrokeBrushProperty, value);
    }

    public Brush FillBrush
    {
        get => (Brush?)GetValue(FillBrushProperty) ?? new SolidColorBrush(_defaultFillColor);
        set => SetValue(FillBrushProperty, value);
    }

    public Brush GridBrush
    {
        get => (Brush?)GetValue(GridBrushProperty) ?? new SolidColorBrush(_defaultGridColor);
        set => SetValue(GridBrushProperty, value);
    }

    public Brush OverlayStrokeBrush
    {
        get => (Brush?)GetValue(OverlayStrokeBrushProperty) ?? new SolidColorBrush(_defaultOverlayStrokeColor);
        set => SetValue(OverlayStrokeBrushProperty, value);
    }

    public double StrokeThickness
    {
        get => (double)GetValue(StrokeThicknessProperty);
        set => SetValue(StrokeThicknessProperty, value);
    }

    public double OverlayStrokeThickness
    {
        get => (double)GetValue(OverlayStrokeThicknessProperty);
        set => SetValue(OverlayStrokeThicknessProperty, value);
    }

    public bool ShowOverlay
    {
        get => (bool)GetValue(ShowOverlayProperty);
        set => SetValue(ShowOverlayProperty, value);
    }

    public bool EnableSmoothPointTransitions
    {
        get => (bool)GetValue(EnableSmoothPointTransitionsProperty);
        set => SetValue(EnableSmoothPointTransitionsProperty, value);
    }

    public int PointTransitionDurationMs
    {
        get => MetricTrendTransitionMath.NormalizeDurationMs((int)GetValue(PointTransitionDurationMsProperty));
        set => SetValue(PointTransitionDurationMsProperty, MetricTrendTransitionMath.NormalizeDurationMs(value));
    }

    public void RequestRender()
    {
        ScheduleRender();
    }

    private void OnLoaded(object sender, RoutedEventArgs e)
    {
        ResetTransitionState();
        AttachDataContextNotifier(DataContext as INotifyPropertyChanged);
        ScheduleRender();
    }

    private void OnUnloaded(object sender, RoutedEventArgs e)
    {
        ResetTransitionState();
        AttachDataContextNotifier(null);
    }

    private void OnDataContextChanged(FrameworkElement sender, DataContextChangedEventArgs args)
    {
        AttachDataContextNotifier(args.NewValue as INotifyPropertyChanged);
    }

    private void PlotBorder_SizeChanged(object sender, SizeChangedEventArgs e)
    {
        if (e.NewSize.Width <= 1d || e.NewSize.Height <= 1d)
        {
            ResetTransitionState();
            return;
        }

        ScheduleRender();
    }

    private void AttachDataContextNotifier(INotifyPropertyChanged? notifier)
    {
        if (ReferenceEquals(_dataContextNotifier, notifier))
        {
            return;
        }

        if (_dataContextNotifier is not null)
        {
            _dataContextNotifier.PropertyChanged -= OnDataContextPropertyChanged;
        }

        _dataContextNotifier = notifier;
        if (_dataContextNotifier is not null)
        {
            _dataContextNotifier.PropertyChanged += OnDataContextPropertyChanged;
        }
    }

    private void OnDataContextPropertyChanged(object? sender, PropertyChangedEventArgs args)
    {
        if (!IsTrendDataPropertyChange(args.PropertyName))
        {
            return;
        }

        ScheduleRender();
    }

    private static bool IsTrendDataPropertyChange(string? propertyName)
    {
        if (string.IsNullOrWhiteSpace(propertyName))
        {
            return false;
        }

        return propertyName.Equals("Values", StringComparison.Ordinal)
            || propertyName.Equals("OverlayValues", StringComparison.Ordinal)
            || propertyName.Equals("MiniTrendValues", StringComparison.Ordinal)
            || propertyName.EndsWith("TrendValues", StringComparison.Ordinal);
    }

    private void ScheduleRender()
    {
        if (DispatcherQueue is { } dispatcherQueue && !dispatcherQueue.HasThreadAccess)
        {
            _ = dispatcherQueue.TryEnqueue(ScheduleRender);
            return;
        }

        if (Interlocked.Exchange(ref _renderQueued, 1) == 1)
        {
            return;
        }

        if (DispatcherQueue is { } queue)
        {
            _ = queue.TryEnqueue(ProcessScheduledRender);
            return;
        }

        ProcessScheduledRender();
    }

    private void ProcessScheduledRender()
    {
        Interlocked.Exchange(ref _renderQueued, 0);
        if (!TryEnsureUiThreadForRender() || !IsLoaded)
        {
            return;
        }

        int visibleCount = MetricTrendChartRenderPlanner.NormalizeVisiblePointCount(VisiblePointCount);
        bool showAxes = ShowGrid;
        if (_requiresTransitionReset)
        {
            ResetTransitionState();
            _requiresTransitionReset = false;
        }

        if (_requiresSeriesRebuild)
        {
            RebuildChartSeries();
            _requiresSeriesRebuild = false;
        }

        bool viewportSwitchRequested = _pendingViewportSwitch
            && (!_hasTransitionSnapshot || _transitionSnapshot.VisiblePointCount != visibleCount);
        _pendingViewportSwitch = false;

        bool shrinkingViewportSwitch = viewportSwitchRequested && IsShrinkingViewportSwitch(visibleCount);

        ApplyVisualState(visibleCount, showAxes);
        ChartRenderMeta renderMeta = BuildRenderMeta(visibleCount);
        MetricTrendTransitionSnapshot nextTransitionSnapshot = CreateTransitionSnapshot(visibleCount, renderMeta.Plan, renderMeta.PointFallbackUsed);
        bool canAnimate = !shrinkingViewportSwitch && MetricTrendTransitionMath.CanAnimateTransition(
            enableTransitions: EnableSmoothPointTransitions,
            hasPreviousFrame: _hasTransitionSnapshot,
            previous: _transitionSnapshot,
            next: nextTransitionSnapshot);

        if (viewportSwitchRequested && !shrinkingViewportSwitch && ShouldUseViewportTransition(renderMeta.Plan, visibleCount))
        {
            ApplyViewportTransition(renderMeta.Plan, renderMeta.DomainMax, visibleCount, showAxes);
        }
        else if (shrinkingViewportSwitch)
        {
            ApplyViewportCutover(renderMeta.Plan, renderMeta.DomainMax, visibleCount, showAxes);
        }
        else
        {
            StopViewportTransitionCrossfade();
            ApplyAxes(TrendChart, _xAxis, _yAxis, renderMeta.Plan, renderMeta.DomainMax, visibleCount, showAxes, canAnimate);
            ApplySeries(renderMeta.Plan);
        }

        _transitionSnapshot = nextTransitionSnapshot;
        _hasTransitionSnapshot = true;
        LogSanitizedRenderIfNeeded(renderMeta);
    }

    private bool TryEnsureUiThreadForRender()
    {
        if (DispatcherQueue is { } dispatcherQueue && !dispatcherQueue.HasThreadAccess)
        {
            _ = dispatcherQueue.TryEnqueue(ProcessScheduledRender);
            return false;
        }

        return true;
    }

    private void ApplyVisualState(int visibleCount, bool showAxes)
    {
        BottomAxisPanel.Visibility = showAxes ? Visibility.Visible : Visibility.Collapsed;
        TopRightScaleLabel.Visibility = showAxes ? Visibility.Visible : Visibility.Collapsed;
        GridPath.Visibility = Visibility.Collapsed;
        PlotBorder.BorderThickness = showAxes ? new Thickness(0.7) : new Thickness(0);
        TimeWindowLabel.Text = $"{visibleCount} seconds";
    }

    private ChartRenderMeta BuildRenderMeta(int visibleCount)
    {
        MetricTrendChartRenderPlan plan = MetricTrendChartRenderPlanner.CreatePlan(new MetricTrendChartRenderRequest(
            Values ?? Array.Empty<double>(),
            OverlayValues ?? Array.Empty<double>(),
            visibleCount,
            ScaleMode,
            DomainMaxOverride,
            _dynamicDomainMaxRaw));

        _dynamicDomainMaxRaw = plan.NextRawDomainMax;

        return new ChartRenderMeta(
            Plan: plan,
            DomainMax: plan.DomainMax,
            MaxVisible: plan.MaxVisible,
            NonFiniteSeriesDetected: plan.NonFiniteSeriesDetected,
            DomainFallbackUsed: plan.DomainFallbackUsed,
            PointFallbackUsed: plan.LineFallbackUsed || (ShowOverlay && plan.OverlayFallbackUsed));
    }

    private void InitializeChartSurface(CartesianChart chart, Axis[] xAxes, Axis[] yAxes, ISeries[] singleSeries)
    {
        chart.XAxes = xAxes;
        chart.YAxes = yAxes;
        chart.Series = singleSeries;
        chart.ZoomMode = ZoomAndPanMode.None;
        chart.LegendPosition = LegendPosition.Hidden;
        chart.TooltipPosition = TooltipPosition.Hidden;
        chart.EasingFunction = EasingFunctions.CubicOut;
    }

    private void ApplyAxes(
        CartesianChart chart,
        Axis xAxis,
        Axis yAxis,
        MetricTrendChartRenderPlan plan,
        double domainMax,
        int visibleCount,
        bool showAxes,
        bool canAnimate)
    {
        TopRightScaleLabel.Text = showAxes ? FormatScaleLabel(domainMax) : string.Empty;

        int slotCount = Math.Max(plan.SlotCount, 2);
        double maxLimit = Math.Max(1d, slotCount - 1d);
        double xMinStep = Math.Max(1d, Math.Ceiling(Math.Max(visibleCount, 2) / 12d));
        double safeDomainMax = Math.Max(domainMax, 1d);
        double yMinStep = Math.Max(safeDomainMax / 8d, 0.01d);

        ConfigureAxis(xAxis, 0d, maxLimit, xMinStep, showAxes);
        ConfigureAxis(yAxis, 0d, safeDomainMax, yMinStep, showAxes);
        chart.AnimationsSpeed = canAnimate
            ? TimeSpan.FromMilliseconds(PointTransitionDurationMs)
            : TimeSpan.Zero;
    }

    private MetricTrendTransitionSnapshot CreateTransitionSnapshot(int visibleCount, MetricTrendChartRenderPlan plan, bool fallbackUsed)
    {
        double width = PlotBorder.ActualWidth;
        double height = PlotBorder.ActualHeight;

        return new MetricTrendTransitionSnapshot(
            VisiblePointCount: visibleCount,
            ScaleMode: ScaleMode,
            DomainMaxOverride: DomainMaxOverride,
            Width: width,
            Height: height,
            LinePointCount: plan.LineSeries.Values.Count,
            OverlayPointCount: ShowOverlay ? plan.OverlaySeries.Values.Count : 0,
            FallbackUsed: fallbackUsed);
    }

    private bool IsShrinkingViewportSwitch(int visibleCount)
    {
        return _hasTransitionSnapshot
            && visibleCount < _transitionSnapshot.VisiblePointCount;
    }

    private bool ShouldUseViewportTransition(MetricTrendChartRenderPlan plan, int visibleCount)
    {
        return EnableSmoothPointTransitions
            && _hasTransitionSnapshot
            && visibleCount >= _transitionSnapshot.VisiblePointCount
            && _transitionSnapshot.LinePointCount > 0
            && plan.LineSeries.Values.Count > 0;
    }

    private void ApplyViewportCutover(MetricTrendChartRenderPlan plan, double domainMax, int visibleCount, bool showAxes)
    {
        StopViewportTransitionCrossfade();
        RebuildChartSeries();
        ReplaceActiveSeries(plan);
        ApplyAxes(TrendChart, _xAxis, _yAxis, plan, domainMax, visibleCount, showAxes, canAnimate: false);
    }

    private void ApplyViewportTransition(MetricTrendChartRenderPlan plan, double domainMax, int visibleCount, bool showAxes)
    {
        StopViewportTransitionCrossfade();
        SnapshotCurrentStateIntoTransitionSurface();
        TrendChart.Opacity = 0d;
        TransitionChart.Opacity = 1d;
        TransitionChart.Visibility = Visibility.Visible;

        ReplaceActiveSeries(plan);
        ApplyAxes(TrendChart, _xAxis, _yAxis, plan, domainMax, visibleCount, showAxes, canAnimate: false);

        StartViewportTransitionCrossfade();
    }

    private void ApplySeries(MetricTrendChartRenderPlan plan)
    {
        UpdateObservablePoints(_primaryPoints, plan.LineSeries, renderFallback: true);
        UpdateObservablePoints(_overlayPoints, plan.OverlaySeries, renderFallback: false);

        ApplySeriesPaints(_primarySeries, _overlaySeries);

        ISeries[] targetSeries = ShowOverlay && _overlayPoints.Count > 0
            ? _dualSeries
            : _singleSeries;

        if (!ReferenceEquals(TrendChart.Series, targetSeries))
        {
            TrendChart.Series = targetSeries;
        }
    }

    private void ReplaceActiveSeries(MetricTrendChartRenderPlan plan)
    {
        _primaryPoints = CreateObservablePointsCollection(plan.LineSeries, renderFallback: true);
        _overlayPoints = CreateObservablePointsCollection(plan.OverlaySeries, renderFallback: false);
        _primarySeries.Values = _primaryPoints;
        _overlaySeries.Values = _overlayPoints;
        ApplySeriesPaints(_primarySeries, _overlaySeries);

        ISeries[] targetSeries = ShowOverlay && _overlayPoints.Count > 0
            ? _dualSeries
            : _singleSeries;

        if (!ReferenceEquals(TrendChart.Series, targetSeries))
        {
            TrendChart.Series = targetSeries;
        }
    }

    private void SnapshotCurrentStateIntoTransitionSurface()
    {
        _transitionPrimaryPoints = ClonePoints(_primaryPoints);
        _transitionOverlayPoints = ClonePoints(_overlayPoints);
        _transitionPrimarySeries.Values = _transitionPrimaryPoints;
        _transitionOverlaySeries.Values = _transitionOverlayPoints;
        ApplySeriesPaints(_transitionPrimarySeries, _transitionOverlaySeries);
        CopyAxisState(_xAxis, _transitionXAxis);
        CopyAxisState(_yAxis, _transitionYAxis);

        ISeries[] targetSeries = ShowOverlay && _transitionOverlayPoints.Count > 0
            ? _transitionDualSeries
            : _transitionSingleSeries;

        if (!ReferenceEquals(TransitionChart.Series, targetSeries))
        {
            TransitionChart.Series = targetSeries;
        }
        TransitionChart.AnimationsSpeed = TimeSpan.Zero;
    }

    private void StartViewportTransitionCrossfade()
    {
        TimeSpan duration = TimeSpan.FromMilliseconds(PointTransitionDurationMs);
        DoubleAnimation fadeOut = new()
        {
            From = 1d,
            To = 0d,
            Duration = new Duration(duration),
            EnableDependentAnimation = true,
        };
        DoubleAnimation fadeIn = new()
        {
            From = 0d,
            To = 1d,
            Duration = new Duration(duration),
            EnableDependentAnimation = true,
        };

        Storyboard storyboard = new();
        Storyboard.SetTarget(fadeOut, TransitionChart);
        Storyboard.SetTargetProperty(fadeOut, "Opacity");
        Storyboard.SetTarget(fadeIn, TrendChart);
        Storyboard.SetTargetProperty(fadeIn, "Opacity");
        storyboard.Children.Add(fadeOut);
        storyboard.Children.Add(fadeIn);
        storyboard.Completed += ViewportTransitionStoryboard_Completed;
        _viewportTransitionStoryboard = storyboard;
        EnsureViewportTransitionCleanupTimer(duration);
        storyboard.Begin();
    }

    private void ViewportTransitionStoryboard_Completed(object? sender, object e)
    {
        StopViewportTransitionCrossfade();
    }

    private void StopViewportTransitionCrossfade()
    {
        if (_viewportTransitionStoryboard is not null)
        {
            _viewportTransitionStoryboard.Completed -= ViewportTransitionStoryboard_Completed;
            _viewportTransitionStoryboard.Stop();
            _viewportTransitionStoryboard = null;
        }

        _viewportTransitionCleanupTimer?.Stop();

        TrendChart.Opacity = 1d;
        TransitionChart.Opacity = 0d;
        TransitionChart.Visibility = Visibility.Collapsed;
        ClearTransitionSurface();
    }

    private void ResetTransitionState()
    {
        StopViewportTransitionCrossfade();
        _hasTransitionSnapshot = false;
        _pendingViewportSwitch = false;
        _transitionSnapshot = default;
        _dynamicDomainMaxRaw = 0d;
        _requiresSeriesRebuild = true;
    }

    private void RebuildChartSeries()
    {
        _primarySeries = CreateSeries(_primaryPoints);
        _overlaySeries = CreateSeries(_overlayPoints);
        _singleSeries = [_primarySeries];
        _dualSeries = [_primarySeries, _overlaySeries];
        _transitionPrimarySeries = CreateSeries(_transitionPrimaryPoints);
        _transitionOverlaySeries = CreateSeries(_transitionOverlayPoints);
        _transitionSingleSeries = [_transitionPrimarySeries];
        _transitionDualSeries = [_transitionPrimarySeries, _transitionOverlaySeries];

        ApplySeriesPaints(_primarySeries, _overlaySeries);
        ApplySeriesPaints(_transitionPrimarySeries, _transitionOverlaySeries);

        TrendChart.Series = Array.Empty<ISeries>();
        ClearTransitionSurface();
    }

    private void ClearTransitionSurface()
    {
        _transitionPrimaryPoints.Clear();
        _transitionOverlayPoints.Clear();
        _transitionPrimarySeries.Values = _transitionPrimaryPoints;
        _transitionOverlaySeries.Values = _transitionOverlayPoints;
        TransitionChart.Series = Array.Empty<ISeries>();
    }

    private void EnsureViewportTransitionCleanupTimer(TimeSpan duration)
    {
        if (DispatcherQueue is null)
        {
            return;
        }

        _viewportTransitionCleanupTimer ??= DispatcherQueue.CreateTimer();
        _viewportTransitionCleanupTimer.Stop();
        _viewportTransitionCleanupTimer.Interval = duration;
        _viewportTransitionCleanupTimer.Tick -= ViewportTransitionCleanupTimer_Tick;
        _viewportTransitionCleanupTimer.Tick += ViewportTransitionCleanupTimer_Tick;
        _viewportTransitionCleanupTimer.Start();
    }

    private void ViewportTransitionCleanupTimer_Tick(DispatcherQueueTimer sender, object args)
    {
        sender.Stop();
        StopViewportTransitionCrossfade();
    }

    private void ApplySeriesPaints(LineSeries<ObservablePoint> primarySeries, LineSeries<ObservablePoint> overlaySeries)
    {
        primarySeries.Stroke = CreateStrokePaint(StrokeBrush, _defaultStrokeColor, Math.Max(0.6d, StrokeThickness));
        primarySeries.Fill = ShowAreaFill ? CreateFillPaint(FillBrush, _defaultFillColor) : null;

        SolidColorPaint overlayPaint = CreateStrokePaint(
            OverlayStrokeBrush,
            _defaultOverlayStrokeColor,
            Math.Max(0.5d, OverlayStrokeThickness));
        overlayPaint.PathEffect = OverlayDashEffect;
        overlaySeries.Stroke = overlayPaint;
        overlaySeries.Fill = null;
    }

    private static ObservableCollection<ObservablePoint> CreateObservablePointsCollection(
        MetricTrendChartSeriesWindow window,
        bool renderFallback)
    {
        ObservableCollection<ObservablePoint> points = [];
        UpdateObservablePoints(points, window, renderFallback);
        return points;
    }

    private static ObservableCollection<ObservablePoint> ClonePoints(ObservableCollection<ObservablePoint> source)
    {
        ObservableCollection<ObservablePoint> clone = [];
        foreach (ObservablePoint point in source)
        {
            clone.Add(new ObservablePoint(point.X, point.Y));
        }

        return clone;
    }

    private static void CopyAxisState(Axis source, Axis target)
    {
        target.MinLimit = source.MinLimit;
        target.MaxLimit = source.MaxLimit;
        target.MinStep = source.MinStep;
        target.ForceStepToMin = source.ForceStepToMin;
        target.ShowSeparatorLines = source.ShowSeparatorLines;
        target.LabelsPaint = source.LabelsPaint;
        target.SeparatorsPaint = source.SeparatorsPaint;
        target.SubseparatorsPaint = source.SubseparatorsPaint;
        target.TicksPaint = source.TicksPaint;
        target.TextSize = source.TextSize;
    }

    private static void UpdateObservablePoints(
        ObservableCollection<ObservablePoint> target,
        MetricTrendChartSeriesWindow window,
        bool renderFallback)
    {
        if (window.Values.Count == 0)
        {
            if (renderFallback)
            {
                SetOrAddPoint(target, 0, 0d, 0d);
                SetOrAddPoint(target, 1, 1d, 0d);
                TrimExcessPoints(target, 2);
                return;
            }

            TrimExcessPoints(target, 0);
            return;
        }

        for (int index = 0; index < window.Values.Count; index++)
        {
            double value = window.Values[index];
            SetOrAddPoint(
                target,
                index,
                window.LeadingSlots + index,
                SanitizeSeriesValue(value));
        }

        TrimExcessPoints(target, window.Values.Count);
    }

    private static void SetOrAddPoint(ObservableCollection<ObservablePoint> target, int index, double x, double y)
    {
        if (index < target.Count)
        {
            target[index].X = x;
            target[index].Y = y;
            return;
        }

        target.Add(new ObservablePoint(x, y));
    }

    private static void TrimExcessPoints(ObservableCollection<ObservablePoint> target, int desiredCount)
    {
        while (target.Count > desiredCount)
        {
            target.RemoveAt(target.Count - 1);
        }
    }

    private static double SanitizeSeriesValue(double value)
    {
        return !double.IsFinite(value) || value <= 0d
            ? 0d
            : value;
    }

    private void ConfigureAxis(Axis axis, double minLimit, double maxLimit, double minStep, bool showAxes)
    {
        axis.MinLimit = minLimit;
        axis.MaxLimit = maxLimit;
        axis.MinStep = minStep;
        axis.ForceStepToMin = true;
        axis.ShowSeparatorLines = showAxes;
        axis.LabelsPaint = null;
        axis.SeparatorsPaint = showAxes ? CreateGridPaint() : null;
        axis.SubseparatorsPaint = null;
        axis.TicksPaint = null;
        axis.TextSize = 0d;
    }

    private static LineSeries<ObservablePoint> CreateSeries(ObservableCollection<ObservablePoint> values)
    {
        return new LineSeries<ObservablePoint>
        {
            Values = values,
            GeometrySize = 0d,
            LineSmoothness = 0d,
            IsVisibleAtLegend = false,
        };
    }

    private void LogSanitizedRenderIfNeeded(ChartRenderMeta renderMeta)
    {
        if (renderMeta.NonFiniteSeriesDetected || renderMeta.DomainFallbackUsed || renderMeta.PointFallbackUsed)
        {
            Debug.WriteLine(
                $"[MetricTrendChart] Sanitized render inputs. scale={ScaleMode}, maxVisible={renderMeta.MaxVisible}, domainMax={renderMeta.DomainMax}, fallbackUsed={renderMeta.DomainFallbackUsed || renderMeta.PointFallbackUsed}, nonFiniteSeries={renderMeta.NonFiniteSeriesDetected}");
        }
    }

    private static void OnChartPropertyChanged(DependencyObject dependencyObject, DependencyPropertyChangedEventArgs args)
    {
        MetricTrendChart chart = (MetricTrendChart)dependencyObject;
        if (chart.DispatcherQueue is { } dispatcherQueue && !dispatcherQueue.HasThreadAccess)
        {
            _ = dispatcherQueue.TryEnqueue(() => chart.HandleChartPropertyChanged(args.Property, args.OldValue, args.NewValue));
            return;
        }

        chart.HandleChartPropertyChanged(args.Property, args.OldValue, args.NewValue);
    }

    private void HandleChartPropertyChanged(DependencyProperty property, object oldValue, object newValue)
    {
        if ((property == ScaleModeProperty || property == DomainMaxOverrideProperty)
            && !Equals(oldValue, newValue))
        {
            _dynamicDomainMaxRaw = 0d;
        }

        if ((property == ScaleModeProperty
                || property == DomainMaxOverrideProperty
                || property == ShowOverlayProperty
                || property == ShowAreaFillProperty
                || property == ChartIdentityKeyProperty)
            && !Equals(oldValue, newValue))
        {
            _requiresTransitionReset = true;
        }

        if (property == VisiblePointCountProperty && !Equals(oldValue, newValue))
        {
            _pendingViewportSwitch = true;
        }

        if (property == PointTransitionDurationMsProperty)
        {
            int normalized = MetricTrendTransitionMath.NormalizeDurationMs(newValue is int value ? value : MetricTrendTransitionMath.DefaultDurationMs);
            if (!Equals(newValue, normalized))
            {
                SetValue(PointTransitionDurationMsProperty, normalized);
                return;
            }
        }

        ScheduleRender();
    }

    private string FormatScaleLabel(double domainMax)
    {
        if (!double.IsFinite(domainMax))
        {
            Debug.WriteLine($"[MetricTrendChart] Non-finite domain label detected. scale={ScaleMode}, domainMax={domainMax}");
            return ScaleMode switch
            {
                MetricTrendScaleMode.CpuPercent => "0%",
                MetricTrendScaleMode.MemoryBytes => "0 B",
                MetricTrendScaleMode.BitsRate => "0 bps",
                _ => "0 B/s",
            };
        }

        if (ScaleMode == MetricTrendScaleMode.CpuPercent)
        {
            return $"{Math.Max(0d, domainMax):F0}%";
        }

        if (ScaleMode == MetricTrendScaleMode.BitsRate)
        {
            return ValueFormat.FormatBitsRate(Math.Max(0d, domainMax));
        }

        ulong clamped = ClampToUlongNonNegative(domainMax);
        return ScaleMode == MetricTrendScaleMode.MemoryBytes
            ? ValueFormat.FormatBytes(clamped)
            : ValueFormat.FormatRate(clamped);
    }

    private static ulong ClampToUlongNonNegative(double value)
    {
        if (!double.IsFinite(value) || value <= 0d)
        {
            return 0UL;
        }

        if (value >= ulong.MaxValue)
        {
            return ulong.MaxValue;
        }

        return (ulong)Math.Round(value);
    }

    private SolidColorPaint CreateGridPaint()
    {
        SolidColorPaint paint = CreateFillPaint(GridBrush, _defaultGridColor);
        paint.StrokeCap = SKStrokeCap.Round;
        paint.StrokeJoin = SKStrokeJoin.Round;
        paint.StrokeThickness = 0.65f;
        return paint;
    }

    private SolidColorPaint CreateStrokePaint(Brush brush, Color fallback, double thickness)
    {
        SolidColorPaint paint = new(ToSkColor(brush, fallback), (float)thickness)
        {
            StrokeCap = SKStrokeCap.Round,
            StrokeJoin = SKStrokeJoin.Round,
        };

        return paint;
    }

    private SolidColorPaint CreateFillPaint(Brush brush, Color fallback)
    {
        return new SolidColorPaint(ToSkColor(brush, fallback));
    }

    private static SKColor ToSkColor(Brush brush, Color fallback)
    {
        Color color = brush is SolidColorBrush solidColorBrush
            ? solidColorBrush.Color
            : fallback;

        return new SKColor(color.R, color.G, color.B, color.A);
    }

    private readonly record struct ChartRenderMeta(
        MetricTrendChartRenderPlan Plan,
        double DomainMax,
        double MaxVisible,
        bool NonFiniteSeriesDetected,
        bool DomainFallbackUsed,
        bool PointFallbackUsed);
}
