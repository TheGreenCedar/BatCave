using BatCave.Core.Collector;

namespace BatCave.Core.Tests.Collector;

public class WindowsSystemGlobalMetricsSamplerTests
{
    [Fact]
    public void ResolveDiskActiveTimePct_WhenBothCountersMissing_ReturnsNull()
    {
        double? resolved = WindowsSystemGlobalMetricsSampler.ResolveDiskActiveTimePct(diskTimePct: null, idleTimePct: null);
        Assert.Null(resolved);
    }

    [Fact]
    public void ResolveDiskActiveTimePct_WhenDiskTimePresent_UsesDiskTime()
    {
        double? resolved = WindowsSystemGlobalMetricsSampler.ResolveDiskActiveTimePct(diskTimePct: 37, idleTimePct: null);
        Assert.Equal(37d, resolved);
    }

    [Fact]
    public void ResolveDiskActiveTimePct_WhenDiskTimeZeroAndIdlePresent_UsesIdleFallback()
    {
        double? resolved = WindowsSystemGlobalMetricsSampler.ResolveDiskActiveTimePct(diskTimePct: 0, idleTimePct: 72);
        Assert.Equal(28d, resolved);
    }

    [Fact]
    public void ResolveDiskActiveTimePct_WhenBothPresent_UsesHigherActiveValue()
    {
        double? resolved = WindowsSystemGlobalMetricsSampler.ResolveDiskActiveTimePct(diskTimePct: 5, idleTimePct: 70);
        Assert.Equal(30d, resolved);
    }

    [Fact]
    public void ResolveDiskActiveTimePct_WhenValuesOutOfRange_ClampsToPercentBounds()
    {
        double? resolved = WindowsSystemGlobalMetricsSampler.ResolveDiskActiveTimePct(diskTimePct: 140, idleTimePct: 180);
        Assert.Equal(100d, resolved);
    }

    [Fact]
    public void ResolvePreferredDiskActiveTimePct_WhenBothPresent_UsesHigherValue()
    {
        double? resolved = WindowsSystemGlobalMetricsSampler.ResolvePreferredDiskActiveTimePct(primaryPct: 12, fallbackPct: 67);
        Assert.Equal(67d, resolved);
    }

    [Fact]
    public void ResolvePreferredDiskActiveTimePct_WhenPrimaryMissing_UsesFallback()
    {
        double? resolved = WindowsSystemGlobalMetricsSampler.ResolvePreferredDiskActiveTimePct(primaryPct: null, fallbackPct: 31);
        Assert.Equal(31d, resolved);
    }

    [Fact]
    public void ResolvePreferredDiskActiveTimePct_WhenBothInvalid_ReturnsNull()
    {
        double? resolved = WindowsSystemGlobalMetricsSampler.ResolvePreferredDiskActiveTimePct(primaryPct: double.NaN, fallbackPct: double.PositiveInfinity);
        Assert.Null(resolved);
    }
}
