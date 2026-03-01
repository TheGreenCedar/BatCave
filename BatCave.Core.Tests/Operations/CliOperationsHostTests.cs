using BatCave.Core.Abstractions;
using BatCave.Core.Domain;
using BatCave.Core.Operations;

namespace BatCave.Core.Tests.Operations;

public class CliOperationsHostTests
{
    [Fact]
    public async Task ExecuteAsync_PrintGateStatus_WritesJsonAndReturnsSuccess()
    {
        CliOperationsHost host = new(new FixedLaunchPolicyGate(
            StartupGateStatus.PassedContext(new LaunchContext
            {
                Os = "windows",
                WindowsBuild = 26000,
            })));

        using StringWriter output = new();
        TextWriter original = Console.Out;
        Console.SetOut(output);
        try
        {
            int exitCode = await host.ExecuteAsync(["--print-gate-status"], CancellationToken.None);
            Assert.Equal(0, exitCode);
            Assert.Contains("\"passed\": true", output.ToString(), StringComparison.OrdinalIgnoreCase);
        }
        finally
        {
            Console.SetOut(original);
        }
    }

    [Fact]
    public async Task ExecuteAsync_Benchmark_ParsesOptionsAndCompletes()
    {
        CliOperationsHost host = new(new FixedLaunchPolicyGate(
            StartupGateStatus.PassedContext(new LaunchContext
            {
                Os = "windows",
                WindowsBuild = 26000,
            })));

        int exitCode = await host.ExecuteAsync(
            ["--benchmark", "--ticks", "0", "--sleep-ms", "0", "--strict"],
            CancellationToken.None);

        Assert.Equal(0, exitCode);
    }

    [Fact]
    public async Task ExecuteAsync_ElevatedHelperMissingArgs_ReturnsFailure()
    {
        CliOperationsHost host = new(new FixedLaunchPolicyGate(
            StartupGateStatus.PassedContext(new LaunchContext
            {
                Os = "windows",
                WindowsBuild = 26000,
            })));

        int exitCode = await host.ExecuteAsync(["--elevated-helper"], CancellationToken.None);

        Assert.Equal(2, exitCode);
    }

    [Fact]
    public void IsCliMode_RecognizesKnownFlags()
    {
        CliOperationsHost host = new(new FixedLaunchPolicyGate(
            StartupGateStatus.PassedContext(new LaunchContext
            {
                Os = "windows",
                WindowsBuild = 26000,
            })));

        Assert.True(host.IsCliMode(["--benchmark"]));
        Assert.True(host.IsCliMode(["--print-gate-status"]));
        Assert.True(host.IsCliMode(["--elevated-helper"]));
        Assert.False(host.IsCliMode(["--unknown-flag"]));
    }

    private sealed class FixedLaunchPolicyGate : ILaunchPolicyGate
    {
        private readonly StartupGateStatus _status;

        public FixedLaunchPolicyGate(StartupGateStatus status)
        {
            _status = status;
        }

        public StartupGateStatus Enforce()
        {
            return _status;
        }
    }
}
