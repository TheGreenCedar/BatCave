using BatCave.Core.Tests.TestSupport;

namespace BatCave.Core.Tests.Scripts;

public class ScriptRegressionTests
{
    [Fact]
    public void ValidateWinUi_InvalidPlatform_ExitsNonZeroAndSkipsDotnetTest()
    {
        using PowerShellScriptHarness harness = PowerShellScriptHarness.Create();
        harness.SetBuildExitCode(87);

        ScriptRunResult result = harness.Run("validate-winui.ps1", "-Platform", "invalid");

        Assert.NotEqual(0, result.ExitCode);
        Assert.Contains("Platform=invalid", result.StandardOutput, StringComparison.OrdinalIgnoreCase);
        Assert.NotEmpty(result.DotnetInvocations);
        Assert.StartsWith("build ", result.DotnetInvocations[0], StringComparison.OrdinalIgnoreCase);
        Assert.DoesNotContain(
            result.DotnetInvocations,
            invocation => invocation.StartsWith("test ", StringComparison.OrdinalIgnoreCase));
    }

    [Fact]
    public void RunBenchmark_CoreStrict_ForwardsArgsAndPropagatesExitCode()
    {
        using PowerShellScriptHarness harness = PowerShellScriptHarness.Create();
        harness.SetRunExitCode(23);

        ScriptRunResult result = harness.Run(
            "run-benchmark.ps1",
            "-BenchmarkHost", "core",
            "-Ticks", "7",
            "-SleepMs", "11",
            "-Strict",
            "-NoBuild");

        Assert.Equal(23, result.ExitCode);
        string invocation = Assert.Single(result.DotnetInvocations);
        Assert.StartsWith("run ", invocation, StringComparison.OrdinalIgnoreCase);
        Assert.Contains("BatCave.Bench.csproj", invocation, StringComparison.OrdinalIgnoreCase);
        Assert.Contains("--ticks 7", invocation, StringComparison.OrdinalIgnoreCase);
        Assert.Contains("--sleep-ms 11", invocation, StringComparison.OrdinalIgnoreCase);
        Assert.Contains("--strict", invocation, StringComparison.OrdinalIgnoreCase);
        Assert.DoesNotContain("--benchmark", invocation, StringComparison.OrdinalIgnoreCase);
    }

    [Fact]
    public void RunBenchmark_WinUiStrict_ForwardsArgsAndPropagatesExitCode()
    {
        using PowerShellScriptHarness harness = PowerShellScriptHarness.Create();
        harness.SetRunExitCode(41);

        ScriptRunResult result = harness.Run(
            "run-benchmark.ps1",
            "-BenchmarkHost", "winui",
            "-Platform", "x64",
            "-Ticks", "9",
            "-SleepMs", "15",
            "-Strict",
            "-NoBuild");

        Assert.Equal(41, result.ExitCode);
        string invocation = Assert.Single(result.DotnetInvocations);
        Assert.StartsWith("run ", invocation, StringComparison.OrdinalIgnoreCase);
        Assert.Contains("BatCave.csproj", invocation, StringComparison.OrdinalIgnoreCase);
        Assert.Contains("-p:Platform=x64", invocation, StringComparison.OrdinalIgnoreCase);
        Assert.Contains("--benchmark", invocation, StringComparison.OrdinalIgnoreCase);
        Assert.Contains("--ticks 9", invocation, StringComparison.OrdinalIgnoreCase);
        Assert.Contains("--sleep-ms 15", invocation, StringComparison.OrdinalIgnoreCase);
        Assert.Contains("--strict", invocation, StringComparison.OrdinalIgnoreCase);
    }
}
