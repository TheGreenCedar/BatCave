# macOS native adapter source boundary

Issue #117 defines the source-only handoff between the #111 artifact capability and the exact native macOS work that remains in #114. It adds no package execution path.

`scripts/macos-native-install-smoke-adapter.mjs` accepts only a process-local install-smoke plan and the historical `artifact_owned_bytes_verified` receipt produced by #111. It derives one frozen descriptor for `macos:dmg` or `macos:macos_updater`; callers cannot provide a command, path, status, trust observation, cleanup assertion, or evidence field.

The descriptor records the closed profile, fixed future tool identifiers, future resource ownership, plan timeouts, exact selected asset identity, and mandatory limitations. Tool identifiers are design constraints only. This slice does not invoke `hdiutil`, the archive extractor, `codesign`, `spctl`, `stapler`, `lipo`, `PlistBuddy`, `ditto`, or an application process.

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
| macOS DMG             | mount, copy, and install an isolated app | `owned_dmg_mount_copy_required`               | none                         |
| macOS updater archive | safely extract and stage an isolated app | `owned_updater_archive_safe_extract_required` | `macos_updater_staging_only` |

The updater profile can never reinterpret staging as installation or a public A-to-B update. Both profiles remain blocked until a reviewed private process boundary consumes the still-live #111 capability and derives observations from settled native execution.

## Verification

Run the source contract with:

```sh
node --test scripts/native-install-smoke-executor.test.mjs
```

That suite creates process-local plans and #111 capabilities, rejects injected adapter arguments and forged receipts, checks the two distinct macOS profiles, and confirms every source result remains skipped with null native/evidence receipts. The existing validation and release workflows run the suite on Windows, Linux, and universal macOS.

Archive traversal, link, collision, size-budget, and replacement coverage remains in `scripts/test-macos-updater-archive.sh`. Those fixtures validate safe extraction code; they do not prove that the future native adapter consumed a selected public archive.

## What closes #114

#114 still requires a reviewed private handshake that holds #111-owned bytes through fixed tokenized process execution, exact signed public universal DMG and updater artifacts, destination trust and identity rechecks, bounded termination with settled cleanup, launch and runtime gates, removal and residue proof, sanitized #98 evidence, and the explicit updater staging-only non-claim. Hosted source tests cannot supply that evidence.
