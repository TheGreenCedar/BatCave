using BatCave.Core.Domain;
using System.Globalization;
using System.Management;
using System.Net.NetworkInformation;
using System.Runtime.InteropServices;
using System.Text;

namespace BatCave.Core.Collector;

public sealed partial class WindowsSystemGlobalMetricsSampler
{
    private static readonly TimeSpan MetadataRefreshInterval = TimeSpan.FromSeconds(30);
    private const double MinReasonableCpuSpeedMHz = 100d;
    private const double MaxReasonableCpuSpeedMHz = 20000d;

    private DateTimeOffset _nextMetadataRefreshUtc = DateTimeOffset.MinValue;
    private CpuStaticMetadata _cpuStaticMetadata = CpuStaticMetadata.Empty;
    private MemoryStaticMetadata _memoryStaticMetadata = MemoryStaticMetadata.Empty;
    private Dictionary<int, DiskStaticMetadata> _diskMetadataByIndex = [];
    private Dictionary<int, DiskRawPerformanceSnapshot> _previousPhysicalDiskRawByIndex = [];
    private Dictionary<string, int> _diskIndexByDriveLetter = new(StringComparer.OrdinalIgnoreCase);
    private Dictionary<string, NetworkStaticMetadata> _networkMetadataByNormalizedName = [];
    private HashSet<string> _pageFileDrives = [];
    private string _systemDriveLetter = GetSystemDriveLetter();

    private (SystemGlobalCpuSnapshot? CpuSnapshot, SystemGlobalMemorySnapshot? MemorySnapshot, IReadOnlyList<SystemGlobalDiskSnapshot> DiskSnapshots, IReadOnlyList<SystemGlobalNetworkSnapshot> NetworkSnapshots) SampleExtendedSnapshots(
        double? cpuPct,
        ulong? memoryUsedBytes)
    {
        RefreshMetadataCacheIfDue();

        SystemGlobalCpuSnapshot? cpuSnapshot = null;
        SystemGlobalMemorySnapshot? memorySnapshot = null;
        IReadOnlyList<SystemGlobalDiskSnapshot> diskSnapshots = [];
        IReadOnlyList<SystemGlobalNetworkSnapshot> networkSnapshots = [];

        try
        {
            cpuSnapshot = BuildCpuSnapshot(cpuPct, _lastKernelPct);
        }
        catch
        {
            cpuSnapshot = null;
        }

        try
        {
            memorySnapshot = BuildMemorySnapshot(memoryUsedBytes);
        }
        catch
        {
            memorySnapshot = new SystemGlobalMemorySnapshot
            {
                UsedBytes = memoryUsedBytes,
            };
        }

        try
        {
            diskSnapshots = BuildDiskSnapshots();
        }
        catch
        {
            diskSnapshots = [];
        }

        try
        {
            networkSnapshots = BuildNetworkSnapshots();
        }
        catch
        {
            networkSnapshots = [];
        }

        return (cpuSnapshot, memorySnapshot, diskSnapshots, networkSnapshots);
    }

    private void RefreshMetadataCacheIfDue()
    {
        DateTimeOffset now = DateTimeOffset.UtcNow;
        if (now < _nextMetadataRefreshUtc)
        {
            return;
        }

        _nextMetadataRefreshUtc = now.Add(MetadataRefreshInterval);
        RefreshMetadataSnapshot();
    }

    private void RefreshMetadataSnapshot()
    {
        // Metadata refresh may fail in parts; retain previous values for any failed section.

        try
        {
            _cpuStaticMetadata = ReadCpuStaticMetadata();
        }
        catch
        {
            // Keep prior metadata on failures to avoid regressing populated UI fields to n/a.
        }

        try
        {
            _memoryStaticMetadata = ReadMemoryStaticMetadata();
        }
        catch
        {
            // Keep prior metadata on failures to avoid regressing populated UI fields to n/a.
        }

        try
        {
            _diskMetadataByIndex = ReadDiskStaticMetadata();
        }
        catch
        {
            // Keep prior metadata on failures to avoid regressing populated UI fields to n/a.
        }

        try
        {
            _diskIndexByDriveLetter = ReadDiskIndexByDriveLetter();
        }
        catch
        {
            // Keep prior metadata on failures to avoid regressing populated UI fields to n/a.
        }

        try
        {
            _networkMetadataByNormalizedName = ReadNetworkStaticMetadata();
        }
        catch
        {
            // Keep prior metadata on failures to avoid regressing populated UI fields to n/a.
        }

        try
        {
            _pageFileDrives = ReadPageFileDrives();
        }
        catch
        {
            // Keep prior metadata on failures to avoid regressing populated UI fields to n/a.
        }

        _systemDriveLetter = GetSystemDriveLetter();
    }

    private SystemGlobalCpuSnapshot BuildCpuSnapshot(double? cpuPct, double? kernelPct)
    {
        List<double> logicalUtilization = [];
        uint? processes = null;
        uint? threads = null;
        uint? handles = null;
        ulong? uptimeSeconds = null;
        (double? ActualFrequencyMHz, double? ProcessorFrequencyMHz) dynamicCpuSpeedMHz = (null, null);

        try
        {
            using ManagementObjectSearcher processorPerfSearcher = new("SELECT Name, PercentProcessorTime FROM Win32_PerfFormattedData_PerfOS_Processor");
            using ManagementObjectCollection processorRows = processorPerfSearcher.Get();
            foreach (ManagementBaseObject row in processorRows)
            {
                string? name = row["Name"] as string;
                if (string.IsNullOrWhiteSpace(name))
                {
                    continue;
                }

                if (name == "_Total")
                {
                    continue;
                }

                if (int.TryParse(name, out _))
                {
                    logicalUtilization.Add(ReadDouble(row["PercentProcessorTime"]) ?? 0d);
                }
            }
        }
        catch
        {
            logicalUtilization = [];
        }

        try
        {
            using ManagementObjectSearcher osSearcher = new("SELECT NumberOfProcesses, LastBootUpTime FROM Win32_OperatingSystem");
            using ManagementObjectCollection osRows = osSearcher.Get();
            ManagementBaseObject? osRow = osRows.Cast<ManagementBaseObject>().FirstOrDefault();
            if (osRow is not null)
            {
                processes = ReadUInt(osRow["NumberOfProcesses"]);
                if (osRow["LastBootUpTime"] is string lastBoot)
                {
                    DateTime boot = ManagementDateTimeConverter.ToDateTime(lastBoot).ToUniversalTime();
                    TimeSpan uptime = DateTime.UtcNow - boot;
                    if (uptime.TotalSeconds > 0)
                    {
                        uptimeSeconds = (ulong)uptime.TotalSeconds;
                    }
                }
            }
        }
        catch
        {
            // no-op
        }

        try
        {
            using ManagementObjectSearcher perfSystemSearcher = new("SELECT Threads, Processes, SystemCallsPersec FROM Win32_PerfFormattedData_PerfOS_System");
            using ManagementObjectCollection perfRows = perfSystemSearcher.Get();
            ManagementBaseObject? perfRow = perfRows.Cast<ManagementBaseObject>().FirstOrDefault();
            if (perfRow is not null)
            {
                threads = ReadUInt(perfRow["Threads"]);
            }
        }
        catch
        {
            // no-op
        }

        try
        {
            using ManagementObjectSearcher processPerfSearcher = new("SELECT HandleCount FROM Win32_PerfFormattedData_PerfProc_Process WHERE Name = '_Total'");
            using ManagementObjectCollection processRows = processPerfSearcher.Get();
            ManagementBaseObject? processRow = processRows.Cast<ManagementBaseObject>().FirstOrDefault();
            if (processRow is not null)
            {
                handles = ReadUInt(processRow["HandleCount"]);
            }
        }
        catch
        {
            // no-op
        }

        dynamicCpuSpeedMHz = TryReadDynamicCpuSpeedMHz();

        return new SystemGlobalCpuSnapshot
        {
            ProcessorName = _cpuStaticMetadata.ProcessorName,
            KernelPct = kernelPct,
            SpeedMHz = ResolveCpuSpeedMHz(
                dynamicCpuSpeedMHz.ActualFrequencyMHz,
                dynamicCpuSpeedMHz.ProcessorFrequencyMHz,
                _cpuStaticMetadata.CurrentSpeedMHz),
            BaseSpeedMHz = _cpuStaticMetadata.BaseSpeedMHz,
            Sockets = _cpuStaticMetadata.Sockets,
            Cores = _cpuStaticMetadata.Cores,
            LogicalProcessors = _cpuStaticMetadata.LogicalProcessors,
            VirtualizationEnabled = _cpuStaticMetadata.VirtualizationEnabled,
            L1CacheBytes = _cpuStaticMetadata.L1CacheBytes,
            L2CacheBytes = _cpuStaticMetadata.L2CacheBytes,
            L3CacheBytes = _cpuStaticMetadata.L3CacheBytes,
            ProcessCount = processes,
            ThreadCount = threads,
            HandleCount = handles,
            UptimeSeconds = uptimeSeconds,
            LogicalProcessorUtilizationPct = logicalUtilization,
        };
    }

    private static (double? ActualFrequencyMHz, double? ProcessorFrequencyMHz) ReadDynamicCpuSpeedMHz()
    {
        using ManagementObjectSearcher searcher = new(
            "SELECT Name, ActualFrequency, ProcessorFrequency FROM Win32_PerfFormattedData_Counters_ProcessorInformation");
        using ManagementObjectCollection rows = searcher.Get();

        ManagementBaseObject? totalRow = rows
            .Cast<ManagementBaseObject>()
            .FirstOrDefault(static row => string.Equals(row["Name"] as string, "_Total", StringComparison.OrdinalIgnoreCase));
        if (totalRow is null)
        {
            return (null, null);
        }

        return (
            ActualFrequencyMHz: ReadDouble(totalRow["ActualFrequency"]),
            ProcessorFrequencyMHz: ReadDouble(totalRow["ProcessorFrequency"]));
    }

    private static (double? ActualFrequencyMHz, double? ProcessorFrequencyMHz) TryReadDynamicCpuSpeedMHz()
    {
        try
        {
            return ReadDynamicCpuSpeedMHz();
        }
        catch
        {
            return (null, null);
        }
    }

    private SystemGlobalMemorySnapshot BuildMemorySnapshot(ulong? memoryUsedBytes)
    {
        ulong? visibleTotalBytes = null;
        ulong? availableBytes = null;
        ulong? committedBytes = null;
        ulong? commitLimitBytes = null;
        ulong? cacheBytes = null;
        ulong? pagedPoolBytes = null;
        ulong? nonPagedPoolBytes = null;

        try
        {
            MEMORYSTATUSEX memoryStatus = new()
            {
                dwLength = (uint)Marshal.SizeOf<MEMORYSTATUSEX>(),
            };

            if (GlobalMemoryStatusEx(ref memoryStatus))
            {
                visibleTotalBytes = memoryStatus.ullTotalPhys;
                availableBytes = memoryStatus.ullAvailPhys;
            }
        }
        catch
        {
            // no-op
        }

        try
        {
            using ManagementObjectSearcher perfMemorySearcher = new(
                "SELECT CommittedBytes, CommitLimit, CacheBytes, PoolPagedBytes, PoolNonpagedBytes FROM Win32_PerfFormattedData_PerfOS_Memory");
            using ManagementObjectCollection perfRows = perfMemorySearcher.Get();
            ManagementBaseObject? row = perfRows.Cast<ManagementBaseObject>().FirstOrDefault();
            if (row is not null)
            {
                committedBytes = ReadULong(row["CommittedBytes"]);
                commitLimitBytes = ReadULong(row["CommitLimit"]);
                cacheBytes = ReadULong(row["CacheBytes"]);
                pagedPoolBytes = ReadULong(row["PoolPagedBytes"]);
                nonPagedPoolBytes = ReadULong(row["PoolNonpagedBytes"]);
            }
        }
        catch
        {
            // no-op
        }

        ulong? installedTotalBytes = _memoryStaticMetadata.TotalPhysicalBytes;
        ulong? totalBytes = visibleTotalBytes ?? installedTotalBytes;
        ulong? hardwareReservedBytes = ResolveHardwareReservedBytes(installedTotalBytes, visibleTotalBytes);

        return new SystemGlobalMemorySnapshot
        {
            TotalBytes = totalBytes,
            UsedBytes = memoryUsedBytes,
            AvailableBytes = availableBytes,
            CommittedUsedBytes = committedBytes,
            CommittedLimitBytes = commitLimitBytes,
            CachedBytes = cacheBytes,
            PagedPoolBytes = pagedPoolBytes,
            NonPagedPoolBytes = nonPagedPoolBytes,
            SpeedMTps = _memoryStaticMetadata.SpeedMTps,
            SlotsUsed = _memoryStaticMetadata.SlotsUsed,
            SlotsTotal = _memoryStaticMetadata.SlotsTotal,
            FormFactor = _memoryStaticMetadata.FormFactor,
            HardwareReservedBytes = hardwareReservedBytes,
        };
    }

    private IReadOnlyList<SystemGlobalDiskSnapshot> BuildDiskSnapshots()
    {
        List<SystemGlobalDiskSnapshot> result = [];
        Dictionary<int, DiskPerformanceSnapshot> physicalPerfByIndex = ReadPhysicalDiskPerfByIndex();
        foreach ((int diskIndex, DiskPerformanceSnapshot perf) in physicalPerfByIndex.OrderBy(static pair => pair.Key))
        {
            IReadOnlyList<string> driveLetters = ResolveDriveLettersForDiskIndex(diskIndex);
            if (driveLetters.Count == 0)
            {
                // Hide backing members that do not directly expose a mounted logical drive (e.g., Storage Spaces members).
                continue;
            }

            string driveLetter = driveLetters[0];
            _diskMetadataByIndex.TryGetValue(diskIndex, out DiskStaticMetadata? metadata);
            bool? isSystemDisk = driveLetters.Any(drive => string.Equals(drive, _systemDriveLetter, StringComparison.OrdinalIgnoreCase));
            bool? hasPageFile = _pageFileDrives.Count == 0 ? null : driveLetters.Any(drive => _pageFileDrives.Contains(drive));

            result.Add(new SystemGlobalDiskSnapshot
            {
                DiskId = driveLetter,
                DisplayName = BuildDiskDisplayName(diskIndex, driveLetter),
                Model = metadata?.Model,
                TypeLabel = ResolveDiskTypeLabel(metadata?.TypeLabel, metadata?.Model),
                ActiveTimePct = perf.ActiveTimePct,
                AvgResponseMs = perf.AvgResponseMs,
                ReadBps = perf.ReadBps,
                WriteBps = perf.WriteBps,
                CapacityBytes = metadata?.CapacityBytes,
                FormattedBytes = metadata?.FormattedBytes,
                IsSystemDisk = isSystemDisk,
                HasPageFile = hasPageFile,
            });
        }

        return result.OrderBy(static snapshot => snapshot.DisplayName, StringComparer.OrdinalIgnoreCase).ToList();
    }

    private Dictionary<int, DiskPerformanceSnapshot> ReadPhysicalDiskPerfByIndex()
    {
        Dictionary<int, DiskPerformanceSnapshot> result = [];
        Dictionary<int, double?> avgResponseMsByIndex = ResolvePhysicalDiskAvgResponseMsByIndex();

        try
        {
            using ManagementObjectSearcher searcher = new(
                "SELECT Name, PercentDiskTime, PercentIdleTime, DiskReadBytesPersec, DiskWriteBytesPersec FROM Win32_PerfFormattedData_PerfDisk_PhysicalDisk");
            using ManagementObjectCollection rows = searcher.Get();

            foreach (ManagementBaseObject row in rows)
            {
                string? name = row["Name"] as string;
                if (string.IsNullOrWhiteSpace(name) || string.Equals(name, "_Total", StringComparison.OrdinalIgnoreCase))
                {
                    continue;
                }

                int? diskIndex = ParseDiskIndex(name);
                if (!diskIndex.HasValue)
                {
                    continue;
                }

                double? active = ResolveDiskActiveTimePct(
                    diskTimePct: ReadDouble(row["PercentDiskTime"]),
                    idleTimePct: ReadDouble(row["PercentIdleTime"]));
                avgResponseMsByIndex.TryGetValue(diskIndex.Value, out double? avgResponseMs);

                ulong? read = ReadULong(row["DiskReadBytesPersec"]);
                ulong? write = ReadULong(row["DiskWriteBytesPersec"]);

                if (result.TryGetValue(diskIndex.Value, out DiskPerformanceSnapshot? existing))
                {
                    result[diskIndex.Value] = new DiskPerformanceSnapshot
                    {
                        ActiveTimePct = ResolvePreferredDiskActiveTimePct(active, existing.ActiveTimePct),
                        AvgResponseMs = MergeMaxNullable(avgResponseMs, existing.AvgResponseMs),
                        ReadBps = SumNullable(existing.ReadBps, read),
                        WriteBps = SumNullable(existing.WriteBps, write),
                    };
                    continue;
                }

                result[diskIndex.Value] = new DiskPerformanceSnapshot
                {
                    ActiveTimePct = active,
                    AvgResponseMs = avgResponseMs,
                    ReadBps = read,
                    WriteBps = write,
                };
            }
        }
        catch
        {
            return [];
        }

        return result;
    }

    private Dictionary<int, double?> ResolvePhysicalDiskAvgResponseMsByIndex()
    {
        Dictionary<int, DiskRawPerformanceSnapshot>? rawByIndex = ReadPhysicalDiskRawPerfByIndex();
        if (rawByIndex is null)
        {
            return [];
        }

        Dictionary<int, double?> result = [];
        foreach ((int diskIndex, DiskRawPerformanceSnapshot current) in rawByIndex)
        {
            _previousPhysicalDiskRawByIndex.TryGetValue(diskIndex, out DiskRawPerformanceSnapshot? previous);
            result[diskIndex] = ResolveAvgResponseMsFromRawCounters(
                previousCounterValue: previous?.AvgDiskSecPerTransfer,
                currentCounterValue: current.AvgDiskSecPerTransfer,
                previousCounterBase: previous?.AvgDiskSecPerTransferBase,
                currentCounterBase: current.AvgDiskSecPerTransferBase,
                frequencyPerfTime: current.FrequencyPerfTime);
        }

        _previousPhysicalDiskRawByIndex = rawByIndex;
        return result;
    }

    private static Dictionary<int, DiskRawPerformanceSnapshot>? ReadPhysicalDiskRawPerfByIndex()
    {
        Dictionary<int, DiskRawPerformanceSnapshot> result = [];

        try
        {
            using ManagementObjectSearcher searcher = new(
                "SELECT Name, AvgDiskSecPerTransfer, AvgDiskSecPerTransfer_Base, Frequency_PerfTime FROM Win32_PerfRawData_PerfDisk_PhysicalDisk");
            using ManagementObjectCollection rows = searcher.Get();

            foreach (ManagementBaseObject row in rows)
            {
                string? name = row["Name"] as string;
                if (string.IsNullOrWhiteSpace(name) || string.Equals(name, "_Total", StringComparison.OrdinalIgnoreCase))
                {
                    continue;
                }

                int? diskIndex = ParseDiskIndex(name);
                if (!diskIndex.HasValue)
                {
                    continue;
                }

                DiskRawPerformanceSnapshot current = new()
                {
                    AvgDiskSecPerTransfer = ReadULong(row["AvgDiskSecPerTransfer"]),
                    AvgDiskSecPerTransferBase = ReadULong(row["AvgDiskSecPerTransfer_Base"]),
                    FrequencyPerfTime = ReadULong(row["Frequency_PerfTime"]),
                };

                if (result.TryGetValue(diskIndex.Value, out DiskRawPerformanceSnapshot? existing))
                {
                    result[diskIndex.Value] = new DiskRawPerformanceSnapshot
                    {
                        AvgDiskSecPerTransfer = SumNullable(existing.AvgDiskSecPerTransfer, current.AvgDiskSecPerTransfer),
                        AvgDiskSecPerTransferBase = SumNullable(existing.AvgDiskSecPerTransferBase, current.AvgDiskSecPerTransferBase),
                        FrequencyPerfTime = MergeMaxNullable(existing.FrequencyPerfTime, current.FrequencyPerfTime),
                    };
                    continue;
                }

                result[diskIndex.Value] = current;
            }
        }
        catch
        {
            return null;
        }

        return result;
    }

    private static double? ResolveDiskActiveTimePct(ManagementBaseObject row)
    {
        return ResolveDiskActiveTimePct(
            diskTimePct: ReadDouble(row["PercentDiskTime"]),
            idleTimePct: ReadDouble(row["PercentIdleTime"]));
    }

    internal static double? ResolveDiskActiveTimePct(double? diskTimePct, double? idleTimePct)
    {
        double? normalizedDiskTimePct = NormalizePercent(diskTimePct);
        double? activeFromIdlePct = idleTimePct.HasValue
            ? NormalizePercent(100d - idleTimePct.Value)
            : null;

        if (normalizedDiskTimePct.HasValue && activeFromIdlePct.HasValue)
        {
            return Math.Max(normalizedDiskTimePct.Value, activeFromIdlePct.Value);
        }

        return normalizedDiskTimePct ?? activeFromIdlePct;
    }

    internal static double? ResolvePreferredDiskActiveTimePct(double? primaryPct, double? fallbackPct)
    {
        double? normalizedPrimary = NormalizePercent(primaryPct);
        double? normalizedFallback = NormalizePercent(fallbackPct);

        if (normalizedPrimary.HasValue && normalizedFallback.HasValue)
        {
            return Math.Max(normalizedPrimary.Value, normalizedFallback.Value);
        }

        return normalizedPrimary ?? normalizedFallback;
    }

    internal static double? ResolveCpuSpeedMHz(double? actualFrequencyMHz, double? processorFrequencyMHz, double? staticCurrentClockSpeedMHz)
    {
        return NormalizeCpuSpeedMHz(actualFrequencyMHz)
            ?? NormalizeCpuSpeedMHz(processorFrequencyMHz)
            ?? NormalizeCpuSpeedMHz(staticCurrentClockSpeedMHz);
    }

    private IReadOnlyList<SystemGlobalNetworkSnapshot> BuildNetworkSnapshots()
    {
        Dictionary<string, SystemGlobalNetworkSnapshot> byAdapterId = new(StringComparer.OrdinalIgnoreCase);
        using ManagementObjectSearcher searcher = new(
            "SELECT Name, BytesReceivedPersec, BytesSentPersec, CurrentBandwidth FROM Win32_PerfFormattedData_Tcpip_NetworkInterface");
        using ManagementObjectCollection rows = searcher.Get();

        foreach (ManagementBaseObject row in rows)
        {
            string? name = row["Name"] as string;
            if (string.IsNullOrWhiteSpace(name))
            {
                continue;
            }

            string normalized = NormalizeCounterName(name);
            _networkMetadataByNormalizedName.TryGetValue(normalized, out NetworkStaticMetadata? metadata);
            if (metadata is null || !metadata.IncludeInGlobalList)
            {
                continue;
            }

            string adapterId = metadata.AdapterId ?? normalized;
            ulong? send = ReadULong(row["BytesSentPersec"]);
            ulong? receive = ReadULong(row["BytesReceivedPersec"]);
            ulong? bandwidth = ReadULong(row["CurrentBandwidth"]);

            if (byAdapterId.TryGetValue(adapterId, out SystemGlobalNetworkSnapshot? existing))
            {
                byAdapterId[adapterId] = existing with
                {
                    SendBps = SumNullable(existing.SendBps, send),
                    ReceiveBps = SumNullable(existing.ReceiveBps, receive),
                    LinkSpeedBps = Math.Max(existing.LinkSpeedBps ?? 0UL, metadata.LinkSpeedBps ?? bandwidth ?? 0UL),
                };
                continue;
            }

            byAdapterId[adapterId] = new SystemGlobalNetworkSnapshot
            {
                AdapterId = adapterId,
                DisplayName = metadata.DisplayName ?? name,
                AdapterName = metadata.AdapterName ?? name,
                ConnectionType = metadata.ConnectionType,
                IPv4Address = metadata.IPv4Address,
                IPv6Address = metadata.IPv6Address,
                SendBps = send,
                ReceiveBps = receive,
                LinkSpeedBps = metadata.LinkSpeedBps ?? bandwidth,
            };
        }

        return byAdapterId.Values
            .OrderBy(static snapshot => snapshot.DisplayName, StringComparer.OrdinalIgnoreCase)
            .ToList();
    }

    private CpuStaticMetadata ReadCpuStaticMetadata()
    {
        (ulong? l1CacheBytes, ulong? l2CacheBytes, ulong? l3CacheBytes) = ReadCpuCacheBytesByLevel();
        using ManagementObjectSearcher processorSearcher = new(
            "SELECT Name, CurrentClockSpeed, MaxClockSpeed, NumberOfCores, NumberOfLogicalProcessors, L2CacheSize, L3CacheSize, VirtualizationFirmwareEnabled FROM Win32_Processor");
        using ManagementObjectCollection rows = processorSearcher.Get();
        ManagementBaseObject? row = rows.Cast<ManagementBaseObject>().FirstOrDefault();
        if (row is null)
        {
            return CpuStaticMetadata.Empty;
        }

        int? sockets = rows.Count;
        return new CpuStaticMetadata
        {
            ProcessorName = row["Name"] as string,
            CurrentSpeedMHz = ReadDouble(row["CurrentClockSpeed"]),
            BaseSpeedMHz = ReadDouble(row["MaxClockSpeed"]),
            Sockets = sockets,
            Cores = ReadInt(row["NumberOfCores"]),
            LogicalProcessors = ReadInt(row["NumberOfLogicalProcessors"]),
            VirtualizationEnabled = ReadBool(row["VirtualizationFirmwareEnabled"]),
            L1CacheBytes = l1CacheBytes,
            L2CacheBytes = l2CacheBytes ?? ScaleKbToBytes(ReadULong(row["L2CacheSize"])),
            L3CacheBytes = l3CacheBytes ?? ScaleKbToBytes(ReadULong(row["L3CacheSize"])),
        };
    }

    private static (ulong? L1CacheBytes, ulong? L2CacheBytes, ulong? L3CacheBytes) ReadCpuCacheBytesByLevel()
    {
        ulong? l1CacheBytes = null;
        ulong? l2CacheBytes = null;
        ulong? l3CacheBytes = null;

        try
        {
            using ManagementObjectSearcher cacheSearcher = new("SELECT Level, InstalledSize FROM Win32_CacheMemory");
            using ManagementObjectCollection cacheRows = cacheSearcher.Get();
            foreach (ManagementBaseObject row in cacheRows)
            {
                byte? cacheTier = ResolveCacheTierFromWmiLevel(ReadUInt(row["Level"]));
                ulong? cacheBytes = ScaleKbToBytes(ReadULong(row["InstalledSize"]));
                if (!cacheTier.HasValue || !cacheBytes.HasValue)
                {
                    continue;
                }

                switch (cacheTier.Value)
                {
                    case 1:
                        l1CacheBytes = MergeMaxNullable(l1CacheBytes, cacheBytes);
                        break;
                    case 2:
                        l2CacheBytes = MergeMaxNullable(l2CacheBytes, cacheBytes);
                        break;
                    case 3:
                        l3CacheBytes = MergeMaxNullable(l3CacheBytes, cacheBytes);
                        break;
                }
            }
        }
        catch
        {
            // no-op
        }

        return (l1CacheBytes, l2CacheBytes, l3CacheBytes);
    }

    private static MemoryStaticMetadata ReadMemoryStaticMetadata()
    {
        using ManagementObjectSearcher computerSearcher = new("SELECT TotalPhysicalMemory FROM Win32_ComputerSystem");
        using ManagementObjectCollection computerRows = computerSearcher.Get();
        ManagementBaseObject? computer = computerRows.Cast<ManagementBaseObject>().FirstOrDefault();

        ulong? totalBytes = ReadULong(computer?["TotalPhysicalMemory"]);
        ulong? capacityBytes = null;
        uint? speed = null;
        int slotsUsed = 0;
        int slotsTotal = 0;
        string? formFactor = null;

        using ManagementObjectSearcher physicalMemorySearcher = new("SELECT Capacity, Speed, FormFactor FROM Win32_PhysicalMemory");
        using ManagementObjectCollection physicalRows = physicalMemorySearcher.Get();
        foreach (ManagementBaseObject row in physicalRows)
        {
            slotsTotal++;
            if (ReadULong(row["Capacity"]) is ulong moduleCapacity && moduleCapacity > 0)
            {
                capacityBytes = SumNullable(capacityBytes, moduleCapacity);
            }

            if (ReadUInt(row["Speed"]) is uint moduleSpeed && moduleSpeed > 0)
            {
                speed = speed is null ? moduleSpeed : Math.Max(speed.Value, moduleSpeed);
                slotsUsed++;
            }

            if (formFactor is null && ReadUInt(row["FormFactor"]) is uint formFactorValue)
            {
                formFactor = ResolveMemoryFormFactor(formFactorValue);
            }
        }

        totalBytes = capacityBytes ?? totalBytes;

        return new MemoryStaticMetadata
        {
            TotalPhysicalBytes = totalBytes,
            SpeedMTps = speed,
            SlotsUsed = slotsUsed == 0 ? null : slotsUsed,
            SlotsTotal = slotsTotal == 0 ? null : slotsTotal,
            FormFactor = formFactor,
        };
    }

    private static Dictionary<int, DiskStaticMetadata> ReadDiskStaticMetadata()
    {
        Dictionary<int, DiskStaticMetadata> result = [];
        using ManagementObjectSearcher diskSearcher = new("SELECT Index, Model, MediaType, Size FROM Win32_DiskDrive");
        using ManagementObjectCollection diskRows = diskSearcher.Get();
        foreach (ManagementBaseObject row in diskRows)
        {
            if (ReadInt(row["Index"]) is not int index)
            {
                continue;
            }

            result[index] = new DiskStaticMetadata
            {
                Model = row["Model"] as string,
                TypeLabel = row["MediaType"] as string,
                CapacityBytes = ReadULong(row["Size"]),
                FormattedBytes = ReadULong(row["Size"]),
            };
        }

        return result;
    }

    private static Dictionary<string, NetworkStaticMetadata> ReadNetworkStaticMetadata()
    {
        Dictionary<string, NetworkStaticMetadata> result = new(StringComparer.OrdinalIgnoreCase);
        foreach (NetworkInterface networkInterface in NetworkInterface.GetAllNetworkInterfaces())
        {
            IPInterfaceProperties properties = networkInterface.GetIPProperties();
            string? ipv4 = properties.UnicastAddresses
                .Select(static address => address.Address)
                .FirstOrDefault(static address => address.AddressFamily == System.Net.Sockets.AddressFamily.InterNetwork)
                ?.ToString();
            string? ipv6 = properties.UnicastAddresses
                .Select(static address => address.Address)
                .FirstOrDefault(static address => address.AddressFamily == System.Net.Sockets.AddressFamily.InterNetworkV6)
                ?.ToString();
            bool hasAnyIpAddress = !string.IsNullOrWhiteSpace(ipv4) || !string.IsNullOrWhiteSpace(ipv6);

            bool isUp = networkInterface.OperationalStatus == OperationalStatus.Up;
            bool isWiredOrWireless = networkInterface.NetworkInterfaceType is NetworkInterfaceType.Ethernet
                or NetworkInterfaceType.GigabitEthernet
                or NetworkInterfaceType.FastEthernetFx
                or NetworkInterfaceType.FastEthernetT
                or NetworkInterfaceType.Wireless80211;
            bool looksVirtual = LooksLikeVirtualAdapter(networkInterface.Name, networkInterface.Description);

            NetworkStaticMetadata metadata = new()
            {
                AdapterId = networkInterface.Id,
                DisplayName = string.IsNullOrWhiteSpace(networkInterface.Name) ? networkInterface.Description : networkInterface.Name,
                AdapterName = networkInterface.Description,
                ConnectionType = networkInterface.NetworkInterfaceType.ToString(),
                IPv4Address = ipv4,
                IPv6Address = ipv6,
                LinkSpeedBps = networkInterface.Speed > 0 ? (ulong)networkInterface.Speed : null,
                IncludeInGlobalList = isWiredOrWireless && !looksVirtual && isUp && hasAnyIpAddress,
            };

            string normalizedDescription = NormalizeCounterName(networkInterface.Description);
            if (!string.IsNullOrWhiteSpace(normalizedDescription))
            {
                result[normalizedDescription] = metadata;
            }

            string normalizedName = NormalizeCounterName(networkInterface.Name);
            if (!string.IsNullOrWhiteSpace(normalizedName))
            {
                result[normalizedName] = metadata;
            }
        }

        return result;
    }

    private static HashSet<string> ReadPageFileDrives()
    {
        HashSet<string> result = new(StringComparer.OrdinalIgnoreCase);
        using ManagementObjectSearcher searcher = new("SELECT Name FROM Win32_PageFileUsage");
        using ManagementObjectCollection rows = searcher.Get();
        foreach (ManagementBaseObject row in rows)
        {
            if (row["Name"] is not string name || string.IsNullOrWhiteSpace(name))
            {
                continue;
            }

            if (name.Length >= 2 && char.IsLetter(name[0]) && name[1] == ':')
            {
                result.Add(name[..2]);
            }
        }

        return result;
    }

    private static string GetSystemDriveLetter()
    {
        string path = Environment.SystemDirectory;
        if (string.IsNullOrWhiteSpace(path) || path.Length < 2)
        {
            return "C:";
        }

        return path[..2].ToUpperInvariant();
    }

    private static int? ParseDiskIndex(string name)
    {
        int separator = name.IndexOf(' ');
        string token = separator > 0 ? name[..separator] : name;
        return int.TryParse(token, out int parsed) ? parsed : null;
    }

    private int? ResolveDiskIndexForDrive(string driveLetter)
    {
        return _diskIndexByDriveLetter.TryGetValue(driveLetter, out int index)
            ? index
            : null;
    }

    private IReadOnlyList<string> ResolveDriveLettersForDiskIndex(int diskIndex)
    {
        List<string> drives = [];
        foreach ((string driveLetter, int mappedIndex) in _diskIndexByDriveLetter.OrderBy(static pair => pair.Key, StringComparer.OrdinalIgnoreCase))
        {
            if (mappedIndex == diskIndex)
            {
                drives.Add(driveLetter);
            }
        }

        return drives;
    }

    private static string BuildDiskDisplayName(int? diskIndex, string driveLetter)
    {
        if (diskIndex.HasValue)
        {
            return string.IsNullOrWhiteSpace(driveLetter)
                ? $"Disk {diskIndex.Value}"
                : $"Disk {diskIndex.Value} ({driveLetter})";
        }

        return string.IsNullOrWhiteSpace(driveLetter)
            ? "Disk"
            : $"Disk ({driveLetter})";
    }

    private static string NormalizeDriveLetter(string name)
    {
        if (string.IsNullOrWhiteSpace(name))
        {
            return string.Empty;
        }

        string candidate = name.Trim();
        if (candidate.Length >= 2 && char.IsLetter(candidate[0]) && candidate[1] == ':')
        {
            return candidate[..2].ToUpperInvariant();
        }

        return string.Empty;
    }

    private static string? ResolveDiskTypeLabel(string? mediaType, string? model)
    {
        string token = $"{mediaType} {model}".Trim();
        if (string.IsNullOrWhiteSpace(token))
        {
            return null;
        }

        if (token.Contains("nvme", StringComparison.OrdinalIgnoreCase))
        {
            return "SSD (NVMe)";
        }

        if (token.Contains("ssd", StringComparison.OrdinalIgnoreCase))
        {
            return "SSD";
        }

        if (token.Contains("hdd", StringComparison.OrdinalIgnoreCase)
            || token.Contains("hard", StringComparison.OrdinalIgnoreCase))
        {
            return "HDD";
        }

        return mediaType;
    }

    private bool? TryResolveDriveFlag(string diskDisplayName, string driveLetter)
    {
        if (string.IsNullOrWhiteSpace(diskDisplayName) || string.IsNullOrWhiteSpace(driveLetter))
        {
            return null;
        }

        return diskDisplayName.Contains(driveLetter, StringComparison.OrdinalIgnoreCase);
    }

    private bool? TryResolvePageFileFlag(string diskDisplayName)
    {
        if (string.IsNullOrWhiteSpace(diskDisplayName) || _pageFileDrives.Count == 0)
        {
            return null;
        }

        foreach (string drive in _pageFileDrives)
        {
            if (diskDisplayName.Contains(drive, StringComparison.OrdinalIgnoreCase))
            {
                return true;
            }
        }

        return false;
    }

    private static string NormalizeCounterName(string name)
    {
        if (string.IsNullOrWhiteSpace(name))
        {
            return string.Empty;
        }

        StringBuilder builder = new(name.Length);
        foreach (char c in name)
        {
            if (char.IsLetterOrDigit(c))
            {
                builder.Append(char.ToLowerInvariant(c));
            }
        }

        return builder.ToString();
    }

    private static ulong? ScaleKbToBytes(ulong? valueInKb)
    {
        if (!valueInKb.HasValue)
        {
            return null;
        }

        return valueInKb.Value * 1024UL;
    }

    internal static byte? ResolveCacheTierFromWmiLevel(uint? level)
    {
        return level switch
        {
            3 => 1,
            4 => 2,
            5 => 3,
            _ => null,
        };
    }

    internal static ulong? ResolveHardwareReservedBytes(ulong? installedBytes, ulong? visibleBytes)
    {
        if (!installedBytes.HasValue || !visibleBytes.HasValue)
        {
            return null;
        }

        return installedBytes.Value >= visibleBytes.Value
            ? installedBytes.Value - visibleBytes.Value
            : 0UL;
    }

    internal static double? ResolveAvgResponseMsFromRawCounters(
        ulong? previousCounterValue,
        ulong? currentCounterValue,
        ulong? previousCounterBase,
        ulong? currentCounterBase,
        ulong? frequencyPerfTime)
    {
        if (!previousCounterValue.HasValue
            || !currentCounterValue.HasValue
            || !previousCounterBase.HasValue
            || !currentCounterBase.HasValue
            || !frequencyPerfTime.HasValue
            || frequencyPerfTime.Value == 0
            || currentCounterValue.Value < previousCounterValue.Value
            || currentCounterBase.Value <= previousCounterBase.Value)
        {
            return null;
        }

        ulong deltaCounter = currentCounterValue.Value - previousCounterValue.Value;
        ulong deltaBase = currentCounterBase.Value - previousCounterBase.Value;
        double responseMs = ((deltaCounter / (double)frequencyPerfTime.Value) / deltaBase) * 1000d;
        return double.IsFinite(responseMs) && responseMs >= 0d ? responseMs : null;
    }

    private static string? ResolveMemoryFormFactor(uint formFactor)
    {
        return formFactor switch
        {
            8 => "DIMM",
            12 => "SODIMM",
            _ => null,
        };
    }

    private static uint? ReadUInt(object? value)
    {
        if (value is null)
        {
            return null;
        }

        if (uint.TryParse(Convert.ToString(value, CultureInfo.InvariantCulture), NumberStyles.Integer, CultureInfo.InvariantCulture, out uint parsed))
        {
            return parsed;
        }

        return null;
    }

    private static int? ReadInt(object? value)
    {
        if (value is null)
        {
            return null;
        }

        if (int.TryParse(Convert.ToString(value, CultureInfo.InvariantCulture), NumberStyles.Integer, CultureInfo.InvariantCulture, out int parsed))
        {
            return parsed;
        }

        return null;
    }

    private static ulong? ReadULong(object? value)
    {
        if (value is null)
        {
            return null;
        }

        if (ulong.TryParse(Convert.ToString(value, CultureInfo.InvariantCulture), NumberStyles.Integer, CultureInfo.InvariantCulture, out ulong parsed))
        {
            return parsed;
        }

        return null;
    }

    private static double? ReadDouble(object? value)
    {
        if (value is null)
        {
            return null;
        }

        if (double.TryParse(Convert.ToString(value, CultureInfo.InvariantCulture), NumberStyles.Float, CultureInfo.InvariantCulture, out double parsed))
        {
            return parsed;
        }

        return null;
    }

    private static bool? ReadBool(object? value)
    {
        if (value is null)
        {
            return null;
        }

        if (bool.TryParse(Convert.ToString(value, CultureInfo.InvariantCulture), out bool parsed))
        {
            return parsed;
        }

        return null;
    }

    private static double? NormalizePercent(double? value)
    {
        if (!value.HasValue || !double.IsFinite(value.Value))
        {
            return null;
        }

        return Math.Clamp(value.Value, 0d, 100d);
    }

    internal static double? NormalizeCpuSpeedMHz(double? value)
    {
        if (!value.HasValue || !double.IsFinite(value.Value))
        {
            return null;
        }

        double speedMhz = value.Value;
        if (speedMhz is <= 0d or < MinReasonableCpuSpeedMHz or > MaxReasonableCpuSpeedMHz)
        {
            return null;
        }

        return speedMhz;
    }

    private static double? MergeMaxNullable(double? left, double? right)
    {
        if (left.HasValue && right.HasValue)
        {
            return Math.Max(left.Value, right.Value);
        }

        return left ?? right;
    }

    private static ulong? MergeMaxNullable(ulong? left, ulong? right)
    {
        if (left.HasValue && right.HasValue)
        {
            return Math.Max(left.Value, right.Value);
        }

        return left ?? right;
    }

    private static Dictionary<string, int> ReadDiskIndexByDriveLetter()
    {
        Dictionary<string, int> mapping = new(StringComparer.OrdinalIgnoreCase);
        using ManagementObjectSearcher assocSearcher = new("SELECT Antecedent, Dependent FROM Win32_LogicalDiskToPartition");
        using ManagementObjectCollection rows = assocSearcher.Get();
        foreach (ManagementBaseObject row in rows)
        {
            string antecedent = Convert.ToString(row["Antecedent"], CultureInfo.InvariantCulture) ?? string.Empty;
            string dependent = Convert.ToString(row["Dependent"], CultureInfo.InvariantCulture) ?? string.Empty;
            string drive = ExtractDriveLetterFromDependent(dependent);
            int? index = ExtractDiskIndexFromAntecedent(antecedent);
            if (string.IsNullOrWhiteSpace(drive) || !index.HasValue)
            {
                continue;
            }

            mapping[drive] = index.Value;
        }

        return mapping;
    }

    private static string ExtractDriveLetterFromDependent(string dependent)
    {
        int marker = dependent.IndexOf("DeviceID=\"", StringComparison.OrdinalIgnoreCase);
        if (marker < 0)
        {
            return string.Empty;
        }

        int start = marker + "DeviceID=\"".Length;
        int end = dependent.IndexOf('"', start);
        if (end <= start)
        {
            return string.Empty;
        }

        return NormalizeDriveLetter(dependent[start..end]);
    }

    private static int? ExtractDiskIndexFromAntecedent(string antecedent)
    {
        int marker = antecedent.IndexOf("Disk #", StringComparison.OrdinalIgnoreCase);
        if (marker < 0)
        {
            return null;
        }

        int start = marker + "Disk #".Length;
        int end = antecedent.IndexOf(',', start);
        string token = end > start ? antecedent[start..end] : antecedent[start..];
        token = token.Trim();
        return int.TryParse(token, NumberStyles.Integer, CultureInfo.InvariantCulture, out int parsed)
            ? parsed
            : null;
    }

    private static bool LooksLikeVirtualAdapter(string? name, string? description)
    {
        string token = $"{name} {description}".Trim();
        if (string.IsNullOrWhiteSpace(token))
        {
            return false;
        }

        return token.Contains("virtual", StringComparison.OrdinalIgnoreCase)
            || token.Contains("hyper-v", StringComparison.OrdinalIgnoreCase)
            || token.Contains("vmware", StringComparison.OrdinalIgnoreCase)
            || token.Contains("vbox", StringComparison.OrdinalIgnoreCase)
            || token.Contains("openvpn", StringComparison.OrdinalIgnoreCase)
            || token.Contains("tap", StringComparison.OrdinalIgnoreCase)
            || token.Contains("vpn", StringComparison.OrdinalIgnoreCase)
            || token.Contains("bluetooth", StringComparison.OrdinalIgnoreCase)
            || token.Contains("loopback", StringComparison.OrdinalIgnoreCase)
            || token.Contains("tunnel", StringComparison.OrdinalIgnoreCase)
            || token.Contains("teredo", StringComparison.OrdinalIgnoreCase)
            || token.Contains("isatap", StringComparison.OrdinalIgnoreCase)
            || token.Contains("pseudo", StringComparison.OrdinalIgnoreCase)
            || token.Contains("miniport", StringComparison.OrdinalIgnoreCase);
    }

    private static ulong? SumNullable(ulong? left, ulong? right)
    {
        if (!left.HasValue)
        {
            return right;
        }

        if (!right.HasValue)
        {
            return left;
        }

        ulong sum = left.Value + right.Value;
        return sum < left.Value ? ulong.MaxValue : sum;
    }

    private sealed record CpuStaticMetadata
    {
        public static CpuStaticMetadata Empty { get; } = new();

        public string? ProcessorName { get; init; }

        public double? CurrentSpeedMHz { get; init; }

        public double? BaseSpeedMHz { get; init; }

        public int? Sockets { get; init; }

        public int? Cores { get; init; }

        public int? LogicalProcessors { get; init; }

        public bool? VirtualizationEnabled { get; init; }

        public ulong? L1CacheBytes { get; init; }

        public ulong? L2CacheBytes { get; init; }

        public ulong? L3CacheBytes { get; init; }
    }

    private sealed record MemoryStaticMetadata
    {
        public static MemoryStaticMetadata Empty { get; } = new();

        public ulong? TotalPhysicalBytes { get; init; }

        public uint? SpeedMTps { get; init; }

        public int? SlotsUsed { get; init; }

        public int? SlotsTotal { get; init; }

        public string? FormFactor { get; init; }
    }

    private sealed record DiskStaticMetadata
    {
        public string? Model { get; init; }

        public string? TypeLabel { get; init; }

        public ulong? CapacityBytes { get; init; }

        public ulong? FormattedBytes { get; init; }
    }

    private sealed record DiskPerformanceSnapshot
    {
        public double? ActiveTimePct { get; init; }

        public double? AvgResponseMs { get; init; }

        public ulong? ReadBps { get; init; }

        public ulong? WriteBps { get; init; }
    }

    private sealed record DiskRawPerformanceSnapshot
    {
        public ulong? AvgDiskSecPerTransfer { get; init; }

        public ulong? AvgDiskSecPerTransferBase { get; init; }

        public ulong? FrequencyPerfTime { get; init; }
    }

    private sealed record NetworkStaticMetadata
    {
        public string? AdapterId { get; init; }

        public string? DisplayName { get; init; }

        public string? AdapterName { get; init; }

        public string? ConnectionType { get; init; }

        public string? IPv4Address { get; init; }

        public string? IPv6Address { get; init; }

        public ulong? LinkSpeedBps { get; init; }

        public bool IncludeInGlobalList { get; init; }
    }
}
