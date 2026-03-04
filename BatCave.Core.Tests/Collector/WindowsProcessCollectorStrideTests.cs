using BatCave.Core.Collector;
using System.Reflection;

namespace BatCave.Core.Tests.Collector;

public class WindowsProcessCollectorStrideTests
{
    private static readonly MethodInfo ShouldRefreshMetricMethod =
        typeof(WindowsProcessCollector).GetMethod(
            "ShouldRefreshMetric",
            BindingFlags.NonPublic | BindingFlags.Static)
        ?? throw new InvalidOperationException("Could not find WindowsProcessCollector.ShouldRefreshMetric.");

    [Theory]
    [InlineData(1UL)]
    [InlineData(0UL)]
    public void ShouldRefreshMetric_WhenStrideAtMostOne_AlwaysRefreshes(ulong stride)
    {
        bool shouldRefresh = InvokeShouldRefreshMetric(seq: 10, lastSampleSeq: 10, stride: stride);

        Assert.True(shouldRefresh);
    }

    [Fact]
    public void ShouldRefreshMetric_WhenElapsedIsBelowStride_DoesNotRefresh()
    {
        bool shouldRefresh = InvokeShouldRefreshMetric(seq: 11, lastSampleSeq: 10, stride: 2);

        Assert.False(shouldRefresh);
    }

    [Fact]
    public void ShouldRefreshMetric_WhenElapsedMeetsStride_Refreshes()
    {
        bool shouldRefresh = InvokeShouldRefreshMetric(seq: 12, lastSampleSeq: 10, stride: 2);

        Assert.True(shouldRefresh);
    }

    [Fact]
    public void ShouldRefreshMetric_WhenSequenceRegresses_DoesNotRefresh()
    {
        bool shouldRefresh = InvokeShouldRefreshMetric(seq: 9, lastSampleSeq: 10, stride: 2);

        Assert.False(shouldRefresh);
    }

    private static bool InvokeShouldRefreshMetric(ulong seq, ulong lastSampleSeq, ulong stride)
    {
        object? result = ShouldRefreshMetricMethod.Invoke(null, [seq, lastSampleSeq, stride]);
        return result is true;
    }
}
