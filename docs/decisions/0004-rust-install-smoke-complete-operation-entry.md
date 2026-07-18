# Rust install-smoke complete-operation entry

- Status: superseded; the Rust-owned `batcave-install-smoke` entry now preserves the accepted boundary
- Date: 2026-07-14
- Decision issue: #138
- Architecture dependency: #130 / ADR 0003 at `b83953e41b8a667c2a3c4d4f8af0e1fa3d66c62c`
- Integration contained by that head: `090b29667a1e9d04f8e38b88d212b54717e87155`
- Successor: [Native install-smoke executor](../native-install-smoke-executor.md)

## Supersession

This decision remains the historical record for rejecting a serialized JavaScript-to-Rust authority bridge. The production `batcave-install-smoke` binary now avoids that bridge: it independently verifies the immutable public release, complete inventory, checksums, source-bound attestations, protected source identity, and selected bytes before private dispatch. The sections below describe the repository state and constraints at the time of this decision.

## Decision

Do not add a production Rust install-smoke composition root yet. The repository has no product-owned entry that can give Rust independently verified public-release identity and exact bytes without trusting ordinary JavaScript or exposing an authority protocol prohibited by [ADR 0003](0003-private-native-artifact-consumption-authority.md).

The current JavaScript source slice remains the only executable boundary. It continues to return only `skipped` or `failed`, with no native execution receipt or release-evidence packet. The isolated Rust authority prototype remains test-only until a successor reproduces its hostile cases behind a complete, independently verified operation.

This is the bounded fallback required by issue #138. Adding private Rust types that no production entry can reach would create dead authority code. Making them reachable through caller JSON, a verified directory, a digest, a generic helper, or a Tauri command would weaken #110 and ADR 0003.

## Current proof boundary

The public verifier and install-smoke plan use object identity as part of their proof:

1. `verifyPublicRelease` adds its receipt to a module-private JavaScript `WeakSet` only after the immutable-release, exact asset, checksum, and source-attestation checks finish.
2. `createInstallSmokePlan` creates a second module-private identity receipt bound to that exact in-process verification result, selected asset, release, platform, and observation.
3. A structured clone, JSON round trip, separate Node process, or Rust process retains the visible fields but loses both brands. The validators reject it.
4. `native-artifact-capability.mjs` then acquires a private exact-byte copy for the source-only JavaScript result. It intentionally exports no path, handle, reader, callback, or consumer.
5. `nativeExecutionReceipts` has no producer. No current path can derive `native_proven` or a #98 packet.

The focused executor suite now makes the process boundary explicit: structured-cloned and JSON-round-tripped plans fail before filesystem acquisition or adapter dispatch.

## Entry options considered

| Candidate | Decision | Boundary failure |
| --- | --- | --- |
| Serialize the current plan or receipts to Rust | Rejected | Serialization preserves caller-authored fields, not either process-local brand. |
| Pass `verified_root`, asset path, size, or digest to a Rust helper | Rejected | The caller chooses both the bytes and the assertion about those bytes; it reopens caller injection and verify-then-open. |
| Generic CLI, stdin, environment, socket, or named-pipe protocol | Rejected | These are ordinary-caller authority surfaces prohibited by ADR 0003. |
| Tauri command or hidden desktop CLI mode | Rejected | Ordinary WebView or local callers could invoke a release/install authority from the product runtime. |
| Node native addon, callback, exported handle, or public Rust authority API | Rejected | It exposes intermediate authority or returns it to the mutable JavaScript process. |
| Production Rust module with unit tests but no accepted entry | Rejected | It is unreachable production code and cannot replace the current source slice. |
| Dedicated Rust-owned complete operation | Viable future boundary | Rust must independently establish the public release, complete inventory, checksum, attestation, source identity, and exact bytes before private dispatch. That verifier and its narrow invocation do not exist today. |

The Windows elevation broker exception in ADR 0003 does not solve this entry. It begins after the Rust executor has established one closed Windows operation; it cannot authenticate a JavaScript plan or authorize Linux/macOS execution.

## Required successor contract

Implementation may resume only after a separately reviewed child defines and proves a Rust-owned public-release verifier and complete-operation request with all of these properties:

- The repository, source ref, signer workflow, release asset contract, and five platform profiles are compiled-in closed values.
- Caller input is limited to non-proof selectors needed to choose one public release and one closed profile. A caller cannot provide a directory, asset path, URL, size, digest, checksum result, attestation result, status, observation, command, environment, callback, receipt, or evidence.
- Rust independently reads the immutable public release, verifies its complete asset inventory, downloads anonymously into an internally created root, verifies `SHA256SUMS.txt`, verifies every source-bound attestation, and binds the selected exact bytes before private dispatch.
- The network and GitHub-attestation implementation has an explicit trust, dependency, credential, redirect, timeout, and offline-failure contract. It cannot inherit a caller-selected executable or mutable `PATH` as proof.
- Ordinary JavaScript may request only the complete operation. It cannot reach acquisition, byte ownership, adapter dispatch, process supervision, completion validation, cleanup, or result derivation.
- Until native platform work is independently proven, the result remains sanitized `skipped` or `failed`; the native receipt and #98 evidence path remain absent.
- Hostile tests cover replay, selector substitution, release/readback drift, asset replacement, links, mismatched checksums or attestations, timeout, settlement, cleanup, residue, cross-operation completion, and caller-authored proof fields.

Porting or replacing `verify-public-release.mjs` is a material release-control change. It must not be hidden inside a platform adapter or treated as a mechanical translation.

## Repository state preserved

- No production Rust install-smoke module, public API, Tauri command, helper mode, dedicated binary, native addon, or generic protocol is added.
- `scripts/native-artifact-capability.mjs` and `scripts/native-install-smoke-executor.mjs` remain authoritative for source-only behavior.
- Linux and macOS source descriptors remain non-consuming and non-proof.
- The Rust prototype from #130 remains isolated under `src-tauri/tests` and cannot be invoked by JavaScript or the desktop runtime.
- #110, #114, and #115 remain open for the complete executor and native platform evidence. This decision does not execute or prove a package on any platform.

## Verification

```sh
node --test scripts/native-install-smoke-executor.test.mjs
cargo test --manifest-path src/BatCave.App/src-tauri/Cargo.toml \
  --test native_artifact_consumption_authority_prototype
cargo clippy --manifest-path src/BatCave.App/src-tauri/Cargo.toml \
  --all-targets -- -D warnings
bash scripts/validate-tauri.sh --skip-bundle
```

The normal release-contract and validation jobs already run the executor and Rust suites on Windows, Linux, and Apple Silicon macOS. Passing them proves this source decision and the absence of a newly registered bridge; it does not prove public or native package execution.
