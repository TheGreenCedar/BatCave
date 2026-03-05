using BatCave.Converters;
using BatCave.Core.Domain;
using CommunityToolkit.Mvvm.Input;
using System;
using System.Collections.Generic;
using System.Threading.Tasks;

namespace BatCave.ViewModels;

public partial class MonitoringShellViewModel
{
    private static readonly TimeSpan GlobalMetricsSampleSoftWait = TimeSpan.FromMilliseconds(35);
    private Task<SystemGlobalMetricsSample>? _globalMetricsSampleTask;

    private void EnsureGlobalMetricsSamplingStarted()
    {
        if (_globalMetricsSampleTask is null)
        {
            _globalMetricsSampleTask = Task.Run(() => _systemGlobalMetricsSampler.Sample());
        }
    }

    [RelayCommand]
    private void MetricFocusSelected(string? focusTag)
    {
        if (!Enum.TryParse(focusTag, out DetailMetricFocus focus))
        {
            return;
        }

        MetricFocus = focus;
    }

    [RelayCommand]
    private void MetricTrendWindowSelected(string? windowSeconds)
    {
        if (!int.TryParse(windowSeconds, out int requested))
        {
            return;
        }

        int normalized = NormalizeMetricTrendWindowSeconds(requested);
        if (normalized == MetricTrendWindowSeconds)
        {
            // Keep one-way bound toggle visuals consistent when the selected window is clicked again.
            OnPropertyChanged(nameof(IsTrendWindow60Selected));
            OnPropertyChanged(nameof(IsTrendWindow120Selected));
            return;
        }

        MetricTrendWindowSeconds = normalized;
        _runtime.SetMetricTrendWindowSeconds(normalized);
    }

    private void RaiseMetricFocusProperties()
    {
        RaiseProperties(
            nameof(IsCpuMetricFocused),
            nameof(IsMemoryMetricFocused),
            nameof(IsIoReadMetricFocused),
            nameof(IsIoWriteMetricFocused),
            nameof(IsOtherIoMetricFocused));
    }

    private void RaiseMetadataProperties()
    {
        RaiseProperties(
            nameof(MetadataStatus),
            nameof(MetadataParentPid),
            nameof(MetadataCommandLine),
            nameof(MetadataExecutablePath));
    }

    private void RefreshDetailMetrics()
    {
        ProcessSample detailSample = SelectedRow ?? _globalSummaryRow;
        MetricHistoryBuffer history = GetDetailHistory(detailSample);

        UpdateMetricChipValues(detailSample);

        bool cpuTrendChanged = ApplyMetricTrendValues(history.Cpu, ref _cpuMetricTrendValues, nameof(CpuMetricTrendValues));
        bool memoryTrendChanged = ApplyMetricTrendValues(history.Memory, ref _memoryMetricTrendValues, nameof(MemoryMetricTrendValues));
        bool ioReadTrendChanged = ApplyMetricTrendValues(history.IoRead, ref _ioReadMetricTrendValues, nameof(IoReadMetricTrendValues));
        bool ioWriteTrendChanged = ApplyMetricTrendValues(history.IoWrite, ref _ioWriteMetricTrendValues, nameof(IoWriteMetricTrendValues));
        bool otherIoTrendChanged = ApplyMetricTrendValues(history.OtherIo, ref _otherIoMetricTrendValues, nameof(OtherIoMetricTrendValues));

        (string expandedTitle, string expandedValue, double[] expandedTrendValues, bool expandedSeriesChanged) = ResolveExpandedMetric(
            detailSample,
            cpuTrendChanged,
            memoryTrendChanged,
            ioReadTrendChanged,
            ioWriteTrendChanged,
            otherIoTrendChanged);

        ExpandedMetricTitle = expandedTitle;
        ExpandedMetricValue = expandedValue;

        if (!ReferenceEquals(_expandedMetricTrendValues, expandedTrendValues))
        {
            ExpandedMetricTrendValues = expandedTrendValues;
        }
        else if (expandedSeriesChanged)
        {
            OnPropertyChanged(nameof(ExpandedMetricTrendValues));
        }
    }

    private MetricHistoryBuffer GetDetailHistory(ProcessSample detailSample)
    {
        if (SelectedRow is null)
        {
            return _globalHistory;
        }

        if (_metricHistory.TryGetValue(detailSample.Identity(), out MetricHistoryBuffer? history))
        {
            return history;
        }

        MetricHistoryBuffer fallback = new(HistoryLimit);
        fallback.Append(detailSample);
        return fallback;
    }

    private void ResetSummaryFromRows(IEnumerable<ProcessSample> rows)
    {
        ResetSummaryTotals();

        foreach (ProcessSample row in rows)
        {
            ApplySummaryDelta(row, 1d);
            _summarySeq = Math.Max(_summarySeq, row.Seq);
            _summaryTsMs = Math.Max(_summaryTsMs, row.TsMs);
        }

        ClampSummary();
    }

    private void ApplySummaryDelta(ProcessSample sample, double multiplier)
    {
        _summaryCpuPct += sample.CpuPct * multiplier;
        _summaryRssBytes += sample.RssBytes * multiplier;
        _summaryPrivateBytes += sample.PrivateBytes * multiplier;
        _summaryIoReadBps += sample.IoReadBps * multiplier;
        _summaryIoWriteBps += sample.IoWriteBps * multiplier;
        _summaryOtherIoBps += sample.OtherIoBps * multiplier;
        _summaryThreads += sample.Threads * multiplier;
        _summaryHandles += sample.Handles * multiplier;
    }

    private void ClampSummary()
    {
        _summaryCpuPct = ClampNonNegative(_summaryCpuPct);
        _summaryRssBytes = ClampNonNegative(_summaryRssBytes);
        _summaryPrivateBytes = ClampNonNegative(_summaryPrivateBytes);
        _summaryIoReadBps = ClampNonNegative(_summaryIoReadBps);
        _summaryIoWriteBps = ClampNonNegative(_summaryIoWriteBps);
        _summaryOtherIoBps = ClampNonNegative(_summaryOtherIoBps);
        _summaryThreads = ClampNonNegative(_summaryThreads);
        _summaryHandles = ClampNonNegative(_summaryHandles);
    }

    private void UpdateGlobalSummaryHistory()
    {
        _globalSummaryRow = CreateProjectedGlobalSample();

        _globalHistory.Append(_globalSummaryRow);
        if (SelectedRow is null)
        {
            OnPropertyChanged(nameof(DetailTitle));
        }
    }

    private ProcessSample CreateProjectedGlobalSample()
    {
        ProcessSample previous = _globalSummaryRow;
        SystemGlobalMetricsSample sampled = SampleGlobalMetricsForUiFrame();

        SetGlobalAvailability(
            cpu: sampled.CpuPct.HasValue,
            memory: sampled.MemoryUsedBytes.HasValue,
            ioRead: sampled.DiskReadBps.HasValue,
            ioWrite: sampled.DiskWriteBps.HasValue,
            otherIo: sampled.OtherIoBps.HasValue);
        RefreshGlobalPerformanceState(sampled);

        ulong projectedTsMs = sampled.TsMs > 0 ? sampled.TsMs : _summaryTsMs;
        double projectedCpuPct = ResolveGlobalMetricForTrend(sampled.CpuPct, previous.CpuPct);
        ulong projectedMemory = ResolveGlobalMetricForTrend(sampled.MemoryUsedBytes, previous.RssBytes);
        ulong projectedIoRead = ResolveGlobalMetricForTrend(sampled.DiskReadBps, previous.IoReadBps);
        ulong projectedIoWrite = ResolveGlobalMetricForTrend(sampled.DiskWriteBps, previous.IoWriteBps);
        ulong projectedOtherIo = ResolveGlobalMetricForTrend(sampled.OtherIoBps, previous.OtherIoBps);

        return CreateGlobalSummarySample(
            seq: _summarySeq,
            tsMs: projectedTsMs,
            cpuPct: projectedCpuPct,
            rssBytes: projectedMemory,
            privateBytes: _summaryPrivateBytes,
            ioReadBps: projectedIoRead,
            ioWriteBps: projectedIoWrite,
            otherIoBps: projectedOtherIo,
            threads: _summaryThreads,
            handles: _summaryHandles);
    }

    private SystemGlobalMetricsSample SampleGlobalMetricsForUiFrame()
    {
        EnsureGlobalMetricsSamplingStarted();
        return _globalMetricsSampleTask is { IsCompleted: true }
            ? ConsumeGlobalMetricsSampleAndQueueNext()
            : _latestGlobalMetricsSample;
    }

    private SystemGlobalMetricsSample ConsumeGlobalMetricsSampleAndQueueNext()
    {
        SystemGlobalMetricsSample sampled = _latestGlobalMetricsSample;
        if (_globalMetricsSampleTask is null)
        {
            return sampled;
        }

        if (_globalMetricsSampleTask.IsCompletedSuccessfully)
        {
            sampled = _globalMetricsSampleTask.Result;
        }
        else if (_globalMetricsSampleTask.IsFaulted)
        {
            _ = _globalMetricsSampleTask.Exception;
        }

        _globalMetricsSampleTask = Task.Run(() => _systemGlobalMetricsSampler.Sample());
        return sampled;
    }

    private static ProcessSample CreateEmptyGlobalSummary()
    {
        return CreateGlobalSummarySample(
            seq: 0,
            tsMs: UnixNowMs(),
            cpuPct: 0,
            rssBytes: 0,
            privateBytes: 0,
            ioReadBps: 0,
            ioWriteBps: 0,
            otherIoBps: 0,
            threads: 0,
            handles: 0);
    }

    private static ProcessSample CreateGlobalSummarySample(
        ulong seq,
        ulong tsMs,
        double cpuPct,
        double rssBytes,
        double privateBytes,
        double ioReadBps,
        double ioWriteBps,
        double otherIoBps,
        double threads,
        double handles)
    {
        return new ProcessSample
        {
            Seq = seq,
            TsMs = tsMs,
            Pid = 0,
            ParentPid = 0,
            StartTimeMs = 0,
            Name = "Global System Values",
            CpuPct = cpuPct,
            RssBytes = ClampToUlong(rssBytes),
            PrivateBytes = ClampToUlong(privateBytes),
            IoReadBps = ClampToUlong(ioReadBps),
            IoWriteBps = ClampToUlong(ioWriteBps),
            OtherIoBps = ClampToUlong(otherIoBps),
            Threads = ClampToUInt(threads),
            Handles = ClampToUInt(handles),
            AccessState = AccessState.Full,
        };
    }

    private static ulong ClampToUlong(double value)
    {
        return (ulong)ClampRounded(value, ulong.MaxValue);
    }

    private static uint ClampToUInt(double value)
    {
        return (uint)ClampRounded(value, uint.MaxValue);
    }

    private static double ClampRounded(double value, double maxValue)
    {
        if (!double.IsFinite(value) || value <= 0)
        {
            return 0;
        }

        if (value >= maxValue)
        {
            return maxValue;
        }

        return Math.Round(value);
    }

    private static ulong UnixNowMs()
    {
        long now = DateTimeOffset.UtcNow.ToUnixTimeMilliseconds();
        return now <= 0 ? 0UL : (ulong)now;
    }

    private void SetGlobalAvailability(bool cpu, bool memory, bool ioRead, bool ioWrite, bool otherIo)
    {
        _isGlobalCpuAvailable = cpu;
        _isGlobalMemoryAvailable = memory;
        _isGlobalIoReadAvailable = ioRead;
        _isGlobalIoWriteAvailable = ioWrite;
        _isGlobalOtherIoAvailable = otherIo;
    }

    private static double ResolveGlobalMetricForTrend(double? currentValue, double previousValue)
    {
        return currentValue ?? previousValue;
    }

    private static ulong ResolveGlobalMetricForTrend(ulong? currentValue, ulong previousValue)
    {
        return currentValue ?? previousValue;
    }

    private static string FormatMetricWhenAvailable(bool isAvailable, string value)
    {
        return isAvailable ? value : "n/a";
    }

    private string FormatGlobalMetricValue(bool isAvailable, Func<string> formatValue)
    {
        return SelectedRow is null && !isAvailable
            ? "n/a"
            : formatValue();
    }

    private void RaiseProperties(params string[] propertyNames)
    {
        foreach (string propertyName in propertyNames)
        {
            OnPropertyChanged(propertyName);
        }
    }

    private static double ClampNonNegative(double value)
    {
        return Math.Max(0d, value);
    }

    private void UpdateMetricChipValues(ProcessSample detailSample)
    {
        if (SelectedRow is not null)
        {
            CpuMetricChipValue = $"{detailSample.CpuPct:F2}%";
            MemoryMetricChipValue = ValueFormat.FormatBytes(detailSample.RssBytes);
            IoReadMetricChipValue = ValueFormat.FormatRate(detailSample.IoReadBps);
            IoWriteMetricChipValue = ValueFormat.FormatRate(detailSample.IoWriteBps);
            OtherIoMetricChipValue = ValueFormat.FormatRate(detailSample.OtherIoBps);
            return;
        }

        CpuMetricChipValue = FormatMetricWhenAvailable(_isGlobalCpuAvailable, $"{detailSample.CpuPct:F2}%");
        MemoryMetricChipValue = FormatMetricWhenAvailable(_isGlobalMemoryAvailable, ValueFormat.FormatBytes(detailSample.RssBytes));
        IoReadMetricChipValue = FormatMetricWhenAvailable(_isGlobalIoReadAvailable, ValueFormat.FormatRate(detailSample.IoReadBps));
        IoWriteMetricChipValue = FormatMetricWhenAvailable(_isGlobalIoWriteAvailable, ValueFormat.FormatRate(detailSample.IoWriteBps));
        OtherIoMetricChipValue = FormatMetricWhenAvailable(_isGlobalOtherIoAvailable, ValueFormat.FormatRate(detailSample.OtherIoBps));
    }

    private bool ApplyMetricTrendValues(IReadOnlyList<double> source, ref double[] target, string propertyName)
    {
        int visiblePointCount = MetricTrendWindowSeconds;
        int take = Math.Min(source.Count, visiblePointCount);
        int sourceStartIndex = source.Count - take;
        int leadingZeroCount = visiblePointCount - take;
        bool changed = false;
        if (target.Length != visiblePointCount)
        {
            target = new double[visiblePointCount];
            changed = true;
        }

        for (int index = 0; index < leadingZeroCount; index++)
        {
            if (target[index] == 0d)
            {
                continue;
            }

            target[index] = 0d;
            changed = true;
        }

        for (int index = 0; index < take; index++)
        {
            double next = source[sourceStartIndex + index];
            int targetIndex = leadingZeroCount + index;
            if (target[targetIndex] == next)
            {
                continue;
            }

            target[targetIndex] = next;
            changed = true;
        }

        if (changed)
        {
            OnPropertyChanged(propertyName);
        }

        return changed;
    }

    private (string Title, string Value, double[] TrendValues, bool SeriesChanged) ResolveExpandedMetric(
        ProcessSample detailSample,
        bool cpuTrendChanged,
        bool memoryTrendChanged,
        bool ioReadTrendChanged,
        bool ioWriteTrendChanged,
        bool otherIoTrendChanged)
    {
        return MetricFocus switch
        {
            DetailMetricFocus.Memory => (
                "Memory Trend",
                FormatGlobalMetricValue(_isGlobalMemoryAvailable, () => $"{ValueFormat.FormatBytes(detailSample.RssBytes)} RSS"),
                _memoryMetricTrendValues,
                memoryTrendChanged),
            DetailMetricFocus.IoRead => (
                "Disk Read Trend",
                FormatGlobalMetricValue(_isGlobalIoReadAvailable, () => $"{ValueFormat.FormatRate(detailSample.IoReadBps)} read"),
                _ioReadMetricTrendValues,
                ioReadTrendChanged),
            DetailMetricFocus.IoWrite => (
                "Disk Write Trend",
                FormatGlobalMetricValue(_isGlobalIoWriteAvailable, () => $"{ValueFormat.FormatRate(detailSample.IoWriteBps)} write"),
                _ioWriteMetricTrendValues,
                ioWriteTrendChanged),
            DetailMetricFocus.OtherIo => (
                "Other I/O Trend",
                FormatGlobalMetricValue(_isGlobalOtherIoAvailable, () => $"{ValueFormat.FormatRate(detailSample.OtherIoBps)} net"),
                _otherIoMetricTrendValues,
                otherIoTrendChanged),
            _ => (
                "CPU Trend",
                FormatGlobalMetricValue(_isGlobalCpuAvailable, () => $"{detailSample.CpuPct:F1}% CPU"),
                _cpuMetricTrendValues,
                cpuTrendChanged),
        };
    }

    private void ResetSummaryTotals()
    {
        _summarySeq = 0;
        _summaryTsMs = UnixNowMs();
        _summaryCpuPct = 0;
        _summaryRssBytes = 0;
        _summaryPrivateBytes = 0;
        _summaryIoReadBps = 0;
        _summaryIoWriteBps = 0;
        _summaryOtherIoBps = 0;
        _summaryThreads = 0;
        _summaryHandles = 0;
    }
}


