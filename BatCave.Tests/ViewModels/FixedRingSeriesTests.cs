using BatCave.ViewModels;

namespace BatCave.Tests.ViewModels;

public sealed class FixedRingSeriesTests
{
    [Fact]
    public void CopyLatestInto_PadsLeadingWindowWithZeros()
    {
        FixedRingSeries series = new(capacity: 120);
        series.Add(5d);
        series.Add(7d);

        double[] destination = [];
        bool changed = series.CopyLatestInto(ref destination, limit: 60);

        Assert.True(changed);
        Assert.Equal(60, destination.Length);
        Assert.All(destination.Take(58), value => Assert.Equal(0d, value));
        Assert.Equal(5d, destination[58]);
        Assert.Equal(7d, destination[59]);
    }
}