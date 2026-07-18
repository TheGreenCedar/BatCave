# macOS native adapter source boundary

Issue #117 defines the source-only handoff between the #111 artifact capability and the macOS adapter boundary reviewed under #114. Issue #114 is closed with the DMG exact-byte transport limitation accepted; this document records that boundary and the non-claims still relevant to #76. The JavaScript descriptor adds no package execution path; the separate private Rust release verifier now owns updater staging.

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

Both profiles carry the same consumed-destination revalidation contract. The future closed adapter must derive all of these again from the copied or staged app:

- compiled Tauri bundle identifier;
- verified release version;
- an `arm64` slice and no unsupported Intel `x86_64` slice;
- code-signature integrity;
- Developer ID application authority;
- notarization acceptance; and
- a valid staple.

The contract maps the final three trust observations to the #98 `contained_app_developer_id`, `contained_app_notarization`, and `contained_app_staple` roles. The source descriptor cannot mark any observation passed, prove that sequential tool observations came from one immutable destination tree, or create evidence. [ADR 0008](decisions/0008-macos-dmg-destination-revalidation.md) exercises the fixed DMG destination hooks with an inert ad-hoc-signed fixture, including mid-revalidation substitution and post-spawn settlement retention. It retains both the ADR 0006 exact-transport non-claim and the destination-binding non-claim.

The updater profile can never reinterpret staging as installation or a public A-to-B update. Its private Rust path now consumes owned selected bytes for bounded staging and cleanup, but neither profile reaches the reviewed native process boundary needed for destination trust, launch, settlement, removal, or A-to-B observations.

The transport decisions are now distinct. [ADR 0006](decisions/0006-macos-dmg-owned-byte-transport.md) records that `hdiutil` cannot consume the Rust-owned `/dev/fd/N` input and forbids a path fallback. [ADR 0007](decisions/0007-macos-updater-owned-stream-transport.md) records that updater archives can be preflighted and staged from Rust-owned immutable compressed bytes without a package path. The private Rust public-release verifier now dispatches the updater profile through that owned-stream staging path. The source-only JavaScript descriptor remains non-executing, and the DMG profile remains blocked by its distinct exact-byte transport boundary.

## Verification

Run the source contract with:

```sh
node --test scripts/native-install-smoke-executor.test.mjs
cargo test --manifest-path src/BatCave.App/src-tauri/Cargo.toml \
  --bin batcave-install-smoke --features private-release-verifier \
  install_smoke_macos_updater
```

The JavaScript suite creates process-local plans and #111 capabilities, rejects injected adapter arguments, copied descriptors, same-asset receipt substitution, and equivalent-plan replay, checks the two distinct macOS profiles, and confirms every source result remains skipped with null native/evidence receipts. The production Rust suite consumes immutable owned bytes, validates one complete gzip member and a zero-only tar tail, rejects trailers, hidden post-marker entries, trailing data, second members, hostile tar entries, and macOS filesystem collisions before materialization, charges retained `String` and `Vec` prefix allocations to its path budget, rechecks every staged file, and retains cleanup authority with both failures visible until bounded retry succeeds. The Apple Silicon macOS validation job runs the private verifier tests.

The release extractor retains its separate traversal, link, collision, size-budget, and replacement coverage in `scripts/test-macos-updater-archive.sh`. Those shell fixtures remain local and do not replace exact-public updater A-to-B proof.

## Current #76 proof boundary

The accepted #114 boundary and the updater staging observer do not claim installation or stable-release execution. For #76 to treat either macOS profile as stable-release proof, the Rust-owned public-release entry would still need to dispatch exact signed public bytes through a reviewed native adapter, complete all seven destination gates, settle bounded execution and cleanup, exercise launch and runtime behavior, prove removal and residue state, produce sanitized #98 evidence, and retain the updater staging-only non-claim. The updater path currently stops after exact-public staging and cleanup; the DMG path remains blocked because no safe primitive replaces the rejected descriptor or path fallback. Hosted source tests cannot supply the missing native evidence.
