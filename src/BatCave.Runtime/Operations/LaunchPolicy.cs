using BatCave.Runtime.Contracts;
using System.Runtime.InteropServices;

namespace BatCave.Runtime.Operations;

public interface ILaunchPolicyGate
{
    StartupGateStatus Enforce();
}

public sealed class WindowsLaunchPolicyGate : ILaunchPolicyGate
{
    private const uint Windows11Build = 22000;

    public StartupGateStatus Enforce()
    {
        if (!RuntimeInformation.IsOSPlatform(OSPlatform.Windows))
        {
            return StartupGateStatus.Blocked(LaunchBlockReason.UnsupportedPlatform(RuntimeInformation.OSDescription));
        }

        Version version = Environment.OSVersion.Version;
        uint build = version.Build > 0 ? (uint)version.Build : 0;
        if (build < Windows11Build)
        {
            return StartupGateStatus.Blocked(LaunchBlockReason.RequiresWindows11(build));
        }

        return StartupGateStatus.PassedContext(new LaunchContext
        {
            Os = "windows",
            WindowsBuild = build,
        });
    }
}
