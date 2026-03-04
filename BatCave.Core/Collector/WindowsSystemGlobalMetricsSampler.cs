using BatCave.Core.Abstractions;
using BatCave.Core.Domain;
using System.Runtime.InteropServices;

namespace BatCave.Core.Collector;

public sealed partial class WindowsSystemGlobalMetricsSampler : ISystemGlobalMetricsSampler, IDisposable
{
    private static readonly TimeSpan DefaultExtendedProbeSoftTimeout = TimeSpan.FromMilliseconds(750);
    private static readonly TimeSpan MetadataRefreshRetryInterval = TimeSpan.FromSeconds(1);
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
    private bool _cpuRateWarmed;
    private bool _extendedProbeCycleCompleted;
    private Task? _extendedProbeTask;
    private Task? _metadataRefreshTask;
    private SystemGlobalCpuSnapshot? _latestCpuSnapshot;
    private SystemGlobalMemorySnapshot? _latestMemorySnapshot;
    private IReadOnlyList<SystemGlobalDiskSnapshot> _latestDiskSnapshots = [];
    private IReadOnlyList<SystemGlobalNetworkSnapshot> _latestNetworkSnapshots = [];

    private readonly Func<double?, double?, SystemGlobalCpuSnapshot?> _cpuSnapshotFactory;
    private readonly Func<ulong?, SystemGlobalMemorySnapshot> _memorySnapshotFactory;
    private readonly Func<IReadOnlyList<SystemGlobalDiskSnapshot>> _diskSnapshotFactory;
    private readonly Func<IReadOnlyList<SystemGlobalNetworkSnapshot>> _networkSnapshotFactory;
    private readonly Action _metadataRefreshAction;
    private readonly TimeSpan _extendedProbeSoftTimeout;

    public WindowsSystemGlobalMetricsSampler()
        : this(
            cpuSnapshotFactory: null,
            memorySnapshotFactory: null,
            diskSnapshotFactory: null,
            networkSnapshotFactory: null,
            metadataRefreshAction: null,
            extendedProbeSoftTimeout: null,
            initializePdhCounters: true)
    {
    }

    internal WindowsSystemGlobalMetricsSampler(
        Func<double?, double?, SystemGlobalCpuSnapshot?>? cpuSnapshotFactory,
        Func<ulong?, SystemGlobalMemorySnapshot>? memorySnapshotFactory,
        Func<IReadOnlyList<SystemGlobalDiskSnapshot>>? diskSnapshotFactory,
        Func<IReadOnlyList<SystemGlobalNetworkSnapshot>>? networkSnapshotFactory,
        Action? metadataRefreshAction,
        TimeSpan? extendedProbeSoftTimeout,
        bool initializePdhCounters)
    {
        _cpuSnapshotFactory = cpuSnapshotFactory ?? BuildCpuSnapshot;
        _memorySnapshotFactory = memorySnapshotFactory ?? BuildMemorySnapshot;
        _diskSnapshotFactory = diskSnapshotFactory ?? BuildDiskSnapshots;
        _networkSnapshotFactory = networkSnapshotFactory ?? BuildNetworkSnapshots;
        _metadataRefreshAction = metadataRefreshAction ?? RefreshMetadataSnapshot;
        _extendedProbeSoftTimeout = extendedProbeSoftTimeout ?? DefaultExtendedProbeSoftTimeout;

        if (initializePdhCounters)
        {
            InitializePdhCounters();
        }
    }

    public SystemGlobalMetricsSample Sample()
    {
        lock (_sync)
        {
            ThrowIfDisposed();
            (ulong? diskReadBps, ulong? diskWriteBps, ulong? otherIoBps) = SampleRateMetrics();
            double? cpuPct = SampleCpuPct();
            ulong? memoryUsedBytes = SampleMemoryUsedBytes();
            double? kernelPct = _lastKernelPct;
            if (cpuPct.HasValue)
            {
                _cpuRateWarmed = true;
            }

            TryPromoteCompletedMetadataRefresh_NoThrow();
            TryScheduleMetadataRefresh_NoThrow();
            TryStartExtendedProbeCycle_NoThrow(cpuPct, memoryUsedBytes, kernelPct);
            bool rateCountersReady = _rateCountersWarmed || _pdhQuery == IntPtr.Zero;
            bool isReady = _cpuRateWarmed && rateCountersReady && _extendedProbeCycleCompleted;

            return new SystemGlobalMetricsSample
            {
                TsMs = (ulong)DateTimeOffset.UtcNow.ToUnixTimeMilliseconds(),
                CpuPct = cpuPct,
                MemoryUsedBytes = memoryUsedBytes,
                DiskReadBps = diskReadBps,
                DiskWriteBps = diskWriteBps,
                OtherIoBps = otherIoBps,
                CpuSnapshot = _latestCpuSnapshot,
                MemorySnapshot = _latestMemorySnapshot,
                DiskSnapshots = _latestDiskSnapshots,
                NetworkSnapshots = _latestNetworkSnapshots,
                CpuRateWarmed = _cpuRateWarmed,
                RateCountersWarmed = rateCountersReady,
                ExtendedProbeCycleCompleted = _extendedProbeCycleCompleted,
                IsReady = isReady,
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
            _cpuRateWarmed = false;
            _extendedProbeCycleCompleted = false;
            _latestCpuSnapshot = null;
            _latestMemorySnapshot = null;
            _latestDiskSnapshots = [];
            _latestNetworkSnapshots = [];
        }

        GC.SuppressFinalize(this);
    }

    private void TryStartExtendedProbeCycle_NoThrow(double? cpuPct, ulong? memoryUsedBytes, double? kernelPct)
    {
        if (_extendedProbeTask is { IsCompleted: false })
        {
            return;
        }

        _extendedProbeTask = Task.Run(() => RunExtendedProbeCycleAsync(cpuPct, memoryUsedBytes, kernelPct));
    }

    private void TryPromoteCompletedMetadataRefresh_NoThrow()
    {
        if (_metadataRefreshTask is null || !_metadataRefreshTask.IsCompleted)
        {
            return;
        }

        if (_metadataRefreshTask.IsFaulted)
        {
            _ = _metadataRefreshTask.Exception;
        }

        _metadataRefreshTask = null;
    }

    private void TryScheduleMetadataRefresh_NoThrow()
    {
        if (_metadataRefreshTask is { IsCompleted: false })
        {
            return;
        }

        if (DateTimeOffset.UtcNow < _nextMetadataRefreshUtc)
        {
            return;
        }

        TimeSpan nextRefreshDelay = HasWarmMetadataCache()
            ? MetadataRefreshInterval
            : MetadataRefreshRetryInterval;
        _nextMetadataRefreshUtc = DateTimeOffset.UtcNow.Add(nextRefreshDelay);
        _metadataRefreshTask = Task.Run(() =>
        {
            try
            {
                _metadataRefreshAction();
            }
            catch
            {
                // Keep the runtime resilient; stale metadata is acceptable.
            }
        });
    }

    private bool HasWarmMetadataCache()
    {
        return _cpuStaticMetadata != CpuStaticMetadata.Empty
            && _memoryStaticMetadata != MemoryStaticMetadata.Empty
            && !string.IsNullOrWhiteSpace(_cpuStaticMetadata.ProcessorName);
    }

    private async Task RunExtendedProbeCycleAsync(double? cpuPct, ulong? memoryUsedBytes, double? kernelPct)
    {
        Task<SystemGlobalCpuSnapshot?> cpuTask = Task.Run(() => _cpuSnapshotFactory(cpuPct, kernelPct));
        Task<SystemGlobalMemorySnapshot> memoryTask = Task.Run(() => _memorySnapshotFactory(memoryUsedBytes));
        Task<IReadOnlyList<SystemGlobalDiskSnapshot>> diskTask = Task.Run(() => _diskSnapshotFactory());
        Task<IReadOnlyList<SystemGlobalNetworkSnapshot>> networkTask = Task.Run(() => _networkSnapshotFactory());

        Task allProbeTasks = Task.WhenAll(cpuTask, memoryTask, diskTask, networkTask);
        Task softTimeout = Task.Delay(_extendedProbeSoftTimeout);
        Task completed = await Task.WhenAny(allProbeTasks, softTimeout).ConfigureAwait(false);
        bool completedWithinBudget = ReferenceEquals(completed, allProbeTasks);

        ApplyExtendedProbeResults(
            GetCompletedTaskResult(cpuTask),
            cpuTask.IsCompletedSuccessfully,
            GetCompletedTaskResult(memoryTask),
            memoryTask.IsCompletedSuccessfully,
            GetCompletedTaskResult(diskTask),
            diskTask.IsCompletedSuccessfully,
            GetCompletedTaskResult(networkTask),
            networkTask.IsCompletedSuccessfully,
            markCycleComplete: true);

        if (completedWithinBudget)
        {
            if (allProbeTasks.IsFaulted)
            {
                _ = allProbeTasks.Exception;
            }

            return;
        }

        try
        {
            await allProbeTasks.ConfigureAwait(false);
        }
        catch
        {
            // Individual probe failures are intentionally handled via stale-value fallbacks.
        }

        // Promote any probe results that completed after the soft timeout.
        ApplyExtendedProbeResults(
            GetCompletedTaskResult(cpuTask),
            cpuTask.IsCompletedSuccessfully,
            GetCompletedTaskResult(memoryTask),
            memoryTask.IsCompletedSuccessfully,
            GetCompletedTaskResult(diskTask),
            diskTask.IsCompletedSuccessfully,
            GetCompletedTaskResult(networkTask),
            networkTask.IsCompletedSuccessfully,
            markCycleComplete: false);
    }

    private static T? GetCompletedTaskResult<T>(Task<T> task)
    {
        return task.IsCompletedSuccessfully ? task.Result : default;
    }

    private void ApplyExtendedProbeResults(
        SystemGlobalCpuSnapshot? cpuSnapshot,
        bool cpuCompleted,
        SystemGlobalMemorySnapshot? memorySnapshot,
        bool memoryCompleted,
        IReadOnlyList<SystemGlobalDiskSnapshot>? diskSnapshots,
        bool diskCompleted,
        IReadOnlyList<SystemGlobalNetworkSnapshot>? networkSnapshots,
        bool networkCompleted,
        bool markCycleComplete)
    {
        lock (_sync)
        {
            if (_disposed)
            {
                return;
            }

            if (cpuCompleted)
            {
                _latestCpuSnapshot = cpuSnapshot;
            }

            if (memoryCompleted)
            {
                _latestMemorySnapshot = memorySnapshot;
            }

            if (diskCompleted)
            {
                _latestDiskSnapshots = diskSnapshots ?? [];
            }

            if (networkCompleted)
            {
                _latestNetworkSnapshots = networkSnapshots ?? [];
            }

            if (markCycleComplete)
            {
                _extendedProbeCycleCompleted = true;
            }
        }
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

        uint status = PdhGetFormattedCounterValue(counter, PdhFmtDouble, out _, out PDH_FMT_COUNTERVALUE_DOUBLE value);
        if (status != ErrorSuccess || !IsValidCounterStatus(value.CStatus))
        {
            return null;
        }

        double candidate = value.DoubleValue;
        if (!IsFiniteNonNegative(candidate))
        {
            return null;
        }

        if (candidate > ulong.MaxValue)
        {
            return ulong.MaxValue;
        }

        return (ulong)candidate;
    }

    private static bool IsValidCounterStatus(uint cStatus)
    {
        return cStatus is PdhCstatusValidData or PdhCstatusNewData;
    }

    private static bool IsFiniteNonNegative(double value)
    {
        return !double.IsNaN(value) && !double.IsInfinity(value) && value >= 0d;
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

    [LibraryImport("kernel32.dll", SetLastError = true)]
    [return: MarshalAs(UnmanagedType.Bool)]
    private static partial bool GetSystemTimes(out FILETIME idleTime, out FILETIME kernelTime, out FILETIME userTime);

    [LibraryImport("kernel32.dll", SetLastError = true)]
    [return: MarshalAs(UnmanagedType.Bool)]
    private static partial bool GlobalMemoryStatusEx(ref MEMORYSTATUSEX buffer);

    [LibraryImport("pdh.dll", StringMarshalling = StringMarshalling.Utf16)]
    private static partial uint PdhOpenQueryW(string? dataSource, IntPtr userData, out IntPtr query);

    [LibraryImport("pdh.dll", StringMarshalling = StringMarshalling.Utf16)]
    private static partial uint PdhAddEnglishCounterW(IntPtr query, string fullCounterPath, IntPtr userData, out IntPtr counter);

    [LibraryImport("pdh.dll")]
    private static partial uint PdhCollectQueryData(IntPtr query);

    [LibraryImport("pdh.dll")]
    private static partial uint PdhGetFormattedCounterValue(
        IntPtr counter,
        uint format,
        out uint type,
        out PDH_FMT_COUNTERVALUE_DOUBLE value);

    [LibraryImport("pdh.dll")]
    private static partial uint PdhCloseQuery(IntPtr query);
}
