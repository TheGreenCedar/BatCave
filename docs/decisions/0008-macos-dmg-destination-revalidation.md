# macOS DMG consumed-destination revalidation

- Status: accepted source contract; exact public execution remains open
- Date: 2026-07-14
- Issue: [#114](https://github.com/TheGreenCedar/BatCave/issues/114)
- Transport decision: [ADR 0006](0006-macos-dmg-owned-byte-transport.md)
- Production-entry decision: [ADR 0004](0004-rust-install-smoke-complete-operation-entry.md)

## Decision

A future closed macOS DMG adapter must rederive bundle identity, version, universal architectures, signature integrity, Developer ID authority, notarization, and staple status from the copied app at the isolated consumed destination. A source-side observation, mounted-app observation, caller assertion, or evidence field cannot satisfy a destination gate.

Every gate is mandatory. Failure, timeout, substitution, an unsupported tool result, or trust drift rejects the operation. The future adapter may derive the three contained-app signature roles used by the #98 evidence contract only after all destination gates and the rest of #114 settle successfully.

This decision does not revise ADR 0006. `hdiutil` still cannot consume the Rust-owned descriptor, and a private filesystem path still has an unclosed same-user replace/open race. The fixed system tools also inspect the destination sequentially, so the fixture cannot bind all seven observations to one immutable copied tree; a same-user replacement between gates remains open. The local fixture therefore exercises the downstream mount/copy/revalidation lifecycle but always records both `exact_transport_proven: false` and `destination_binding_proven: false`. It cannot create a native execution receipt or evidence packet even if a synthetic gate observation is all true.

## Fixture contract

[`macos_dmg_destination_gate_spike.rs`](../../src/BatCave.App/src-tauri/tests/macos_dmg_destination_gate_spike.rs) is a macOS-only Rust integration test. It is not linked into the production library, registered as a Tauri command, exposed through a helper or CLI mode, or callable from JavaScript.

The fixture:

- builds an inert universal local app, ad-hoc signs it, places it in a local read-only DMG, and copies captured image bytes into an authority-owned root;
- runs fixed `hdiutil` and `ditto` operations in owned process groups with deadlines, bounded output, termination, settlement, detach, and zero-residue checks;
- binds the open image descriptor and pathname by device, inode, size, and digest before and after attachment, while retaining the ADR 0006 same-user race non-claim;
- copies exactly one fixed app into an isolated destination and compares a no-symlink file-tree digest before running trust commands;
- rederives the bundle ID, version, `arm64` plus `x86_64` architectures, code-signature integrity, Developer ID authority, notarization, and staple from that destination with fixed system tools;
- rejects image substitution before attachment, deterministic substitution after preflight, invalid image mount, destination symlink substitution, bundle drift, executable trust drift, mount/copy timeout, and cleanup failure;
- injects a mid-revalidation executable substitution to prove earlier identity/version/architecture observations are not a destination-binding claim; and
- retains post-spawn settlement and cleanup failures under process/root authority until bounded retry or drop recovery succeeds, while reporting no path, command output, signature text, receipt, or evidence.

The inert fixture is intentionally ad-hoc signed. Its identity, version, architecture, and signature-integrity hooks pass, while Developer ID, notarization, and staple fail. That proves the required hook ordering and fail-closed result without fabricating Apple trust.

## Source descriptor

The closed macOS source descriptor records a shared `consumed_destination_only` revalidation contract for both DMG installation and updater staging. It lists the seven mandatory gate IDs, the universal architecture set, and the three #98 contained-app signature roles. The descriptor remains process-local and explicitly says a source fixture can neither prove destination binding nor mint proof.

For DMG, copying into an authority-owned isolated destination is an install-smoke staging action, not a system install. No app is placed in `/Applications`, launched, restarted, granted permissions, observed for telemetry, removed from a real installation location, or used to create state-policy evidence.

## Remaining real-artifact gates

#114 remains open until one closed Rust-owned complete operation:

1. independently verifies the immutable public release, full asset inventory, checksum, attestations, source identity, and selected universal DMG bytes;
2. gives DiskImages a transport that proves those exact owned bytes were consumed without the rejected descriptor or path fallback;
3. copies into the isolated destination and passes all seven destination gates against the real app;
4. settles launch, release identity, settings restart, permission-limited degradation, telemetry, removal, process cleanup, and user-state policy; and
5. emits a sanitized #98 packet derived only from the settled operation.

Hosted/source tests and this local unsigned fixture cannot satisfy those gates.

## Verification

```sh
cargo test --manifest-path src/BatCave.App/src-tauri/Cargo.toml \
  --test macos_dmg_destination_gate_spike
node --test scripts/native-install-smoke-executor.test.mjs
bash scripts/validate-tauri.sh --skip-bundle
```

## Non-claims

- No exact public artifact is downloaded, mounted, installed, launched, or removed.
- No Developer ID, notarization, staple, release identity, runtime behavior, or public A-to-B update is proven.
- No production adapter, safe DMG transport, native receipt, `native_proven` result, or #98 evidence packet is created.
- Sequential fixture gates do not prove that every observation came from one immutable destination tree.
- The updater archive remains staging-only under ADR 0007.
