using System;
using System.Collections.Generic;
using BatCave.Converters;
using BatCave.Core.Domain;
using CommunityToolkit.Mvvm.ComponentModel;
using Windows.Foundation;

namespace BatCave.ViewModels;

public sealed class ProcessRowViewState : ObservableObject
{
    private const double CpuSortPrecision = 0.01;

    private ProcessSample _sample;
    private IReadOnlyList<Point> _cpuTrendGeometry;
    private string _cpuText;
    private string _rssText;
    private string _ioReadText;
    private string _ioWriteText;
    private string _otherIoText;

    public ProcessRowViewState(ProcessSample sample, IReadOnlyList<Point> cpuTrendGeometry)
    {
        _sample = sample;
        _cpuTrendGeometry = cpuTrendGeometry;
        (_cpuText, _rssText, _ioReadText, _ioWriteText, _otherIoText) = CreateDisplayText(sample);
    }

    public ProcessSample Sample => _sample;

    public ProcessIdentity Identity => _sample.Identity();

    public string Name => _sample.Name;

    public uint Pid => _sample.Pid;

    public ulong StartTimeMs => _sample.StartTimeMs;

    public double CpuPct => _sample.CpuPct;

    public double CpuSortBucket => QuantizeCpu(_sample.CpuPct);

    public ulong RssBytes => _sample.RssBytes;

    public ulong IoReadBps => _sample.IoReadBps;

    public ulong IoWriteBps => _sample.IoWriteBps;

    public ulong OtherIoBps => _sample.OtherIoBps;

    public string CpuText
    {
        get => _cpuText;
        private set => SetProperty(ref _cpuText, value);
    }

    public string RssText
    {
        get => _rssText;
        private set => SetProperty(ref _rssText, value);
    }

    public string IoReadText
    {
        get => _ioReadText;
        private set => SetProperty(ref _ioReadText, value);
    }

    public string IoWriteText
    {
        get => _ioWriteText;
        private set => SetProperty(ref _ioWriteText, value);
    }

    public string OtherIoText
    {
        get => _otherIoText;
        private set => SetProperty(ref _otherIoText, value);
    }

    public uint Threads => _sample.Threads;

    public uint Handles => _sample.Handles;

    public AccessState AccessState => _sample.AccessState;

    public IReadOnlyList<Point> CpuTrendGeometry
    {
        get => _cpuTrendGeometry;
        private set => SetProperty(ref _cpuTrendGeometry, value);
    }

    public void UpdateSample(ProcessSample sample)
    {
        if (_sample == sample)
        {
            return;
        }

        ProcessSample previous = _sample;
        _sample = sample;
        RaiseSamplePropertyChanges(previous, sample);
    }

    public void UpdateCpuTrendGeometry(IReadOnlyList<Point> geometry)
    {
        ArgumentNullException.ThrowIfNull(geometry);

        IReadOnlyList<Point> currentGeometry = CpuTrendGeometry;
        if (geometry.Count == currentGeometry.Count && currentGeometry is IList<Point> writableGeometry)
        {
            bool changed = false;
            for (int index = 0; index < geometry.Count; index++)
            {
                Point next = geometry[index];
                if (writableGeometry[index] == next)
                {
                    continue;
                }

                writableGeometry[index] = next;
                changed = true;
            }

            if (changed)
            {
                OnPropertyChanged(nameof(CpuTrendGeometry));
            }

            return;
        }

        CpuTrendGeometry = geometry;
    }

    private static double QuantizeCpu(double cpuPct)
    {
        return Math.Round(cpuPct / CpuSortPrecision, MidpointRounding.AwayFromZero) * CpuSortPrecision;
    }

    internal static bool IsCpuSortBucketChanged(double previous, double current)
    {
        return QuantizeCpu(previous) != QuantizeCpu(current);
    }

    private static string FormatCpu(double cpuPct)
    {
        return $"{cpuPct:F2}%";
    }

    private static (string Cpu, string Rss, string IoRead, string IoWrite, string OtherIo) CreateDisplayText(ProcessSample sample)
    {
        return (
            FormatCpu(sample.CpuPct),
            ValueFormat.FormatBytes(sample.RssBytes),
            ValueFormat.FormatRate(sample.IoReadBps),
            ValueFormat.FormatRate(sample.IoWriteBps),
            ValueFormat.FormatRate(sample.OtherIoBps));
    }

    private void RaiseSamplePropertyChanges(ProcessSample previous, ProcessSample current)
    {
        if (!string.Equals(previous.Name, current.Name, StringComparison.Ordinal))
        {
            OnPropertyChanged(nameof(Name));
        }

        if (previous.CpuPct != current.CpuPct)
        {
            OnPropertyChanged(nameof(CpuPct));
            if (IsCpuSortBucketChanged(previous.CpuPct, current.CpuPct))
            {
                OnPropertyChanged(nameof(CpuSortBucket));
            }

            CpuText = FormatCpu(current.CpuPct);
        }

        UpdateFormattedMetricIfChanged(previous.RssBytes, current.RssBytes, nameof(RssBytes), value => RssText = value, ValueFormat.FormatBytes);
        UpdateFormattedMetricIfChanged(previous.IoReadBps, current.IoReadBps, nameof(IoReadBps), value => IoReadText = value, ValueFormat.FormatRate);
        UpdateFormattedMetricIfChanged(previous.IoWriteBps, current.IoWriteBps, nameof(IoWriteBps), value => IoWriteText = value, ValueFormat.FormatRate);
        UpdateFormattedMetricIfChanged(previous.OtherIoBps, current.OtherIoBps, nameof(OtherIoBps), value => OtherIoText = value, ValueFormat.FormatRate);

        RaiseIfChanged(previous.Threads, current.Threads, nameof(Threads));
        RaiseIfChanged(previous.Handles, current.Handles, nameof(Handles));
        RaiseIfChanged(previous.AccessState, current.AccessState, nameof(AccessState));
    }

    private void UpdateFormattedMetricIfChanged<TValue>(
        TValue previous,
        TValue current,
        string propertyName,
        Action<string> applyText,
        Func<TValue, string> formatter)
    {
        if (RaiseIfChanged(previous, current, propertyName))
        {
            applyText(formatter(current));
        }
    }

    private bool RaiseIfChanged<T>(T previous, T current, string propertyName)
    {
        if (!EqualityComparer<T>.Default.Equals(previous, current))
        {
            OnPropertyChanged(propertyName);
            return true;
        }

        return false;
    }
}
