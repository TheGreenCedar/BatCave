using System;
using System.Collections.Generic;
using System.Collections.ObjectModel;
using System.Linq;
using System.Threading;
using System.Threading.Tasks;
using BatCave.Charts;
using BatCave.Converters;
using BatCave.Core.Abstractions;
using BatCave.Core.Domain;
using BatCave.Core.Runtime;
using BatCave.Services;
using CommunityToolkit.Mvvm.ComponentModel;
using CommunityToolkit.Mvvm.Input;
using Microsoft.UI.Dispatching;
using Microsoft.UI.Xaml;
using AdvancedCollectionView = CommunityToolkit.WinUI.Collections.AdvancedCollectionView;
using SortDescription = CommunityToolkit.WinUI.Collections.SortDescription;

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
    private const ulong RowSparklineStride = 2;
    private const double RowSparklineWidth = 96;
    private const double RowSparklineHeight = 22;

    private readonly ILaunchPolicyGate _launchPolicyGate;
    private readonly MonitoringRuntime _runtime;
    private readonly RuntimeLoopService _runtimeLoopService;
    private readonly IRuntimeEventGateway _runtimeEventGateway;
    private readonly IProcessMetadataProvider _metadataProvider;

    private readonly Dictionary<ProcessIdentity, ProcessSample> _allRows = new();
    private readonly Dictionary<ProcessIdentity, ProcessMetadata?> _metadataCache = new();
    private readonly Dictionary<ProcessIdentity, MetricHistoryBuffer> _metricHistory = new();
    private readonly Dictionary<ProcessIdentity, ulong> _metricHistoryLastSeq = new();
    private readonly Dictionary<ProcessIdentity, ProcessRowViewState> _visibleRowStateByIdentity = new();
    private readonly ObservableCollection<ProcessRowViewState> _rowViewSource = [];
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
    private SortDirection _currentSortDirection = SortDirection.Desc;
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

    private double[] _cpuMetricTrendValues = [];
    private double[] _memoryMetricTrendValues = [];
    private double[] _ioReadMetricTrendValues = [];
    private double[] _ioWriteMetricTrendValues = [];
    private double[] _otherIoMetricTrendValues = [];
    private double[] _expandedMetricTrendValues = [];

    private string _cpuMetricChipValue = "0.00%";
    private string _memoryMetricChipValue = "0 B";
    private string _ioReadMetricChipValue = "0 B/s";
    private string _ioWriteMetricChipValue = "0 B/s";
    private string _otherIoMetricChipValue = "0 B/s";
    private string _expandedMetricTitle = "CPU Trend";
    private string _expandedMetricValue = "0.0% CPU";

    public MonitoringShellViewModel(
        ILaunchPolicyGate launchPolicyGate,
        MonitoringRuntime runtime,
        RuntimeLoopService runtimeLoopService,
        IRuntimeEventGateway runtimeEventGateway,
        IProcessMetadataProvider metadataProvider)
    {
        _launchPolicyGate = launchPolicyGate;
        _runtime = runtime;
        _runtimeLoopService = runtimeLoopService;
        _runtimeEventGateway = runtimeEventGateway;
        _metadataProvider = metadataProvider;

        _runtimeEventGateway.TelemetryDelta += OnTelemetryDelta;
        _runtimeEventGateway.RuntimeHealthChanged += OnRuntimeHealthChanged;
        _runtimeEventGateway.CollectorWarningRaised += OnCollectorWarningRaised;

        VisibleRows = new AdvancedCollectionView(_rowViewSource, true);
        VisibleRows.Filter = ShouldShowRow;
        ApplySortDescriptions();
    }

    public AdvancedCollectionView VisibleRows { get; }

    public bool IsLoading
    {
        get => _isLoading;
        private set
        {
            if (SetProperty(ref _isLoading, value))
            {
                RaiseStateVisibilityProperties();
            }
        }
    }

    public bool IsBlocked
    {
        get => _isBlocked;
        private set
        {
            if (SetProperty(ref _isBlocked, value))
            {
                RaiseStateVisibilityProperties();
            }
        }
    }

    public bool IsStartupError
    {
        get => _isStartupError;
        private set
        {
            if (SetProperty(ref _isStartupError, value))
            {
                RaiseStateVisibilityProperties();
            }
        }
    }

    public bool IsLive
    {
        get => _isLive;
        private set
        {
            if (SetProperty(ref _isLive, value))
            {
                RaiseStateVisibilityProperties();
            }
        }
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

    public SortDirection CurrentSortDirection
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
                OnPropertyChanged(nameof(HasSelection));
                OnPropertyChanged(nameof(DetailTitle));
                RaiseMetadataProperties();
                RefreshDetailMetrics();
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
                OnPropertyChanged(nameof(SelectedVisibleRowBinding));
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

    public bool HasSelection => SelectedRow is not null;

    public string DetailTitle =>
        SelectedRow is null
            ? _globalSummaryRow.Name
            : $"{SelectedRow.Name} ({SelectedRow.Pid})";

    public string DetailMetricValue => ExpandedMetricValue;

    public string MetadataStatus
    {
        get
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

            if (SelectedMetadata is null)
            {
                return "Metadata unavailable for this process identity.";
            }

            return "Metadata loaded.";
        }
    }

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

    public string NameSortLabel => SortLabel("Name", SortColumn.Name);

    public string PidSortLabel => SortLabel("PID", SortColumn.Pid);

    public string CpuSortLabel => SortLabel("CPU", SortColumn.CpuPct);

    public string MemorySortLabel => SortLabel("Memory", SortColumn.RssBytes);

    public string IoReadSortLabel => SortLabel("IO Read", SortColumn.IoReadBps);

    public string IoWriteSortLabel => SortLabel("IO Write", SortColumn.IoWriteBps);

    public string OtherIoSortLabel => SortLabel("Other I/O", SortColumn.OtherIoBps);

    public string ThreadsSortLabel => SortLabel("Threads", SortColumn.Threads);

    public string HandlesSortLabel => SortLabel("Handles", SortColumn.Handles);

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
}
