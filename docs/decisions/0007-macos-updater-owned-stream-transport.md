# macOS updater owned-stream transport

- Status: superseded by the production private release verifier; staging-only non-claim retained
- Date: 2026-07-14
- Issue: [#114](https://github.com/TheGreenCedar/BatCave/issues/114)
- Architecture: [ADR 0003](0003-private-native-artifact-consumption-authority.md) and [ADR 0004](0004-rust-install-smoke-complete-operation-entry.md)

## Decision

Use a Rust-owned immutable compressed stream for the future macOS updater adapter. Do not pass the updater archive through the current caller-selected Python path or expose a private path, descriptor, reader, command, callback, or completion validator to JavaScript.

Unlike DMG attachment, updater archive extraction does not require another process to reopen the package. Rust can own the selected bytes, decode and preflight the complete tar stream, and materialize only validated entries into an authority-owned staging root. This preserves the `macos_updater_staging_only` limitation: extracting an app is not installation and does not prove an A-to-B update.

The original decision was blocked by ADR 0004 and introduced a non-production integration-test probe. That blocker is now superseded: `batcave-install-smoke` owns public-release verification and dispatches a `VerifiedMacOsUpdaterArtifact` directly into `install_smoke_macos_updater.rs`. The predecessor probe was retired after its unique hostile-archive, budget, stream-integrity, and cleanup-retention cases moved into the production module tests.

## Successor contract

[`install_smoke_macos_updater.rs`](../../src/BatCave.App/src-tauri/src/bin/batcave-install-smoke/install_smoke_macos_updater.rs) is compiled only behind the private `batcave-install-smoke` release-verifier feature. It is not linked into the desktop library, registered as a Tauri command, or callable from JavaScript.

The successor:

- receives an already-bound `VerifiedMacOsUpdaterArtifact`, immediately revalidates its release, asset, signature-asset, size, and digest identity, and never accepts a package path;
- is reachable only after the private release verifier closes the updater profile and consumes the selected artifact through its one-shot, operation-bound ownership gate;
- accepts exactly one gzip member, consumes through its validated trailer, requires the complete tar tail after the first end marker to be zero block-aligned padding, and rejects hidden entries, trailing bytes, or a second gzip member;
- enforces the same compressed, decompressed, member, path, prefix, file, and expanded-size ceilings as the release extractor, including retained record and canonical-prefix `Vec<String>` storage and string capacities charged to the path-bookkeeping ceiling;
- preflights every archive entry before creating a staging root;
- rejects absolute paths, traversal, backslashes, invalid UTF-8, links, devices, extra roots, nested apps, duplicate paths, file/directory conflicts, and macOS case or Unicode-normalization collisions;
- decodes and fully consumes the immutable stream a second time, creates files with exclusive no-follow semantics on Unix, rechecks every header and file digest, and rejects extra staged entries;
- creates the private root with restrictive permissions in the create operation, preserves both primary and cleanup failures, and retains cleanup authority inside the Rust error through bounded cleanup retries;
- removes the complete successful staging root before emitting an observation, while any cleanup failure prevents success and maps to a sanitized failure; and
- returns only a sanitized exact-public staging observation with no path, raw archive member, command, native receipt, or release-evidence packet.

The private verifier preserves these properties behind the Rust-owned public-release boundary. Staging remains narrower than installation: process settlement, launch, removal, settings preservation, and updater A-to-B behavior still require separate exact-public proof.

## DMG distinction

[ADR 0006](0006-macos-dmg-owned-byte-transport.md) remains unchanged. `hdiutil` could not consume an inherited `/dev/fd/N`, and a same-user-visible path cannot prove which bytes DiskImages opened. Updater stream extraction being viable does not make the DMG profile viable and does not authorize a path fallback.

## Verification

```sh
cargo test --manifest-path src/BatCave.App/src-tauri/Cargo.toml \
  --bin batcave-install-smoke --features private-release-verifier
node --test scripts/native-install-smoke-executor.test.mjs
```

The Rust binary suite covers valid staging, artifact ownership and replay rejection, hostile entry types, macOS path collisions, retained `String` and `Vec` prefix budgets, invalid gzip trailers, hidden entries after an end marker, trailing bytes, second gzip members, persistent materialization and verification cleanup failures, cleanup retention, and bounded cleanup retry.

## Non-claims

- The unit fixtures are local and inert; they do not themselves prove a public BatCave updater artifact.
- The exact-public path verifies the Tauri updater signature and release identity, but it does not reverify the staged app's Developer ID, notarization, staple, bundle identity, architecture, or version.
- No app is installed, launched, restarted, updated, or observed under degraded permissions.
- The private verifier emits only its bounded staging observation. It does not create an installed-app, launch, removal, settings-preservation, public A-to-B, or stable-release acceptance claim.
