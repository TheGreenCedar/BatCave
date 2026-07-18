# Rust install-smoke complete-operation entry

- Status: superseded by the private `batcave-install-smoke` verifier
- Date: 2026-07-14
- Decision issue: #138
- Architecture: [ADR 0003](0003-private-native-artifact-consumption-authority.md)

## Historical decision

Do not bridge a process-local JavaScript verification object into Rust by serializing its visible fields. A structured clone, JSON document, verified directory, path, digest, environment variable, generic helper, Tauri command, or native addon would carry caller-authored assertions rather than the original authority.

At the time of this decision the repository had no Rust-owned entry that independently established the public release and exact selected bytes. Adding unreachable private Rust types would have created dead authority code; exposing them through a generic protocol would have weakened ADR 0003.

## Successor

The feature-gated [`batcave-install-smoke`](../../src/BatCave.App/src-tauri/src/bin/batcave-install-smoke.rs) binary supersedes that gap without a JavaScript bridge. Caller input is limited to a release tag and one closed profile. Rust independently verifies the immutable release, complete inventory, checksums, source-bound attestations, protected source identity, and selected bytes before private dispatch.

The verifier is not linked into the desktop library, registered as a Tauri command, or exposed as a generic path-taking helper. Linux currently returns `skipped` after descriptor revalidation, and the macOS updater profile emits only a staging observation. Neither path can mint native proof or a release-evidence packet.

## Verification

```sh
cargo test --manifest-path src/BatCave.App/src-tauri/Cargo.toml \
  --bin batcave-install-smoke --features private-release-verifier
cargo clippy --manifest-path src/BatCave.App/src-tauri/Cargo.toml \
  --bin batcave-install-smoke --tests --features private-release-verifier -- -D warnings
```

Passing these tests proves the closed Rust entry and its fail-closed source contracts. It does not prove native package installation, launch, removal, or release evidence.
