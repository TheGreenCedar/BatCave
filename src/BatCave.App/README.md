# BatCave Monitor

BatCave Monitor is the production Rust + Tauri + Svelte desktop shell. It keeps telemetry local, exposes a small snake_case JSON contract through Tauri commands, and renders a dense resource cockpit for CPU, logical cores, memory, disk, network, process triage, and runtime health.

The UI is designed for repeated use rather than a landing-page feel:

- Top metric charts act as navigation for larger CPU, memory, disk, and network detail views.
- Logical cores render as stretched time-series cards instead of static bars.
- Process triage supports pause/resume, refresh cadence, search, focus modes, sorting, and history reset.
- Selected processes include CPU and I/O trend context alongside totals.
- Themes are swappable from the top bar and persist in `localStorage` under `batcave.monitor.theme`.
- Browser dev mode falls back to deterministic fixture telemetry so layout work does not require launching the native shell.

## Commands

```powershell
npm install
npm run dev
npm run verify
npm run tauri:dev
npm run tauri:build
```

## Native Collector

The first collector uses `sysinfo` for CPU, memory, process, process disk totals, and network totals. Kernel CPU and true disk device throughput should move to a Windows-specific PDH/ETW module before installer packaging is considered final.

## Production Notes

- The app metadata now builds as `BatCave Monitor` with identifier `dev.batcave.monitor`.
- Tauri bundling targets an unsigned NSIS installer. Signing and update-channel work remain separate.
- Keep telemetry local. Do not add outbound analytics or remote logging.
