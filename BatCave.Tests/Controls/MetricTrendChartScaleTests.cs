using BatCave.Charts;

namespace BatCave.Tests.Controls;

public sealed class MetricTrendChartScaleTests
{
    [Fact]
    public void CpuDomain_LowUsage_ResolvesToFivePercentFloor()
    {
        double raw = MetricTrendScaleDomain.ResolveNextRawDomainMax(
            previousRawDomainMax: 0d,
            maxVisible: 3.7d,
            floor: MetricTrendScaleDomain.CpuFloorPercent,
            ceiling: MetricTrendScaleDomain.CpuCeilingPercent,
            paddingRatio: MetricTrendScaleDomain.DefaultPaddingRatio,
            decayFactor: MetricTrendScaleDomain.DefaultDecayFactor);

        double rendered = MetricTrendScaleDomain.ResolveRenderedDomainMax(
            rawDomainMax: raw,
            floor: MetricTrendScaleDomain.CpuFloorPercent,
            ceiling: MetricTrendScaleDomain.CpuCeilingPercent);

        Assert.Equal(5d, rendered);
    }

    [Fact]
    public void CpuDomain_MediumUsage_UsesChunkedNiceScale()
    {
        double raw = MetricTrendScaleDomain.ResolveNextRawDomainMax(
            previousRawDomainMax: 0d,
            maxVisible: 14.3d,
            floor: MetricTrendScaleDomain.CpuFloorPercent,
            ceiling: MetricTrendScaleDomain.CpuCeilingPercent,
            paddingRatio: MetricTrendScaleDomain.DefaultPaddingRatio,
            decayFactor: MetricTrendScaleDomain.DefaultDecayFactor);

        double rendered = MetricTrendScaleDomain.ResolveRenderedDomainMax(
            rawDomainMax: raw,
            floor: MetricTrendScaleDomain.CpuFloorPercent,
            ceiling: MetricTrendScaleDomain.CpuCeilingPercent);

        Assert.Equal(20d, rendered);
    }

    [Fact]
    public void CpuDomain_HighUsage_CapsAtHundredPercent()
    {
        double raw = MetricTrendScaleDomain.ResolveNextRawDomainMax(
            previousRawDomainMax: 0d,
            maxVisible: 92d,
            floor: MetricTrendScaleDomain.CpuFloorPercent,
            ceiling: MetricTrendScaleDomain.CpuCeilingPercent,
            paddingRatio: MetricTrendScaleDomain.DefaultPaddingRatio,
            decayFactor: MetricTrendScaleDomain.DefaultDecayFactor);

        double rendered = MetricTrendScaleDomain.ResolveRenderedDomainMax(
            rawDomainMax: raw,
            floor: MetricTrendScaleDomain.CpuFloorPercent,
            ceiling: MetricTrendScaleDomain.CpuCeilingPercent);

        Assert.Equal(100d, rendered);
    }

    [Fact]
    public void CpuDomain_DecaysAfterSpike_AndEventuallyReturnsToFloor()
    {
        double raw = MetricTrendScaleDomain.ResolveNextRawDomainMax(
            previousRawDomainMax: 0d,
            maxVisible: 95d,
            floor: MetricTrendScaleDomain.CpuFloorPercent,
            ceiling: MetricTrendScaleDomain.CpuCeilingPercent,
            paddingRatio: MetricTrendScaleDomain.DefaultPaddingRatio,
            decayFactor: MetricTrendScaleDomain.DefaultDecayFactor);

        for (int sample = 0; sample < 10; sample++)
        {
            raw = MetricTrendScaleDomain.ResolveNextRawDomainMax(
                previousRawDomainMax: raw,
                maxVisible: 3d,
                floor: MetricTrendScaleDomain.CpuFloorPercent,
                ceiling: MetricTrendScaleDomain.CpuCeilingPercent,
                paddingRatio: MetricTrendScaleDomain.DefaultPaddingRatio,
                decayFactor: MetricTrendScaleDomain.DefaultDecayFactor);
        }

        double renderedAfterDecay = MetricTrendScaleDomain.ResolveRenderedDomainMax(
            rawDomainMax: raw,
            floor: MetricTrendScaleDomain.CpuFloorPercent,
            ceiling: MetricTrendScaleDomain.CpuCeilingPercent);
        Assert.True(renderedAfterDecay < 100d);

        for (int sample = 0; sample < 90; sample++)
        {
            raw = MetricTrendScaleDomain.ResolveNextRawDomainMax(
                previousRawDomainMax: raw,
                maxVisible: 3d,
                floor: MetricTrendScaleDomain.CpuFloorPercent,
                ceiling: MetricTrendScaleDomain.CpuCeilingPercent,
                paddingRatio: MetricTrendScaleDomain.DefaultPaddingRatio,
                decayFactor: MetricTrendScaleDomain.DefaultDecayFactor);
        }

        double renderedAtFloor = MetricTrendScaleDomain.ResolveRenderedDomainMax(
            rawDomainMax: raw,
            floor: MetricTrendScaleDomain.CpuFloorPercent,
            ceiling: MetricTrendScaleDomain.CpuCeilingPercent);
        Assert.Equal(5d, renderedAtFloor);
    }

    [Fact]
    public void IoDomain_UsesChunkedScaleAboveIoFloor()
    {
        double raw = MetricTrendScaleDomain.ResolveNextRawDomainMax(
            previousRawDomainMax: 0d,
            maxVisible: 0d,
            floor: MetricTrendScaleDomain.IoRateFloorBytes,
            ceiling: null,
            paddingRatio: MetricTrendScaleDomain.DefaultPaddingRatio,
            decayFactor: MetricTrendScaleDomain.DefaultDecayFactor);

        double rendered = MetricTrendScaleDomain.ResolveRenderedDomainMax(
            rawDomainMax: raw,
            floor: MetricTrendScaleDomain.IoRateFloorBytes,
            ceiling: null);

        Assert.True(rendered >= MetricTrendScaleDomain.IoRateFloorBytes);
        Assert.Equal(SparklineMath.RoundUpToNice(MetricTrendScaleDomain.IoRateFloorBytes), rendered);
    }
}
