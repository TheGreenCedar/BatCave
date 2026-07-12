# Release channels and verification

BatCave publishes immutable GitHub Releases from commits already on protected `main`. Cargo, npm, the npm lockfile workspace, and Tauri must all contain the exact SemVer in the release tag. `node scripts/verify-release-version.mjs <tag>` enforces that contract before any platform build starts.

Stable tags use `vMAJOR.MINOR.PATCH`. Prerelease tags add a SemVer suffix such as `v0.2.0-rc.1` and are always marked as GitHub prereleases. Stable and prerelease artifacts never share a tag or installed version.

The `Versioned release` workflow supports two paths:

- Pushing an aligned `v*` tag publishes automatically after all builds and provenance steps pass.
- A manual run from `main` accepts an aligned tag and channel. `publish: false` is the default dry run; it retains the complete release artifact without creating a tag or GitHub Release. Set `publish: true` only for an approved release.

Every release artifact contains the offline-capable Windows NSIS installer, Windows GUI and benchmark CLI executables, Linux deb and AppImage packages, the universal macOS DMG and updater archive, `SHA256SUMS.txt`, and a Sigstore/GitHub build-provenance bundle. Workflow artifacts are retained for 30 days; published GitHub Release assets are durable.

Verify a downloaded file with `Get-FileHash -Algorithm SHA256` on Windows, `sha256sum --check SHA256SUMS.txt` on Linux, or `shasum -a 256 -c SHA256SUMS.txt` on macOS. Verify provenance with `gh attestation verify <file> --repo TheGreenCedar/BatCave`. On Windows, confirm the installed version in Apps settings and the executable file properties matches the release tag without the leading `v`.

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

The release job writes the API key and certificate to mode-600 temporary files, imports the certificate into a temporary keychain, and lets Tauri sign, notarize, and staple the universal app. It then signs, notarizes, and staples the containing DMG separately before removing every temporary credential in an `always()` cleanup step. `scripts/verify-macos-bundle.sh --mode release` requires both `arm64` and `x86_64` slices, macOS 12 as the deployment minimum, hardened runtime, a Developer ID signature, accepted Gatekeeper assessments, valid app and DMG staples, and a healthy DMG filesystem. The release is blocked if any gate fails.

For a downloaded public DMG, run:

```bash
hdiutil verify BatCave*.dmg
spctl --assess --type open --context context:primary-signature --verbose=4 BatCave*.dmg
xcrun stapler validate BatCave*.dmg
```

Mount the image and run `spctl --assess --type execute --verbose=4` against `BatCave Monitor.app` before first launch when performing release QA on a clean machine.

## Signed in-app updates

BatCave never checks for updates at startup or in the background. The Settings drawer provides a manual **Check now** action that contacts `github.com` with a 15-second timeout. It reads only the latest stable release; prereleases and downgrades are not offered. A failed or offline check leaves monitoring unchanged.

Tauri updater signatures are mandatory and independent from Windows Authenticode or Apple Developer ID signing. Release builds use `TAURI_SIGNING_PRIVATE_KEY` and `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` from GitHub Actions secrets, while the public key is embedded in `tauri.conf.json`. The release workflow creates signed NSIS, AppImage, and universal `.app.tar.gz` updater artifacts plus `latest.json`; missing, empty, or duplicate signatures fail manifest generation. Both `darwin-aarch64` and `darwin-x86_64` manifest entries intentionally point to the same universal archive and signature. Ordinary local builds use no updater signing key and do not create updater artifacts.

Never replace or delete the private key without a rotation release. To rotate, first publish an update signed by the old key that embeds the new public key, then sign later releases with the new private key. If the private key is lost before that transition, existing installations cannot accept another in-app update and users must install a new release manually. Invalid or tampered signatures are rejected by the updater before installation.
