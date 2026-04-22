using BatCave.Runtime.Contracts;
using System.Globalization;

namespace BatCave.App.Presentation;

public sealed record ProcessRowViewModel
{
    public ProcessRowViewModel(ProcessSample sample)
    {
        Sample = sample;
        Identity = sample.Identity();
        Name = string.IsNullOrWhiteSpace(sample.Name) ? $"PID {sample.Pid}" : sample.Name;
        PidText = sample.Pid.ToString(CultureInfo.InvariantCulture);
        CpuText = sample.CpuPct.ToString("0.0", CultureInfo.InvariantCulture) + "%";
        MemoryText = FormatBytes(sample.MemoryBytes);
        DiskText = FormatRate(sample.DiskBps);
        OtherIoText = FormatRate(sample.OtherIoBps);
        ThreadsText = sample.Threads.ToString(CultureInfo.InvariantCulture);
        HandlesText = sample.Handles.ToString(CultureInfo.InvariantCulture);
        MemoryCompactText = $"Mem {MemoryText}";
        CpuCompactText = $"CPU {CpuText}";
        DiskCompactText = $"Disk {DiskText}";
        OtherIoCompactText = $"I/O {OtherIoText}";
        PidCompactText = $"PID {PidText}";
    }

    public ProcessSample Sample { get; }
    public ProcessIdentity Identity { get; }
    public string Name { get; }
    public string PidText { get; }
    public string CpuText { get; }
    public string MemoryText { get; }
    public string DiskText { get; }
    public string OtherIoText { get; }
    public string ThreadsText { get; }
    public string HandlesText { get; }
    public string MemoryCompactText { get; }
    public string CpuCompactText { get; }
    public string DiskCompactText { get; }
    public string OtherIoCompactText { get; }
    public string PidCompactText { get; }

    public bool HasSameDisplayState(ProcessSample sample)
    {
        return Identity.Equals(sample.Identity())
               && string.Equals(Sample.Name, sample.Name, StringComparison.Ordinal)
               && Sample.CpuPct.Equals(sample.CpuPct)
               && Sample.MemoryBytes == sample.MemoryBytes
               && Sample.DiskBps == sample.DiskBps
               && Sample.OtherIoBps == sample.OtherIoBps
               && Sample.Threads == sample.Threads
               && Sample.Handles == sample.Handles;
    }

    public static string FormatBytes(ulong bytes)
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

    public static string FormatRate(ulong bytesPerSecond) => FormatBytes(bytesPerSecond) + "/s";
}
