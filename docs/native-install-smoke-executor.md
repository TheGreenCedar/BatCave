# Native install-smoke executor boundary

Issue #111's JavaScript source slice owns the selected public artifact through an opaque, process-local capability. It closes the file-identity and path-replacement boundary for that process, and the Linux source descriptor from #116 and macOS source descriptor from #117 remain contract-only. This source slice cannot produce native proof or release evidence.

Issue #130 accepted the architecture for the missing handshake in [the Rust-owned native artifact consumption authority decision](decisions/0003-private-native-artifact-consumption-authority.md). Issue #138 then recorded why the JavaScript brands could not be transferred into Rust in [the complete-operation entry decision](decisions/0004-rust-install-smoke-complete-operation-entry.md). That remains the historical decision for the rejected bridge. The production `batcave-install-smoke` binary now avoids that bridge: it independently reads and verifies the public release in Rust, owns the selected bytes, and dispatches a sealed Linux artifact to a Linux-only handler. The current handler revalidates the authority and returns `skipped`; it does not execute the package or create evidence.

The implementation is split deliberately:

- `scripts/native-artifact-capability.mjs` acquires and owns the selected bytes;
- `scripts/native-install-smoke-executor.mjs` validates the closed result states and the future `release_evidence` derivation;
- `scripts/linux-native-install-smoke-adapter.mjs` registers the built-in deb/AppImage source profiles and proves fixed process-group settlement without executing package bytes;
- `scripts/macos-native-install-smoke-adapter.mjs` registers the DMG/updater source descriptors without executing a process; and
- `scripts/native-install-smoke-executor.test.mjs` exercises hostile path, receipt, caller-seam, and cleanup boundaries;
- `src/BatCave.App/src-tauri/src/bin/batcave-install-smoke/install_smoke_release.rs` independently verifies the public release and binds the selected bytes; and
- `src/BatCave.App/src-tauri/src/bin/batcave-install-smoke/install_smoke_linux.rs` is the closed Linux dispatch target.

## Artifact ownership

Acquisition accepts only a process-local plan created from the public verifier receipt. Fixtures fail before filesystem access. The selected asset must be a regular direct child of an absolute, non-link verified root.

The capability inspects the root with `lstat`, resolves it, and compares both device and inode before it inspects the candidate. It rechecks that root identity and containment during acquisition. The candidate is opened with no-follow semantics where the host provides them, and the opened device and inode must match the inspected file. The capability then copies and hashes the exact expected byte count through that open handle into a newly created mode-600 private file. The private copy is rehashed through its owned handle, changed to mode 400, and retained behind a `WeakMap`; neither its path nor its handle appears in the public capability.

Acquisition rechecks the original path identity and containment after the copy. A replaced path, link, size mismatch, digest mismatch, truncated read, extra byte, or containment change fails without returning a capability. Later replacement of the public path cannot change the private bytes already owned by the capability.

## Owned-byte verification receipt

The capability exposes no reader, handle, path, callback, command, or adapter interface. `verifyOwnedNativeArtifactCapability` accepts only the branded capability and internally re-reads the exact expected byte count through the private handle. Any extra callback, reader, adapter, or options argument fails before the private bytes are touched. Verification is process-local and single-use; concurrent or repeated calls fail closed.

A successful verification returns a process-local `artifact_owned_bytes_verified` receipt. That receipt proves only that the entire private copy was re-read once and remained digest-stable. It does **not** prove that an installer, stager, launcher, or operating-system package service consumed those bytes.

Cleanup always attempts every available source/owned handle closure and private-root removal. Acquisition preserves its primary failure together with any cleanup failures in an aggregate. If cleanup fails, the source-slice result records `cleanup.owned_runtime_cleanup` as failed and emits no evidence packet; an acquisition failure can therefore mark both its rehash and cleanup gates without exposing raw errors.

## Current invocation and result

The JavaScript source-slice entry point remains:

```js
const result = await runNativeInstallSmokeSourceSlice(plan, {
  verified_root: absoluteVerifiedDownloadDirectory,
});
```

With valid selected bytes it returns `skipped`, marks `preflight.package_trust` unsupported, blocks native actions, and returns `evidence_packet: null`. Artifact acquisition or cleanup failure returns a derived `failed` disposition and still returns no packet. Callers cannot provide an adapter, command, disposition, or native receipt.

A structured clone, JSON document, separate Node process, or Rust process cannot preserve the JavaScript verifier or plan brand. The Rust `batcave-install-smoke` entry therefore accepts only a release tag and one closed package profile. It independently verifies the immutable public release, complete asset inventory, checksums, build and release attestations, exact protected source identity, and selected bytes. On Linux it moves the selected bytes into a sealed private descriptor, binds the release and asset identity, and revalidates both immediately before dispatch. Accepting the visible JavaScript plan fields or its `verified_root` would still violate the proof boundary.

All current Rust profiles return a sanitized `skipped` result after verification and owned cleanup. Linux reaches the closed Linux handler first; Windows and macOS stop before a platform adapter. No current profile produces a native execution receipt or release-evidence packet.

Run the capability and executor contract suite with:

```sh
node --test scripts/native-install-smoke-executor.test.mjs
```

## Closed platform command registry

| Package profile | Native operation required later | Current production path |
| --- | --- | --- |
| Windows NSIS | install and uninstall | Rust public verification and owned cleanup; no platform dispatch |
| Linux deb | install and remove | Rust public verification, sealed-byte handoff, and Linux revalidation; no package command |
| Linux AppImage | stage, launch, and remove | Rust public verification, sealed-byte handoff, and Linux revalidation; no package command |
| macOS DMG | mount, copy, launch, and remove | Rust public verification and owned cleanup; no native dispatch; JavaScript descriptor remains contract-only |
| macOS updater archive | extract, stage, launch, and remove | Rust public verification and owned cleanup; no native dispatch; JavaScript staging descriptor remains contract-only |

There is no general command runner, shell-string surface, public handle accessor, or injected callback that can create a native execution receipt. The JavaScript [Linux source boundary](linux-native-install-smoke-adapter.md) runs fixed internal settlement probes only. The production Rust Linux handler receives and revalidates exact sealed bytes but never executes the selected package.

The [macOS source boundary](macos-native-install-smoke-adapter.md) binds the exact verified asset identity to a frozen future profile. Its fixed tool IDs and owned-resource list are descriptors, not observations. The receipt explicitly records that no live capability is held and no package, process, trust, settlement, cleanup, or evidence action occurred.

## Remaining native adapter work

The Rust-owned public verifier, production entry, and sealed Linux handoff now exist. The remaining adapter must make that private owned artifact the install or staging input itself; draining or rehashing it and then executing an unrelated path is insufficient. The closed adapter must also own package trust, exact argument arrays, minimal environment, timeouts and settled process-tree termination, runtime identity and settings probes, degradation and telemetry observations, application removal, residue checks, and user-state policy.

Only that internal adapter may mint the still-unreachable native execution receipt. After every ordered gate passes, the executor can derive `native_proven` and construct the sanitized #98 packet from its own receipt and observations. Timeout, partial settlement, unsupported operation, command failure, cleanup failure, and residue must remain explicit gate outcomes; all prevent a release-proof claim.

The later release lane may serialize and attach the already-validated packet. It must not accept caller-authored evidence or reinterpret this source-slice receipt as platform acceptance.
