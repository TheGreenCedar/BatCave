# BatCave Monitor

BatCave Monitor is the production Rust + Tauri + Svelte desktop shell. It keeps telemetry local, exposes a small snake_case JSON contract through Tauri commands, persists runtime state under `%LOCALAPPDATA%\BatCaveMonitor`, and renders a dense resource cockpit for CPU, logical cores, memory, disk, network, process triage, and runtime health.

The UI is designed for repeated use rather than a landing-page feel:

- Top metric charts act as navigation for larger CPU, memory, disk, and network detail views.
- Logical cores render as stretched time-series cards instead of static bars.
- Process triage supports pause/resume, refresh cadence, search, focus modes, sorting, and history reset.
- Selected processes include CPU and I/O trend context alongside totals.
- Themes are swappable from the top bar and persist in `localStorage` under `batcave.monitor.theme`.
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

## Native Collector

The runtime prefers Rust Win32 collectors for process enumeration, parent PID, process start time, memory counters, process I/O totals, thread counts, handle counts, access state, physical memory, pagefile totals, aggregate CPU deltas, PDH physical-disk rates, interface-level network totals, and ETW per-process network attribution. `sysinfo` remains a Rust-only fallback and CPU enrichment path. If standard access cannot start the Windows kernel logger, process network quality reports the ETW failure reason; admin-mode helper snapshots use the same Rust collector and can carry attributed network rows when elevated.

## Production Notes

- The app metadata now builds as `BatCave Monitor` with identifier `dev.batcave.monitor`.
- Tauri bundling targets an unsigned NSIS installer. Signing and update-channel work remain separate.
- Keep telemetry local. Do not add outbound analytics or remote logging.
