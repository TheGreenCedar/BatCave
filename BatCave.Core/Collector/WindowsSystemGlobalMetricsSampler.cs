using System.Runtime.InteropServices;
using BatCave.Core.Abstractions;
using BatCave.Core.Domain;

namespace BatCave.Core.Collector;

public sealed partial class WindowsSystemGlobalMetricsSampler : ISystemGlobalMetricsSampler, IDisposable
{
    private const uint ErrorSuccess = 0;
    private const uint PdhFmtDouble = 0x00000200;
    private const uint PdhCstatusValidData = 0x00000000;
    private const uint PdhCstatusNewData = 0x00000001;

    private const string DiskReadCounterPath = @"\\PhysicalDisk(_Total)\\Disk Read Bytes/sec";
    private const string DiskWriteCounterPath = @"\\PhysicalDisk(_Total)\\Disk Write Bytes/sec";
    private const string OtherIoCounterPath = @"\\Process(_Total)\\IO Other Bytes/sec";

    private readonly object _sync = new();
    private IntPtr _pdhQuery;
    private IntPtr _diskReadCounter;
    private IntPtr _diskWriteCounter;
    private IntPtr _otherIoCounter;
    private bool _rateCountersWarmed;
    private bool _disposed;
    private ulong? _previousSystemTotal100ns;
    private ulong? _previousIdle100ns;
    private ulong? _previousKernel100ns;
    private double? _lastKernelPct;

    public WindowsSystemGlobalMetricsSampler()
    {
        InitializePdhCounters();
    }

    public SystemGlobalMetricsSample Sample()
    {
        lock (_sync)
        {
            ThrowIfDisposed();
            (ulong? diskReadBps, ulong? diskWriteBps, ulong? otherIoBps) = SampleRateMetrics();
            double? cpuPct = SampleCpuPct();
            ulong? memoryUsedBytes = SampleMemoryUsedBytes();
            (SystemGlobalCpuSnapshot? cpuSnapshot, SystemGlobalMemorySnapshot? memorySnapshot, IReadOnlyList<SystemGlobalDiskSnapshot> diskSnapshots, IReadOnlyList<SystemGlobalNetworkSnapshot> networkSnapshots) =
                SampleExtendedSnapshots(cpuPct, memoryUsedBytes);

            return new SystemGlobalMetricsSample
            {
                TsMs = (ulong)DateTimeOffset.UtcNow.ToUnixTimeMilliseconds(),
                CpuPct = cpuPct,
                MemoryUsedBytes = memoryUsedBytes,
                DiskReadBps = diskReadBps,
                DiskWriteBps = diskWriteBps,
                OtherIoBps = otherIoBps,
                CpuSnapshot = cpuSnapshot,
                MemorySnapshot = memorySnapshot,
                DiskSnapshots = diskSnapshots,
                NetworkSnapshots = networkSnapshots,
            };
        }
    }

    public void Dispose()
    {
        lock (_sync)
        {
            if (_disposed)
            {
                return;
            }

            _disposed = true;
            ClosePdhCounters();
            _previousSystemTotal100ns = null;
            _previousIdle100ns = null;
            _previousKernel100ns = null;
            _lastKernelPct = null;
        }

        GC.SuppressFinalize(this);
    }

    private void ThrowIfDisposed()
    {
        ObjectDisposedException.ThrowIf(_disposed, this);
    }

    private void InitializePdhCounters()
    {
        if (PdhOpenQueryW(dataSource: null, userData: IntPtr.Zero, out _pdhQuery) != ErrorSuccess)
        {
            _pdhQuery = IntPtr.Zero;
            return;
        }

        _diskReadCounter = AddEnglishCounter(DiskReadCounterPath);
        _diskWriteCounter = AddEnglishCounter(DiskWriteCounterPath);
        _otherIoCounter = AddEnglishCounter(OtherIoCounterPath);

        if (_diskReadCounter == IntPtr.Zero && _diskWriteCounter == IntPtr.Zero && _otherIoCounter == IntPtr.Zero)
        {
            ClosePdhCounters();
        }
    }

    private IntPtr AddEnglishCounter(string path)
    {
        if (_pdhQuery == IntPtr.Zero)
        {
            return IntPtr.Zero;
        }

        return PdhAddEnglishCounterW(_pdhQuery, path, userData: IntPtr.Zero, out IntPtr counter) == ErrorSuccess
            ? counter
            : IntPtr.Zero;
    }

    private double? SampleCpuPct()
    {
        if (!GetSystemTimes(out FILETIME idleTime, out FILETIME kernelTime, out FILETIME userTime))
        {
            return null;
        }

        ulong currentIdle100ns = FiletimeToU64(idleTime);
        ulong currentSystemTotal100ns = FiletimeToU64(kernelTime) + FiletimeToU64(userTime);

        if (_previousSystemTotal100ns is not ulong previousSystemTotal100ns || _previousIdle100ns is not ulong previousIdle100ns)
        {
            _previousSystemTotal100ns = currentSystemTotal100ns;
            _previousIdle100ns = currentIdle100ns;
            _previousKernel100ns = FiletimeToU64(kernelTime);
            _lastKernelPct = null;
            return null;
        }

        ulong deltaSystem100ns = currentSystemTotal100ns >= previousSystemTotal100ns
            ? currentSystemTotal100ns - previousSystemTotal100ns
            : 0;
        ulong deltaIdle100ns = currentIdle100ns >= previousIdle100ns ? currentIdle100ns - previousIdle100ns : 0;
        ulong currentKernel100ns = FiletimeToU64(kernelTime);
        ulong previousKernel100ns = _previousKernel100ns ?? currentKernel100ns;
        ulong deltaKernel100ns = currentKernel100ns >= previousKernel100ns ? currentKernel100ns - previousKernel100ns : 0;

        _previousSystemTotal100ns = currentSystemTotal100ns;
        _previousIdle100ns = currentIdle100ns;
        _previousKernel100ns = currentKernel100ns;

        if (deltaSystem100ns == 0)
        {
            _lastKernelPct = null;
            return null;
        }

        ulong busy100ns = deltaSystem100ns > deltaIdle100ns ? deltaSystem100ns - deltaIdle100ns : 0;
        ulong kernelBusy100ns = deltaKernel100ns > deltaIdle100ns ? deltaKernel100ns - deltaIdle100ns : 0;
        double cpuPct = (busy100ns * 100.0) / deltaSystem100ns;
        _lastKernelPct = (kernelBusy100ns * 100.0) / deltaSystem100ns;
        return Math.Clamp(cpuPct, 0, 100);
    }

    private static ulong? SampleMemoryUsedBytes()
    {
        MEMORYSTATUSEX memoryStatus = new()
        {
            dwLength = (uint)Marshal.SizeOf<MEMORYSTATUSEX>(),
        };

        if (!GlobalMemoryStatusEx(ref memoryStatus))
        {
            return null;
        }

        return memoryStatus.ullTotalPhys >= memoryStatus.ullAvailPhys
            ? memoryStatus.ullTotalPhys - memoryStatus.ullAvailPhys
            : 0;
    }

    private (ulong? DiskReadBps, ulong? DiskWriteBps, ulong? OtherIoBps) SampleRateMetrics()
    {
        if (_pdhQuery == IntPtr.Zero)
        {
            return (null, null, null);
        }

        if (PdhCollectQueryData(_pdhQuery) != ErrorSuccess)
        {
            return (null, null, null);
        }

        if (!_rateCountersWarmed)
        {
            _rateCountersWarmed = true;
            return (null, null, null);
        }

        return (
            ReadRateCounter(_diskReadCounter),
            ReadRateCounter(_diskWriteCounter),
            ReadRateCounter(_otherIoCounter));
    }

    private static ulong? ReadRateCounter(IntPtr counter)
    {
        if (counter == IntPtr.Zero)
        {
            return null;
        }

        if (PdhGetFormattedCounterValue(counter, PdhFmtDouble, out _, out PDH_FMT_COUNTERVALUE_DOUBLE value) != ErrorSuccess)
        {
            return null;
        }

        if (value.CStatus != PdhCstatusValidData && value.CStatus != PdhCstatusNewData)
        {
            return null;
        }

        if (double.IsNaN(value.DoubleValue) || double.IsInfinity(value.DoubleValue) || value.DoubleValue < 0)
        {
            return null;
        }

        if (value.DoubleValue > ulong.MaxValue)
        {
            return ulong.MaxValue;
        }

        return (ulong)value.DoubleValue;
    }

    private void ClosePdhCounters()
    {
        if (_pdhQuery != IntPtr.Zero)
        {
            _ = PdhCloseQuery(_pdhQuery);
        }

        _pdhQuery = IntPtr.Zero;
        _diskReadCounter = IntPtr.Zero;
        _diskWriteCounter = IntPtr.Zero;
        _otherIoCounter = IntPtr.Zero;
        _rateCountersWarmed = false;
    }

    private static ulong FiletimeToU64(FILETIME filetime)
    {
        return ((ulong)filetime.dwHighDateTime << 32) | filetime.dwLowDateTime;
    }

    [StructLayout(LayoutKind.Sequential)]
    private struct FILETIME
    {
        public uint dwLowDateTime;
        public uint dwHighDateTime;
    }

    [StructLayout(LayoutKind.Sequential)]
    private struct MEMORYSTATUSEX
    {
        public uint dwLength;
        public uint dwMemoryLoad;
        public ulong ullTotalPhys;
        public ulong ullAvailPhys;
        public ulong ullTotalPageFile;
        public ulong ullAvailPageFile;
        public ulong ullTotalVirtual;
        public ulong ullAvailVirtual;
        public ulong ullAvailExtendedVirtual;
    }

    [StructLayout(LayoutKind.Sequential)]
    private struct PDH_FMT_COUNTERVALUE_DOUBLE
    {
        public uint CStatus;
        public double DoubleValue;
    }

    [DllImport("kernel32.dll", SetLastError = true)]
    [return: MarshalAs(UnmanagedType.Bool)]
    private static extern bool GetSystemTimes(out FILETIME idleTime, out FILETIME kernelTime, out FILETIME userTime);

    [DllImport("kernel32.dll", SetLastError = true)]
    [return: MarshalAs(UnmanagedType.Bool)]
    private static extern bool GlobalMemoryStatusEx(ref MEMORYSTATUSEX buffer);

    [DllImport("pdh.dll", CharSet = CharSet.Unicode)]
    private static extern uint PdhOpenQueryW(string? dataSource, IntPtr userData, out IntPtr query);

    [DllImport("pdh.dll", CharSet = CharSet.Unicode)]
    private static extern uint PdhAddEnglishCounterW(IntPtr query, string fullCounterPath, IntPtr userData, out IntPtr counter);

    [DllImport("pdh.dll")]
    private static extern uint PdhCollectQueryData(IntPtr query);

    [DllImport("pdh.dll")]
    private static extern uint PdhGetFormattedCounterValue(
        IntPtr counter,
        uint format,
        out uint type,
        out PDH_FMT_COUNTERVALUE_DOUBLE value);

    [DllImport("pdh.dll")]
    private static extern uint PdhCloseQuery(IntPtr query);
}
