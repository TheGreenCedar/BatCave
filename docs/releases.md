# Release channels and verification

BatCave publishes immutable GitHub Releases from commits already on protected `main`. The package version in `src/BatCave.App/src-tauri/Cargo.toml` is the only authored app SemVer. Tauri inherits that value for the runtime and platform bundles; the private npm package and lockfile do not carry a second app version. `node scripts/verify-release-version.mjs <tag>` requires the release tag to match Cargo before any platform build starts.

Release tooling keeps syntax parsing separate from repository verification. `parseReleaseTag` is the pure API for table-driven contracts and tests that use synthetic tags. Every executable release boundary must call `verifyWorkspaceReleaseVersion` before it generates or verifies artifacts; that API reads the Cargo package version and rejects drift. New release scripts must preserve that split rather than passing a placeholder version into the bound verifier.

Stable tags use `vMAJOR.MINOR.PATCH`. Prerelease tags add a SemVer suffix such as `v0.2.0-rc.1` and are always marked as GitHub prereleases. Stable and prerelease artifacts never share a tag or installed version.

The `Versioned release` workflow runs only through a manual dispatch from `main`. Enter the tag explicitly, select its stable or prerelease channel, and supply the exact 40-character source commit SHA approved for release. The workflow has no reusable tag default. It requires that SHA to be both the checked-out commit and the current tip of protected `main`; it does not publish from a pushed tag. `publish: false` is the default dry run and retains the complete workflow artifact without creating a tag or GitHub Release. Set `publish: true` only for an approved release.

All build and publication jobs enter the protected `release` environment. Configure its `RELEASE_ADMIN_READ_TOKEN` secret with a fine-grained personal access token or GitHub App token that can read repository Administration settings. Each sensitive job uses that credential only for its first control check, before reading signing secrets or changing release state. The workflow's ordinary `GITHUB_TOKEN` remains limited to the repository permissions needed by later artifact and release operations.

Every release artifact contains the offline-capable Windows NSIS installer, Windows GUI and benchmark CLI executables, Linux deb and AppImage packages, the universal macOS DMG and updater archive, `SHA256SUMS.txt`, and a Sigstore/GitHub build-provenance bundle. Workflow artifacts are retained for 30 days; published GitHub Release assets are durable.

Verify a downloaded file with `Get-FileHash -Algorithm SHA256` on Windows, `sha256sum --check SHA256SUMS.txt` on Linux, or `shasum -a 256 -c SHA256SUMS.txt` on macOS. Verify provenance with `gh attestation verify <file> --repo TheGreenCedar/BatCave`. On Windows, confirm the installed version in Apps settings and the executable file properties matches the release tag without the leading `v`.

After publication, the release workflow downloads every expected asset again through its unauthenticated public URL into a new directory. It rejects any name, size, or SHA-256 difference from the prepublication inventory, verifies that `SHA256SUMS.txt` covers every build subject, requires GitHub's immutable-release attestation, and verifies each subject against the exact `main` source SHA and `.github/workflows/release.yml` on a GitHub-hosted runner. Passing contract tests proves the verifier source; only a successful run against the published assets proves a release, and that live evidence remains part of the stable-release gate.

Native install lanes compose that verifier with the [public-artifact install smoke harness](public-artifact-install-smoke.md). The process-local verifier receipt gates the adapter before package mutation, while the adapter rehashes the selected file and verifies package trust immediately before installation or staging. Harness fixtures and plans are contract proof only; they are not native or release proof.

Windows artifacts remain unsigned until the code-signing issue is resolved. Do not promote an unsigned prerelease to the stable channel.

## macOS signing and notarization

Pushes to `main` and manual `Platform bundles` runs produce an ad-hoc-signed universal `.app` and DMG for internal validation. They are not notarized public downloads. Versioned releases require a Developer ID Application certificate and App Store Connect API key; the workflow will fail before packaging if any required secret is absent.

Configure these GitHub Actions secrets:

- `APPLE_CERTIFICATE`: base64-encoded Developer ID Application `.p12` contents.
- `APPLE_CERTIFICATE_PASSWORD`: password used when exporting the `.p12`.
- `APPLE_SIGNING_IDENTITY`: the complete `Developer ID Application: Name (TEAMID)` identity.
- `APPLE_API_KEY`: App Store Connect API key ID.
- `APPLE_API_ISSUER`: App Store Connect issuer ID.
- `APPLE_API_KEY_CONTENT`: the complete private `.p8` file contents, including its BEGIN/END lines.
- `TAURI_SIGNING_PRIVATE_KEY` and `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`: the existing Tauri updater signing credentials shared by all platforms.

The release job writes the API key and certificate to mode-600 temporary files, imports the certificate into a temporary keychain, and lets Tauri sign, notarize, and staple the universal app. It then signs, notarizes, and staples the containing DMG separately before removing every temporary credential in an `always()` cleanup step. `scripts/verify-macos-bundle.sh --mode release` requires both `arm64` and `x86_64` slices, macOS 12 as the deployment minimum, hardened runtime, one consistent bundle ID and Developer ID team, accepted Gatekeeper assessments, valid app and DMG staples, and a healthy DMG filesystem. The same checks apply to the app mounted from the DMG and the app extracted from the updater archive. The release is blocked if any gate fails.

For a downloaded public DMG, run:

```bash
hdiutil verify BatCave*.dmg
spctl --assess --type open --context context:primary-signature --verbose=4 BatCave*.dmg
xcrun stapler validate BatCave*.dmg
```

Mount the image and run `spctl --assess --type execute --verbose=4` against `BatCave Monitor.app` before first launch when performing release QA on a clean machine.

## Signed in-app updates

BatCave never checks for updates at startup or in the background. The Settings drawer provides a manual **Check now** action that contacts `github.com` with a 15-second timeout. It reads only the latest stable release; prereleases and downgrades are not offered. A failed or offline check leaves monitoring unchanged.

Tauri updater signatures are mandatory and independent from Windows Authenticode or Apple Developer ID signing. Release builds use `TAURI_SIGNING_PRIVATE_KEY` and `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` from GitHub Actions secrets, while the public key is embedded in `tauri.conf.json`. The release workflow creates signed NSIS, AppImage, and universal `.app.tar.gz` updater artifacts plus `latest.json`; missing, empty, duplicate, or byte-mismatched signatures fail verification. The macOS verifier extracts only a private copy of the exact signed archive bytes and rejects absolute or traversing paths, links, device entries, extra roots, and missing or multiple app roots before writing bundle contents. Both `darwin-aarch64` and `darwin-x86_64` manifest entries intentionally point to the same universal archive and signature. Ordinary local builds use no updater signing key and do not create updater artifacts.

Never replace or delete the private key without a rotation release. To rotate, first publish an update signed by the old key that embeds the new public key, then sign later releases with the new private key. If the private key is lost before that transition, existing installations cannot accept another in-app update and users must install a new release manually. Invalid or tampered signatures are rejected by the updater before installation.
