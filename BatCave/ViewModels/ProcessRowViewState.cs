using System;
using BatCave.Core.Domain;
using CommunityToolkit.Mvvm.ComponentModel;

namespace BatCave.ViewModels;

public sealed class ProcessRowViewState : ObservableObject
{
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

    public double CpuPct => _sample.CpuPct;

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
        }

        if (previous.RssBytes != current.RssBytes)
        {
            OnPropertyChanged(nameof(RssBytes));
        }

        if (previous.IoReadBps != current.IoReadBps)
        {
            OnPropertyChanged(nameof(IoReadBps));
        }

        if (previous.IoWriteBps != current.IoWriteBps)
        {
            OnPropertyChanged(nameof(IoWriteBps));
        }

        if (previous.NetBps != current.NetBps)
        {
            OnPropertyChanged(nameof(NetBps));
        }

        if (previous.Threads != current.Threads)
        {
            OnPropertyChanged(nameof(Threads));
        }

        if (previous.Handles != current.Handles)
        {
            OnPropertyChanged(nameof(Handles));
        }

        if (previous.AccessState != current.AccessState)
        {
            OnPropertyChanged(nameof(AccessState));
        }
    }
}
