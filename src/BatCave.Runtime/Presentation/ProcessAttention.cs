using BatCave.Runtime.Contracts;
using System.Globalization;

namespace BatCave.Runtime.Presentation;

public static class ProcessAttention
{
    public const double CpuSpikeThresholdPct = 15d;
    public const ulong MemoryHeavyThresholdBytes = 512UL * 1024UL * 1024UL;
    public const ulong ActiveIoThresholdBytesPerSecond = 1024UL * 1024UL;

    public static double Score(ProcessSample sample)
    {
        double score = sample.CpuPct * 3d;
        score += Math.Min(sample.MemoryBytes / (128d * 1024d * 1024d), 20d);
        score += Math.Min((sample.DiskBps + sample.OtherIoBps) / (512d * 1024d), 20d);
        if (sample.AccessState != AccessState.Full)
        {
            score += 12d;
        }

        return score;
    }

    public static string Label(ProcessSample sample, bool isNew)
    {
        if (isNew)
        {
            return "New";
        }

        if (sample.AccessState != AccessState.Full)
        {
            return "Limited access";
        }

        if (sample.CpuPct >= CpuSpikeThresholdPct)
        {
            return "CPU spike";
        }

        if (sample.MemoryBytes >= MemoryHeavyThresholdBytes)
        {
            return "Memory heavy";
        }

        if (sample.DiskBps + sample.OtherIoBps >= ActiveIoThresholdBytesPerSecond)
        {
            return "I/O active";
        }

        return "Normal";
    }

    public static string DescribeChange(ProcessSample? previous, ProcessSample current, bool isNew)
    {
        if (isNew || previous is null)
        {
            return "New process in this session.";
        }

        if (!previous.CpuPct.Equals(current.CpuPct))
        {
            return $"CPU {previous.CpuPct.ToString("0.0", CultureInfo.InvariantCulture)}% -> {current.CpuPct.ToString("0.0", CultureInfo.InvariantCulture)}%.";
        }

        if (previous.MemoryBytes != current.MemoryBytes)
        {
            return $"Memory {FormatBytes(previous.MemoryBytes)} -> {FormatBytes(current.MemoryBytes)}.";
        }

        if (previous.DiskBps != current.DiskBps || previous.OtherIoBps != current.OtherIoBps)
        {
            return $"I/O {FormatRate(previous.DiskBps + previous.OtherIoBps)} -> {FormatRate(current.DiskBps + current.OtherIoBps)}.";
        }

        if (previous.AccessState != current.AccessState)
        {
            return $"Access {previous.AccessState} -> {current.AccessState}.";
        }

        return "No display-impacting change.";
    }

    private static string FormatBytes(ulong bytes)
    {
        string[] units = ["B", "KB", "MB", "GB", "TB"];
        double value = bytes;
        int unit = 0;
        while (value >= 1024d && unit < units.Length - 1)
        {
            value /= 1024d;
            unit++;
        }

        return unit == 0
            ? $"{value:0} {units[unit]}"
            : $"{value:0.0} {units[unit]}";
    }

    private static string FormatRate(ulong bytesPerSecond) => FormatBytes(bytesPerSecond) + "/s";
}
