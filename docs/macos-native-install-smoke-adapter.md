# macOS native adapter source boundary

Issue #117 defines the source-only handoff between the #111 artifact capability and the exact native macOS work that remains in #114. It adds no package execution path.

`scripts/macos-native-install-smoke-adapter.mjs` accepts only a process-local install-smoke plan and the historical `artifact_owned_bytes_verified` receipt produced by #111. It derives one frozen descriptor for `macos:dmg` or `macos:macos_updater`; callers cannot provide a command, path, status, trust observation, cleanup assertion, or evidence field. A private `WeakMap` binds that descriptor to the exact plan object, its exact identity receipt, and the exact #111 receipt object. An equivalent plan, a second valid receipt for the same asset, or a copied descriptor cannot replay it.

The descriptor records the closed profile, fixed future tool identifiers, future resource ownership, plan timeouts, exact selected asset identity, and mandatory limitations. Tool identifiers are design constraints only. This slice does not invoke `hdiutil`, the Rust owned-stream extractor, `codesign`, `spctl`, `stapler`, `lipo`, `PlistBuddy`, `ditto`, or an application process.

## Exact current claim

The source receipt proves only that a valid macOS plan and the exact #111 verification receipt selected the same asset identity when the descriptor was created. It explicitly records:

- `live_capability_held: false`;
- `descriptor_only: true`;
- no package consumption or process execution;
- no Developer ID, notarization, staple, updater-signature, or contained-app trust verification;
- no launch, telemetry, settings, degradation, settlement, removal, or residue proof; and
- no `native_proven`, native execution receipt, or #98 release-evidence packet.

The native executor therefore keeps `preflight.package_trust` unsupported and every package/runtime/cleanup action blocked. It may mark `preflight.asset_rehash` passed because #111 re-read the private copy before the source descriptor was created; this is not proof that any future tool consumed that private copy.

## Profile boundary

| Profile               | Future package claim                     | Source descriptor                             | Required limitation          |
| --------------------- | ---------------------------------------- | --------------------------------------------- | ---------------------------- |
| macOS DMG             | mount, copy, and install an isolated app | `owned_dmg_mount_copy_required`                  | none                         |
| macOS updater archive | safely extract and stage an isolated app | `rust_owned_updater_archive_stream_required`     | `macos_updater_staging_only` |

The updater profile can never reinterpret staging as installation or a public A-to-B update. Both profiles remain blocked until a reviewed private process boundary consumes the still-live #111 capability and derives observations from settled native execution.

The transport decisions are now distinct. [ADR 0006](decisions/0006-macos-dmg-owned-byte-transport.md) records that `hdiutil` cannot consume the Rust-owned `/dev/fd/N` input and forbids a path fallback. [ADR 0007](decisions/0007-macos-updater-owned-stream-transport.md) records that updater archives can be preflighted and staged from Rust-owned immutable compressed bytes without a package path. ADR 0007 is a test-only transport proof; it does not create the production composition root that ADR 0004 still requires.

## Verification

Run the source contract with:

```sh
node --test scripts/native-install-smoke-executor.test.mjs
cargo test --manifest-path src/BatCave.App/src-tauri/Cargo.toml \
  --test macos_updater_owned_stream_transport_spike
```

The JavaScript suite creates process-local plans and #111 capabilities, rejects injected adapter arguments, copied descriptors, same-asset receipt substitution, and equivalent-plan replay, checks the two distinct macOS profiles, and confirms every source result remains skipped with null native/evidence receipts. The Rust probe copies fixture bytes into an immutable owned stream, consumes and validates one complete gzip member, rejects trailers, trailing data, second members, hostile tar entries, and macOS filesystem collisions before materialization, charges retained canonical-prefix copies to its path budget, rechecks every staged file, proves one-shot completion identity, and reports cleanup failures instead of hiding them. Existing validation and release workflows run both suites on Windows, Linux, and universal macOS.

The release extractor retains its separate traversal, link, collision, size-budget, and replacement coverage in `scripts/test-macos-updater-archive.sh`. Those fixtures and the Rust transport probe use local archives. Neither proves that a production adapter consumed the selected public updater archive.

## What closes #114

#114 still requires a Rust-owned public-release verifier and complete-operation entry, exact signed public universal DMG and updater artifacts, destination trust and identity rechecks, bounded termination with settled cleanup, launch and runtime gates, removal and residue proof, sanitized #98 evidence, and the explicit updater staging-only non-claim. The DMG transport also needs a safe primitive other than the rejected descriptor or path fallback. Hosted source tests cannot supply that evidence.
