# BatCave Monitor App Runbook

This directory contains the production Rust + Tauri + Svelte desktop app. Use this runbook when you want to try the app locally, verify a build, work on the UI, or understand why a metric is present, delayed, or unavailable.

BatCave has two useful run modes:

- Native desktop mode talks to the Rust runtime store through Tauri commands and uses platform collectors.
- Browser fixture mode runs only the Svelte UI with deterministic local fixture telemetry, which is perfect for layout work and useless as collector proof. Beautiful, limited, honest. The holy trinity.

Product screenshots and verification screenshots must come from the native Tauri desktop window, captured with Computer Use. Browser fixture screenshots are layout-only and should not be committed as product evidence.

## Prerequisites

- Node.js 24
- A current stable Rust toolchain
- Windows for `tauri:dev:windows` and `tauri:build:windows`
- Microsoft Edge WebView2 Evergreen Runtime for Windows installs. The NSIS bundle embeds Microsoft's Evergreen Standalone Installer and does not need network access during installation.
- Ubuntu/Debian plus the native Tauri packages for `tauri:dev:linux` and `tauri:build:linux`
- macOS 12 or newer plus Xcode Command Line Tools for `tauri:dev:macos`; universal builds require both Apple Rust targets

Install Linux native prerequisites from the repository root:

```bash
bash scripts/install-linux-deps.sh
```

Add `--with-bpftrace` only when you want to exercise optional per-process eBPF network attribution.

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

The same shell entry points detect macOS automatically:

```bash
rustup target add aarch64-apple-darwin x86_64-apple-darwin
bash scripts/run-dev.sh
bash scripts/run-dev.sh --web-only
```

From this app directory, the lower-level commands are:

```powershell
npm run dev
npm run tauri:dev:windows
npm run tauri:dev:linux
npm run tauri:dev:macos
```

`npm run dev` starts Vite at `http://127.0.0.1:1420`. The platform-specific Tauri commands launch the native shell around that UI.

## Verify And Build

Fast app checks from this directory:

```powershell
npm run verify
```

`npm run verify` runs:

- `npm run test:process-order`
- `npm run test:runtime-contract`
- `npm run smoke:bridge`
- `npm run build`
- `npm run typecheck`
- `npm run lint`
- `npm run format:check`

Full repository validation from the repository root:

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/validate-tauri.ps1
```

Linux or macOS:

```bash
bash scripts/validate-tauri.sh
```

Fast recovery loops after a successful full build:

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/validate-tauri.ps1 -SkipBundle
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/run-dev.ps1 -NoBuild
```

```bash
bash scripts/validate-tauri.sh --skip-bundle
bash scripts/run-dev.sh --no-build
```

Use `-SkipBundle`/`--skip-bundle` and `-NoBuild`/`--no-build` only after a successful full build and only when the edit does not affect packaging or generated assets.

The validation scripts run frontend checks, Rust formatting, Rust check, Rust tests, and the Tauri bundle unless explicitly skipped.

Build platform bundles from this app directory:

```powershell
npm run tauri:build:windows
npm run tauri:build:linux
npm run tauri:build:macos:universal
```

Windows build output lands under `src-tauri/target/release`, including the release executable and unsigned NSIS installer. `tauri.windows.conf.json` selects `offlineInstaller`, so the NSIS artifact embeds Microsoft's WebView2 Evergreen Standalone Installer. The trade-off is roughly 127 MB of additional package size in exchange for installation without network access and continued Evergreen runtime servicing. There is no online-bootstrapper artifact. Build hosts can still need network access to populate Tauri's WebView2 download cache. Linux bundle output lands under `src-tauri/target/release/bundle`, including `.deb` and AppImage artifacts. The Mac universal `.app` and DMG land under `src-tauri/target/universal-apple-darwin/release/bundle`; local builds are not notarized and main-branch CI artifacts are ad-hoc signed.

## Runtime Behavior

The native app exposes a small snake_case JSON contract through Tauri commands:

- `get_snapshot`
- `refresh_now`
- `pause_runtime`
- `resume_runtime`
- `set_sample_interval`
- `set_process_query`
- `set_admin_mode`
- `get_process_icons`

`publication_seq` and `published_at_ms` identify every runtime publication. `sample_seq` and nullable `sampled_at_ms` advance only after successful telemetry collection, so query, pause, cadence, and error publications cannot create fake chart samples. `environment` reports `platform`, current-process `process_elevation`, runtime-derived `install_kind`, and the resolved local data directory. `admin_mode.source` separately reports whether privileged collection comes from the current process or the local elevated helper. Windows distinguishes an NSIS install whose registry location matches the running executable from portable and development binaries. Linux checks AppImage runtime state or local Debian package ownership. macOS distinguishes development, app-bundle, and standalone portable runtimes without claiming the app's original download container. Process identity is the PID plus `start_time_ms`, not the reusable PID alone.

The Rust runtime store owns settings, pause/resume state, refresh cadence, query shaping, admin-mode preference, warm cache, diagnostics, health budgets, byte-rate derivation, and local JSON persistence.

Local state stays under:

- Windows: `%LOCALAPPDATA%\BatCaveMonitor`
- Linux: `$XDG_DATA_HOME/BatCaveMonitor` or `~/.local/share/BatCaveMonitor`
- macOS: `~/Library/Application Support/BatCaveMonitor`

The UI stores theme preference in `localStorage` under `batcave.monitor.theme`.

## Triage UI Contract

The attention queue groups rows by executable identity when available, then process name, then PID as a last resort. Group rows always have a stable key so they can be expanded, collapsed, selected, and inspected.

Live values may update in place, but ranking order is held while the pointer or keyboard focus is inside the queue, a group is expanded, or a workload is selected. A newer order is applied only through the visible `Ranking updated` control. At 1280px and wider the resource rail and inspector remain visible; from 900–1279px the resource selector becomes horizontal and the inspector becomes a drawer; below 900px the workload queue becomes a compact list of metric cards.

Selecting a group shows aggregate CPU, memory, read/write I/O, network, and thread totals from the grouped rows. The contextual detail pane uses those same aggregate live values, including network rates, instead of falling back to an unavailable state just because the selected row is a group. System resource selection uses the same pane. Settings, diagnostics, and compact detail use native modal dialogs, close with Escape, contain keyboard focus, and restore focus to their opener.

## Platform Telemetry Notes

Windows native collectors read process identity, parent PID, start time, CPU, kernel CPU, memory, private bytes, process I/O, thread count, handle count, access state, physical memory, commit totals, kernel paged/nonpaged pool, top kernel pool tags with best-effort local driver candidates, system cache, interface network totals, and PDH physical-disk rates. Windows exposes commit through `memory_accounting` and omits cross-platform swap and process virtual-memory fields instead of relabeling commit charge.

Windows current-process status comes from `GetTokenInformation(TokenElevation)`. An elevated token is reported as an administrator token; a standard token stays standard; a failed query is unknown. That truth does not change when the local elevated helper becomes active. The helper has its own source and lifecycle state, so a cancelled or denied UAC request leaves standard monitoring running and exposes a retry action. Linux and macOS report this Windows-specific helper capability as unavailable.

Kernel pool tag driver names are candidates, not proof of ownership. BatCave reads current pool-tag usage from Windows and scans local installed `.sys` binaries for matching tag bytes when the app needs a driver clue for a leaking pool bucket. That local driver scan is cached and runs outside the telemetry hot path, so candidate names may appear after the first pool-tag snapshot.

Windows per-process network attribution uses one ETW kernel logger. The main runtime keeps ownership when healthy and merges values into helper rows on an exact PID/start-time match; otherwise the elevated helper owns ETW until it stops, then the main runtime retries ownership. Elevated helper arguments are restricted to the per-run local pipe and stop path. Gaps under three seconds hold the last helper rows, gaps through fifteen seconds publish current standard rows as `recovering`, and longer gaps, disconnects, protocol failures, or helper exits fail closed to standard access. Helper collector errors are framed and retried without ending the elevated session.

Linux native collectors read aggregate CPU/kernel/logical CPU deltas, memory and swap, block-device I/O totals/rates, interface network totals/rates, process identity, parent PID, start time, RSS/private memory, virtual memory, process I/O totals, thread counts, and file descriptor counts.

Linux per-process network attribution is optional. It uses `bpftrace`/eBPF kretprobes on `sock_sendmsg` and `sock_recvmsg` when the app has sufficient host permissions or capabilities. Install it with `bash scripts/install-linux-deps.sh --with-bpftrace`. Without those permissions or the opt-in package, BatCave keeps running and marks per-process network rates unavailable.

`sysinfo` remains a fallback when native collectors cannot read the expected host files.

macOS collectors use sysinfo as a resilient base and enrich local process rows with libproc details such as physical footprint, read/write I/O totals, thread count, and file-descriptor count when access allows. Physical-disk throughput stays explicitly unavailable because the current macOS collector has no trusted device-level source; process I/O is never substituted for it. Per-process network attribution and privileged helper mode are unavailable on macOS in this release.

## Benchmarking

From the repository root on Windows:

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/run-benchmark.ps1 -BenchmarkHost core -Platform x64 -Ticks 120 -SleepMs 1000
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/capture-benchmark-baseline.ps1 -BenchmarkHost core -Platform x64
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/run-benchmark-gate.ps1 -BenchmarkHost core -Platform x64 -BaselineArtifactPath artifacts\benchmarks\baseline-core-YYYYMMDD-HHMMSS.json
```

Linux or macOS:

```bash
bash scripts/run-benchmark.sh --benchmark-host core --ticks 120 --sleep-ms 1000
bash scripts/capture-benchmark-baseline.sh --benchmark-host core
bash scripts/run-benchmark-gate.sh --benchmark-host core --baseline-artifact artifacts/benchmarks/baseline-core-YYYYMMDD-HHMMSS.json
```

Benchmarks build the current release CLI, use an isolated temporary data directory, and time the core runtime host's refresh plus snapshot JSON serialization. Output carries `evidence_scope: core_runtime_host_only`; it does not measure the Tauri shell, webview, renderer, or whole process tree. The default protocol runs 30 warmup ticks and five 120-tick measured repeats, selecting the median repeat p95. Generated artifacts under `artifacts/benchmarks` record the commit, binary hash, platform, architecture, machine class, workload, protocol, and all repeats; revision fields append `-dirty` when the measured worktree is not clean.

Strict mode is a configuration error without either a baseline or explicit p95 ceiling. A speed multiplier without a baseline is also a configuration error. Matching baselines use `baseline_p95 / candidate_p95` and require at least `0.90` by default. Use `run-benchmark-gate` for release/local regression checks and its generated report artifact.

CI validates Windows, Linux, and both macOS architectures on pull requests and `codex/**` pushes. Pushes to `main` and manual bundle runs retain Windows NSIS, Linux deb/AppImage, and ad-hoc-signed universal Mac artifacts for 14 days. The versioned release workflow validates the shared SemVer and produces checksums plus GitHub build provenance before an optional durable release; its Mac job additionally enforces Developer ID signing, notarization, stapling, universal slices, and DMG integrity. Moderate dependency changes fail pull requests; production npm and Rust advisories are audited every Monday and on demand.

## Production Notes

- Product name: `BatCave Monitor`
- App identifier: `dev.batcave.monitor`
- Frontend: Svelte + Vite
- Desktop shell: Tauri 2
- Runtime: Rust
- Public runtime contract: snake_case JSON

Keep telemetry local. Do not introduce outbound tracking, remote collection, or hosted logging. BatCave should feel like opening a clean instrument panel on your own machine, not inviting a stranger to rummage through the drawers.
