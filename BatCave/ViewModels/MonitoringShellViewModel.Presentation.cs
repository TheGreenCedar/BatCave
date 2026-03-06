using CommunityToolkit.Mvvm.Input;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Media;
using System;
using System.Collections.Generic;
using System.Linq;

namespace BatCave.ViewModels;

public enum InspectorSection
{
    Summary,
    Performance,
    Details,
}

public enum InspectorLayoutMode
{
    SystemOverview,
    ProcessInspector,
}

public enum RuntimeStatusTone
{
    Info,
    Success,
    Warning,
    Error,
}

public partial class MonitoringShellViewModel
{
    private InspectorSection _inspectorSection = InspectorSection.Summary;
    private RuntimeStatusTone _runtimeStatusTone = RuntimeStatusTone.Info;
    private string _runtimeStatusTag = "INFO";
    private string _runtimeStatusTitle = "Runtime Healthy";
    private string _runtimeStatusSummary = "Live monitor shell ready.";
    private bool _isRuntimeStatusVisible = true;

    public InspectorSection InspectorSection
    {
        get => _inspectorSection;
        private set
        {
            if (SetProperty(ref _inspectorSection, value))
            {
                RaiseInspectorSectionProperties();
            }
        }
    }

    public bool IsSummarySectionSelected => InspectorSection == InspectorSection.Summary;

    public bool IsPerformanceSectionSelected => InspectorSection == InspectorSection.Performance;

    public bool IsDetailsSectionSelected => InspectorSection == InspectorSection.Details;

    public Visibility SummarySectionVisibility => IsSummarySectionSelected ? Visibility.Visible : Visibility.Collapsed;

    public Visibility PerformanceSectionVisibility => IsPerformanceSectionSelected ? Visibility.Visible : Visibility.Collapsed;

    public Visibility DetailsSectionVisibility => IsDetailsSectionSelected ? Visibility.Visible : Visibility.Collapsed;

    public Visibility SystemSummarySectionVisibility =>
        IsSummarySectionSelected && IsSystemOverview ? Visibility.Visible : Visibility.Collapsed;

    public Visibility ProcessSummarySectionVisibility =>
        IsSummarySectionSelected && IsProcessInspector ? Visibility.Visible : Visibility.Collapsed;

    public InspectorLayoutMode InspectorLayoutMode => HasSelection ? InspectorLayoutMode.ProcessInspector : InspectorLayoutMode.SystemOverview;

    public bool IsSystemOverview => !HasSelection;

    public bool IsProcessInspector => HasSelection;

    public string InspectorContextTitle => HasSelection ? DetailTitle : "System Overview";

    public string InspectorContextSubtitle =>
        HasSelection
            ? "Focused process telemetry with a larger active-metric view."
            : "Live system telemetry, runtime health, and resource detail.";

    public string InspectorOverviewEyebrow => HasSelection ? "PROCESS VIEW" : "SYSTEM VIEW";

    public string InspectorOverviewSummary =>
        HasSelection
            ? "The active chart, summary, and metadata stay aligned to the selected process and metric."
            : "The active chart, summary, and metadata stay aligned to the selected system resource.";

    public Visibility InspectorOverviewSummaryVisibility => IsSystemOverview ? Visibility.Visible : Visibility.Collapsed;

    public Visibility SystemSummaryLayoutVisibility => IsSystemOverview ? Visibility.Visible : Visibility.Collapsed;

    public Visibility ProcessSummaryLayoutVisibility => IsProcessInspector ? Visibility.Visible : Visibility.Collapsed;

    public RuntimeStatusTone RuntimeStatusTone
    {
        get => _runtimeStatusTone;
        private set
        {
            if (SetProperty(ref _runtimeStatusTone, value))
            {
                RaiseProperties(
                    nameof(RuntimeStatusInfoVisibility),
                    nameof(RuntimeStatusSuccessVisibility),
                    nameof(RuntimeStatusWarningVisibility),
                    nameof(RuntimeStatusErrorVisibility),
                    nameof(RuntimeStatusSurfaceBrush),
                    nameof(RuntimeStatusAccentBrush),
                    nameof(RuntimeStatusTagBackgroundBrush),
                    nameof(RuntimeStatusTagForegroundBrush),
                    nameof(RuntimeStatusTitleBrush),
                    nameof(RuntimeStatusSummaryBrush));
            }
        }
    }

    public Visibility RuntimeStatusInfoVisibility => RuntimeStatusTone == RuntimeStatusTone.Info ? Visibility.Visible : Visibility.Collapsed;

    public Visibility RuntimeStatusSuccessVisibility => RuntimeStatusTone == RuntimeStatusTone.Success ? Visibility.Visible : Visibility.Collapsed;

    public Visibility RuntimeStatusWarningVisibility => RuntimeStatusTone == RuntimeStatusTone.Warning ? Visibility.Visible : Visibility.Collapsed;

    public Visibility RuntimeStatusErrorVisibility => RuntimeStatusTone == RuntimeStatusTone.Error ? Visibility.Visible : Visibility.Collapsed;

    public string RuntimeStatusTag
    {
        get => _runtimeStatusTag;
        private set => SetProperty(ref _runtimeStatusTag, value);
    }

    public string RuntimeStatusTitle
    {
        get => _runtimeStatusTitle;
        private set => SetProperty(ref _runtimeStatusTitle, value);
    }

    public string RuntimeStatusSummary
    {
        get => _runtimeStatusSummary;
        private set => SetProperty(ref _runtimeStatusSummary, value);
    }

    public bool IsRuntimeStatusVisible
    {
        get => _isRuntimeStatusVisible;
        private set
        {
            if (SetProperty(ref _isRuntimeStatusVisible, value))
            {
                OnPropertyChanged(nameof(RuntimeStatusVisibility));
            }
        }
    }

    public Visibility RuntimeStatusVisibility => IsRuntimeStatusVisible ? Visibility.Visible : Visibility.Collapsed;

    public IReadOnlyList<GlobalStatItemViewState> SummaryStatCards =>
        _globalDetailStats.Take(Math.Min(HasSelection ? 4 : 6, _globalDetailStats.Count)).ToArray();

    public double InspectorChartMaxWidth => HasSelection ? 960 : 680;

    public double SummaryStatCardWidth => HasSelection ? 232 : 208;

    public string DetailsPaneTitle =>
        HasSelection
            ? "Process Metadata"
            : "Runtime Diagnostics";

    public string DetailsPanePrimaryText =>
        HasSelection
            ? MetadataExecutablePath
            : RuntimeHealthStatus;

    public string DetailsPaneSecondaryText =>
        HasSelection
            ? MetadataCommandLine
            : InteractionTimingProbe;

    public Brush RuntimeStatusSurfaceBrush => ResolveBrush(RuntimeStatusTone switch
    {
        RuntimeStatusTone.Success => "BatCaveBannerSuccessSurfaceBrush",
        RuntimeStatusTone.Warning => "BatCaveBannerWarningSurfaceBrush",
        RuntimeStatusTone.Error => "BatCaveBannerErrorSurfaceBrush",
        _ => "BatCaveBannerInfoSurfaceBrush",
    });

    public Brush RuntimeStatusAccentBrush => ResolveBrush(RuntimeStatusTone switch
    {
        RuntimeStatusTone.Success => "BatCaveSuccessBrush",
        RuntimeStatusTone.Warning => "BatCaveWarningBrush",
        RuntimeStatusTone.Error => "BatCaveDangerBrush",
        _ => "BatCavePrimaryBrush",
    });

    public Brush RuntimeStatusTagBackgroundBrush => RuntimeStatusAccentBrush;

    public Brush RuntimeStatusTagForegroundBrush => ResolveBrush("BatCaveOnPrimaryBrush");

    public Brush RuntimeStatusTitleBrush => ResolveBrush(RuntimeStatusTone switch
    {
        RuntimeStatusTone.Success => "BatCaveSuccessBrush",
        RuntimeStatusTone.Warning => "BatCaveWarningBrush",
        RuntimeStatusTone.Error => "BatCaveDangerBrush",
        _ => "BatCaveTextPrimaryBrush",
    });

    public Brush RuntimeStatusSummaryBrush => ResolveBrush(RuntimeStatusTone switch
    {
        RuntimeStatusTone.Warning or RuntimeStatusTone.Error => "BatCaveTextPrimaryBrush",
        _ => "BatCaveTextSecondaryBrush",
    });

    public Brush SummaryTabBackgroundBrush => GetSegmentBackground(IsSummarySectionSelected);

    public Brush SummaryTabForegroundBrush => GetSegmentForeground(IsSummarySectionSelected);

    public Brush PerformanceTabBackgroundBrush => GetSegmentBackground(IsPerformanceSectionSelected);

    public Brush PerformanceTabForegroundBrush => GetSegmentForeground(IsPerformanceSectionSelected);

    public Brush DetailsTabBackgroundBrush => GetSegmentBackground(IsDetailsSectionSelected);

    public Brush DetailsTabForegroundBrush => GetSegmentForeground(IsDetailsSectionSelected);

    public Brush Trend60BackgroundBrush => GetSegmentBackground(IsTrendWindow60Selected);

    public Brush Trend60ForegroundBrush => GetSegmentForeground(IsTrendWindow60Selected);

    public Brush Trend120BackgroundBrush => GetSegmentBackground(IsTrendWindow120Selected);

    public Brush Trend120ForegroundBrush => GetSegmentForeground(IsTrendWindow120Selected);

    public Brush CombinedModeBackgroundBrush => GetSegmentBackground(IsCpuCombinedMode);

    public Brush CombinedModeForegroundBrush => GetSegmentForeground(IsCpuCombinedMode);

    public Brush LogicalModeBackgroundBrush => GetSegmentBackground(IsCpuLogicalMode);

    public Brush LogicalModeForegroundBrush => GetSegmentForeground(IsCpuLogicalMode);

    [RelayCommand]
    private void SelectInspectorSection(string? sectionTag)
    {
        if (!Enum.TryParse(sectionTag, ignoreCase: true, out InspectorSection parsed))
        {
            return;
        }

        InspectorSection = parsed;
    }

    private void RaiseInspectorSectionProperties()
    {
        RaiseProperties(
            nameof(IsSummarySectionSelected),
            nameof(IsPerformanceSectionSelected),
            nameof(IsDetailsSectionSelected),
            nameof(SummarySectionVisibility),
            nameof(SystemSummarySectionVisibility),
            nameof(ProcessSummarySectionVisibility),
            nameof(PerformanceSectionVisibility),
            nameof(DetailsSectionVisibility),
            nameof(SummaryTabBackgroundBrush),
            nameof(SummaryTabForegroundBrush),
            nameof(PerformanceTabBackgroundBrush),
            nameof(PerformanceTabForegroundBrush),
            nameof(DetailsTabBackgroundBrush),
            nameof(DetailsTabForegroundBrush));
    }

    private void RaiseTrendWindowChromeProperties()
    {
        RaiseProperties(
            nameof(Trend60BackgroundBrush),
            nameof(Trend60ForegroundBrush),
            nameof(Trend120BackgroundBrush),
            nameof(Trend120ForegroundBrush));
    }

    private void RaiseCpuModeChromeProperties()
    {
        RaiseProperties(
            nameof(CombinedModeBackgroundBrush),
            nameof(CombinedModeForegroundBrush),
            nameof(LogicalModeBackgroundBrush),
            nameof(LogicalModeForegroundBrush));
    }

    private void RaisePresentationProperties()
    {
        RaiseProperties(
            nameof(InspectorLayoutMode),
            nameof(InspectorContextTitle),
            nameof(InspectorContextSubtitle),
            nameof(IsSystemOverview),
            nameof(IsProcessInspector),
            nameof(InspectorOverviewEyebrow),
            nameof(InspectorOverviewSummary),
            nameof(InspectorOverviewSummaryVisibility),
            nameof(SystemSummaryLayoutVisibility),
            nameof(ProcessSummaryLayoutVisibility),
            nameof(InspectorChartMaxWidth),
            nameof(SummaryStatCardWidth),
            nameof(SummaryStatCards),
            nameof(DetailsPaneTitle),
            nameof(DetailsPanePrimaryText),
            nameof(DetailsPaneSecondaryText));
    }

    private void SetRuntimeStatusPresentation(RuntimeStatusTone tone, string title, string summary)
    {
        RuntimeStatusTone = tone;
        RuntimeStatusTag = tone switch
        {
            RuntimeStatusTone.Success => "HEALTH",
            RuntimeStatusTone.Warning => "WARN",
            RuntimeStatusTone.Error => "ERROR",
            _ => "INFO",
        };
        RuntimeStatusTitle = title;
        RuntimeStatusSummary = summary;
        IsRuntimeStatusVisible = true;
        RaisePresentationProperties();
    }

    private Brush GetSegmentBackground(bool isSelected)
    {
        return ResolveBrush(isSelected ? "BatCaveSegmentSelectedBrush" : "BatCaveSegmentSurfaceBrush");
    }

    private Brush GetSegmentForeground(bool isSelected)
    {
        return ResolveBrush(isSelected ? "BatCaveOnPrimaryBrush" : "BatCaveTextPrimaryBrush");
    }

    private static Brush ResolveBrush(string resourceKey)
    {
        if (Application.Current?.Resources.TryGetValue(resourceKey, out object? resource) == true
            && resource is Brush brush)
        {
            return brush;
        }

        return new SolidColorBrush(Microsoft.UI.Colors.Transparent);
    }
}
