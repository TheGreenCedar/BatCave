# Linux native install-smoke adapter source boundary

Issue #116 adds the source-only boundary for the Linux deb and AppImage adapters. It registers the two built-in profiles with the native executor and proves the Linux process-group settlement contract. It does not execute package bytes, install a deb, stage or launch an AppImage, or emit native release evidence.

## Closed profiles

`scripts/linux-native-install-smoke-adapter.mjs` accepts only an already validated contract-only plan. The package identity must be exactly `linux:deb` or `linux:appimage`, and the plan's operation, install kind, trust basis, limitations, and 13 ordered gates must match the existing install-smoke platform contract.

The returned descriptor is process-local. A clone or reconstruction is not a registered adapter. The descriptor contains no command, environment, host path, callback, caller status, observation, native receipt, or evidence packet. The native executor creates and validates this descriptor itself; its public options remain limited to `verified_root`.

| Profile | Source contract | Trust boundary retained | Execution in this slice |
| --- | --- | --- | --- |
| Linux deb | install/remove gates remain mandatory | public checksum and source-bound attestation; `deb_checksum_attestation_only` remains required | none |
| Linux AppImage | stage/remove gates remain mandatory | Tauri updater trust remains required | none |

Registering the source descriptor changes only the explanation attached to the existing `unsupported` package-trust gate. The source result remains `skipped`, `native_execution_receipt` remains `null`, and `evidence_packet` remains `null`.

## Process settlement contract

The module contains one private Linux process supervisor. There is no exported generic spawn function. Its source test runs only fixed internal probes through the current absolute Node executable with tokenized arguments, `shell: false`, a minimal fixed environment, a private mode-700 temporary root, and a 4 KiB combined-output limit.

The probes cover:

- ordinary child exit;
- a stubborn descendant that survives its parent exit until the owned process group is terminated;
- a stubborn process tree that requires escalation from `SIGTERM` to `SIGKILL`; and
- output-limit termination.

A timeout or output limit is incomplete until both the direct child and its entire process group settle. Normal parent exit also waits for the group and terminates surviving descendants. Every wait is bounded. If hard-stop settlement cannot be confirmed, the result is `failed` with `cleanup: retained_unsettled`; the supervisor does not remove the private root while an owned group may still use it. A filesystem cleanup error is a distinct `cleanup: failed` result.

The settlement result is process-local and always records `package_bytes_executed: false`, `native_execution_receipt: null`, and `evidence_packet: null`. Probe output and host paths are never returned.

Run the focused contract with:

```sh
node --test scripts/linux-native-install-smoke-adapter.test.mjs
```

On non-Linux hosts the fixed process probes return `unsupported` without spawning or mutating anything; descriptor and forgery tests still run.

## Remaining work in #115

The [real-package owned-transport gate](decisions/0008-linux-package-owned-transport.md) checks locally built deb extraction and AppImage runtime execution through sealed inherited descriptors on hosted Linux. Its [owned-package payload-launch successor](decisions/0011-linux-owned-payload-launch.md) now carries those bytes into the packaged BatCave benchmark entry, validates the embedded version and optional source SHA, requires one advancing core-runtime sample, and settles every owned process and private root.

That successor still uses pull-request bundles. The deb payload runs from a private extraction rather than a package installation. The AppImage payload uses fixed extract-and-run staging rather than canonical staging or updater trust. Its benchmark scope is the core runtime host, not a desktop-window observation, and it keeps public/native/evidence claims false. Unconfirmed settlement retains the artifact, process/subreaper authority, and private root behind one fixed recovery/drop path instead of collapsing ownership into an ordinary error.

Parent issue #115 remains open. A later exact-public-artifact lane must independently re-establish the public release inside the future Rust-owned complete operation, make the fixed deb installer or AppImage stager consume those exact owned bytes, and complete package trust, launch, release identity, settings restart, degradation, telemetry, removal, process cleanup, and user-state gates.

Only that reviewed native path may create the internal native execution receipt and derive a sanitized #98 `release_evidence` packet. Source tests, fixed settlement probes, hosted validation, and local package fixtures are not Linux release proof.

The [test-only Linux gate-pipeline contract](decisions/0009-linux-native-gate-pipeline.md) now fixes the downstream deb/AppImage operation order after an inert process-local capability consumes its owned fixture bytes. Its typestate sequence covers launch, identity, settings restart, permission-limited degradation, telemetry, removal, process cleanup, and user-state policy, including skip, failure, residue, cleanup-failure, and unconfirmed-settlement cases. It accepts no caller command, environment, path, status, output, or evidence input.

That contract is deliberately not connected to production. It runs no package command, retains the deb checksum/source-attestation limitation and unexercised AppImage updater trust, and always records `public_artifact_verified: false`, `native_proven: false`, and `release_evidence: null`. ADR 0004 still requires a Rust-owned public-release verifier and complete-operation entry before the real adapter can use this sequence.

The [test-only complete-operation source contract](decisions/0010-linux-owned-complete-operation-source-contract.md) now composes the process-local owned capability, closed deb/AppImage transport selection, and that typestate sequence behind one Rust entry. Post-authority transport failure cannot skip cleanup, transport timeout with unconfirmed settlement retains all authority, and skipped launch remains distinct from failure. Its retained result keeps artifact, process, and private-root authority alive across timeout, residue, unsettled-process, and cleanup-failure paths until a fixed cleanup retry settles them.

This composition still uses inert bytes and runs no package command. It is absent from the production crate and cannot receive caller commands, environment, paths, statuses, output, callbacks, receipts, or evidence. It hard-wires `package_bytes_executed`, `public_artifact_verified`, and `native_proven` false and `release_evidence` null. Exact selected-public-artifact execution and native #98 evidence remain outstanding in #115.
