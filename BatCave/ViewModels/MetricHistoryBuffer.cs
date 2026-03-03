using System.Collections;
using System.Collections.Generic;
using System;
using BatCave.Core.Domain;

namespace BatCave.ViewModels;

public sealed class MetricHistoryBuffer
{
    private readonly RingSeries _cpu;
    private readonly RingSeries _memory;
    private readonly RingSeries _ioRead;
    private readonly RingSeries _ioWrite;
    private readonly RingSeries _otherIo;

    public MetricHistoryBuffer(int limit)
    {
        int normalizedLimit = Math.Max(1, limit);
        _cpu = new RingSeries(normalizedLimit);
        _memory = new RingSeries(normalizedLimit);
        _ioRead = new RingSeries(normalizedLimit);
        _ioWrite = new RingSeries(normalizedLimit);
        _otherIo = new RingSeries(normalizedLimit);
    }

    public IReadOnlyList<double> Cpu => _cpu;

    public IReadOnlyList<double> Memory => _memory;

    public IReadOnlyList<double> IoRead => _ioRead;

    public IReadOnlyList<double> IoWrite => _ioWrite;

    public IReadOnlyList<double> OtherIo => _otherIo;

    public void Reset()
    {
        _cpu.Clear();
        _memory.Clear();
        _ioRead.Clear();
        _ioWrite.Clear();
        _otherIo.Clear();
    }

    public void Append(ProcessSample sample)
    {
        _cpu.Add(sample.CpuPct);
        _memory.Add(sample.RssBytes);
        _ioRead.Add(sample.IoReadBps);
        _ioWrite.Add(sample.IoWriteBps);
        _otherIo.Add(sample.OtherIoBps);
    }

    public static IReadOnlyList<double> Singleton(double value)
    {
        return [value];
    }

    private sealed class RingSeries : IReadOnlyList<double>
    {
        private readonly double[] _buffer;
        private int _start;
        private int _count;

        public RingSeries(int capacity)
        {
            _buffer = new double[capacity];
        }

        public int Count => _count;

        public double this[int index]
        {
            get
            {
                if ((uint)index >= (uint)_count)
                {
                    throw new ArgumentOutOfRangeException(nameof(index));
                }

                return _buffer[(_start + index) % _buffer.Length];
            }
        }

        public void Add(double value)
        {
            if (_count < _buffer.Length)
            {
                _buffer[(_start + _count) % _buffer.Length] = value;
                _count++;
                return;
            }

            _buffer[_start] = value;
            _start = (_start + 1) % _buffer.Length;
        }

        public void Clear()
        {
            _start = 0;
            _count = 0;
        }

        public IEnumerator<double> GetEnumerator()
        {
            for (int index = 0; index < _count; index++)
            {
                yield return this[index];
            }
        }

        IEnumerator IEnumerable.GetEnumerator()
        {
            return GetEnumerator();
        }
    }
}
