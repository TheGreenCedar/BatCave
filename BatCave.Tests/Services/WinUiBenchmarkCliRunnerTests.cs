using BatCave.Services;

namespace BatCave.Tests.Services;

public sealed class WinUiBenchmarkCliRunnerTests
{
    [Fact]
    public void IsBenchmarkCommand_IsCaseInsensitive()
    {
        Assert.True(WinUiBenchmarkCliRunner.IsBenchmarkCommand(["--BENCHMARK"]));
        Assert.False(WinUiBenchmarkCliRunner.IsBenchmarkCommand(["--strict"]));
    }

    [Fact]
    public void TryCreateOptions_WhenArgumentsValid_ParsesDefaultsAndGateOptions()
    {
        string baselinePath = Path.GetTempFileName();
        try
        {
            File.WriteAllText(
                baselinePath,
                """
                {"tick_p95_ms":10.0,"sort_p95_ms":5.0}
                """);

            bool parsed = WinUiBenchmarkCliRunner.TryCreateOptions(
                [
                    "--benchmark",
                    "--ticks", "12",
                    "--sleep-ms", "34",
                    "--strict",
                    "--baseline-json", baselinePath,
                    "--min-speedup-multiplier", "1.5",
                    "--max-p95-ms", "25.5",
                ],
                out WinUiBenchmarkCliRunner.WinUiBenchmarkCliOptions options,
                out IReadOnlyList<string> errors);

            Assert.True(parsed);
            Assert.Empty(errors);
            Assert.Equal(12, options.Ticks);
            Assert.Equal(34, options.SleepMs);
            Assert.True(options.Strict);
            Assert.Equal(1.5, options.GateOptions.MinSpeedupMultiplier);
            Assert.Equal(25.5, options.GateOptions.MaxP95Ms);
            Assert.NotNull(options.GateOptions.Baseline);
            Assert.Equal(10.0, options.GateOptions.Baseline!.TickP95Ms);
        }
        finally
        {
            File.Delete(baselinePath);
        }
    }

    [Fact]
    public void TryCreateOptions_WhenMinSpeedupHasNoBaseline_ReturnsExistingValidationMessage()
    {
        bool parsed = WinUiBenchmarkCliRunner.TryCreateOptions(
            ["--benchmark", "--min-speedup-multiplier", "1.2"],
            out _,
            out IReadOnlyList<string> errors);

        Assert.False(parsed);
        Assert.Single(errors);
        Assert.Equal("--min-speedup-multiplier requires --baseline-json.", errors[0]);
    }

    [Theory]
    [InlineData("--ticks", "abc", "Missing or invalid value for --ticks.")]
    [InlineData("--sleep-ms", "abc", "Missing or invalid value for --sleep-ms.")]
    [InlineData("--min-speedup-multiplier", "-1", "Missing or invalid value for --min-speedup-multiplier (must be > 0).")]
    [InlineData("--max-p95-ms", "0", "Missing or invalid value for --max-p95-ms (must be > 0).")]
    public void TryCreateOptions_WhenNumericArgumentInvalid_ReturnsFriendlyMessage(
        string optionName,
        string value,
        string expectedError)
    {
        bool parsed = WinUiBenchmarkCliRunner.TryCreateOptions(
            ["--benchmark", optionName, value],
            out _,
            out IReadOnlyList<string> errors);

        Assert.False(parsed);
        Assert.Contains(expectedError, errors);
    }

    [Fact]
    public void TryCreateOptions_WhenBaselineValueMissing_ReturnsSystemCommandLineError()
    {
        bool parsed = WinUiBenchmarkCliRunner.TryCreateOptions(
            ["--benchmark", "--baseline-json"],
            out _,
            out IReadOnlyList<string> errors);

        Assert.False(parsed);
        Assert.Single(errors);
        Assert.Contains("--baseline-json", errors[0], StringComparison.OrdinalIgnoreCase);
    }

    [Fact]
    public void TryCreateOptions_WhenUnknownArgumentProvided_ReturnsParseError()
    {
        bool parsed = WinUiBenchmarkCliRunner.TryCreateOptions(
            ["--benchmark", "--unknown-option"],
            out _,
            out IReadOnlyList<string> errors);

        Assert.False(parsed);
        Assert.Single(errors);
        Assert.Contains("--unknown-option", errors[0], StringComparison.OrdinalIgnoreCase);
    }
}
