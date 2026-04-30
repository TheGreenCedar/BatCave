# BatCave Monitor App Runbook

This directory contains the production Rust + Tauri + Svelte desktop app. Use this runbook when you want to try the app locally, verify a build, work on the UI, or understand why a metric is present, delayed, or unavailable.

BatCave has two useful run modes:

- Native desktop mode talks to the Rust runtime store through Tauri commands and uses platform collectors.
- Browser fixture mode runs only the Svelte UI with deterministic local fixture telemetry, which is perfect for layout work and useless as collector proof. Beautiful, limited, honest. The holy trinity.

## Prerequisites

- Node.js 24
- A current stable Rust toolchain
- Windows for `tauri:dev:windows` and `tauri:build:windows`
- Ubuntu/Debian plus the native Tauri packages for `tauri:dev:linux` and `tauri:build:linux`

Install Linux native prerequisites from the repository root:

```bash
bash scripts/install-linux-deps.sh
```

Install app dependencies from this directory:

```powershell
npm install
```

## Run Modes

From the repository root, launch the Windows desktop app:

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/run-dev.ps1
```

Launch only the Windows/browser fixture UI:

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/run-dev.ps1 -WebOnly
```

From the repository root on Linux:

```bash
bash scripts/run-dev.sh
bash scripts/run-dev.sh --web-only
```

From this app directory, the lower-level commands are:

```powershell
npm run dev
npm run tauri:dev:windows
npm run tauri:dev:linux
```

`npm run dev` starts Vite at `http://127.0.0.1:1420`. The platform-specific Tauri commands launch the native shell around that UI.

## Verify And Build

Fast app checks from this directory:

```powershell
npm run verify
```

`npm run verify` runs:

- `npm run build`
- `npm run typecheck`
- `npm run lint`
- `npm run format:check`

Full repository validation from the repository root:

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/validate-tauri.ps1
```

Linux:

```bash
bash scripts/validate-tauri.sh
```

The validation scripts run frontend checks, Rust formatting, Rust check, Rust tests, and the Tauri bundle unless explicitly skipped.

Build platform bundles from this app directory:

```powershell
npm run tauri:build:windows
npm run tauri:build:linux
```

Windows build output lands under `src-tauri/target/release`, including the release executable and unsigned NSIS installer. Linux bundle output lands under `src-tauri/target/release/bundle`, including `.deb` and AppImage artifacts.

## Runtime Behavior

The native app exposes a small snake_case JSON contract through Tauri commands:

- `get_snapshot`
- `refresh_now`
- `pause_runtime`
- `resume_runtime`
- `set_admin_mode`
- `set_query`

The Rust runtime store owns settings, pause/resume state, refresh cadence, query shaping, admin-mode preference, warm cache, diagnostics, health budgets, byte-rate derivation, and local JSON persistence.

Local state stays under:

- Windows: `%LOCALAPPDATA%\BatCaveMonitor`
- Linux: `$XDG_DATA_HOME/BatCaveMonitor` or `~/.local/share/BatCaveMonitor`

The UI stores theme preference in `localStorage` under `batcave.monitor.theme`.

## Platform Telemetry Notes

Windows native collectors read process identity, parent PID, start time, CPU, kernel CPU, memory, private bytes, process I/O, thread count, handle count, access state, physical memory, pagefile totals, interface network totals, and PDH physical-disk rates.

Windows per-process network attribution uses ETW over the kernel TCP/IP provider. If standard access cannot start the kernel logger, the process network quality reports the ETW failure reason. Admin mode can launch a local elevated helper and reuse the same Rust collector for richer snapshots. If elevation is denied or unavailable, standard access remains the fallback path.

Linux native collectors read aggregate CPU/kernel/logical CPU deltas, memory and swap, block-device I/O totals/rates, interface network totals/rates, process identity, parent PID, start time, RSS/private memory, virtual memory, process I/O totals, thread counts, and file descriptor counts.

Linux per-process network attribution is optional. It uses `bpftrace`/eBPF kretprobes on `sock_sendmsg` and `sock_recvmsg` when the app has sufficient host permissions or capabilities. Without those permissions, BatCave keeps running and marks per-process network rates unavailable.

`sysinfo` remains a fallback when native collectors cannot read the expected host files.

## Benchmarking

From the repository root on Windows:

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/run-benchmark.ps1 -BenchmarkHost core -Platform x64 -Ticks 120 -SleepMs 1000
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/capture-benchmark-baseline.ps1 -BenchmarkHost core -Platform x64
```

Linux:

```bash
bash scripts/run-benchmark.sh --benchmark-host core --ticks 120 --sleep-ms 1000
bash scripts/capture-benchmark-baseline.sh --benchmark-host core
```

Benchmarks run through the Rust runtime host and emit generated artifacts under `artifacts/benchmarks`.

## Production Notes

- Product name: `BatCave Monitor`
- App identifier: `dev.batcave.monitor`
- Frontend: Svelte + Vite
- Desktop shell: Tauri 2
- Runtime: Rust
- Public runtime contract: snake_case JSON

Keep telemetry local. Do not introduce outbound tracking, remote collection, or hosted logging. BatCave should feel like opening a clean instrument panel on your own machine, not inviting a stranger to rummage through the drawers.
