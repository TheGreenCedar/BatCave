# BatCave

BatCave is a local-first Windows and Linux resource monitor built on Rust, Tauri, and Svelte. The app keeps telemetry local, persists runtime state under `%LOCALAPPDATA%\BatCaveMonitor` on Windows or `$XDG_DATA_HOME/BatCaveMonitor` on Linux, and renders a dense resource cockpit for CPU, logical cores, memory, disk, network, process triage, and runtime health.

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

On Ubuntu/Debian, use Node.js 24 and a current stable Rust toolchain, install the native Tauri prerequisites once, then use the Bash workflows:

```bash
bash scripts/install-linux-deps.sh
bash scripts/run-dev.sh
bash scripts/run-dev.sh --web-only
bash scripts/validate-tauri.sh
```

Linux per-process network attribution is optional. The prerequisite script installs `bpftrace`, but the eBPF monitor only starts when the app has the kernel permissions/capabilities required to attach probes; otherwise the app keeps running with `/proc`/`/sys` metrics and marks per-process network attribution unavailable.

Run a headless runtime benchmark:

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/run-benchmark.ps1 -BenchmarkHost core -Platform x64 -Ticks 120 -SleepMs 1000
```

Capture a reusable benchmark baseline artifact:

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/capture-benchmark-baseline.ps1 -BenchmarkHost core -Platform x64
```

## Repository Layout

- `src/BatCave.App/`: production Tauri desktop app with Svelte UI, Rust runtime store, local JSON persistence, native Windows and Linux telemetry collectors, benchmark CLI, and platform packaging.
- `scripts/`: repeatable local workflows for app launch, Tauri validation, benchmark runs, baseline capture, and Linux prerequisite setup.
- `artifacts/`: generated benchmark and screenshot output.

## App Commands

From `src/BatCave.App`:

```powershell
npm install
npm run dev
npm run verify
npm run tauri:dev:windows
npm run tauri:build:windows
```

`npm run tauri:build:windows` emits the release executable and unsigned NSIS installer under `src/BatCave.App/src-tauri/target/release`.
On Linux, use `npm run tauri:dev:linux` and `npm run tauri:build:linux`; the Linux build emits `.deb` and AppImage bundles under `src/BatCave.App/src-tauri/target/release/bundle`.

## Data, Logs, and Privacy

BatCave is built around local-only telemetry. Do not add outbound analytics, telemetry uploads, or remote logging. Local app data should remain under `%LOCALAPPDATA%\BatCaveMonitor` on Windows and `$XDG_DATA_HOME/BatCaveMonitor` or `~/.local/share/BatCaveMonitor` on Linux unless a future migration explicitly changes that contract.

## Development Notes

- Keep desktop UI work in `src/BatCave.App/src`.
- Keep runtime state, persistence, collectors, helper modes, and benchmarks in `src/BatCave.App/src-tauri`.
- Keep generated output out of commits: `node_modules`, `dist`, Tauri `target`, app screenshots, and benchmark artifacts are disposable.
- Before opening a PR, run `scripts/validate-tauri.ps1` on Windows or `bash scripts/validate-tauri.sh` on Linux.
