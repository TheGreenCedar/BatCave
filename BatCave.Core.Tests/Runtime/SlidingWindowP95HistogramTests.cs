using BatCave.Core.Runtime;

namespace BatCave.Core.Tests.Runtime;

public class SlidingWindowP95HistogramTests
{
    [Fact]
    public void Percentile95Ms_WhenNoSamples_ReturnsZero()
    {
        SlidingWindowP95Histogram histogram = new(windowSize: 8, maxBucketInclusive: 100);

        Assert.Equal(0d, histogram.Percentile95Ms());
    }

    [Fact]
    public void Percentile95Ms_ComputesExpectedRankWithinWindow()
    {
        SlidingWindowP95Histogram histogram = new(windowSize: 20, maxBucketInclusive: 100);

        for (int value = 1; value <= 20; value++)
        {
            histogram.AddSampleMs(value);
        }

        Assert.Equal(19d, histogram.Percentile95Ms());
    }

    [Fact]
    public void AddSampleMs_WhenWindowSlides_EvictsOldSamplesFromPercentile()
    {
        SlidingWindowP95Histogram histogram = new(windowSize: 10, maxBucketInclusive: 200);

        histogram.AddSampleMs(100);
        for (int index = 0; index < 9; index++)
        {
            histogram.AddSampleMs(1);
        }

        Assert.Equal(100d, histogram.Percentile95Ms());

        for (int index = 0; index < 10; index++)
        {
            histogram.AddSampleMs(1);
        }

        Assert.Equal(1d, histogram.Percentile95Ms());
    }

    [Fact]
    public void AddSampleMs_IgnoresNonFiniteAndClampsBucketRange()
    {
        SlidingWindowP95Histogram histogram = new(windowSize: 4, maxBucketInclusive: 50);

        histogram.AddSampleMs(double.NaN);
        histogram.AddSampleMs(double.PositiveInfinity);
        histogram.AddSampleMs(double.NegativeInfinity);

        Assert.Equal(0d, histogram.Percentile95Ms());

        histogram.AddSampleMs(-10);
        histogram.AddSampleMs(25.1);
        histogram.AddSampleMs(999);

        Assert.Equal(50d, histogram.Percentile95Ms());
    }
}
