using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Media;
using Microsoft.UI.Xaml.Shapes;
using Windows.Foundation;

namespace BatCave.App.Controls;

public sealed partial class SparklineControl : UserControl
{
    public static readonly DependencyProperty ValuesProperty = DependencyProperty.Register(
        nameof(Values),
        typeof(double[]),
        typeof(SparklineControl),
        new PropertyMetadata(Array.Empty<double>(), OnValuesChanged));

    public SparklineControl()
    {
        InitializeComponent();
        SizeChanged += (_, _) => Render();
    }

    public double[] Values
    {
        get => (double[])GetValue(ValuesProperty);
        set => SetValue(ValuesProperty, value);
    }

    private static void OnValuesChanged(DependencyObject d, DependencyPropertyChangedEventArgs e)
    {
        ((SparklineControl)d).Render();
    }

    private void Render()
    {
        double[] values = Values;
        double width = PlotCanvas.ActualWidth > 0 ? PlotCanvas.ActualWidth : ActualWidth;
        double height = PlotCanvas.ActualHeight > 0 ? PlotCanvas.ActualHeight : ActualHeight;
        if (values.Length < 2 || width <= 0 || height <= 0)
        {
            Line.Points.Clear();
            Area.Points.Clear();
            EmptyState.Visibility = Visibility.Visible;
            return;
        }

        EmptyState.Visibility = Visibility.Collapsed;
        double max = Math.Max(1d, values.Max());
        double min = Math.Min(0d, values.Min());
        double range = Math.Max(1d, max - min);
        double xStep = width / (values.Length - 1);
        PointCollection points = new();
        for (int index = 0; index < values.Length; index++)
        {
            double x = index * xStep;
            double normalized = Math.Clamp((values[index] - min) / range, 0d, 1d);
            double y = height - normalized * height;
            points.Add(new Point(x, y));
        }

        PointCollection areaPoints = new()
        {
            new Point(0, height),
        };
        foreach (Point point in points)
        {
            areaPoints.Add(point);
        }

        areaPoints.Add(new Point(width, height));
        Area.Points = areaPoints;
        Line.Points = points;
    }
}
