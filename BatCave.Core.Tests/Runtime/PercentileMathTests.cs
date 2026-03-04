using BatCave.Core.Runtime;

namespace BatCave.Core.Tests.Runtime;

public class PercentileMathTests
{
    [Fact]
    public void Percentile95_ReadOnlyList_Empty_ReturnsZero()
    {
        double result = PercentileMath.Percentile95(Array.Empty<double>());

        Assert.Equal(0d, result);
    }

    [Fact]
    public void Percentile95_BufferedCount_NonPositive_ReturnsZero()
    {
        double[] values = [1d, 2d, 3d];
        double[] scratch = new double[values.Length];

        double result = PercentileMath.Percentile95(values, count: 0, scratch);

        Assert.Equal(0d, result);
    }

    [Fact]
    public void Percentile95_Overloads_ReturnEquivalentResultsForSameSamples()
    {
        double[] values = [5d, 1d, 9d, 2d, 7d, 6d, 4d, 8d, 3d, 10d];
        double[] scratch = new double[values.Length];

        double listResult = PercentileMath.Percentile95(values);
        double bufferedResult = PercentileMath.Percentile95(values, values.Length, scratch);

        Assert.Equal(listResult, bufferedResult, precision: 10);
        Assert.Equal(10d, listResult);
    }
}
