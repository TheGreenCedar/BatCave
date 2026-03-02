using System;
using System.Collections.Generic;
using BatCave.Core.Domain;
using CommunityToolkit.Mvvm.ComponentModel;

namespace BatCave.ViewModels;

public sealed class ProcessRowViewState : ObservableObject
{
    private const double CpuSortPrecision = 0.01;

    private ProcessSample _sample;
    private string _cpuTrendPoints;

    public ProcessRowViewState(ProcessSample sample, string cpuTrendPoints)
    {
        _sample = sample;
        _cpuTrendPoints = cpuTrendPoints;
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
            if (QuantizeCpu(previous.CpuPct) != QuantizeCpu(current.CpuPct))
            {
                OnPropertyChanged(nameof(CpuSortBucket));
            }
        }

        RaiseIfChanged(previous.RssBytes, current.RssBytes, nameof(RssBytes));
        RaiseIfChanged(previous.IoReadBps, current.IoReadBps, nameof(IoReadBps));
        RaiseIfChanged(previous.IoWriteBps, current.IoWriteBps, nameof(IoWriteBps));
        RaiseIfChanged(previous.NetBps, current.NetBps, nameof(NetBps));
        RaiseIfChanged(previous.Threads, current.Threads, nameof(Threads));
        RaiseIfChanged(previous.Handles, current.Handles, nameof(Handles));
        RaiseIfChanged(previous.AccessState, current.AccessState, nameof(AccessState));
    }

    private static double QuantizeCpu(double cpuPct)
    {
        return Math.Round(cpuPct / CpuSortPrecision, MidpointRounding.AwayFromZero) * CpuSortPrecision;
    }

    private void RaiseIfChanged<T>(T previous, T current, string propertyName)
    {
        if (!EqualityComparer<T>.Default.Equals(previous, current))
        {
            OnPropertyChanged(propertyName);
        }
    }
}
