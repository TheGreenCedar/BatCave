using System;
using Humanizer;
using Microsoft.UI.Xaml.Data;

namespace BatCave.Converters;

public static class ValueFormat
{
    public static string FormatBytes(ulong value)
    {
        ByteSize size = ByteSize.FromBytes(value);

        if (value >= 1024UL * 1024UL * 1024UL)
        {
            return $"{size.Gigabytes:F2} GB";
        }

        if (value >= 1024UL * 1024UL)
        {
            return $"{size.Megabytes:F1} MB";
        }

        if (value >= 1024UL)
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
        if (value is double cpu)
        {
            return $"{cpu:F2}%";
        }

        return "0.00%";
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
        (bool Success, ulong Value) parsed = value switch
        {
            ulong u => (true, u),
            long l when l >= 0 => (true, (ulong)l),
            int i when i >= 0 => (true, (ulong)i),
            uint ui => (true, ui),
            _ => (false, 0),
        };

        result = parsed.Value;
        return parsed.Success;
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
