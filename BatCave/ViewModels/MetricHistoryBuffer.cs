using System.Collections.Generic;
using BatCave.Core.Domain;

namespace BatCave.ViewModels;

public sealed class MetricHistoryBuffer
{
    private readonly int _limit;

    private readonly List<double> _cpu = [];
    private readonly List<double> _memory = [];
    private readonly List<double> _ioRead = [];
    private readonly List<double> _ioWrite = [];
    private readonly List<double> _net = [];

    public MetricHistoryBuffer(int limit)
    {
        _limit = limit;
    }

    public IReadOnlyList<double> Cpu => _cpu;

    public IReadOnlyList<double> Memory => _memory;

    public IReadOnlyList<double> IoRead => _ioRead;

    public IReadOnlyList<double> IoWrite => _ioWrite;

    public IReadOnlyList<double> Net => _net;

    public void Reset()
    {
        foreach (List<double> series in Series())
        {
            series.Clear();
        }
    }

    public void Append(ProcessSample sample)
    {
        Append(_cpu, sample.CpuPct);
        Append(_memory, sample.RssBytes);
        Append(_ioRead, sample.IoReadBps);
        Append(_ioWrite, sample.IoWriteBps);
        Append(_net, sample.NetBps);
    }

    public static IReadOnlyList<double> Singleton(double value)
    {
        return [value];
    }

    private void Append(List<double> values, double value)
    {
        values.Add(value);
        if (values.Count > _limit)
        {
            values.RemoveRange(0, values.Count - _limit);
        }
    }

    private IEnumerable<List<double>> Series()
    {
        yield return _cpu;
        yield return _memory;
        yield return _ioRead;
        yield return _ioWrite;
        yield return _net;
    }
}
