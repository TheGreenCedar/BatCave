# Native install-smoke executor boundary

Issue #111's native executor source slice owns the selected public artifact through an opaque, process-local capability. It closes the file-identity and path-replacement boundary before parent issue #110 can add a platform adapter. No platform adapter or installer command is registered yet, so this slice cannot produce native proof or release evidence.

The implementation is split deliberately:

- `scripts/native-artifact-capability.mjs` acquires and owns the selected bytes;
- `scripts/native-install-smoke-executor.mjs` validates the closed result states and the future `release_evidence` derivation; and
- `scripts/native-install-smoke-executor.test.mjs` exercises hostile path, receipt, caller-seam, and cleanup boundaries.

## Artifact ownership

Acquisition accepts only a process-local plan created from the public verifier receipt. Fixtures fail before filesystem access. The selected asset must be a regular direct child of an absolute, non-link verified root.

The capability inspects the root with `lstat`, resolves it, and compares both device and inode before it inspects the candidate. It rechecks that root identity and containment during acquisition. The candidate is opened with no-follow semantics where the host provides them, and the opened device and inode must match the inspected file. The capability then copies and hashes the exact expected byte count through that open handle into a newly created mode-600 private file. The private copy is rehashed through its owned handle, changed to mode 400, and retained behind a `WeakMap`; neither its path nor its handle appears in the public capability.

Acquisition rechecks the original path identity and containment after the copy. A replaced path, link, size mismatch, digest mismatch, truncated read, extra byte, or containment change fails without returning a capability. Later replacement of the public path cannot change the private bytes already owned by the capability.

## Owned-byte verification receipt

The capability exposes no reader, handle, path, callback, command, or adapter interface. `verifyOwnedNativeArtifactCapability` accepts only the branded capability and internally re-reads the exact expected byte count through the private handle. Any extra callback, reader, adapter, or options argument fails before the private bytes are touched. Verification is process-local and single-use; concurrent or repeated calls fail closed.

A successful verification returns a process-local `artifact_owned_bytes_verified` receipt. That receipt proves only that the entire private copy was re-read once and remained digest-stable. It does **not** prove that an installer, stager, launcher, or operating-system package service consumed those bytes.

Cleanup always attempts every available source/owned handle closure and private-root removal. Acquisition preserves its primary failure together with any cleanup failures in an aggregate. If cleanup fails, the source-slice result records `cleanup.owned_runtime_cleanup` as failed and emits no evidence packet; an acquisition failure can therefore mark both its rehash and cleanup gates without exposing raw errors.

## Current invocation and result

The only executable entry point is the source-slice API:

```js
const result = await runNativeInstallSmokeSourceSlice(plan, {
  verified_root: absoluteVerifiedDownloadDirectory,
});
```

With valid selected bytes it returns `skipped`, marks `preflight.package_trust` unsupported, blocks native actions, and returns `evidence_packet: null`. Artifact acquisition or cleanup failure returns a derived `failed` disposition and still returns no packet. Callers cannot provide an adapter, command, disposition, or native receipt.

Run the capability and executor contract suite with:

```sh
node --test scripts/native-install-smoke-executor.test.mjs
```

## Closed platform command registry

| Package profile | Native operation required later | Registered command now |
| --- | --- | --- |
| Windows NSIS | install and uninstall | none |
| Linux deb | install and remove | none |
| Linux AppImage | stage, launch, and remove | none |
| macOS DMG | mount, copy, launch, and remove | none |
| macOS updater archive | extract, stage, launch, and remove | none |

There is no general command runner, shell-string surface, public handle accessor, or injected callback that can create a native execution receipt.

## Remaining native adapter work

A later reviewed adapter must consume the private owned capability as the install or staging input itself. Draining or rehashing the capability and then executing an unrelated path is insufficient. The closed adapter must also own package trust, exact argument arrays, minimal environment, timeouts and settled process-tree termination, runtime identity and settings probes, degradation and telemetry observations, application removal, residue checks, and user-state policy.

Only that internal adapter may mint the still-unreachable native execution receipt. After every ordered gate passes, the executor can derive `native_proven` and construct the sanitized #98 packet from its own receipt and observations. Timeout, partial settlement, unsupported operation, command failure, cleanup failure, and residue must remain explicit gate outcomes; all prevent a release-proof claim.

The later release lane may serialize and attach the already-validated packet. It must not accept caller-authored evidence or reinterpret this source-slice receipt as platform acceptance.
