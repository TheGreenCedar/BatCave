# BatCave Monitor

BatCave Monitor is a local-first resource cockpit for Windows, Linux, and macOS. It shows the machine underneath the machine: machine-total CPU, memory, disk and network movement, process triage, runtime health, and the little permission-shaped holes where the operating system says "not today."

This is a public preview. It is useful now, honest about what it cannot see, and intentionally boring about privacy: BatCave reads local telemetry and keeps it local.

### Light Theme

![BatCave Monitor attention-first resource overview in the Daylight theme](docs/images/batcave-monitor-remediation-overview-daylight.png)

![BatCave Monitor selected workload detail in the Daylight theme](docs/images/batcave-monitor-remediation-workload-daylight.png)

![BatCave Monitor plain-language diagnostics drawer in the Daylight theme](docs/images/batcave-monitor-remediation-diagnostics-daylight.png)

![BatCave Monitor settings and privileged-access drawer in the Daylight theme](docs/images/batcave-monitor-remediation-settings-daylight.png)

### Dark Theme

![BatCave Monitor attention-first resource overview in the dark Cave theme](docs/images/batcave-monitor-remediation-overview-cave.png)

![BatCave Monitor compact card layout in the dark Cave theme](docs/images/batcave-monitor-remediation-narrow-cave.png)

![BatCave Monitor dismissible compact detail drawer in the dark Cave theme](docs/images/batcave-monitor-remediation-narrow-detail-cave.png)

Screenshots show the native Tauri app with live Windows telemetry. Browser fixture screenshots are layout-only and should not be used as product proof.

## What It Shows

- A selected-resource summary that keeps the headline, value, chart, time window, source quality, and compatible process attribution on one semantic.
- A stable grouped workload ranking that keeps row identity and scroll position fixed while the user inspects live values.
- A contextual detail pane with Overview, Resources, and Technical views for the selected workload or system resource.
- Plain-language telemetry diagnostics that explain impact, next steps, and raw collector detail on demand.
- Focused drawers for appearance, sampling, privileged access, and local data controls.
- A compact card layout at narrow window widths, with the same diagnosis path as the desktop table.

BatCave does not pretend. If ETW, eBPF, `/proc`, `/sys`, libproc, PDH, or process permissions are unavailable, the app keeps running and marks the affected metric honestly instead of painting fake numbers over the crack.

## Preview Status

BatCave is ready for source-based testing and local preview builds.

- Windows, Linux, and macOS native telemetry collectors are implemented.
- The Tauri app can run as a native desktop shell or as a browser-only fixture UI for layout testing.
- Windows bundles currently produce an unsigned executable and NSIS installer.
- Linux builds produce `.deb` and AppImage bundles.
- macOS builds produce one universal Apple Silicon/Intel DMG with a macOS 12 minimum.
- The signed updater is implemented; Windows Authenticode release promotion remains gated on SignPath approval.

## Try It

Install prerequisites first:

- Node.js 24
- A current stable Rust toolchain
- On Windows, Microsoft Edge WebView2 Evergreen Runtime. The NSIS bundle embeds Microsoft's Evergreen Standalone Installer, so installation works without network access when WebView2 is missing.
- On Linux, the WebKitGTK/GTK/Tauri native packages installed by `scripts/install-linux-deps.sh`
- On macOS, Xcode Command Line Tools. Universal bundles also require `rustup target add aarch64-apple-darwin x86_64-apple-darwin`.

From the repository root on Windows:

```powershell
cd src\BatCave.App
npm install
cd ..\..
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/run-dev.ps1
```

Run only the browser UI with deterministic fixture telemetry:

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/run-dev.ps1 -WebOnly
```

From the repository root on Ubuntu/Debian:

```bash
bash scripts/install-linux-deps.sh
cd src/BatCave.App
npm install
cd ../..
bash scripts/run-dev.sh
```

Run the Linux browser-only fixture UI:

```bash
bash scripts/run-dev.sh --web-only
```

On macOS, the shared shell launcher detects Darwin and starts the native Mac build:

```bash
cd src/BatCave.App
npm install
cd ../..
bash scripts/run-dev.sh
```

Use `bash scripts/run-dev.sh --web-only` only for deterministic layout work.

## Validate And Build

Run the full Windows validation workflow:

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/validate-tauri.ps1
```

Run the Linux or macOS equivalent (Darwin builds the universal DMG unless `--skip-bundle` is supplied):

```bash
bash scripts/validate-tauri.sh
```

For faster app-level checks from `src/BatCave.App`:

```powershell
npm run verify
npm run tauri:dev:windows
npm run tauri:build:windows
```

On Linux, use:

```bash
npm run verify
npm run tauri:dev:linux
npm run tauri:build:linux
```

On macOS, use:

```bash
npm run verify
npm run tauri:dev:macos
npm run tauri:build:macos:universal
```

Windows release builds emit the release executable and unsigned, offline-capable NSIS installer under `src/BatCave.App/src-tauri/target/release`. The installer embeds Microsoft's WebView2 Evergreen Standalone Installer. This adds roughly 127 MB to the artifact, avoids install-time network access, and leaves runtime security servicing with the Evergreen updater rather than pinning a fixed WebView2 version. BatCave does not publish a separate online-bootstrapper variant. Building the bundle can still download the Microsoft redistributable into Tauri's build cache; shipping and installation do not require that build-time connection. Distribution remains subject to the [Microsoft Edge WebView2 Runtime license](https://www.microsoft.com/software-download/webview2). Linux builds emit `.deb` and AppImage bundles under `src/BatCave.App/src-tauri/target/release/bundle`. Universal macOS output lands under `src/BatCave.App/src-tauri/target/universal-apple-darwin/release/bundle`, including the `.app`, DMG, and release-only updater archive.

## Privacy And Local Data

BatCave is local-only by design. Do not add outbound tracking, remote logging, hosted collection, or surprise network dependencies.

Runtime state, settings, warm cache, helper snapshots, and logs stay under:

- Windows: `%LOCALAPPDATA%\BatCaveMonitor`
- Linux: `$XDG_DATA_HOME/BatCaveMonitor` or `~/.local/share/BatCaveMonitor`
- macOS: `~/Library/Application Support/BatCaveMonitor`

Theme preference is stored in browser `localStorage` under `batcave.monitor.theme`.

## Platform Notes

- Windows per-process network attribution uses ETW over the kernel TCP/IP provider. If the kernel logger cannot start or access is denied, BatCave reports the reason and continues.
- Installed Windows releases request administrator access at startup so protected telemetry is always available. Development builds remain unelevated. The release is single-instance and the per-machine installer is the only supported installed configuration.
- Linux aggregate telemetry uses `/proc` and `/sys`. Optional per-process network attribution uses `bpftrace`/eBPF when the host has the needed permissions or capabilities. Install that optional tool with `bash scripts/install-linux-deps.sh --with-bpftrace`; the default dependency install does not require it.
- macOS telemetry uses sysinfo plus local libproc data for process details. Per-process network attribution and privileged helper mode are intentionally unavailable in this release; the cockpit labels those gaps instead of reporting zero traffic.
- Browser fixture mode is for UI work. It is deterministic on purpose and is not proof of native collector behavior.

## Benchmarks

Run a headless runtime benchmark:

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/run-benchmark.ps1 -BenchmarkHost core -Platform x64 -Ticks 120 -SleepMs 1000
```

Capture a reusable benchmark baseline artifact:

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/capture-benchmark-baseline.ps1 -BenchmarkHost core -Platform x64
```

Run the strict regression gate with either a matching baseline artifact or an explicit p95 budget. The normal validation scripts keep their fast smoke check; this gate is for release/local performance validation and writes a report under `artifacts/benchmarks`.

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/run-benchmark-gate.ps1 -BenchmarkHost core -Platform x64 -BaselineArtifactPath artifacts\benchmarks\baseline-core-YYYYMMDD-HHMMSS.json
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/run-benchmark-gate.ps1 -BenchmarkHost core -Platform x64 -MaxP95Ms 10000
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/validate-tauri.ps1 -SkipBundle -BenchmarkGate -BenchmarkMaxP95Ms 10000
```

Linux and macOS equivalents are available at `scripts/run-benchmark.sh`, `scripts/capture-benchmark-baseline.sh`, and `scripts/run-benchmark-gate.sh`; the shared scripts detect the host and normalize Apple `arm64` to the public `aarch64` contract.

The release benchmark measures the core runtime host's `RuntimeState::refresh_now` path plus snapshot JSON serialization in an isolated temporary data directory. Output carries `evidence_scope: core_runtime_host_only`; it is not whole-app or process-tree evidence. Protocol v3 derives platform and architecture from the executing binary, requires samples to advance, and gates strict runs on latency, speed ratio, app CPU, and RSS. Baseline artifacts include the commit, release-binary hash, machine class, workload, and every repeat.

The complete-remediation release comparison is preserved in [docs/evidence/benchmarks/remediation-20260710.json](docs/evidence/benchmarks/remediation-20260710.json), including source hashes, commit provenance, protocol settings, all repeats, and the strict gate result.

## Continuous Integration

Pull requests and `codex/**` pushes run Windows, Linux, and dual-architecture macOS validation without packaging. Pull requests also reject newly introduced dependencies with moderate-or-higher advisories. Pushes to `main` and manual bundle runs produce Windows NSIS, Linux deb/AppImage, and ad-hoc-signed universal Mac artifacts retained for 14 days. The versioned release workflow produces 30-day dry-run artifacts or durable GitHub Releases with aligned versions, checksums, build provenance, and a Developer ID-signed/notarized/stapled Mac DMG. A separate Monday/manual audit runs `npm audit --omit=dev` and pinned `cargo-audit 0.22.2`.

## More Documentation

- [App runbook](src/BatCave.App/README.md) covers native/browser run modes, app scripts, and platform troubleshooting.
- [Runtime telemetry](docs/runtime-telemetry.md) covers the Rust runtime store, native collectors, quality states, admin helper behavior, and benchmark surfaces.
- [Release channels and verification](docs/releases.md) covers version alignment, stable/prerelease policy, checksums, provenance, and publication.

## Contributing

Keep the app local, explicit, and boringly reliable. Match the existing Rust/Tauri/Svelte boundaries, preserve the snake_case JSON contracts, and run the narrowest meaningful verification before opening a PR.
