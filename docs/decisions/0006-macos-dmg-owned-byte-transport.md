# macOS DMG owned-byte transport

- Status: unsupported for `/dev/fd`; no path fallback accepted
- Date: 2026-07-14
- Tested host: macOS 26.5.2 (25F84), arm64, system `/usr/bin/hdiutil`
- Issue: [#140](https://github.com/TheGreenCedar/BatCave/issues/140)
- Parent: [#114](https://github.com/TheGreenCedar/BatCave/issues/114)
- Authority decision: [ADR 0003](0003-private-native-artifact-consumption-authority.md)

## Decision

Do not implement the macOS DMG adapter by passing an inherited Rust-owned descriptor to `hdiutil attach` as `/dev/fd/N`.

On the tested macOS host, Rust kept a valid local fixture DMG open, cleared `FD_CLOEXEC` only in the fixed `hdiutil` child, unlinked and replaced the original source path, and invoked the fixed read-only attach operation with `/dev/fd/N`. `hdiutil` settled with `attach failed - Bad file descriptor`. It created no mount. The same result occurred in the smaller direct host experiment before the Rust probe was written.

The failure means the descriptor does not survive the complete DiskImages attachment path. The authority still owns valid bytes, but the fixed platform operation cannot consume them through that descriptor. A pre-attach rehash would not repair this boundary because `hdiutil` would still consume a different input.

This is `unsupported`, not `native_proven`, and not a reason to expose a path.

## Probe contract

[`macos_dmg_owned_byte_transport_spike.rs`](../../src/BatCave.App/src-tauri/tests/macos_dmg_owned_byte_transport_spike.rs) is a macOS-only, non-installing Rust integration-test crate. It is not linked into the production library, registered as a Tauri command, exposed through a CLI mode, or callable from JavaScript.

The probe:

- creates a small unsigned local read-only DMG containing one inert marker file;
- opens the DMG in Rust, hashes it, unlinks its source name, and writes different bytes at that old name;
- makes only the exact owned descriptor inheritable in the fixed child and passes only `/dev/fd/N` to `hdiutil attach`;
- runs every fixed `hdiutil` child in an owned process group with a deadline, `SIGTERM`/`SIGKILL` settlement, bounded output, and no stdin;
- owns the private fixture root and mount point, attempts bounded detach before deletion, and checks owned process-group, mount, and temporary-root residue;
- distinguishes unsupported descriptor consumption, failed attachment with a known-invalid fixture, timeout, failed detach, early close, replay, cleanup failure, and cleanup retry; and
- returns only a sanitized test outcome with no path, descriptor, command, raw output, receipt, or evidence.

The cleanup-failure case deliberately retains its private fixture root and reports `retained_cleanup_failed`; a bounded retry then removes it. Every case that starts `hdiutil` settles the owned process group. Every case other than the deliberate retention leaves no mount or temporary residue.

On Windows and Linux, the integration test reports the host as unsupported without running a process or creating a file.

## Private-path evaluation

`hdiutil` does accept ordinary filesystem paths, but a random name and mode-`0700` parent do not isolate a file from another process running as the same user. That same-user process can enumerate, rename, replace, or remove the path. Holding a Rust descriptor and rehashing before attachment does not prove that DiskImages later opened the same bytes.

This spike therefore does not adopt a private-path design. A later issue may evaluate a macOS primitive that binds DiskImages to immutable authority-owned storage, but it must prove the exact bytes consumed across open, attach, mount, detach, and cleanup. Until that exists, the DMG native adapter remains unsupported. It must not silently fall back to a caller-visible or merely randomized path.

## Verification

Focused macOS probe:

```sh
cargo test --manifest-path src/BatCave.App/src-tauri/Cargo.toml --test macos_dmg_owned_byte_transport_spike
```

Cross-platform compilation and the normal Rust suite keep the unsupported-host result exercised on Windows and Linux. All-target Clippy must also pass with warnings denied.

## Non-claims

- No app is installed, copied into Applications, launched, updated, or removed.
- The fixture is local, inert, unsigned, and not a public BatCave artifact.
- No Developer ID, notarization, staple, updater signature, package identity, runtime telemetry, settings, accessibility, or cleanup policy is verified.
- The probe creates no native execution receipt, release-evidence packet, or #114 acceptance evidence.
- The result says nothing about Windows NSIS, Linux deb/AppImage, or macOS updater-archive transport.
