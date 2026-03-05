using BatCave.Charts;
using BatCave.Converters;
using Microsoft.UI;
using Microsoft.UI.Dispatching;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Media;
using System;
using System.Collections.Generic;
using System.ComponentModel;
using System.Diagnostics;
using System.Threading;
using Windows.Foundation;
using Windows.UI;

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

    private readonly Brush _defaultStrokeBrush = new SolidColorBrush(Colors.DodgerBlue);
    private readonly Brush _defaultFillBrush = new SolidColorBrush(Color.FromArgb(38, 30, 144, 255));
    private readonly Brush _defaultOverlayStrokeBrush = new SolidColorBrush(Color.FromArgb(204, 76, 93, 112));
    private readonly Brush _defaultGridBrush = new SolidColorBrush(Color.FromArgb(36, 140, 148, 163));
    private readonly PointCollection _linePoints = [];
    private readonly PointCollection _overlayPoints = [];
    private readonly PointCollection _areaPoints = [];
    private readonly PointCollection _targetLinePoints = [];
    private readonly PointCollection _targetOverlayPoints = [];
    private readonly PointCollection _animationStartLinePoints = [];
    private readonly PointCollection _animationStartOverlayPoints = [];
    private readonly DoubleCollection _overlayDashArray = [2, 2];

    private double _dynamicDomainMaxRaw;
    private double _cachedGridWidth = double.NaN;
    private double _cachedGridHeight = double.NaN;
    private Geometry? _cachedGridGeometry;
    private DispatcherQueueTimer? _transitionTimer;
    private DateTimeOffset _transitionStartedAt;
    private double _transitionPlotHeight;
    private bool _isTransitionRunning;
    private bool _hasTransitionSnapshot;
    private MetricTrendTransitionSnapshot _transitionSnapshot;
    private int _pendingInvalidationMask = (int)RenderInvalidation.All;
    private int _renderQueued;
    private INotifyPropertyChanged? _dataContextNotifier;

    public MetricTrendChart()
    {
        InitializeComponent();
        LinePolyline.Points = _linePoints;
        OverlayPolyline.Points = _overlayPoints;
        AreaPolygon.Points = _areaPoints;
        OverlayPolyline.StrokeDashArray = _overlayDashArray;

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
        get => (Brush?)GetValue(StrokeBrushProperty) ?? _defaultStrokeBrush;
        set => SetValue(StrokeBrushProperty, value);
    }

    public Brush FillBrush
    {
        get => (Brush?)GetValue(FillBrushProperty) ?? _defaultFillBrush;
        set => SetValue(FillBrushProperty, value);
    }

    public Brush GridBrush
    {
        get => (Brush?)GetValue(GridBrushProperty) ?? _defaultGridBrush;
        set => SetValue(GridBrushProperty, value);
    }

    public Brush OverlayStrokeBrush
    {
        get => (Brush?)GetValue(OverlayStrokeBrushProperty) ?? _defaultOverlayStrokeBrush;
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
        Invalidate(RenderInvalidation.Geometry | RenderInvalidation.Axes);
        ScheduleRender();
    }

    private void OnLoaded(object sender, RoutedEventArgs e)
    {
        StopTransition();
        _hasTransitionSnapshot = false;
        AttachDataContextNotifier(DataContext as INotifyPropertyChanged);
        Invalidate(RenderInvalidation.All);
        ScheduleRender();
    }

    private void OnUnloaded(object sender, RoutedEventArgs e)
    {
        StopTransition();
        _hasTransitionSnapshot = false;
        AttachDataContextNotifier(null);
    }

    private void OnDataContextChanged(FrameworkElement sender, DataContextChangedEventArgs args)
    {
        AttachDataContextNotifier(args.NewValue as INotifyPropertyChanged);
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

        Invalidate(RenderInvalidation.Geometry | RenderInvalidation.Axes);
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

    private void PlotBorder_SizeChanged(object sender, SizeChangedEventArgs e)
    {
        Invalidate(RenderInvalidation.Geometry | RenderInvalidation.Axes);
        ScheduleRender();
    }

    private void Invalidate(RenderInvalidation invalidation)
    {
        Interlocked.Or(ref _pendingInvalidationMask, (int)invalidation);
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
        if (!TryEnsureUiThreadForRender())
        {
            return;
        }

        if (!IsLoaded)
        {
            return;
        }

        int invalidationMask = Interlocked.Exchange(ref _pendingInvalidationMask, (int)RenderInvalidation.None);
        if (invalidationMask == (int)RenderInvalidation.None)
        {
            return;
        }

        if (!TryGetRenderSize(out double width, out double height))
        {
            Interlocked.Or(ref _pendingInvalidationMask, invalidationMask);
            return;
        }

        RenderInvalidation invalidation = (RenderInvalidation)invalidationMask;
        int visibleCount = MetricTrendChartRenderPlanner.NormalizeVisiblePointCount(VisiblePointCount);
        bool showAxes = ShowGrid;

        if ((invalidation & RenderInvalidation.Style) != 0)
        {
            ApplyVisualState(visibleCount, showAxes);
        }

        if ((invalidation & (RenderInvalidation.Geometry | RenderInvalidation.Axes)) == 0)
        {
            return;
        }

        ChartRenderMeta renderMeta = BuildRenderMeta(visibleCount, width, height);

        if ((invalidation & RenderInvalidation.Axes) != 0)
        {
            ApplyAxes(showAxes, width, height, renderMeta.DomainMax);
        }

        if ((invalidation & RenderInvalidation.Geometry) != 0)
        {
            ApplySeriesGeometry(renderMeta, visibleCount, width, height);
        }

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
        GridPath.Visibility = showAxes ? Visibility.Visible : Visibility.Collapsed;
        PlotBorder.BorderThickness = showAxes ? new Thickness(0.7) : new Thickness(0);
        TimeWindowLabel.Text = $"{visibleCount} seconds";

        LinePolyline.Stroke = StrokeBrush;
        AreaPolygon.Fill = ShowAreaFill ? FillBrush : null;
        GridPath.Stroke = GridBrush;
        LinePolyline.StrokeThickness = Math.Max(0.6, StrokeThickness);
        OverlayPolyline.Stroke = OverlayStrokeBrush;
        OverlayPolyline.StrokeThickness = Math.Max(0.5, OverlayStrokeThickness);
        OverlayPolyline.StrokeDashArray = _overlayDashArray;
        OverlayPolyline.Visibility = ShowOverlay ? Visibility.Visible : Visibility.Collapsed;
    }

    private bool TryGetRenderSize(out double width, out double height)
    {
        width = PlotBorder.ActualWidth;
        height = PlotBorder.ActualHeight;
        if (!double.IsFinite(width) || !double.IsFinite(height) || width <= 1d || height <= 1d)
        {
            if (!double.IsFinite(width) || !double.IsFinite(height))
            {
                Debug.WriteLine($"[MetricTrendChart] Skipping render due to non-finite size. scale={ScaleMode}, width={width}, height={height}");
            }

            return false;
        }

        return true;
    }

    private ChartRenderMeta BuildRenderMeta(int visibleCount, double width, double height)
    {
        MetricTrendChartRenderPlan plan = MetricTrendChartRenderPlanner.CreatePlan(new MetricTrendChartRenderRequest(
            Values ?? Array.Empty<double>(),
            OverlayValues ?? Array.Empty<double>(),
            visibleCount,
            width,
            height,
            ScaleMode,
            DomainMaxOverride,
            _dynamicDomainMaxRaw));

        _dynamicDomainMaxRaw = plan.NextRawDomainMax;
        CopyPointCollection(plan.LinePoints, _targetLinePoints);
        CopyPointCollection(plan.OverlayPoints, _targetOverlayPoints);

        return new ChartRenderMeta(
            DomainMax: plan.DomainMax,
            MaxVisible: plan.MaxVisible,
            NonFiniteSeriesDetected: plan.NonFiniteSeriesDetected,
            DomainFallbackUsed: plan.DomainFallbackUsed,
            PointFallbackUsed: plan.LineFallbackUsed || (ShowOverlay && plan.OverlayFallbackUsed),
            LinePointCount: _targetLinePoints.Count,
            OverlayPointCount: ShowOverlay ? _targetOverlayPoints.Count : 0);
    }

    private void ApplyAxes(bool showAxes, double width, double height, double domainMax)
    {
        if (!showAxes)
        {
            TopRightScaleLabel.Text = string.Empty;
            GridPath.Data = null;
            return;
        }

        TopRightScaleLabel.Text = FormatScaleLabel(domainMax);
        GridPath.Data = GetOrBuildGridGeometry(width, height);
    }

    private Geometry GetOrBuildGridGeometry(double width, double height)
    {
        double roundedWidth = Math.Round(width, 2);
        double roundedHeight = Math.Round(height, 2);
        if (_cachedGridGeometry is not null
            && _cachedGridWidth == roundedWidth
            && _cachedGridHeight == roundedHeight)
        {
            return _cachedGridGeometry;
        }

        _cachedGridWidth = roundedWidth;
        _cachedGridHeight = roundedHeight;
        _cachedGridGeometry = BuildGridGeometry(roundedWidth, roundedHeight, verticalDivisions: 12, horizontalDivisions: 8);
        return _cachedGridGeometry;
    }

    private void ApplySeriesGeometry(ChartRenderMeta renderMeta, int visiblePointCount, double width, double height)
    {
        MetricTrendTransitionSnapshot nextSnapshot = new(
            VisiblePointCount: visiblePointCount,
            ScaleMode: ScaleMode,
            DomainMaxOverride: DomainMaxOverride,
            Width: width,
            Height: height,
            LinePointCount: renderMeta.LinePointCount,
            OverlayPointCount: renderMeta.OverlayPointCount,
            FallbackUsed: renderMeta.PointFallbackUsed);

        bool canAnimate = MetricTrendTransitionMath.CanAnimateTransition(
            enableTransitions: EnableSmoothPointTransitions,
            hasPreviousFrame: _hasTransitionSnapshot,
            previous: _transitionSnapshot,
            next: nextSnapshot);

        if (canAnimate)
        {
            StartOrRetargetTransition(height);
        }
        else
        {
            StopTransition();
            ApplyTargetPointsInstant(height);
        }

        _transitionSnapshot = nextSnapshot;
        _hasTransitionSnapshot = true;
    }

    private void StartOrRetargetTransition(double height)
    {
        if (_linePoints.Count != _targetLinePoints.Count
            || (ShowOverlay && _overlayPoints.Count != _targetOverlayPoints.Count))
        {
            ApplyTargetPointsInstant(height);
            return;
        }

        CopyPointCollection(_linePoints, _animationStartLinePoints);
        if (ShowOverlay)
        {
            CopyPointCollection(_overlayPoints, _animationStartOverlayPoints);
        }
        else
        {
            _animationStartOverlayPoints.Clear();
        }

        _transitionPlotHeight = height;
        _transitionStartedAt = DateTimeOffset.UtcNow;
        _isTransitionRunning = true;
        EnsureTransitionTimer();
        if (_transitionTimer is null)
        {
            _isTransitionRunning = false;
            ApplyTargetPointsInstant(height);
            return;
        }

        ApplyTransitionFrame(easedProgress: 0d);
        _transitionTimer.Start();
    }

    private void EnsureTransitionTimer()
    {
        if (_transitionTimer is not null || DispatcherQueue is null)
        {
            return;
        }

        _transitionTimer = DispatcherQueue.CreateTimer();
        _transitionTimer.Interval = TimeSpan.FromMilliseconds(16);
        _transitionTimer.Tick += TransitionTimer_Tick;
    }

    private void TransitionTimer_Tick(DispatcherQueueTimer sender, object args)
    {
        if (!_isTransitionRunning)
        {
            sender.Stop();
            return;
        }

        double progress = MetricTrendTransitionMath.ComputeProgress(
            DateTimeOffset.UtcNow - _transitionStartedAt,
            PointTransitionDurationMs);
        double eased = MetricTrendTransitionMath.EaseOutCubic(progress);
        ApplyTransitionFrame(eased);

        if (progress >= 1d)
        {
            _isTransitionRunning = false;
            sender.Stop();
        }
    }

    private void ApplyTransitionFrame(double easedProgress)
    {
        if (_animationStartLinePoints.Count != _targetLinePoints.Count
            || (ShowOverlay && _animationStartOverlayPoints.Count != _targetOverlayPoints.Count))
        {
            _isTransitionRunning = false;
            ApplyTargetPointsInstant(_transitionPlotHeight);
            return;
        }

        InterpolatePointCollection(_animationStartLinePoints, _targetLinePoints, easedProgress, _linePoints);
        if (ShowOverlay)
        {
            InterpolatePointCollection(_animationStartOverlayPoints, _targetOverlayPoints, easedProgress, _overlayPoints);
        }
        else
        {
            _overlayPoints.Clear();
        }

        UpdateAreaGeometry(_transitionPlotHeight);
    }

    private void ApplyTargetPointsInstant(double height)
    {
        CopyPointCollection(_targetLinePoints, _linePoints);
        if (ShowOverlay)
        {
            CopyPointCollection(_targetOverlayPoints, _overlayPoints);
        }
        else
        {
            _overlayPoints.Clear();
        }

        UpdateAreaGeometry(height);
    }

    private void UpdateAreaGeometry(double height)
    {
        if (ShowAreaFill)
        {
            SparklineMath.WriteFillPolygon(_linePoints, height, _areaPoints);
            AreaPolygon.Fill = FillBrush;
        }
        else
        {
            AreaPolygon.Fill = null;
            _areaPoints.Clear();
        }

        OverlayPolyline.Visibility = ShowOverlay ? Visibility.Visible : Visibility.Collapsed;
    }

    private void StopTransition()
    {
        _isTransitionRunning = false;
        _transitionTimer?.Stop();
    }

    private static void InterpolatePointCollection(
        IList<Point> start,
        IList<Point> target,
        double easedProgress,
        PointCollection destination)
    {
        if (start.Count != target.Count)
        {
            CopyPointCollection(target, destination);
            return;
        }

        EnsurePointCollectionSize(destination, target.Count);
        for (int index = 0; index < target.Count; index++)
        {
            destination[index] = MetricTrendTransitionMath.InterpolatePoint(start[index], target[index], easedProgress);
        }
    }

    private static void CopyPointCollection(IReadOnlyList<Point> source, PointCollection destination)
    {
        EnsurePointCollectionSize(destination, source.Count);
        for (int index = 0; index < source.Count; index++)
        {
            destination[index] = source[index];
        }
    }

    private static void CopyPointCollection(IList<Point> source, PointCollection destination)
    {
        EnsurePointCollectionSize(destination, source.Count);
        for (int index = 0; index < source.Count; index++)
        {
            destination[index] = source[index];
        }
    }

    private static void EnsurePointCollectionSize(PointCollection destination, int count)
    {
        while (destination.Count > count)
        {
            destination.RemoveAt(destination.Count - 1);
        }

        while (destination.Count < count)
        {
            destination.Add(new Point(0d, 0d));
        }
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

        if (property == PointTransitionDurationMsProperty)
        {
            int normalized = MetricTrendTransitionMath.NormalizeDurationMs(newValue is int value ? value : MetricTrendTransitionMath.DefaultDurationMs);
            if (!Equals(newValue, normalized))
            {
                SetValue(PointTransitionDurationMsProperty, normalized);
                return;
            }
        }

        if (property == EnableSmoothPointTransitionsProperty
            && newValue is bool enabled
            && !enabled)
        {
            StopTransition();
            ApplyTargetPointsInstant(Math.Max(1d, PlotBorder.ActualHeight));
        }

        Invalidate(ResolveInvalidation(property));
        ScheduleRender();
    }

    private static RenderInvalidation ResolveInvalidation(DependencyProperty property)
    {
        if (property == ShowGridProperty)
        {
            return RenderInvalidation.Style | RenderInvalidation.Axes;
        }

        if (property == StrokeBrushProperty
            || property == FillBrushProperty
            || property == GridBrushProperty
            || property == OverlayStrokeBrushProperty
            || property == StrokeThicknessProperty
            || property == OverlayStrokeThicknessProperty
            || property == ShowAreaFillProperty
            || property == ShowOverlayProperty)
        {
            return RenderInvalidation.Style;
        }

        if (property == VisiblePointCountProperty)
        {
            return RenderInvalidation.Style | RenderInvalidation.Geometry | RenderInvalidation.Axes;
        }

        if (property == EnableSmoothPointTransitionsProperty || property == PointTransitionDurationMsProperty)
        {
            return RenderInvalidation.Geometry;
        }

        return RenderInvalidation.Geometry | RenderInvalidation.Axes;
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

    private static Geometry BuildGridGeometry(double width, double height, int verticalDivisions, int horizontalDivisions)
    {
        GeometryGroup group = new();
        if (width <= 0 || height <= 0)
        {
            return group;
        }

        for (int index = 1; index < verticalDivisions; index++)
        {
            double x = Math.Round((width * index) / verticalDivisions, 2);
            group.Children.Add(new LineGeometry
            {
                StartPoint = new Windows.Foundation.Point(x, 0),
                EndPoint = new Windows.Foundation.Point(x, height),
            });
        }

        for (int index = 1; index < horizontalDivisions; index++)
        {
            double y = Math.Round((height * index) / horizontalDivisions, 2);
            group.Children.Add(new LineGeometry
            {
                StartPoint = new Windows.Foundation.Point(0, y),
                EndPoint = new Windows.Foundation.Point(width, y),
            });
        }

        return group;
    }

    [Flags]
    private enum RenderInvalidation
    {
        None = 0,
        Style = 1 << 0,
        Geometry = 1 << 1,
        Axes = 1 << 2,
        All = Style | Geometry | Axes,
    }

    private readonly record struct ChartRenderMeta(
        double DomainMax,
        double MaxVisible,
        bool NonFiniteSeriesDetected,
        bool DomainFallbackUsed,
        bool PointFallbackUsed,
        int LinePointCount,
        int OverlayPointCount);
}




