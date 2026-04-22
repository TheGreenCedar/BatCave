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
        double width = ActualWidth;
        double height = ActualHeight;
        if (values.Length == 0 || width <= 0 || height <= 0)
        {
            Line.Points.Clear();
            return;
        }

        double max = Math.Max(1d, values.Max());
        double xStep = values.Length <= 1 ? width : width / (values.Length - 1);
        PointCollection points = new();
        for (int index = 0; index < values.Length; index++)
        {
            double x = index * xStep;
            double y = height - Math.Clamp(values[index] / max, 0d, 1d) * height;
            points.Add(new Point(x, y));
        }

        Line.Points = points;
    }
}
