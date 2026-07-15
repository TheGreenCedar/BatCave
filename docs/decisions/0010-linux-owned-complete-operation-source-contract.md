# Linux owned complete-operation source contract

- Status: test-only source contract; not public or native release proof
- Parent issue: #115
- Capability dependency: #111
- Transport dependency: [Linux real-package owned transport](0008-linux-package-owned-transport.md)
- Gate-order dependency: [Linux native gate pipeline](0009-linux-native-gate-pipeline.md)

## Decision

Add one Rust integration-test entry that accepts only an opaque, process-local owned capability and carries it through a private Linux operation authority. The authority chooses the deb or AppImage descriptor transport from a closed enum, fixes the nine required install/stage, runtime, removal, settlement, and user-state gates through typestate, and derives only a sanitized source-contract result.

This entry connects the accepted ownership, transport, and gate-order contracts without adding the production bridge rejected by [ADR 0004](0004-rust-install-smoke-complete-operation-entry.md). The capability contains inert fixture bytes. The entry has no caller command, executable, arguments, environment, path, status, output, callback, receipt, or evidence input. It is not linked from `src/lib.rs`, registered as a Tauri command, exposed through a CLI, or callable from JavaScript.

The Rust fixture reproduces #111's one-shot owned-byte and rehash boundary; it is not the JavaScript #111 capability crossing a process boundary. That brand still cannot cross into Rust as proof. A production successor must independently establish the public release and exact bytes inside the Rust composition root before this source composition can become an executable adapter.

## Closed ownership

The fixture capability rehashes its owned bytes before the transport authority exists. The complete operation then retains three private ownership tokens:

- the consumed artifact and its process-local seal;
- the fixed transport's process authority; and
- the private staging-root authority.

A clean, skipped, or failed operation releases those resources only after removal, process settlement, root cleanup, and user-state policy complete. A post-authority transport failure still runs removal and cleanup. A transport timeout with unconfirmed settlement, removal residue, or cleanup failure returns a private retained owner rather than a terminal result. That owner keeps all three authorities live and exposes only a fixed, argument-free cleanup retry. The retry settles removal/process/root state before the owner can disappear; dropping a retained result takes the same fixed fail-safe settlement path before releasing its tokens.

The typed sequence is:

1. deb install or AppImage stage binding
2. launch
3. release identity
4. settings restart
5. permission-limited degradation
6. telemetry
7. application removal and residue inspection
8. owned process cleanup and settlement
9. user-state policy

Transport or runtime failure blocks later runtime observations but cannot skip removal or cleanup. A skipped launch remains a distinct `skipped` status and disposition while still requiring removal and cleanup. Capability rehash failure occurs before transport construction and blocks every gate.

## Proof boundary

Every outcome records:

- `package_bytes_executed: false`;
- `public_artifact_verified: false`;
- `native_proven: false`; and
- `release_evidence: null`.

The deb result retains `deb_checksum_attestation_only`. The AppImage result retains `app_image_updater_trust_not_exercised`. A clean source-contract traversal proves only that the private types preserve the accepted composition and lifetime rules; it does not prove that `dpkg`, an AppImage runtime, a selected public artifact, or the BatCave payload ran.

## Verification

[`linux_owned_complete_operation.rs`](../../src/BatCave.App/src-tauri/tests/linux_owned_complete_operation.rs) covers both closed profiles, exact gate order, capability rehash failure, post-authority transport failure, timeout with retained process/root/artifact ownership, skipped launch, runtime failure with mandatory cleanup, residue recovery, cleanup-failure recovery, sanitized output, and production-entry absence.

```sh
cargo test --manifest-path src/BatCave.App/src-tauri/Cargo.toml \
  --test linux_owned_complete_operation
cargo clippy --manifest-path src/BatCave.App/src-tauri/Cargo.toml \
  --test linux_owned_complete_operation -- -D warnings
bash scripts/validate-tauri.sh --skip-bundle
```

Issue #115 remains open. Exact native proof still requires a Rust-owned public-release verifier, the selected public deb/AppImage bytes consumed by the fixed native operation, real install/stage and runtime behavior, native removal/settlement, and sanitized #98 evidence. Hosted and source tests cannot provide that proof.
