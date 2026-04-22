using BatCave.Runtime.Contracts;
using Microsoft.Extensions.Logging;
using Microsoft.Extensions.Logging.Abstractions;
using System.Collections.Concurrent;
using System.Diagnostics;
using System.Runtime.InteropServices;

namespace BatCave.Runtime.Collectors;

public interface IProcessCollector
{
    IReadOnlyList<ProcessSample> Collect(ulong seq);

    string? TakeWarning() => null;
}

public interface ISystemMetricsCollector
{
    SystemMetricsSnapshot Sample();
}

public sealed class WindowsProcessCollector : IProcessCollector
{
    private readonly ILogger<WindowsProcessCollector> _logger;
    private readonly Func<ulong> _nowMs;
    private readonly Func<Process, ProcessIoCounters?> _readIoCounters;
    private readonly Func<Process[]> _getProcesses;
    private readonly Dictionary<int, CpuSample> _previousCpuByPid = [];
    private readonly Dictionary<ProcessIdentity, IoSample> _previousIoByIdentity = [];
    private readonly Queue<string> _pendingWarnings = [];
    private IReadOnlyList<ProcessSample> _lastSuccessfulRows = [];
    private readonly int _processorCount = Math.Max(1, Environment.ProcessorCount);

    public WindowsProcessCollector(ILogger<WindowsProcessCollector>? logger = null)
        : this(logger, UnixNowMs, DefaultReadIoCounters, Process.GetProcesses)
    {
    }

    internal WindowsProcessCollector(
        Func<ulong> nowMs,
        Func<Process, ProcessIoCounters?> readIoCounters,
        ILogger<WindowsProcessCollector>? logger = null)
        : this(logger, nowMs, readIoCounters, Process.GetProcesses)
    {
    }

    internal WindowsProcessCollector(
        Func<ulong> nowMs,
        Func<Process, ProcessIoCounters?> readIoCounters,
        Func<Process[]> getProcesses,
        ILogger<WindowsProcessCollector>? logger = null)
        : this(logger, nowMs, readIoCounters, getProcesses)
    {
    }

    private WindowsProcessCollector(
        ILogger<WindowsProcessCollector>? logger,
        Func<ulong> nowMs,
        Func<Process, ProcessIoCounters?> readIoCounters,
        Func<Process[]> getProcesses)
    {
        _logger = logger ?? NullLogger<WindowsProcessCollector>.Instance;
        _nowMs = nowMs;
        _readIoCounters = readIoCounters;
        _getProcesses = getProcesses;
    }

    public IReadOnlyList<ProcessSample> Collect(ulong seq)
    {
        ulong nowMs = _nowMs();
        Process[] processes;
        try
        {
            processes = _getProcesses();
        }
        catch (Exception ex)
        {
            _logger.LogWarning(ex, "process_collect_failed");
            _pendingWarnings.Enqueue($"process_collect_failed error={ex.GetType().Name}: {ex.Message}");
            return RestampLastSuccessfulRows(seq, nowMs);
        }

        List<ProcessSample> rows = new(processes.Length);
        HashSet<int> seenPids = [];
        HashSet<ProcessIdentity> seenIdentities = [];
        IReadOnlyDictionary<uint, uint> parentPidByPid = SnapshotParentPids();

        foreach (Process process in processes)
        {
            using (process)
            {
                ProcessSample? sample = TryBuildSample(process, seq, nowMs, parentPidByPid);
                if (sample is null)
                {
                    continue;
                }

                seenPids.Add((int)sample.Pid);
                seenIdentities.Add(sample.Identity());
                rows.Add(sample);
            }
        }

        RemoveStaleCpuSamples(seenPids);
        RemoveStaleIoSamples(seenIdentities);
        _lastSuccessfulRows = Array.AsReadOnly(rows.ToArray());
        return rows;
    }

    public string? TakeWarning()
    {
        return _pendingWarnings.TryDequeue(out string? warning) ? warning : null;
    }

    private ProcessSample? TryBuildSample(
        Process process,
        ulong seq,
        ulong nowMs,
        IReadOnlyDictionary<uint, uint> parentPidByPid)
    {
        int pid;
        string name;
        try
        {
            pid = process.Id;
            name = process.ProcessName;
        }
        catch
        {
            return null;
        }

        (ulong startTimeMs, bool hasStartTime) = TryGetStartTimeMs(process);
        (TimeSpan totalProcessorTime, bool hasCpuTime) = TryGetTotalProcessorTime(process);
        double cpuPct = CalculateCpuPct(pid, startTimeMs, totalProcessorTime, nowMs);
        ProcessIdentity identity = new((uint)Math.Max(0, pid), startTimeMs);
        (ulong diskBps, ulong otherIoBps, bool hasIo) = CalculateIoRates(process, identity, nowMs);
        (ulong memoryBytes, bool hasMemory) = TryGetUlong(() => process.WorkingSet64);
        (ulong privateBytes, bool hasPrivateBytes) = TryGetUlong(() => process.PrivateMemorySize64);
        (uint threads, bool hasThreads) = TryGetUint(() => process.Threads.Count);
        (uint handles, bool hasHandles) = TryGetUint(() => process.HandleCount);
        uint normalizedPid = (uint)Math.Max(0, pid);
        _ = parentPidByPid.TryGetValue(normalizedPid, out uint parentPid);

        return new ProcessSample
        {
            Seq = seq,
            TsMs = nowMs,
            Pid = normalizedPid,
            ParentPid = parentPid,
            StartTimeMs = startTimeMs,
            Name = name,
            CpuPct = cpuPct,
            MemoryBytes = memoryBytes,
            PrivateBytes = privateBytes,
            DiskBps = diskBps,
            OtherIoBps = otherIoBps,
            Threads = threads,
            Handles = handles,
            AccessState = ResolveAccessState(hasStartTime, hasCpuTime, hasMemory, hasPrivateBytes, hasIo, hasThreads, hasHandles),
        };
    }

    private double CalculateCpuPct(int pid, ulong startTimeMs, TimeSpan totalProcessorTime, ulong nowMs)
    {
        CpuSample current = new(startTimeMs, nowMs, totalProcessorTime.TotalMilliseconds);
        if (!_previousCpuByPid.TryGetValue(pid, out CpuSample previous)
            || previous.StartTimeMs != startTimeMs
            || nowMs <= previous.TimestampMs)
        {
            _previousCpuByPid[pid] = current;
            return 0d;
        }

        _previousCpuByPid[pid] = current;
        double elapsedMs = Math.Max(1d, nowMs - previous.TimestampMs);
        double cpuDeltaMs = Math.Max(0d, current.TotalCpuMs - previous.TotalCpuMs);
        return Math.Clamp(cpuDeltaMs / elapsedMs / _processorCount * 100d, 0d, 100d);
    }

    private (ulong DiskBps, ulong OtherIoBps, bool Succeeded) CalculateIoRates(Process process, ProcessIdentity identity, ulong nowMs)
    {
        ProcessIoCounters? totals;
        try
        {
            totals = _readIoCounters(process);
        }
        catch
        {
            totals = null;
        }

        if (totals is not ProcessIoCounters currentTotals)
        {
            _previousIoByIdentity.Remove(identity);
            return (0, 0, false);
        }

        IoSample current = new(
            nowMs,
            SaturatingAdd(currentTotals.ReadTransferCount, currentTotals.WriteTransferCount),
            currentTotals.OtherTransferCount);

        if (!_previousIoByIdentity.TryGetValue(identity, out IoSample previous) || nowMs <= previous.TimestampMs)
        {
            _previousIoByIdentity[identity] = current;
            return (0, 0, true);
        }

        _previousIoByIdentity[identity] = current;
        ulong elapsedMs = Math.Max(1, nowMs - previous.TimestampMs);
        ulong diskBps = BytesPerSecond(current.DiskTransferCount, previous.DiskTransferCount, elapsedMs);
        ulong otherIoBps = BytesPerSecond(current.OtherTransferCount, previous.OtherTransferCount, elapsedMs);
        return (diskBps, otherIoBps, true);
    }

    private void RemoveStaleCpuSamples(HashSet<int> seenPids)
    {
        foreach (int pid in _previousCpuByPid.Keys.ToArray())
        {
            if (!seenPids.Contains(pid))
            {
                _previousCpuByPid.Remove(pid);
            }
        }
    }

    private void RemoveStaleIoSamples(HashSet<ProcessIdentity> seenIdentities)
    {
        foreach (ProcessIdentity identity in _previousIoByIdentity.Keys.ToArray())
        {
            if (!seenIdentities.Contains(identity))
            {
                _previousIoByIdentity.Remove(identity);
            }
        }
    }

    private static (ulong Value, bool Succeeded) TryGetStartTimeMs(Process process)
    {
        try
        {
            return ((ulong)Math.Max(0L, new DateTimeOffset(process.StartTime.ToUniversalTime()).ToUnixTimeMilliseconds()), true);
        }
        catch
        {
            return (0, false);
        }
    }

    private static (TimeSpan Value, bool Succeeded) TryGetTotalProcessorTime(Process process)
    {
        try
        {
            return (process.TotalProcessorTime, true);
        }
        catch
        {
            return (TimeSpan.Zero, false);
        }
    }

    private static (ulong Value, bool Succeeded) TryGetUlong(Func<long> read)
    {
        try
        {
            return ((ulong)Math.Max(0L, read()), true);
        }
        catch
        {
            return (0, false);
        }
    }

    private static (uint Value, bool Succeeded) TryGetUint(Func<int> read)
    {
        try
        {
            return ((uint)Math.Max(0, read()), true);
        }
        catch
        {
            return (0, false);
        }
    }

    private static ProcessIoCounters? DefaultReadIoCounters(Process process)
    {
        if (!RuntimeInformation.IsOSPlatform(OSPlatform.Windows))
        {
            return null;
        }

        try
        {
            return GetProcessIoCounters(process.Handle, out IO_COUNTERS counters)
                ? new ProcessIoCounters(counters.ReadTransferCount, counters.WriteTransferCount, counters.OtherTransferCount)
                : null;
        }
        catch
        {
            return null;
        }
    }

    private static IReadOnlyDictionary<uint, uint> SnapshotParentPids()
    {
        Dictionary<uint, uint> parentPidByPid = [];
        if (!RuntimeInformation.IsOSPlatform(OSPlatform.Windows))
        {
            return parentPidByPid;
        }

        IntPtr snapshot = CreateToolhelp32Snapshot(Th32csSnapProcess, 0);
        if (snapshot == InvalidHandleValue)
        {
            return parentPidByPid;
        }

        try
        {
            PROCESSENTRY32W entry = new()
            {
                dwSize = (uint)Marshal.SizeOf<PROCESSENTRY32W>(),
            };

            if (!Process32FirstW(snapshot, ref entry))
            {
                return parentPidByPid;
            }

            do
            {
                parentPidByPid[entry.th32ProcessID] = entry.th32ParentProcessID;
                entry.dwSize = (uint)Marshal.SizeOf<PROCESSENTRY32W>();
            }
            while (Process32NextW(snapshot, ref entry));
        }
        catch
        {
            parentPidByPid.Clear();
        }
        finally
        {
            _ = CloseHandle(snapshot);
        }

        return parentPidByPid;
    }

    internal static ulong BytesPerSecond(ulong current, ulong previous, ulong elapsedMs)
    {
        if (elapsedMs == 0 || current < previous)
        {
            return 0;
        }

        double rate = (current - previous) * 1000d / elapsedMs;
        return rate >= ulong.MaxValue ? ulong.MaxValue : (ulong)rate;
    }

    private static ulong SaturatingAdd(ulong left, ulong right)
    {
        return ulong.MaxValue - left < right ? ulong.MaxValue : left + right;
    }

    internal static AccessState ResolveAccessState(params bool[] probes)
    {
        if (probes.All(static probe => probe))
        {
            return AccessState.Full;
        }

        return probes.Any(static probe => probe) ? AccessState.Partial : AccessState.Denied;
    }

    private IReadOnlyList<ProcessSample> RestampLastSuccessfulRows(ulong seq, ulong nowMs)
    {
        if (_lastSuccessfulRows.Count == 0)
        {
            return [];
        }

        return Array.AsReadOnly(_lastSuccessfulRows.Select(row => row with
        {
            Seq = seq,
            TsMs = nowMs,
        }).ToArray());
    }

    private static ulong UnixNowMs() => (ulong)Math.Max(0L, DateTimeOffset.UtcNow.ToUnixTimeMilliseconds());

    private readonly record struct CpuSample(ulong StartTimeMs, ulong TimestampMs, double TotalCpuMs);

    private readonly record struct IoSample(ulong TimestampMs, ulong DiskTransferCount, ulong OtherTransferCount);

    [StructLayout(LayoutKind.Sequential)]
    private struct IO_COUNTERS
    {
        public ulong ReadOperationCount;
        public ulong WriteOperationCount;
        public ulong OtherOperationCount;
        public ulong ReadTransferCount;
        public ulong WriteTransferCount;
        public ulong OtherTransferCount;
    }

    [StructLayout(LayoutKind.Sequential, CharSet = CharSet.Unicode)]
    private struct PROCESSENTRY32W
    {
        public uint dwSize;
        public uint cntUsage;
        public uint th32ProcessID;
        public IntPtr th32DefaultHeapID;
        public uint th32ModuleID;
        public uint cntThreads;
        public uint th32ParentProcessID;
        public int pcPriClassBase;
        public uint dwFlags;

        [MarshalAs(UnmanagedType.ByValTStr, SizeConst = 260)]
        public string szExeFile;
    }

    private const uint Th32csSnapProcess = 0x00000002;
    private static readonly IntPtr InvalidHandleValue = new(-1);

    [DllImport("kernel32.dll", SetLastError = true)]
    private static extern bool GetProcessIoCounters(IntPtr processHandle, out IO_COUNTERS ioCounters);

    [DllImport("kernel32.dll", SetLastError = true)]
    private static extern IntPtr CreateToolhelp32Snapshot(uint dwFlags, uint th32ProcessID);

    [DllImport("kernel32.dll", SetLastError = true, CharSet = CharSet.Unicode)]
    private static extern bool Process32FirstW(IntPtr hSnapshot, ref PROCESSENTRY32W lppe);

    [DllImport("kernel32.dll", SetLastError = true, CharSet = CharSet.Unicode)]
    private static extern bool Process32NextW(IntPtr hSnapshot, ref PROCESSENTRY32W lppe);

    [DllImport("kernel32.dll", SetLastError = true)]
    private static extern bool CloseHandle(IntPtr hObject);
}

internal readonly record struct ProcessIoCounters(ulong ReadTransferCount, ulong WriteTransferCount, ulong OtherTransferCount);

public sealed class WindowsSystemMetricsCollector : ISystemMetricsCollector, IDisposable
{
    private const uint ErrorSuccess = 0;
    private const uint PdhFmtDouble = 0x00000200;
    private const uint PdhCstatusValidData = 0x00000000;
    private const uint PdhCstatusNewData = 0x00000001;

    private const string DiskReadCounterPath = @"\PhysicalDisk(_Total)\Disk Read Bytes/sec";
    private const string DiskWriteCounterPath = @"\PhysicalDisk(_Total)\Disk Write Bytes/sec";
    private const string OtherIoCounterPath = @"\Process(_Total)\IO Other Bytes/sec";

    private readonly object _sync = new();
    private readonly Func<(ulong? DiskReadBps, ulong? DiskWriteBps, ulong? OtherIoBps)>? _rateMetricSampler;
    private SystemTimes? _previousTimes;
    private IntPtr _pdhQuery;
    private IntPtr _diskReadCounter;
    private IntPtr _diskWriteCounter;
    private IntPtr _otherIoCounter;
    private bool _rateCountersWarmed;

    public WindowsSystemMetricsCollector()
    {
        InitializePdhCounters();
    }

    internal WindowsSystemMetricsCollector(Func<(ulong? DiskReadBps, ulong? DiskWriteBps, ulong? OtherIoBps)> rateMetricSampler)
    {
        _rateMetricSampler = rateMetricSampler;
    }

    public SystemMetricsSnapshot Sample()
    {
        lock (_sync)
        {
            ulong nowMs = (ulong)Math.Max(0L, DateTimeOffset.UtcNow.ToUnixTimeMilliseconds());
            (ulong? used, ulong? total, ulong? available) = SampleMemory();
            (double? cpu, double? kernel) = SampleCpu();
            (ulong? diskReadBps, ulong? diskWriteBps, ulong? otherIoBps) = SampleRateMetrics();

            return new SystemMetricsSnapshot
            {
                TsMs = nowMs,
                CpuPct = cpu,
                KernelCpuPct = kernel,
                MemoryUsedBytes = used,
                MemoryTotalBytes = total,
                MemoryAvailableBytes = available,
                DiskReadBps = diskReadBps,
                DiskWriteBps = diskWriteBps,
                OtherIoBps = otherIoBps,
                LogicalProcessorCount = Math.Max(1, Environment.ProcessorCount),
                IsReady = cpu.HasValue || used.HasValue || diskReadBps.HasValue || diskWriteBps.HasValue || otherIoBps.HasValue,
            };
        }
    }

    public void Dispose()
    {
        lock (_sync)
        {
            ClosePdhCounters();
            _previousTimes = null;
        }
    }

    private (double? CpuPct, double? KernelPct) SampleCpu()
    {
        if (!RuntimeInformation.IsOSPlatform(OSPlatform.Windows) || !GetSystemTimes(out FILETIME idle, out FILETIME kernel, out FILETIME user))
        {
            return (null, null);
        }

        SystemTimes current = new(ToUInt64(idle), ToUInt64(kernel), ToUInt64(user));
        if (_previousTimes is not SystemTimes previous)
        {
            _previousTimes = current;
            return (null, null);
        }

        _previousTimes = current;
        (double? cpu, double? kernelPct) = CalculateCpuPercentages(
            previous.Idle,
            previous.Kernel,
            previous.User,
            current.Idle,
            current.Kernel,
            current.User);
        if (!cpu.HasValue || !kernelPct.HasValue)
        {
            return (0d, 0d);
        }

        return (cpu, kernelPct);
    }

    private static (ulong? Used, ulong? Total, ulong? Available) SampleMemory()
    {
        if (!RuntimeInformation.IsOSPlatform(OSPlatform.Windows))
        {
            return (null, null, null);
        }

        MEMORYSTATUSEX status = new();
        status.dwLength = (uint)Marshal.SizeOf<MEMORYSTATUSEX>();
        if (!GlobalMemoryStatusEx(ref status))
        {
            return (null, null, null);
        }

        ulong used = status.ullTotalPhys >= status.ullAvailPhys
            ? status.ullTotalPhys - status.ullAvailPhys
            : 0;
        return (used, status.ullTotalPhys, status.ullAvailPhys);
    }

    private void InitializePdhCounters()
    {
        if (!RuntimeInformation.IsOSPlatform(OSPlatform.Windows))
        {
            return;
        }

        if (PdhOpenQueryW(null, IntPtr.Zero, out _pdhQuery) != ErrorSuccess)
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

        return PdhAddEnglishCounterW(_pdhQuery, path, IntPtr.Zero, out IntPtr counter) == ErrorSuccess
            ? counter
            : IntPtr.Zero;
    }

    private (ulong? DiskReadBps, ulong? DiskWriteBps, ulong? OtherIoBps) SampleRateMetrics()
    {
        if (_rateMetricSampler is not null)
        {
            return _rateMetricSampler();
        }

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

        uint status = PdhGetFormattedCounterValue(counter, PdhFmtDouble, out _, out PDH_FMT_COUNTERVALUE_DOUBLE value);
        if (status != ErrorSuccess || value.CStatus is not (PdhCstatusValidData or PdhCstatusNewData))
        {
            return null;
        }

        double candidate = value.DoubleValue;
        if (double.IsNaN(candidate) || double.IsInfinity(candidate) || candidate < 0d)
        {
            return null;
        }

        return candidate >= ulong.MaxValue ? ulong.MaxValue : (ulong)candidate;
    }

    internal static (double? CpuPct, double? KernelPct) CalculateCpuPercentages(
        ulong previousIdle,
        ulong previousKernel,
        ulong previousUser,
        ulong currentIdle,
        ulong currentKernel,
        ulong currentUser)
    {
        ulong idleDelta = CounterDelta(currentIdle, previousIdle);
        ulong kernelDelta = CounterDelta(currentKernel, previousKernel);
        ulong userDelta = CounterDelta(currentUser, previousUser);
        ulong total = SaturatingAdd(kernelDelta, userDelta);
        if (total == 0)
        {
            return (null, null);
        }

        ulong busy = total > idleDelta ? total - idleDelta : 0;
        ulong kernelBusy = kernelDelta > idleDelta ? kernelDelta - idleDelta : 0;
        double cpuPct = busy * 100d / total;
        double kernelPct = kernelBusy * 100d / total;
        return (Math.Clamp(cpuPct, 0d, 100d), Math.Clamp(kernelPct, 0d, 100d));
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

    private static ulong ToUInt64(FILETIME value) => ((ulong)value.dwHighDateTime << 32) | value.dwLowDateTime;

    private static ulong CounterDelta(ulong current, ulong previous) => current >= previous ? current - previous : 0;

    private static ulong SaturatingAdd(ulong left, ulong right) => ulong.MaxValue - left < right ? ulong.MaxValue : left + right;

    private readonly record struct SystemTimes(ulong Idle, ulong Kernel, ulong User);

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
    private static extern bool GetSystemTimes(out FILETIME idleTime, out FILETIME kernelTime, out FILETIME userTime);

    [DllImport("kernel32.dll", SetLastError = true)]
    private static extern bool GlobalMemoryStatusEx(ref MEMORYSTATUSEX lpBuffer);

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
