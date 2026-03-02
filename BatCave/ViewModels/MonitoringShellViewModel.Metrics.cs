using System;
using System.Collections.Generic;
using System.Linq;
using BatCave.Converters;
using BatCave.Core.Domain;
using CommunityToolkit.Mvvm.Input;

namespace BatCave.ViewModels;

public partial class MonitoringShellViewModel
{
    [RelayCommand]
    private void MetricFocusSelected(string? focusTag)
    {
        if (!Enum.TryParse(focusTag, out DetailMetricFocus focus))
        {
            return;
        }

        MetricFocus = focus;
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
        MetricTrends trends = BuildMetricTrends(history);

        CpuMetricTrendValues = trends.Cpu;
        MemoryMetricTrendValues = trends.Memory;
        IoReadMetricTrendValues = trends.IoRead;
        IoWriteMetricTrendValues = trends.IoWrite;
        OtherIoMetricTrendValues = trends.OtherIo;

        (ExpandedMetricTitle, ExpandedMetricValue, ExpandedMetricTrendValues) =
            MetricFocus switch
            {
                DetailMetricFocus.Memory => ("Memory Trend", $"{ValueFormat.FormatBytes(detailSample.RssBytes)} RSS", trends.Memory),
                DetailMetricFocus.IoRead => ("Disk Read Trend", $"{ValueFormat.FormatRate(detailSample.IoReadBps)} read", trends.IoRead),
                DetailMetricFocus.IoWrite => ("Disk Write Trend", $"{ValueFormat.FormatRate(detailSample.IoWriteBps)} write", trends.IoWrite),
                DetailMetricFocus.OtherIo => ("Other I/O Trend", $"{ValueFormat.FormatRate(detailSample.OtherIoBps)} net", trends.OtherIo),
                _ => ("CPU Trend", $"{detailSample.CpuPct:F1}% CPU", trends.Cpu),
            };
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
        _globalSummaryRow = new ProcessSample
        {
            Seq = _summarySeq,
            TsMs = _summaryTsMs,
            Pid = 0,
            ParentPid = 0,
            StartTimeMs = 0,
            Name = "Global System Values",
            CpuPct = _summaryCpuPct,
            RssBytes = ClampToUlong(_summaryRssBytes),
            PrivateBytes = ClampToUlong(_summaryPrivateBytes),
            IoReadBps = ClampToUlong(_summaryIoReadBps),
            IoWriteBps = ClampToUlong(_summaryIoWriteBps),
            OtherIoBps = ClampToUlong(_summaryOtherIoBps),
            Threads = ClampToUInt(_summaryThreads),
            Handles = ClampToUInt(_summaryHandles),
            AccessState = AccessState.Full,
        };

        _globalHistory.Append(_globalSummaryRow);
        if (SelectedRow is null)
        {
            OnPropertyChanged(nameof(DetailTitle));
        }
    }

    private static ProcessSample CreateEmptyGlobalSummary()
    {
        return new ProcessSample
        {
            Seq = 0,
            TsMs = UnixNowMs(),
            Pid = 0,
            ParentPid = 0,
            StartTimeMs = 0,
            Name = "Global System Values",
            CpuPct = 0,
            RssBytes = 0,
            PrivateBytes = 0,
            IoReadBps = 0,
            IoWriteBps = 0,
            OtherIoBps = 0,
            Threads = 0,
            Handles = 0,
            AccessState = AccessState.Full,
        };
    }

    private static ulong ClampToUlong(double value)
    {
        if (value <= 0)
        {
            return 0;
        }

        if (value >= ulong.MaxValue)
        {
            return ulong.MaxValue;
        }

        return (ulong)Math.Round(value);
    }

    private static uint ClampToUInt(double value)
    {
        if (value <= 0)
        {
            return 0;
        }

        if (value >= uint.MaxValue)
        {
            return uint.MaxValue;
        }

        return (uint)Math.Round(value);
    }

    private static ulong UnixNowMs()
    {
        long now = DateTimeOffset.UtcNow.ToUnixTimeMilliseconds();
        return now <= 0 ? 0UL : (ulong)now;
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
        CpuMetricChipValue = $"{detailSample.CpuPct:F2}%";
        MemoryMetricChipValue = ValueFormat.FormatBytes(detailSample.RssBytes);
        IoReadMetricChipValue = ValueFormat.FormatRate(detailSample.IoReadBps);
        IoWriteMetricChipValue = ValueFormat.FormatRate(detailSample.IoWriteBps);
        OtherIoMetricChipValue = ValueFormat.FormatRate(detailSample.OtherIoBps);
    }

    private static MetricTrends BuildMetricTrends(MetricHistoryBuffer history)
    {
        return new MetricTrends(
            history.Cpu.ToArray(),
            history.Memory.ToArray(),
            history.IoRead.ToArray(),
            history.IoWrite.ToArray(),
            history.OtherIo.ToArray());
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

    private readonly record struct MetricTrends(
        double[] Cpu,
        double[] Memory,
        double[] IoRead,
        double[] IoWrite,
        double[] OtherIo);
}
