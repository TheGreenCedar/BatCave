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

The first native performance run exposed unnecessary hidden-inspector reads. Overview and closed detail panes now suspend those reads; reopening fetches current evidence, and late replies still return their memory credits. History allocation grows to at most 360 slots per identity instead of reserving 512, and row shaping moves already-owned processes without an extra clone. Focused lifecycle, archive, wire, and runtime tests pass; native savings require the next candidate measurement.

Native exit investigation also exposed eviction pressure from unknown-start processes. Their identities are valid for one publication, so repeated observations cannot join into a history. When the archive needs space, it now reclaims obsolete publication-scoped observations before verified process histories. A regression test retains a verified 40-point exit while unknown-start observations churn under a 2 MiB admitted budget. Current unknown identities remain inspectable, and all memory reserves remain in force. Retention still yields when the global budget is exhausted.

Workload membership now requires executable or enclosing bundle identity and verified ancestry. Native probes recheck process generation. Unknown or equal rounded birth times cannot establish parent order, and changing membership changes aggregate identity.

Inspection uses a separate runtime archive rather than the filtered list. It retains CPU, memory, read/write I/O, and network observations with timestamps, quality, scope, and gaps. A 64 MiB admission budget includes retained metadata, shaping allocations, and response copies. Response credits remain held through IPC delivery and are acknowledged after decoding, including discarded responses. A lost acknowledgement intentionally retains its credit until the runtime exits. The archive is session-local; warm cache is not a substitute for live history.

Current-user persistence runs through a bounded worker queue. Only disposable warm-cache writes coalesce. Pending writes remain session-only until the worker confirms them, and an older successful write cannot hide a rejected newer preference. Storage stalls do not hold the publication path.

AI providers select from host-offered explanation IDs. The host validates the selection and renders current measurements and qualifications. Invalid, stale, or failed selections fall back to the deterministic explanation.

Windows creates a missing current-user Start entry on an unelevated installed GUI launch. It preserves existing entries and shared-shortcut retirement policy. The machine uninstaller leaves that per-user entry; users can remove it from Start. Native visibility and lifecycle still require Windows proof.

## Current verification

- Rust library tests pass, including generation reuse, clock/freshness transitions, archive budget and reply lifetime, independent detail lookup, queued persistence, failed writes, and blocked storage.
- Frontend behavior, protocol decoding, type checks, lint, and production build pass. Browser accessibility checks cover selection switching, same-ID reselection, filtered Explore to Overview, process exit, and compact detail. These are layout checks.
- All 26 browser accessibility checks pass after making the icon fixture choose its donor from the canonical Overview ranking. The direct, matched, and fallback icon assertions remain intact.
- [Validation at `ebe7a97`](https://github.com/TheGreenCedar/BatCave/actions/runs/33986856874) passes Windows, Linux, macOS, repository policy, and Linux release transport. Windows passes 499 library tests, including the four native Start-entry tests and COM shortcut contract. Linux passes 429 library tests, all 26 accessibility checks, and four package transport tests. This does not establish installed Start-menu behavior.
- The Rust audit reports zero vulnerabilities and matches the unchanged 17-warning review baseline. Targeted updates cover h2, event-listener, and a yanked chacha20 version.
- npm still reports one high and five moderate findings. Updated nanoid and PostCSS versions are the newest compatible versions currently available from the registry, while the advisories name newer unavailable fixes. The audit remains failing and retains its JSON report.
- Desktop measurement ownership and budget policy passes 32 focused tests. Live Mac checks reject an inherited launcher coalition and validate the resource-coalition counter prefix with the native Mach timebase. Cumulative CPU includes departed helpers. Task starts or exits make sampled RSS coverage incomplete, so an invisible helper cannot produce a false whole-app pass. Linux and Windows AI-on lifetime resource coverage remains unqualified.
- The two-tick debug core smoke passes after removing redundant archive serialization. Publication p95 is 170.4 ms, command p95 is 202.8 ms, and peak runtime CPU is 19.1 percent of one core. This short runtime-only smoke does not establish the whole-app performance contract. The focused 1,024-workload archive probe falls from 113.82 ms to 43.37 ms while retaining every identity; the inspection wire golden remains unchanged.
- [All three package builds at `42af734`](https://github.com/TheGreenCedar/BatCave/actions/runs/33985794915) pass. The later Mac candidate at `8d352af` also passes local packaging, ad-hoc signature checks, and DMG structure checks. These artifacts are not signed public releases.
- [All three package builds at `13abb54`](https://github.com/TheGreenCedar/BatCave/actions/runs/33987858753) also pass. Its local Mac package passes the same ad-hoc and DMG checks. The GUI SHA-256 is `ccdabff13c9ba35970fee69927e1f26ee1c25d1279452a4abcbbb69d1cb75379`.
- Native Mac candidate `8d352af`, GUI SHA-256 `9692d890898110a72112c95a8cd27d7378d32bdebc2da9a93f390242b1ca0ad0`, passes filtered Explore to Overview contributor selection, A to B to A history, same-ID reselection, all four history choices, pause/resume, and search/settings keyboard checks. The 360-point view retains timestamped evidence collected before selection. Process-network measurements correctly remain unavailable on the unqualified Darwin 27 layout. A later controlled process exit reaches the archive's explicit eviction state under the global history budget; unrestricted exit retention is not claimed.
- The first native AI-off gate on that exact Mac candidate records 117 publications, 13 trusted interactions, and at least 805 processes. Publication age p95 is 1,042 ms and input-to-first-render p95 is 90 ms. CPU fails at 12.76 percent of one core. Sampled RSS reaches 630.7 MiB, above 512 MiB, and helper churn prevents complete RSS coverage. The report remains incomplete and failing. Local evidence is under `artifacts/rescue/macos-8d352af/`.
- The later `13abb54` native resource window also fails: 14.24 percent of one core and 627.2 MiB sampled RSS. It spends more time in Explore, includes a controlled process-exit investigation, and records only eight trusted interactions within the window, so it cannot establish a performance improvement or a responsiveness pass. Its incomplete report is retained under `artifacts/rescue/macos-13abb54/`. Native profiling identifies repeated sysinfo process-metadata reads and frontend work as remaining costs; no collector replacement or budget relaxation is included.

## Remaining delivery gates

The local desktop measurement mode writes only to an explicitly supplied, exclusive local report path. It records accepted publication-to-render delay and trusted input-to-first-render delay. `scripts/measure-desktop.py` adds native process resources, generation and ownership checks, AI preference verification, and the fixed 30-second warmup plus 120-second window. First-render timing does not establish asynchronous inspector completion. A native run must meet the original budgets before performance is accepted.

To capture a native run, select one-second sampling and the intended AI preference before launch. Start the packaged GUI with `BATCAVE_DESKTOP_PROBE_PATH` set to a new absolute JSONL path, then attach `python3 scripts/measure-desktop.py --pid <GUI_PID> --probe-jsonl <ABSOLUTE_JSONL> --output <NEW_ABSOLUTE_REPORT> --expected-executable <ABSOLUTE_GUI_BINARY> --ai-mode off` during the first 29 seconds. On macOS, launch the app bundle through LaunchServices with `open -n <APP_BUNDLE> --env BATCAVE_DESKTOP_PROBE_PATH=<ABSOLUTE_JSONL>`; executing the binary from a shell inherits the shell's coalition and is rejected. Keep the native window visible and perform at least ten normal interactions during seconds 30–150. The report requires at least 500 observed processes, complete publication coverage, verified resource ownership and lifetime coverage, and the existing resource budgets. Mac CPU uses cumulative resource-coalition counters. Process churn preserves valid CPU evidence but makes sampled RSS incomplete; it cannot produce an overall pass. It writes an incomplete report on missing evidence and never replaces an existing report. Run AI-on separately with `--ai-mode on`.

Mac native screenshots and investigation evidence are captured; its performance gate remains open. Windows and Linux native keyboard, investigation, and whole-app performance proof remain open because the configured proof hosts timed out. This Mac's Darwin 27 process-network layout is unqualified; its expected behavior is explicit unavailable process traffic while other telemetry continues. No new NStat layout was enabled.

Final-candidate package builds and exact-byte installed lifecycle evidence remain release gates. The repository currently lacks the independent reviewer and platform signing credentials required by its release controls. Those controls remain in force. Building a candidate or passing source tests does not complete public release verification.

macOS 27 exposed a compiler-tooling failure before application compilation: the loader rejects a stripped proc-macro library's misaligned string table, matching [Rust issue 157750](https://github.com/rust-lang/rust/issues/157750). The release profile now preserves build-tool metadata. The same release compilation passes with that override; application optimization settings are unchanged.
