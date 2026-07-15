# Linux native gate-pipeline contract

- Status: test-only downstream contract; not public or native release proof
- Date: 2026-07-14
- Parent architecture: [Rust-owned native artifact consumption authority](0003-private-native-artifact-consumption-authority.md)
- Transport dependency: [Linux real-package owned transport](0008-linux-package-owned-transport.md)
- Scope: issues #115 and #62

## Decision

Fix the Linux deb/AppImage operation order in a Rust integration-test crate before a production complete-operation entry exists. The pipeline accepts only a private `ConsumedArtifact` value created when an in-memory process-local fixture capability consumes and rehashes its owned bytes. It accepts no command, executable, arguments, environment, host path, status, callback, output, receipt, or evidence value.

Keep this contract test-only. ADR 0004 still prevents a production Linux adapter entry until Rust can independently verify the public release and selected bytes without trusting caller-supplied JavaScript state. The pipeline does not install a deb, stage or launch an AppImage, mutate user state, or execute a child process.

## Fixed operation sequence

Typestate transitions make this sequence the only complete path after owned-byte consumption:

1. deb install or AppImage stage
2. application launch
3. release identity
4. settings restart
5. permission-limited degradation
6. telemetry
7. application removal
8. owned process cleanup and settlement
9. user-state policy

Every gate produces one typed status and a fixed outcome code. A skipped or failed runtime gate blocks later runtime observations, but removal and owned cleanup still run. Removal residue, cleanup failure, and unconfirmed process settlement remain distinct. User-state work is blocked when process settlement is unconfirmed.

## Fixture and proof boundary

The clean fixture result means only that the fixed contract traversed every typestate stage. Hostile fixtures cover skipped launch, failed release identity, removal residue, cleanup failure, and unconfirmed settlement. They are private enum variants inside the integration-test crate, not caller-authored statuses.

Every result retains these non-claims:

- `public_artifact_verified: false`
- `native_proven: false`
- `release_evidence: null`
- inert fixture only
- package command not run
- deb checksum/source-attestation limitation or unexercised AppImage updater trust, as applicable

The serialized result contains fixed identifiers and status codes only. It cannot contain raw output, a machine path, environment data, or caller evidence. Even a clean fixture cannot produce a #98 packet because it did not use a selected public artifact or perform native operations.

## Verification

```sh
cargo test --manifest-path src/BatCave.App/src-tauri/Cargo.toml \
  --test linux_native_gate_pipeline
cargo clippy --manifest-path src/BatCave.App/src-tauri/Cargo.toml \
  --test linux_native_gate_pipeline -- -D warnings
bash scripts/validate-tauri.sh --skip-bundle
```

The follow-up [Linux owned complete-operation source contract](0010-linux-owned-complete-operation-source-contract.md) composes this typestate sequence with an opaque inert capability and fixed descriptor-transport selection while retaining artifact, process, and root authority through cleanup. It remains test-only and cannot prove public or native execution.

Parent issue #115 remains open for the production Rust-owned public-release verifier and exact public deb/AppImage install/stage, runtime, removal, settlement, sanitized evidence, and native-host proof.
