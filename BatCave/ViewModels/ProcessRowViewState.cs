using System;
using System.Collections.Generic;
using BatCave.Converters;
using BatCave.Core.Domain;
using CommunityToolkit.Mvvm.ComponentModel;

namespace BatCave.ViewModels;

public sealed class ProcessRowViewState : ObservableObject
{
    private const double CpuSortPrecision = 0.01;

    private ProcessSample _sample;
    private string _cpuTrendPoints;
    private string _cpuText;
    private string _rssText;
    private string _ioReadText;
    private string _ioWriteText;
    private string _netText;

    public ProcessRowViewState(ProcessSample sample, string cpuTrendPoints)
    {
        _sample = sample;
        _cpuTrendPoints = cpuTrendPoints;
        _cpuText = FormatCpu(sample.CpuPct);
        _rssText = ValueFormat.FormatBytes(sample.RssBytes);
        _ioReadText = ValueFormat.FormatRate(sample.IoReadBps);
        _ioWriteText = ValueFormat.FormatRate(sample.IoWriteBps);
        _netText = ValueFormat.FormatRate(sample.NetBps);
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

    public ulong NetBps => _sample.NetBps;

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

    public string NetText
    {
        get => _netText;
        private set => SetProperty(ref _netText, value);
    }

    public uint Threads => _sample.Threads;

    public uint Handles => _sample.Handles;

    public AccessState AccessState => _sample.AccessState;

    public string CpuTrendPoints
    {
        get => _cpuTrendPoints;
        private set => SetProperty(ref _cpuTrendPoints, value);
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

    public void UpdateCpuTrendPoints(string points)
    {
        CpuTrendPoints = points;
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

        if (RaiseIfChanged(previous.RssBytes, current.RssBytes, nameof(RssBytes)))
        {
            RssText = ValueFormat.FormatBytes(current.RssBytes);
        }

        if (RaiseIfChanged(previous.IoReadBps, current.IoReadBps, nameof(IoReadBps)))
        {
            IoReadText = ValueFormat.FormatRate(current.IoReadBps);
        }

        if (RaiseIfChanged(previous.IoWriteBps, current.IoWriteBps, nameof(IoWriteBps)))
        {
            IoWriteText = ValueFormat.FormatRate(current.IoWriteBps);
        }

        if (RaiseIfChanged(previous.NetBps, current.NetBps, nameof(NetBps)))
        {
            NetText = ValueFormat.FormatRate(current.NetBps);
        }

        RaiseIfChanged(previous.Threads, current.Threads, nameof(Threads));
        RaiseIfChanged(previous.Handles, current.Handles, nameof(Handles));
        RaiseIfChanged(previous.AccessState, current.AccessState, nameof(AccessState));
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
