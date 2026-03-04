using System;
using System.Collections.Generic;
using System.Collections.ObjectModel;
using System.Diagnostics;
using System.Globalization;
using System.Linq;
using System.Threading;
using BatCave.Controls;
using BatCave.Converters;
using BatCave.Core.Domain;
using CommunityToolkit.Mvvm.Input;
using Microsoft.UI.Dispatching;
using Microsoft.UI.Xaml;
using Windows.UI;

namespace BatCave.ViewModels;

public partial class MonitoringShellViewModel
{
    private const string CpuGlobalResourceId = "cpu";
    private const string MemoryGlobalResourceId = "memory";
    private static readonly TimeSpan GlobalResourceStaleRetention = TimeSpan.FromMinutes(5);
    private static readonly Color CpuStrokeColor = Color.FromArgb(0xFF, 0x0B, 0x84, 0xD8);
    private static readonly Color CpuFillColor = Color.FromArgb(0x33, 0x0B, 0x84, 0xD8);
    private static readonly Color CpuKernelStrokeColor = Color.FromArgb(0xFF, 0x07, 0x5C, 0x8F);
    private static readonly Color MemoryStrokeColor = Color.FromArgb(0xFF, 0x25, 0x63, 0xEB);
    private static readonly Color MemoryFillColor = Color.FromArgb(0x33, 0x25, 0x63, 0xEB);
    private static readonly Color DiskStrokeColor = Color.FromArgb(0xFF, 0x6A, 0x9F, 0x2A);
    private static readonly Color DiskFillColor = Color.FromArgb(0x33, 0x6A, 0x9F, 0x2A);
    private static readonly Color NetworkStrokeColor = Color.FromArgb(0xFF, 0xD8, 0x1B, 0x60);
    private static readonly Color NetworkFillColor = Color.FromArgb(0x33, 0xD8, 0x1B, 0x60);
    private static readonly Color NetworkOverlayStrokeColor = Color.FromArgb(0xFF, 0xA1, 0x14, 0x49);

    private readonly ObservableCollection<GlobalResourceRowViewState> _globalResourceRows = [];
    private readonly Dictionary<string, GlobalTrendHistory> _globalTrendByResourceId = new(StringComparer.OrdinalIgnoreCase);
    private readonly Dictionary<string, DateTimeOffset> _globalResourceLastSeenUtc = new(StringComparer.OrdinalIgnoreCase);
    private readonly ObservableCollection<GlobalStatItemViewState> _globalDetailStats = [];
    private readonly ObservableCollection<LogicalProcessorTrendViewState> _globalCpuLogicalProcessorRows = [];

    private SystemGlobalMetricsSample _latestGlobalMetricsSample = new();
    private GlobalResourceRowViewState? _selectedGlobalResource;
    private CpuGraphMode _cpuGraphMode = CpuGraphMode.Combined;
    private bool _isRefreshingGlobalDetailState;
    private int _globalDetailRefreshQueued;

    private string _globalDetailTitle = "CPU";
    private string _globalDetailSubtitle = "System";
    private string _globalDetailCurrentValue = "0%";
    private string _globalPrimaryChartTitle = "Utilization";
    private string _globalAuxiliaryChartTitle = "Transfer rate";
    private bool _globalShowSecondaryOverlay;
    private bool _globalShowAuxiliaryChart;
    private MetricTrendScaleMode _globalPrimaryScaleMode = MetricTrendScaleMode.CpuPercent;
    private MetricTrendScaleMode _globalAuxiliaryScaleMode = MetricTrendScaleMode.IoRate;
    private double[] _globalPrimaryTrendValues = [];
    private double[] _globalSecondaryTrendValues = [];
    private double[] _globalAuxiliaryTrendValues = [];
    private Color _globalPrimaryStrokeColor = CpuStrokeColor;
    private Color _globalPrimaryFillColor = CpuFillColor;
    private Color _globalSecondaryStrokeColor = CpuKernelStrokeColor;
    private Color _globalAuxiliaryStrokeColor = DiskStrokeColor;
    private Color _globalAuxiliaryFillColor = DiskFillColor;
    private double _globalPrimaryDomainMax = double.NaN;
    private double _globalAuxiliaryDomainMax = double.NaN;

    public ObservableCollection<GlobalResourceRowViewState> GlobalResourceRows => _globalResourceRows;

    public GlobalResourceRowViewState? SelectedGlobalResource
    {
        get => _selectedGlobalResource;
        set
        {
            if (!SetProperty(ref _selectedGlobalResource, value))
            {
                return;
            }

            try
            {
                OnPropertyChanged(nameof(IsCpuResourceSelected));
                OnPropertyChanged(nameof(GlobalCpuModeToggleVisibility));
                OnPropertyChanged(nameof(GlobalCombinedChartVisibility));
                OnPropertyChanged(nameof(GlobalCpuLogicalGridVisibility));
                QueueGlobalDetailStateRefresh();
            }
            catch (Exception ex)
            {
                Debug.WriteLine($"[GlobalSelection] Failed to apply selection for {value?.ResourceId ?? "<null>"}. {ex}");
                ApplyDetailFallbackState(value);
            }
        }
    }

    public ObservableCollection<GlobalStatItemViewState> GlobalDetailStats => _globalDetailStats;

    public ObservableCollection<LogicalProcessorTrendViewState> GlobalCpuLogicalProcessorRows => _globalCpuLogicalProcessorRows;

    public bool IsGlobalPerformanceMode => SelectedRow is null;

    public Visibility GlobalPerformanceVisibility => IsGlobalPerformanceMode ? Visibility.Visible : Visibility.Collapsed;

    public bool IsGlobalMetricsReady => _latestGlobalMetricsSample.IsReady;

    public Visibility GlobalPerformanceContentVisibility =>
        IsGlobalPerformanceMode && IsGlobalMetricsReady ? Visibility.Visible : Visibility.Collapsed;

    public Visibility GlobalPerformanceSkeletonVisibility =>
        IsGlobalPerformanceMode && !IsGlobalMetricsReady ? Visibility.Visible : Visibility.Collapsed;

    public Visibility ProcessDetailVisibility => IsGlobalPerformanceMode ? Visibility.Collapsed : Visibility.Visible;

    public string GlobalDetailTitle
    {
        get => _globalDetailTitle;
        private set => SetProperty(ref _globalDetailTitle, value);
    }

    public string GlobalDetailSubtitle
    {
        get => _globalDetailSubtitle;
        private set => SetProperty(ref _globalDetailSubtitle, value);
    }

    public string GlobalDetailCurrentValue
    {
        get => _globalDetailCurrentValue;
        private set => SetProperty(ref _globalDetailCurrentValue, value);
    }

    public string GlobalPrimaryChartTitle
    {
        get => _globalPrimaryChartTitle;
        private set => SetProperty(ref _globalPrimaryChartTitle, value);
    }

    public string GlobalAuxiliaryChartTitle
    {
        get => _globalAuxiliaryChartTitle;
        private set => SetProperty(ref _globalAuxiliaryChartTitle, value);
    }

    public bool GlobalShowSecondaryOverlay
    {
        get => _globalShowSecondaryOverlay;
        private set => SetProperty(ref _globalShowSecondaryOverlay, value);
    }

    public bool GlobalShowAuxiliaryChart
    {
        get => _globalShowAuxiliaryChart;
        private set
        {
            if (SetProperty(ref _globalShowAuxiliaryChart, value))
            {
                OnPropertyChanged(nameof(GlobalAuxiliaryChartVisibility));
            }
        }
    }

    public Visibility GlobalAuxiliaryChartVisibility => GlobalShowAuxiliaryChart ? Visibility.Visible : Visibility.Collapsed;

    public MetricTrendScaleMode GlobalPrimaryScaleMode
    {
        get => _globalPrimaryScaleMode;
        private set => SetProperty(ref _globalPrimaryScaleMode, value);
    }

    public MetricTrendScaleMode GlobalAuxiliaryScaleMode
    {
        get => _globalAuxiliaryScaleMode;
        private set => SetProperty(ref _globalAuxiliaryScaleMode, value);
    }

    public double[] GlobalPrimaryTrendValues
    {
        get => _globalPrimaryTrendValues;
        private set => SetProperty(ref _globalPrimaryTrendValues, value);
    }

    public double[] GlobalSecondaryTrendValues
    {
        get => _globalSecondaryTrendValues;
        private set => SetProperty(ref _globalSecondaryTrendValues, value);
    }

    public double[] GlobalAuxiliaryTrendValues
    {
        get => _globalAuxiliaryTrendValues;
        private set => SetProperty(ref _globalAuxiliaryTrendValues, value);
    }

    public Color GlobalPrimaryStrokeColor
    {
        get => _globalPrimaryStrokeColor;
        private set => SetProperty(ref _globalPrimaryStrokeColor, value);
    }

    public Color GlobalPrimaryFillColor
    {
        get => _globalPrimaryFillColor;
        private set => SetProperty(ref _globalPrimaryFillColor, value);
    }

    public Color GlobalSecondaryStrokeColor
    {
        get => _globalSecondaryStrokeColor;
        private set => SetProperty(ref _globalSecondaryStrokeColor, value);
    }

    public Color GlobalAuxiliaryStrokeColor
    {
        get => _globalAuxiliaryStrokeColor;
        private set => SetProperty(ref _globalAuxiliaryStrokeColor, value);
    }

    public Color GlobalAuxiliaryFillColor
    {
        get => _globalAuxiliaryFillColor;
        private set => SetProperty(ref _globalAuxiliaryFillColor, value);
    }

    public double GlobalPrimaryDomainMax
    {
        get => _globalPrimaryDomainMax;
        private set => SetProperty(ref _globalPrimaryDomainMax, value);
    }

    public double GlobalAuxiliaryDomainMax
    {
        get => _globalAuxiliaryDomainMax;
        private set => SetProperty(ref _globalAuxiliaryDomainMax, value);
    }

    public CpuGraphMode CpuGraphMode
    {
        get => _cpuGraphMode;
        private set
        {
            if (SetProperty(ref _cpuGraphMode, value))
            {
                OnPropertyChanged(nameof(IsCpuCombinedMode));
                OnPropertyChanged(nameof(IsCpuLogicalMode));
                OnPropertyChanged(nameof(IsCpuResourceSelected));
                OnPropertyChanged(nameof(GlobalCpuModeToggleVisibility));
                OnPropertyChanged(nameof(GlobalCombinedChartVisibility));
                OnPropertyChanged(nameof(GlobalCpuLogicalGridVisibility));
            }
        }
    }

    public bool IsCpuResourceSelected => SelectedGlobalResource?.Kind == GlobalResourceKind.Cpu;

    public bool IsCpuCombinedMode => CpuGraphMode == CpuGraphMode.Combined;

    public bool IsCpuLogicalMode => CpuGraphMode == CpuGraphMode.LogicalProcessors;

    public Visibility GlobalCpuModeToggleVisibility => IsCpuResourceSelected ? Visibility.Visible : Visibility.Collapsed;

    public Visibility GlobalCombinedChartVisibility => IsCpuResourceSelected && IsCpuLogicalMode ? Visibility.Collapsed : Visibility.Visible;

    public Visibility GlobalCpuLogicalGridVisibility => IsCpuResourceSelected && IsCpuLogicalMode ? Visibility.Visible : Visibility.Collapsed;

    [RelayCommand]
    private void CpuGraphModeSelected(string? modeTag)
    {
        if (!Enum.TryParse(modeTag, out CpuGraphMode parsed))
        {
            return;
        }

        bool changed = CpuGraphMode != parsed;
        CpuGraphMode = parsed;
        if (!changed)
        {
            // ToggleButtons can change local IsChecked state even when the mode is unchanged.
            // Re-raise mode booleans so bindings snap back to the authoritative enum state.
            OnPropertyChanged(nameof(IsCpuCombinedMode));
            OnPropertyChanged(nameof(IsCpuLogicalMode));
        }

        QueueGlobalDetailStateRefresh();
    }

    private void RaiseGlobalModeProperties()
    {
        RaiseProperties(
            nameof(IsGlobalPerformanceMode),
            nameof(GlobalPerformanceVisibility),
            nameof(IsGlobalMetricsReady),
            nameof(GlobalPerformanceContentVisibility),
            nameof(GlobalPerformanceSkeletonVisibility),
            nameof(ProcessDetailVisibility));
    }

    private void RefreshGlobalPerformanceState(SystemGlobalMetricsSample sampled)
    {
        DispatcherQueue? dispatcherQueue = _dispatcherQueue;
        if (dispatcherQueue is not null && !dispatcherQueue.HasThreadAccess)
        {
            _ = dispatcherQueue.TryEnqueue(() => RefreshGlobalPerformanceState(sampled));
            return;
        }

        _latestGlobalMetricsSample = sampled;
        OnPropertyChanged(nameof(IsGlobalMetricsReady));
        OnPropertyChanged(nameof(GlobalPerformanceContentVisibility));
        OnPropertyChanged(nameof(GlobalPerformanceSkeletonVisibility));
        BuildAndAppendResourceRows(sampled);
        QueueGlobalDetailStateRefresh();
    }

    private void BuildAndAppendResourceRows(SystemGlobalMetricsSample sampled)
    {
        string? selectedResourceId = SelectedGlobalResource?.ResourceId;
        List<GlobalResourceDescriptor> descriptors = BuildResourceDescriptors(sampled);
        DateTimeOffset now = DateTimeOffset.UtcNow;
        HashSet<string> currentIds = descriptors
            .Select(static d => d.ResourceId)
            .ToHashSet(StringComparer.OrdinalIgnoreCase);

        foreach (string resourceId in currentIds)
        {
            _globalResourceLastSeenUtc[resourceId] = now;
        }

        for (int index = _globalResourceRows.Count - 1; index >= 0; index--)
        {
            GlobalResourceRowViewState row = _globalResourceRows[index];
            if (currentIds.Contains(row.ResourceId))
            {
                continue;
            }

            if (!CanRemoveStaleGlobalResource(row.ResourceId, now))
            {
                continue;
            }

            _globalResourceRows.RemoveAt(index);
            _globalTrendByResourceId.Remove(row.ResourceId);
            _globalResourceLastSeenUtc.Remove(row.ResourceId);
        }

        foreach (GlobalResourceDescriptor descriptor in descriptors)
        {
            GlobalTrendHistory history = GetOrCreateGlobalTrendHistory(descriptor.ResourceId);
            history.Append(descriptor.PrimaryValue, descriptor.SecondaryValue, descriptor.AuxiliaryValue, descriptor.LogicalValues);
            double[] miniTrend = history.Primary.SliceLatest(60);

            GlobalResourceRowViewState? existing = _globalResourceRows.FirstOrDefault(
                row => string.Equals(row.ResourceId, descriptor.ResourceId, StringComparison.OrdinalIgnoreCase));
            if (existing is null)
            {
                _globalResourceRows.Add(new GlobalResourceRowViewState(
                    descriptor.ResourceId,
                    descriptor.Kind,
                    descriptor.Title,
                    descriptor.Subtitle,
                    descriptor.ValueText,
                    miniTrend,
                    descriptor.MiniScaleMode,
                    descriptor.MiniStrokeColor,
                    descriptor.MiniFillColor,
                    descriptor.MiniDomainMax));
            }
            else
            {
                existing.Update(
                    descriptor.Subtitle,
                    descriptor.ValueText,
                    miniTrend,
                    descriptor.MiniScaleMode,
                    descriptor.MiniStrokeColor,
                    descriptor.MiniFillColor,
                    descriptor.MiniDomainMax);
            }
        }

        ReconcileSelectedGlobalResource(selectedResourceId);
    }

    private void QueueGlobalDetailStateRefresh()
    {
        if (Interlocked.Exchange(ref _globalDetailRefreshQueued, 1) == 1)
        {
            return;
        }

        RunOnUiThread(() =>
        {
            Interlocked.Exchange(ref _globalDetailRefreshQueued, 0);
            RefreshGlobalDetailState();
        });
    }

    private bool CanRemoveStaleGlobalResource(string resourceId, DateTimeOffset nowUtc)
    {
        if (string.Equals(resourceId, CpuGlobalResourceId, StringComparison.OrdinalIgnoreCase)
            || string.Equals(resourceId, MemoryGlobalResourceId, StringComparison.OrdinalIgnoreCase))
        {
            return false;
        }

        if (!_globalResourceLastSeenUtc.TryGetValue(resourceId, out DateTimeOffset lastSeenUtc))
        {
            return true;
        }

        return nowUtc - lastSeenUtc >= GlobalResourceStaleRetention;
    }

    private void EnsureSelectedGlobalResource()
    {
        ReconcileSelectedGlobalResource(SelectedGlobalResource?.ResourceId);
    }

    private void ReconcileSelectedGlobalResource(string? preferredResourceId)
    {
        if (!string.IsNullOrWhiteSpace(preferredResourceId))
        {
            GlobalResourceRowViewState? preserved = _globalResourceRows.FirstOrDefault(
                row => string.Equals(row.ResourceId, preferredResourceId, StringComparison.OrdinalIgnoreCase));
            if (preserved is not null)
            {
                if (!ReferenceEquals(SelectedGlobalResource, preserved))
                {
                    SelectedGlobalResource = preserved;
                }

                return;
            }
        }

        GlobalResourceRowViewState? fallback = _globalResourceRows.FirstOrDefault(row => row.ResourceId == CpuGlobalResourceId)
            ?? _globalResourceRows.FirstOrDefault();
        if (!ReferenceEquals(SelectedGlobalResource, fallback))
        {
            SelectedGlobalResource = fallback;
        }
    }

    private List<GlobalResourceDescriptor> BuildResourceDescriptors(SystemGlobalMetricsSample sampled)
    {
        List<GlobalResourceDescriptor> descriptors = new(capacity: 2 + sampled.DiskSnapshots.Count + sampled.NetworkSnapshots.Count);

        double cpuPct = sampled.CpuPct ?? 0d;
        string cpuSpeed = ValueFormat.FormatFrequencyGHz(sampled.CpuSnapshot?.SpeedMHz);
        descriptors.Add(new GlobalResourceDescriptor(
            ResourceId: CpuGlobalResourceId,
            Kind: GlobalResourceKind.Cpu,
            Title: "CPU",
            Subtitle: cpuSpeed == "n/a" ? $"{cpuPct:F0}%" : $"{cpuPct:F0}% {cpuSpeed}",
            ValueText: string.Empty,
            PrimaryValue: cpuPct,
            SecondaryValue: sampled.CpuSnapshot?.KernelPct ?? 0d,
            AuxiliaryValue: 0d,
            LogicalValues: sampled.CpuSnapshot?.LogicalProcessorUtilizationPct ?? [],
            MiniScaleMode: MetricTrendScaleMode.CpuPercent,
            MiniStrokeColor: CpuStrokeColor,
            MiniFillColor: CpuFillColor,
            MiniDomainMax: double.NaN));

        ulong memoryUsedBytes = sampled.MemorySnapshot?.UsedBytes ?? sampled.MemoryUsedBytes ?? 0UL;
        ulong memoryTotalBytes = sampled.MemorySnapshot?.TotalBytes ?? 0UL;
        string memoryValueText = memoryTotalBytes > 0
            ? $"{ValueFormat.FormatBytes(memoryUsedBytes)} / {ValueFormat.FormatBytes(memoryTotalBytes)} ({(memoryUsedBytes * 100d / memoryTotalBytes):F0}%)"
            : ValueFormat.FormatBytes(memoryUsedBytes);
        descriptors.Add(new GlobalResourceDescriptor(
            ResourceId: MemoryGlobalResourceId,
            Kind: GlobalResourceKind.Memory,
            Title: "Memory",
            Subtitle: memoryValueText,
            ValueText: string.Empty,
            PrimaryValue: memoryUsedBytes,
            SecondaryValue: 0d,
            AuxiliaryValue: 0d,
            LogicalValues: [],
            MiniScaleMode: MetricTrendScaleMode.MemoryBytes,
            MiniStrokeColor: MemoryStrokeColor,
            MiniFillColor: MemoryFillColor,
            MiniDomainMax: memoryTotalBytes > 0 ? memoryTotalBytes : double.NaN));

        IEnumerable<SystemGlobalDiskSnapshot> disks = sampled.DiskSnapshots
            .Where(static disk => !string.Equals(disk.DiskId, "_Total", StringComparison.OrdinalIgnoreCase))
            .OrderBy(static disk => disk.DisplayName, StringComparer.OrdinalIgnoreCase);
        foreach (SystemGlobalDiskSnapshot disk in disks)
        {
            double? activeTimePct = NormalizeNonNegativeFiniteMetric(disk.ActiveTimePct);
            double activePct = activeTimePct ?? 0d;
            ulong read = disk.ReadBps ?? 0UL;
            ulong write = disk.WriteBps ?? 0UL;
            double totalRate = SumRatesAsDouble(read, write);
            string subtitle = string.IsNullOrWhiteSpace(disk.TypeLabel) ? "Disk" : disk.TypeLabel!;
            string title = string.IsNullOrWhiteSpace(disk.DisplayName) ? disk.DiskId : disk.DisplayName;
            string activeValueText = FormatDiskRowActiveTimePercent(activeTimePct);
            descriptors.Add(new GlobalResourceDescriptor(
                ResourceId: $"disk:{disk.DiskId}",
                Kind: GlobalResourceKind.Disk,
                Title: title,
                Subtitle: subtitle,
                ValueText: activeValueText,
                PrimaryValue: activePct,
                SecondaryValue: totalRate,
                AuxiliaryValue: totalRate,
                LogicalValues: [],
                MiniScaleMode: MetricTrendScaleMode.CpuPercent,
                MiniStrokeColor: DiskStrokeColor,
                MiniFillColor: DiskFillColor,
                MiniDomainMax: double.NaN));
        }

        List<SystemGlobalNetworkSnapshot> networks = sampled.NetworkSnapshots
            .Where(IsVisibleNetworkAdapter)
            .OrderBy(static adapter => adapter.DisplayName, StringComparer.OrdinalIgnoreCase)
            .ToList();

        foreach ((SystemGlobalNetworkSnapshot adapter, string title) in BuildNetworkDisplayRows(networks))
        {
            ulong receiveBps = adapter.ReceiveBps ?? 0UL;
            ulong sendBps = adapter.SendBps ?? 0UL;
            string subtitle = string.IsNullOrWhiteSpace(adapter.DisplayName)
                ? (string.IsNullOrWhiteSpace(adapter.AdapterName) ? "Adapter" : adapter.AdapterName!)
                : adapter.DisplayName;
            descriptors.Add(new GlobalResourceDescriptor(
                ResourceId: $"net:{adapter.AdapterId}",
                Kind: GlobalResourceKind.Network,
                Title: title,
                Subtitle: subtitle,
                ValueText: $"S: {ValueFormat.FormatBitsRateFromBytes(sendBps)} R: {ValueFormat.FormatBitsRateFromBytes(receiveBps)}",
                PrimaryValue: Math.Max(receiveBps, sendBps) * 8d,
                SecondaryValue: sendBps * 8d,
                AuxiliaryValue: 0d,
                LogicalValues: [],
                MiniScaleMode: MetricTrendScaleMode.BitsRate,
                MiniStrokeColor: NetworkStrokeColor,
                MiniFillColor: NetworkFillColor,
                MiniDomainMax: double.NaN));
        }

        return descriptors;
    }

    private static bool IsVisibleNetworkAdapter(SystemGlobalNetworkSnapshot adapter)
    {
        string name = $"{adapter.DisplayName} {adapter.AdapterName}".Trim();
        if (string.IsNullOrWhiteSpace(name))
        {
            return false;
        }

        return !name.Contains("loopback", StringComparison.OrdinalIgnoreCase)
            && !name.Contains("tunnel", StringComparison.OrdinalIgnoreCase)
            && !name.Contains("isatap", StringComparison.OrdinalIgnoreCase)
            && !name.Contains("teredo", StringComparison.OrdinalIgnoreCase)
            && !name.Contains("pseudo", StringComparison.OrdinalIgnoreCase);
    }

    private GlobalTrendHistory GetOrCreateGlobalTrendHistory(string resourceId)
    {
        if (_globalTrendByResourceId.TryGetValue(resourceId, out GlobalTrendHistory? history))
        {
            return history;
        }

        GlobalTrendHistory created = new(HistoryLimit);
        _globalTrendByResourceId[resourceId] = created;
        return created;
    }

    private void RefreshGlobalDetailState()
    {
        if (!IsGlobalPerformanceMode)
        {
            return;
        }

        if (_isRefreshingGlobalDetailState)
        {
            return;
        }

        GlobalResourceRowViewState? selected = null;
        _isRefreshingGlobalDetailState = true;
        try
        {
            EnsureSelectedGlobalResource();
            selected = SelectedGlobalResource;
            if (selected is null)
            {
                return;
            }

            switch (selected.Kind)
            {
                case GlobalResourceKind.Memory:
                    ApplyMemoryDetailState();
                    break;
                case GlobalResourceKind.Disk:
                    ApplyDiskDetailState(selected.ResourceId);
                    break;
                case GlobalResourceKind.Network:
                    ApplyNetworkDetailState(selected.ResourceId);
                    break;
                default:
                    ApplyCpuDetailState();
                    break;
            }
        }
        catch (Exception ex)
        {
            string selectedKind = selected?.Kind.ToString() ?? "<none>";
            string selectedId = selected?.ResourceId ?? "<none>";
            Debug.WriteLine($"[GlobalDetail] Failed for {selectedKind}:{selectedId}. {ex}");
            ApplyDetailFallbackState(selected);
        }
        finally
        {
            _isRefreshingGlobalDetailState = false;
        }
    }

    private void ApplyCpuDetailState()
    {
        SystemGlobalCpuSnapshot? cpu = _latestGlobalMetricsSample.CpuSnapshot;
        GlobalTrendHistory history = GetOrCreateGlobalTrendHistory(CpuGlobalResourceId);

        GlobalDetailTitle = "CPU";
        GlobalDetailSubtitle = cpu?.ProcessorName ?? "Processor";
        string speed = ValueFormat.FormatFrequencyGHz(cpu?.SpeedMHz);
        string cpuPct = _latestGlobalMetricsSample.CpuPct.HasValue
            ? $"{_latestGlobalMetricsSample.CpuPct.Value:F0}%"
            : "n/a";
        GlobalDetailCurrentValue = speed == "n/a" ? cpuPct : $"{cpuPct} {speed}";

        GlobalPrimaryScaleMode = MetricTrendScaleMode.CpuPercent;
        GlobalAuxiliaryScaleMode = MetricTrendScaleMode.IoRate;
        GlobalPrimaryChartTitle = "Utilization";
        GlobalAuxiliaryChartTitle = "Transfer rate";
        GlobalShowSecondaryOverlay = true;
        GlobalShowAuxiliaryChart = false;
        GlobalPrimaryStrokeColor = CpuStrokeColor;
        GlobalPrimaryFillColor = CpuFillColor;
        GlobalSecondaryStrokeColor = CpuKernelStrokeColor;
        GlobalAuxiliaryStrokeColor = DiskStrokeColor;
        GlobalAuxiliaryFillColor = DiskFillColor;
        GlobalPrimaryDomainMax = double.NaN;
        GlobalAuxiliaryDomainMax = double.NaN;
        GlobalPrimaryTrendValues = history.Primary.SliceLatest(MetricTrendWindowSeconds);
        GlobalSecondaryTrendValues = history.Secondary.SliceLatest(MetricTrendWindowSeconds);
        GlobalAuxiliaryTrendValues = [];

        UpdateCpuLogicalRows(history.LogicalByProcessor, MetricTrendWindowSeconds);
        PopulateCpuStats(cpu);
    }

    private void ApplyMemoryDetailState()
    {
        SystemGlobalMemorySnapshot? memory = _latestGlobalMetricsSample.MemorySnapshot;
        GlobalTrendHistory history = GetOrCreateGlobalTrendHistory(MemoryGlobalResourceId);

        GlobalDetailTitle = "Memory";
        GlobalDetailSubtitle = memory?.TotalBytes.HasValue == true ? ValueFormat.FormatBytes(memory.TotalBytes.Value) : "System memory";
        if (memory?.UsedBytes is ulong used && memory.TotalBytes is ulong total && total > 0)
        {
            GlobalDetailCurrentValue = $"{ValueFormat.FormatBytes(used)} / {ValueFormat.FormatBytes(total)} ({(used * 100d / total):F0}%)";
        }
        else
        {
            GlobalDetailCurrentValue = memory?.UsedBytes is ulong value ? ValueFormat.FormatBytes(value) : "n/a";
        }

        GlobalPrimaryScaleMode = MetricTrendScaleMode.MemoryBytes;
        GlobalAuxiliaryScaleMode = MetricTrendScaleMode.IoRate;
        GlobalPrimaryChartTitle = "Memory usage";
        GlobalAuxiliaryChartTitle = "Transfer rate";
        GlobalShowSecondaryOverlay = false;
        GlobalShowAuxiliaryChart = false;
        GlobalPrimaryStrokeColor = MemoryStrokeColor;
        GlobalPrimaryFillColor = MemoryFillColor;
        GlobalSecondaryStrokeColor = MemoryStrokeColor;
        GlobalAuxiliaryStrokeColor = DiskStrokeColor;
        GlobalAuxiliaryFillColor = DiskFillColor;
        GlobalPrimaryDomainMax = memory?.TotalBytes.HasValue == true && memory.TotalBytes.Value > 0 ? memory.TotalBytes.Value : double.NaN;
        GlobalAuxiliaryDomainMax = double.NaN;
        GlobalPrimaryTrendValues = history.Primary.SliceLatest(MetricTrendWindowSeconds);
        GlobalSecondaryTrendValues = [];
        GlobalAuxiliaryTrendValues = [];

        PopulateMemoryStats(memory);
    }

    private void ApplyDiskDetailState(string resourceId)
    {
        if (!TryParseResourceId(resourceId, "disk:", out string diskId))
        {
            ApplyDetailFallbackState(SelectedGlobalResource);
            return;
        }

        SystemGlobalDiskSnapshot? disk = _latestGlobalMetricsSample.DiskSnapshots.FirstOrDefault(candidate => candidate.DiskId == diskId);
        GlobalTrendHistory history = GetOrCreateGlobalTrendHistory(resourceId);

        GlobalDetailTitle = string.IsNullOrWhiteSpace(disk?.DisplayName) ? "Disk" : disk.DisplayName;
        GlobalDetailSubtitle = string.IsNullOrWhiteSpace(disk?.Model) ? (disk?.TypeLabel ?? "Disk") : disk.Model!;
        double? activeTimePct = NormalizeNonNegativeFiniteMetric(disk?.ActiveTimePct);
        GlobalDetailCurrentValue = activeTimePct.HasValue ? $"{activeTimePct.Value:F1}%" : "n/a";

        GlobalPrimaryScaleMode = MetricTrendScaleMode.CpuPercent;
        GlobalAuxiliaryScaleMode = MetricTrendScaleMode.IoRate;
        GlobalPrimaryChartTitle = "Active time";
        GlobalAuxiliaryChartTitle = "Disk transfer rate";
        GlobalShowSecondaryOverlay = false;

        double[] auxiliaryTrendValues = SanitizeTrendValues(history.Auxiliary.SliceLatest(MetricTrendWindowSeconds));
        bool canShowAuxiliaryChart = auxiliaryTrendValues.Length > 0;
        GlobalShowAuxiliaryChart = canShowAuxiliaryChart;
        GlobalPrimaryStrokeColor = DiskStrokeColor;
        GlobalPrimaryFillColor = DiskFillColor;
        GlobalSecondaryStrokeColor = DiskStrokeColor;
        GlobalAuxiliaryStrokeColor = DiskStrokeColor;
        GlobalAuxiliaryFillColor = DiskFillColor;
        GlobalPrimaryDomainMax = double.NaN;
        GlobalAuxiliaryDomainMax = double.NaN;
        GlobalPrimaryTrendValues = SanitizeTrendValues(history.Primary.SliceLatest(MetricTrendWindowSeconds));
        GlobalSecondaryTrendValues = [];
        GlobalAuxiliaryTrendValues = canShowAuxiliaryChart ? auxiliaryTrendValues : [];

        PopulateDiskStats(disk);
    }

    private void ApplyNetworkDetailState(string resourceId)
    {
        if (!TryParseResourceId(resourceId, "net:", out string adapterId))
        {
            ApplyDetailFallbackState(SelectedGlobalResource);
            return;
        }

        SystemGlobalNetworkSnapshot? network = _latestGlobalMetricsSample.NetworkSnapshots.FirstOrDefault(candidate => candidate.AdapterId == adapterId);
        GlobalTrendHistory history = GetOrCreateGlobalTrendHistory(resourceId);

        GlobalDetailTitle = string.IsNullOrWhiteSpace(network?.DisplayName) ? "Network" : network.DisplayName;
        GlobalDetailSubtitle = string.IsNullOrWhiteSpace(network?.AdapterName) ? "Adapter" : network.AdapterName!;
        if (network is null)
        {
            GlobalDetailCurrentValue = "n/a";
        }
        else
        {
            GlobalDetailCurrentValue = $"S: {ValueFormat.FormatBitsRateFromBytes(network.SendBps ?? 0)} R: {ValueFormat.FormatBitsRateFromBytes(network.ReceiveBps ?? 0)}";
        }

        GlobalPrimaryScaleMode = MetricTrendScaleMode.BitsRate;
        GlobalAuxiliaryScaleMode = MetricTrendScaleMode.IoRate;
        GlobalPrimaryChartTitle = "Throughput";
        GlobalAuxiliaryChartTitle = "Transfer rate";
        GlobalShowSecondaryOverlay = true;
        GlobalShowAuxiliaryChart = false;
        GlobalPrimaryStrokeColor = NetworkStrokeColor;
        GlobalPrimaryFillColor = NetworkFillColor;
        GlobalSecondaryStrokeColor = NetworkOverlayStrokeColor;
        GlobalAuxiliaryStrokeColor = DiskStrokeColor;
        GlobalAuxiliaryFillColor = DiskFillColor;
        GlobalPrimaryDomainMax = double.NaN;
        GlobalAuxiliaryDomainMax = double.NaN;
        GlobalPrimaryTrendValues = history.Primary.SliceLatest(MetricTrendWindowSeconds);
        GlobalSecondaryTrendValues = history.Secondary.SliceLatest(MetricTrendWindowSeconds);
        GlobalAuxiliaryTrendValues = [];

        PopulateNetworkStats(network);
    }

    private void ApplyDetailFallbackState(GlobalResourceRowViewState? selected)
    {
        GlobalDetailTitle = selected?.Title ?? "Performance";
        GlobalDetailSubtitle = selected?.Subtitle ?? "n/a";
        GlobalDetailCurrentValue = "n/a";
        GlobalPrimaryChartTitle = "Activity";
        GlobalAuxiliaryChartTitle = "Transfer rate";
        GlobalShowSecondaryOverlay = false;
        GlobalShowAuxiliaryChart = false;
        GlobalPrimaryScaleMode = MetricTrendScaleMode.CpuPercent;
        GlobalAuxiliaryScaleMode = MetricTrendScaleMode.IoRate;
        GlobalPrimaryStrokeColor = CpuStrokeColor;
        GlobalPrimaryFillColor = CpuFillColor;
        GlobalSecondaryStrokeColor = CpuKernelStrokeColor;
        GlobalAuxiliaryStrokeColor = DiskStrokeColor;
        GlobalAuxiliaryFillColor = DiskFillColor;
        GlobalPrimaryDomainMax = double.NaN;
        GlobalAuxiliaryDomainMax = double.NaN;
        GlobalPrimaryTrendValues = [];
        GlobalSecondaryTrendValues = [];
        GlobalAuxiliaryTrendValues = [];
        SetGlobalStats(new[] { ("Status", "n/a") });
    }

    private void UpdateCpuLogicalRows(IReadOnlyList<FixedRingSeries> logicalSeries, int visiblePointCount)
    {
        if (logicalSeries.Count == 0)
        {
            _globalCpuLogicalProcessorRows.Clear();
            return;
        }

        while (_globalCpuLogicalProcessorRows.Count < logicalSeries.Count)
        {
            int index = _globalCpuLogicalProcessorRows.Count;
            _globalCpuLogicalProcessorRows.Add(new LogicalProcessorTrendViewState($"CPU {index}", []));
        }

        while (_globalCpuLogicalProcessorRows.Count > logicalSeries.Count)
        {
            _globalCpuLogicalProcessorRows.RemoveAt(_globalCpuLogicalProcessorRows.Count - 1);
        }

        for (int index = 0; index < logicalSeries.Count; index++)
        {
            _globalCpuLogicalProcessorRows[index].UpdateValues(logicalSeries[index].SliceLatest(visiblePointCount));
        }
    }

    private void PopulateCpuStats(SystemGlobalCpuSnapshot? cpu)
    {
        SetGlobalStats(new (string, string)[]
        {
            ("Utilization", _latestGlobalMetricsSample.CpuPct.HasValue ? $"{_latestGlobalMetricsSample.CpuPct.Value:F1}%" : "n/a"),
            ("Speed", ValueFormat.FormatFrequencyGHz(cpu?.SpeedMHz)),
            ("Base speed", ValueFormat.FormatFrequencyGHz(cpu?.BaseSpeedMHz)),
            ("Sockets", FormatValue(cpu?.Sockets)),
            ("Cores", FormatValue(cpu?.Cores)),
            ("Logical processors", FormatValue(cpu?.LogicalProcessors)),
            ("Virtualization", cpu?.VirtualizationEnabled.HasValue == true ? (cpu.VirtualizationEnabled.Value ? "Enabled" : "Disabled") : "n/a"),
            ("Processes", FormatValue(cpu?.ProcessCount)),
            ("Threads", FormatValue(cpu?.ThreadCount)),
            ("Handles", FormatValue(cpu?.HandleCount)),
            ("Up time", FormatDuration(cpu?.UptimeSeconds)),
            ("L1 cache", FormatBytesNullable(cpu?.L1CacheBytes)),
            ("L2 cache", FormatBytesNullable(cpu?.L2CacheBytes)),
            ("L3 cache", FormatBytesNullable(cpu?.L3CacheBytes)),
        });
    }

    private void PopulateMemoryStats(SystemGlobalMemorySnapshot? memory)
    {
        SetGlobalStats(new (string, string)[]
        {
            ("In use", FormatBytesNullable(memory?.UsedBytes)),
            ("Available", FormatBytesNullable(memory?.AvailableBytes)),
            ("Committed", FormatCommitted(memory?.CommittedUsedBytes, memory?.CommittedLimitBytes)),
            ("Cached", FormatBytesNullable(memory?.CachedBytes)),
            ("Paged pool", FormatBytesNullable(memory?.PagedPoolBytes)),
            ("Non-paged pool", FormatBytesNullable(memory?.NonPagedPoolBytes)),
            ("Speed", memory?.SpeedMTps.HasValue == true ? $"{memory.SpeedMTps.Value} MT/s" : "n/a"),
            ("Slots used", memory?.SlotsUsed.HasValue == true && memory.SlotsTotal.HasValue ? $"{memory.SlotsUsed.Value} of {memory.SlotsTotal.Value}" : "n/a"),
            ("Form factor", string.IsNullOrWhiteSpace(memory?.FormFactor) ? "n/a" : memory.FormFactor!),
            ("Hardware reserved", FormatBytesNullable(memory?.HardwareReservedBytes)),
        });
    }

    private void PopulateDiskStats(SystemGlobalDiskSnapshot? disk)
    {
        double? activeTimePct = NormalizeNonNegativeFiniteMetric(disk?.ActiveTimePct);
        SetGlobalStats(new (string, string)[]
        {
            ("Active time", activeTimePct.HasValue ? $"{activeTimePct.Value:F1}%" : "n/a"),
            ("Average response time", FormatDiskAverageResponseTimeMs(disk?.AvgResponseMs)),
            ("Read speed", disk?.ReadBps.HasValue == true ? ValueFormat.FormatRate(disk.ReadBps.Value) : "n/a"),
            ("Write speed", disk?.WriteBps.HasValue == true ? ValueFormat.FormatRate(disk.WriteBps.Value) : "n/a"),
            ("Capacity", FormatBytesNullable(disk?.CapacityBytes)),
            ("Formatted", FormatBytesNullable(disk?.FormattedBytes)),
            ("System disk", FormatBool(disk?.IsSystemDisk)),
            ("Page file", FormatBool(disk?.HasPageFile)),
            ("Type", string.IsNullOrWhiteSpace(disk?.TypeLabel) ? "n/a" : disk.TypeLabel!),
        });
    }

    private void PopulateNetworkStats(SystemGlobalNetworkSnapshot? network)
    {
        SetGlobalStats(new (string, string)[]
        {
            ("Send", network?.SendBps.HasValue == true ? ValueFormat.FormatBitsRateFromBytes(network.SendBps.Value) : "n/a"),
            ("Receive", network?.ReceiveBps.HasValue == true ? ValueFormat.FormatBitsRateFromBytes(network.ReceiveBps.Value) : "n/a"),
            ("Adapter name", string.IsNullOrWhiteSpace(network?.AdapterName) ? "n/a" : network.AdapterName!),
            ("Connection type", string.IsNullOrWhiteSpace(network?.ConnectionType) ? "n/a" : network.ConnectionType!),
            ("IPv4 address", string.IsNullOrWhiteSpace(network?.IPv4Address) ? "n/a" : network.IPv4Address!),
            ("IPv6 address", string.IsNullOrWhiteSpace(network?.IPv6Address) ? "n/a" : network.IPv6Address!),
            ("Link speed", network?.LinkSpeedBps.HasValue == true ? ValueFormat.FormatBitsRate(network.LinkSpeedBps.Value) : "n/a"),
        });
    }

    private void SetGlobalStats(IEnumerable<(string Label, string Value)> values)
    {
        List<(string Label, string Value)> snapshot = values.ToList();
        while (_globalDetailStats.Count < snapshot.Count)
        {
            (string label, string value) = snapshot[_globalDetailStats.Count];
            _globalDetailStats.Add(new GlobalStatItemViewState(label, value));
        }

        for (int index = 0; index < snapshot.Count; index++)
        {
            (string label, string value) = snapshot[index];
            if (_globalDetailStats[index].Label != label)
            {
                _globalDetailStats[index] = new GlobalStatItemViewState(label, value);
            }
            else
            {
                _globalDetailStats[index].UpdateValue(value);
            }
        }

        while (_globalDetailStats.Count > snapshot.Count)
        {
            _globalDetailStats.RemoveAt(_globalDetailStats.Count - 1);
        }
    }

    private static string FormatValue<T>(T? value)
        where T : struct
    {
        return value.HasValue ? Convert.ToString(value.Value, CultureInfo.InvariantCulture) ?? "n/a" : "n/a";
    }

    private static string FormatBytesNullable(ulong? value)
    {
        return value.HasValue ? ValueFormat.FormatBytes(value.Value) : "n/a";
    }

    private static string FormatBool(bool? value)
    {
        return value.HasValue ? (value.Value ? "Yes" : "No") : "n/a";
    }

    private static string FormatCommitted(ulong? used, ulong? limit)
    {
        if (!used.HasValue || !limit.HasValue)
        {
            return "n/a";
        }

        return $"{ValueFormat.FormatBytes(used.Value)} / {ValueFormat.FormatBytes(limit.Value)}";
    }

    private static string FormatDiskRowActiveTimePercent(double? value)
    {
        if (!value.HasValue)
        {
            return "n/a";
        }

        return value.Value < 1d
            ? "0%"
            : $"{value.Value:F0}%";
    }

    private static string FormatDiskAverageResponseTimeMs(double? value)
    {
        double? normalized = NormalizeNonNegativeFiniteMetric(value);
        if (!normalized.HasValue)
        {
            return "n/a";
        }

        if (normalized.Value == 0d)
        {
            return "0.0 ms";
        }

        if (normalized.Value < 0.001d)
        {
            return "<0.001 ms";
        }

        return normalized.Value < 0.1d
            ? $"{normalized.Value:F3} ms"
            : $"{normalized.Value:F1} ms";
    }

    private static double? NormalizeNonNegativeFiniteMetric(double? value)
    {
        if (!value.HasValue || !double.IsFinite(value.Value))
        {
            return null;
        }

        return Math.Max(0d, value.Value);
    }

    private static string FormatDuration(ulong? uptimeSeconds)
    {
        if (!uptimeSeconds.HasValue)
        {
            return "n/a";
        }

        TimeSpan span = TimeSpan.FromSeconds(uptimeSeconds.Value);
        return $"{(int)span.TotalDays}:{span:hh\\:mm\\:ss}";
    }

    private readonly record struct GlobalResourceDescriptor(
        string ResourceId,
        GlobalResourceKind Kind,
        string Title,
        string Subtitle,
        string ValueText,
        double PrimaryValue,
        double SecondaryValue,
        double AuxiliaryValue,
        IReadOnlyList<double> LogicalValues,
        MetricTrendScaleMode MiniScaleMode,
        Color MiniStrokeColor,
        Color MiniFillColor,
        double MiniDomainMax);

    private static bool TryParseResourceId(string resourceId, string prefix, out string id)
    {
        id = string.Empty;
        if (string.IsNullOrWhiteSpace(resourceId) || !resourceId.StartsWith(prefix, StringComparison.OrdinalIgnoreCase))
        {
            return false;
        }

        id = resourceId[prefix.Length..];
        return !string.IsNullOrWhiteSpace(id);
    }

    private static IEnumerable<(SystemGlobalNetworkSnapshot Adapter, string Title)> BuildNetworkDisplayRows(IEnumerable<SystemGlobalNetworkSnapshot> adapters)
    {
        List<SystemGlobalNetworkSnapshot> sorted = adapters
            .OrderBy(static adapter => adapter.DisplayName, StringComparer.OrdinalIgnoreCase)
            .ToList();

        foreach (IGrouping<string, SystemGlobalNetworkSnapshot> group in sorted.GroupBy(GetNetworkBaseTitle, StringComparer.OrdinalIgnoreCase))
        {
            int index = 1;
            foreach (SystemGlobalNetworkSnapshot adapter in group)
            {
                string title = index == 1 ? group.Key : $"{group.Key} {index}";
                index++;
                yield return (adapter, title);
            }
        }
    }

    private static string GetNetworkBaseTitle(SystemGlobalNetworkSnapshot adapter)
    {
        return IsWirelessAdapter(adapter) ? "WiFi" : "Ethernet";
    }

    private static bool IsWirelessAdapter(SystemGlobalNetworkSnapshot adapter)
    {
        string token = $"{adapter.ConnectionType} {adapter.DisplayName} {adapter.AdapterName}".Trim();
        return token.Contains("wireless", StringComparison.OrdinalIgnoreCase)
            || token.Contains("wi-fi", StringComparison.OrdinalIgnoreCase)
            || token.Contains("wifi", StringComparison.OrdinalIgnoreCase)
            || token.Contains("wlan", StringComparison.OrdinalIgnoreCase);
    }

    private static double SumRatesAsDouble(ulong left, ulong right)
    {
        double sum = left + right;
        return double.IsFinite(sum) ? sum : double.MaxValue;
    }

    private static double[] SanitizeTrendValues(IReadOnlyList<double> values)
    {
        if (values.Count == 0)
        {
            return [];
        }

        double[] sanitized = new double[values.Count];
        for (int index = 0; index < values.Count; index++)
        {
            double value = values[index];
            sanitized[index] = double.IsFinite(value) && value >= 0d ? value : 0d;
        }

        return sanitized;
    }

    private sealed class GlobalTrendHistory
    {
        private readonly int _capacity;

        public GlobalTrendHistory(int capacity)
        {
            _capacity = Math.Max(1, capacity);
            Primary = new FixedRingSeries(capacity);
            Secondary = new FixedRingSeries(capacity);
            Auxiliary = new FixedRingSeries(capacity);
        }

        public FixedRingSeries Primary { get; }

        public FixedRingSeries Secondary { get; }

        public FixedRingSeries Auxiliary { get; }

        public List<FixedRingSeries> LogicalByProcessor { get; } = [];

        public void Append(double primary, double secondary, double auxiliary, IReadOnlyList<double> logicalValues)
        {
            Primary.Add(NormalizeSeriesValue(primary));
            Secondary.Add(NormalizeSeriesValue(secondary));
            Auxiliary.Add(NormalizeSeriesValue(auxiliary));

            if (logicalValues.Count == 0)
            {
                LogicalByProcessor.Clear();
                return;
            }

            while (LogicalByProcessor.Count < logicalValues.Count)
            {
                LogicalByProcessor.Add(new FixedRingSeries(_capacity));
            }

            while (LogicalByProcessor.Count > logicalValues.Count)
            {
                LogicalByProcessor.RemoveAt(LogicalByProcessor.Count - 1);
            }

            for (int index = 0; index < logicalValues.Count; index++)
            {
                LogicalByProcessor[index].Add(NormalizeSeriesValue(logicalValues[index]));
            }
        }

        private static double NormalizeSeriesValue(double value)
        {
            if (!double.IsFinite(value) || value < 0d)
            {
                return 0d;
            }

            return value;
        }
    }
}
