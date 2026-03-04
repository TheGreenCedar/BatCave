using System;
using Humanizer;
using Microsoft.UI.Xaml.Media;
using Microsoft.UI.Xaml.Data;
using Windows.UI;

namespace BatCave.Converters;

public static class ValueFormat
{
    private const ulong OneKb = 1024UL;
    private const ulong OneMb = OneKb * 1024UL;
    private const ulong OneGb = OneMb * 1024UL;
    private const double OneKbps = 1000d;
    private const double OneMbps = OneKbps * 1000d;
    private const double OneGbps = OneMbps * 1000d;

    public static string FormatBytes(ulong value)
    {
        ByteSize size = ByteSize.FromBytes(value);

        if (value >= OneGb)
        {
            return $"{size.Gigabytes:F2} GB";
        }

        if (value >= OneMb)
        {
            return $"{size.Megabytes:F1} MB";
        }

        if (value >= OneKb)
        {
            return $"{size.Kilobytes:F1} KB";
        }

        return $"{size.Bytes:F0} B";
    }

    public static string FormatRate(ulong value)
    {
        return $"{FormatBytes(value)}/s";
    }

    public static string FormatBitsRateFromBytes(ulong bytesPerSecond)
    {
        return FormatBitsRate(bytesPerSecond * 8d);
    }

    public static string FormatBitsRate(double bitsPerSecond)
    {
        if (!double.IsFinite(bitsPerSecond) || bitsPerSecond <= 0d)
        {
            return "0 bps";
        }

        if (bitsPerSecond >= OneGbps)
        {
            return $"{bitsPerSecond / OneGbps:F1} Gbps";
        }

        if (bitsPerSecond >= OneMbps)
        {
            return $"{bitsPerSecond / OneMbps:F1} Mbps";
        }

        if (bitsPerSecond >= OneKbps)
        {
            return $"{bitsPerSecond / OneKbps:F1} Kbps";
        }

        return $"{bitsPerSecond:F0} bps";
    }

    public static string FormatFrequencyGHz(double? mhz)
    {
        if (!mhz.HasValue || mhz.Value <= 0d)
        {
            return "n/a";
        }

        return $"{mhz.Value / 1000d:F2} GHz";
    }
}

public sealed partial class CpuPercentConverter : IValueConverter
{
    public object Convert(object value, Type targetType, object parameter, string language)
    {
        return value is double cpu ? $"{cpu:F2}%" : "0.00%";
    }

    public object ConvertBack(object value, Type targetType, object parameter, string language)
    {
        throw new NotSupportedException();
    }
}

public sealed partial class BytesConverter : IValueConverter
{
    public object Convert(object value, Type targetType, object parameter, string language)
    {
        return TryAsUlong(value, out ulong bytes)
            ? ValueFormat.FormatBytes(bytes)
            : "0 B";
    }

    public object ConvertBack(object value, Type targetType, object parameter, string language)
    {
        throw new NotSupportedException();
    }

    private static bool TryAsUlong(object value, out ulong result)
    {
        switch (value)
        {
            case ulong u:
                result = u;
                return true;
            case long l when l >= 0:
                result = (ulong)l;
                return true;
            case int i when i >= 0:
                result = (ulong)i;
                return true;
            case uint ui:
                result = ui;
                return true;
            default:
                result = 0;
                return false;
        }
    }
}

public sealed partial class BytesRateConverter : IValueConverter
{
    public object Convert(object value, Type targetType, object parameter, string language)
    {
        if (value is ulong rate)
        {
            return ValueFormat.FormatRate(rate);
        }

        return "0 B/s";
    }

    public object ConvertBack(object value, Type targetType, object parameter, string language)
    {
        throw new NotSupportedException();
    }
}

public sealed partial class ColorToBrushConverter : IValueConverter
{
    public object Convert(object value, Type targetType, object parameter, string language)
    {
        if (value is Color color)
        {
            return new SolidColorBrush(color);
        }

        return new SolidColorBrush(Color.FromArgb(0, 0, 0, 0));
    }

    public object ConvertBack(object value, Type targetType, object parameter, string language)
    {
        throw new NotSupportedException();
    }
}
