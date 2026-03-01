# Requirements Document

## Introduction
This document defines the functional and non-functional requirements for porting AlbertsCave to a native WinUI + .NET implementation in BatCave while preserving v1 monitoring behavior and operational workflows.

## Glossary
- **Identity Key**: Tuple `(pid,start_time_ms)` used to prevent PID reuse collisions.
- **Delta Batch**: Tick-scoped set of process upserts and exits.
- **Runtime Health**: App-overhead and cadence quality metrics.
- **Admin Mode**: Elevated collection mode using an elevated helper bridge.
- **Warm Cache**: Persisted live row snapshot used to hydrate startup continuity.

## Requirements

### Requirement 1: Launch Policy Enforcement and Blocked Startup UX

#### Acceptance Criteria
1. WHEN the app starts on a non-Windows platform, THE **LaunchPolicyGate** SHALL block runtime startup and return `unsupported_platform`.
2. WHEN the app starts on Windows with build `< 22000`, THE **LaunchPolicyGate** SHALL block runtime startup and return `requires_windows_11` with detected build.
3. WHEN the app starts on Windows with build `>= 22000`, THE **LaunchPolicyGate** SHALL allow runtime startup.
4. WHEN startup is blocked, THE **MonitoringShellViewModel** SHALL expose a non-crashing blocked-state message and prevent runtime loop activation.

### Requirement 2: Process Collection Fidelity

#### Acceptance Criteria
1. WHEN each tick executes, THE **WindowsProcessCollector** SHALL enumerate active processes and capture `pid`, `parent_pid`, `name`, and `threads`.
2. WHEN previous counters are available, THE **WindowsProcessCollector** SHALL compute `cpu_pct`, `io_read_bps`, `io_write_bps`, and `net_bps` from counter deltas.
3. WHEN process query rights vary, THE **WindowsProcessCollector** SHALL assign `access_state` as `full`, `partial`, or `denied` based on available metrics.
4. WHEN process creation time is unavailable or changes, THE **WindowsProcessCollector** SHALL maintain a stable fallback start-time strategy keyed by PID fingerprint.

### Requirement 3: Identity-Safe Delta Processing

#### Acceptance Criteria
1. WHEN raw samples change for an identity, THE **DeltaTelemetryPipeline** SHALL emit an upsert for that identity.
2. WHEN an identity disappears between ticks, THE **DeltaTelemetryPipeline** SHALL emit that identity in `exits`.
3. WHEN samples are unchanged across ticks, THE **DeltaTelemetryPipeline** SHALL emit heartbeat upserts at the configured interval.
4. WHEN warm cache rows are seeded before first live tick, THE **DeltaTelemetryPipeline** SHALL reconcile stale identities via `exits`.

### Requirement 4: Runtime Cadence and Health Emission

#### Acceptance Criteria
1. WHEN monitoring is active, THE **RuntimeLoopService** SHALL schedule tick execution at a default 1-second cadence.
2. WHEN each tick completes, THE **MonitoringRuntime** SHALL update `last_tick_ms`, `jitter_p95_ms`, and `dropped_ticks`.
3. WHEN tick outputs are produced, THE **RuntimeEventGateway** SHALL publish `telemetry_delta`, `runtime_health`, and collector-warning events.
4. WHEN backend restart occurs, THE **RuntimeLoopService** SHALL stop the previous generation loop and continue only the latest generation.

### Requirement 5: Snapshot Query, Sorting, and Filtering

#### Acceptance Criteria
1. WHEN a snapshot query is requested, THE **IncrementalSortIndexEngine** SHALL return `seq`, `total`, and paged `rows`.
2. WHEN sort preference changes, THE **IncrementalSortIndexEngine** SHALL apply supported sort columns and directions matching v1 contract.
3. WHEN filter text is provided, THE **IncrementalSortIndexEngine** SHALL filter by process `name` or `pid` substring.
4. WHEN delta updates are small relative to row count, THE **IncrementalSortIndexEngine** SHALL apply incremental ordering updates instead of mandatory full re-sort.

### Requirement 6: Local Persistence and Diagnostics

#### Acceptance Criteria
1. WHEN sort/filter/admin settings change, THE **LocalJsonPersistenceStore** SHALL persist settings under `%LOCALAPPDATA%\\BatCaveMonitor\\settings.json`.
2. WHEN runtime warm-cache save points are reached, THE **LocalJsonPersistenceStore** SHALL persist warm-cache under `%LOCALAPPDATA%\\BatCaveMonitor\\warm-cache.json`.
3. WHEN diagnostics logging is active, THE **LocalJsonPersistenceStore** SHALL write structured local logs under `%LOCALAPPDATA%\\BatCaveMonitor\\logs`.
4. WHEN runtime diagnostics are emitted, THE **MonitoringRuntime** SHALL keep telemetry and diagnostics local-only with no outbound transmission.

### Requirement 7: Admin Mode Elevation and Access Controls

#### Acceptance Criteria
1. WHEN admin mode is toggled, THE **MonitoringRuntime** SHALL restart collector execution in requested admin/non-admin mode.
2. WHEN admin mode is enabled, THE **ElevatedBridgeClient** SHALL launch and poll an elevated helper using tokenized snapshot files.
3. WHEN elevated bridge polling faults, THE **ElevatedBridgeClient** SHALL surface fault state and warning context to runtime.
4. WHEN admin mode is disabled, THE **MonitoringShellViewModel** SHALL hide denied rows by default and gate admin-enabled-only filtering.

### Requirement 8: Process Metadata Detail Parity

#### Acceptance Criteria
1. WHEN process detail is requested, THE **ProcessMetadataProvider** SHALL return parent PID, command line, and executable path when available.
2. WHEN metadata identity start time mismatches requested identity, THE **ProcessMetadataProvider** SHALL return `null`.
3. WHEN metadata retrieval fails, THE **MonitoringShellViewModel** SHALL expose non-fatal error state while preserving live monitoring.
4. WHEN metadata is successfully retrieved, THE **MonitoringShellViewModel** SHALL cache results per process identity to avoid redundant lookups.

### Requirement 9: WinUI Shell Interaction Parity

#### Acceptance Criteria
1. WHEN the app initializes, THE **MonitoringShellView** SHALL support loading, blocked, startup-error, and live-monitor states.
2. WHEN live data is shown, THE **MonitoringShellView** SHALL provide a virtualized sortable process list with row selection.
3. WHEN row selection changes, THE **MonitoringShellViewModel** SHALL drive detail panel focus and clear-selection fallback to global summary.
4. WHEN health data changes, THE **MonitoringShellView** SHALL render runtime health footer metrics and unavailable fallback messaging.

### Requirement 10: Dual Deployment Mode Support

#### Acceptance Criteria
1. WHEN packaged build/publish is executed, THE **WinUiLaunchHost** SHALL support MSIX packaged deployment.
2. WHEN unpackaged profile is executed, THE **WinUiLaunchHost** SHALL support unpackaged launch and Windows App SDK bootstrap.
3. WHEN platform targets are built, THE **WinUiLaunchHost** SHALL support x86, x64, and ARM64 configurations.
4. WHEN app mode changes between packaged and unpackaged, THE **WinUiLaunchHost** SHALL preserve equivalent runtime behavior.

### Requirement 11: CLI Operational Parity

#### Acceptance Criteria
1. WHEN `--print-gate-status` is invoked, THE **CliOperationsHost** SHALL output `StartupGateStatus` JSON and use pass/block exit codes.
2. WHEN `--benchmark` is invoked, THE **CliOperationsHost** SHALL execute benchmark ticks and output summary JSON.
3. WHEN strict benchmark conditions are met, THE **CliOperationsHost** SHALL enforce CPU/RSS budget gate outcome.
4. WHEN developers run project scripts, THE **CliOperationsHost** SHALL support PowerShell `run-dev.ps1` and `run-benchmark.ps1` workflows.
