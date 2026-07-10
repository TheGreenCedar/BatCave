# Release channels and verification

BatCave publishes immutable GitHub Releases from commits already on protected `main`. Cargo, npm, the npm lockfile workspace, and Tauri must all contain the exact SemVer in the release tag. `node scripts/verify-release-version.mjs <tag>` enforces that contract before any platform build starts.

Stable tags use `vMAJOR.MINOR.PATCH`. Prerelease tags add a SemVer suffix such as `v0.2.0-rc.1` and are always marked as GitHub prereleases. Stable and prerelease artifacts never share a tag or installed version.

The `Versioned release` workflow supports two paths:

- Pushing an aligned `v*` tag publishes automatically after all builds and provenance steps pass.
- A manual run from `main` accepts an aligned tag and channel. `publish: false` is the default dry run; it retains the complete release artifact without creating a tag or GitHub Release. Set `publish: true` only for an approved release.

Every release artifact contains the offline-capable Windows NSIS installer, Windows GUI and benchmark CLI executables, Linux deb and AppImage packages, `SHA256SUMS.txt`, and a Sigstore/GitHub build-provenance bundle. Workflow artifacts are retained for 30 days; published GitHub Release assets are durable.

Verify a downloaded file with `Get-FileHash -Algorithm SHA256` on Windows or `sha256sum --check SHA256SUMS.txt` on Linux. Verify provenance with `gh attestation verify <file> --repo TheGreenCedar/BatCave`. On Windows, confirm the installed version in Apps settings and the executable file properties matches the release tag without the leading `v`.

Windows artifacts remain unsigned until the code-signing issue is resolved. Do not promote an unsigned prerelease to the stable channel.

## Signed in-app updates

BatCave never checks for updates at startup or in the background. The Settings drawer provides a manual **Check now** action that contacts `github.com` with a 15-second timeout. It reads only the latest stable release; prereleases and downgrades are not offered. A failed or offline check leaves monitoring unchanged.

Tauri updater signatures are mandatory and independent from Windows Authenticode signing. Release builds use `TAURI_SIGNING_PRIVATE_KEY` and `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` from GitHub Actions secrets, while the public key is embedded in `tauri.conf.json`. The release workflow creates signed NSIS and AppImage updater artifacts plus `latest.json`; missing or empty signatures fail manifest generation. Ordinary local builds use no updater signing key and do not create updater artifacts.

Never replace or delete the private key without a rotation release. To rotate, first publish an update signed by the old key that embeds the new public key, then sign later releases with the new private key. If the private key is lost before that transition, existing installations cannot accept another in-app update and users must install a new release manually. Invalid or tampered signatures are rejected by the updater before installation.
