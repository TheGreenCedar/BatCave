# Runtime Telemetry

**Updated**: 2026-04-30

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
- `set_query`

The UI should not own long-lived runtime truth. Settings, pause state, refresh cadence, process query shape, admin-mode preference, warm cache, health, diagnostics, and persistence belong in Rust.

## Current Coverage

Implemented runtime surfaces:

- Production snake_case JSON contracts for runtime snapshots, settings, process samples, system metrics, quality metadata, warnings, and health.
- Runtime settings, warm cache, warnings, diagnostics, health budgets, query shaping, pause/resume, refresh, and admin-mode state in the Rust store.
- Local JSON persistence with atomic writes for settings and runtime state.
- Rust CLI modes for benchmarking and elevated-helper snapshots.

Windows native telemetry:

- Process identity, PID, parent PID, start-time identity, executable path, access state, CPU, kernel CPU, memory, private bytes, process I/O totals, thread count, and handle count.
- Physical memory, pagefile totals, aggregate CPU deltas, logical CPU percentages, interface-level network totals/rates, and PDH physical-disk rates.
- ETW per-process network attribution over the Windows kernel TCP/IP provider.
- Local elevated-helper snapshots that can carry attributed network rows when standard access lacks kernel trace rights.

Linux native telemetry:

- Aggregate CPU, kernel CPU, logical CPU deltas, memory, swap, block-device I/O totals/rates, and interface network totals/rates.
- Process identity, PID, parent PID, start time, RSS/private memory, virtual memory, process I/O totals, thread count, and file descriptor count.
- Optional per-process network attribution through `bpftrace`/eBPF kretprobes on `sock_sendmsg` and `sock_recvmsg`.

Fallback behavior:

- `sysinfo` remains available when a native collector cannot read expected host files.
- Missing or delayed metrics use quality metadata instead of fabricated values.

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

## Remaining Product Work

Distribution polish remains outside the runtime contract:

- Add installer signing before broad external distribution.
- Add automatic updater work when release channels exist.
- Expand screenshot-based validation when changing visible cockpit layout, metric-quality messaging, or platform-specific collector states.

The runtime rule stays simple: collect locally, persist locally, report quality explicitly, and keep the UI fed with truth instead of theater.
