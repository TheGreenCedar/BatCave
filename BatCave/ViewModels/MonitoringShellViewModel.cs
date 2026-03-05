using BatCave.Core.Abstractions;
using BatCave.Core.Domain;
using BatCave.Core.Runtime;
using BatCave.Services;
using CommunityToolkit.Mvvm.ComponentModel;
using DynamicData;
using Microsoft.UI.Dispatching;
using Microsoft.UI.Xaml;
using System;
using System.Collections.Generic;
using System.Collections.ObjectModel;
using System.Reactive.Subjects;
using System.Threading;

namespace BatCave.ViewModels;

public enum DetailMetricFocus
{
    Cpu,
    Memory,
    IoRead,
    IoWrite,
    OtherIo,
}

public partial class MonitoringShellViewModel : ObservableObject
{
    private const int FilterDebounceMs = 160;
    private const int HistoryLimit = 120;

    private readonly ILaunchPolicyGate _launchPolicyGate;
    private readonly MonitoringRuntime _runtime;
    private readonly RuntimeLoopService _runtimeLoopService;
    private readonly IRuntimeEventGateway _runtimeEventGateway;
    private readonly IRuntimeHealthService _runtimeHealthService;
    private readonly IProcessMetadataProvider _metadataProvider;
    private readonly ISystemGlobalMetricsSampler _systemGlobalMetricsSampler;

    private readonly Dictionary<ProcessIdentity, ProcessSample> _allRows = [];
    private readonly Dictionary<ProcessIdentity, ProcessMetadata?> _metadataCache = [];
    private readonly Dictionary<ProcessIdentity, MetricHistoryBuffer> _metricHistory = [];
    private readonly Dictionary<ProcessIdentity, ulong> _metricHistoryLastSeq = [];
    private readonly Dictionary<ProcessIdentity, ProcessRowViewState> _visibleRowStateByIdentity = [];
    private readonly SourceCache<ProcessRowViewState, ProcessIdentity> _rowViewSource = new(row => row.Identity);
    private readonly ReadOnlyObservableCollection<ProcessRowViewState> _visibleRows;
    private readonly BehaviorSubject<Func<ProcessRowViewState, bool>> _rowFilter;
    private readonly BehaviorSubject<IComparer<ProcessRowViewState>> _rowSorter;
    private readonly IDisposable _rowShapingSubscription;
    private readonly MetricHistoryBuffer _globalHistory = new(HistoryLimit);

    private DispatcherQueue? _dispatcherQueue;
    private CancellationTokenSource? _filterDebounceCts;

    private bool _isLoading = true;
    private bool _isBlocked;
    private bool _isStartupError;
    private bool _isLive;
    private string _shellHeadline = "Initializing monitor runtime...";
    private string _shellBody = "Starting monitoring services.";
    private string _blockedReasonMessage = string.Empty;
    private string _startupErrorMessage = string.Empty;
    private string _filterText = string.Empty;
    private SortColumn _currentSortColumn = SortColumn.CpuPct;
    private BatCave.Core.Domain.SortDirection _currentSortDirection = BatCave.Core.Domain.SortDirection.Desc;
    private bool _adminModeEnabled;
    private bool _adminEnabledOnlyFilter;
    private bool _adminModePending;
    private string? _adminModeError;
    private string _runtimeHealthStatus = "Runtime health unavailable.";
    private StartupGateStatus _startupGateStatus = new();
    private ProcessSample? _selectedRow;
    private ProcessRowViewState? _selectedVisibleRow;
    private ProcessMetadata? _selectedMetadata;
    private bool _isApplyingSelectedVisibleRowBinding;
    private bool _isMetadataLoading;
    private string? _metadataError;
    private DetailMetricFocus _metricFocus = DetailMetricFocus.Cpu;
    private int _metricTrendWindowSeconds = 60;
    private long _metadataRequestVersion;

    private ulong _summarySeq;
    private ulong _summaryTsMs;
    private double _summaryCpuPct;
    private double _summaryRssBytes;
    private double _summaryPrivateBytes;
    private double _summaryIoReadBps;
    private double _summaryIoWriteBps;
    private double _summaryOtherIoBps;
    private double _summaryThreads;
    private double _summaryHandles;
    private ProcessSample _globalSummaryRow = CreateEmptyGlobalSummary();

    private double[] _cpuMetricTrendValues = new double[60];
    private double[] _memoryMetricTrendValues = new double[60];
    private double[] _ioReadMetricTrendValues = new double[60];
    private double[] _ioWriteMetricTrendValues = new double[60];
    private double[] _otherIoMetricTrendValues = new double[60];
    private double[] _expandedMetricTrendValues = new double[60];

    private string _cpuMetricChipValue = "0.00%";
    private string _memoryMetricChipValue = "0 B";
    private string _ioReadMetricChipValue = "0 B/s";
    private string _ioWriteMetricChipValue = "0 B/s";
    private string _otherIoMetricChipValue = "0 B/s";
    private string _expandedMetricTitle = "CPU Trend";
    private string _expandedMetricValue = "0.0% CPU";
    private bool _isGlobalCpuAvailable = true;
    private bool _isGlobalMemoryAvailable = true;
    private bool _isGlobalIoReadAvailable = true;
    private bool _isGlobalIoWriteAvailable = true;
    private bool _isGlobalOtherIoAvailable = true;

    public MonitoringShellViewModel(
        ILaunchPolicyGate launchPolicyGate,
        MonitoringRuntime runtime,
        RuntimeLoopService runtimeLoopService,
        IRuntimeEventGateway runtimeEventGateway,
        IRuntimeHealthService runtimeHealthService,
        IProcessMetadataProvider metadataProvider,
        ISystemGlobalMetricsSampler systemGlobalMetricsSampler)
    {
        _launchPolicyGate = launchPolicyGate;
        _runtime = runtime;
        _runtimeLoopService = runtimeLoopService;
        _runtimeEventGateway = runtimeEventGateway;
        _runtimeHealthService = runtimeHealthService;
        _metadataProvider = metadataProvider;
        _systemGlobalMetricsSampler = systemGlobalMetricsSampler;

        _runtimeEventGateway.TelemetryDelta += OnTelemetryDelta;
        _runtimeEventGateway.RuntimeHealthChanged += OnRuntimeHealthChanged;
        _runtimeEventGateway.CollectorWarningRaised += OnCollectorWarningRaised;

        _rowFilter = new BehaviorSubject<Func<ProcessRowViewState, bool>>(BuildVisibilityFilter());
        _rowSorter = new BehaviorSubject<IComparer<ProcessRowViewState>>(BuildSortComparer(CurrentSortColumn, CurrentSortDirection));
        _rowShapingSubscription = _rowViewSource
            .Connect()
            .Filter(_rowFilter)
            .SortAndBind(out _visibleRows, _rowSorter)
            .Subscribe();

        ApplyCanonicalShaping();
        EnsureGlobalMetricsSamplingStarted();
        RefreshGlobalPerformanceState(new SystemGlobalMetricsSample());
        RuntimeHealthStatus = _runtimeHealthService.Snapshot().StatusSummary;
    }

    public IReadOnlyList<ProcessRowViewState> VisibleRows => _visibleRows;

    public bool IsLoading
    {
        get => _isLoading;
        private set => SetStateFlagAndRaiseVisibility(ref _isLoading, value);
    }

    public bool IsBlocked
    {
        get => _isBlocked;
        private set => SetStateFlagAndRaiseVisibility(ref _isBlocked, value);
    }

    public bool IsStartupError
    {
        get => _isStartupError;
        private set => SetStateFlagAndRaiseVisibility(ref _isStartupError, value);
    }

    public bool IsLive
    {
        get => _isLive;
        private set => SetStateFlagAndRaiseVisibility(ref _isLive, value);
    }

    public string ShellHeadline
    {
        get => _shellHeadline;
        private set => SetProperty(ref _shellHeadline, value);
    }

    public string ShellBody
    {
        get => _shellBody;
        private set => SetProperty(ref _shellBody, value);
    }

    public string BlockedReasonMessage
    {
        get => _blockedReasonMessage;
        private set => SetProperty(ref _blockedReasonMessage, value);
    }

    public string StartupErrorMessage
    {
        get => _startupErrorMessage;
        private set => SetProperty(ref _startupErrorMessage, value);
    }

    public string FilterText
    {
        get => _filterText;
        set
        {
            if (SetProperty(ref _filterText, value))
            {
                ScheduleFilterApply(value);
            }
        }
    }

    public SortColumn CurrentSortColumn
    {
        get => _currentSortColumn;
        private set
        {
            if (SetProperty(ref _currentSortColumn, value))
            {
                RaiseSortHeaderLabels();
            }
        }
    }

    public BatCave.Core.Domain.SortDirection CurrentSortDirection
    {
        get => _currentSortDirection;
        private set
        {
            if (SetProperty(ref _currentSortDirection, value))
            {
                RaiseSortHeaderLabels();
            }
        }
    }

    public bool AdminModeEnabled
    {
        get => _adminModeEnabled;
        private set
        {
            if (SetProperty(ref _adminModeEnabled, value))
            {
                if (!value)
                {
                    AdminEnabledOnlyFilter = false;
                }

                RefreshVisibleRows(refreshFilter: true);
            }
        }
    }

    public bool AdminEnabledOnlyFilter
    {
        get => _adminEnabledOnlyFilter;
        set
        {
            if (!AdminModeEnabled && value)
            {
                value = false;
            }

            if (SetProperty(ref _adminEnabledOnlyFilter, value))
            {
                RefreshVisibleRows(refreshFilter: true);
            }
        }
    }

    public bool AdminModePending
    {
        get => _adminModePending;
        private set => SetProperty(ref _adminModePending, value);
    }

    public string? AdminModeError
    {
        get => _adminModeError;
        private set
        {
            if (SetProperty(ref _adminModeError, value))
            {
                OnPropertyChanged(nameof(HasAdminModeError));
                OnPropertyChanged(nameof(AdminErrorVisibility));
            }
        }
    }

    public bool HasAdminModeError => !string.IsNullOrWhiteSpace(AdminModeError);

    public Visibility LoadingVisibility => IsLoading ? Visibility.Visible : Visibility.Collapsed;

    public Visibility BlockedVisibility => IsBlocked ? Visibility.Visible : Visibility.Collapsed;

    public Visibility StartupErrorVisibility => IsStartupError ? Visibility.Visible : Visibility.Collapsed;

    public Visibility LiveVisibility => IsLive ? Visibility.Visible : Visibility.Collapsed;

    public Visibility AdminErrorVisibility => HasAdminModeError ? Visibility.Visible : Visibility.Collapsed;

    public string RuntimeHealthStatus
    {
        get => _runtimeHealthStatus;
        private set => SetProperty(ref _runtimeHealthStatus, value);
    }

    public StartupGateStatus StartupGateStatus
    {
        get => _startupGateStatus;
        private set => SetProperty(ref _startupGateStatus, value);
    }

    public ProcessSample? SelectedRow
    {
        get => _selectedRow;
        private set
        {
            if (SetProperty(ref _selectedRow, value))
            {
                RaiseSelectedRowDerivedProperties();
            }
        }
    }

    public ProcessRowViewState? SelectedVisibleRow
    {
        get => _selectedVisibleRow;
        private set
        {
            if (SetProperty(ref _selectedVisibleRow, value))
            {
                RaiseSelectedVisibleRowBindingProperty();
            }
        }
    }

    public ProcessRowViewState? SelectedVisibleRowBinding
    {
        get => SelectedVisibleRow;
        set => ApplySelectedVisibleRowBinding(value);
    }

    public ProcessMetadata? SelectedMetadata
    {
        get => _selectedMetadata;
        private set
        {
            if (SetProperty(ref _selectedMetadata, value))
            {
                RaiseMetadataProperties();
            }
        }
    }

    public bool IsMetadataLoading
    {
        get => _isMetadataLoading;
        private set
        {
            if (SetProperty(ref _isMetadataLoading, value))
            {
                RaiseMetadataProperties();
            }
        }
    }

    public string? MetadataError
    {
        get => _metadataError;
        private set
        {
            if (SetProperty(ref _metadataError, value))
            {
                RaiseMetadataProperties();
            }
        }
    }

    public DetailMetricFocus MetricFocus
    {
        get => _metricFocus;
        set
        {
            if (SetProperty(ref _metricFocus, value))
            {
                RaiseMetricFocusProperties();
                RefreshDetailMetrics();
            }
        }
    }

    public bool IsCpuMetricFocused => MetricFocus == DetailMetricFocus.Cpu;

    public bool IsMemoryMetricFocused => MetricFocus == DetailMetricFocus.Memory;

    public bool IsIoReadMetricFocused => MetricFocus == DetailMetricFocus.IoRead;

    public bool IsIoWriteMetricFocused => MetricFocus == DetailMetricFocus.IoWrite;

    public bool IsOtherIoMetricFocused => MetricFocus == DetailMetricFocus.OtherIo;

    public int MetricTrendWindowSeconds
    {
        get => _metricTrendWindowSeconds;
        private set
        {
            int normalized = NormalizeMetricTrendWindowSeconds(value);
            if (!SetProperty(ref _metricTrendWindowSeconds, normalized))
            {
                return;
            }

            OnPropertyChanged(nameof(IsTrendWindow60Selected));
            OnPropertyChanged(nameof(IsTrendWindow120Selected));
            RefreshDetailMetrics();
            QueueGlobalDetailStateRefresh();
        }
    }

    public bool IsTrendWindow60Selected => MetricTrendWindowSeconds == 60;

    public bool IsTrendWindow120Selected => MetricTrendWindowSeconds == 120;

    public bool HasSelection => SelectedRow is not null;

    public string DetailTitle => BuildDetailTitle();

    public string DetailMetricValue => ExpandedMetricValue;

    public string MetadataStatus => ResolveMetadataStatus();

    public string MetadataParentPid =>
        SelectedMetadata is null
            ? "n/a"
            : SelectedMetadata.ParentPid.ToString();

    public string MetadataCommandLine =>
        string.IsNullOrWhiteSpace(SelectedMetadata?.CommandLine)
            ? "n/a"
            : SelectedMetadata.CommandLine!;

    public string MetadataExecutablePath =>
        string.IsNullOrWhiteSpace(SelectedMetadata?.ExecutablePath)
            ? "n/a"
            : SelectedMetadata.ExecutablePath!;

    public double[] CpuMetricTrendValues
    {
        get => _cpuMetricTrendValues;
        private set => SetProperty(ref _cpuMetricTrendValues, value);
    }

    public double[] MemoryMetricTrendValues
    {
        get => _memoryMetricTrendValues;
        private set => SetProperty(ref _memoryMetricTrendValues, value);
    }

    public double[] IoReadMetricTrendValues
    {
        get => _ioReadMetricTrendValues;
        private set => SetProperty(ref _ioReadMetricTrendValues, value);
    }

    public double[] IoWriteMetricTrendValues
    {
        get => _ioWriteMetricTrendValues;
        private set => SetProperty(ref _ioWriteMetricTrendValues, value);
    }

    public double[] OtherIoMetricTrendValues
    {
        get => _otherIoMetricTrendValues;
        private set => SetProperty(ref _otherIoMetricTrendValues, value);
    }

    public double[] ExpandedMetricTrendValues
    {
        get => _expandedMetricTrendValues;
        private set => SetProperty(ref _expandedMetricTrendValues, value);
    }

    public string CpuMetricChipValue
    {
        get => _cpuMetricChipValue;
        private set => SetProperty(ref _cpuMetricChipValue, value);
    }

    public string MemoryMetricChipValue
    {
        get => _memoryMetricChipValue;
        private set => SetProperty(ref _memoryMetricChipValue, value);
    }

    public string IoReadMetricChipValue
    {
        get => _ioReadMetricChipValue;
        private set => SetProperty(ref _ioReadMetricChipValue, value);
    }

    public string IoWriteMetricChipValue
    {
        get => _ioWriteMetricChipValue;
        private set => SetProperty(ref _ioWriteMetricChipValue, value);
    }

    public string OtherIoMetricChipValue
    {
        get => _otherIoMetricChipValue;
        private set => SetProperty(ref _otherIoMetricChipValue, value);
    }

    public string ExpandedMetricTitle
    {
        get => _expandedMetricTitle;
        private set => SetProperty(ref _expandedMetricTitle, value);
    }

    public string ExpandedMetricValue
    {
        get => _expandedMetricValue;
        private set
        {
            if (SetProperty(ref _expandedMetricValue, value))
            {
                OnPropertyChanged(nameof(DetailMetricValue));
            }
        }
    }

    public void AttachDispatcherQueue(DispatcherQueue dispatcherQueue)
    {
        _dispatcherQueue = dispatcherQueue;
    }

    private void SetStateFlagAndRaiseVisibility(ref bool stateField, bool value)
    {
        if (SetProperty(ref stateField, value))
        {
            RaiseStateVisibilityProperties();
        }
    }

    private void RaiseSelectedRowDerivedProperties()
    {
        OnPropertyChanged(nameof(HasSelection));
        OnPropertyChanged(nameof(DetailTitle));
        RaiseGlobalModeProperties();
        BuildAndAppendResourceRows(_latestGlobalMetricsSample);
        RaiseMetadataProperties();
        RefreshDetailMetrics();
        RefreshGlobalDetailState();
    }

    private void RaiseSelectedVisibleRowBindingProperty()
    {
        OnPropertyChanged(nameof(SelectedVisibleRowBinding));
    }

    private string BuildDetailTitle()
    {
        ProcessSample? selected = SelectedRow;
        return selected is null
            ? _globalSummaryRow.Name
            : $"{selected.Name} ({selected.Pid})";
    }

    private string ResolveMetadataStatus()
    {
        if (SelectedRow is null)
        {
            return "Select a process to load metadata.";
        }

        if (IsMetadataLoading)
        {
            return "Loading metadata...";
        }

        if (!string.IsNullOrWhiteSpace(MetadataError))
        {
            return $"Metadata error: {MetadataError}";
        }

        return SelectedMetadata is null
            ? "Metadata unavailable for this process identity."
            : "Metadata loaded.";
    }

    private static int NormalizeMetricTrendWindowSeconds(int seconds)
    {
        return seconds >= 120 ? 120 : 60;
    }
}
