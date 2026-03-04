using BatCave.Core.Abstractions;
using BatCave.Core.Domain;
using Microsoft.Win32.SafeHandles;
using System.Runtime.InteropServices;

namespace BatCave.Core.Collector;

public sealed partial class WindowsProcessCollector : IProcessCollector
{
    private const ulong WindowsToUnixEpoch100ns = 116_444_736_000_000_000;
    private const ulong HandleRetryBackoffMs = 5_000;
    private const ulong MemorySampleStride = 2;
    private const ulong HandleSampleStride = 3;

    private const uint Th32CsSnapProcess = 0x00000002;
    private const uint ProcessQueryInformation = 0x0400;
    private const uint ProcessVmRead = 0x0010;
    private const uint ProcessQueryLimitedInformation = 0x1000;
    private const uint SynchronizeAccess = 0x0010_0000;

    private const uint WaitTimeout = 0x00000102;

    private static readonly uint[] ProcessAccessMasks =
    [
        ProcessQueryInformation | ProcessVmRead | SynchronizeAccess,
        ProcessQueryInformation | SynchronizeAccess,
        ProcessQueryLimitedInformation | ProcessVmRead | SynchronizeAccess,
        ProcessQueryLimitedInformation | SynchronizeAccess,
        ProcessQueryInformation | ProcessVmRead,
        ProcessQueryInformation,
        ProcessQueryLimitedInformation | ProcessVmRead,
        ProcessQueryLimitedInformation,
    ];

    private readonly Dictionary<ProcessIdentity, ProcessCounterSnapshot> _previousProcess = new();
    private readonly Dictionary<uint, FallbackIdentity> _pidFallbackStart = new();
    private readonly Dictionary<uint, OwnedProcessHandle> _processHandles = new();
    private readonly Dictionary<uint, ulong> _deniedHandleRetryUntilMs = new();
    private readonly List<uint> _stalePidScratch = [];

    private ulong? _previousSystemTotal100ns;
    private ulong? _previousTickMs;
    private string? _pendingWarning;

    public IReadOnlyList<ProcessSample> CollectTick(ulong seq)
    {
        ulong now = NowMs();
        ulong? systemTotal100ns = QuerySystemTotal100ns();
        ulong? systemDelta100ns = ResolveSystemDelta100ns(_previousSystemTotal100ns, systemTotal100ns);
        ulong elapsedMs = ResolveElapsedMs(now, _previousTickMs);

        Dictionary<ProcessIdentity, ProcessCounterSnapshot> currentSnapshot = new(_previousProcess.Count + 256);
        HashSet<uint> seenPids = new();
        if (!TryCollectSnapshotRows(seq, now, elapsedMs, systemDelta100ns, currentSnapshot, seenPids, out List<ProcessSample> rows))
        {
            return rows;
        }

        UpdatePreviousProcessSnapshot(currentSnapshot);

        _previousSystemTotal100ns = systemTotal100ns;
        _previousTickMs = now;

        RetainOnlySeenPids(seenPids);

        return rows;
    }

    private bool TryCollectSnapshotRows(
        ulong seq,
        ulong now,
        ulong elapsedMs,
        ulong? systemDelta100ns,
        Dictionary<ProcessIdentity, ProcessCounterSnapshot> currentSnapshot,
        HashSet<uint> seenPids,
        out List<ProcessSample> rows)
    {
        rows = new List<ProcessSample>(Math.Max(256, _previousProcess.Count));
        IntPtr snapshotHandle = CreateToolhelp32Snapshot(Th32CsSnapProcess, 0);
        if (IsInvalidHandle(snapshotHandle))
        {
            _pendingWarning = "failed to create process snapshot";
            return false;
        }

        using SafeSnapshotHandle safeSnapshotHandle = new(snapshotHandle, ownsHandle: true);

        PROCESSENTRY32 processEntry = new PROCESSENTRY32
        {
            dwSize = (uint)Marshal.SizeOf<PROCESSENTRY32>(),
        };

        bool hasEntry = Process32FirstW(safeSnapshotHandle.DangerousGetHandle(), ref processEntry);
        while (hasEntry)
        {
            uint pid = processEntry.th32ProcessID;
            if (pid != 0)
            {
                seenPids.Add(pid);
                ProcessRowCapture capture = CaptureProcessRow(ref processEntry, seq, now, elapsedMs, systemDelta100ns);
                currentSnapshot[capture.Identity] = capture.Counters;
                rows.Add(capture.Sample);
            }

            hasEntry = Process32NextW(safeSnapshotHandle.DangerousGetHandle(), ref processEntry);
        }

        return true;
    }

    private void UpdatePreviousProcessSnapshot(Dictionary<ProcessIdentity, ProcessCounterSnapshot> currentSnapshot)
    {
        _previousProcess.Clear();
        foreach ((ProcessIdentity identity, ProcessCounterSnapshot snapshot) in currentSnapshot)
        {
            _previousProcess[identity] = snapshot;
        }
    }

    private ProcessRowCapture CaptureProcessRow(
        ref PROCESSENTRY32 processEntry,
        ulong seq,
        ulong now,
        ulong elapsedMs,
        ulong? systemDelta100ns)
    {
        uint pid = processEntry.th32ProcessID;
        ProcessSample sample = CreateBaseSample(ref processEntry, seq, now);
        ProcessCounterSnapshot counters = default;

        IntPtr processHandle = EnsureProcessHandle(pid, now);
        bool hasTimes = false;
        bool hasIo = false;
        bool hasHandles = false;

        if (processHandle != IntPtr.Zero)
        {
            sample = sample with { AccessState = AccessState.Partial };
            CaptureTimesAndIo(processHandle, ref sample, ref counters, ref hasTimes, ref hasIo);
        }

        sample = sample with
        {
            StartTimeMs = ResolveStartTimeMs(
                _pidFallbackStart,
                pid,
                sample.StartTimeMs,
                now,
                sample.ParentPid,
                sample.Name),
        };

        ProcessIdentity identity = sample.Identity();

        bool hadPrevious = _previousProcess.TryGetValue(identity, out ProcessCounterSnapshot previous);
        if (hadPrevious)
        {
            MergePreviousCounters(processHandle, ref sample, ref counters, previous, hasTimes, hasIo);
        }

        if (processHandle != IntPtr.Zero)
        {
            CaptureMemoryAndHandles(seq, processHandle, ref sample, ref counters, ref hasHandles);

            if (hasTimes && hasIo && hasHandles)
            {
                sample = sample with { AccessState = AccessState.Full };
            }
        }

        if (hadPrevious)
        {
            sample = ApplyRateDeltas(sample, counters, previous, elapsedMs, systemDelta100ns);
        }

        return new ProcessRowCapture(identity, sample, counters);
    }

    private static ProcessSample CreateBaseSample(ref PROCESSENTRY32 processEntry, ulong seq, ulong now)
    {
        return new ProcessSample
        {
            Seq = seq,
            TsMs = now,
            Pid = processEntry.th32ProcessID,
            ParentPid = processEntry.th32ParentProcessID,
            StartTimeMs = 0,
            Name = ReadProcessEntryExecutableName(ref processEntry),
            CpuPct = 0,
            RssBytes = 0,
            PrivateBytes = 0,
            IoReadBps = 0,
            IoWriteBps = 0,
            OtherIoBps = 0,
            Threads = processEntry.cntThreads,
            Handles = 0,
            AccessState = AccessState.Denied,
        };
    }

    private static unsafe string ReadProcessEntryExecutableName(ref PROCESSENTRY32 processEntry)
    {
        fixed (char* executableName = processEntry.szExeFile)
        {
            return new string(executableName);
        }
    }

    private static void CaptureTimesAndIo(
        IntPtr processHandle,
        ref ProcessSample sample,
        ref ProcessCounterSnapshot counters,
        ref bool hasTimes,
        ref bool hasIo)
    {
        if (GetProcessTimes(processHandle, out FILETIME created, out _, out FILETIME kernel, out FILETIME user))
        {
            sample = sample with
            {
                StartTimeMs = FiletimeToUnixMs(created),
            };
            counters.CpuTotal100ns = FiletimeToU64(kernel) + FiletimeToU64(user);
            hasTimes = true;
        }

        if (GetProcessIoCounters(processHandle, out IO_COUNTERS ioCounters))
        {
            counters.IoReadTotal = ioCounters.ReadTransferCount;
            counters.IoWriteTotal = ioCounters.WriteTransferCount;
            counters.IoOtherTotal = ioCounters.OtherTransferCount;
            hasIo = true;
        }
    }

    private static void MergePreviousCounters(
        IntPtr processHandle,
        ref ProcessSample sample,
        ref ProcessCounterSnapshot counters,
        ProcessCounterSnapshot previous,
        bool hasTimes,
        bool hasIo)
    {
        if (!hasTimes)
        {
            counters.CpuTotal100ns = previous.CpuTotal100ns;
        }

        if (!hasIo)
        {
            counters.IoReadTotal = previous.IoReadTotal;
            counters.IoWriteTotal = previous.IoWriteTotal;
            counters.IoOtherTotal = previous.IoOtherTotal;
        }

        counters.RssBytes = previous.RssBytes;
        counters.PrivateBytes = previous.PrivateBytes;
        counters.Handles = previous.Handles;
        counters.LastMemorySeq = previous.LastMemorySeq;
        counters.LastHandlesSeq = previous.LastHandlesSeq;
        counters.HasMemory = previous.HasMemory;
        counters.HasHandles = previous.HasHandles;

        if (processHandle != IntPtr.Zero)
        {
            sample = sample with
            {
                RssBytes = previous.RssBytes,
                PrivateBytes = previous.PrivateBytes,
                Handles = previous.Handles,
            };
        }
    }

    private static void CaptureMemoryAndHandles(
        ulong seq,
        IntPtr processHandle,
        ref ProcessSample sample,
        ref ProcessCounterSnapshot counters,
        ref bool hasHandles)
    {
        bool refreshMemory = !counters.HasMemory || ShouldRefreshMetric(seq, counters.LastMemorySeq, MemorySampleStride);
        if (refreshMemory)
        {
            if (GetProcessMemoryInfo(processHandle, out PROCESS_MEMORY_COUNTERS_EX memoryCounters, (uint)Marshal.SizeOf<PROCESS_MEMORY_COUNTERS_EX>()))
            {
                sample = sample with
                {
                    RssBytes = (ulong)memoryCounters.WorkingSetSize,
                    PrivateBytes = (ulong)memoryCounters.PrivateUsage,
                };
                counters.RssBytes = sample.RssBytes;
                counters.PrivateBytes = sample.PrivateBytes;
                counters.LastMemorySeq = seq;
                counters.HasMemory = true;
            }
        }

        bool refreshHandles = !counters.HasHandles || ShouldRefreshMetric(seq, counters.LastHandlesSeq, HandleSampleStride);
        if (refreshHandles)
        {
            if (GetProcessHandleCount(processHandle, out uint handleCount))
            {
                sample = sample with { Handles = handleCount };
                counters.Handles = handleCount;
                counters.LastHandlesSeq = seq;
                counters.HasHandles = true;
                hasHandles = true;
            }
        }
        else if (counters.HasHandles)
        {
            sample = sample with { Handles = counters.Handles };
            hasHandles = true;
        }
    }

    private static ProcessSample ApplyRateDeltas(
        ProcessSample sample,
        ProcessCounterSnapshot counters,
        ProcessCounterSnapshot previousSnapshot,
        ulong elapsedMs,
        ulong? systemDelta100ns)
    {
        double cpuPct = 0;
        if (systemDelta100ns is ulong systemDeltaValue && systemDeltaValue > 0)
        {
            ulong processDelta = CounterDelta(counters.CpuTotal100ns, previousSnapshot.CpuTotal100ns);
            cpuPct = processDelta * 100.0 / systemDeltaValue;
        }

        ulong readDelta = CounterDelta(counters.IoReadTotal, previousSnapshot.IoReadTotal);
        ulong writeDelta = CounterDelta(counters.IoWriteTotal, previousSnapshot.IoWriteTotal);
        ulong otherDelta = CounterDelta(counters.IoOtherTotal, previousSnapshot.IoOtherTotal);

        return sample with
        {
            CpuPct = cpuPct,
            IoReadBps = (ulong)((readDelta * 1000.0) / elapsedMs),
            IoWriteBps = (ulong)((writeDelta * 1000.0) / elapsedMs),
            OtherIoBps = (ulong)((otherDelta * 1000.0) / elapsedMs),
        };
    }

    public string? TakeWarning()
    {
        string? warning = _pendingWarning;
        _pendingWarning = null;
        return warning;
    }

    private IntPtr EnsureProcessHandle(uint pid, ulong now)
    {
        if (_processHandles.TryGetValue(pid, out OwnedProcessHandle? cachedHandle))
        {
            if (IsProcessHandleAlive(cachedHandle.DangerousGetHandle()))
            {
                return cachedHandle.DangerousGetHandle();
            }

            cachedHandle.Dispose();
            _processHandles.Remove(pid);
        }

        if (_deniedHandleRetryUntilMs.TryGetValue(pid, out ulong nextRetryMs) && now < nextRetryMs)
        {
            return IntPtr.Zero;
        }

        OwnedProcessHandle? openedHandle = OpenProcessHandle(pid);
        if (openedHandle is null)
        {
            _deniedHandleRetryUntilMs[pid] = now + HandleRetryBackoffMs;
            return IntPtr.Zero;
        }

        _processHandles[pid] = openedHandle;
        _deniedHandleRetryUntilMs.Remove(pid);
        return openedHandle.DangerousGetHandle();
    }

    private static OwnedProcessHandle? OpenProcessHandle(uint pid)
    {
        foreach (uint accessMask in ProcessAccessMasks)
        {
            IntPtr handle = OpenProcess(accessMask, false, pid);
            if (handle != IntPtr.Zero)
            {
                return new OwnedProcessHandle(handle, ownsHandle: true);
            }
        }

        return null;
    }

    private static bool IsProcessHandleAlive(IntPtr handle)
    {
        if (IsInvalidHandle(handle))
        {
            return false;
        }

        return WaitForSingleObject(handle, 0) == WaitTimeout;
    }

    private static ulong? QuerySystemTotal100ns()
    {
        if (!GetSystemTimes(out _, out FILETIME kernel, out FILETIME user))
        {
            return null;
        }

        return FiletimeToU64(kernel) + FiletimeToU64(user);
    }

    private static ulong FiletimeToU64(FILETIME filetime)
    {
        return ((ulong)filetime.dwHighDateTime << 32) | filetime.dwLowDateTime;
    }

    private static ulong FiletimeToUnixMs(FILETIME filetime)
    {
        ulong windows100ns = FiletimeToU64(filetime);
        if (windows100ns <= WindowsToUnixEpoch100ns)
        {
            return 0;
        }

        return (windows100ns - WindowsToUnixEpoch100ns) / 10_000;
    }

    private static ulong? ResolveSystemDelta100ns(ulong? previousSystemTotal100ns, ulong? currentSystemTotal100ns)
    {
        if (previousSystemTotal100ns is not ulong previous || currentSystemTotal100ns is not ulong current)
        {
            return null;
        }

        return current >= previous ? current - previous : 0;
    }

    private static ulong ResolveElapsedMs(ulong now, ulong? previousTickMs)
    {
        if (previousTickMs is not ulong previous)
        {
            return 1000;
        }

        return Math.Max(1, now >= previous ? now - previous : 1);
    }

    private static bool IsInvalidHandle(IntPtr handle)
    {
        return handle == IntPtr.Zero || handle == new IntPtr(-1);
    }

    private static bool ShouldRefreshMetric(ulong seq, ulong lastSampleSeq, ulong stride)
    {
        if (stride <= 1)
        {
            return true;
        }

        return seq >= lastSampleSeq && seq - lastSampleSeq >= stride;
    }

    private static ulong ResolveStartTimeMs(
        Dictionary<uint, FallbackIdentity> fallbackStart,
        uint pid,
        ulong reportedStartTimeMs,
        ulong now,
        uint parentPid,
        string name)
    {
        if (reportedStartTimeMs != 0)
        {
            fallbackStart[pid] = new FallbackIdentity
            {
                StartTimeMs = reportedStartTimeMs,
                ParentPid = parentPid,
                Name = name,
            };
            return reportedStartTimeMs;
        }

        if (!fallbackStart.TryGetValue(pid, out FallbackIdentity? fallback))
        {
            fallback = new FallbackIdentity
            {
                StartTimeMs = now,
                ParentPid = parentPid,
                Name = name,
            };
            fallbackStart[pid] = fallback;
            return fallback.StartTimeMs;
        }

        if (fallback.ParentPid != parentPid || !string.Equals(fallback.Name, name, StringComparison.Ordinal))
        {
            fallback.StartTimeMs = now;
            fallback.ParentPid = parentPid;
            fallback.Name = name;
        }

        return fallback.StartTimeMs;
    }

    private void RetainOnlySeenPids(HashSet<uint> seenPids)
    {
        RemoveMissingPids(_pidFallbackStart, seenPids, _stalePidScratch);
        RemoveMissingPids(_processHandles, seenPids, _stalePidScratch, handle => handle.Dispose());
        RemoveMissingPids(_deniedHandleRetryUntilMs, seenPids, _stalePidScratch);
    }

    private static ulong NowMs()
    {
        return (ulong)DateTimeOffset.UtcNow.ToUnixTimeMilliseconds();
    }

    private static ulong CounterDelta(ulong current, ulong previous)
    {
        return current >= previous ? current - previous : 0;
    }

    private static void RemoveMissingPids<TValue>(
        Dictionary<uint, TValue> entries,
        HashSet<uint> seenPids,
        List<uint> stalePids,
        Action<TValue>? onRemove = null)
    {
        stalePids.Clear();
        foreach (uint pid in entries.Keys)
        {
            if (!seenPids.Contains(pid))
            {
                stalePids.Add(pid);
            }
        }

        foreach (uint pid in stalePids)
        {
            if (!entries.TryGetValue(pid, out TValue? value))
            {
                continue;
            }

            onRemove?.Invoke(value);
            entries.Remove(pid);
        }
    }

    private struct ProcessCounterSnapshot
    {
        public ulong CpuTotal100ns;
        public ulong IoReadTotal;
        public ulong IoWriteTotal;
        public ulong IoOtherTotal;
        public ulong RssBytes;
        public ulong PrivateBytes;
        public uint Handles;
        public ulong LastMemorySeq;
        public ulong LastHandlesSeq;
        public bool HasMemory;
        public bool HasHandles;
    }

    private readonly record struct ProcessRowCapture(
        ProcessIdentity Identity,
        ProcessSample Sample,
        ProcessCounterSnapshot Counters);

    private sealed class FallbackIdentity
    {
        public ulong StartTimeMs { get; set; }

        public uint ParentPid { get; set; }

        public string Name { get; set; } = string.Empty;
    }

    private sealed class SafeSnapshotHandle : SafeHandleZeroOrMinusOneIsInvalid
    {
        public SafeSnapshotHandle(IntPtr handle, bool ownsHandle)
            : base(ownsHandle)
        {
            SetHandle(handle);
        }

        protected override bool ReleaseHandle()
        {
            return CloseHandle(handle);
        }
    }

    private sealed class OwnedProcessHandle : SafeHandleZeroOrMinusOneIsInvalid
    {
        public OwnedProcessHandle(IntPtr handle, bool ownsHandle)
            : base(ownsHandle)
        {
            SetHandle(handle);
        }

        protected override bool ReleaseHandle()
        {
            return CloseHandle(handle);
        }
    }

    [StructLayout(LayoutKind.Sequential)]
    private unsafe struct PROCESSENTRY32
    {
        public uint dwSize;
        public uint cntUsage;
        public uint th32ProcessID;
        public nuint th32DefaultHeapID;
        public uint th32ModuleID;
        public uint cntThreads;
        public uint th32ParentProcessID;
        public int pcPriClassBase;
        public uint dwFlags;
        public fixed char szExeFile[260];
    }

    [StructLayout(LayoutKind.Sequential)]
    private struct FILETIME
    {
        public uint dwLowDateTime;
        public uint dwHighDateTime;
    }

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

    [StructLayout(LayoutKind.Sequential)]
    private struct PROCESS_MEMORY_COUNTERS_EX
    {
        public uint cb;
        public uint PageFaultCount;
        public nuint PeakWorkingSetSize;
        public nuint WorkingSetSize;
        public nuint QuotaPeakPagedPoolUsage;
        public nuint QuotaPagedPoolUsage;
        public nuint QuotaPeakNonPagedPoolUsage;
        public nuint QuotaNonPagedPoolUsage;
        public nuint PagefileUsage;
        public nuint PeakPagefileUsage;
        public nuint PrivateUsage;
    }

    [LibraryImport("kernel32.dll", SetLastError = true)]
    private static partial IntPtr CreateToolhelp32Snapshot(uint flags, uint processId);

    [LibraryImport("kernel32.dll", StringMarshalling = StringMarshalling.Utf16, SetLastError = true)]
    [return: MarshalAs(UnmanagedType.Bool)]
    private static partial bool Process32FirstW(IntPtr snapshotHandle, ref PROCESSENTRY32 processEntry);

    [LibraryImport("kernel32.dll", StringMarshalling = StringMarshalling.Utf16, SetLastError = true)]
    [return: MarshalAs(UnmanagedType.Bool)]
    private static partial bool Process32NextW(IntPtr snapshotHandle, ref PROCESSENTRY32 processEntry);

    [LibraryImport("kernel32.dll", SetLastError = true)]
    [return: MarshalAs(UnmanagedType.Bool)]
    private static partial bool CloseHandle(IntPtr handle);

    [LibraryImport("kernel32.dll", SetLastError = true)]
    private static partial IntPtr OpenProcess(uint desiredAccess, [MarshalAs(UnmanagedType.Bool)] bool inheritHandle, uint processId);

    [LibraryImport("kernel32.dll", SetLastError = true)]
    [return: MarshalAs(UnmanagedType.Bool)]
    private static partial bool GetProcessTimes(
        IntPtr processHandle,
        out FILETIME creationTime,
        out FILETIME exitTime,
        out FILETIME kernelTime,
        out FILETIME userTime);

    [LibraryImport("kernel32.dll", SetLastError = true)]
    [return: MarshalAs(UnmanagedType.Bool)]
    private static partial bool GetProcessIoCounters(IntPtr processHandle, out IO_COUNTERS ioCounters);

    [LibraryImport("kernel32.dll", SetLastError = true)]
    [return: MarshalAs(UnmanagedType.Bool)]
    private static partial bool GetProcessHandleCount(IntPtr processHandle, out uint handleCount);

    [LibraryImport("psapi.dll", SetLastError = true)]
    [return: MarshalAs(UnmanagedType.Bool)]
    private static partial bool GetProcessMemoryInfo(IntPtr processHandle, out PROCESS_MEMORY_COUNTERS_EX counters, uint size);

    [LibraryImport("kernel32.dll", SetLastError = true)]
    [return: MarshalAs(UnmanagedType.Bool)]
    private static partial bool GetSystemTimes(out FILETIME idleTime, out FILETIME kernelTime, out FILETIME userTime);

    [LibraryImport("kernel32.dll", SetLastError = true)]
    private static partial uint WaitForSingleObject(IntPtr handle, uint milliseconds);
}
