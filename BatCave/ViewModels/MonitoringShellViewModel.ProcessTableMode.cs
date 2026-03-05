using BatCave.Core.Domain;
using BatCave.Styling;
using CommunityToolkit.Mvvm.Input;
using Microsoft.UI.Xaml.Media;
using System;
using System.Runtime.InteropServices;
using Windows.UI;

namespace BatCave.ViewModels;

public partial class MonitoringShellViewModel
{
    public string CompactNameSortLabel => SortLabel("Name", SortColumn.Name);

    public string CompactCpuSortLabel => SortLabel("CPU", SortColumn.CpuPct);

    public string CompactMemorySortLabel => SortLabel("Memory", SortColumn.RssBytes);

    public string CompactDiskSortLabel => SortLabel("Disk", SortColumn.DiskBps);

    public string CompactNetworkSortLabel => SortLabel("Network", SortColumn.OtherIoBps);

    public string CompactNameTotalLabel => BuildProcessCountLabel(VisibleRows.Count);

    public string CompactCpuTotalLabel => FormatPercentOrNa(_latestGlobalMetricsSample.CpuPct);

    public string CompactMemoryTotalLabel => FormatPercentOrNa(ComputeMemoryUsagePercent());

    public string CompactDiskTotalLabel => FormatPercentOrNa(ComputeDiskUsagePercent());

    public string CompactNetworkTotalLabel => FormatPercentOrNa(ComputeNetworkUsagePercent());

    public bool IsCompactNameSortActive => CurrentSortColumn == SortColumn.Name;

    public bool IsCompactCpuSortActive => CurrentSortColumn == SortColumn.CpuPct;

    public bool IsCompactMemorySortActive => CurrentSortColumn == SortColumn.RssBytes;

    public bool IsCompactDiskSortActive => CurrentSortColumn == SortColumn.DiskBps;

    public bool IsCompactNetworkSortActive => CurrentSortColumn == SortColumn.OtherIoBps;

    public Brush? CompactNameSortForeground => ResolveCompactSortForeground(IsCompactNameSortActive);

    public Brush? CompactCpuSortForeground => ResolveCompactSortForeground(IsCompactCpuSortActive);

    public Brush? CompactMemorySortForeground => ResolveCompactSortForeground(IsCompactMemorySortActive);

    public Brush? CompactDiskSortForeground => ResolveCompactSortForeground(IsCompactDiskSortActive);

    public Brush? CompactNetworkSortForeground => ResolveCompactSortForeground(IsCompactNetworkSortActive);

    public Brush? CompactNameColumnBackground => ResolveCompactColumnBackground(IsCompactNameSortActive);

    public Brush? CompactCpuColumnBackground => ResolveCompactColumnBackground(IsCompactCpuSortActive);

    public Brush? CompactMemoryColumnBackground => ResolveCompactColumnBackground(IsCompactMemorySortActive);

    public Brush? CompactDiskColumnBackground => ResolveCompactColumnBackground(IsCompactDiskSortActive);

    public Brush? CompactNetworkColumnBackground => ResolveCompactColumnBackground(IsCompactNetworkSortActive);

    [RelayCommand]
    private void CompactSortHeader(string? sortTag)
    {
        if (!Enum.TryParse(sortTag, out SortColumn column))
        {
            return;
        }

        ChangeSort(column);
    }

    private void RaiseCompactSortVisualProperties()
    {
        RaiseProperties(
            nameof(CompactNameSortLabel),
            nameof(CompactCpuSortLabel),
            nameof(CompactMemorySortLabel),
            nameof(CompactDiskSortLabel),
            nameof(CompactNetworkSortLabel),
            nameof(IsCompactNameSortActive),
            nameof(IsCompactCpuSortActive),
            nameof(IsCompactMemorySortActive),
            nameof(IsCompactDiskSortActive),
            nameof(IsCompactNetworkSortActive),
            nameof(CompactNameSortForeground),
            nameof(CompactCpuSortForeground),
            nameof(CompactMemorySortForeground),
            nameof(CompactDiskSortForeground),
            nameof(CompactNetworkSortForeground),
            nameof(CompactNameColumnBackground),
            nameof(CompactCpuColumnBackground),
            nameof(CompactMemoryColumnBackground),
            nameof(CompactDiskColumnBackground),
            nameof(CompactNetworkColumnBackground));
    }

    private void RaiseCompactTotalsProperties()
    {
        RaiseProperties(
            nameof(CompactNameTotalLabel),
            nameof(CompactCpuTotalLabel),
            nameof(CompactMemoryTotalLabel),
            nameof(CompactDiskTotalLabel),
            nameof(CompactNetworkTotalLabel));
    }

    private static string BuildProcessCountLabel(int processCount)
    {
        string noun = processCount == 1 ? "process" : "processes";
        return $"{processCount:N0} {noun}";
    }

    private static Brush? ResolveCompactSortForeground(bool isActive)
    {
        return isActive
            ? ResolveThemeBrushOrNull("AccentTextFillColorPrimaryBrush", Color.FromArgb(0xFF, 0x0B, 0x84, 0xD8))
            : ResolveThemeBrushOrNull("TextFillColorPrimaryBrush", Color.FromArgb(0xFF, 0xE5, 0xE5, 0xE5));
    }

    private static Brush? ResolveCompactColumnBackground(bool isActive)
    {
        return isActive
            ? ResolveThemeBrushOrNull("SubtleFillColorSecondaryBrush", Color.FromArgb(0x22, 0x0B, 0x84, 0xD8))
            : null;
    }

    private double? ComputeMemoryUsagePercent()
    {
        SystemGlobalMemorySnapshot? memory = _latestGlobalMetricsSample.MemorySnapshot;
        if (memory?.UsedBytes is not ulong used || memory.TotalBytes is not ulong total || total == 0)
        {
            return null;
        }

        return used * 100d / total;
    }

    private double? ComputeDiskUsagePercent()
    {
        double? max = null;
        foreach (SystemGlobalDiskSnapshot disk in _latestGlobalMetricsSample.DiskSnapshots)
        {
            double? normalized = NormalizeNonNegativeFiniteMetric(disk.ActiveTimePct);
            if (!normalized.HasValue)
            {
                continue;
            }

            max = !max.HasValue ? normalized.Value : Math.Max(max.Value, normalized.Value);
        }

        return max;
    }

    private double? ComputeNetworkUsagePercent()
    {
        double? max = null;
        foreach (SystemGlobalNetworkSnapshot adapter in _latestGlobalMetricsSample.NetworkSnapshots)
        {
            if (!IsVisibleNetworkAdapter(adapter))
            {
                continue;
            }

            if (adapter.LinkSpeedBps is not ulong linkSpeed || linkSpeed == 0)
            {
                continue;
            }

            ulong send = adapter.SendBps ?? 0UL;
            ulong receive = adapter.ReceiveBps ?? 0UL;
            ulong total = SaturatingAdd(send, receive);
            double utilizationPct = total * 100d / linkSpeed;
            max = !max.HasValue ? utilizationPct : Math.Max(max.Value, utilizationPct);
        }

        return max;
    }

    private static string FormatPercentOrNa(double? value)
    {
        double? normalized = NormalizeNonNegativeFiniteMetric(value);
        if (!normalized.HasValue)
        {
            return "n/a";
        }

        return $"{Math.Clamp(normalized.Value, 0d, 100d):F0}%";
    }

    private static ulong SaturatingAdd(ulong left, ulong right)
    {
        ulong maxAdd = ulong.MaxValue - left;
        return right > maxAdd ? ulong.MaxValue : left + right;
    }

    private static Brush? ResolveThemeBrushOrNull(string key, Color fallback)
    {
        try
        {
            return AppThemeTokens.ResolveBrush(key, fallback);
        }
        catch (COMException)
        {
            return null;
        }
    }
}
