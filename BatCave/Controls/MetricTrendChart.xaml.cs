using System;
using System.Collections.Generic;
using BatCave.Charts;
using BatCave.Converters;
using Microsoft.UI;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Media;
using Windows.Foundation;
using Windows.UI;

namespace BatCave.Controls;

public enum MetricTrendScaleMode
{
    CpuPercent,
    MemoryBytes,
    IoRate,
}

public sealed partial class MetricTrendChart : UserControl
{
    private const int MinVisiblePointCount = 60;
    private const int MaxVisiblePointCount = 120;
    private const double MemoryFloorBytes = 256d * 1024d * 1024d;
    private const double IoRateFloorBytes = 1d * 1024d * 1024d;
    private const double DynamicCeilingPaddingRatio = 1.12;
    private const double DynamicCeilingDecayFactor = 0.08;

    private double _dynamicDomainMax;
    private IReadOnlyList<double> _values = Array.Empty<double>();
    private int _visiblePointCount = MinVisiblePointCount;
    private MetricTrendScaleMode _scaleMode = MetricTrendScaleMode.CpuPercent;
    private bool _showGrid;
    private Brush _strokeBrush = new SolidColorBrush(Colors.DodgerBlue);
    private Brush _fillBrush = new SolidColorBrush(Color.FromArgb(51, 30, 144, 255));
    private Brush _gridBrush = new SolidColorBrush(Color.FromArgb(64, 140, 148, 163));
    private double _strokeThickness = 1.6d;

    public MetricTrendChart()
    {
        InitializeComponent();
        Loaded += OnLoaded;
        PlotBorder.SizeChanged += PlotBorder_SizeChanged;
        RefreshChart();
    }

    public IReadOnlyList<double> Values
    {
        get => _values;
        set
        {
            _values = value ?? Array.Empty<double>();
            RefreshChart();
        }
    }

    public int VisiblePointCount
    {
        get => _visiblePointCount;
        set
        {
            _visiblePointCount = value;
            RefreshChart();
        }
    }

    public MetricTrendScaleMode ScaleMode
    {
        get => _scaleMode;
        set
        {
            if (_scaleMode == value)
            {
                return;
            }

            _scaleMode = value;
            _dynamicDomainMax = 0d;
            RefreshChart();
        }
    }

    public bool ShowGrid
    {
        get => _showGrid;
        set
        {
            _showGrid = value;
            RefreshChart();
        }
    }

    public Brush StrokeBrush
    {
        get => _strokeBrush;
        set
        {
            _strokeBrush = value ?? new SolidColorBrush(Colors.DodgerBlue);
            RefreshChart();
        }
    }

    public Brush FillBrush
    {
        get => _fillBrush;
        set
        {
            _fillBrush = value ?? new SolidColorBrush(Color.FromArgb(51, 30, 144, 255));
            RefreshChart();
        }
    }

    public Brush GridBrush
    {
        get => _gridBrush;
        set
        {
            _gridBrush = value ?? new SolidColorBrush(Color.FromArgb(64, 140, 148, 163));
            RefreshChart();
        }
    }

    public double StrokeThickness
    {
        get => _strokeThickness;
        set
        {
            _strokeThickness = value;
            RefreshChart();
        }
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
        if (!IsLoaded)
        {
            return;
        }

        int visibleCount = NormalizeVisiblePointCount(VisiblePointCount);
        bool showAxes = ShowGrid;
        BottomAxisPanel.Visibility = showAxes ? Visibility.Visible : Visibility.Collapsed;
        TopRightScaleLabel.Visibility = showAxes ? Visibility.Visible : Visibility.Collapsed;
        GridPath.Visibility = showAxes ? Visibility.Visible : Visibility.Collapsed;
        PlotBorder.BorderThickness = showAxes ? new Thickness(1) : new Thickness(0);

        TimeWindowLabel.Text = $"{visibleCount} seconds";
        LinePolyline.Stroke = StrokeBrush;
        AreaPolygon.Fill = FillBrush;
        GridPath.Stroke = GridBrush;
        LinePolyline.StrokeThickness = Math.Max(0.8, StrokeThickness);

        double width = PlotBorder.ActualWidth;
        double height = PlotBorder.ActualHeight;
        if (width <= 1d || height <= 1d)
        {
            return;
        }

        IReadOnlyList<double> source = Values ?? Array.Empty<double>();
        double[] window = CopyWindow(source, visibleCount);
        double maxVisible = ResolveMax(window);
        double domainMax = ResolveDomainMax(maxVisible);

        TopRightScaleLabel.Text = FormatScaleLabel(domainMax);
        GridPath.Data = showAxes ? BuildGridGeometry(width, height, verticalDivisions: 12, horizontalDivisions: 8) : null;

        IReadOnlyList<Point> linePoints = SparklineMath.BuildPointsInDomainWithFallback(window, width, height, minDomain: 0d, maxDomain: domainMax);
        IReadOnlyList<Point> fillPoints = SparklineMath.BuildFillPolygon(linePoints, width, height);

        LinePolyline.Points = SparklineMath.ToPointCollection(linePoints);
        AreaPolygon.Points = SparklineMath.ToPointCollection(fillPoints);
    }

    private double ResolveDomainMax(double maxVisible)
    {
        if (ScaleMode == MetricTrendScaleMode.CpuPercent)
        {
            _dynamicDomainMax = 100d;
            return _dynamicDomainMax;
        }

        double floor = ScaleMode == MetricTrendScaleMode.MemoryBytes
            ? MemoryFloorBytes
            : IoRateFloorBytes;

        double targetMax = Math.Max(maxVisible * DynamicCeilingPaddingRatio, floor);
        targetMax = SparklineMath.RoundUpToNice(targetMax);

        if (_dynamicDomainMax <= 0d || targetMax > _dynamicDomainMax)
        {
            _dynamicDomainMax = targetMax;
        }
        else
        {
            _dynamicDomainMax += (targetMax - _dynamicDomainMax) * DynamicCeilingDecayFactor;
            _dynamicDomainMax = Math.Max(floor, _dynamicDomainMax);
            _dynamicDomainMax = SparklineMath.RoundUpToNice(_dynamicDomainMax);
        }

        return _dynamicDomainMax;
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
        if (ScaleMode == MetricTrendScaleMode.CpuPercent)
        {
            return $"{domainMax:F0}%";
        }

        ulong clamped = domainMax <= 0d
            ? 0UL
            : (ulong)Math.Min(Math.Round(domainMax), ulong.MaxValue);
        return ScaleMode == MetricTrendScaleMode.MemoryBytes
            ? ValueFormat.FormatBytes(clamped)
            : ValueFormat.FormatRate(clamped);
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
