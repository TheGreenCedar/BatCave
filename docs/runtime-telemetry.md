# Runtime telemetry

The Rust runtime collects local telemetry and publishes snake_case JSON to the UI. Each metric includes its source, quality, and limitations. Sampling, history, health, and persistence stay in Rust.

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
        -> Rust collectors and collector-service client
          -> Win32 process telemetry
          -> Win32 system telemetry
          -> PDH disk-rate telemetry
          -> ETW per-process network attribution
          -> Linux /proc and /sys telemetry
          -> Linux optional bpftrace/eBPF network attribution
          -> macOS sysinfo, libproc, NStat, and deduplicated IOKit telemetry
          -> installed Windows collector service with standard-access fallback
          -> Rust benchmark CLI
```

The monitoring commands are:

- `get_snapshot`
- `get_workload_inspection`
- `acknowledge_workload_inspection`
- `refresh_now`
- `pause_runtime`
- `resume_runtime`
- `set_sample_interval`
- `set_process_query`
- `get_process_icons`

The frontend renders runtime state and keeps temporary interaction state. The Rust runtime owns settings, pause state, cadence, queries, collector-service state, cache, health, diagnostics, and persistence.

Every snapshot carries two clocks. `publication_seq` and `published_at_ms` change for every response, including query, pause, cadence, and error publications. `sample_seq` and nullable `sampled_at_ms` change only after successful collection. A Rust worker owns sampling; frontend reads are passive. The required `environment` object reports `platform`, current-process `process_elevation`, runtime-derived `install_kind`, and the resolved `data_directory`. `admin_mode.source` separately identifies whether protected collection comes from the current process or the installed collector service.

Runtime protocol v4 omits the retired `seq`, `ts_ms`, and `focus_mode: active` aliases. Process identity uses PID and verified start time. Unknown-start identities last for one telemetry sample (`sample_seq`), so PID reuse cannot inherit earlier state. Query and pause publications retain that sample's identity. Empty kernel-pool-tag `driver_candidates` serialize as `[]`.

The v2 rename established the process I/O names retained in protocol v4. Canonical publications use `process_contributors.io`, process `io_read_total_bytes`, `io_write_total_bytes`, `io_read_bps`, and `io_write_bps`, process `quality.io`, and the `io_bps` sort column. The former process names `disk`, `disk_*`, and `disk_bps` remain Rust deserialization aliases only, so persisted legacy input can be read and republished under the current names. BatCave does not dual-write those aliases, and they are not canonical output or a compatibility promise. System `disk_*` fields keep their existing meaning: physical-device telemetry. Version migration and transport hardening are tracked in issue #67.

`process_contributors` is a compact, query-independent summary of the top CPU, memory, read/write I/O, and network process names from the complete sample. Each resource also carries contributor quality and an explicit `*_name_ambiguous` flag computed from that full sample. Held and unavailable rows cannot become the published contributor; estimated and partial winners remain visible with limited confidence. A `null` name with valid quality means no process reported activity, while a `null` name with held or unavailable quality means attribution is not currently trustworthy. Search, focus, sorting, and limits still shape `processes` and `process_view_rows` for the workload table, but cannot rewrite or falsely disambiguate the selected-resource overview. Overview selects from query-independent `overview_rows` with workload identities. Names and ambiguity flags alone do not identify the selected workload.

## Current coverage

The runtime provides:

- Production snake_case JSON contracts for runtime snapshots, settings, process samples, system metrics, quality metadata, warnings, and health.
- Runtime settings, warm cache, keyed current warnings, diagnostics transition history, health budgets, query shaping, pause/resume, refresh, and collector-service state in the Rust store.
- Stable process grouping for runtime process views, including aggregate one-core-equivalent CPU, memory, read/write I/O, network, and thread totals for group rows.
- Local JSON persistence with same-directory, per-writer atomic temporary files for settings and runtime state.
- Rust CLI mode for deterministic benchmark snapshots.

### Windows

- Process identity, PID, parent PID, start-time identity, executable path, access state, one-core-equivalent CPU, kernel CPU, memory, private bytes, process read/write I/O totals, thread count, and handle count. Windows `GetProcessIoCounters` exposes `ReadTransferCount`, `WriteTransferCount`, and `OtherTransferCount` separately. BatCave's read/write I/O value sums only read and write; other I/O remains a separate raw process field. None of these counters are called physical-disk traffic.
- Physical memory, Windows commit totals, kernel paged/nonpaged pool, system cache, aggregate CPU deltas, logical CPU percentages, interface-level network totals/rates, and PDH physical-disk rates.
- ETW per-process network attribution over the Windows kernel TCP/IP provider.
- The running Windows process token determines `environment.process_elevation`. NSIS, portable, and development provenance does not imply elevation. The app starts with the invoking user's token and remains `asInvoker`. `admin_mode.source` reports `collector_service` only after authenticated service negotiation and never rewrites the parent process token. Missing or rejected service access keeps standard monitoring running. An unreadable token is labeled `unknown` and never presented as confirmed standard or elevated access.

### Linux

- Aggregate CPU, kernel CPU, logical CPU deltas, memory, swap, block-device I/O totals/rates, and interface network totals/rates.
- Process identity, PID, parent PID, start time, RSS/private memory, virtual memory, process I/O totals, thread count, and file descriptor count.
Linux process-network attribution is optional. It uses `bpftrace`/eBPF entry and return probes on `__sock_sendmsg` and `sock_recvmsg`. bpftrace 0.22.0 or newer is required for synchronous map iteration across probes, keyed existence checks, and controlled map printing on exit. The send probe follows the syscall path because the `sock_sendmsg` wrapper is unused on some supported kernels.

Each entry clears orphaned per-thread family state before accepting IPv4 or IPv6. A missed return therefore cannot attribute later Unix-domain traffic as IP traffic or consume attribution capacity.

#### Epoch validation

Each one-second boundary starts a new in-kernel epoch. The epoch that just closed waits for a full interval; the previous epoch is then drained. This grace covers bounded, non-sleeping kprobe programs that observed the old epoch while it was active. Every later sweep counts keys older than its drain target and fails if it finds one.

The probe's protocol v2 holds each validated epoch in Rust until the next complete epoch boundary. Ordered receive and transmit sections contain copied scalar entries, count and byte summaries, stale-key and in-band overflow counts, a monotonic epoch, and a final marker. A mismatch, stale key, overflow, missing boundary, missing marker, or missing epoch rejects the interval without consuming the held epoch. Validated windows accumulate until the 500/1000/2000/5000 ms app cadence consumes them.

#### Capacity and failure handling

Maps have a hard 16,384-key limit and an 8,192-key soft insertion limit. The spare half reserves one check-to-insert race per configured CPU. Startup rejects hosts with more than 8,192 configured CPUs. Existing keys may keep accumulating above the soft limit.

The child writes stderr and protocol output to the same pipe, preserving write order for the single reader. Any non-protocol diagnostic is fatal. The ordered overflow fields independently detect capacity loss.

Shutdown reaps the child and joins the reader. A killed child, pipe EOF or error, malformed output, diagnostic, or missing interval makes attribution unavailable. Startup and post-spawn failures share three attempts, 30 seconds apart. Only a complete publishable interval resets that failure episode.

`scripts/install-linux-deps.sh --with-bpftrace` installs an apt candidate only if it meets the version floor, or verifies a supported preinstalled build. It rejects Ubuntu 24.04's 0.20.x candidate. The base app continues monitoring and reports the unavailable probe.


### macOS

- Sysinfo aggregate CPU, logical CPU, available/used memory, swap, and interface network counters.
- Local libproc enrichment for resident memory, physical footprint, virtual memory, process read/write totals, thread count, and file-descriptor count when process access allows.
- IOKit `IOBlockStorageDriver` byte counters aggregated once per physical registry entry. Disk-image paths are excluded; incomplete physical coverage is unavailable, and topology changes establish a new rate baseline. Process read/write I/O is never substituted for system disk telemetry.
- The sysinfo interface aggregate includes `lo0`; protocol v4 labels its scope `all_interface_aggregate` rather than non-loopback.
- Per-process TCP, UDP, and QUIC network attribution comes from one long-lived XNU NStat control socket. Absolute source counters are baselined and differenced on a dedicated reader thread, including final close updates for short-lived flows. Each collector session checks the consumed NStat revision-9 fields against four local TCP/UDP flows before publishing rates. It verifies socket endpoints, the current process identity from libproc, and exact payload counters; these probe flows are excluded from app rates. Compatible aligned descriptor tails can grow without an OS-version allowlist. Unknown message types, unrequested extensions, malformed prefixes, or a failed six-second qualification window make attribution unavailable and trigger a bounded retry. Subscriptions include zero-byte counters because XNU can report intentionally filtered closing updates as dropped data. It needs neither root nor a private entitlement. Privileged collection remains unavailable.

### Fallbacks

- `sysinfo` remains available when a native collector cannot read expected host files.
- The shared `sysinfo` system fallback marks physical-disk throughput unavailable because it has no device-level rate source; its zero placeholders are never presented as measured disk activity.
- Missing or delayed metrics use quality metadata instead of fabricated values.
- If native Windows process memory is blocked but `sysinfo` has a value for the same PID, BatCave reports that fallback as estimated memory instead of a native zero.
- Grouping requires executable or bundle identity and verified ancestry. Missing metadata leaves independent processes separate; a matching name alone cannot establish a group.

## Memory accounting

`memory_used_bytes` is physical memory used by the machine. It includes process working sets, kernel memory, cache, drivers, virtualization/WSL, and other OS-resident memory. It is not expected to equal the sum of process rows.

On Windows, `swap_used_bytes`, `swap_total_bytes`, and process `virtual_memory_bytes` are omitted because the available native counters represented commit charge, not those cross-platform concepts. Windows commit remains available as `memory_accounting.commit_used_bytes` and `commit_limit_bytes`. Linux reports real swap and process virtual-memory values when available. macOS reports available memory directly and uses libproc physical footprint as its private-memory presentation when accessible.

When available, `system.memory_accounting` adds the reconciliation view:

- `process_working_set_bytes` and `process_private_bytes`: diagnostic sums across process rows whose memory quality is reported. Shared pages can appear in more than one working set, so these sums are not reconciled against physical memory use.
- `denied_process_count` and `partial_process_count`: rows where BatCave did not get complete access.
- `commit_used_bytes` and `commit_limit_bytes`: Windows commit charge and limit from `GetPerformanceInfo`.
- `kernel_paged_pool_bytes`, `kernel_nonpaged_pool_bytes`, `kernel_total_bytes`, and `system_cache_bytes`: Windows OS memory buckets from `GetPerformanceInfo`.
- `kernel_pool_tags`: top Windows kernel pool tags from `NtQuerySystemInformation(SystemPoolTagInformation)`, split by paged/nonpaged pool and sorted by bytes.

Denied process rows remain visible because blocked access is itself useful telemetry. The UI must render unavailable process memory as blocked/unavailable, not as measured `0 B`.

Pool tags are allocation labels, not guaranteed driver ownership. BatCave may attach `driver_candidates` by scanning local installed `.sys` binaries for the tag bytes, but the UI must present those names as best-effort candidates. The scan runs outside the telemetry hot path, so `driver_candidates_pending` can be true on early snapshots. Unknown tags and missing candidates are expected after the scan completes.

## Quality states

BatCave treats metric quality as part of the product contract. If a number is incomplete, warming up, blocked, or platform-dependent, the snapshot should say so.

Common quality states:

- `native`: collected directly by the platform collector.
- `held`: temporarily held while the runtime waits for a second sample or rate derivation.
- `estimated`: supplied by a known fallback, such as Linux RSS when anonymous private memory is unavailable.
- `partial`: derived from a fallback or incomplete source.
- `unavailable`: unavailable on this platform or blocked by permissions.

Examples:

- First CPU samples may be held until native deltas are available.
- Disk rates may be partial if PDH or block-device counters are unavailable.
- Linux CPU, disk, and network retain independent last-good baselines. A failed read does not replace a baseline with zero, and the first recovered rate is derived only from valid counters.
- Windows process network attribution reports the ETW failure reason when the kernel logger cannot start.
- Linux per-process network attribution reports the eBPF prerequisite or capability failure when the host cannot attach probes.
- macOS per-process network attribution is `held/nstat` during its initial baseline, `native/nstat` after a complete interval, `partial/nstat` on detected counter loss, and `unavailable/nstat` if the private protocol is unavailable or no longer matches its qualified layout.
- Linux process `/proc` parsers remain manual after the bounded [`procfs` parity decision](decisions/0002-linux-procfs-parser-parity.md). Required malformed counters fail instead of becoming measured zero; a crate replacement requires native dual-reader parity first.
- macOS physical-disk quality is `held/iokit` while a device identity baseline is pending, `native/iokit` for a complete stable device set, and `unavailable/iokit` when complete host coverage cannot be proven.
- macOS libproc failures retain the sysinfo process row and classify exit, access denial, unsupported fields, and collector failures separately. Exit drops ordinary churn; independently successful fields remain publishable.

## Process groups and history

Groups contain only verified related processes. Their CPU, memory, read/write I/O, network, and thread totals carry aggregate quality and coverage. A membership change creates a different group identity.

The inspector reads from a query-independent runtime archive. Each identity can retain up to 360 timestamped samples, subject to a global 64 MiB budget that includes metadata, shaping, and response copies. The 30/72/180/360 controls select the visible window. Selection changes and process exit do not erase retained history; eviction is reported when the budget requires it.

Overview and closed detail panes stop inspection reads. The frontend acknowledges every decoded reply, including replies discarded after selection changes. Until acknowledged, its memory credit remains reserved.

Group Other I/O total and rate remain unavailable because the current process-view row has no typed aggregate for them. The UI must not copy the representative process's total or manufacture a zero rate. Issue #64 owns that typed aggregate coverage.

Network readouts prefer a nonzero live attributed rate when the row has one, while the quality label still reports the source or limitation. This keeps estimated, fixture, partial, or unavailable quality honest without hiding useful accumulated traffic.

## Local data

Runtime state, settings, warm cache, and logs are local-only.

- Windows: `%LOCALAPPDATA%\BatCaveMonitor`
- Linux: `$XDG_DATA_HOME/BatCaveMonitor` or `~/.local/share/BatCaveMonitor`
- macOS: `~/Library/Application Support/BatCaveMonitor`

The runtime publishes the resolved path through `environment.data_directory`. The typed `environment.install_kind` is derived from running-package evidence:

The [current-user state ownership and retention contract](current-user-state.md) defines the files below this path, their permissions, failure behavior, retention limits, and manual-cleanup boundary.

- Windows reports `nsis` only when the current executable directory matches Tauri's `BatCave Monitor` uninstall-registry location; unmatched release executables are `portable`, and debug binaries running from a development output directory are `development`. If the executable path or required registry evidence cannot be read, the package state is `unknown` rather than fabricated as portable.
- Linux reports `appimage` from the AppImage runtime environment, `deb` when the local Debian package database owns the executable, `development` for a debug binary in a development output directory, and `portable` otherwise.
- macOS reports `development` for a debug binary in a development output directory, `app_bundle` for a running `.app`, and `portable` for a standalone binary. A copied app does not claim `dmg` because its original download container is no longer observable at runtime.

On Windows, `GetTokenInformation(TokenElevation)` determines the parent process state: `standard`, `elevated`, or `unknown`. Privileged collection has its own state and source. A manually elevated parent uses `current_process`; an installed collector uses `collector_service`; missing, stopped, incompatible, or unauthorized service states retain standard monitoring. Linux and macOS report privileged collection as unavailable.

Do not add outbound tracking, hosted collection, or remote logging.

## Validation

Run Rust tests directly:

```powershell
cargo test --manifest-path src/BatCave.App/src-tauri/Cargo.toml
```

Run full Windows validation:

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/validate-tauri.ps1
```

Run full Linux or macOS validation:

```bash
bash scripts/validate-tauri.sh
```

The macOS bundle path requires the Apple Silicon Rust target and produces one `arm64` DMG. Intel Macs are unsupported:

```bash
rustup target add aarch64-apple-darwin
cd src/BatCave.App
npm run tauri -- build --target aarch64-apple-darwin
```

Run a headless benchmark:

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/run-benchmark.ps1 -BenchmarkHost core -Platform x64 -Ticks 120 -SleepMs 1000
```

Capture a reusable benchmark baseline:

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/capture-benchmark-baseline.ps1 -BenchmarkHost core -Platform x64
```

Run the strict regression gate:

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/run-benchmark-gate.ps1 -BenchmarkHost core -Platform x64 -BaselineArtifactPath artifacts\benchmarks\baseline-core-YYYYMMDD-HHMMSS.json
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/run-benchmark-gate.ps1 -BenchmarkHost core -Platform x64 -MaxP95Ms 10000
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/validate-tauri.ps1 -SkipBundle -BenchmarkGate -BenchmarkMaxP95Ms 10000
```

The benchmark builds the current release CLI and drives one-shot `RuntimeState::refresh_now` commands through the owned sampling engine in an isolated temporary data directory. Artifact format v4 counts collector work and runtime shaping in collection latency. Persistence writes run on a separate worker. Publication latency starts with immutable snapshot construction and ends at the single `Arc`/`RwLock` swap. Protocol-v4 encoding and JSON serialization are measured afterward. Each repeat reports `collection_p95_ms`, `publication_p95_ms`, `serialization_p95_ms`, and `live_command_p95_ms`; the summary records the median of each component. The latency ceiling and baseline speed ratio use `median_live_command_p95_ms`.

The default protocol is 30 warmup commands followed by five 120-command measured repeats with a 1000 ms inter-command delay. The CLI keeps `-SleepMs`/`--sleep-ms` for compatibility, while v4 JSON names the field `inter_command_delay_ms`. Normal validation uses zero warmup, one two-command repeat, and the same explicit delay. Every measured refresh must advance the sample sequence exactly once.

V4 baseline artifacts record `measurement_origin: owned_sampling_engine_refresh_and_protocol_serialization`, `evidence_scope: core_runtime_host_only`, `whole_app_measured: false`, `live_command: refresh_now`, the in-process bounded-channel transport, and the protocol-v4 encoding/JSON serialization scope. They also record the source commit, release-binary SHA-256, binary-derived platform and architecture, machine class, workload, protocol parameters, component medians, repeat results, and `baseline_selection: median-by-live-command-p95`. Older artifact versions are rejected because their timing boundaries are different. Strict mode also requires app CPU at or below 25% and RSS at or below 350 MiB; these remain core-host measurements, not Tauri shell, webview, renderer, or process-tree evidence.

## Continuous integration

- Pull requests and `codex/**` pushes run Windows and Linux validation without bundles.
- Pull requests and `codex/**` pushes also check and lint Apple Silicon macOS and build the `aarch64-apple-darwin` target without packaging.
- Pull requests run dependency review and fail on new moderate-or-higher advisories.
- Pushes to `main` and manual bundle runs produce an offline-capable Windows NSIS installer, Linux deb/AppImage artifacts, and an ad-hoc-signed Apple Silicon Mac `.app`/DMG retained for 90 days. The Windows artifact embeds the WebView2 Evergreen Standalone Installer, trading roughly 127 MB of package size for install-time network independence while retaining Evergreen servicing.
- Monday 09:00 UTC and manual advisory runs audit all npm dependencies at moderate severity and run pinned `cargo-audit 0.22.2`. Both JSON reports are retained for 90 days even when the gate fails. Rust vulnerabilities fail; informational warnings must match the owned, expiring baseline documented in `docs/dependency-advisories.md`. Dependabot proposes weekly compatible npm and Cargo updates.

## Remaining product work

Release and native verification still require:

- Add installer signing before broad external distribution.
- Promote the stable channel only after Authenticode signing proof is available.
- Supply the documented Apple Developer ID and notarization secrets for public Mac releases; ad-hoc main-branch bundles are test artifacts only.
- Expand screenshot-based validation from the native Tauri app when changing layout, metric-quality messages, themes, or platform-specific collector states.

Whole-app performance results and remaining acceptance checks are recorded in [Rescue implementation](rescue-implementation.md).
