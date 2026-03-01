# Verifiable Research and Technology Proposal

## 1. Core Problem Analysis
The project needs a Windows-native WinUI implementation that preserves AlbertsCave v1 monitoring behavior (1-second process telemetry, runtime health, identity-safe deltas, admin elevation flow, and local persistence) while replacing Tauri + Rust + React with .NET components. The primary technical challenges are preserving telemetry fidelity and responsiveness under churn while moving to WinUI architecture and dual packaged/unpackaged deployment.

## 2. Verifiable Technology Recommendations
| Technology/Pattern | Rationale & Evidence |
|---|---|
| **WinUI 3 + Windows App SDK** | Microsoft documents WinUI 3 as the modern Windows UI stack and provides first-app guidance for Windows App SDK projects in C# and XAML [cite:2]. This directly matches the target blank WinUI app baseline already present in BatCave [cite:2]. |
| **Dual deployment support (packaged + unpackaged)** | Windows App deployment guidance distinguishes packaged and unpackaged app models and their tradeoffs [cite:3]. Microsoft also provides explicit unpackaged WinUI setup guidance, which supports the requested dual mode day-1 target [cite:4]. |
| **Generic Host + DI for app composition** | The .NET Generic Host is designed to centralize DI, logging, configuration, and app lifetime concerns [cite:1]. Using it in BatCave keeps runtime services and WinUI shell composition testable and modular during the port [cite:1]. |
| **CommunityToolkit.Mvvm source generators** | ObservableProperty reduces ViewModel boilerplate by generating observable properties from annotated fields [cite:6]. RelayCommand generates ICommand and async command plumbing suitable for WinUI interaction handlers like sort, retry, and admin toggle actions [cite:7]. |
| **x:Bind-first WinUI data binding** | Microsoft WinUI binding guidance explains x:Bind modes and compile-time binding semantics for app bindings [cite:5]. This is a fit for strongly typed, high-frequency telemetry surfaces where binding errors should be caught early [cite:5]. |
| **Virtualized process list rendering** | Microsoft performance guidance for ListView/GridView documents UI virtualization and related optimization controls for large item sets [cite:8]. This aligns with v1 requirements to keep UI responsive under high process churn [cite:8]. |
| **Win32 process telemetry via ToolHelp + process query APIs** | CreateToolhelp32Snapshot is a documented Win32 mechanism for creating process snapshots used in process enumeration pipelines [cite:9]. The required metrics are available from documented APIs: OpenProcess, GetProcessTimes, GetProcessIoCounters, GetProcessMemoryInfo, GetProcessHandleCount, and GetSystemTimes [cite:18][cite:13][cite:10][cite:11][cite:12][cite:14]. |
| **Process metadata lookup via WMI/CIM** | The Win32_Process schema includes process metadata such as parent process relationship, command line, and executable path [cite:15]. This supports parity for process detail expansion without introducing outbound dependencies [cite:15]. |
| **Admin elevation using elevated helper process** | .NET process launch semantics document ProcessStartInfo.Verb and UseShellExecute, including shell-verb usage needed for elevation flows [cite:16][cite:17]. This supports a C# equivalent of AlbertsCave’s elevated bridge helper model for admin telemetry mode [cite:16][cite:17]. |

## 3. Browsed Sources
- [1] https://learn.microsoft.com/en-us/dotnet/core/extensions/generic-host
- [2] https://learn.microsoft.com/en-us/windows/apps/winui/winui3/create-your-first-winui3-app
- [3] https://learn.microsoft.com/en-us/windows/apps/package-and-deploy/deploy-overview
- [4] https://learn.microsoft.com/en-us/windows/apps/package-and-deploy/unpackage-winui-app
- [5] https://learn.microsoft.com/en-us/windows/apps/develop/data-binding/data-binding-in-depth
- [6] https://learn.microsoft.com/en-us/dotnet/communitytoolkit/mvvm/generators/observableproperty
- [7] https://learn.microsoft.com/en-us/dotnet/communitytoolkit/mvvm/generators/relaycommand
- [8] https://learn.microsoft.com/en-us/windows/uwp/debug-test-perf/optimize-gridview-and-listview
- [9] https://learn.microsoft.com/en-us/windows/win32/api/tlhelp32/nf-tlhelp32-createtoolhelp32snapshot
- [10] https://learn.microsoft.com/en-us/windows/win32/api/ioapiset/nf-ioapiset-getprocessiocounters
- [11] https://learn.microsoft.com/en-us/windows/win32/api/psapi/nf-psapi-getprocessmemoryinfo
- [12] https://learn.microsoft.com/en-us/windows/win32/api/processthreadsapi/nf-processthreadsapi-getprocesshandlecount
- [13] https://learn.microsoft.com/en-us/windows/win32/api/processthreadsapi/nf-processthreadsapi-getprocesstimes
- [14] https://learn.microsoft.com/en-us/windows/win32/api/processthreadsapi/nf-processthreadsapi-getsystemtimes
- [15] https://learn.microsoft.com/en-us/windows/win32/cimwin32prov/win32-process
- [16] https://learn.microsoft.com/en-us/dotnet/api/system.diagnostics.processstartinfo.verb
- [17] https://learn.microsoft.com/en-us/dotnet/api/system.diagnostics.processstartinfo.useshellexecute
- [18] https://learn.microsoft.com/en-us/windows/win32/api/processthreadsapi/nf-processthreadsapi-openprocess
