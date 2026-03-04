using System.Globalization;
using System.Management;
using System.Net.NetworkInformation;
using System.Runtime.InteropServices;
using System.Text;
using System.Linq;
using BatCave.Core.Domain;

namespace BatCave.Core.Collector;

public sealed partial class WindowsSystemGlobalMetricsSampler
{
    private static readonly TimeSpan MetadataRefreshInterval = TimeSpan.FromSeconds(30);

    private DateTimeOffset _nextMetadataRefreshUtc = DateTimeOffset.MinValue;
    private CpuStaticMetadata _cpuStaticMetadata = CpuStaticMetadata.Empty;
    private MemoryStaticMetadata _memoryStaticMetadata = MemoryStaticMetadata.Empty;
    private Dictionary<int, DiskStaticMetadata> _diskMetadataByIndex = [];
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
            cpuSnapshot = BuildCpuSnapshot(cpuPct);
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

        try
        {
            _cpuStaticMetadata = ReadCpuStaticMetadata();
        }
        catch
        {
            _cpuStaticMetadata = CpuStaticMetadata.Empty;
        }

        try
        {
            _memoryStaticMetadata = ReadMemoryStaticMetadata();
        }
        catch
        {
            _memoryStaticMetadata = MemoryStaticMetadata.Empty;
        }

        try
        {
            _diskMetadataByIndex = ReadDiskStaticMetadata();
        }
        catch
        {
            _diskMetadataByIndex = [];
        }

        try
        {
            _diskIndexByDriveLetter = ReadDiskIndexByDriveLetter();
        }
        catch
        {
            _diskIndexByDriveLetter = new Dictionary<string, int>(StringComparer.OrdinalIgnoreCase);
        }

        try
        {
            _networkMetadataByNormalizedName = ReadNetworkStaticMetadata();
        }
        catch
        {
            _networkMetadataByNormalizedName = [];
        }

        try
        {
            _pageFileDrives = ReadPageFileDrives();
        }
        catch
        {
            _pageFileDrives = [];
        }

        _systemDriveLetter = GetSystemDriveLetter();
    }

    private SystemGlobalCpuSnapshot BuildCpuSnapshot(double? cpuPct)
    {
        List<double> logicalUtilization = [];
        uint? processes = null;
        uint? threads = null;
        uint? handles = null;
        ulong? uptimeSeconds = null;

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

        return new SystemGlobalCpuSnapshot
        {
            ProcessorName = _cpuStaticMetadata.ProcessorName,
            KernelPct = _lastKernelPct,
            SpeedMHz = _cpuStaticMetadata.CurrentSpeedMHz,
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

    private SystemGlobalMemorySnapshot BuildMemorySnapshot(ulong? memoryUsedBytes)
    {
        ulong? totalBytes = null;
        ulong? availableBytes = null;
        ulong? hardwareReservedBytes = null;
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
                totalBytes = memoryStatus.ullTotalPhys;
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

        if (totalBytes.HasValue && _memoryStaticMetadata.TotalPhysicalBytes.HasValue && _memoryStaticMetadata.TotalPhysicalBytes.Value > totalBytes.Value)
        {
            hardwareReservedBytes = _memoryStaticMetadata.TotalPhysicalBytes.Value - totalBytes.Value;
        }

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
        using ManagementObjectSearcher searcher = new(
            "SELECT Name, PercentDiskTime, PercentIdleTime, AvgDisksecPerTransfer, DiskReadBytesPersec, DiskWriteBytesPersec FROM Win32_PerfFormattedData_PerfDisk_LogicalDisk");
        using ManagementObjectCollection rows = searcher.Get();

        foreach (ManagementBaseObject row in rows)
        {
            string? name = row["Name"] as string;
            if (string.IsNullOrWhiteSpace(name))
            {
                continue;
            }

            string driveLetter = NormalizeDriveLetter(name);
            if (string.IsNullOrWhiteSpace(driveLetter))
            {
                continue;
            }

            string diskId = driveLetter;
            int? diskIndex = ResolveDiskIndexForDrive(driveLetter);
            _diskMetadataByIndex.TryGetValue(diskIndex ?? -1, out DiskStaticMetadata? metadata);
            physicalPerfByIndex.TryGetValue(diskIndex ?? -1, out DiskPerformanceSnapshot? physicalPerf);

            bool? isSystemDisk = TryResolveDriveFlag(driveLetter, _systemDriveLetter);
            bool? hasPageFile = TryResolvePageFileFlag(driveLetter);
            double? avgResponseMs = ReadDouble(row["AvgDisksecPerTransfer"]);
            if (avgResponseMs.HasValue)
            {
                avgResponseMs *= 1000d;
            }

            ulong? read = ReadULong(row["DiskReadBytesPersec"]);
            ulong? write = ReadULong(row["DiskWriteBytesPersec"]);
            double? logicalActiveTimePct = ResolveDiskActiveTimePct(row);

            result.Add(new SystemGlobalDiskSnapshot
            {
                DiskId = diskId,
                DisplayName = BuildDiskDisplayName(diskIndex, driveLetter),
                Model = metadata?.Model,
                TypeLabel = ResolveDiskTypeLabel(metadata?.TypeLabel, metadata?.Model),
                ActiveTimePct = ResolvePreferredDiskActiveTimePct(physicalPerf?.ActiveTimePct, logicalActiveTimePct),
                AvgResponseMs = avgResponseMs,
                ReadBps = read,
                WriteBps = write,
                CapacityBytes = metadata?.CapacityBytes,
                FormattedBytes = metadata?.FormattedBytes,
                IsSystemDisk = isSystemDisk,
                HasPageFile = hasPageFile,
            });
        }

        return result.OrderBy(static snapshot => snapshot.DisplayName, StringComparer.OrdinalIgnoreCase).ToList();
    }

    private static Dictionary<int, DiskPerformanceSnapshot> ReadPhysicalDiskPerfByIndex()
    {
        Dictionary<int, DiskPerformanceSnapshot> result = [];

        try
        {
            using ManagementObjectSearcher searcher = new(
                "SELECT Name, PercentDiskTime, PercentIdleTime, AvgDisksecPerTransfer, DiskReadBytesPersec, DiskWriteBytesPersec FROM Win32_PerfFormattedData_PerfDisk_PhysicalDisk");
            using ManagementObjectCollection rows = searcher.Get();

            foreach (ManagementBaseObject row in rows)
            {
                string? name = row["Name"] as string;
                if (string.IsNullOrWhiteSpace(name) || string.Equals(name, "_Total", StringComparison.OrdinalIgnoreCase))
                {
                    continue;
                }

                int? index = ParseDiskIndex(name);
                if (!index.HasValue)
                {
                    continue;
                }

                double? active = ResolveDiskActiveTimePct(
                    diskTimePct: ReadDouble(row["PercentDiskTime"]),
                    idleTimePct: ReadDouble(row["PercentIdleTime"]));
                double? avgResponseMs = ReadDouble(row["AvgDisksecPerTransfer"]);
                if (avgResponseMs.HasValue)
                {
                    avgResponseMs *= 1000d;
                }

                ulong? read = ReadULong(row["DiskReadBytesPersec"]);
                ulong? write = ReadULong(row["DiskWriteBytesPersec"]);

                if (result.TryGetValue(index.Value, out DiskPerformanceSnapshot? existing))
                {
                    result[index.Value] = new DiskPerformanceSnapshot
                    {
                        ActiveTimePct = ResolvePreferredDiskActiveTimePct(active, existing.ActiveTimePct),
                        AvgResponseMs = MergeMaxNullable(avgResponseMs, existing.AvgResponseMs),
                        ReadBps = SumNullable(existing.ReadBps, read),
                        WriteBps = SumNullable(existing.WriteBps, write),
                    };
                    continue;
                }

                result[index.Value] = new DiskPerformanceSnapshot
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
            L2CacheBytes = ScaleKbToBytes(ReadULong(row["L2CacheSize"])),
            L3CacheBytes = ScaleKbToBytes(ReadULong(row["L3CacheSize"])),
        };
    }

    private static MemoryStaticMetadata ReadMemoryStaticMetadata()
    {
        using ManagementObjectSearcher computerSearcher = new("SELECT TotalPhysicalMemory FROM Win32_ComputerSystem");
        using ManagementObjectCollection computerRows = computerSearcher.Get();
        ManagementBaseObject? computer = computerRows.Cast<ManagementBaseObject>().FirstOrDefault();

        ulong? totalBytes = ReadULong(computer?["TotalPhysicalMemory"]);
        uint? speed = null;
        int slotsUsed = 0;
        int slotsTotal = 0;
        string? formFactor = null;

        using ManagementObjectSearcher physicalMemorySearcher = new("SELECT Speed, FormFactor FROM Win32_PhysicalMemory");
        using ManagementObjectCollection physicalRows = physicalMemorySearcher.Get();
        foreach (ManagementBaseObject row in physicalRows)
        {
            slotsTotal++;
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

    private static string BuildDiskDisplayName(int? diskIndex, string driveLetter)
    {
        return diskIndex.HasValue
            ? $"Disk {diskIndex.Value} ({driveLetter})"
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

    private static double? MergeMaxNullable(double? left, double? right)
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
