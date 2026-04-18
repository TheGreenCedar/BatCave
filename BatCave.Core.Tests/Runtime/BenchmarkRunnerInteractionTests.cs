using BatCave.Core.Runtime;

namespace BatCave.Core.Tests.Runtime;

public class BenchmarkRunnerInteractionTests
{
    [Fact]
    public void Run_PopulatesCoreBenchmarkMetadata()
    {
        BenchmarkSummary summary = BenchmarkRunner.Run(
            ticks: 0,
            sleepMs: 0,
            ct: CancellationToken.None);

        Assert.Equal("core", summary.Host);
        Assert.Equal("headless_runtime", summary.MeasurementOrigin);
        Assert.False(summary.UsesAttachedDispatcher);
        Assert.True(summary.BaselineMetadataMatched);
    }

    [Fact]
    public void Run_WithRequiredInteractionSpeedupAndMissingSamples_FailsStrictGate()
    {
        BenchmarkSummary baseline = new()
        {
            TickP95Ms = 1.0,
            SortP95Ms = 1.0,
            InteractionProbeP95 = BuildProbeP95(10.0),
        };

        BenchmarkSummary summary = BenchmarkRunner.Run(
            ticks: 0,
            sleepMs: 0,
            ct: CancellationToken.None,
            gateOptions: new BenchmarkGateOptions
            {
                Baseline = baseline,
                MinSpeedupMultiplier = 10.0,
                RequireInteractionProbeSpeedup = true,
            });

        Assert.False(summary.InteractionSpeedupPassed);
        Assert.False(summary.SpeedupPassed);
        Assert.False(summary.StrictPassed);
    }

    [Fact]
    public void CreateSummary_WithMismatchedBaselineMetadata_FailsStrictGate()
    {
        BenchmarkSummary baseline = new()
        {
            Host = "core",
            MeasurementOrigin = "headless_runtime",
            TickP95Ms = 1.0,
            SortP95Ms = 1.0,
        };

        BenchmarkSummary summary = BenchmarkRunner.CreateSummary(
            new BenchmarkMeasurement
            {
                Host = "winui",
                MeasurementOrigin = "live_shell",
                UsesAttachedDispatcher = true,
                Ticks = 0,
                SleepMs = 0,
                TickP95Ms = 0.5,
                SortP95Ms = 0.5,
            },
            new BenchmarkGateOptions
            {
                Baseline = baseline,
                MinSpeedupMultiplier = 1.2,
            });

        Assert.False(summary.BaselineMetadataMatched);
        Assert.Null(summary.BaselineComparison);
        Assert.False(summary.CoreSpeedupPassed);
        Assert.False(summary.SpeedupPassed);
        Assert.False(summary.StrictPassed);
    }

    [Fact]
    public void Run_WithRequiredInteractionSpeedupAndAllProbeSpeedupsMeetingTarget_PassesStrictGate()
    {
        BenchmarkSummary baseline = new()
        {
            TickP95Ms = 1.0,
            SortP95Ms = 1.0,
            InteractionProbeP95 = BuildProbeP95(10.0),
        };

        BenchmarkSummary summary = BenchmarkRunner.Run(
            ticks: 0,
            sleepMs: 0,
            ct: CancellationToken.None,
            gateOptions: new BenchmarkGateOptions
            {
                Baseline = baseline,
                MinSpeedupMultiplier = 10.0,
                InteractionProbeP95 = BuildProbeP95(0.5),
                RequireInteractionProbeSpeedup = true,
            });

        Assert.True(summary.InteractionSpeedupPassed);
        Assert.NotNull(summary.InteractionBaselineComparison);
        Assert.True(summary.InteractionBaselineComparison!.MeetsMinSpeedup);
        Assert.True(summary.SpeedupPassed);
        Assert.True(summary.StrictPassed);
    }

    [Fact]
    public void Run_WithRequiredInteractionSpeedupAndOneProbeBelowTarget_FailsInteractionGate()
    {
        BenchmarkSummary baseline = new()
        {
            TickP95Ms = 1.0,
            SortP95Ms = 1.0,
            InteractionProbeP95 = BuildProbeP95(10.0),
        };

        BenchmarkInteractionProbeP95 currentProbe = BuildProbeP95(0.5) with
        {
            PlotP95Ms = 1.5,
        };

        BenchmarkSummary summary = BenchmarkRunner.Run(
            ticks: 0,
            sleepMs: 0,
            ct: CancellationToken.None,
            gateOptions: new BenchmarkGateOptions
            {
                Baseline = baseline,
                MinSpeedupMultiplier = 10.0,
                InteractionProbeP95 = currentProbe,
                RequireInteractionProbeSpeedup = true,
            });

        Assert.NotNull(summary.InteractionBaselineComparison);
        Assert.False(summary.InteractionBaselineComparison!.MeetsMinSpeedup);
        Assert.False(summary.InteractionSpeedupPassed);
        Assert.False(summary.StrictPassed);
    }

    private static BenchmarkInteractionProbeP95 BuildProbeP95(double value)
    {
        return new BenchmarkInteractionProbeP95
        {
            FilterP95Ms = value,
            SortP95Ms = value,
            SelectionP95Ms = value,
            BatchP95Ms = value,
            PlotP95Ms = value,
        };
    }
}
