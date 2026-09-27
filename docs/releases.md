# Release channels and verification

BatCave releases must come from an owner-selected commit at the tip of `main` with completed validation. The workflow builds the packages, signs the updater payloads, publishes an immutable GitHub Release when authorized, and verifies the public downloads. A successful build alone does not complete release verification.

## Version and source

`src/BatCave.App/src-tauri/Cargo.toml` contains the only authored app version. Tauri uses it for the runtime and packages. The private npm package and lockfile do not carry another app version.

Stable tags use `vMAJOR.MINOR.PATCH`. Prerelease tags add a SemVer suffix, such as `v0.2.0-rc.1`, and must be marked as GitHub prereleases. Stable and prerelease builds use distinct tags and installed versions.

Run `node scripts/verify-release-version.mjs <tag>` before building. Build and publication scripts use `verifyWorkspaceReleaseVersion` to bind the tag to Cargo. Post-publication observers parse the original tag and bind it to the retained candidate and downloaded package, so a newer verifier checkout can check an older release.

## Run the release workflow

Dispatch `Versioned release` manually from `main` with the tag, channel, and approved 40-character source SHA. The SHA must match both the checked-out commit and the current tip of protected `main`. The workflow has no tag default and does not publish in response to a pushed tag.

Use the default `publish: false` for a dry run. It retains the complete workflow artifact and candidate inventory without creating a tag or GitHub Release or moving the public latest release. A dry run still builds packages and invokes the selected signing providers. Use `publish: true` only for an approved release.

The repository owner can merge a PR after its six required checks pass; a second account is not required. Release preparation then requires successful `Validation` on the exact `main` commit. The workflow checks the latest run and attempt, requires all five main validation jobs to pass, and allows only the PR-only dependency review to be skipped. It repeats this check before publication. These reads use the built-in `GITHUB_TOKEN`; no admin-read token is needed. The default unsigned Windows path needs no release environment approval.

Repository immutable releases must remain enabled. The workflow verifies the published release's immutable state, attestations, and public bytes. It does not read repository administration settings during the build.

`windows_signing` defaults to `unsigned`, preserving Windows packages without Authenticode or an Azure dependency. Explicit `azure` mode requires the approved signing authority and existing `release` environment described below. Missing readiness, configuration, failed signing, or invalid evidence fails the selected mode; it never falls back to unsigned. Neither mode submits to the Microsoft Store. All three platforms still require the existing Tauri updater signing key.

`macos_signing` defaults to `notarized` and requires the Apple credentials below. A prerelease may explicitly select `adhoc`; that preview is not notarized and its release notes say so. Stable releases reject `adhoc`. A missing or failed Apple credential never silently switches the selected mode.

A release contains:

- the offline-capable Windows NSIS installer and standalone GUI and benchmark CLI executables;
- Linux deb and AppImage packages;
- the Apple Silicon `arm64` macOS DMG and updater archive;
- `SHA256SUMS.txt` and a Sigstore/GitHub build-provenance bundle.

Release workflow artifacts last 30 days. Published GitHub Release assets are durable. The separate `Platform bundles` workflow retains test packages for 90 days.

Trusted `main` builds seed dependency caches. Pull requests and versioned releases restore them without saving workspace-crate output. The Linux package-transport job uses the release profile to reuse those dependencies. All workflow actions remain pinned to immutable commits; cache reuse does not skip verification.

## Recheck a published release

Run `Verify published release` from `main` with the existing tag, original source SHA, and original `Versioned release` run ID. It checks the successful publication job and retained prepublication candidate, then runs the same Linux deb, AppImage, and macOS updater observers used after publication. The verifier comes from the current `main` commit; downloaded packages must match the original release. This workflow has read-only permissions and does not rebuild or republish packages.

The original release run keeps its historical result. A later successful verification run records the checks against those same immutable assets. Candidate inventories remain available for 30 days; an expired inventory blocks a rerun.

## Windows package ownership

NSIS installs the desktop and collector service. `batcave-monitor-cli.exe` is a standalone release asset, not an installed component. Upgrade and uninstall remove only the one historical installed CLI that matches its recorded size and SHA-256. A different object at that path blocks cleanup.

The native installer provisioner owns one `BatCave.lnk` in OS-resolved Common Programs, visible to every user. It records the exclusively created object's identity, content and security before publishing the entry, and removes only that recorded object before NSIS deletes the installed binaries. Foreign replacements and ambiguous interrupted state are preserved and reported as incomplete cleanup. Shared folder ACLs stay unchanged. Stock Tauri shortcut callbacks remain disabled; Public Desktop and the exact historical shared `BatCave Monitor.lnk` entries remain retired. Generated `installer.nsi` checks enforce these native gates while preserving AppUserModelId cleanup.

The machine-wide App Paths registration remains installer-owned. App startup no longer creates per-user Start links. Existing raw links have no creation receipt and cannot be adopted as installer-owned based on their name or target. Inventory those legacy objects separately and obtain approval for the exact deletion list; unresolved profiles or redirected folders prevent a complete historical-cleanup claim. Independent user copies and modified or moved entries remain user state. See [Windows shortcut ownership](decisions/0013-windows-shared-shortcut-retirement.md) and the [lifecycle qualification plan](windows-start-lifecycle-plan.md) for the ownership boundary and remaining native proof.

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

## Optional Azure Windows signing

Ordinary local builds and the default `windows_signing: unsigned` release use the existing unsigned Windows package configuration. The release still creates and verifies a mandatory Tauri updater signature against the embedded public key after packaging. It makes no Authenticode or Store readiness claim.

Select `windows_signing: azure` only after reviewing an existing Azure account's verified identity and Public Trust certificate profile, confirming the actual subject matches the pinned `CN=Albert Najjar` contract, and granting the signing principal `Artifact Signing Certificate Profile Signer` at that profile's scope. Historical onboarding and this source configuration do not prove that account or profile is ready. The workflow creates no Azure account, certificate profile, role assignment, or federated credential.

Before enabling dispatch, review the existing GitHub `release` environment's branch restriction and approval model and the federated subject `repo:TheGreenCedar/BatCave:environment:release`. GitHub can automatically create a referenced missing environment; its name alone proves no protection. Set the repository variable `BATCAVE_ARTIFACT_SIGNING_READY` to the exact value `true` only after the account and environment review is approved. This variable is an operator readiness assertion, not an automated protection or identity verification. Preparation rejects unapproved Azure mode before selecting that environment, and the signed job independently requires the same readiness flag. No second-account or repository-administration read token is introduced.

The signed job requires these configured `release` environment variables, rather than workflow inputs:

- `AZURE_CLIENT_ID`, `AZURE_TENANT_ID`, and `AZURE_SUBSCRIPTION_ID`: the approved OIDC principal and account identifiers.
- `BATCAVE_ARTIFACT_SIGNING_ENDPOINT`: the exact regional HTTPS `https://<region>.codesigning.azure.net` endpoint.
- `BATCAVE_ARTIFACT_SIGNING_ACCOUNT` and `BATCAVE_ARTIFACT_SIGNING_CERTIFICATE_PROFILE`: the reviewed account and Public Trust profile.

Only the Azure Windows job receives signing OIDC authority. Its pinned Azure login authenticates the metadata helper's Azure CLI credential; a client secret is not used. The existing Tauri signing secrets remain mandatory. Required inputs are checked before login, and selected-mode success is required before any platform assets are finalized.

`scripts/build-signed-windows-release.ps1` signs the GUI, standalone CLI, collector service, permitted Foundry dependency, generated uninstaller, and outer NSIS installer. It preserves verified Microsoft signatures on upstream files, validates the pinned unsigned Foundry source before its sole re-signing exception, verifies RFC3161 timestamps, and rejects a byte-tampered installer. It creates and verifies the Tauri updater signature only after the Authenticode installer bytes are final. The generated NSIS ownership contract is checked before collection; no signing or rebundling follows updater verification. Manifest generation, checksums, and build provenance then use those same final bytes.

An Azure dry run uses the approved exact main SHA, matching unused version/tag, `windows_signing: azure`, and `publish: false`. Review `windows-signing-evidence-<tag>` for the inner/final signature inventories, certificate and timestamp fingerprints, source and final hashes, post-sign uninstaller, and signing/tamper receipt. This separate workflow artifact is excluded from the public distributable glob. The retained `batcave-release-candidate-<tag>` includes the validated final production inventory and binds it to the source SHA and exact GUI, CLI, and installer digests. Azure mode cannot finalize without that evidence; all candidate rebuilds repeat the check. Artifact retention remains 30 days.

This candidate is concrete signing/build evidence, not clean-machine publisher trust or installed lifecycle proof. Attended Windows trust, tamper, install/update/repair/uninstall checks remain separate qualification. Store submission is optional and requires its own [Store source checklist](store/windows-submission-checklist.md); Authenticode candidate validation accepts signing evidence without a Store preflight receipt.

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
