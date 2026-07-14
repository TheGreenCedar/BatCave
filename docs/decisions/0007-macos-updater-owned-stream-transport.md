# macOS updater owned-stream transport

- Status: accepted transport primitive; production entry and exact-artifact proof remain open
- Date: 2026-07-14
- Issue: [#114](https://github.com/TheGreenCedar/BatCave/issues/114)
- Architecture: [ADR 0003](0003-private-native-artifact-consumption-authority.md) and [ADR 0004](0004-rust-install-smoke-complete-operation-entry.md)

## Decision

Use a Rust-owned immutable compressed stream for the future macOS updater adapter. Do not pass the updater archive through the current caller-selected Python path or expose a private path, descriptor, reader, command, callback, or completion validator to JavaScript.

Unlike DMG attachment, updater archive extraction does not require another process to reopen the package. Rust can own the selected bytes, decode and preflight the complete tar stream, and materialize only validated entries into an authority-owned staging root. This preserves the `macos_updater_staging_only` limitation: extracting an app is not installation and does not prove an A-to-B update.

The production adapter remains blocked by ADR 0004. BatCave does not yet have a Rust-owned complete-operation entry that independently verifies the public release, checksums, attestations, and selected bytes. This decision therefore adds a non-production integration-test probe and aligns the source descriptor with the accepted future transport. It does not add unreachable production authority code or a serializable bridge.

## Probe contract

[`macos_updater_owned_stream_transport_spike.rs`](../../src/BatCave.App/src-tauri/tests/macos_updater_owned_stream_transport_spike.rs) is compiled only as a Rust integration test. It is not linked into the BatCave library, registered as a Tauri command, exposed as a CLI mode, or callable from JavaScript.

The probe:

- copies the exact expected fixture bytes into immutable Rust-owned storage before the original source can matter again;
- accepts one closed updater profile and rejects replay or completion from another authority;
- accepts exactly one gzip member, consumes through its validated trailer, and rejects trailing bytes or a second gzip member;
- enforces the same compressed, decompressed, member, path, prefix, file, and expanded-size ceilings as the release extractor, including the retained record and canonical-prefix string copies charged to the path-bookkeeping ceiling;
- preflights every archive entry before creating a staging root;
- rejects absolute paths, traversal, backslashes, invalid UTF-8, links, devices, extra roots, nested apps, duplicate paths, file/directory conflicts, and macOS case or Unicode-normalization collisions;
- decodes and fully consumes the immutable stream a second time, creates files with exclusive no-follow semantics on Unix, rechecks every header and file digest, and rejects extra staged entries;
- creates the private root with restrictive permissions in the create operation, removes it on materialization or verification failure, and reports a cleanup failure instead of hiding it;
- removes the complete successful staging root or reports retained cleanup state for a bounded retry; and
- returns only a sanitized fixture outcome with no path, raw archive member, command, trust assertion, native receipt, or evidence packet.

The production follow-up must preserve these properties behind the future Rust-owned public-release verifier. It must independently review dependency behavior, archive metadata handling, stage-root race resistance, trust and identity rechecks, process settlement, launch, removal, and residue before any native claim becomes reachable.

## DMG distinction

[ADR 0006](0006-macos-dmg-owned-byte-transport.md) remains unchanged. `hdiutil` could not consume an inherited `/dev/fd/N`, and a same-user-visible path cannot prove which bytes DiskImages opened. Updater stream extraction being viable does not make the DMG profile viable and does not authorize a path fallback.

## Verification

```sh
cargo test --manifest-path src/BatCave.App/src-tauri/Cargo.toml \
  --test macos_updater_owned_stream_transport_spike
node --test scripts/native-install-smoke-executor.test.mjs
```

The focused Rust suite covers valid staging, source replacement, wrong bytes, replay, cross-operation completion, hostile entry types, macOS path collisions, retained-prefix budgets, invalid gzip trailers, trailing bytes, second gzip members, materialization and verification cleanup failures, cleanup retention, cleanup retry, and absence of a production entry.

## Non-claims

- The fixture is local, unsigned, inert, and not a public BatCave updater artifact.
- No Developer ID, notarization, staple, Tauri updater signature, bundle identity, architecture, or version is verified.
- No app is installed, launched, restarted, updated, or observed under degraded permissions.
- No production adapter, native execution receipt, `native_proven` result, #98 packet, public A-to-B proof, or release acceptance is created.
