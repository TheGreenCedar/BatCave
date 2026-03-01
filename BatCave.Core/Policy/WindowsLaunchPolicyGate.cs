using System;
using System.Runtime.InteropServices;
using BatCave.Core.Abstractions;
using BatCave.Core.Domain;

namespace BatCave.Core.Policy;

public sealed class WindowsLaunchPolicyGate : ILaunchPolicyGate
{
    private const uint Windows11Build = 22000;

    public StartupGateStatus Enforce()
    {
        if (!OperatingSystem.IsWindows())
        {
            return StartupGateStatus.Blocked(
                LaunchBlockReason.UnsupportedPlatform(Environment.OSVersion.Platform.ToString().ToLowerInvariant()));
        }

        if (!TryGetWindowsBuild(out uint build))
        {
            return StartupGateStatus.Blocked(LaunchBlockReason.UnsupportedPlatform("windows"));
        }

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

    private static bool TryGetWindowsBuild(out uint build)
    {
        build = 0;
        if (!OperatingSystem.IsWindows())
        {
            return false;
        }

        RTL_OSVERSIONINFOW info = new RTL_OSVERSIONINFOW
        {
            dwOSVersionInfoSize = (uint)Marshal.SizeOf<RTL_OSVERSIONINFOW>(),
        };

        int status = RtlGetVersion(ref info);
        if (status != 0)
        {
            return false;
        }

        build = info.dwBuildNumber;
        return true;
    }

    [StructLayout(LayoutKind.Sequential, CharSet = CharSet.Unicode)]
    private struct RTL_OSVERSIONINFOW
    {
        public uint dwOSVersionInfoSize;
        public uint dwMajorVersion;
        public uint dwMinorVersion;
        public uint dwBuildNumber;
        public uint dwPlatformId;
        [MarshalAs(UnmanagedType.ByValTStr, SizeConst = 128)]
        public string szCSDVersion;
    }

    [DllImport("ntdll.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    private static extern int RtlGetVersion(ref RTL_OSVERSIONINFOW versionInfo);
}
