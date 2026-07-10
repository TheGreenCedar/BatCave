# Release channels and verification

BatCave publishes immutable GitHub Releases from commits already on protected `main`. Cargo, npm, the npm lockfile workspace, and Tauri must all contain the exact SemVer in the release tag. `node scripts/verify-release-version.mjs <tag>` enforces that contract before any platform build starts.

Stable tags use `vMAJOR.MINOR.PATCH`. Prerelease tags add a SemVer suffix such as `v0.2.0-rc.1` and are always marked as GitHub prereleases. Stable and prerelease artifacts never share a tag or installed version. Automatic updates are not enabled by this release channel.

The `Versioned release` workflow supports two paths:

- Pushing an aligned `v*` tag publishes automatically after all builds and provenance steps pass.
- A manual run from `main` accepts an aligned tag and channel. `publish: false` is the default dry run; it retains the complete release artifact without creating a tag or GitHub Release. Set `publish: true` only for an approved release.

Every release artifact contains the offline-capable Windows NSIS installer, Windows GUI and benchmark CLI executables, Linux deb and AppImage packages, `SHA256SUMS.txt`, and a Sigstore/GitHub build-provenance bundle. Workflow artifacts are retained for 30 days; published GitHub Release assets are durable.

Verify a downloaded file with `Get-FileHash -Algorithm SHA256` on Windows or `sha256sum --check SHA256SUMS.txt` on Linux. Verify provenance with `gh attestation verify <file> --repo TheGreenCedar/BatCave`. On Windows, confirm the installed version in Apps settings and the executable file properties matches the release tag without the leading `v`.

Windows artifacts remain unsigned until the code-signing issue is resolved. Do not promote an unsigned prerelease to the stable channel.
