# Installed current-user persistence evidence

Issue [#153](https://github.com/TheGreenCedar/BatCave/issues/153) tracks native proof for the current-user storage contract. The proof surface is deliberately separate from release evidence: a persistence packet records what one exact native artifact did on one host, but it does not prove public provenance, package trust, updater behavior, or release readiness. Those decisions remain with [#76](https://github.com/TheGreenCedar/BatCave/issues/76).

Windows service storage remains outside this contract. Service SIDs, DACLs, `ProgramData`, diagnostics, and uninstall behavior belong to [#69](https://github.com/TheGreenCedar/BatCave/issues/69).

## Packaged probe

Both packaged GUI and CLI binaries expose one fixed diagnostic command before the desktop app starts:

```text
BATCAVE_CURRENT_USER_PERSISTENCE_PROOF=1 <packaged-executable> \
  --current-user-persistence-proof --phase initialize|restart|degraded
```

The environment sentinel is required because `initialize` writes a fixed UI preference (`ember`, 180 history points) through the production runtime store. The command accepts no root, file, output, or subprocess argument. It always resolves the production current-user root for the process environment.

Each phase emits one compact JSON receipt. The receipt includes exact embedded source identity, platform, architecture, install kind, settings, persistence state, current-user permission state, component durability, and sanitized failure codes. It deliberately omits local paths, diagnostic messages, corrupt bytes, raw logs, timestamps from component failures, environment dumps, and service-owned state.

The intended sequence is:

1. `initialize`: write the fixed mutation and shut down cleanly.
2. `restart`: reopen the same production root without mutation and observe the retained value.
3. Corrupt `settings.json` outside the process.
4. `degraded`: reopen without mutation, observe visible persistence degradation, and confirm the corrupt source bytes remain unchanged.
5. Remove the installed or staged application, then confirm the application is gone while the current-user root and an outside sentinel remain.

## macOS source automation

The macOS helper runs that sequence against a local `.app` copied into an isolated temporary `Applications` directory:

```bash
node scripts/capture-macos-current-user-persistence.mjs \
  --app "src/BatCave.App/src-tauri/target/universal-apple-darwin/release/bundle/macos/BatCave Monitor.app" \
  --source-sha "$(git rev-parse HEAD)" \
  --output artifacts/current-user-persistence/macos-app-bundle.json
```

Build the app with the same exact source identity first:

```bash
BATCAVE_SOURCE_COMMIT_SHA="$(git rev-parse HEAD)" \
  npm run tauri -- build --target universal-apple-darwin
```

Run the build command from `src/BatCave.App`. The capture helper refuses to overwrite an existing output file. It rejects linked app roots, linked executable paths, and source-to-copy drift before execution. Its app-bundle digest is a bytewise-sorted sequence of length-prefixed type, relative-path, mode, and payload fields. The helper records the copied tree that it executes and rehashes that tree before removal. This repository-local digest is not a DMG digest. The resulting packet therefore retains the `staged_application_bundle_only` limitation and cannot fill the `macos-dmg` index profile.

### Retained integration candidate

[`macos-app-bundle-f010d2eaa8f3.json`](evidence/persistence/native-candidates/macos-app-bundle-f010d2eaa8f3.json) retains the sanitized app-bundle packet captured from integration source `f010d2eaa8f32959309ffda8deaef2a53ce5bda8` on macOS 26.5.2. The input was the sole app observed inside that source tree's freshly built, read-only-mounted local DMG. Its canonical tree digest matched the directly built app before the lifecycle run.

The packet deliberately remains `artifact.kind: app_bundle` with `staged_application_bundle_only`. A path-based `hdiutil` mount cannot prove that DiskImages consumed the previously hashed DMG bytes, and the isolated application copy is not a canonical installation. The packet therefore does not populate the `macos-dmg` profile or prove Developer ID signing, notarization, stapling, publication, or release readiness. The contract tests validate every JSON packet under `native-candidates` even though those packets remain outside the package index.

## Packet and index rules

[`validate-current-user-persistence-evidence.mjs`](../scripts/validate-current-user-persistence-evidence.mjs) is the executable version-1 packet and index contract. It enforces:

- exact keys and closed platform, architecture, artifact, install-kind, permission, phase, component, and failure-code shapes;
- exact source SHA and app-version agreement across all three packaged receipts;
- Unix `0700` root and `0600` owned-file modes, or a Windows ACL evidence model with no invented Unix modes;
- recomputed pass/fail from the seven lifecycle checks, external root/file permissions, every phase's runtime-reported root permission state, and degraded runtime health;
- sorted limitations including `candidate_not_release_evidence`;
- rejection of local paths, raw logs, environment material, credentials, extra fields, digest drift, linked index paths, unstable packet files, and packet/profile mismatches.

The checked-in [evidence index](evidence/persistence/current-user-persistence-index.v1.json) covers Windows NSIS, Linux deb, Linux AppImage, and macOS DMG. `pending` means that native package evidence has not been attached. It is work remaining for #153, not a source-development blocker and not a failure disposition. `native_candidate` means a packet exists and validates internally; it still grants no release status.

Run the repeatable contract proof with:

```bash
node --test \
  scripts/capture-macos-current-user-persistence.test.mjs \
  scripts/validate-current-user-persistence-evidence.test.mjs

node scripts/validate-current-user-persistence-evidence.mjs \
  docs/evidence/persistence/current-user-persistence-index.v1.json

cargo test --manifest-path src/BatCave.App/src-tauri/Cargo.toml persistence_proof --lib
```

Native Windows execution is independently owned. Adding its sanitized packet updates the `windows-nsis` entry; it does not gate the source probe, validator, macOS helper, or other platform evidence work.
