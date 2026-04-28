# BatCave

BatCave is a local-first Windows resource monitor built on Rust, Tauri, and Svelte. The app keeps telemetry local, persists runtime state under `%LOCALAPPDATA%\BatCaveMonitor`, and renders a dense resource cockpit for CPU, logical cores, memory, disk, network, process triage, and runtime health.

## Quick Start

Run the desktop app:

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/run-dev.ps1
```

Run the web UI only, with fixture telemetry:

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/run-dev.ps1 -WebOnly
```

Validate the app and runtime:

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/validate-tauri.ps1
```

Run a headless runtime benchmark:

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/run-benchmark.ps1 -BenchmarkHost core -Platform x64 -Ticks 120 -SleepMs 1000
```

Capture a reusable benchmark baseline artifact:

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/capture-benchmark-baseline.ps1 -BenchmarkHost core -Platform x64
```

## Repository Layout

- `src/BatCave.App/`: production Tauri desktop app with Svelte UI, Rust runtime store, local JSON persistence, native Windows telemetry collectors, benchmark CLI, and NSIS packaging.
- `scripts/`: repeatable local workflows for app launch, Tauri validation, benchmark runs, and baseline capture.
- `artifacts/`: generated benchmark and screenshot output.

## App Commands

From `src/BatCave.App`:

```powershell
npm install
npm run dev
npm run verify
npm run tauri:dev
npm run tauri:build
```

`npm run tauri:build` emits the release executable and unsigned NSIS installer under `src/BatCave.App/src-tauri/target/release`.

## Data, Logs, and Privacy

BatCave is built around local-only telemetry. Do not add outbound analytics, telemetry uploads, or remote logging. Local app data should remain under `%LOCALAPPDATA%\BatCaveMonitor` unless a future migration explicitly changes that contract.

## Development Notes

- Keep desktop UI work in `src/BatCave.App/src`.
- Keep runtime state, persistence, collectors, helper modes, and benchmarks in `src/BatCave.App/src-tauri`.
- Keep generated output out of commits: `node_modules`, `dist`, Tauri `target`, app screenshots, and benchmark artifacts are disposable.
- Before opening a PR, run `scripts/validate-tauri.ps1`.
