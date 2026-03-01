using System;
using BatCave.Core.Domain;
using Microsoft.UI.Xaml.Data;

namespace BatCave.Converters;

public static class ValueFormat
{
    public static string FormatBytes(ulong value)
    {
        const double kb = 1024d;
        const double mb = kb * 1024d;
        const double gb = mb * 1024d;

        if (value >= gb)
        {
            return $"{value / gb:F2} GB";
        }

        if (value >= mb)
        {
            return $"{value / mb:F1} MB";
        }

        if (value >= kb)
        {
            return $"{value / kb:F1} KB";
        }

        return $"{value} B";
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
        if (TryAsUlong(value, out ulong bytes))
        {
            return ValueFormat.FormatBytes(bytes);
        }

        return "0 B";
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

public sealed class AccessStateConverter : IValueConverter
{
    public object Convert(object value, Type targetType, object parameter, string language)
    {
        if (value is not AccessState state)
        {
            return "unknown";
        }

        return state switch
        {
            AccessState.Full => "full",
            AccessState.Partial => "partial",
            AccessState.Denied => "denied",
            _ => "unknown",
        };
    }

    public object ConvertBack(object value, Type targetType, object parameter, string language)
    {
        throw new NotSupportedException();
    }
}
