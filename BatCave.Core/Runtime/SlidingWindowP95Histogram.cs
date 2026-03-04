namespace BatCave.Core.Runtime;

internal sealed class SlidingWindowP95Histogram
{
    private readonly long[] _bucketCounts;
    private readonly int[] _windowBuckets;
    private int _windowCursor;
    private int _sampleCount;

    public SlidingWindowP95Histogram(int windowSize, int maxBucketInclusive)
    {
        int safeWindowSize = Math.Max(1, windowSize);
        int safeMaxBucket = Math.Max(1, maxBucketInclusive);
        _bucketCounts = new long[safeMaxBucket + 1];
        _windowBuckets = new int[safeWindowSize];
    }

    public void AddSampleMs(double sampleMs)
    {
        if (!double.IsFinite(sampleMs))
        {
            return;
        }

        int bucket = (int)Math.Round(sampleMs);
        bucket = Math.Clamp(bucket, 0, _bucketCounts.Length - 1);

        if (_sampleCount < _windowBuckets.Length)
        {
            _sampleCount++;
        }
        else
        {
            int overwrittenBucket = _windowBuckets[_windowCursor];
            _bucketCounts[overwrittenBucket] = Math.Max(0, _bucketCounts[overwrittenBucket] - 1);
        }

        _windowBuckets[_windowCursor] = bucket;
        _windowCursor = (_windowCursor + 1) % _windowBuckets.Length;
        _bucketCounts[bucket]++;
    }

    public double Percentile95Ms()
    {
        if (_sampleCount <= 0)
        {
            return 0d;
        }

        long targetRank = (long)Math.Ceiling(_sampleCount * 0.95);
        long cumulative = 0;
        for (int bucket = 0; bucket < _bucketCounts.Length; bucket++)
        {
            cumulative += _bucketCounts[bucket];
            if (cumulative >= targetRank)
            {
                return bucket;
            }
        }

        return _bucketCounts.Length - 1;
    }
}
