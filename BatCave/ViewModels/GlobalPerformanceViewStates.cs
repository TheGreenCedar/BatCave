using BatCave.Controls;
using CommunityToolkit.Mvvm.ComponentModel;
using Microsoft.UI;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Media;
using Windows.UI;

namespace BatCave.ViewModels;

public enum GlobalResourceKind
{
    Cpu,
    Memory,
    Disk,
    Network,
    OtherIo,
}

public enum CpuGraphMode
{
    Combined,
    LogicalProcessors,
}

public sealed class GlobalResourceRowViewState : ObservableObject
{
    private string _subtitle;
    private string _valueText;
    private string _chartIdentityKey;
    private double[] _miniTrendValues;
    private MetricTrendScaleMode _miniScaleMode;
    private Color _miniStrokeColor;
    private Color _miniFillColor;
    private double _miniDomainMax;
    private Brush? _containerBackgroundBrush = ResolveBrush("BatCavePanelAltBrush");
    private Brush? _containerBorderBrush = ResolveBrush("BatCaveBorderBrush");
    private Brush? _primaryTextBrush = ResolveBrush("BatCaveTextPrimaryBrush");
    private Brush? _secondaryTextBrush = ResolveBrush("BatCaveTextSecondaryBrush");

    public GlobalResourceRowViewState(
        string resourceId,
        GlobalResourceKind kind,
        string title,
        string subtitle,
        string valueText,
        string chartIdentityKey,
        double[] miniTrendValues,
        MetricTrendScaleMode miniScaleMode,
        Color miniStrokeColor,
        Color miniFillColor,
        double miniDomainMax)
    {
        ResourceId = resourceId;
        Kind = kind;
        Title = title;
        _subtitle = subtitle;
        _valueText = valueText;
        _chartIdentityKey = chartIdentityKey;
        _miniTrendValues = miniTrendValues;
        _miniScaleMode = miniScaleMode;
        _miniStrokeColor = miniStrokeColor;
        _miniFillColor = miniFillColor;
        _miniDomainMax = miniDomainMax;
    }

    public string ResourceId { get; }

    public GlobalResourceKind Kind { get; }

    public string Title { get; }

    public string ChartIdentityKey
    {
        get => _chartIdentityKey;
        private set => SetProperty(ref _chartIdentityKey, value);
    }

    public string Subtitle
    {
        get => _subtitle;
        private set => SetTextWithVisibility(ref _subtitle, value, nameof(Subtitle), nameof(SubtitleVisibility));
    }

    public string ValueText
    {
        get => _valueText;
        private set => SetTextWithVisibility(ref _valueText, value, nameof(ValueText), nameof(ValueVisibility));
    }

    public Visibility SubtitleVisibility => string.IsNullOrWhiteSpace(Subtitle) ? Visibility.Collapsed : Visibility.Visible;

    public Visibility ValueVisibility => string.IsNullOrWhiteSpace(ValueText) ? Visibility.Collapsed : Visibility.Visible;

    private void SetTextWithVisibility(ref string field, string value, string propertyName, string visibilityPropertyName)
    {
        if (SetProperty(ref field, value, propertyName))
        {
            OnPropertyChanged(visibilityPropertyName);
        }
    }

    public double[] MiniTrendValues
    {
        get => _miniTrendValues;
        private set => SetProperty(ref _miniTrendValues, value);
    }

    public MetricTrendScaleMode MiniScaleMode
    {
        get => _miniScaleMode;
        private set => SetProperty(ref _miniScaleMode, value);
    }

    public Color MiniStrokeColor
    {
        get => _miniStrokeColor;
        private set => SetProperty(ref _miniStrokeColor, value);
    }

    public Color MiniFillColor
    {
        get => _miniFillColor;
        private set => SetProperty(ref _miniFillColor, value);
    }

    public double MiniDomainMax
    {
        get => _miniDomainMax;
        private set => SetProperty(ref _miniDomainMax, value);
    }

    public Brush? ContainerBackgroundBrush
    {
        get => _containerBackgroundBrush;
        private set => SetProperty(ref _containerBackgroundBrush, value);
    }

    public Brush? ContainerBorderBrush
    {
        get => _containerBorderBrush;
        private set => SetProperty(ref _containerBorderBrush, value);
    }

    public Brush? PrimaryTextBrush
    {
        get => _primaryTextBrush;
        private set => SetProperty(ref _primaryTextBrush, value);
    }

    public Brush? SecondaryTextBrush
    {
        get => _secondaryTextBrush;
        private set => SetProperty(ref _secondaryTextBrush, value);
    }

    public string MiniChartAutomationName => $"{Title} mini trend chart";

    public string MiniChartAutomationHelpText =>
        $"{Title} mini chart. Passive summary view for quick scanning; use the inspector chart for detailed values.";

    public void Update(
        string subtitle,
        string valueText,
        string chartIdentityKey,
        double[] miniTrendValues,
        MetricTrendScaleMode miniScaleMode,
        Color miniStrokeColor,
        Color miniFillColor,
        double miniDomainMax)
    {
        Subtitle = subtitle;
        ValueText = valueText;
        ChartIdentityKey = chartIdentityKey;
        MiniTrendValues = miniTrendValues;
        MiniScaleMode = miniScaleMode;
        MiniStrokeColor = miniStrokeColor;
        MiniFillColor = miniFillColor;
        MiniDomainMax = miniDomainMax;
    }

    public void SetSelectionState(bool isSelected)
    {
        ContainerBackgroundBrush = ResolveBrush(isSelected ? "BatCaveSelectionBrush" : "BatCavePanelAltBrush");
        ContainerBorderBrush = ResolveBrush(isSelected ? "BatCavePrimaryBrush" : "BatCaveBorderBrush");
        PrimaryTextBrush = ResolveBrush(isSelected ? "BatCaveSelectionTextBrush" : "BatCaveTextPrimaryBrush");
        SecondaryTextBrush = ResolveBrush(isSelected ? "BatCaveSelectionTextBrush" : "BatCaveTextSecondaryBrush");
    }

    internal void RefreshMiniTrend(IReadOnlyList<double> series, int visiblePointCount)
    {
        if (CopyLatestInto(series, ref _miniTrendValues, visiblePointCount))
        {
            OnPropertyChanged(nameof(MiniTrendValues));
        }
    }

    internal void RefreshMiniTrend(IReadOnlyList<double> series, int visiblePointCount, Func<double, double> map)
    {
        if (CopyLatestInto(series, ref _miniTrendValues, visiblePointCount, map))
        {
            OnPropertyChanged(nameof(MiniTrendValues));
        }
    }

    internal void RefreshMiniTrend(IReadOnlyList<double> left, IReadOnlyList<double> right, int visiblePointCount)
    {
        if (CopyCombinedLatestInto(left, right, ref _miniTrendValues, visiblePointCount))
        {
            OnPropertyChanged(nameof(MiniTrendValues));
        }
    }

    private static bool CopyLatestInto(
        IReadOnlyList<double> source,
        ref double[] destination,
        int visiblePointCount,
        Func<double, double>? map = null)
    {
        int windowSize = Math.Max(1, visiblePointCount);
        int take = Math.Min(source.Count, windowSize);
        int sourceStart = source.Count - take;
        int destinationStart = windowSize - take;
        bool changed = EnsureTrendBufferSize(ref destination, windowSize);

        for (int index = 0; index < destinationStart; index++)
        {
            if (destination[index] == 0d)
            {
                continue;
            }

            destination[index] = 0d;
            changed = true;
        }

        for (int index = 0; index < take; index++)
        {
            double current = source[sourceStart + index];
            double next = map is null ? current : map(current);
            int targetIndex = destinationStart + index;
            if (destination[targetIndex] == next)
            {
                continue;
            }

            destination[targetIndex] = next;
            changed = true;
        }

        return changed;
    }

    private static bool CopyCombinedLatestInto(
        IReadOnlyList<double> left,
        IReadOnlyList<double> right,
        ref double[] destination,
        int visiblePointCount)
    {
        int windowSize = Math.Max(1, visiblePointCount);
        bool changed = EnsureTrendBufferSize(ref destination, windowSize);

        for (int outputIndex = 0; outputIndex < windowSize; outputIndex++)
        {
            int leftIndex = left.Count - windowSize + outputIndex;
            int rightIndex = right.Count - windowSize + outputIndex;
            double leftValue = leftIndex >= 0 && leftIndex < left.Count ? left[leftIndex] : 0d;
            double rightValue = rightIndex >= 0 && rightIndex < right.Count ? right[rightIndex] : 0d;
            double next = leftValue + rightValue;
            if (destination[outputIndex] == next)
            {
                continue;
            }

            destination[outputIndex] = next;
            changed = true;
        }

        return changed;
    }

    private static bool EnsureTrendBufferSize(ref double[] destination, int windowSize)
    {
        if (destination.Length == windowSize)
        {
            return false;
        }

        destination = new double[windowSize];
        return true;
    }

    private static Brush? ResolveBrush(string resourceKey)
    {
        try
        {
            if (Application.Current?.Resources.TryGetValue(resourceKey, out object? resource) == true
                && resource is Brush brush)
            {
                return brush;
            }
        }
        catch
        {
            // Unit tests can instantiate view state without a WinUI application host.
        }

        return null;
    }
}

public sealed class GlobalStatItemViewState(string label, string value) : ObservableObject
{
    private string _value = value;

    public string Label { get; } = label;

    public string Value
    {
        get => _value;
        private set => SetProperty(ref _value, value);
    }

    public void UpdateValue(string value) => Value = value;
}

public sealed class LogicalProcessorTrendViewState(string title, string chartIdentityKey, double[] values, double[] overlayValues) : ObservableObject
{
    private double[] _values = values;
    private double[] _overlayValues = overlayValues;

    public string Title { get; } = title;

    public string ChartIdentityKey { get; } = chartIdentityKey;

    public string LogicalChartAutomationName => $"{Title} logical CPU trend chart";

    public string LogicalChartAutomationHelpText =>
        $"{Title} passive logical processor chart showing user and kernel utilization over time.";

    public double[] Values
    {
        get => _values;
        private set => SetProperty(ref _values, value);
    }

    public double[] OverlayValues
    {
        get => _overlayValues;
        private set => SetProperty(ref _overlayValues, value);
    }

    public void UpdateValues(double[] values) => Values = values;

    internal void UpdateValues(FixedRingSeries series, FixedRingSeries overlaySeries, int visiblePointCount)
    {
        bool changed = series.CopyLatestInto(ref _values, visiblePointCount);
        if (changed)
        {
            OnPropertyChanged(nameof(Values));
        }

        if (overlaySeries.CopyLatestInto(ref _overlayValues, visiblePointCount))
        {
            OnPropertyChanged(nameof(OverlayValues));
        }
    }
}
