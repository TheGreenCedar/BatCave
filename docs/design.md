# Design Document

## Overview
The WinUI port separates UI composition (`BatCave`) from monitoring/runtime logic (`BatCave.Core`). The UI layer owns interaction state and rendering. The core layer owns launch policy, collection, telemetry pipeline, state, sorting/filtering, persistence, runtime cadence, and CLI operational modes.

## Principles
1. Preserve monitor-only behavior and v1 data contracts.
2. Keep identity safety with `(pid,start_time_ms)` across all lifecycle logic.
3. Keep local-only persistence and diagnostics.
4. Keep UI responsive by combining virtualization and incremental ordering.
5. Keep deployment and operations practical with packaged + unpackaged support and CLI parity.

## Component Specifications

#### Component: WinUiLaunchHost
**Purpose**: Bootstrap app startup, DI, and window lifecycle for packaged/unpackaged runs.
**Location**: `BatCave/App.xaml.cs`, `BatCave/BatCave.csproj`
**Interface**:
```csharp
public sealed class WinUiLaunchHost
{
    public IHost BuildHost();
    public Task<int> RunAsync(string[] args);
    // Implements Req 10.1, 10.2, 10.3, 10.4
}
```

#### Component: CliOperationsHost
**Purpose**: Handle `--print-gate-status`, `--benchmark`, and helper-mode argument routing.
**Location**: `BatCave.Core/Operations/CliOperationsHost.cs`
**Interface**:
```csharp
public interface ICliOperationsHost
{
    bool IsCliMode(string[] args);
    Task<int> ExecuteAsync(string[] args, CancellationToken ct);
    // Implements Req 11.1, 11.2, 11.3, 11.4
}
```

#### Component: LaunchPolicyGate
**Purpose**: Enforce startup gate with Windows and Windows 11 build checks.
**Location**: `BatCave.Core/Policy/LaunchPolicyGate.cs`
**Interface**:
```csharp
public interface ILaunchPolicyGate
{
    StartupGateStatus Enforce();
    // Implements Req 1.1, 1.2, 1.3
}
```

#### Component: MonitoringRuntime
**Purpose**: Coordinate collector, pipeline, state, sort/query, persistence, and health.
**Location**: `BatCave.Core/Runtime/MonitoringRuntime.cs`
**Interface**:
```csharp
public interface IMonitoringRuntime
{
    QueryResponse GetSnapshot();
    RuntimeHealth GetRuntimeHealth();
    void SetSort(SortColumn sortCol, SortDirection sortDir);
    void SetFilter(string filterText);
    bool IsAdminMode();
    Task RestartAsync(bool adminMode, CancellationToken ct);
    TickOutcome Tick(double jitterMs);
    void RecordDroppedTicks(ulong dropped);
    // Implements Req 4.2, 5.1, 5.2, 6.4, 7.1
}
```

#### Component: RuntimeLoopService
**Purpose**: Execute the 1-second loop and generation-aware lifecycle.
**Location**: `BatCave.Core/Runtime/RuntimeLoopService.cs`
**Interface**:
```csharp
public interface IRuntimeLoopService
{
    void Start(long generation);
    void StopAndAdvanceGeneration();
    // Implements Req 4.1, 4.4
}
```

#### Component: WindowsProcessCollector
**Purpose**: Sample process metrics via Win32 APIs.
**Location**: `BatCave.Core/Collector/WindowsProcessCollector.cs`
**Interface**:
```csharp
public interface IProcessCollector
{
    IReadOnlyList<ProcessSample> CollectTick(ulong seq);
    string? TakeWarning();
    // Implements Req 2.1, 2.2, 2.3, 2.4
}
```

#### Component: ElevatedBridgeClient
**Purpose**: Launch/poll elevated helper snapshots and manage bridge faults.
**Location**: `BatCave.Core/Collector/ElevatedBridgeClient.cs`
**Interface**:
```csharp
public sealed class ElevatedBridgeClient
{
    public static Task<ElevatedBridgeClient> LaunchAsync(CancellationToken ct);
    public BridgePollResult PollRows();
    public void Dispose();
    // Implements Req 7.2, 7.3
}
```

#### Component: DeltaTelemetryPipeline
**Purpose**: Convert raw samples into upserts/exits and heartbeat updates.
**Location**: `BatCave.Core/Pipeline/DeltaTelemetryPipeline.cs`
**Interface**:
```csharp
public interface ITelemetryPipeline
{
    void SeedFromWarmCache(IReadOnlyList<ProcessSample> rows);
    ProcessDeltaBatch ApplyRaw(ulong seq, IReadOnlyList<ProcessSample> raw);
    // Implements Req 3.1, 3.2, 3.3, 3.4
}
```

#### Component: InMemoryStateStore
**Purpose**: Maintain live process rows and warm-cache state.
**Location**: `BatCave.Core/State/InMemoryStateStore.cs`
**Interface**:
```csharp
public interface IStateStore
{
    void ApplyDelta(ProcessDeltaBatch delta);
    IReadOnlyList<ProcessSample> AllRows();
    WarmCache ExportWarmCache(ulong seq);
    void ImportWarmCache(WarmCache cache);
    int RowCount();
    void CompactTo(int maxRows);
}
```

#### Component: IncrementalSortIndexEngine
**Purpose**: Serve filtered/sorted query responses with incremental ordering updates.
**Location**: `BatCave.Core/Sort/IncrementalSortIndexEngine.cs`
**Interface**:
```csharp
public interface ISortIndexEngine
{
    void OnDelta(ProcessDeltaBatch delta);
    QueryResponse Query(QueryRequest request, IReadOnlyList<ProcessSample> rows, ulong seq);
    // Implements Req 5.1, 5.2, 5.3, 5.4
}
```

#### Component: LocalJsonPersistenceStore
**Purpose**: Persist settings, warm cache, and local diagnostics.
**Location**: `BatCave.Core/Persistence/LocalJsonPersistenceStore.cs`
**Interface**:
```csharp
public interface IPersistenceStore
{
    UserSettings? LoadSettings();
    Task SaveSettingsAsync(UserSettings settings, CancellationToken ct);
    WarmCache? LoadWarmCache();
    Task SaveWarmCacheAsync(WarmCache cache, CancellationToken ct);
    string BaseDirectory { get; }
    // Implements Req 6.1, 6.2, 6.3
}
```

#### Component: ProcessMetadataProvider
**Purpose**: Fetch and validate metadata for process identity.
**Location**: `BatCave.Core/Metadata/ProcessMetadataProvider.cs`
**Interface**:
```csharp
public interface IProcessMetadataProvider
{
    Task<ProcessMetadata?> GetAsync(uint pid, ulong startTimeMs, CancellationToken ct);
    // Implements Req 8.1, 8.2
}
```

#### Component: RuntimeEventGateway
**Purpose**: Deliver runtime events to UI subscribers.
**Location**: `BatCave/Services/RuntimeEventGateway.cs`
**Interface**:
```csharp
public interface IRuntimeEventGateway
{
    event EventHandler<ProcessDeltaBatch> TelemetryDelta;
    event EventHandler<RuntimeHealth> RuntimeHealthChanged;
    event EventHandler<CollectorWarning> CollectorWarningRaised;
    // Implements Req 4.3
}
```

#### Component: MonitoringShellViewModel
**Purpose**: Orchestrate UI states, sort/filter/admin toggles, selection, metadata caching, and retry logic.
**Location**: `BatCave/ViewModels/MonitoringShellViewModel.cs`
**Interface**:
```csharp
public sealed partial class MonitoringShellViewModel : ObservableObject
{
    public Task BootstrapAsync(CancellationToken ct);
    public Task RetryBootstrapAsync(CancellationToken ct);
    public Task ToggleAdminModeAsync(bool nextAdminMode, CancellationToken ct);
    public void ChangeSort(SortColumn column);
    public void ChangeFilter(string filterText);
    public void ToggleSelection(ProcessSample row);
    // Implements Req 1.4, 7.4, 8.3, 8.4, 9.1, 9.3
}
```

#### Component: MonitoringShellView
**Purpose**: Render top bar, virtualized table, detail panel, and health footer.
**Location**: `BatCave/MainWindow.xaml`, `BatCave/Views/*`
**Interface**:
```xaml
<!-- Main composition regions -->
<Grid>
  <!-- Top bar, main monitoring region, runtime health footer -->
</Grid>
<!-- Implements Req 9.1, 9.2, 9.4 -->
```
