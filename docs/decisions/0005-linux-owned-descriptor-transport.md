# Linux Rust-owned descriptor transport

- Status: accepted transport contract; not accepted as native package proof
- Date: 2026-07-14
- Parent architecture: [ADR 0003](0003-private-native-artifact-consumption-authority.md)
- Scope: issues #139, #115, #130, and #62

## Decision

A sealed anonymous memory file, reopened read-only and inherited only by a fixed child, is the Linux byte transport for the Rust-owned authority. A fixed child that needs a filename-shaped input may use its private `/proc/self/fd/N` alias. The descriptor number and alias remain inside the Rust supervisor and child.

There is no temporary-file fallback. If `memfd_create`, sealing, read-only reopening, descriptor inheritance, process-group ownership, subreaping, or required `/proc` behavior is unavailable, the transport is unsupported.

## Closed transport

1. Rust verifies the selected regular non-link source against its closed size and SHA-256 binding.
2. Rust copies the bytes into `memfd_create(MFD_ALLOW_SEALING)` and applies write, grow, shrink, and seal seals.
3. Rust reopens the memory object through its private `/proc/self/fd/N` view as `O_RDONLY`, confirms identity, access mode, and seals, then drops the writable descriptor.
4. A fixed child receives the object at a fixed inherited descriptor. No caller provides a descriptor, path, executable, arguments, environment, callback, status, or completion.
5. Rust owns the process group, deadline, descendant settlement, output bounds, and cleanup. An unresolved operation retains the descriptor and process authority for bounded retry.
6. The sanitized outcome contains no descriptor, private path, generic command surface, native receipt, or evidence packet.

The production private verifier uses this authority for selected Linux bytes and currently returns `skipped` after revalidation. The hosted [`linux_package_owned_transport.rs`](../../src/BatCave.App/src-tauri/tests/linux_package_owned_transport.rs) gate exercises fixed deb extraction and AppImage payload launch against locally built bundles without claiming public native installation.

## Verification

```sh
cargo test --manifest-path src/BatCave.App/src-tauri/Cargo.toml \
  --bin batcave-install-smoke --features private-release-verifier
cargo test --manifest-path src/BatCave.App/src-tauri/Cargo.toml \
  --test linux_package_owned_transport
```

The verifier suite covers exact sealing, read-only reopening, stable descriptor offset, one-shot operation ownership, replay rejection, release and asset drift, and sanitized skipped results. The package-transport suite covers fixed locally built package consumers, process settlement, retained cleanup authority, hostile output, and cross-platform unsupported behavior.

These checks do not prove public acquisition, deb installation, canonical AppImage staging, package trust, UI behavior, settings restart, removal, user-state policy, native proof, or release evidence. Those remain issue #115 work.
