using BatCave.Runtime.Contracts;
using BatCave.Runtime.Presentation;
using System.Globalization;

namespace BatCave.App.Presentation;

public sealed record ProcessRowViewModel
{
    public ProcessRowViewModel(ProcessSample sample, ProcessSample? previousSample = null, bool isNew = false)
    {
        Sample = sample;
        Identity = sample.Identity();
        Name = string.IsNullOrWhiteSpace(sample.Name) ? $"PID {sample.Pid}" : sample.Name;
        PidText = sample.Pid.ToString(CultureInfo.InvariantCulture);
        ParentPidText = sample.ParentPid == 0
            ? "n/a"
            : sample.ParentPid.ToString(CultureInfo.InvariantCulture);
        StartTimeText = sample.StartTimeMs == 0
            ? "n/a"
            : DateTimeOffset.FromUnixTimeMilliseconds((long)Math.Min(sample.StartTimeMs, (ulong)long.MaxValue))
                .LocalDateTime
                .ToString("g", CultureInfo.CurrentCulture);
        CpuText = sample.CpuPct.ToString("0.0", CultureInfo.InvariantCulture) + "%";
        MemoryText = FormatBytes(sample.MemoryBytes);
        PrivateMemoryText = FormatBytes(sample.PrivateBytes);
        DiskText = FormatRate(sample.DiskBps);
        OtherIoText = FormatRate(sample.OtherIoBps);
        ThreadsText = sample.Threads.ToString(CultureInfo.InvariantCulture);
        HandlesText = sample.Handles.ToString(CultureInfo.InvariantCulture);
        AccessStateText = sample.AccessState.ToString();
        AttentionScore = ProcessAttention.Score(sample);
        AttentionBadgeText = ProcessAttention.Label(sample, isNew);
        AttentionSummaryText = $"{AttentionBadgeText} ({AttentionScore:0})";
        LastMeaningfulChangeText = ProcessAttention.DescribeChange(previousSample, sample, isNew);
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
    public string ParentPidText { get; }
    public string StartTimeText { get; }
    public string CpuText { get; }
    public string MemoryText { get; }
    public string PrivateMemoryText { get; }
    public string DiskText { get; }
    public string OtherIoText { get; }
    public string ThreadsText { get; }
    public string HandlesText { get; }
    public string AccessStateText { get; }
    public double AttentionScore { get; }
    public string AttentionBadgeText { get; }
    public string AttentionSummaryText { get; }
    public string LastMeaningfulChangeText { get; }
    public string MemoryCompactText { get; }
    public string CpuCompactText { get; }
    public string DiskCompactText { get; }
    public string OtherIoCompactText { get; }
    public string PidCompactText { get; }

    public bool HasSameDisplayState(ProcessSample sample)
    {
        return Identity.Equals(sample.Identity())
               && Sample.ParentPid == sample.ParentPid
               && string.Equals(Sample.Name, sample.Name, StringComparison.Ordinal)
               && Sample.CpuPct.Equals(sample.CpuPct)
               && Sample.MemoryBytes == sample.MemoryBytes
               && Sample.PrivateBytes == sample.PrivateBytes
               && Sample.DiskBps == sample.DiskBps
               && Sample.OtherIoBps == sample.OtherIoBps
               && Sample.Threads == sample.Threads
               && Sample.Handles == sample.Handles
               && Sample.AccessState == sample.AccessState;
    }

    public string ToClipboardText()
    {
        return string.Join(Environment.NewLine, [
            $"Name: {Name}",
            $"PID: {PidText}",
            $"Parent PID: {ParentPidText}",
            $"Started: {StartTimeText}",
            $"Access: {AccessStateText}",
            $"CPU: {CpuText}",
            $"Memory: {MemoryText}",
            $"Private bytes: {PrivateMemoryText}",
            $"Disk: {DiskText}",
            $"Other I/O: {OtherIoText}",
            $"Threads: {ThreadsText}",
            $"Handles: {HandlesText}",
            $"Attention: {AttentionSummaryText}",
            $"Last meaningful change: {LastMeaningfulChangeText}",
        ]);
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
