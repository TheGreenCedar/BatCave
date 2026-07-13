# Runtime Telemetry

**Updated**: 2026-07-10

BatCave Monitor is built around a Rust runtime store that collects local telemetry, shapes it into a stable snake_case JSON contract, and tells the UI exactly how trustworthy each metric is. The important part is not just "show numbers." The important part is "show what the machine actually said, and admit when it would not answer."

## Architecture

```text
Svelte cockpit
  -> Tauri commands
    -> Rust RuntimeState
      -> Rust RuntimeStore
        -> settings
        -> warm cache
        -> diagnostics
        -> runtime health
        -> query shaping
        -> byte-rate derivation
        -> local JSON persistence
        -> Rust collectors and helper modes
          -> Win32 process telemetry
          -> Win32 system telemetry
          -> PDH disk-rate telemetry
          -> ETW per-process network attribution
          -> Linux /proc and /sys telemetry
          -> Linux optional bpftrace/eBPF network attribution
          -> macOS sysinfo and libproc telemetry
          -> privileged in-process Windows collectors in installed releases
          -> Rust benchmark CLI
```

The UI talks to the runtime through Tauri commands:

- `get_snapshot`
- `refresh_now`
- `pause_runtime`
- `resume_runtime`
- `set_sample_interval`
- `set_process_query`
- `get_process_icon`

The UI should not own long-lived runtime truth. Settings, pause state, refresh cadence, process query shape, session-scoped admin state, warm cache, health, diagnostics, and persistence belong in Rust.

Every snapshot carries two clocks. `publication_seq` and `published_at_ms` change for every response, including query, pause, cadence, and error publications. `sample_seq` and nullable `sampled_at_ms` change only after successful collection. A Rust worker owns sampling; frontend reads are passive. The required `environment` object reports `platform`, `install_kind`, and the resolved `data_directory`.

This is the preview contract; the removed `seq`, `ts_ms`, and `focus_mode: active` aliases are not serialized. Process selection and rate identity use `pid` plus `start_time_ms` so PID reuse cannot inherit an earlier process's state. Empty kernel-pool-tag `driver_candidates` are always serialized as `[]`.

`process_contributors` is a compact, query-independent summary of the top CPU, memory, read/write I/O, and network process names from the complete sample. Its fields are `null` when no process reports activity for that resource. Search, focus, sorting, and limits still shape `processes` and `process_view_rows` for the workload table, but cannot rewrite the selected-resource overview. Process read/write I/O is intentionally not presented as physical-disk attribution.

## Current Coverage

Implemented runtime surfaces:

- Production snake_case JSON contracts for runtime snapshots, settings, process samples, system metrics, quality metadata, warnings, and health.
- Runtime settings, warm cache, keyed current warnings, diagnostics transition history, health budgets, query shaping, pause/resume, refresh, and admin-mode state in the Rust store.
- Stable process grouping for runtime process views, including aggregate one-core-equivalent CPU, memory, read/write I/O, network, and thread totals for group rows.
- Local JSON persistence with same-directory, per-writer atomic temporary files for settings and runtime state.
- Rust CLI mode for deterministic benchmark snapshots.

Windows native telemetry:

- Process identity, PID, parent PID, start-time identity, executable path, access state, one-core-equivalent CPU, kernel CPU, memory, private bytes, process read/write I/O totals, thread count, and handle count. Windows `GetProcessIoCounters` transfers include file, device, and other I/O, so BatCave does not call them physical-disk traffic.
- Physical memory, Windows commit totals, kernel paged/nonpaged pool, system cache, aggregate CPU deltas, logical CPU percentages, interface-level network totals/rates, and PDH physical-disk rates.
- ETW per-process network attribution over the Windows kernel TCP/IP provider.
- Installed Windows releases run privileged collectors in process. Development and portable builds keep standard access and mark permission-shaped gaps explicitly.

Linux native telemetry:

- Aggregate CPU, kernel CPU, logical CPU deltas, memory, swap, block-device I/O totals/rates, and interface network totals/rates.
- Process identity, PID, parent PID, start time, RSS/private memory, virtual memory, process I/O totals, thread count, and file descriptor count.
- Optional per-process network attribution through `bpftrace`/eBPF kretprobes on `sock_sendmsg` and `sock_recvmsg`. Install this optional tool with `bash scripts/install-linux-deps.sh --with-bpftrace`; base build dependencies do not include it.

macOS native telemetry:

- Sysinfo aggregate CPU, logical CPU, available/used memory, swap, and interface network counters.
- Local libproc enrichment for resident memory, physical footprint, virtual memory, process read/write totals, thread count, and file-descriptor count when process access allows.
- Physical-disk throughput is unavailable until the macOS collector has a trusted device-level source. Process read/write I/O is never substituted for system disk telemetry.
- Per-process network attribution and privileged helper mode are unavailable in this release. Rows remain visible with quality messages rather than fabricated zero rates.

Fallback behavior:

- `sysinfo` remains available when a native collector cannot read expected host files.
- Missing or delayed metrics use quality metadata instead of fabricated values.
- If native Windows process memory is blocked but `sysinfo` has a value for the same PID, BatCave reports that fallback as estimated memory instead of a native zero.
- If an executable path is unavailable, process grouping falls back to the process name before using a PID-specific key. Group rows should remain expandable and selectable even for OS/system processes with incomplete executable metadata.

## Memory Accounting

`memory_used_bytes` is physical memory used by the machine. It includes process working sets, kernel memory, cache, drivers, virtualization/WSL, and other OS-resident memory. It is not expected to equal the sum of process rows.

On Windows, `swap_used_bytes`, `swap_total_bytes`, and process `virtual_memory_bytes` are omitted because the available native counters represented commit charge, not those cross-platform concepts. Windows commit remains available as `memory_accounting.commit_used_bytes` and `commit_limit_bytes`. Linux reports real swap and process virtual-memory values when available. macOS reports available memory directly and uses libproc physical footprint as its private-memory presentation when accessible.

When available, `system.memory_accounting` adds the reconciliation view:

- `process_working_set_bytes` and `process_private_bytes`: diagnostic sums across process rows whose memory quality is reported. Shared pages can appear in more than one working set, so these sums are not reconciled against physical memory use.
- `denied_process_count` and `partial_process_count`: rows where BatCave did not get complete access.
- `commit_used_bytes` and `commit_limit_bytes`: Windows commit charge and limit from `GetPerformanceInfo`.
- `kernel_paged_pool_bytes`, `kernel_nonpaged_pool_bytes`, `kernel_total_bytes`, and `system_cache_bytes`: Windows OS memory buckets from `GetPerformanceInfo`.
- `kernel_pool_tags`: top Windows kernel pool tags from `NtQuerySystemInformation(SystemPoolTagInformation)`, split by paged/nonpaged pool and sorted by bytes.

Denied process rows remain visible because blocked access is itself useful telemetry. The UI must render unavailable process memory as blocked/unavailable, not as measured `0 B`.

Pool tags are allocation labels, not guaranteed driver ownership. BatCave may attach `driver_candidates` by scanning local installed `.sys` binaries for the tag bytes, but the UI must present those names as best-effort candidates. The scan runs outside the telemetry hot path, so `driver_candidates_pending` can be true on early snapshots. Unknown tags and missing candidates are expected after the scan completes.

## Quality States

BatCave treats metric quality as part of the product contract. If a number is incomplete, warming up, blocked, or platform-dependent, the snapshot should say so.

Common quality states:

- `native`: collected directly by the platform collector.
- `held`: temporarily held while the runtime waits for a second sample or rate derivation.
- `estimated`: supplied by a known fallback, such as Linux RSS when anonymous private memory is unavailable.
- `partial`: derived from a fallback or incomplete source.
- `unavailable`: unavailable on this platform or blocked by permissions.

Examples:

- First CPU samples may be held until native deltas are available.
- Disk rates may be partial if PDH or block-device counters are unavailable.
- Linux CPU, disk, and network retain independent last-good baselines. A failed read does not replace a baseline with zero, and the first recovered rate is derived only from valid counters.
- Windows process network attribution reports the ETW failure reason when the kernel logger cannot start.
- Linux per-process network attribution reports the eBPF prerequisite or capability failure when the host cannot attach probes.
- macOS physical-disk quality is `unavailable/runtime`; the limitation explains that process read/write I/O remains a separate resource.
- macOS libproc failures retain the sysinfo process row and identify physical footprint, thread, descriptor, read/write I/O, or network limitations independently.

## Process Groups And History

Process groups are UI rows backed by accumulated row telemetry, not placeholders. When a group is selected, the inspector should show the group's aggregate one-core-equivalent CPU, memory, read/write I/O, and network history from the same live values used in the process table.

Network readouts prefer a nonzero live attributed rate when the row has one, while the quality label still reports the source or limitation. This keeps estimated, fixture, partial, or unavailable quality honest without hiding useful accumulated traffic.

## Local Data

Runtime state, settings, warm cache, helper snapshots, and logs are local-only.

- Windows: `%LOCALAPPDATA%\BatCaveMonitor`
- Linux: `$XDG_DATA_HOME/BatCaveMonitor` or `~/.local/share/BatCaveMonitor`
- macOS: `~/Library/Application Support/BatCaveMonitor`

The runtime publishes the resolved path through `environment.data_directory` and identifies NSIS, AppImage, deb, DMG, or portable installation through the typed `environment.install_kind`. Installed Windows release binaries request administrator access in their embedded manifest; debug builds do not. Debian packages never invoke the AppImage updater, and macOS always reports admin mode unavailable.

Do not add outbound tracking, hosted collection, or remote logging. BatCave is a local instrument panel, not a service backend in a trench coat.

## Validation

Run Rust tests directly:

```powershell
cargo test --manifest-path src/BatCave.App/src-tauri/Cargo.toml
```

Run full Windows validation:

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/validate-tauri.ps1
```

Run full Linux or macOS validation:

```bash
bash scripts/validate-tauri.sh
```

The macOS bundle path requires both Rust targets and produces one universal DMG:

```bash
rustup target add aarch64-apple-darwin x86_64-apple-darwin
cd src/BatCave.App
npm run tauri:build:macos:universal
```

Run a headless benchmark:

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/run-benchmark.ps1 -BenchmarkHost core -Platform x64 -Ticks 120 -SleepMs 1000
```

Capture a reusable benchmark baseline:

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/capture-benchmark-baseline.ps1 -BenchmarkHost core -Platform x64
```

Run the strict regression gate:

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/run-benchmark-gate.ps1 -BenchmarkHost core -Platform x64 -BaselineArtifactPath artifacts\benchmarks\baseline-core-YYYYMMDD-HHMMSS.json
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/run-benchmark-gate.ps1 -BenchmarkHost core -Platform x64 -MaxP95Ms 10000
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/validate-tauri.ps1 -SkipBundle -BenchmarkGate -BenchmarkMaxP95Ms 10000
```

The benchmark builds the current release CLI and measures `RuntimeState::refresh_now` plus snapshot JSON serialization in an isolated temporary data directory. Output labels this as `evidence_scope: core_runtime_host_only`: it does not measure the Tauri shell, webview, renderer, or whole process tree. The default protocol is 30 warmup ticks followed by five 120-tick measured repeats at 1000 ms, with the median repeat p95 used for gating. Normal validation overrides that protocol with zero warmup, one two-tick repeat, and no sleep.

Protocol-v3 baselines record the source commit, release-binary SHA-256, binary-derived platform and architecture, machine class, workload, protocol parameters, repeat results, and selected median. Strict mode also requires advancing samples, app CPU at or below 25%, and RSS at or below 350 MiB.

## Continuous Integration

- Pull requests and `codex/**` pushes run Windows and Linux validation without bundles.
- Pull requests and `codex/**` pushes also check and lint both Apple architectures and build the universal target without packaging.
- Pull requests run dependency review and fail on new moderate-or-higher advisories.
- Pushes to `main` and manual bundle runs produce an offline-capable Windows NSIS installer, Linux deb/AppImage artifacts, and an ad-hoc-signed universal Mac `.app`/DMG retained for 14 days. The Windows artifact embeds the WebView2 Evergreen Standalone Installer, trading roughly 127 MB of package size for install-time network independence while retaining Evergreen servicing.
- Monday 09:00 UTC and manual advisory runs execute `npm audit --omit=dev --audit-level=moderate` and pinned `cargo-audit 0.22.2`. Rust vulnerabilities fail immediately; informational warnings must match the owned, expiring baseline documented in `docs/dependency-advisories.md`.

## Remaining Product Work

Distribution polish that remains outside the runtime contract:

- Add installer signing before broad external distribution.
- Promote the stable channel only after Authenticode signing proof is available.
- Supply the documented Apple Developer ID and notarization secrets for public Mac releases; ad-hoc main-branch bundles are test artifacts only.
- Expand screenshot-based validation from the native Tauri app when changing visible cockpit layout, metric-quality messaging, theme surfaces, or platform-specific collector states.

The runtime rule stays simple: collect locally, persist locally, report quality explicitly, and keep the UI fed with truth instead of theater.
