using BatCave.Core.Abstractions;
using BatCave.Core.Domain;

namespace BatCave.Core.Policy;

public sealed class WindowsLaunchPolicyGate : ILaunchPolicyGate
{
    private const int Windows11Build = 22000;

    public StartupGateStatus Enforce()
    {
        if (!OperatingSystem.IsWindows())
        {
            return StartupGateStatus.Blocked(
                LaunchBlockReason.UnsupportedPlatform(Environment.OSVersion.Platform.ToString().ToLowerInvariant()));
        }

        int build = Environment.OSVersion.Version.Build;
        if (build <= 0)
        {
            return StartupGateStatus.Blocked(LaunchBlockReason.UnsupportedPlatform("windows"));
        }

        if (!OperatingSystem.IsWindowsVersionAtLeast(10, 0, Windows11Build))
        {
            return StartupGateStatus.Blocked(LaunchBlockReason.RequiresWindows11((uint)build));
        }

        return StartupGateStatus.PassedContext(new LaunchContext
        {
            Os = "windows",
            WindowsBuild = (uint)build,
        });
    }
}
