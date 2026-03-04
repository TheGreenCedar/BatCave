using BatCave.Charts;
using BatCave.Converters;
using Microsoft.UI;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Media;
using System;
using System.Buffers;
using System.Collections.Generic;
using System.Diagnostics;
using System.Runtime.InteropServices;
using System.Threading;
using Windows.UI;

namespace BatCave.Controls;

public sealed partial class MetricTrendChart : UserControl
{
    private const int MinVisiblePointCount = 60;
    private const int MaxVisiblePointCount = 120;

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
        new PropertyMetadata(MinVisiblePointCount, OnChartPropertyChanged));

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

    private readonly Brush _defaultStrokeBrush = new SolidColorBrush(Colors.DodgerBlue);
    private readonly Brush _defaultFillBrush = new SolidColorBrush(Color.FromArgb(38, 30, 144, 255));
    private readonly Brush _defaultOverlayStrokeBrush = new SolidColorBrush(Color.FromArgb(204, 76, 93, 112));
    private readonly Brush _defaultGridBrush = new SolidColorBrush(Color.FromArgb(36, 140, 148, 163));
    private readonly PointCollection _linePoints = [];
    private readonly PointCollection _overlayPoints = [];
    private readonly PointCollection _areaPoints = [];
    private readonly DoubleCollection _overlayDashArray = [2, 2];

    private double _dynamicDomainMaxRaw;
    private double _cachedGridWidth = double.NaN;
    private double _cachedGridHeight = double.NaN;
    private Geometry? _cachedGridGeometry;
    private int _pendingInvalidationMask = (int)RenderInvalidation.All;
    private int _renderQueued;

    public MetricTrendChart()
    {
        InitializeComponent();
        LinePolyline.Points = _linePoints;
        OverlayPolyline.Points = _overlayPoints;
        AreaPolygon.Points = _areaPoints;
        OverlayPolyline.StrokeDashArray = _overlayDashArray;

        Loaded += OnLoaded;
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

    public void RequestRender()
    {
        Invalidate(RenderInvalidation.Geometry | RenderInvalidation.Axes);
        ScheduleRender();
    }

    private void OnLoaded(object sender, RoutedEventArgs e)
    {
        Invalidate(RenderInvalidation.All);
        ScheduleRender();
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
        int visibleCount = NormalizeVisiblePointCount(VisiblePointCount);
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
            ApplySeriesGeometry(height);
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
        IReadOnlyList<double> values = Values ?? Array.Empty<double>();
        IReadOnlyList<double> overlayValues = OverlayValues ?? Array.Empty<double>();

        (int lineStart, int lineCount) = ResolveWindow(values, visibleCount);
        (int overlayStart, int overlayCount) = ResolveWindow(overlayValues, visibleCount);

        double[]? lineLease = null;
        double[]? overlayLease = null;
        ReadOnlySpan<double> lineWindow = AcquireWindowSpan(values, lineStart, lineCount, out lineLease);
        ReadOnlySpan<double> overlayWindow = AcquireWindowSpan(overlayValues, overlayStart, overlayCount, out overlayLease);

        try
        {
            WindowStats lineStats = AnalyzeWindow(lineWindow);
            WindowStats overlayStats = AnalyzeWindow(overlayWindow);
            bool nonFiniteSeriesDetected = lineStats.HasNonFinite || overlayStats.HasNonFinite;

            double maxVisible = Math.Max(lineStats.Max, overlayStats.Max);
            double domainMax = ResolveDomainMax(maxVisible);
            (double floor, _) = ResolveDomainPolicy();
            bool domainFallbackUsed = false;
            if (!double.IsFinite(domainMax) || domainMax <= 0d)
            {
                domainFallbackUsed = true;
                domainMax = floor;
            }

            bool lineFallbackUsed = SparklineMath.WritePointsInDomainWithFallback(
                lineWindow,
                width,
                height,
                minDomain: 0d,
                maxDomain: domainMax,
                destination: _linePoints);

            bool overlayFallbackUsed = SparklineMath.WritePointsInDomainWithFallback(
                overlayWindow,
                width,
                height,
                minDomain: 0d,
                maxDomain: domainMax,
                destination: _overlayPoints);

            return new ChartRenderMeta(
                DomainMax: domainMax,
                MaxVisible: maxVisible,
                NonFiniteSeriesDetected: nonFiniteSeriesDetected,
                DomainFallbackUsed: domainFallbackUsed,
                PointFallbackUsed: lineFallbackUsed || (ShowOverlay && overlayFallbackUsed));
        }
        finally
        {
            ReturnWindowLease(lineLease);
            ReturnWindowLease(overlayLease);
        }
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

    private void ApplySeriesGeometry(double height)
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

        return RenderInvalidation.Geometry | RenderInvalidation.Axes;
    }

    private double ResolveDomainMax(double maxVisible)
    {
        (double floor, double? ceiling) = ResolveDomainPolicy();

        _dynamicDomainMaxRaw = MetricTrendScaleDomain.ResolveNextRawDomainMax(
            previousRawDomainMax: _dynamicDomainMaxRaw,
            maxVisible: maxVisible,
            floor: floor,
            ceiling: ceiling,
            paddingRatio: MetricTrendScaleDomain.DefaultPaddingRatio,
            decayFactor: MetricTrendScaleDomain.DefaultDecayFactor);

        return MetricTrendScaleDomain.ResolveRenderedDomainMax(
            rawDomainMax: _dynamicDomainMaxRaw,
            floor: floor,
            ceiling: ceiling);
    }

    private (double Floor, double? Ceiling) ResolveDomainPolicy()
    {
        double? overrideMax = double.IsNaN(DomainMaxOverride)
            ? null
            : Math.Max(0d, DomainMaxOverride);

        return ScaleMode switch
        {
            MetricTrendScaleMode.CpuPercent => (MetricTrendScaleDomain.CpuFloorPercent, MetricTrendScaleDomain.CpuCeilingPercent),
            MetricTrendScaleMode.MemoryBytes => (MetricTrendScaleDomain.MemoryFloorBytes, overrideMax),
            MetricTrendScaleMode.BitsRate => (MetricTrendScaleDomain.BitsRateFloor, overrideMax),
            _ => (MetricTrendScaleDomain.IoRateFloorBytes, overrideMax),
        };
    }

    private static (int Start, int Count) ResolveWindow(IReadOnlyList<double> values, int visiblePointCount)
    {
        if (values.Count == 0)
        {
            return (0, 0);
        }

        int count = Math.Min(values.Count, visiblePointCount);
        return (values.Count - count, count);
    }

    private static ReadOnlySpan<double> AcquireWindowSpan(
        IReadOnlyList<double> values,
        int start,
        int count,
        out double[]? lease)
    {
        lease = null;
        if (count <= 0 || values.Count == 0)
        {
            return ReadOnlySpan<double>.Empty;
        }

        int safeStart = Math.Max(0, start);
        int safeCount = Math.Min(count, values.Count - safeStart);
        if (safeCount <= 0)
        {
            return ReadOnlySpan<double>.Empty;
        }

        if (values is double[] array)
        {
            return array.AsSpan(safeStart, safeCount);
        }

        if (values is List<double> list)
        {
            return CollectionsMarshal.AsSpan(list).Slice(safeStart, safeCount);
        }

        lease = ArrayPool<double>.Shared.Rent(safeCount);
        for (int index = 0; index < safeCount; index++)
        {
            lease[index] = values[safeStart + index];
        }

        return lease.AsSpan(0, safeCount);
    }

    private static void ReturnWindowLease(double[]? lease)
    {
        if (lease is null)
        {
            return;
        }

        ArrayPool<double>.Shared.Return(lease, clearArray: false);
    }

    private static WindowStats AnalyzeWindow(ReadOnlySpan<double> values)
    {
        if (values.Length == 0)
        {
            return new WindowStats(Max: 0d, HasNonFinite: false);
        }

        double max = 0d;
        bool hasNonFinite = false;
        for (int index = 0; index < values.Length; index++)
        {
            double value = values[index];
            if (!double.IsFinite(value))
            {
                hasNonFinite = true;
                continue;
            }

            if (value > max)
            {
                max = value;
            }
        }

        return new WindowStats(Max: max, HasNonFinite: hasNonFinite);
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

    private static int NormalizeVisiblePointCount(int candidate)
    {
        return candidate >= MaxVisiblePointCount ? MaxVisiblePointCount : MinVisiblePointCount;
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
        bool PointFallbackUsed);

    private readonly record struct WindowStats(
        double Max,
        bool HasNonFinite);
}
