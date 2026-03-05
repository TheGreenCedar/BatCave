using BatCave.Controls;
using BatCave.Converters;
using BatCave.Core.Domain;
using BatCave.Styling;
using CommunityToolkit.Mvvm.Input;
using Microsoft.UI.Dispatching;
using Microsoft.UI.Xaml;
using System;
using System.Collections.Generic;
using System.Collections.ObjectModel;
using System.Diagnostics;
using System.Globalization;
using System.Linq;
using System.Threading;
using Windows.UI;

namespace BatCave.ViewModels;

public partial class MonitoringShellViewModel
{
    private const string CpuGlobalResourceId = "cpu";
    private const string MemoryGlobalResourceId = "memory";
    private const string ProcessCpuResourceId = "proc:cpu";
    private const string ProcessMemoryResourceId = "proc:memory";
    private const string ProcessDiskResourceId = "proc:disk";
    private const string ProcessNetworkResourceId = "proc:network";
    private const string ProcessOtherIoResourceId = "proc:otherio";
    private static readonly TimeSpan GlobalResourceStaleRetention = TimeSpan.FromMinutes(5);
    private static readonly FixedRingSeries EmptyTrendSeries = new(1);
    private static Color CpuStrokeColor => AppThemeTokens.ResolveColor("ChartCpuStrokeColor", Color.FromArgb(0xFF, 0x0B, 0x84, 0xD8));
    private static Color CpuFillColor => AppThemeTokens.ResolveColor("ChartCpuFillColor", Color.FromArgb(0x33, 0x0B, 0x84, 0xD8));
    private static Color CpuKernelStrokeColor => AppThemeTokens.ResolveColor("ChartCpuKernelStrokeColor", Color.FromArgb(0xFF, 0x07, 0x5C, 0x8F));
    private static Color MemoryStrokeColor => AppThemeTokens.ResolveColor("ChartMemoryStrokeColor", Color.FromArgb(0xFF, 0x25, 0x63, 0xEB));
    private static Color MemoryFillColor => AppThemeTokens.ResolveColor("ChartMemoryFillColor", Color.FromArgb(0x33, 0x25, 0x63, 0xEB));
    private static Color DiskStrokeColor => AppThemeTokens.ResolveColor("ChartIoReadStrokeColor", Color.FromArgb(0xFF, 0x6A, 0x9F, 0x2A));
    private static Color DiskFillColor => AppThemeTokens.ResolveColor("ChartIoReadFillColor", Color.FromArgb(0x33, 0x6A, 0x9F, 0x2A));
    private static Color OtherIoStrokeColor => AppThemeTokens.ResolveColor("ChartOtherIoStrokeColor", Color.FromArgb(0xFF, 0xD1, 0x34, 0x38));
    private static Color OtherIoFillColor => AppThemeTokens.ResolveColor("ChartOtherIoFillColor", Color.FromArgb(0x33, 0xD1, 0x34, 0x38));
    private static Color NetworkStrokeColor => AppThemeTokens.ResolveColor("ChartNetworkStrokeColor", Color.FromArgb(0xFF, 0xD8, 0x1B, 0x60));
    private static Color NetworkFillColor => AppThemeTokens.ResolveColor("ChartNetworkFillColor", Color.FromArgb(0x33, 0xD8, 0x1B, 0x60));
    private static Color NetworkOverlayStrokeColor => AppThemeTokens.ResolveColor("ChartNetworkOverlayStrokeColor", Color.FromArgb(0xFF, 0xA1, 0x14, 0x49));

    private readonly ObservableCollection<GlobalResourceRowViewState> _globalResourceRows = [];
    private readonly Dictionary<string, GlobalTrendHistory> _globalTrendByResourceId = new(StringComparer.OrdinalIgnoreCase);
    private readonly Dictionary<string, DateTimeOffset> _globalResourceLastSeenUtc = new(StringComparer.OrdinalIgnoreCase);
    private readonly ObservableCollection<GlobalStatItemViewState> _globalDetailStats = [];
    private readonly ObservableCollection<LogicalProcessorTrendViewState> _globalCpuLogicalProcessorRows = [];

    private SystemGlobalMetricsSample _latestGlobalMetricsSample = new();
    private GlobalResourceRowViewState? _selectedGlobalResource;
    private string _lastSelectedGlobalResourceId = CpuGlobalResourceId;
    private string _lastSelectedProcessResourceId = ProcessCpuResourceId;
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
    private double[] _globalPrimaryTrendValues = new double[60];
    private double[] _globalSecondaryTrendValues = new double[60];
    private double[] _globalAuxiliaryTrendValues = new double[60];
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
                if (!string.IsNullOrWhiteSpace(value?.ResourceId))
                {
                    if (IsGlobalPerformanceMode)
                    {
                        _lastSelectedGlobalResourceId = value.ResourceId;
                    }
                    else
                    {
                        _lastSelectedProcessResourceId = value.ResourceId;
                    }
                }

                OnPropertyChanged(nameof(IsCpuResourceSelected));
                OnPropertyChanged(nameof(GlobalCpuModeToggleVisibility));
                OnPropertyChanged(nameof(GlobalCombinedChartVisibility));
                OnPropertyChanged(nameof(GlobalCpuLogicalGridVisibility));
                OnPropertyChanged(nameof(GlobalCpuLogicalPlaceholderVisibility));
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

    public Visibility GlobalPerformanceVisibility => Visibility.Visible;

    public bool IsGlobalMetricsReady => _latestGlobalMetricsSample.IsReady;

    public Visibility GlobalPerformanceContentVisibility =>
        IsGlobalPerformanceMode
            ? (IsGlobalMetricsReady ? Visibility.Visible : Visibility.Collapsed)
            : Visibility.Visible;

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
                OnPropertyChanged(nameof(GlobalCpuLogicalPlaceholderVisibility));
            }
        }
    }

    public bool IsCpuResourceSelected => SelectedGlobalResource?.Kind == GlobalResourceKind.Cpu;

    public bool IsCpuCombinedMode => CpuGraphMode == CpuGraphMode.Combined;

    public bool IsCpuLogicalMode => CpuGraphMode == CpuGraphMode.LogicalProcessors;

    public Visibility GlobalCpuModeToggleVisibility => IsCpuResourceSelected ? Visibility.Visible : Visibility.Collapsed;

    public Visibility GlobalCombinedChartVisibility => IsCpuResourceSelected && IsCpuLogicalMode ? Visibility.Collapsed : Visibility.Visible;

    public Visibility GlobalCpuLogicalGridVisibility =>
        IsGlobalPerformanceMode && IsCpuResourceSelected && IsCpuLogicalMode ? Visibility.Visible : Visibility.Collapsed;

    public Visibility GlobalCpuLogicalPlaceholderVisibility =>
        !IsGlobalPerformanceMode && IsCpuResourceSelected && IsCpuLogicalMode ? Visibility.Visible : Visibility.Collapsed;

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
            nameof(ProcessDetailVisibility),
            nameof(GlobalCpuLogicalGridVisibility),
            nameof(GlobalCpuLogicalPlaceholderVisibility));
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
        RaiseCompactTotalsProperties();
        BuildAndAppendResourceRows(sampled);
        QueueGlobalDetailStateRefresh();
    }

    private void BuildAndAppendResourceRows(SystemGlobalMetricsSample sampled)
    {
        if (!IsGlobalPerformanceMode)
        {
            BuildAndAppendProcessResourceRows();
            return;
        }

        string? selectedResourceId = SelectedGlobalResource?.ResourceId;
        List<GlobalResourceDescriptor> descriptors = BuildGlobalResourceDescriptors(sampled);
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
            history.Append(
                descriptor.PrimaryValue,
                descriptor.SecondaryValue,
                descriptor.AuxiliaryValue,
                descriptor.LogicalValues,
                descriptor.LogicalKernelValues);

            GlobalResourceRowViewState? existing = _globalResourceRows.FirstOrDefault(
                row => string.Equals(row.ResourceId, descriptor.ResourceId, StringComparison.OrdinalIgnoreCase));
            if (existing is null)
            {
                double[] miniTrend = [];
                _ = history.Primary.CopyLatestInto(ref miniTrend, 60);
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
                    existing.MiniTrendValues,
                    descriptor.MiniScaleMode,
                    descriptor.MiniStrokeColor,
                    descriptor.MiniFillColor,
                    descriptor.MiniDomainMax);
                existing.RefreshMiniTrend(history.Primary, 60);
            }
        }

        ReconcileSelectedGlobalResource(selectedResourceId);
    }

    private void BuildAndAppendProcessResourceRows()
    {
        ProcessSample? selected = SelectedRow;
        if (selected is null)
        {
            _globalResourceRows.Clear();
            _globalCpuLogicalProcessorRows.Clear();
            return;
        }

        string? selectedResourceId = SelectedGlobalResource?.ResourceId;
        List<ProcessResourceDescriptor> descriptors = BuildProcessResourceDescriptors(selected);
        HashSet<string> currentIds = descriptors
            .Select(static descriptor => descriptor.ResourceId)
            .ToHashSet(StringComparer.OrdinalIgnoreCase);

        for (int index = _globalResourceRows.Count - 1; index >= 0; index--)
        {
            if (currentIds.Contains(_globalResourceRows[index].ResourceId))
            {
                continue;
            }

            _globalResourceRows.RemoveAt(index);
        }

        foreach (ProcessResourceDescriptor descriptor in descriptors)
        {
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
                    descriptor.MiniTrendValues,
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
                    descriptor.MiniTrendValues,
                    descriptor.MiniScaleMode,
                    descriptor.MiniStrokeColor,
                    descriptor.MiniFillColor,
                    descriptor.MiniDomainMax);
            }
        }

        ReconcileSelectedGlobalResource(selectedResourceId);
    }

    private List<ProcessResourceDescriptor> BuildProcessResourceDescriptors(ProcessSample selected)
    {
        MetricHistoryBuffer history = ResolveProcessHistory(selected);
        ulong diskRate = SaturatingAddRates(selected.IoReadBps, selected.IoWriteBps);

        return
        [
            new ProcessResourceDescriptor(
                ResourceId: ProcessCpuResourceId,
                Kind: GlobalResourceKind.Cpu,
                Title: "CPU",
                Subtitle: selected.Name,
                ValueText: $"{selected.CpuPct:F1}%",
                MiniTrendValues: SnapshotSeries(history.Cpu, 60),
                MiniScaleMode: MetricTrendScaleMode.CpuPercent,
                MiniStrokeColor: CpuStrokeColor,
                MiniFillColor: CpuFillColor,
                MiniDomainMax: double.NaN),
            new ProcessResourceDescriptor(
                ResourceId: ProcessMemoryResourceId,
                Kind: GlobalResourceKind.Memory,
                Title: "Memory",
                Subtitle: selected.Name,
                ValueText: ValueFormat.FormatBytes(selected.RssBytes),
                MiniTrendValues: SnapshotSeries(history.Memory, 60),
                MiniScaleMode: MetricTrendScaleMode.MemoryBytes,
                MiniStrokeColor: MemoryStrokeColor,
                MiniFillColor: MemoryFillColor,
                MiniDomainMax: double.NaN),
            new ProcessResourceDescriptor(
                ResourceId: ProcessDiskResourceId,
                Kind: GlobalResourceKind.Disk,
                Title: "Disk",
                Subtitle: "Read + Write",
                ValueText: ValueFormat.FormatRate(diskRate),
                MiniTrendValues: SnapshotCombinedSeries(history.IoRead, history.IoWrite, 60),
                MiniScaleMode: MetricTrendScaleMode.IoRate,
                MiniStrokeColor: DiskStrokeColor,
                MiniFillColor: DiskFillColor,
                MiniDomainMax: double.NaN),
            new ProcessResourceDescriptor(
                ResourceId: ProcessNetworkResourceId,
                Kind: GlobalResourceKind.Network,
                Title: "Network",
                Subtitle: "Derived from Other I/O",
                ValueText: ValueFormat.FormatBitsRateFromBytes(selected.OtherIoBps),
                MiniTrendValues: SnapshotSeries(history.OtherIo, 60, static value => value * 8d),
                MiniScaleMode: MetricTrendScaleMode.BitsRate,
                MiniStrokeColor: NetworkStrokeColor,
                MiniFillColor: NetworkFillColor,
                MiniDomainMax: double.NaN),
            new ProcessResourceDescriptor(
                ResourceId: ProcessOtherIoResourceId,
                Kind: GlobalResourceKind.OtherIo,
                Title: "Other I/O",
                Subtitle: selected.Name,
                ValueText: ValueFormat.FormatRate(selected.OtherIoBps),
                MiniTrendValues: SnapshotSeries(history.OtherIo, 60),
                MiniScaleMode: MetricTrendScaleMode.IoRate,
                MiniStrokeColor: OtherIoStrokeColor,
                MiniFillColor: OtherIoFillColor,
                MiniDomainMax: double.NaN),
        ];
    }

    private MetricHistoryBuffer ResolveProcessHistory(ProcessSample selected)
    {
        if (_metricHistory.TryGetValue(selected.Identity(), out MetricHistoryBuffer? history))
        {
            return history;
        }

        MetricHistoryBuffer fallback = new(HistoryLimit);
        fallback.Append(selected);
        return fallback;
    }

    private static double[] SnapshotSeries(IReadOnlyList<double> source, int visiblePointCount, Func<double, double>? map = null)
    {
        int count = Math.Max(1, visiblePointCount);
        double[] result = new double[count];
        int take = Math.Min(source.Count, count);
        int sourceStart = source.Count - take;
        int destinationStart = count - take;
        for (int index = 0; index < take; index++)
        {
            double value = source[sourceStart + index];
            result[destinationStart + index] = map is null ? value : map(value);
        }

        return result;
    }

    private static double[] SnapshotCombinedSeries(IReadOnlyList<double> left, IReadOnlyList<double> right, int visiblePointCount)
    {
        int count = Math.Max(1, visiblePointCount);
        double[] result = new double[count];
        for (int outputIndex = 0; outputIndex < count; outputIndex++)
        {
            int leftIndex = left.Count - count + outputIndex;
            int rightIndex = right.Count - count + outputIndex;
            double leftValue = leftIndex >= 0 && leftIndex < left.Count ? left[leftIndex] : 0d;
            double rightValue = rightIndex >= 0 && rightIndex < right.Count ? right[rightIndex] : 0d;
            result[outputIndex] = leftValue + rightValue;
        }

        return result;
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
            bool expectedProcessPrefix = !IsGlobalPerformanceMode;
            bool hasProcessPrefix = preferredResourceId.StartsWith("proc:", StringComparison.OrdinalIgnoreCase);
            if (expectedProcessPrefix != hasProcessPrefix)
            {
                preferredResourceId = null;
            }
        }

        string? effectivePreferredResourceId = !string.IsNullOrWhiteSpace(preferredResourceId)
            ? preferredResourceId
            : (IsGlobalPerformanceMode ? _lastSelectedGlobalResourceId : _lastSelectedProcessResourceId);

        if (!string.IsNullOrWhiteSpace(effectivePreferredResourceId))
        {
            GlobalResourceRowViewState? preserved = _globalResourceRows.FirstOrDefault(
                row => string.Equals(row.ResourceId, effectivePreferredResourceId, StringComparison.OrdinalIgnoreCase));
            if (preserved is not null)
            {
                if (!ReferenceEquals(SelectedGlobalResource, preserved))
                {
                    SelectedGlobalResource = preserved;
                }

                return;
            }
        }

        string fallbackResourceId = IsGlobalPerformanceMode ? CpuGlobalResourceId : ProcessCpuResourceId;
        GlobalResourceRowViewState? fallback = _globalResourceRows.FirstOrDefault(row => row.ResourceId == fallbackResourceId)
            ?? _globalResourceRows.FirstOrDefault();
        if (!ReferenceEquals(SelectedGlobalResource, fallback))
        {
            SelectedGlobalResource = fallback;
        }
    }

    private List<GlobalResourceDescriptor> BuildGlobalResourceDescriptors(SystemGlobalMetricsSample sampled)
    {
        List<GlobalResourceDescriptor> descriptors = new(capacity: 2 + sampled.DiskSnapshots.Count + sampled.NetworkSnapshots.Count);
        descriptors.Add(BuildCpuDescriptor(sampled));
        descriptors.Add(BuildMemoryDescriptor(sampled));

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
                LogicalKernelValues: [],
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
                LogicalKernelValues: [],
                MiniScaleMode: MetricTrendScaleMode.BitsRate,
                MiniStrokeColor: NetworkStrokeColor,
                MiniFillColor: NetworkFillColor,
                MiniDomainMax: double.NaN));
        }

        return descriptors;
    }

    private static GlobalResourceDescriptor BuildCpuDescriptor(SystemGlobalMetricsSample sampled)
    {
        double cpuPct = sampled.CpuPct ?? 0d;
        string cpuSpeed = ValueFormat.FormatFrequencyGHz(sampled.CpuSnapshot?.SpeedMHz);
        return new GlobalResourceDescriptor(
            ResourceId: CpuGlobalResourceId,
            Kind: GlobalResourceKind.Cpu,
            Title: "CPU",
            Subtitle: cpuSpeed == "n/a" ? $"{cpuPct:F0}%" : $"{cpuPct:F0}% {cpuSpeed}",
            ValueText: string.Empty,
            PrimaryValue: cpuPct,
            SecondaryValue: sampled.CpuSnapshot?.KernelPct ?? 0d,
            AuxiliaryValue: 0d,
            LogicalValues: sampled.CpuSnapshot?.LogicalProcessorUtilizationPct ?? [],
            LogicalKernelValues: sampled.CpuSnapshot?.LogicalProcessorKernelPct ?? [],
            MiniScaleMode: MetricTrendScaleMode.CpuPercent,
            MiniStrokeColor: CpuStrokeColor,
            MiniFillColor: CpuFillColor,
            MiniDomainMax: double.NaN);
    }

    private static GlobalResourceDescriptor BuildMemoryDescriptor(SystemGlobalMetricsSample sampled)
    {
        ulong memoryUsedBytes = sampled.MemorySnapshot?.UsedBytes ?? sampled.MemoryUsedBytes ?? 0UL;
        ulong memoryTotalBytes = sampled.MemorySnapshot?.TotalBytes ?? 0UL;
        string memoryValueText = memoryTotalBytes > 0
            ? $"{ValueFormat.FormatBytes(memoryUsedBytes)} / {ValueFormat.FormatBytes(memoryTotalBytes)} ({(memoryUsedBytes * 100d / memoryTotalBytes):F0}%)"
            : ValueFormat.FormatBytes(memoryUsedBytes);
        return new GlobalResourceDescriptor(
            ResourceId: MemoryGlobalResourceId,
            Kind: GlobalResourceKind.Memory,
            Title: "Memory",
            Subtitle: memoryValueText,
            ValueText: string.Empty,
            PrimaryValue: memoryUsedBytes,
            SecondaryValue: 0d,
            AuxiliaryValue: 0d,
            LogicalValues: [],
            LogicalKernelValues: [],
            MiniScaleMode: MetricTrendScaleMode.MemoryBytes,
            MiniStrokeColor: MemoryStrokeColor,
            MiniFillColor: MemoryFillColor,
            MiniDomainMax: memoryTotalBytes > 0 ? memoryTotalBytes : double.NaN);
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

            if (IsGlobalPerformanceMode)
            {
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
            else
            {
                switch (selected.Kind)
                {
                    case GlobalResourceKind.Memory:
                        ApplyProcessMemoryDetailState();
                        break;
                    case GlobalResourceKind.Disk:
                        ApplyProcessDiskDetailState();
                        break;
                    case GlobalResourceKind.Network:
                        ApplyProcessNetworkDetailState();
                        break;
                    case GlobalResourceKind.OtherIo:
                        ApplyProcessOtherIoDetailState();
                        break;
                    default:
                        ApplyProcessCpuDetailState();
                        break;
                }
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
        ApplyGlobalTrendValues(history.Primary, ref _globalPrimaryTrendValues, nameof(GlobalPrimaryTrendValues));
        ApplyGlobalTrendValues(history.Secondary, ref _globalSecondaryTrendValues, nameof(GlobalSecondaryTrendValues));
        ClearGlobalTrendValues(ref _globalAuxiliaryTrendValues, nameof(GlobalAuxiliaryTrendValues));

        UpdateCpuLogicalRows(history.LogicalByProcessor, history.LogicalKernelByProcessor, MetricTrendWindowSeconds);
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
        ApplyGlobalTrendValues(history.Primary, ref _globalPrimaryTrendValues, nameof(GlobalPrimaryTrendValues));
        ClearGlobalTrendValues(ref _globalSecondaryTrendValues, nameof(GlobalSecondaryTrendValues));
        ClearGlobalTrendValues(ref _globalAuxiliaryTrendValues, nameof(GlobalAuxiliaryTrendValues));

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

        ApplyGlobalTrendValues(history.Primary, ref _globalPrimaryTrendValues, nameof(GlobalPrimaryTrendValues), sanitize: true);
        ApplyGlobalTrendValues(history.Auxiliary, ref _globalAuxiliaryTrendValues, nameof(GlobalAuxiliaryTrendValues), sanitize: true);
        bool canShowAuxiliaryChart = _globalAuxiliaryTrendValues.Length > 0;
        GlobalShowAuxiliaryChart = canShowAuxiliaryChart;
        GlobalPrimaryStrokeColor = DiskStrokeColor;
        GlobalPrimaryFillColor = DiskFillColor;
        GlobalSecondaryStrokeColor = DiskStrokeColor;
        GlobalAuxiliaryStrokeColor = DiskStrokeColor;
        GlobalAuxiliaryFillColor = DiskFillColor;
        GlobalPrimaryDomainMax = double.NaN;
        GlobalAuxiliaryDomainMax = double.NaN;
        ClearGlobalTrendValues(ref _globalSecondaryTrendValues, nameof(GlobalSecondaryTrendValues));
        if (!canShowAuxiliaryChart)
        {
            ClearGlobalTrendValues(ref _globalAuxiliaryTrendValues, nameof(GlobalAuxiliaryTrendValues));
        }

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
        ApplyGlobalTrendValues(history.Primary, ref _globalPrimaryTrendValues, nameof(GlobalPrimaryTrendValues));
        ApplyGlobalTrendValues(history.Secondary, ref _globalSecondaryTrendValues, nameof(GlobalSecondaryTrendValues));
        ClearGlobalTrendValues(ref _globalAuxiliaryTrendValues, nameof(GlobalAuxiliaryTrendValues));

        PopulateNetworkStats(network);
    }

    private bool TryResolveSelectedProcessForDetail(out ProcessSample selected, out MetricHistoryBuffer history)
    {
        if (SelectedRow is not ProcessSample row)
        {
            selected = _globalSummaryRow;
            history = new MetricHistoryBuffer(HistoryLimit);
            return false;
        }

        selected = row;
        history = ResolveProcessHistory(row);
        return true;
    }

    private void ApplyProcessCpuDetailState()
    {
        if (!TryResolveSelectedProcessForDetail(out ProcessSample selected, out MetricHistoryBuffer history))
        {
            ApplyDetailFallbackState(SelectedGlobalResource);
            return;
        }

        GlobalDetailTitle = "CPU";
        GlobalDetailSubtitle = $"{selected.Name} ({selected.Pid})";
        GlobalDetailCurrentValue = $"{selected.CpuPct:F1}%";

        GlobalPrimaryScaleMode = MetricTrendScaleMode.CpuPercent;
        GlobalAuxiliaryScaleMode = MetricTrendScaleMode.IoRate;
        GlobalPrimaryChartTitle = "Utilization";
        GlobalAuxiliaryChartTitle = "Transfer rate";
        GlobalShowSecondaryOverlay = false;
        GlobalShowAuxiliaryChart = false;
        GlobalPrimaryStrokeColor = CpuStrokeColor;
        GlobalPrimaryFillColor = CpuFillColor;
        GlobalSecondaryStrokeColor = CpuKernelStrokeColor;
        GlobalAuxiliaryStrokeColor = DiskStrokeColor;
        GlobalAuxiliaryFillColor = DiskFillColor;
        GlobalPrimaryDomainMax = double.NaN;
        GlobalAuxiliaryDomainMax = double.NaN;

        ApplyGlobalTrendValuesFromSource(history.Cpu, ref _globalPrimaryTrendValues, nameof(GlobalPrimaryTrendValues));
        ClearGlobalTrendValues(ref _globalSecondaryTrendValues, nameof(GlobalSecondaryTrendValues));
        ClearGlobalTrendValues(ref _globalAuxiliaryTrendValues, nameof(GlobalAuxiliaryTrendValues));
        _globalCpuLogicalProcessorRows.Clear();
        PopulateProcessStats(selected);
    }

    private void ApplyProcessMemoryDetailState()
    {
        if (!TryResolveSelectedProcessForDetail(out ProcessSample selected, out MetricHistoryBuffer history))
        {
            ApplyDetailFallbackState(SelectedGlobalResource);
            return;
        }

        GlobalDetailTitle = "Memory";
        GlobalDetailSubtitle = $"{selected.Name} ({selected.Pid})";
        GlobalDetailCurrentValue = ValueFormat.FormatBytes(selected.RssBytes);

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
        GlobalPrimaryDomainMax = double.NaN;
        GlobalAuxiliaryDomainMax = double.NaN;

        ApplyGlobalTrendValuesFromSource(history.Memory, ref _globalPrimaryTrendValues, nameof(GlobalPrimaryTrendValues));
        ClearGlobalTrendValues(ref _globalSecondaryTrendValues, nameof(GlobalSecondaryTrendValues));
        ClearGlobalTrendValues(ref _globalAuxiliaryTrendValues, nameof(GlobalAuxiliaryTrendValues));
        _globalCpuLogicalProcessorRows.Clear();
        PopulateProcessStats(selected);
    }

    private void ApplyProcessDiskDetailState()
    {
        if (!TryResolveSelectedProcessForDetail(out ProcessSample selected, out MetricHistoryBuffer history))
        {
            ApplyDetailFallbackState(SelectedGlobalResource);
            return;
        }

        GlobalDetailTitle = "Disk";
        GlobalDetailSubtitle = $"{selected.Name} ({selected.Pid})";
        GlobalDetailCurrentValue = ValueFormat.FormatRate(SaturatingAddRates(selected.IoReadBps, selected.IoWriteBps));

        GlobalPrimaryScaleMode = MetricTrendScaleMode.IoRate;
        GlobalAuxiliaryScaleMode = MetricTrendScaleMode.IoRate;
        GlobalPrimaryChartTitle = "Read + write throughput";
        GlobalAuxiliaryChartTitle = "Transfer rate";
        GlobalShowSecondaryOverlay = false;
        GlobalShowAuxiliaryChart = false;
        GlobalPrimaryStrokeColor = DiskStrokeColor;
        GlobalPrimaryFillColor = DiskFillColor;
        GlobalSecondaryStrokeColor = DiskStrokeColor;
        GlobalAuxiliaryStrokeColor = DiskStrokeColor;
        GlobalAuxiliaryFillColor = DiskFillColor;
        GlobalPrimaryDomainMax = double.NaN;
        GlobalAuxiliaryDomainMax = double.NaN;

        ApplyGlobalCombinedTrendValues(history.IoRead, history.IoWrite, ref _globalPrimaryTrendValues, nameof(GlobalPrimaryTrendValues));
        ClearGlobalTrendValues(ref _globalSecondaryTrendValues, nameof(GlobalSecondaryTrendValues));
        ClearGlobalTrendValues(ref _globalAuxiliaryTrendValues, nameof(GlobalAuxiliaryTrendValues));
        _globalCpuLogicalProcessorRows.Clear();
        PopulateProcessStats(selected);
    }

    private void ApplyProcessNetworkDetailState()
    {
        if (!TryResolveSelectedProcessForDetail(out ProcessSample selected, out MetricHistoryBuffer history))
        {
            ApplyDetailFallbackState(SelectedGlobalResource);
            return;
        }

        GlobalDetailTitle = "Network";
        GlobalDetailSubtitle = $"{selected.Name} ({selected.Pid})";
        GlobalDetailCurrentValue = ValueFormat.FormatBitsRateFromBytes(selected.OtherIoBps);

        GlobalPrimaryScaleMode = MetricTrendScaleMode.BitsRate;
        GlobalAuxiliaryScaleMode = MetricTrendScaleMode.IoRate;
        GlobalPrimaryChartTitle = "Derived throughput";
        GlobalAuxiliaryChartTitle = "Transfer rate";
        GlobalShowSecondaryOverlay = false;
        GlobalShowAuxiliaryChart = false;
        GlobalPrimaryStrokeColor = NetworkStrokeColor;
        GlobalPrimaryFillColor = NetworkFillColor;
        GlobalSecondaryStrokeColor = NetworkOverlayStrokeColor;
        GlobalAuxiliaryStrokeColor = DiskStrokeColor;
        GlobalAuxiliaryFillColor = DiskFillColor;
        GlobalPrimaryDomainMax = double.NaN;
        GlobalAuxiliaryDomainMax = double.NaN;

        ApplyGlobalTrendValuesFromSource(history.OtherIo, ref _globalPrimaryTrendValues, nameof(GlobalPrimaryTrendValues), static value => value * 8d);
        ClearGlobalTrendValues(ref _globalSecondaryTrendValues, nameof(GlobalSecondaryTrendValues));
        ClearGlobalTrendValues(ref _globalAuxiliaryTrendValues, nameof(GlobalAuxiliaryTrendValues));
        _globalCpuLogicalProcessorRows.Clear();
        PopulateProcessStats(selected);
    }

    private void ApplyProcessOtherIoDetailState()
    {
        if (!TryResolveSelectedProcessForDetail(out ProcessSample selected, out MetricHistoryBuffer history))
        {
            ApplyDetailFallbackState(SelectedGlobalResource);
            return;
        }

        GlobalDetailTitle = "Other I/O";
        GlobalDetailSubtitle = $"{selected.Name} ({selected.Pid})";
        GlobalDetailCurrentValue = ValueFormat.FormatRate(selected.OtherIoBps);

        GlobalPrimaryScaleMode = MetricTrendScaleMode.IoRate;
        GlobalAuxiliaryScaleMode = MetricTrendScaleMode.IoRate;
        GlobalPrimaryChartTitle = "Transfer rate";
        GlobalAuxiliaryChartTitle = "Transfer rate";
        GlobalShowSecondaryOverlay = false;
        GlobalShowAuxiliaryChart = false;
        GlobalPrimaryStrokeColor = OtherIoStrokeColor;
        GlobalPrimaryFillColor = OtherIoFillColor;
        GlobalSecondaryStrokeColor = OtherIoStrokeColor;
        GlobalAuxiliaryStrokeColor = DiskStrokeColor;
        GlobalAuxiliaryFillColor = DiskFillColor;
        GlobalPrimaryDomainMax = double.NaN;
        GlobalAuxiliaryDomainMax = double.NaN;

        ApplyGlobalTrendValuesFromSource(history.OtherIo, ref _globalPrimaryTrendValues, nameof(GlobalPrimaryTrendValues));
        ClearGlobalTrendValues(ref _globalSecondaryTrendValues, nameof(GlobalSecondaryTrendValues));
        ClearGlobalTrendValues(ref _globalAuxiliaryTrendValues, nameof(GlobalAuxiliaryTrendValues));
        _globalCpuLogicalProcessorRows.Clear();
        PopulateProcessStats(selected);
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
        ClearGlobalTrendValues(ref _globalPrimaryTrendValues, nameof(GlobalPrimaryTrendValues));
        ClearGlobalTrendValues(ref _globalSecondaryTrendValues, nameof(GlobalSecondaryTrendValues));
        ClearGlobalTrendValues(ref _globalAuxiliaryTrendValues, nameof(GlobalAuxiliaryTrendValues));
        SetGlobalStats(new[] { ("Status", "n/a") });
    }

    private void UpdateCpuLogicalRows(
        IReadOnlyList<FixedRingSeries> logicalSeries,
        IReadOnlyList<FixedRingSeries> logicalKernelSeries,
        int visiblePointCount)
    {
        int rowCount = Math.Max(logicalSeries.Count, logicalKernelSeries.Count);
        if (rowCount == 0)
        {
            _globalCpuLogicalProcessorRows.Clear();
            return;
        }

        while (_globalCpuLogicalProcessorRows.Count < rowCount)
        {
            int index = _globalCpuLogicalProcessorRows.Count;
            _globalCpuLogicalProcessorRows.Add(new LogicalProcessorTrendViewState($"CPU {index}", [], []));
        }

        while (_globalCpuLogicalProcessorRows.Count > rowCount)
        {
            _globalCpuLogicalProcessorRows.RemoveAt(_globalCpuLogicalProcessorRows.Count - 1);
        }

        for (int index = 0; index < rowCount; index++)
        {
            FixedRingSeries logical = index < logicalSeries.Count ? logicalSeries[index] : EmptyTrendSeries;
            FixedRingSeries kernel = index < logicalKernelSeries.Count ? logicalKernelSeries[index] : EmptyTrendSeries;
            _globalCpuLogicalProcessorRows[index].UpdateValues(logical, kernel, visiblePointCount);
        }
    }

    private void ApplyGlobalTrendValues(
        FixedRingSeries source,
        ref double[] target,
        string propertyName,
        bool sanitize = false)
    {
        bool changed = source.CopyLatestInto(ref target, MetricTrendWindowSeconds);
        if (sanitize)
        {
            for (int index = 0; index < target.Length; index++)
            {
                double value = target[index];
                double sanitized = double.IsFinite(value) && value >= 0d ? value : 0d;
                if (sanitized == value)
                {
                    continue;
                }

                target[index] = sanitized;
                changed = true;
            }
        }

        if (changed)
        {
            OnPropertyChanged(propertyName);
        }
    }

    private void ApplyGlobalTrendValuesFromSource(
        IReadOnlyList<double> source,
        ref double[] target,
        string propertyName,
        Func<double, double>? map = null)
    {
        int visiblePointCount = Math.Max(1, MetricTrendWindowSeconds);
        bool changed = target.Length != visiblePointCount;
        if (changed)
        {
            target = new double[visiblePointCount];
        }

        int take = Math.Min(source.Count, visiblePointCount);
        int sourceStart = source.Count - take;
        int destinationStart = visiblePointCount - take;

        for (int index = 0; index < destinationStart; index++)
        {
            if (target[index] == 0d)
            {
                continue;
            }

            target[index] = 0d;
            changed = true;
        }

        for (int index = 0; index < take; index++)
        {
            double next = source[sourceStart + index];
            next = map is null ? next : map(next);
            int destinationIndex = destinationStart + index;
            if (target[destinationIndex] == next)
            {
                continue;
            }

            target[destinationIndex] = next;
            changed = true;
        }

        if (changed)
        {
            OnPropertyChanged(propertyName);
        }
    }

    private void ApplyGlobalCombinedTrendValues(
        IReadOnlyList<double> left,
        IReadOnlyList<double> right,
        ref double[] target,
        string propertyName)
    {
        int visiblePointCount = Math.Max(1, MetricTrendWindowSeconds);
        bool changed = target.Length != visiblePointCount;
        if (changed)
        {
            target = new double[visiblePointCount];
        }

        for (int index = 0; index < visiblePointCount; index++)
        {
            int leftIndex = left.Count - visiblePointCount + index;
            int rightIndex = right.Count - visiblePointCount + index;
            double leftValue = leftIndex >= 0 && leftIndex < left.Count ? left[leftIndex] : 0d;
            double rightValue = rightIndex >= 0 && rightIndex < right.Count ? right[rightIndex] : 0d;
            double next = leftValue + rightValue;
            if (target[index] == next)
            {
                continue;
            }

            target[index] = next;
            changed = true;
        }

        if (changed)
        {
            OnPropertyChanged(propertyName);
        }
    }

    private void ClearGlobalTrendValues(ref double[] target, string propertyName)
    {
        if (target.Length == 0)
        {
            return;
        }

        target = [];
        OnPropertyChanged(propertyName);
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

    private void PopulateProcessStats(ProcessSample selected)
    {
        SetGlobalStats(new (string, string)[]
        {
            ("Process", selected.Name),
            ("PID", selected.Pid.ToString(CultureInfo.InvariantCulture)),
            ("CPU", $"{selected.CpuPct:F2}%"),
            ("Memory", ValueFormat.FormatBytes(selected.RssBytes)),
            ("Disk read", ValueFormat.FormatRate(selected.IoReadBps)),
            ("Disk write", ValueFormat.FormatRate(selected.IoWriteBps)),
            ("Disk total", ValueFormat.FormatRate(SaturatingAddRates(selected.IoReadBps, selected.IoWriteBps))),
            ("Network", ValueFormat.FormatBitsRateFromBytes(selected.OtherIoBps)),
            ("Other I/O", ValueFormat.FormatRate(selected.OtherIoBps)),
            ("Metadata", MetadataStatus),
            ("Parent PID", MetadataParentPid),
            ("Executable path", MetadataExecutablePath),
            ("Command line", MetadataCommandLine),
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
        IReadOnlyList<double> LogicalKernelValues,
        MetricTrendScaleMode MiniScaleMode,
        Color MiniStrokeColor,
        Color MiniFillColor,
        double MiniDomainMax);

    private readonly record struct ProcessResourceDescriptor(
        string ResourceId,
        GlobalResourceKind Kind,
        string Title,
        string Subtitle,
        string ValueText,
        double[] MiniTrendValues,
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

    private static ulong SaturatingAddRates(ulong left, ulong right)
    {
        ulong result = left + right;
        return result < left ? ulong.MaxValue : result;
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

        public List<FixedRingSeries> LogicalKernelByProcessor { get; } = [];

        public void Append(
            double primary,
            double secondary,
            double auxiliary,
            IReadOnlyList<double> logicalValues,
            IReadOnlyList<double> logicalKernelValues)
        {
            Primary.Add(NormalizeSeriesValue(primary));
            Secondary.Add(NormalizeSeriesValue(secondary));
            Auxiliary.Add(NormalizeSeriesValue(auxiliary));

            int logicalCount = Math.Max(logicalValues.Count, logicalKernelValues.Count);
            if (logicalCount == 0)
            {
                LogicalByProcessor.Clear();
                LogicalKernelByProcessor.Clear();
                return;
            }

            while (LogicalByProcessor.Count < logicalCount)
            {
                LogicalByProcessor.Add(new FixedRingSeries(_capacity));
            }

            while (LogicalByProcessor.Count > logicalCount)
            {
                LogicalByProcessor.RemoveAt(LogicalByProcessor.Count - 1);
            }

            while (LogicalKernelByProcessor.Count < logicalCount)
            {
                LogicalKernelByProcessor.Add(new FixedRingSeries(_capacity));
            }

            while (LogicalKernelByProcessor.Count > logicalCount)
            {
                LogicalKernelByProcessor.RemoveAt(LogicalKernelByProcessor.Count - 1);
            }

            for (int index = 0; index < logicalCount; index++)
            {
                double logicalValue = index < logicalValues.Count ? logicalValues[index] : 0d;
                double logicalKernelValue = index < logicalKernelValues.Count ? logicalKernelValues[index] : 0d;
                LogicalByProcessor[index].Add(NormalizeSeriesValue(logicalValue));
                LogicalKernelByProcessor[index].Add(NormalizeSeriesValue(logicalKernelValue));
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
