# Developer triage rescue

BatCave must identify the right workload, preserve the evidence needed to inspect it, and state the limits of its measurements. This implementation preserves Windows, Linux, Apple Silicon, Overview and Explore, local storage, and opt-in local AI.

## Accepted changes

1. Derive health reasons and freshness in the runtime. Render value, units, quality, coverage, and time consistently. Utilization does not establish pressure or causation.
2. Keep independent development jobs separate. Group only verified related processes; revalidate generation across native probes.
3. Resolve workload details independently of list filters. Keep bounded timestamped history across selection changes and process exit. Preserve the existing 30/72/180/360 history choices, with a global 64 MiB history cap.
4. Let opt-in AI select an eligible explanation ID. The host owns wording, measurements, qualifications, and fallback. Preserve explicit downloads and local inference.
5. Move persistence off publication, measure all owned app processes, repair dependency maintenance, provide a current-user Windows launch entry, and publish verified packages for all three platforms.

## Verification contract

- Genuine zero, unavailable, held, partial, paused, stale, and failed states agree across both screens, charts, summaries, and explanations.
- Independent Node/Python jobs remain separate; related helpers group correctly; PID replacement cannot inherit measurements or history.
- Search to Overview to contributor opens the correct identity. A to B to A preserves history. Ranking behaves across desktop and compact layouts.
- Clock adjustment, sleep/wake, missing samples, cadence changes, collector recovery, and slow persistence preserve truthful state.
- Invalid or stale explanation IDs fall back. A low-activity sample cannot admit heavy-pressure claims. Provider failure leaves monitoring usable.
- Packaged native keyboard, focus, readability, and investigation checks pass on Windows, Linux, and macOS.
- At one-second cadence with 500 processes, p95 publication age is at most two seconds and interaction latency at most 100 ms. Initial AI-off budgets are 10 percent of one logical CPU averaged over the run and 512 MiB for owned processes. Measure 120 seconds after a 30-second warmup; report AI costs separately.
- Exact public bytes pass download, installation, launch, reopening, supported updates/rollback, and uninstall checks. Signing and notarization claims match the artifacts.

## Implementation status

Work began at source commit `31a4382e7e9228ce7198e6b11dfcc726a01cbcdd` on `codex/rescue-developer-triage`. The earlier native audit inspected an installed app reporting rc.4, not a byte-matched build of this branch. The existing untracked `work/` directory is outside the change.

The runtime, inspection, presentation, persistence, AI explanation, and Windows launch changes are implemented. Desktop protocol v4 carries runtime-owned freshness and reasons plus query-independent Overview workloads. Collector-service IPC stays at v1. The declared package version remains `0.2.0-rc.5`.

Workload membership now requires executable or enclosing bundle identity and verified ancestry. Native probes recheck process generation. Unknown or equal rounded birth times cannot establish parent order, and changing membership changes aggregate identity.

Inspection uses a separate runtime archive rather than the filtered list. It retains CPU, memory, read/write I/O, and network observations with timestamps, quality, scope, and gaps. A 64 MiB admission budget includes retained metadata, shaping allocations, and response copies. Response credits remain held through IPC delivery and are acknowledged after decoding, including discarded responses. A lost acknowledgement intentionally retains its credit until the runtime exits. The archive is session-local; warm cache is not a substitute for live history.

Current-user persistence runs through a bounded worker queue. Only disposable warm-cache writes coalesce. Pending writes remain session-only until the worker confirms them, and an older successful write cannot hide a rejected newer preference. Storage stalls do not hold the publication path.

AI providers select from host-offered explanation IDs. The host validates the selection and renders current measurements and qualifications. Invalid, stale, or failed selections fall back to the deterministic explanation.

Windows creates a missing current-user Start entry on an unelevated installed GUI launch. It preserves existing entries and shared-shortcut retirement policy. The machine uninstaller leaves that per-user entry; users can remove it from Start. Native visibility and lifecycle still require Windows proof.

## Current verification

- Rust library tests pass, including generation reuse, clock/freshness transitions, archive budget and reply lifetime, independent detail lookup, queued persistence, failed writes, and blocked storage.
- Frontend behavior, protocol decoding, type checks, lint, and production build pass. Browser accessibility checks cover selection switching, same-ID reselection, filtered Explore to Overview, process exit, and compact detail. These are layout checks.
- Windows-only launch and COM code passed a Windows-target metadata check. The installed-image wrapper was stubbed for that check; it does not establish a full Windows build or execution.
- The Rust audit reports zero vulnerabilities and matches the unchanged 17-warning review baseline. Targeted updates cover h2, event-listener, and a yanked chacha20 version.
- npm still reports one high and five moderate findings. Updated nanoid and PostCSS versions are the newest compatible versions currently available from the registry, while the advisories name newer unavailable fixes. The audit remains failing and retains its JSON report.
- Desktop measurement ownership and budget policy passes 21 focused tests. A live Mac check rejects an inherited launcher coalition. The Windows service path has source and policy-test review, with native execution still pending.
- The two-tick debug core smoke passes after removing redundant archive serialization. Publication p95 is 170.4 ms, command p95 is 202.8 ms, and peak runtime CPU is 19.1 percent of one core. This short runtime-only smoke does not establish the whole-app performance contract. The focused 1,024-workload archive probe falls from 113.82 ms to 43.37 ms while retaining every identity; the inspection wire golden remains unchanged.

## Remaining delivery gates

The local desktop measurement mode writes only to an explicitly supplied, exclusive local report path. It records accepted publication-to-render delay and trusted input-to-first-render delay. `scripts/measure-desktop.py` adds native process resources, generation and ownership checks, AI preference verification, and the fixed 30-second warmup plus 120-second window. First-render timing does not establish asynchronous inspector completion. A native run must meet the original budgets before performance is accepted.

To capture a native run, select one-second sampling and the intended AI preference before launch. Start the packaged GUI with `BATCAVE_DESKTOP_PROBE_PATH` set to a new absolute JSONL path, then attach `python3 scripts/measure-desktop.py --pid <GUI_PID> --probe-jsonl <ABSOLUTE_JSONL> --output <NEW_ABSOLUTE_REPORT> --expected-executable <ABSOLUTE_GUI_BINARY> --ai-mode off` during the first 29 seconds. On macOS, launch the app bundle through LaunchServices with `open -n <APP_BUNDLE> --env BATCAVE_DESKTOP_PROBE_PATH=<ABSOLUTE_JSONL>`; executing the binary from a shell inherits the shell's coalition and is rejected. Keep the native window visible and perform at least ten normal interactions during seconds 30–150. The report requires at least 500 observed processes, complete publication coverage, a stable owned process set, and the existing resource budgets. It writes an incomplete report on missing evidence and never replaces an existing report. Run AI-on separately with `--ai-mode on`.

Fresh native candidate screenshots, keyboard investigation, and interactive performance evidence remain open. The configured Windows and Linux proof hosts timed out. This Mac's Darwin 27 process-network layout is unqualified; its expected behavior is explicit unavailable process traffic while other telemetry continues. No new NStat layout was enabled.

All three package builds and exact-byte installed lifecycle evidence remain release gates. The repository currently lacks the independent reviewer and platform signing credentials required by its release controls. Those controls remain in force. Building a candidate or passing source tests does not complete public release verification.
