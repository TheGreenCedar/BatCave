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

The packet deliberately remains `artifact.kind: app_bundle` with `staged_application_bundle_only`. A path-based `hdiutil` mount cannot prove owned-byte transport through DiskImages, and the isolated application copy is not a canonical installation. This app-bundle packet therefore does not populate the `macos-dmg` profile or prove Developer ID signing, notarization, stapling, publication, or release readiness. The contract tests validate every JSON packet under `native-candidates` even though those packets remain outside the package index.

### Local DMG candidate automation

The DMG helper consumes the actual locally built package and records its artifact-byte digest:

```bash
node scripts/capture-macos-dmg-current-user-persistence.mjs \
  --dmg "src/BatCave.App/src-tauri/target/universal-apple-darwin/release/bundle/dmg/BatCave Monitor_0.2.0-rc.2_universal.dmg" \
  --source-sha "$(git rev-parse HEAD)" \
  --output artifacts/current-user-persistence/macos-dmg.json
```

Build the universal package first with `BATCAVE_SOURCE_COMMIT_SHA` set to that exact source SHA. The helper reads a stable regular source file, copies those bytes into a mode-`0400` file under a private mode-`0700` workspace rooted at `/private/tmp`, and hashes the copied artifact before and throughout the operation. While holding the atomic `/tmp/batcave-diskimages-proof.lock`, it verifies and mounts that copy read-only, requires one real app bundle, copies it with fixed `ditto` arguments, proves the canonical mounted and copied app-tree digests match, detaches, and rehashes the DMG. Every attempted DiskImages operation enters bounded cleanup that checks the original mount-point identity, the native mount inventory, and all newly observed DiskImages helper PIDs. Settled paths release the lock before lifecycle execution. If settlement remains unproven, the helper leaves the lock and private workspace in place for explicit recovery rather than releasing authority.

The lifecycle then uses the copied package application through the same fixed production-root probe as the app-bundle helper. The executable receives only its private fixed `HOME`, private fixed `TMPDIR`, and the proof sentinel; caller `DYLD_*`, `PATH`, temporary-directory, and data-root variables are not inherited. Input paths, mount paths, usernames, raw process output, corrupt bytes, environment values, and other host-local material never enter the packet. A path-based DMG mount still does not establish the immutable owned-byte transport required by [ADR 0006](decisions/0006-macos-dmg-owned-byte-transport.md); the packet keeps that limitation explicit. The local helper/mount settlement check does not claim authority over DiskImages remote-helper internals tracked by #114.

### Retained local DMG candidate

[`macos-dmg-5ced31017975.json`](evidence/persistence/package-candidates/macos-dmg-5ced31017975.json) retains the sanitized packet captured from a universal ad-hoc DMG built from integration source `5ced3101797501c5b6ae1106ee5c947da5f0ae61` on macOS 26.5.2 arm64. The local DMG bytes are `sha256:652765504e9a64bc7e2ebc97c8a23f7dd2547d6f18536fa8fd416a8fc8634b2c`; the retained packet bytes are `sha256:ca63f8bbf7e320f34ce2975f099e857bab938340907077ff34ed60f6381ba218`.

The candidate records a private `0700` Application Support root and `0600` diagnostics, settings, and warm-cache files. It passed restart retention, visible corrupt-state degradation with the corrupt settings bytes retained, application removal with the current-user state retained, and outside-sentinel containment. Every receipt reports exact source `5ced3101797501c5b6ae1106ee5c947da5f0ae61`.

This is a local-build native candidate, not public or release evidence. The DMG and contained app use only an ad-hoc signature. The packet does not prove owned-byte DiskImages transport, Developer ID signing, notarization, stapling, publication, updater behavior, public provenance, or release readiness.

## Linux package automation

The Linux helper captures both locally built package formats on an ephemeral Ubuntu host:

```bash
BATCAVE_SOURCE_COMMIT_SHA="$(git rev-parse HEAD)" \
  bash scripts/validate-tauri.sh --bundle-only

node scripts/capture-linux-current-user-persistence.mjs \
  --deb src/BatCave.App/src-tauri/target/release/bundle/deb/*.deb \
  --appimage src/BatCave.App/src-tauri/target/release/bundle/appimage/*.AppImage \
  --source-sha "$(git rev-parse HEAD)" \
  --output-dir artifacts/current-user-persistence/linux
```

The output directory must not already exist. The helper copies both artifacts into separate mode-700 private workspaces, re-reads each source before accepting it, and rehashes the private copy around execution. It runs each lifecycle under a separate isolated `HOME` with no inherited caller environment, so the production default `~/.local/share/BatCaveMonitor` resolution is exercised without touching the runner account's ordinary state.

The deb profile rejects a pre-existing BatCave package, verifies the expected package name, version, architecture, and installed executable ownership, installs the private copy through fixed absolute `sudo` and `dpkg` commands, and purges it after the degraded phase. The AppImage profile launches the private AppImage through its own extract-and-run runtime, which avoids a hosted-runner FUSE dependency while retaining runtime `APPIMAGE` provenance, then removes the staged package. Both profiles require bounded output, timeout, direct-child exit, and process-group settlement before application removal or workspace cleanup.

Native package execution is restricted to the repository owner's manual `Platform bundles` workflow dispatch with `capture_linux_persistence` enabled. That mode runs only the Linux job; Windows and macOS jobs are skipped. Pull requests cannot trigger the workflow, ordinary main pushes only build bundles, and neither path installs a pull-request deb. The manual job embeds `${{ github.sha }}` in the packaged receipts, validates the two sanitized packets, and uploads only those JSON files as a short-lived workflow artifact.

A locally built package packet is still candidate evidence. It must retain `candidate_not_release_evidence` and `local_bundle_without_public_provenance`; the AppImage packet also retains `appimage_extract_and_run`. These packets can populate the matching `linux-deb` and `linux-appimage` index profiles because the observed artifact kinds are the actual packages, but they do not prove a public checksum, source attestation, package-repository signature, Tauri updater signature, public download, update flow, or release readiness. Those claims remain with #76 and #115.

### Retained Linux package candidates

The indexed [`deb`](evidence/persistence/package-candidates/linux-deb-270e50ebaa3a.json) and [`AppImage`](evidence/persistence/package-candidates/linux-appimage-270e50ebaa3a.json) packets retain the exact sanitized bytes uploaded by trusted owner dispatch [29365844562](https://github.com/TheGreenCedar/BatCave/actions/runs/29365844562). The Ubuntu 22.04 x86_64 job built and exercised both packages from exact source `270e50ebaa3a5716224a84f04c0b8ef730e55ab1`. Every lifecycle receipt reports that same source identity.

The separately uploaded package bytes independently match the packet claims: the deb is `sha256:ae82d2a342d9dfca716818ca2028c17e5498e8c9a17f7e61e407a00ff2d3a720`, and the AppImage is `sha256:cbed1bd6b56bb0fc90bdc3ba3cc26f15f5eefc2c0f953318514a911b3a9046aa`. Both candidates record a private `0700` current-user root and `0600` owned files, settings retained across restart, a successful visibly degraded launch with corrupt settings bytes preserved, package removal with state retained, and an untouched outside sentinel. The retained packet-byte digests are `sha256:05c0612bd7aa0b684907bbbe4a67665102066367927b2fd5e1835525d8661de5` for deb and `sha256:bb17f55542849764933b24e1760fd5eea7322fbfc3addd6e9e52f38f8be90548` for AppImage.

These remain local-build native candidates. The AppImage used its extract-and-run runtime on the hosted runner. Neither packet proves public provenance, signing, updater transport, another Linux distribution or architecture, or release readiness.

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
  scripts/capture-linux-current-user-persistence.test.mjs \
  scripts/capture-macos-current-user-persistence.test.mjs \
  scripts/capture-macos-dmg-current-user-persistence.test.mjs \
  scripts/validate-current-user-persistence-evidence.test.mjs

node scripts/validate-current-user-persistence-evidence.mjs \
  docs/evidence/persistence/current-user-persistence-index.v1.json

cargo test --manifest-path src/BatCave.App/src-tauri/Cargo.toml persistence_proof --lib
```

Native Windows execution is independently owned. Adding its sanitized packet updates the `windows-nsis` entry; it does not gate the source probe, validator, macOS helper, or other platform evidence work.
