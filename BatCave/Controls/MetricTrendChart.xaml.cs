using BatCave.Charts;
using BatCave.Converters;
using Microsoft.UI;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Media;
using System;
using System.Collections.Generic;
using System.Diagnostics;
using Windows.Foundation;
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

    private double _dynamicDomainMaxRaw;
    private readonly Brush _defaultStrokeBrush = new SolidColorBrush(Colors.DodgerBlue);
    private readonly Brush _defaultFillBrush = new SolidColorBrush(Color.FromArgb(38, 30, 144, 255));
    private readonly Brush _defaultOverlayStrokeBrush = new SolidColorBrush(Color.FromArgb(204, 76, 93, 112));
    private readonly Brush _defaultGridBrush = new SolidColorBrush(Color.FromArgb(36, 140, 148, 163));

    public MetricTrendChart()
    {
        InitializeComponent();
        Loaded += OnLoaded;
        PlotBorder.SizeChanged += PlotBorder_SizeChanged;
        RefreshChart();
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
        RefreshChart();
    }

    private void OnLoaded(object sender, RoutedEventArgs e)
    {
        RefreshChart();
    }

    private void PlotBorder_SizeChanged(object sender, SizeChangedEventArgs e)
    {
        RefreshChart();
    }

    private void RefreshChart()
    {
        if (DispatcherQueue is { } dispatcherQueue && !dispatcherQueue.HasThreadAccess)
        {
            _ = dispatcherQueue.TryEnqueue(RefreshChart);
            return;
        }

        if (!IsLoaded)
        {
            return;
        }

        int visibleCount = NormalizeVisiblePointCount(VisiblePointCount);
        bool showAxes = ShowGrid;
        BottomAxisPanel.Visibility = showAxes ? Visibility.Visible : Visibility.Collapsed;
        TopRightScaleLabel.Visibility = showAxes ? Visibility.Visible : Visibility.Collapsed;
        GridPath.Visibility = showAxes ? Visibility.Visible : Visibility.Collapsed;
        PlotBorder.BorderThickness = showAxes ? new Thickness(0.7) : new Thickness(0);

        TimeWindowLabel.Text = $"{visibleCount} seconds";
        LinePolyline.Stroke = StrokeBrush;
        AreaPolygon.Fill = FillBrush;
        GridPath.Stroke = GridBrush;
        LinePolyline.StrokeThickness = Math.Max(0.6, StrokeThickness);
        OverlayPolyline.Stroke = OverlayStrokeBrush;
        OverlayPolyline.StrokeThickness = Math.Max(0.5, OverlayStrokeThickness);
        OverlayPolyline.StrokeDashArray = new DoubleCollection { 2, 2 };

        double width = PlotBorder.ActualWidth;
        double height = PlotBorder.ActualHeight;
        if (!double.IsFinite(width) || !double.IsFinite(height) || width <= 1d || height <= 1d)
        {
            if (!double.IsFinite(width) || !double.IsFinite(height))
            {
                Debug.WriteLine($"[MetricTrendChart] Skipping render due to non-finite size. scale={ScaleMode}, width={width}, height={height}");
            }
            return;
        }

        IReadOnlyList<double> source = Values ?? Array.Empty<double>();
        double[] window = CopyWindow(source, visibleCount);
        double[] overlayWindow = CopyWindow(OverlayValues ?? Array.Empty<double>(), visibleCount);
        bool nonFiniteSeriesDetected = ContainsNonFinite(window) || ContainsNonFinite(overlayWindow);
        double maxVisible = Math.Max(ResolveMax(window), ResolveMax(overlayWindow));
        double domainMax = ResolveDomainMax(maxVisible);
        (double floor, _) = ResolveDomainPolicy();
        bool domainFallbackUsed = false;
        if (!double.IsFinite(domainMax) || domainMax <= 0d)
        {
            domainFallbackUsed = true;
            domainMax = floor;
        }

        if (showAxes)
        {
            TopRightScaleLabel.Text = FormatScaleLabel(domainMax);
            GridPath.Data = BuildGridGeometry(width, height, verticalDivisions: 12, horizontalDivisions: 8);
        }
        else
        {
            TopRightScaleLabel.Text = string.Empty;
            GridPath.Data = null;
        }

        IReadOnlyList<Point> linePoints = SparklineMath.BuildPointsInDomainWithFallback(window, width, height, minDomain: 0d, maxDomain: domainMax);
        IReadOnlyList<Point> overlayPoints = SparklineMath.BuildPointsInDomainWithFallback(overlayWindow, width, height, minDomain: 0d, maxDomain: domainMax);

        LinePolyline.Points = SparklineMath.ToPointCollection(linePoints);
        if (ShowAreaFill)
        {
            IReadOnlyList<Point> fillPoints = SparklineMath.BuildFillPolygon(linePoints, width, height);
            AreaPolygon.Fill = FillBrush;
            AreaPolygon.Points = SparklineMath.ToPointCollection(fillPoints);
        }
        else
        {
            AreaPolygon.Fill = null;
            AreaPolygon.Points = new PointCollection();
        }
        OverlayPolyline.Points = SparklineMath.ToPointCollection(overlayPoints);
        OverlayPolyline.Visibility = ShowOverlay ? Visibility.Visible : Visibility.Collapsed;

        bool pointFallbackUsed = IsFlatlineFallback(linePoints)
            || (ShowOverlay && IsFlatlineFallback(overlayPoints));
        if (nonFiniteSeriesDetected || domainFallbackUsed || pointFallbackUsed)
        {
            Debug.WriteLine(
                $"[MetricTrendChart] Sanitized render inputs. scale={ScaleMode}, maxVisible={maxVisible}, domainMax={domainMax}, fallbackUsed={domainFallbackUsed || pointFallbackUsed}, nonFiniteSeries={nonFiniteSeriesDetected}");
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

        RefreshChart();
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

    private static double[] CopyWindow(IReadOnlyList<double> values, int visiblePointCount)
    {
        int take = Math.Min(values.Count, visiblePointCount);
        if (take <= 0)
        {
            return new[] { 0d };
        }

        double[] result = new double[take];
        int sourceStart = values.Count - take;
        for (int index = 0; index < take; index++)
        {
            result[index] = values[sourceStart + index];
        }

        return result;
    }

    private static double ResolveMax(IReadOnlyList<double> values)
    {
        double max = 0d;
        for (int index = 0; index < values.Count; index++)
        {
            if (values[index] > max)
            {
                max = values[index];
            }
        }

        return max;
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

    private static bool ContainsNonFinite(IReadOnlyList<double> values)
    {
        for (int index = 0; index < values.Count; index++)
        {
            if (!double.IsFinite(values[index]))
            {
                return true;
            }
        }

        return false;
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

    private static bool IsFlatlineFallback(IReadOnlyList<Point> points)
    {
        if (points.Count != 2)
        {
            return false;
        }

        return points[0].X == 0d
            && points[0].Y == 0d
            && points[1].X == 1d
            && points[1].Y == 0d;
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
                StartPoint = new Point(x, 0),
                EndPoint = new Point(x, height),
            });
        }

        for (int index = 1; index < horizontalDivisions; index++)
        {
            double y = Math.Round((height * index) / horizontalDivisions, 2);
            group.Children.Add(new LineGeometry
            {
                StartPoint = new Point(0, y),
                EndPoint = new Point(width, y),
            });
        }

        return group;
    }
}
