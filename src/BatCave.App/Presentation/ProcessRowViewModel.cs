using BatCave.Runtime.Contracts;
using BatCave.Runtime.Presentation;
using CommunityToolkit.Mvvm.ComponentModel;
using System.Globalization;

namespace BatCave.App.Presentation;

public sealed class ProcessRowViewModel : ObservableObject
{
    private ProcessSample _sample = new();
    private string _name = string.Empty;
    private string _pidText = string.Empty;
    private string _parentPidText = string.Empty;
    private string _startTimeText = string.Empty;
    private string _cpuText = string.Empty;
    private string _memoryText = string.Empty;
    private string _privateMemoryText = string.Empty;
    private string _diskText = string.Empty;
    private string _otherIoText = string.Empty;
    private string _threadsText = string.Empty;
    private string _handlesText = string.Empty;
    private string _accessStateText = string.Empty;
    private double _attentionScore;
    private string _attentionBadgeText = string.Empty;
    private string _attentionSummaryText = string.Empty;
    private string _lastMeaningfulChangeText = string.Empty;
    private string _memoryCompactText = string.Empty;
    private string _cpuCompactText = string.Empty;
    private string _diskCompactText = string.Empty;
    private string _otherIoCompactText = string.Empty;
    private string _pidCompactText = string.Empty;

    public ProcessRowViewModel(ProcessSample sample, ProcessSample? previousSample = null, bool isNew = false)
    {
        Identity = sample.Identity();
        Apply(sample, previousSample, isNew, raiseNotifications: false);
    }

    public ProcessSample Sample
    {
        get => _sample;
        private set => SetProperty(ref _sample, value);
    }

    public ProcessIdentity Identity { get; }

    public string Name
    {
        get => _name;
        private set => SetProperty(ref _name, value);
    }

    public string PidText
    {
        get => _pidText;
        private set => SetProperty(ref _pidText, value);
    }

    public string ParentPidText
    {
        get => _parentPidText;
        private set => SetProperty(ref _parentPidText, value);
    }

    public string StartTimeText
    {
        get => _startTimeText;
        private set => SetProperty(ref _startTimeText, value);
    }

    public string CpuText
    {
        get => _cpuText;
        private set => SetProperty(ref _cpuText, value);
    }

    public string MemoryText
    {
        get => _memoryText;
        private set => SetProperty(ref _memoryText, value);
    }

    public string PrivateMemoryText
    {
        get => _privateMemoryText;
        private set => SetProperty(ref _privateMemoryText, value);
    }

    public string DiskText
    {
        get => _diskText;
        private set => SetProperty(ref _diskText, value);
    }

    public string OtherIoText
    {
        get => _otherIoText;
        private set => SetProperty(ref _otherIoText, value);
    }

    public string ThreadsText
    {
        get => _threadsText;
        private set => SetProperty(ref _threadsText, value);
    }

    public string HandlesText
    {
        get => _handlesText;
        private set => SetProperty(ref _handlesText, value);
    }

    public string AccessStateText
    {
        get => _accessStateText;
        private set => SetProperty(ref _accessStateText, value);
    }

    public double AttentionScore
    {
        get => _attentionScore;
        private set => SetProperty(ref _attentionScore, value);
    }

    public string AttentionBadgeText
    {
        get => _attentionBadgeText;
        private set => SetProperty(ref _attentionBadgeText, value);
    }

    public string AttentionSummaryText
    {
        get => _attentionSummaryText;
        private set => SetProperty(ref _attentionSummaryText, value);
    }

    public string LastMeaningfulChangeText
    {
        get => _lastMeaningfulChangeText;
        private set => SetProperty(ref _lastMeaningfulChangeText, value);
    }

    public string MemoryCompactText
    {
        get => _memoryCompactText;
        private set => SetProperty(ref _memoryCompactText, value);
    }

    public string CpuCompactText
    {
        get => _cpuCompactText;
        private set => SetProperty(ref _cpuCompactText, value);
    }

    public string DiskCompactText
    {
        get => _diskCompactText;
        private set => SetProperty(ref _diskCompactText, value);
    }

    public string OtherIoCompactText
    {
        get => _otherIoCompactText;
        private set => SetProperty(ref _otherIoCompactText, value);
    }

    public string PidCompactText
    {
        get => _pidCompactText;
        private set => SetProperty(ref _pidCompactText, value);
    }

    public void Update(ProcessSample sample, ProcessSample? previousSample = null, bool isNew = false)
    {
        if (!Identity.Equals(sample.Identity()))
        {
            throw new InvalidOperationException("A process row can only be updated with the same process identity.");
        }

        Apply(sample, previousSample, isNew, raiseNotifications: true);
    }

    public void UpdateSample(ProcessSample sample)
    {
        if (!Identity.Equals(sample.Identity()))
        {
            throw new InvalidOperationException("A process row can only be updated with the same process identity.");
        }

        _sample = sample;
    }

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

    private void Apply(ProcessSample sample, ProcessSample? previousSample, bool isNew, bool raiseNotifications)
    {
        ProcessRowDisplay display = CreateDisplay(sample, previousSample, isNew);
        if (raiseNotifications)
        {
            _sample = sample;
            Name = display.Name;
            PidText = display.PidText;
            ParentPidText = display.ParentPidText;
            StartTimeText = display.StartTimeText;
            CpuText = display.CpuText;
            MemoryText = display.MemoryText;
            PrivateMemoryText = display.PrivateMemoryText;
            DiskText = display.DiskText;
            OtherIoText = display.OtherIoText;
            ThreadsText = display.ThreadsText;
            HandlesText = display.HandlesText;
            AccessStateText = display.AccessStateText;
            AttentionScore = display.AttentionScore;
            AttentionBadgeText = display.AttentionBadgeText;
            AttentionSummaryText = display.AttentionSummaryText;
            LastMeaningfulChangeText = display.LastMeaningfulChangeText;
            MemoryCompactText = display.MemoryCompactText;
            CpuCompactText = display.CpuCompactText;
            DiskCompactText = display.DiskCompactText;
            OtherIoCompactText = display.OtherIoCompactText;
            PidCompactText = display.PidCompactText;
            return;
        }

        _sample = sample;
        _name = display.Name;
        _pidText = display.PidText;
        _parentPidText = display.ParentPidText;
        _startTimeText = display.StartTimeText;
        _cpuText = display.CpuText;
        _memoryText = display.MemoryText;
        _privateMemoryText = display.PrivateMemoryText;
        _diskText = display.DiskText;
        _otherIoText = display.OtherIoText;
        _threadsText = display.ThreadsText;
        _handlesText = display.HandlesText;
        _accessStateText = display.AccessStateText;
        _attentionScore = display.AttentionScore;
        _attentionBadgeText = display.AttentionBadgeText;
        _attentionSummaryText = display.AttentionSummaryText;
        _lastMeaningfulChangeText = display.LastMeaningfulChangeText;
        _memoryCompactText = display.MemoryCompactText;
        _cpuCompactText = display.CpuCompactText;
        _diskCompactText = display.DiskCompactText;
        _otherIoCompactText = display.OtherIoCompactText;
        _pidCompactText = display.PidCompactText;
    }

    private static ProcessRowDisplay CreateDisplay(ProcessSample sample, ProcessSample? previousSample, bool isNew)
    {
        string name = string.IsNullOrWhiteSpace(sample.Name) ? $"PID {sample.Pid}" : sample.Name;
        string pidText = sample.Pid.ToString(CultureInfo.InvariantCulture);
        string parentPidText = sample.ParentPid == 0
            ? "n/a"
            : sample.ParentPid.ToString(CultureInfo.InvariantCulture);
        string startTimeText = sample.StartTimeMs == 0
            ? "n/a"
            : DateTimeOffset.FromUnixTimeMilliseconds((long)Math.Min(sample.StartTimeMs, (ulong)long.MaxValue))
                .LocalDateTime
                .ToString("g", CultureInfo.CurrentCulture);
        string cpuText = sample.CpuPct.ToString("0.0", CultureInfo.InvariantCulture) + "%";
        string memoryText = FormatBytes(sample.MemoryBytes);
        string privateMemoryText = FormatBytes(sample.PrivateBytes);
        string diskText = FormatRate(sample.DiskBps);
        string otherIoText = FormatRate(sample.OtherIoBps);
        string threadsText = sample.Threads.ToString(CultureInfo.InvariantCulture);
        string handlesText = sample.Handles.ToString(CultureInfo.InvariantCulture);
        string accessStateText = sample.AccessState.ToString();
        double attentionScore = ProcessAttention.Score(sample);
        string attentionBadgeText = ProcessAttention.Label(sample, isNew);
        return new ProcessRowDisplay(
            name,
            pidText,
            parentPidText,
            startTimeText,
            cpuText,
            memoryText,
            privateMemoryText,
            diskText,
            otherIoText,
            threadsText,
            handlesText,
            accessStateText,
            attentionScore,
            attentionBadgeText,
            $"{attentionBadgeText} ({attentionScore:0})",
            ProcessAttention.DescribeChange(previousSample, sample, isNew),
            $"Mem {memoryText}",
            $"CPU {cpuText}",
            $"Disk {diskText}",
            $"I/O {otherIoText}",
            $"PID {pidText}");
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

    private readonly record struct ProcessRowDisplay(
        string Name,
        string PidText,
        string ParentPidText,
        string StartTimeText,
        string CpuText,
        string MemoryText,
        string PrivateMemoryText,
        string DiskText,
        string OtherIoText,
        string ThreadsText,
        string HandlesText,
        string AccessStateText,
        double AttentionScore,
        string AttentionBadgeText,
        string AttentionSummaryText,
        string LastMeaningfulChangeText,
        string MemoryCompactText,
        string CpuCompactText,
        string DiskCompactText,
        string OtherIoCompactText,
        string PidCompactText);
}
