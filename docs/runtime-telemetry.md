# Runtime Telemetry

**Updated**: 2026-06-28

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
          -> local elevated-helper snapshot mode
          -> Rust benchmark CLI
```

The UI talks to the runtime through Tauri commands:

- `get_snapshot`
- `refresh_now`
- `pause_runtime`
- `resume_runtime`
- `set_admin_mode`
- `set_process_query`

The UI should not own long-lived runtime truth. Settings, pause state, refresh cadence, process query shape, admin-mode preference, warm cache, health, diagnostics, and persistence belong in Rust.

## Current Coverage

Implemented runtime surfaces:

- Production snake_case JSON contracts for runtime snapshots, settings, process samples, system metrics, quality metadata, warnings, and health.
- Runtime settings, warm cache, warnings, diagnostics, health budgets, query shaping, pause/resume, refresh, and admin-mode state in the Rust store.
- Stable process grouping for runtime process views, including aggregate CPU, memory, disk, network, and thread totals for group rows.
- Local JSON persistence with atomic writes for settings and runtime state.
- Rust CLI modes for benchmarking and elevated-helper snapshots.

Windows native telemetry:

- Process identity, PID, parent PID, start-time identity, executable path, access state, CPU, kernel CPU, memory, private bytes, process I/O totals, thread count, and handle count.
- Physical memory, pagefile/commit totals, kernel paged/nonpaged pool, system cache, aggregate CPU deltas, logical CPU percentages, interface-level network totals/rates, and PDH physical-disk rates.
- ETW per-process network attribution over the Windows kernel TCP/IP provider.
- Local elevated-helper snapshots that can carry attributed network rows when standard access lacks kernel trace rights.

Linux native telemetry:

- Aggregate CPU, kernel CPU, logical CPU deltas, memory, swap, block-device I/O totals/rates, and interface network totals/rates.
- Process identity, PID, parent PID, start time, RSS/private memory, virtual memory, process I/O totals, thread count, and file descriptor count.
- Optional per-process network attribution through `bpftrace`/eBPF kretprobes on `sock_sendmsg` and `sock_recvmsg`.

Fallback behavior:

- `sysinfo` remains available when a native collector cannot read expected host files.
- Missing or delayed metrics use quality metadata instead of fabricated values.
- If native Windows process memory is blocked but `sysinfo` has a value for the same PID, BatCave reports that fallback as estimated memory instead of a native zero.
- If an executable path is unavailable, process grouping falls back to the process name before using a PID-specific key. Group rows should remain expandable and selectable even for OS/system processes with incomplete executable metadata.

## Memory Accounting

`memory_used_bytes` is physical memory used by the machine. It includes process working sets, kernel memory, cache, drivers, virtualization/WSL, and other OS-resident memory. It is not expected to equal the sum of process rows.

When available, `system.memory_accounting` adds the reconciliation view:

- `process_working_set_bytes` and `process_private_bytes`: totals across process rows whose memory quality is reported.
- `denied_process_count` and `partial_process_count`: rows where BatCave did not get complete access.
- `unattributed_bytes`: physical used memory not covered by reported process working sets. Treat it as an operator clue, not a perfect RAMMap replacement.
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
- `partial`: derived from a fallback or incomplete source.
- `unavailable`: unavailable on this platform or blocked by permissions.

Examples:

- First CPU samples may be held until native deltas are available.
- Disk rates may be partial if PDH or block-device counters are unavailable.
- Windows process network attribution reports the ETW failure reason when the kernel logger cannot start.
- Linux per-process network attribution reports the eBPF prerequisite or capability failure when the host cannot attach probes.

## Process Groups And History

Process groups are UI rows backed by accumulated row telemetry, not placeholders. When a group is selected, the inspector should show the group's aggregate CPU, memory, disk I/O, and network history from the same live values used in the process table.

Network readouts prefer a nonzero live attributed rate when the row has one, while the quality label still reports the source or limitation. This keeps estimated, fixture, partial, or unavailable quality honest without hiding useful accumulated traffic.

## Local Data

Runtime state, settings, warm cache, helper snapshots, and logs are local-only.

- Windows: `%LOCALAPPDATA%\BatCaveMonitor`
- Linux: `$XDG_DATA_HOME/BatCaveMonitor` or `~/.local/share/BatCaveMonitor`

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

Run full Linux validation:

```bash
bash scripts/validate-tauri.sh
```

Run a headless benchmark:

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/run-benchmark.ps1 -BenchmarkHost core -Platform x64 -Ticks 120 -SleepMs 1000
```

Capture a reusable benchmark baseline:

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/capture-benchmark-baseline.ps1 -BenchmarkHost core -Platform x64
```

In strict benchmark mode, the benchmark exits nonzero when `--max-p95-ms` or `--min-speedup-multiplier` gates fail. Use `capture-benchmark-baseline` to create a matching baseline summary before comparing runs.

## Remaining Product Work

Distribution polish remains outside the runtime contract:

- Add installer signing before broad external distribution.
- Add automatic updater work when release channels exist.
- Expand screenshot-based validation from the native Tauri app when changing visible cockpit layout, metric-quality messaging, theme surfaces, or platform-specific collector states.

The runtime rule stays simple: collect locally, persist locally, report quality explicitly, and keep the UI fed with truth instead of theater.
