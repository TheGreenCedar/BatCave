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
    public void ValidateWinUi_DefaultsToX64BuildPlatform_AndUsesHostCompatibleRunPlatformForDiagnostics()
    {
        using PowerShellScriptHarness harness = PowerShellScriptHarness.Create();
        harness.EnableRuntimeDiagnostics();

        ScriptRunResult result = harness.Run("validate-winui.ps1", "-SkipLaunchSmoke");

        Assert.Equal(0, result.ExitCode);
        Assert.Contains("Platform=x64", result.StandardOutput, StringComparison.OrdinalIgnoreCase);
        Assert.Contains("RunPlatform=x64", result.StandardOutput, StringComparison.OrdinalIgnoreCase);

        Assert.Contains(result.DotnetInvocations, invocation => invocation.StartsWith("build ", StringComparison.OrdinalIgnoreCase));
        Assert.Contains(result.DotnetInvocations, invocation => invocation.StartsWith("test ", StringComparison.OrdinalIgnoreCase));

        string gateInvocation = Assert.Single(result.DotnetInvocations, invocation => invocation.Contains("--print-gate-status", StringComparison.OrdinalIgnoreCase));
        Assert.Contains("-p:Platform=x64", gateInvocation, StringComparison.OrdinalIgnoreCase);
        Assert.Contains("-p:WindowsPackageType=None", gateInvocation, StringComparison.OrdinalIgnoreCase);
        Assert.Contains("-p:GenerateAppxPackageOnBuild=false", gateInvocation, StringComparison.OrdinalIgnoreCase);
        Assert.Contains("-p:WindowsAppSdkBootstrapInitialize=true", gateInvocation, StringComparison.OrdinalIgnoreCase);
        Assert.Contains("-p:WindowsAppSdkDeploymentManagerInitialize=false", gateInvocation, StringComparison.OrdinalIgnoreCase);

        string runtimeHealthInvocation = Assert.Single(result.DotnetInvocations, invocation => invocation.Contains("--print-runtime-health", StringComparison.OrdinalIgnoreCase));
        Assert.Contains("-p:Platform=x64", runtimeHealthInvocation, StringComparison.OrdinalIgnoreCase);
        Assert.Contains("-p:WindowsPackageType=None", runtimeHealthInvocation, StringComparison.OrdinalIgnoreCase);
        Assert.Contains("-p:GenerateAppxPackageOnBuild=false", runtimeHealthInvocation, StringComparison.OrdinalIgnoreCase);
        Assert.Contains("-p:WindowsAppSdkBootstrapInitialize=true", runtimeHealthInvocation, StringComparison.OrdinalIgnoreCase);
        Assert.Contains("-p:WindowsAppSdkDeploymentManagerInitialize=false", runtimeHealthInvocation, StringComparison.OrdinalIgnoreCase);
    }

    [Fact]
    public void ValidateWinUi_RunPlatform_OverridesRuntimeDiagnosticsPlatform()
    {
        using PowerShellScriptHarness harness = PowerShellScriptHarness.Create();
        harness.EnableRuntimeDiagnostics();

        ScriptRunResult result = harness.Run(
            "validate-winui.ps1",
            "-Platform", "ARM64",
            "-RunPlatform", "x64",
            "-SkipLaunchSmoke");

        Assert.Equal(0, result.ExitCode);
        Assert.Contains("Platform=ARM64", result.StandardOutput, StringComparison.OrdinalIgnoreCase);
        Assert.Contains("RunPlatform=x64", result.StandardOutput, StringComparison.OrdinalIgnoreCase);

        Assert.Contains(result.DotnetInvocations, invocation => invocation.StartsWith("build ", StringComparison.OrdinalIgnoreCase));

        string gateInvocation = Assert.Single(result.DotnetInvocations, invocation => invocation.Contains("--print-gate-status", StringComparison.OrdinalIgnoreCase));
        Assert.Contains("-p:Platform=x64", gateInvocation, StringComparison.OrdinalIgnoreCase);
        Assert.DoesNotContain("-p:Platform=ARM64", gateInvocation, StringComparison.OrdinalIgnoreCase);

        string runtimeHealthInvocation = Assert.Single(result.DotnetInvocations, invocation => invocation.Contains("--print-runtime-health", StringComparison.OrdinalIgnoreCase));
        Assert.Contains("-p:Platform=x64", runtimeHealthInvocation, StringComparison.OrdinalIgnoreCase);
        Assert.DoesNotContain("-p:Platform=ARM64", runtimeHealthInvocation, StringComparison.OrdinalIgnoreCase);
    }

    [Fact]
    public void RunDev_UsesUnpackagedWinUiRunArguments_AndForwardsAppArgs()
    {
        using PowerShellScriptHarness harness = PowerShellScriptHarness.Create();

        ScriptRunResult result = harness.Run(
            "run-dev.ps1",
            "-NoBuild",
            "-Platform", "x64",
            "--print-runtime-health");

        Assert.Equal(0, result.ExitCode);
        string invocation = Assert.Single(result.DotnetInvocations);
        Assert.StartsWith("run ", invocation, StringComparison.OrdinalIgnoreCase);
        Assert.Contains("-p:Platform=x64", invocation, StringComparison.OrdinalIgnoreCase);
        Assert.Contains("-p:WindowsPackageType=None", invocation, StringComparison.OrdinalIgnoreCase);
        Assert.Contains("-p:GenerateAppxPackageOnBuild=false", invocation, StringComparison.OrdinalIgnoreCase);
        Assert.Contains("-p:WindowsAppSdkBootstrapInitialize=true", invocation, StringComparison.OrdinalIgnoreCase);
        Assert.Contains("-p:WindowsAppSdkDeploymentManagerInitialize=false", invocation, StringComparison.OrdinalIgnoreCase);
        Assert.Contains("-- --print-runtime-health", invocation, StringComparison.OrdinalIgnoreCase);
        Assert.Contains("--print-runtime-health", invocation, StringComparison.OrdinalIgnoreCase);
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
        Assert.Contains("-p:WindowsPackageType=None", invocation, StringComparison.OrdinalIgnoreCase);
        Assert.Contains("-p:GenerateAppxPackageOnBuild=false", invocation, StringComparison.OrdinalIgnoreCase);
        Assert.Contains("-p:WindowsAppSdkBootstrapInitialize=true", invocation, StringComparison.OrdinalIgnoreCase);
        Assert.Contains("-p:WindowsAppSdkDeploymentManagerInitialize=false", invocation, StringComparison.OrdinalIgnoreCase);
        Assert.Contains("--benchmark", invocation, StringComparison.OrdinalIgnoreCase);
        Assert.Contains("--ticks 9", invocation, StringComparison.OrdinalIgnoreCase);
        Assert.Contains("--sleep-ms 15", invocation, StringComparison.OrdinalIgnoreCase);
        Assert.Contains("--strict", invocation, StringComparison.OrdinalIgnoreCase);
    }

    [Fact]
    public void RunDev_DefaultLaunch_UsesUnpackagedWinUiRunArguments()
    {
        using PowerShellScriptHarness harness = PowerShellScriptHarness.Create();

        ScriptRunResult result = harness.Run("run-dev.ps1", "-NoBuild");

        Assert.Equal(0, result.ExitCode);
        string invocation = Assert.Single(result.DotnetInvocations, item => item.StartsWith("run ", StringComparison.OrdinalIgnoreCase));
        Assert.Contains("BatCave.csproj", invocation, StringComparison.OrdinalIgnoreCase);
        Assert.Contains("-p:Platform=x64", invocation, StringComparison.OrdinalIgnoreCase);
        Assert.Contains("-p:WindowsPackageType=None", invocation, StringComparison.OrdinalIgnoreCase);
        Assert.Contains("-p:GenerateAppxPackageOnBuild=false", invocation, StringComparison.OrdinalIgnoreCase);
        Assert.Contains("-p:WindowsAppSdkBootstrapInitialize=true", invocation, StringComparison.OrdinalIgnoreCase);
        Assert.Contains("-p:WindowsAppSdkDeploymentManagerInitialize=false", invocation, StringComparison.OrdinalIgnoreCase);
    }

    [Fact]
    public void RunBenchmark_Core_ForwardsBaselineCompareArgs()
    {
        using PowerShellScriptHarness harness = PowerShellScriptHarness.Create();
        harness.SetRunExitCode(3);

        ScriptRunResult result = harness.Run(
            "run-benchmark.ps1",
            "-BenchmarkHost", "core",
            "-Ticks", "10",
            "-SleepMs", "20",
            "-BaselineJsonPath", "C:/tmp/baseline.json",
            "-MinSpeedupMultiplier", "1.3",
            "-MaxP95Ms", "12.5",
            "-NoBuild");

        Assert.Equal(3, result.ExitCode);
        string invocation = Assert.Single(result.DotnetInvocations);
        Assert.Contains("--baseline-json C:/tmp/baseline.json", invocation, StringComparison.OrdinalIgnoreCase);
        Assert.Contains("--min-speedup-multiplier 1.3", invocation, StringComparison.OrdinalIgnoreCase);
        Assert.Contains("--max-p95-ms 12.5", invocation, StringComparison.OrdinalIgnoreCase);
    }

    [Fact]
    public void RunBenchmark_CoreStrict_WithBaselineAndNoMinSpeedup_UsesDefaultTenXGate()
    {
        using PowerShellScriptHarness harness = PowerShellScriptHarness.Create();
        harness.SetRunExitCode(0);

        ScriptRunResult result = harness.Run(
            "run-benchmark.ps1",
            "-BenchmarkHost", "core",
            "-Ticks", "10",
            "-SleepMs", "20",
            "-BaselineJsonPath", "C:/tmp/baseline.json",
            "-Strict",
            "-NoBuild");

        Assert.Equal(0, result.ExitCode);
        string invocation = Assert.Single(result.DotnetInvocations);
        Assert.Contains("--baseline-json C:/tmp/baseline.json", invocation, StringComparison.OrdinalIgnoreCase);
        Assert.Contains("--min-speedup-multiplier 10", invocation, StringComparison.OrdinalIgnoreCase);
    }

    [Fact]
    public async Task RunBenchmark_WithBaselineArtifact_ForwardsResolvedBaselineSummaryPath()
    {
        using PowerShellScriptHarness harness = PowerShellScriptHarness.Create();
        harness.SetRunExitCode(0);

        string summaryPath = Path.GetTempFileName();
        string artifactPath = Path.GetTempFileName();
        try
        {
            await File.WriteAllTextAsync(summaryPath, """{"tick_p95_ms":1.0}""");
            string escapedSummaryPath = summaryPath.Replace("\\", "\\\\", StringComparison.Ordinal);
            await File.WriteAllTextAsync(
                artifactPath,
                $$"""
                {
                  "host": "core",
                  "platform": "x64",
                  "measured_ticks": 10,
                  "sleep_ms": 20,
                  "baseline_summary_path": "{{escapedSummaryPath}}"
                }
                """);

            ScriptRunResult result = harness.Run(
                "run-benchmark.ps1",
                "-BenchmarkHost", "core",
                "-Platform", "x64",
                "-Ticks", "10",
                "-SleepMs", "20",
                "-BaselineArtifactPath", artifactPath,
                "-NoBuild");

            Assert.Equal(0, result.ExitCode);
            string invocation = Assert.Single(result.DotnetInvocations);
            Assert.Contains("--baseline-json", invocation, StringComparison.OrdinalIgnoreCase);
            Assert.Contains(summaryPath, invocation, StringComparison.OrdinalIgnoreCase);
        }
        finally
        {
            File.Delete(summaryPath);
            File.Delete(artifactPath);
        }
    }

    [Fact]
    public async Task RunBenchmark_WithBaselineArtifactHostMismatch_ExitsNonZeroWithoutRun()
    {
        using PowerShellScriptHarness harness = PowerShellScriptHarness.Create();
        harness.SetRunExitCode(0);

        string artifactPath = Path.GetTempFileName();
        try
        {
            await File.WriteAllTextAsync(
                artifactPath,
                """
                {
                  "host": "winui",
                  "platform": "x64",
                  "measured_ticks": 10,
                  "sleep_ms": 20
                }
                """);

            ScriptRunResult result = harness.Run(
                "run-benchmark.ps1",
                "-BenchmarkHost", "core",
                "-Platform", "x64",
                "-Ticks", "10",
                "-SleepMs", "20",
                "-BaselineArtifactPath", artifactPath,
                "-NoBuild");

            Assert.NotEqual(0, result.ExitCode);
            Assert.Empty(result.DotnetInvocations);
            Assert.Contains("host mismatch", result.StandardError, StringComparison.OrdinalIgnoreCase);
        }
        finally
        {
            File.Delete(artifactPath);
        }
    }

    [Fact]
    public void ValidateWinUi_RunPerformanceGate_ForwardsStrictBenchmarkComparisonArgs()
    {
        using PowerShellScriptHarness harness = PowerShellScriptHarness.Create();
        harness.SetRunExitCode(0);

        ScriptRunResult result = harness.Run(
            "validate-winui.ps1",
            "-Platform", "ARM64",
            "-RunPerformanceGate",
            "-BenchmarkHost", "core",
            "-Ticks", "8",
            "-SleepMs", "12",
            "-BaselineJsonPath", "C:/tmp/baseline.summary.json",
            "-MinSpeedupMultiplier", "10");

        Assert.Equal(0, result.ExitCode);
        Assert.Contains(result.DotnetInvocations, invocation => invocation.StartsWith("build ", StringComparison.OrdinalIgnoreCase));
        Assert.Contains(result.DotnetInvocations, invocation => invocation.StartsWith("test ", StringComparison.OrdinalIgnoreCase));

        string benchmarkInvocation = Assert.Single(result.DotnetInvocations, invocation => invocation.Contains("--strict", StringComparison.OrdinalIgnoreCase));
        Assert.Contains("--baseline-json C:/tmp/baseline.summary.json", benchmarkInvocation, StringComparison.OrdinalIgnoreCase);
        Assert.Contains("--min-speedup-multiplier 10", benchmarkInvocation, StringComparison.OrdinalIgnoreCase);

        string gateInvocation = Assert.Single(result.DotnetInvocations, invocation => invocation.Contains("--print-gate-status", StringComparison.OrdinalIgnoreCase));
        Assert.Contains("-p:Platform=x64", gateInvocation, StringComparison.OrdinalIgnoreCase);
    }

    [Fact]
    public async Task ValidateWinUi_RunPerformanceGate_WithBaselineArtifact_ForwardsResolvedSummaryPath()
    {
        using PowerShellScriptHarness harness = PowerShellScriptHarness.Create();
        harness.SetRunExitCode(0);

        string summaryPath = Path.GetTempFileName();
        string artifactPath = Path.GetTempFileName();
        try
        {
            await File.WriteAllTextAsync(summaryPath, """{"tick_p95_ms":1.0}""");
            string escapedSummaryPath = summaryPath.Replace("\\", "\\\\", StringComparison.Ordinal);
            await File.WriteAllTextAsync(
                artifactPath,
                $$"""
                {
                  "host": "core",
                  "platform": "ARM64",
                  "measured_ticks": 8,
                  "sleep_ms": 12,
                  "baseline_summary_path": "{{escapedSummaryPath}}"
                }
                """);

            ScriptRunResult result = harness.Run(
                "validate-winui.ps1",
                "-Platform", "ARM64",
                "-RunPerformanceGate",
                "-BenchmarkHost", "core",
                "-Ticks", "8",
                "-SleepMs", "12",
                "-BaselineArtifactPath", artifactPath,
                "-MinSpeedupMultiplier", "10");

            Assert.Equal(0, result.ExitCode);
            Assert.Contains(result.DotnetInvocations, invocation => invocation.StartsWith("build ", StringComparison.OrdinalIgnoreCase));
            Assert.Contains(result.DotnetInvocations, invocation => invocation.StartsWith("test ", StringComparison.OrdinalIgnoreCase));

            string benchmarkInvocation = Assert.Single(result.DotnetInvocations, invocation => invocation.Contains("--strict", StringComparison.OrdinalIgnoreCase));
            Assert.Contains("--baseline-json", benchmarkInvocation, StringComparison.OrdinalIgnoreCase);
            Assert.Contains(summaryPath, benchmarkInvocation, StringComparison.OrdinalIgnoreCase);
            Assert.Contains("--min-speedup-multiplier 10", benchmarkInvocation, StringComparison.OrdinalIgnoreCase);

            string gateInvocation = Assert.Single(result.DotnetInvocations, invocation => invocation.Contains("--print-gate-status", StringComparison.OrdinalIgnoreCase));
            Assert.Contains("-p:Platform=x64", gateInvocation, StringComparison.OrdinalIgnoreCase);
        }
        finally
        {
            File.Delete(summaryPath);
            File.Delete(artifactPath);
        }
    }
}
