using BatCave.Core.Domain;
using BatCave.Core.Policy;

namespace BatCave.Core.Tests.Policy;

public class WindowsLaunchPolicyGateTests
{
    [Fact]
    public void Enforce_ReturnsConsistentGateShapeForCurrentHost()
    {
        WindowsLaunchPolicyGate gate = new();
        StartupGateStatus status = gate.Enforce();

        if (status.Passed)
        {
            Assert.NotNull(status.Context);
            Assert.Equal("windows", status.Context!.Os);
            Assert.True(status.Context.WindowsBuild >= 22000);
            Assert.Null(status.Reason);
            return;
        }

        Assert.NotNull(status.Reason);
        Assert.True(
            status.Reason!.Kind == LaunchBlockReasonKind.UnsupportedPlatform
            || status.Reason.Kind == LaunchBlockReasonKind.RequiresWindows11);
    }
}
