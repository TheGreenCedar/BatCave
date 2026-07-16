# Windows lifecycle proof controller

The installed collector-service acceptance matrix must run through one source-controlled controller. Temporary PowerShell orchestration, ad hoc service commands, and repeated elevation attempts are not evidence.

This document is the implementation gate for the attended #69/#70 Windows proof. A private source-controlled controller now owns the fixed plan, exact artifact binding, authenticated one-elevation broker, protected evidence root, and Job Object process settlement. Its mutation entry remains deliberately fail-closed with `lifecycle_controller_not_reviewed` until the complete lifecycle sequence and independent review are accepted. No installed lifecycle evidence is claimed yet.

## Fixed architecture

The controller is a private Rust proof binary with two fixed modes:

1. A standard-user parent performs every read-only preflight available to its token, creates a 256-bit nonce and monotonic request sequence, retains no-write/no-delete handles for the exact installers, launches standard-token desktop phases, and requests one elevation.
2. One elevated worker authenticates the named-pipe peer by PID, process creation time, user SID, session, token elevation, controller file identity, and controller SHA-256. Each nonce-bound message also carries a canonical payload digest. The worker creates a protected private staging root, completes the privileged pre-mutation observations that the standard token cannot read, copies and revalidates the exact installer bytes, and executes a compiled lifecycle state machine. The request cannot supply a command, arguments, destination, observation, result, or evidence payload.

The thin `scripts/run-windows-lifecycle-proof.ps1` wrapper may hash-check and stage the two plan-bound installers, build the exact controller head, copy those controller bytes out of Cargo's hard-linked output into the private proof staging root, and select `preflight` or the explicit `-Run` entry. It must not own installer arguments, service actions, process settlement, cleanup, observations, or evidence decisions.

Every installer, uninstaller, desktop, WebView, and proof child belongs to an owned kill-on-close Job Object with an absolute deadline. The worker creates each child suspended, assigns it to the Job, resumes it, waits for the entire process tree, and revalidates the owned bytes only after the Job reports zero active processes. Timeout terminates the Job and proves settlement before any cleanup or further mutation.

The elevated broker starts from the canonical system directory instead of the invoking user's working directory. Installer and uninstaller trees receive a fixed Unicode environment containing only canonical machine `ComSpec`, `Path`, `SystemRoot`, and `WINDIR` values plus `TEMP` and `TMP` bound to the protected evidence root. Their current directory is also the protected evidence root. The controller does not inherit caller-controlled process lookup or extraction paths into a privileged mutation.

The elevated worker must run a protected copied uninstaller with:

```text
/S _?=C:\Program Files\BatCave Monitor
```

`_?=` is last and unquoted. Waiting on the installed `uninstall.exe /S` launcher observes only NSIS's outer self-copy stub and can report a false zero while the real uninstall fails. See the [NSIS command-line reference](https://nsis.sourceforge.io/Docs/Chapter3.html) and [uninstaller error-level behavior](https://nsis.sourceforge.io/Docs/AppendixD.html).

## Preflight gate before UAC

The parent stops before elevation unless all of these checks pass:

- The repository, branch, exact commit, installer paths, file sizes, and SHA-256 values match the requested proof packet.
- The controller and both worker modes passed unit tests and exact-head build validation.
- The current machine state is one documented allowlisted starting state; an access error or malformed state is not treated as absence.
- The standard token captures the install root, service status, uninstall registration, current binaries, product processes, and readable user-state roots. The authenticated elevated worker must capture the service DACL and protected service data root before its first mutation; neither ACL is weakened for proof convenience.
- No BatCave desktop, WebView, installer, uninstaller, or prior controller process is running.
- The fixed protected `%ProgramData%` evidence leaf for the nonce does not already exist. The controller never reuses or clears a stale evidence root.
- An independent review accepted the exact controller head.

Every environmental probe returns exactly one of:

```text
Present(value) | Absent | Unknown(error)
```

Only documented not-found results are `Absent`. Access denial, malformed registry or SCM data, WMI/CIM failure, ETW query failure, pipe-query failure, and timeout are `Unknown` and stop the proof.

## Closed lifecycle sequence

The worker owns the sequence and does not improvise after failure:

1. Capture the complete initial state.
2. If the machine is in the allowlisted stopped `1066/1` legacy state, run the exact final candidate as `/S /UPDATE` so the new staged transaction repairs it.
3. Prove the exact final service process generation, path and digest, pipe server PID, active ETW lease generation, clean failure-marker state, standard-token desktop fallback/privileged state, and process-tree settlement.
4. Run the protected copied final uninstaller with the direct `_?=` form and prove total product absence.
5. Install the exact baseline.
6. Exercise clean stop/start, second desktop instance, crash/retained ownership, restart/recovery, and standard-token desktop states.
7. Seed the exact historical CLI bytes and known retired-helper artifacts plus an unknown helper sentinel.
8. Upgrade to the exact final installer.
9. Prove the final service generation and that the historical CLI and known helper artifacts were removed while the unknown sentinel was preserved.
10. Exercise final clean stop/start, crash/recovery, and incompatible/missing/stopped fallback states.
11. Uninstall through the protected copied uninstaller and prove final residue and current-user retention policy.

Reboot and independent multi-user evidence remain separate attended packets when they cannot be completed inside the same worker lifetime.

The source-controlled worker now retains the exact historical CLI bytes before the initial final repair, uses the installed service's production `--provision prepare-upgrade` and `--provision install` verbs for the baseline clean restart, and has one private exact-image-verified termination primitive for the baseline crash/recovery transition. The primitive validates the fixed SCM contract, stable path, SHA-256, running PID, and process identity before `TerminateProcess`; a failure after termination is classified as unsettled until SCM, the process handle, and the service lifecycle marker all prove release. This remains source-only while `CONTROLLER_READY` is false.

The incompatible-service fixture is now plan-bound to exact retained bytes built from final-candidate source commit `17270482aae5d762e3ff2af5779369a506dd3ac3`. It is the production collector service with only its Windows ProductVersion overridden to `0.2.0-rc.3`; the desktop must therefore reach the real canonical-path, LocalSystem, pipe, process, and file-identity checks before classifying the transport-verified release as incompatible. The parent and worker independently retain and revalidate its fixed path, size `1,556,992`, and SHA-256 `4a539fb9b86ad7c5e32f61f5cd97650842346ea0e713f58fb457d5f32495378c`; the wrapper additionally verifies the declared ProductVersion. The selected build used Cargo lock SHA-256 `693322cd3885ac4a9c823cd5facd13a9d995572e60f7b99af4cc49a39a5dd28b`, Rust `1.96.1`, and Cargo `1.96.1`. The selected ignored artifact is not rebuilt during an attended run.

`scripts/build-windows-lifecycle-service-fixture.ps1` is the recovery verifier for those selected bytes. It accepts only a clean worktree at the plan's fixed final-candidate source commit, uses a dedicated cache target outside that source worktree, unsets `BATCAVE_SOURCE_COMMIT_SHA`, and applies the plan's fixed Tauri version override. A cache-assisted build must match both the pinned plan identity and any retained bytes before it can populate a missing artifact; the script fails instead of replacing a different retained fixture or rewriting the plan. Cross-machine bit reproducibility is not claimed because the Rust, MSVC, SDK, and linker versions are not fully pinned. A same-machine cache-assisted verification already produced the same size and ProductVersion but a different SHA-256, so the retained selected artifact—not a later build—is the exact proof authority.

A successful production upgrade proves the commit path but cannot prove rollback. Rollback observation still requires a separate plan-bound candidate or fixed fault fixture; an ad hoc corruption or caller-selected failure is not allowed.

The stacked rollback lane defines one private, feature-gated failing service binary. With no arguments—the SCM candidate-start path—it exits with fixed code `70`. Existing fixed `--provision` arguments delegate to the production provisioner. The binary is not packaged and exposes no new production verb, environment switch, or caller-selected failure mode. The selected fixture is pinned in the proof plan to source commit `2e7045bbf6de61b2a93f9df92cd2d1d9bd34d3b6`, ProductVersion `0.2.0-rc.2`, size `1557504`, and SHA-256 `356966442a77cbc3ad2370a2f70f1f8e1db4fc533e3802b20511383ae5848db0`. Its Cargo.lock SHA-256 is `693322cd3885ac4a9c823cd5facd13a9d995572e60f7b99af4cc49a39a5dd28b`, built with rustc/cargo `1.96.1`. `scripts/build-windows-lifecycle-service-fixture.ps1 -Fixture rollback-failing` accepts only that clean source commit and exact pinned output. The retained selected bytes are authoritative because cache-assisted Windows Rust builds are not byte-reproducible.

The elevated worker reads those plan-bound bytes through its retained file handle and invokes one private, feature-gated provisioner API. That API accepts bytes plus the expected old/new digests, writes only the fixed recovery leaf, runs the production prepare transaction, performs the installer-equivalent stable-image replacement, and calls the existing production candidate commit. The fixture creates one fixed-content, create-new execution marker beside the exact stable image before exiting; the provisioner requires that protected marker after candidate failure and removes it by exact digest. Success therefore proves SCM launched the pinned fixture, the production rollback restored and started the exact old digest, the journal returned to `prepared`, and journal-bound cleanup removed every staged, backup, journal, marker, and atomic residue leaf without changing the restored process generation. An unexpected candidate success is rolled back but fails the proof. Any post-prepare error performs phase-aware bounded recovery and reports the service unsettled unless the exact original digest is running with no transaction residue. No second rollback engine or caller-selected path is introduced.

No crate replaces the complete elevated-process, Job Object, service, named-pipe settlement, and evidence boundary. The controller therefore keeps its narrow `windows-sys` implementation. A private Windows UI Automation client may use the narrowly scoped `uiautomation` crate for bounded native Diagnostics inspection; that dependency would replace COM boilerplate only and would not own lifecycle authority.

## Required observations

Each stage records bounded private JSON with exact machine paths under the protected local evidence root. A distinct sanitized export replaces those paths with logical root identities and relative leaves before it can leave the machine.

Each stage records:

- service SCM state, PID, creation time, exact image path and digest, account, type, start mode, exit codes, recovery configuration, owner marker, and service DACL;
- pipe presence and server PID;
- ETW session identity, lease phase and generation, controller PID plus creation time, loss/health state, and owner/process locks;
- stable, staged, rollback, journal, atomic temporary, failure-marker, monitor, CLI, uninstaller, and install-root identities;
- Add/Remove Programs key, machine product key, HKCU autostart value, public desktop shortcut, common Start Menu shortcut and target;
- standard desktop and WebView process tree, token elevation, session, executable identity, and visible collector status;
- declared current-user settings, cache, and diagnostics hashes before and after uninstall.

Final absence requires no service or service registry key, pipe, ETW session, lease/locks, installer-owned Program Files leaf or directory, machine registration, shortcut, autostart value, or BatCave process tree. Declared current-user state is preserved unless the proof explicitly selects its separate deletion policy.

## Failure behavior

Any failed mutation stops after one fixed attempt. When the owned Job settles, the worker writes one stage-bound private packet containing the last settled machine snapshot, the terminal process-tree state captured before termination, the post-settlement machine snapshot, and executable revalidation truth. The private forced-crash primitive likewise records whether the exact service process, SCM state, and lifecycle marker settled. When process or service settlement cannot be proven, it writes only a clearly marked pre-settlement diagnostic packet; the parent then terminates and settles the outer worker Job before accepting any failure receipt. Evidence-write and process-settlement failures remain distinct. The parent accepts only the fixed failure leaf for the attempted stage, verifies its size and SHA-256 after worker settlement, and retains the verified file handle through the final response. A later parent settlement, artifact-revalidation, or receipt-verification failure is appended to the original structured worker failure instead of replacing its stage, cause, or receipt. The controller does not retry with a different command, controller, installer path, uninstaller form, service tool, or cleanup strategy.

One fixed restoration path may run only after the failed mutator and its owned process tree have settled and the resulting machine state remains trusted. Unproven settlement blocks further mutation. Restoration must preserve the original stage failure and report its own verified result separately; it is not permission to choose another command or cleanup strategy.

Evidence becomes acceptable only when the exact controller and installer hashes, request nonce/sequence, every stage result, complete process settlement, and final residue assertions are present. Source tests, package construction, and a zero exit from an outer NSIS launcher are not substitutes.
