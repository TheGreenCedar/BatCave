# BatCave Monitor app runbook

This directory contains the Rust, Tauri, and Svelte desktop app. This guide covers local development, validation, packaging, and telemetry troubleshooting.

Choose a run mode:

- Native desktop mode talks to the Rust runtime store through Tauri commands and uses platform collectors.
- Browser fixture mode runs the Svelte UI with deterministic sample data for layout work. It does not test the collectors.

Product screenshots and verification screenshots must come from the native Tauri desktop window, captured with Computer Use. Browser fixture screenshots are layout-only and should not be committed as product evidence.

## Prerequisites

- Node.js 24
- A current stable Rust toolchain
- Windows with Microsoft Edge WebView2 Evergreen Runtime. The NSIS bundle embeds Microsoft's Evergreen Standalone Installer and does not need network access during installation.
- Ubuntu/Debian plus the native Tauri packages
- macOS 12 or newer on Apple Silicon plus Xcode Command Line Tools; Intel Macs are unsupported

PostCSS's Nano ID dependency is pinned through a scoped override to 5.1.16 for its published security fixes. This relies on Node.js 24 loading synchronous ES modules through `require()`; older Node versions are unsupported. Remove the override when PostCSS resolves a patched compatible release. The dependency audit continues to fail on remaining advisories.

Install Linux native prerequisites from the repository root:

```bash
bash scripts/install-linux-deps.sh
```

Add `--with-bpftrace` only when you want to exercise optional per-process eBPF network attribution.

Install app dependencies from this directory:

```powershell
npm install
```

## Run modes

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
rustup target add aarch64-apple-darwin
bash scripts/run-dev.sh
bash scripts/run-dev.sh --web-only
```

From this app directory, the lower-level commands are:

```powershell
npm run dev
npm run tauri -- dev
```

`npm run dev` starts Vite at `http://127.0.0.1:1420`. `npm run tauri -- dev` launches the native shell and automatically merges the conventional `tauri.windows.conf.json`, `tauri.linux.conf.json`, or `tauri.macos.conf.json` overlay for the current host.

## Verify and build

Fast app checks from this directory:

```powershell
npm run verify
```

`npm run verify` runs the frontend behavior and contract tests, bridge smoke test, production build, type checks, lint, and formatting checks. The script list in [`package.json`](package.json) defines the full set.

Full repository validation from the repository root:

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/validate-tauri.ps1
```

Linux or macOS:

```bash
bash scripts/validate-tauri.sh
```

Fast validation loops after a successful full build:

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/validate-tauri.ps1 -SkipBundle
```

```bash
bash scripts/validate-tauri.sh --skip-bundle
```

Use `-SkipBundle`/`--skip-bundle` only after a successful full build and only when the edit does not affect packaging or generated assets. The development launchers start the existing Vite or Tauri development path directly; production frontend builds remain part of verification and packaging.

The validation scripts run frontend checks, Rust formatting, Rust check, Rust tests, and the Tauri bundle unless explicitly skipped.

Build platform bundles from this app directory:

```powershell
npm run tauri -- build
npm run tauri -- build --target aarch64-apple-darwin  # macOS Apple Silicon
```

Windows output goes to `src-tauri/target/release`, including the executable and unsigned NSIS installer. `tauri.windows.conf.json` selects `offlineInstaller`, which embeds Microsoft's WebView2 Evergreen Standalone Installer. This adds roughly 127 MB but lets users install offline while retaining Evergreen servicing. There is no online-bootstrapper package. Build hosts may need network access to populate Tauri's WebView2 download cache.

Linux `.deb` and AppImage output goes to `src-tauri/target/release/bundle`. Apple Silicon `.app` and DMG output goes to `src-tauri/target/aarch64-apple-darwin/release/bundle`. Local Mac builds are not notarized; main-branch CI artifacts use ad-hoc signatures.

## Runtime behavior

The monitoring commands use snake_case JSON:

- `get_snapshot`
- `get_workload_inspection`
- `acknowledge_workload_inspection`
- `refresh_now`
- `pause_runtime`
- `resume_runtime`
- `set_sample_interval`
- `set_process_query`
- `get_process_icons`

`publication_seq` and `published_at_ms` identify each runtime publication. `sample_seq` and nullable `sampled_at_ms` advance only after successful collection. Query, pause, cadence, and error publications therefore do not add chart samples.

`environment` reports the platform, current-process elevation, package type, and local data directory. `admin_mode.source` separately identifies the current process or installed service supplying privileged collection. Package detection uses the running executable and platform installation records, as described in [Runtime telemetry](../../docs/runtime-telemetry.md).

Workload identity includes process start time. If start time is unknown, the identity is valid for one telemetry sample (`sample_seq`) and changes at the next successful sample. Query and pause publications retain it. A later process using the same PID cannot inherit the earlier process's history.

The Rust runtime owns sampling, settings, queries, collector-service state, histories, cache, diagnostics, health, and byte-rate calculations. A bounded worker writes local state without blocking publication.

Local state stays under:

- Windows: `%LOCALAPPDATA%\BatCaveMonitor`
- Linux: `$XDG_DATA_HOME/BatCaveMonitor` or `~/.local/share/BatCaveMonitor`
- macOS: `~/Library/Application Support/BatCaveMonitor`

The UI stores theme preference in `localStorage` under `batcave.monitor.theme`.

See [Current-user state ownership and retention](../../docs/current-user-state.md) for the owned files, permission checks, diagnostic limits, and safe cleanup boundary.

## Triage UI contract

The workload queue groups processes only when executable or bundle identity and verified ancestry support the relationship. Matching names alone do not merge independent jobs. Group identity changes when membership changes.

Values update in place while ranking stays fixed when the pointer or keyboard focus is inside the Explore queue. The pending order applies when both leave the queue. Use `Ranking updated` to apply it while the order is held.

At 1280px and wider, the resource rail and inspector remain visible. At 900 to 1279px, the resource selector becomes horizontal and the inspector opens in a drawer. Below 900px, the queue uses metric cards. Only the active list is mounted; resizing preserves group expansion and keyboard focus.

Selecting a group shows its aggregate CPU, memory, read/write I/O, network, and thread totals with measurement quality and coverage. The inspector reads from a bounded runtime archive, independently of search and ranking. It retains timestamped history across selection changes and process exit, subject to the global memory budget.

System resources use the same detail pane. Settings, diagnostics, and compact detail use modal dialogs that close with Escape, contain keyboard focus, and restore focus to the opener.

Development-only accessibility fixtures cover bounded overview, process detail, group detail, settings, diagnostics, stale, degraded, and compact states. Install the local Chromium test runtime once with `npx playwright install chromium`, then run `npm run test:accessibility`. The test server uses a strict worktree-derived port; set `BATCAVE_ACCESSIBILITY_TEST_PORT` to an unused port when an explicit override is needed. These browser checks are automated semantic and layout evidence only; they do not replace packaged Windows keyboard or NVDA verification.

## Platform telemetry notes

### Windows

Windows native collectors read process identity, parent PID, start time, CPU, kernel CPU, memory, private bytes, process I/O, thread count, handle count, access state, physical memory, commit totals, kernel paged/nonpaged pool, top kernel pool tags with best-effort local driver candidates, system cache, interface network totals, and PDH physical-disk rates. Windows exposes commit through `memory_accounting` and omits cross-platform swap and process virtual-memory fields instead of relabeling commit charge.

Windows current-process status comes from `GetTokenInformation(TokenElevation)`. An elevated token is reported as an administrator token; a standard token stays standard; a failed query is unknown. The installed collector service has its own authenticated source and lifecycle state and never rewrites the desktop token. Missing, stopped, incompatible, or unauthorized service states keep standard monitoring available.

Kernel pool tag driver names are candidates, not proof of ownership. BatCave reads current pool-tag usage from Windows and scans local installed `.sys` binaries for matching tag bytes when the app needs a driver clue for a leaking pool bucket. That local driver scan is cached and runs outside the telemetry hot path, so candidate names may appear after the first pool-tag snapshot.

Windows per-process network attribution uses one ETW kernel logger owned by the installed collector service. The standard desktop fallback never acquires ETW. Service gaps, disconnects, protocol failures, or identity failures fail closed to standard access and remain visible through collector-service state and warnings.

Windows NSIS upgrades stage a fixed recovery controller beside the stable image and use a protected digest-bound journal plus an atomically created rollback executable to make service replacement resumable. A verified compatibility alias supports future installed uninstallers after this lookup behavior has shipped; the first migration from an older uninstaller remains installer-retry-only if it fails before `uninstall.exe` is replaced. The candidate is accepted only after the exact new stable process generation is running and has produced an initial telemetry sample; failed candidates restore the verified old image.

Same-version different-build and superseding-installer retries are explicit, dirty stopped services can be replaced without restarting the broken old image, uninstall retains transaction authority until SCM deletion succeeds, and a delete-pending timeout is reported as requiring a reboot. Exact installed upgrade, rollback, restart, and uninstall behavior remains a native Windows evidence requirement.

### Linux

Linux native collectors read aggregate CPU/kernel/logical CPU deltas, memory and swap, block-device I/O totals/rates, interface network totals/rates, process identity, parent PID, start time, RSS/private memory, virtual memory, process I/O totals, thread counts, and file descriptor counts.

Linux process-network attribution requires bpftrace 0.22.0 or newer and sufficient host permissions. It probes IPv4 and IPv6 socket payloads, validates complete counter windows, and marks rates unavailable on missing intervals, capacity overflow, reader failure, or shutdown failure.

`bash scripts/install-linux-deps.sh --with-bpftrace` installs an apt candidate only when it meets the version floor, or verifies a supported preinstalled build. It rejects Ubuntu 24.04's 0.20.x candidate. Monitoring remains available without the optional probe. [Runtime telemetry](../../docs/runtime-telemetry.md) documents epoch validation, capacity limits, and recovery.

`sysinfo` remains a fallback when native collectors cannot read the expected host files.

### macOS

macOS uses sysinfo for base measurements and libproc for physical footprint, read/write I/O totals, thread count, and file-descriptor count when access allows. Host-disk rates come from deduplicated IOKit physical block-driver counters. Disk-image paths are excluded, incomplete coverage makes the metric unavailable, and device-set changes require a new baseline. Process I/O does not substitute for host disk.

The sysinfo network aggregate includes `lo0`. Per-process TCP, UDP, and QUIC rates come from one XNU NStat control socket without root or a private entitlement. BatCave computes rates from absolute counters, includes final close updates, and marks traffic unavailable on unqualified revision-9 layouts. Privileged collection remains unavailable.

See [Platform capabilities](../../docs/platform-capabilities.md) for sources, measurement scope, permissions, and packages. Windows ARM64 and Linux ARM64 have no validated collectors or published packages. macOS supports Apple Silicon `arm64` only.

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

Benchmarks build the current release CLI, use an isolated temporary data directory, and issue one-shot refreshes through the owned sampling engine. Artifact format v4 measures collection and runtime shaping, immutable snapshot publication, live-command completion, and protocol-v4 encoding/JSON serialization separately. Persistence writes run on a separate worker. Output carries `evidence_scope: core_runtime_host_only` and `whole_app_measured: false`; it does not measure the Tauri shell, webview, renderer, or whole process tree. The default protocol runs 30 warmup commands and five 120-command measured repeats, selecting by `median_live_command_p95_ms`. Generated artifacts under `artifacts/benchmarks` record the commit, binary hash, platform, architecture, machine class, workload, protocol, component medians, and all repeats; revision fields append `-dirty` when the measured worktree is not clean. The CLI keeps `-SleepMs`/`--sleep-ms`, stored as `inter_command_delay_ms` in v4 output.

Strict mode is a configuration error without either a baseline or explicit p95 ceiling. A speed multiplier without a baseline is also a configuration error. Matching v4 baselines use `baseline median_live_command_p95_ms / candidate median_live_command_p95_ms` and require at least `0.90` by default. Older artifact formats are rejected instead of being compared across different measurement paths. Use `run-benchmark-gate` for release/local regression checks and its generated report artifact.

CI validates Windows, Linux, Apple Silicon macOS, and Linux package transport on pull requests. Only pushes to `main` save Rust caches; pull requests and manual runs restore them. The Linux package-transport job runs its tests in release mode and reuses the package dependency cache without caching workspace crates. Rust warning gates run before the longer test passes. Linux package transport runs alongside the main Linux validation job.

Pushes to `main` and manual bundle runs retain Windows NSIS, Linux deb/AppImage, and ad-hoc-signed Apple Silicon Mac artifacts for 90 days. The versioned release workflow validates the shared SemVer and produces checksums plus GitHub build provenance before an optional durable release; its Mac job additionally enforces Developer ID signing, notarization, stapling, the `arm64` slice, and DMG integrity. Moderate dependency changes fail pull requests; all npm dependencies and Rust advisories are audited every Monday and on demand.

## Production notes

- Product name: `BatCave Monitor`
- App identifier: `dev.batcave.monitor`
- Frontend: Svelte + Vite
- Desktop shell: Tauri 2
- Runtime: Rust
- Public runtime contract: snake_case JSON

Keep telemetry local. Do not add outbound tracking, remote collection, or hosted logging.
