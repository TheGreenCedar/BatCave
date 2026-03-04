using System;
using System.Collections;
using System.Collections.Generic;

namespace BatCave.ViewModels;

internal sealed partial class FixedRingSeries(int capacity) : IReadOnlyList<double>
{
    private readonly double[] _buffer = new double[Math.Max(1, capacity)];
    private int _start;
    private int _count;

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

    public double[] SliceLatest(int limit)
    {
        int take = Math.Min(_count, Math.Max(1, limit));
        double[] result = new double[take];
        int sourceStart = _count - take;
        for (int index = 0; index < take; index++)
        {
            result[index] = this[sourceStart + index];
        }

        return result;
    }

    public bool CopyLatestInto(ref double[] destination, int limit)
    {
        int windowSize = Math.Max(1, limit);
        int take = Math.Min(_count, windowSize);
        int leadingZeroCount = windowSize - take;
        bool changed = false;
        if (destination.Length != windowSize)
        {
            destination = new double[windowSize];
            changed = true;
        }

        for (int index = 0; index < leadingZeroCount; index++)
        {
            if (destination[index] == 0d)
            {
                continue;
            }

            destination[index] = 0d;
            changed = true;
        }

        int sourceStart = _count - take;
        for (int index = 0; index < take; index++)
        {
            double next = this[sourceStart + index];
            int targetIndex = leadingZeroCount + index;
            if (destination[targetIndex] == next)
            {
                continue;
            }

            destination[targetIndex] = next;
            changed = true;
        }

        return changed;
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
