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

        _sample = sample;
        RaiseSamplePropertyChanges();
    }

    public void UpdateCpuTrendPoints(string points)
    {
        CpuTrendPoints = points;
    }

    private void RaiseSamplePropertyChanges()
    {
        OnPropertyChanged(nameof(Sample));
        OnPropertyChanged(nameof(Identity));
        OnPropertyChanged(nameof(Name));
        OnPropertyChanged(nameof(Pid));
        OnPropertyChanged(nameof(CpuPct));
        OnPropertyChanged(nameof(RssBytes));
        OnPropertyChanged(nameof(IoReadBps));
        OnPropertyChanged(nameof(IoWriteBps));
        OnPropertyChanged(nameof(NetBps));
        OnPropertyChanged(nameof(Threads));
        OnPropertyChanged(nameof(Handles));
        OnPropertyChanged(nameof(AccessState));
    }
}
