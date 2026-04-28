# BatCave Monitor

BatCave Monitor is the production Rust + Tauri + Svelte desktop shell. It keeps telemetry local, exposes a small snake_case JSON contract through Tauri commands, persists runtime state under `%LOCALAPPDATA%\BatCaveMonitor` on Windows or `$XDG_DATA_HOME/BatCaveMonitor` on Linux, and renders a dense resource cockpit for CPU, logical cores, memory, disk, network, process triage, and runtime health.

The UI is designed for repeated use rather than a landing-page feel:

- Top metric charts act as navigation for larger CPU, memory, disk, and network detail views.
- Logical cores render as stretched time-series cards instead of static bars.
- Process triage supports pause/resume, refresh cadence, search, focus modes, sorting, and history reset.
- Selected processes include CPU and I/O trend context alongside totals.
- Theme preference is swappable from the top bar, defaults to the system color scheme, updates live while System is selected, and persists explicit choices in `localStorage` under `batcave.monitor.theme`.
- Browser dev mode falls back to deterministic fixture telemetry so layout work does not require launching the native shell.
- Native mode uses the Rust runtime store for pause/resume, refresh, process query shaping, admin-mode preference, settings persistence, warm cache, and health budgets.

## Commands

```powershell
npm install
npm run dev
npm run verify
npm run tauri:dev
npm run tauri:build
```

Ubuntu/Debian hosts also need Node.js 24, a current stable Rust toolchain, and the native WebKitGTK/GTK build prerequisites:

```bash
bash ../../scripts/install-linux-deps.sh
npm install
npm run verify
npm run tauri:build
```

## Native Collector

The runtime prefers Rust Win32 collectors on Windows for process enumeration, parent PID, process start time, memory counters, process I/O totals, thread counts, handle counts, access state, physical memory, pagefile totals, aggregate CPU deltas, PDH physical-disk rates, interface-level network totals, and ETW per-process network attribution. On Linux it uses native `/proc` and `/sys` collectors for aggregate CPU/kernel/logical CPU deltas, memory and swap, block-device I/O totals/rates, interface network totals/rates, process identity, parent PID, start time, RSS/private memory, virtual memory, process I/O totals, thread counts, and file descriptor counts. Linux per-process network attribution is optional and uses `bpftrace`/eBPF kretprobes on `sock_sendmsg` and `sock_recvmsg` when the app is launched with sufficient privileges or capabilities; otherwise rows report the eBPF prerequisite/capability failure and continue without per-process network rates. `sysinfo` remains only as a fallback when a native collector cannot read the expected host files. If standard access cannot start the Windows kernel logger, process network quality reports the ETW failure reason; admin-mode helper snapshots use the same Rust collector and can carry attributed network rows when elevated.

## Production Notes

- The app metadata now builds as `BatCave Monitor` with identifier `dev.batcave.monitor`.
- Tauri bundling targets an unsigned NSIS installer on Windows and `.deb` plus AppImage bundles on Linux. Signing and update-channel work remain separate.
- Keep telemetry local. Do not add outbound analytics or remote logging.
