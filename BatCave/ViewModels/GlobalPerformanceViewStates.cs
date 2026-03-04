using BatCave.Controls;
using CommunityToolkit.Mvvm.ComponentModel;
using Windows.UI;

namespace BatCave.ViewModels;

public enum GlobalResourceKind
{
    Cpu,
    Memory,
    Disk,
    Network,
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
    private double[] _miniTrendValues;
    private MetricTrendScaleMode _miniScaleMode;
    private Color _miniStrokeColor;
    private Color _miniFillColor;
    private double _miniDomainMax;

    public GlobalResourceRowViewState(
        string resourceId,
        GlobalResourceKind kind,
        string title,
        string subtitle,
        string valueText,
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
        _miniTrendValues = miniTrendValues;
        _miniScaleMode = miniScaleMode;
        _miniStrokeColor = miniStrokeColor;
        _miniFillColor = miniFillColor;
        _miniDomainMax = miniDomainMax;
    }

    public string ResourceId { get; }

    public GlobalResourceKind Kind { get; }

    public string Title { get; }

    public string Subtitle
    {
        get => _subtitle;
        private set => SetProperty(ref _subtitle, value);
    }

    public string ValueText
    {
        get => _valueText;
        private set => SetProperty(ref _valueText, value);
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

    public void Update(
        string subtitle,
        string valueText,
        double[] miniTrendValues,
        MetricTrendScaleMode miniScaleMode,
        Color miniStrokeColor,
        Color miniFillColor,
        double miniDomainMax)
    {
        Subtitle = subtitle;
        ValueText = valueText;
        MiniTrendValues = miniTrendValues;
        MiniScaleMode = miniScaleMode;
        MiniStrokeColor = miniStrokeColor;
        MiniFillColor = miniFillColor;
        MiniDomainMax = miniDomainMax;
    }
}

public sealed class GlobalStatItemViewState : ObservableObject
{
    private string _value;

    public GlobalStatItemViewState(string label, string value)
    {
        Label = label;
        _value = value;
    }

    public string Label { get; }

    public string Value
    {
        get => _value;
        private set => SetProperty(ref _value, value);
    }

    public void UpdateValue(string value)
    {
        Value = value;
    }
}

public sealed class LogicalProcessorTrendViewState : ObservableObject
{
    private double[] _values;

    public LogicalProcessorTrendViewState(string title, double[] values)
    {
        Title = title;
        _values = values;
    }

    public string Title { get; }

    public double[] Values
    {
        get => _values;
        private set => SetProperty(ref _values, value);
    }

    public void UpdateValues(double[] values)
    {
        Values = values;
    }
}
