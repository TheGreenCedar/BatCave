using System;
using Humanizer;
using Microsoft.UI.Xaml.Data;

namespace BatCave.Converters;

public static class ValueFormat
{
    private const ulong OneKb = 1024UL;
    private const ulong OneMb = OneKb * 1024UL;
    private const ulong OneGb = OneMb * 1024UL;

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
}

public sealed class CpuPercentConverter : IValueConverter
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

public sealed class BytesConverter : IValueConverter
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

public sealed class BytesRateConverter : IValueConverter
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
