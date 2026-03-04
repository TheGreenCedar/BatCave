using BatCave.Core.Runtime;

namespace BatCave.Core.Tests.Runtime;

public class BenchmarkRunnerInteractionTests
{
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
