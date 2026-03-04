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

    [Fact]
    public void ResolveCpuSpeedMHz_WhenActualFrequencyValid_PrefersActualFrequency()
    {
        double? resolved = WindowsSystemGlobalMetricsSampler.ResolveCpuSpeedMHz(
            actualFrequencyMHz: 4200,
            processorFrequencyMHz: 3900,
            staticCurrentClockSpeedMHz: 3600);
        Assert.Equal(4200d, resolved);
    }

    [Fact]
    public void ResolveCpuSpeedMHz_WhenActualFrequencyInvalid_FallsBackToProcessorFrequency()
    {
        double? resolved = WindowsSystemGlobalMetricsSampler.ResolveCpuSpeedMHz(
            actualFrequencyMHz: 0,
            processorFrequencyMHz: 3900,
            staticCurrentClockSpeedMHz: 3600);
        Assert.Equal(3900d, resolved);
    }

    [Fact]
    public void ResolveCpuSpeedMHz_WhenDynamicValuesInvalid_FallsBackToStaticClockSpeed()
    {
        double? resolved = WindowsSystemGlobalMetricsSampler.ResolveCpuSpeedMHz(
            actualFrequencyMHz: double.NaN,
            processorFrequencyMHz: -1,
            staticCurrentClockSpeedMHz: 3600);
        Assert.Equal(3600d, resolved);
    }

    [Fact]
    public void ResolveCpuSpeedMHz_WhenAllCandidatesInvalid_ReturnsNull()
    {
        double? resolved = WindowsSystemGlobalMetricsSampler.ResolveCpuSpeedMHz(
            actualFrequencyMHz: double.PositiveInfinity,
            processorFrequencyMHz: 99,
            staticCurrentClockSpeedMHz: 25000);
        Assert.Null(resolved);
    }

    [Theory]
    [InlineData(double.NegativeInfinity)]
    [InlineData(double.PositiveInfinity)]
    [InlineData(double.NaN)]
    [InlineData(0d)]
    [InlineData(-5d)]
    [InlineData(99d)]
    [InlineData(20001d)]
    public void NormalizeCpuSpeedMHz_WhenValueIsNotReasonable_ReturnsNull(double value)
    {
        double? resolved = WindowsSystemGlobalMetricsSampler.NormalizeCpuSpeedMHz(value);
        Assert.Null(resolved);
    }

    [Theory]
    [InlineData(100d)]
    [InlineData(3600d)]
    [InlineData(20000d)]
    public void NormalizeCpuSpeedMHz_WhenValueIsReasonable_ReturnsValue(double value)
    {
        double? resolved = WindowsSystemGlobalMetricsSampler.NormalizeCpuSpeedMHz(value);
        Assert.Equal(value, resolved);
    }
}
