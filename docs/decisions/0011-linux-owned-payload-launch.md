# Linux owned-package payload launch

- Status: hosted source gate; not public or native release proof
- Date: 2026-07-14
- Parent issue: #115
- Transport dependency: [Linux real-package owned transport](0008-linux-package-owned-transport.md)
- Lifetime dependency: [Linux owned complete-operation source contract](0010-linux-owned-complete-operation-source-contract.md)

## Decision

Extend the existing Rust integration-test transport so locally built deb and AppImage bytes reach the packaged BatCave payload. The operation keeps the sealed, read-only `memfd` alive, uses only fixed package and payload commands, parses one bounded benchmark-v4 observation, revalidates the owned artifact after the payload settles, and removes every private staging directory.

This is still a source gate. The package bytes come from the exact checkout under hosted validation, not from an anonymously downloaded and independently verified public release. The benchmark reports `core_runtime_host_only` and `whole_app_measured: false`; it observes the packaged runtime engine, release identity, protocol serialization, and advancing samples without claiming a desktop-window launch or the complete #115 lifecycle.

## Fixed payload operations

The operation accepts no caller path, command, executable, argument, environment, digest, status, output, callback, receipt, or evidence.

- deb: the existing fixed `/usr/bin/dpkg-deb --extract` consumer writes into a mode-700 private root. The operation derives `usr/bin/batcave-monitor` from that root, rejects a linked or non-regular executable and any canonical-path traversal, then launches its compiled-in benchmark command. It does not run `dpkg --install`, maintainer scripts, package locks, or the package database.
- AppImage: the owned descriptor is executed through its child-private `/proc/self/fd/198` alias with fixed `--appimage-extract-and-run` and benchmark arguments. `HOME`, `XDG_DATA_HOME`, and `TMPDIR` are internally created mode-700 directories under the private root. This launches the packaged payload without FUSE, but it is not canonical AppImage staging and does not exercise updater trust.

Both commands use Linux as the fixed platform, the compiled architecture, one fixed machine class and workload profile, zero warmup ticks, one measured tick, no delay, and one repeat. No performance threshold or caller baseline participates in this observation.

## Closed observation

The parser accepts at most 4 KiB and exactly one JSON object. Summary, release-identity, and repeat keys must be exact. It requires:

- benchmark format 4 and the fixed core-runtime metadata;
- the compiled app version and optional compiled source SHA;
- Linux plus the compiled architecture;
- the fixed benchmark configuration;
- one repeat with advancing samples and a matching passing sample-quality summary; and
- the expected no-baseline, no-speed-ratio result.

Extra caller-like status, missing or duplicate objects, oversized output, wrong release or platform identity, malformed metrics, non-advancing samples, and contradictory summary state fail closed. The parsed observation is process-local and is not a native execution receipt.

## Ownership and proof boundary

The existing supervisor retains the artifact descriptor, process group, adopted descendants, output pipes, and private root until the payload settles. Output remains nonblocking and bounded. Timeout, output overflow, a surviving descendant, or unsettled pipes fail after bounded termination and reaping. The exact owned bytes are rehashed after each package or payload consumer, and the private root is removed only after process settlement.

Every outcome remains:

- `source_kind: locally_built_bundle`;
- `public_artifact_verified: false`;
- `native_proven: false`; and
- `release_evidence_emitted: false`.

The gate establishes a locally built packaged payload launch, embedded source identity, one runtime refresh, and owned cleanup. It does not establish anonymous public acquisition, checksums or attestations, deb installation, canonical AppImage staging, package trust, UI behavior, settings restart, degradation, removal, user-state policy, or #98 evidence.

## Verification

The parser and hostile output cases run on every host. Linux process-settlement regressions remain active source tests. The real-package payload probes are ignored until the hosted Linux job builds and statically verifies both bundles:

```sh
cargo test --manifest-path src/BatCave.App/src-tauri/Cargo.toml \
  --test linux_package_owned_transport

bash scripts/validate-tauri.sh --bundle-only
cargo test --manifest-path src/BatCave.App/src-tauri/Cargo.toml \
  --test linux_package_owned_transport -- --ignored --nocapture
```

Issue #115 remains open. Its selected public deb/AppImage must still be independently established inside the future Rust-owned complete operation and complete the full native install/stage, trust, runtime, removal, settlement, cleanup, user-state, and evidence gates.
