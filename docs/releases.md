# Release channels and verification

BatCave releases must come from an owner-selected commit at the tip of `main` with completed validation. The workflow builds the packages, signs the updater payloads, publishes an immutable GitHub Release when authorized, and verifies the public downloads. A successful build alone does not complete release verification.

## Version and source

`src/BatCave.App/src-tauri/Cargo.toml` contains the only authored app version. Tauri uses it for the runtime and packages. The private npm package and lockfile do not carry another app version.

Stable tags use `vMAJOR.MINOR.PATCH`. Prerelease tags add a SemVer suffix, such as `v0.2.0-rc.1`, and must be marked as GitHub prereleases. Stable and prerelease builds use distinct tags and installed versions.

Run `node scripts/verify-release-version.mjs <tag>` before building. Release scripts must use `verifyWorkspaceReleaseVersion` to bind the tag to Cargo. `parseReleaseTag` checks syntax only and is suitable for synthetic test inputs; it cannot establish the repository version.

## Run the release workflow

Dispatch `Versioned release` manually from `main` with the tag, channel, and approved 40-character source SHA. The SHA must match both the checked-out commit and the current tip of protected `main`. The workflow has no tag default and does not publish in response to a pushed tag.

Use the default `publish: false` for a dry run. It retains the complete workflow artifact without creating a tag or GitHub Release. Use `publish: true` only for an approved release.

The repository owner can merge a PR after its six required checks pass; a second account is not required. Release preparation then requires successful `Validation` on the exact `main` commit. The workflow checks the latest run and attempt, requires all five main validation jobs to pass, and allows only the PR-only dependency review to be skipped. It repeats this check before publication. These reads use the built-in `GITHUB_TOKEN`; no admin-read token or environment reviewer is needed.

Repository immutable releases must remain enabled. The workflow verifies the published release's immutable state, attestations, and public bytes. It does not read repository administration settings during the build.

Windows packages are not Authenticode-signed. The release workflow has no Azure dependency or Store submission step. All three platforms still require the existing Tauri updater signing key.

`macos_signing` defaults to `notarized` and requires the Apple credentials below. A prerelease may explicitly select `adhoc`; that preview is not notarized and its release notes say so. Stable releases reject `adhoc`. A missing or failed Apple credential never silently switches the selected mode.

A release contains:

- the offline-capable Windows NSIS installer and standalone GUI and benchmark CLI executables;
- Linux deb and AppImage packages;
- the Apple Silicon `arm64` macOS DMG and updater archive;
- `SHA256SUMS.txt` and a Sigstore/GitHub build-provenance bundle.

Release workflow artifacts last 30 days. Published GitHub Release assets are durable. The separate `Platform bundles` workflow retains test packages for 90 days.

Trusted `main` builds seed dependency caches. Pull requests and versioned releases restore them without saving workspace-crate output. The Linux package-transport job uses the release profile to reuse those dependencies. All workflow actions remain pinned to immutable commits; cache reuse does not skip verification.

## Windows package ownership

NSIS installs the desktop and collector service. `batcave-monitor-cli.exe` is a standalone release asset, not an installed component. Upgrade and uninstall remove only the one historical installed CLI that matches its recorded size and SHA-256. A different object at that path blocks cleanup.

The installer keeps Public Desktop and Common Programs shortcuts absent. A native provisioner retires only the exact historical links, using pinned known-folder ancestry and the same verified file handle for deletion. Unknown objects block the installer. Shared folder ACLs stay unchanged. Generated `installer.nsi` checks enforce the callback guards, omitted finish-page shortcut control, and native retirement order while preserving AppUserModelId cleanup.

The machine-wide App Paths registration remains installer-owned. Separately, a normal unelevated launch of the verified installed app may create a missing Start entry for that user. Existing entries are preserved, and machine uninstall leaves them as user state. See [Windows shared-shortcut retirement](decisions/0013-windows-shared-shortcut-retirement.md) for registration, creation, rollback, and cleanup rules.

## Windows collector privilege migration

Protected collection uses an authenticated installed collector service or an already-elevated desktop process. The retired elevated-helper mode, `set_admin_mode`, its persisted preference, helper IPC, and `elevated_helper` source are absent. Missing, stopped, incompatible, or unauthorized service states stay visible while standard-access monitoring continues.

On upgrade, a standard-user launch removes only known legacy helper files and valid run-directory shapes under that user's `BatCaveMonitor/elevated-helper`. It preserves unknown entries and rejects unsafe paths. [Current-user state](current-user-state.md) defines the cleanup limits.

Service replacement uses a fixed recovery controller, a protected atomic journal, and a verified rollback image. A candidate is accepted only when SCM reports `Running`, the process generation and stable executable match the transaction, and the service has produced an initial sample. Failure restores the verified old image. Interrupted preparation, validation, rollback, and finalization can resume, including same-version different-build and superseding-installer retries.

The compatibility alias supports uninstallers that include the recovery lookup. During the first migration from an older uninstaller, failure before replacing `uninstall.exe` requires retrying the installer. A rejected service can leave the new desktop beside the restored old service; version checks keep that desktop on visible standard-access fallback until recovery completes.

Uninstall can remove a settled stopped `1066/1` service without restarting it. The journal remains until SCM deletion succeeds. Delete-pending cleanup reports a required reboot instead of completion. See [Windows collector service host](windows-collector-service-host.md) for authentication, ETW ownership, and service behavior.

## Installed Windows verification

The private [Windows lifecycle proof controller](windows-lifecycle-proof-controller.md) runs the attended install, upgrade, rollback, restart, fallback, and uninstall checks. It binds fixed artifacts to retained file handles, authenticates one elevated worker, supervises child trees through Job Objects, and derives a sanitized export from 28 private receipts plus standard-parent observations.

The readiness gates are enabled after source review and packaged-payload UI Automation checks. Installed lifecycle proof still requires a successful attended run and verification of its sanitized export. Historical protocol-v3 export blockers do not describe the current controller.

The controller contract defines artifact pins, fixed environments, checkpoint and abort ordering, restoration, shortcut and registry observations, user-state preservation, and cleanup. A failed stage retains its original cause and restoration result. Unsettled processes block further mutation. An outer NSIS exit code or a passing source test cannot replace those observations.

Proof builds may export post-sign uninstaller bytes with `BATCAVE_UNINSTALLER_EXPORT_PATH`. A failed export fails the build. This supplies the plan's exact uninstaller identity without an extra installation.

## Platform support and proof

The [platform capabilities matrix](platform-capabilities.md) is the canonical human view of supported release profiles; the [version 1 platform support contract](evidence/releases/platform-support-contract.v1.json) is the machine authority. `declared` records the intended host, architecture, runtime, and package boundary. `source_enforced` records that repository configuration, hosted builds, metadata, and extraction-only package checks agree with that boundary. Every current profile still has `native_oldest_supported: pending`, so none of those checks proves installation or runtime behavior on its oldest-supported host.

Linux release builders are pinned to `ubuntu-22.04`. Package verification requires x86-64 ELF payloads, a maximum required symbol version of `GLIBC_2.35`, and deb dependencies on `libgtk-3-0` and `libwebkit2gtk-4.1-0`. Those are source/build gates only. Native evidence remains separate from build and package checks.

Verify a downloaded file with `Get-FileHash -Algorithm SHA256` on Windows, `sha256sum --check SHA256SUMS.txt` on Linux, or `shasum -a 256 -c SHA256SUMS.txt` on macOS. Verify provenance with `gh attestation verify <file> --repo TheGreenCedar/BatCave`. On Windows, confirm the installed version in Apps settings and the executable file properties matches the release tag without the leading `v`.

After publication, the release workflow downloads every expected asset again through its unauthenticated public URL into a new directory. It rejects any name, size, or SHA-256 difference from the prepublication inventory, verifies that `SHA256SUMS.txt` covers every build subject, requires GitHub's immutable-release attestation, and verifies each subject against the exact `main` source SHA and `.github/workflows/release.yml` on a GitHub-hosted runner. Passing contract tests proves the verifier source; only a successful run against the published assets proves a release, and that live evidence remains part of the stable-release gate.

The private `batcave-install-smoke` Rust binary is the install-smoke entry. It independently verifies the immutable public release, complete inventory, checksums, source-bound attestations, and selected bytes before private dispatch. The release workflow exercises the macOS updater staging profile. On Linux the verifier seals deb or AppImage bytes in a private immutable descriptor, revalidates that authority in the Linux handler, and returns `skipped`; it does not install, stage, launch, or emit native or release evidence. [ADR 0003](decisions/0003-private-native-artifact-consumption-authority.md) records the closed authority boundary.

Published deb and AppImage releases also run separate protected post-public smokes on fresh `ubuntu-22.04` GitHub-hosted runners. Both compare anonymous public bytes to the exact pre-publication candidate inventory, verify checksums and source-bound attestations again, and exercise identity, settings, degradation, advancing core telemetry, removal, and cleanup as a standard user. The [deb job](linux-deb-post-public-smoke.md) installs and purges through owned transient systemd units. The AppImage job additionally verifies the updater signature, then uses fixed extract-and-run staging without claiming a normal package install or A-to-B update. Both sanitized results are post-public observations with `release_evidence_eligible: false`; neither promotes the underlying current-user `native_candidate` packet into a #98 `release_evidence` packet.

Published Apple Silicon macOS updater archives run a separate [protected post-public staging observation](macos-updater-post-public-smoke.md) on `macos-15`. The private Rust verifier independently binds the immutable public release, full inventory, checksums, source attestations, updater signature, and exact archive bytes before it preflights, materializes, reverifies, and removes one private staged app tree. The result remains explicitly ineligible as release evidence and preserves `macos_updater_staging_only`; it does not install or launch the app, prove Developer ID/notarization/stapling at the staged destination, exercise A-to-B updating, or alter the unresolved DMG transport boundary.

After the platform lanes produce sanitized packets for one exact public release, assemble `docs/evidence/releases/<tag>/index.json` and run `node scripts/validate-release-evidence-index.mjs` against it. The index binds packet file digests, release/workflow identity, support profiles, package roles, and selected public assets; it has no passing or accepted disposition. Its successful validation proves only that the review input is internally consistent. Stable-release readiness still depends on live public verification, native platform evidence, and updater proof; a prerelease does not claim those open native checks are complete.

## Optional Windows signing tools

Versioned downloads use the ordinary unsigned Windows package configuration and a mandatory Tauri updater signature. The workflow verifies that signature against the embedded public key after packaging. It publishes no Authenticode or Store readiness claim.

The repository retains `scripts/build-signed-windows-release.ps1` and its Azure Artifact Signing helpers for a future signed distribution. They are not called by the release workflow. Their certificate, timestamp, byte-order, and tamper checks remain intact. The [Store source checklist](store/windows-submission-checklist.md) describes that separate signed distribution; unsigned preview packages do not satisfy it.

## macOS signing and notarization

Pushes to `main` and manual `Platform bundles` runs produce an ad-hoc-signed Apple Silicon `.app` and DMG for internal validation. The explicit `adhoc` prerelease mode packages the same type of build, verifies the DMG and updater archive, and labels the download as not notarized. The default `notarized` mode requires a Developer ID Application certificate and App Store Connect API key; it fails before packaging if any required secret is absent.

Configure these GitHub Actions secrets:

- `APPLE_CERTIFICATE`: base64-encoded Developer ID Application `.p12` contents.
- `APPLE_CERTIFICATE_PASSWORD`: password used when exporting the `.p12`.
- `APPLE_SIGNING_IDENTITY`: the complete `Developer ID Application: Name (TEAMID)` identity.
- `APPLE_API_KEY`: App Store Connect API key ID.
- `APPLE_API_ISSUER`: App Store Connect issuer ID.
- `APPLE_API_KEY_CONTENT`: the complete private `.p8` file contents, including its BEGIN/END lines.
- `TAURI_SIGNING_PRIVATE_KEY` and `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`: the existing Tauri updater signing credentials shared by all platforms.

The release job writes the API key and certificate to mode-600 temporary files, imports the certificate into a temporary keychain, and lets Tauri sign, notarize, and staple the Apple Silicon app. It then signs, notarizes, and staples the containing DMG separately before removing every temporary credential in an `always()` cleanup step. `scripts/verify-macos-bundle.sh --mode release` requires an `arm64` slice and rejects an Intel `x86_64` slice, enforces macOS 12 as the deployment minimum, hardened runtime, one consistent bundle ID and Developer ID team, accepted Gatekeeper assessments, valid app and DMG staples, and a healthy DMG filesystem. The same checks apply to the app mounted from the DMG and the app extracted from the updater archive. The release is blocked if any gate fails.

For a downloaded notarized DMG, run:

```bash
hdiutil verify BatCave*.dmg
spctl --assess --type open --context context:primary-signature --verbose=4 BatCave*.dmg
xcrun stapler validate BatCave*.dmg
```

Mount the image and run `spctl --assess --type execute --verbose=4` against `BatCave Monitor.app` before first launch when performing release QA on a clean machine.

## Signed in-app updates

BatCave never checks for updates at startup or in the background. The Settings drawer provides a manual **Check now** action that contacts `github.com` with a 15-second timeout. It reads only the latest stable release; prereleases and downgrades are not offered. A failed or offline check leaves monitoring unchanged.

Tauri updater signatures are mandatory and independent from Windows Authenticode or Apple Developer ID signing. Release builds use `TAURI_SIGNING_PRIVATE_KEY` and `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` from GitHub Actions secrets, while the public key is embedded in `tauri.conf.json`. The release workflow creates signed NSIS, AppImage, and Apple Silicon `.app.tar.gz` updater artifacts plus `latest.json`; missing, empty, duplicate, or byte-mismatched signatures fail verification. The macOS verifier extracts only a private copy of the exact signed archive bytes and rejects absolute or traversing paths, links, device entries, extra roots, and missing or multiple app roots before writing bundle contents. The updater manifest publishes only the supported `darwin-aarch64` entry; `darwin-x86_64` is absent. Ordinary local builds use no updater signing key and do not create updater artifacts.

The updater signature authenticates payload bytes, not `latest.json`; it does not expire or cryptographically bind a payload to the stable channel. GitHub's latest-release routing, HTTPS, and release controls own that metadata boundary. The exact guarantee and replay limits are recorded in [the updater freshness decision](decisions/0002-update-manifest-freshness-and-expiry.md).

Before replacing a selected update or after download/install completion or failure, the frontend explicitly closes the Tauri `Update` resource. Retry always performs a new check and never reuses the prior in-process selection. The deterministic [local hostile-case matrix](updater-hostile-fixtures.md) exercises the pinned Tauri metadata, comparator, download, and payload-signature boundary without changing production configuration or invoking a platform installer.

Never replace or delete the private key without a rotation release. To rotate, first publish an update signed by the old key that embeds the new public key, then sign later releases with the new private key. If the private key is lost before that transition, existing installations cannot accept another in-app update and users must install a new release manually. Invalid or tampered signatures are rejected by the updater before installation.
