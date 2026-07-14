# Release evidence packets

A release evidence packet is a sanitized receipt for one public package on one tested platform. It binds the package bytes and the observed install, runtime, and cleanup checks to a release tag, one source commit, and one release-workflow attempt.

Schema validity does not mean the release passed. A valid packet can contain `failed` or `blocked` checks and accepted limitations. Release approval still depends on the checks recorded in every required platform packet and the public-release verification gates.

## Contract

The versioned schema lives at `docs/evidence/releases/release-evidence-packet.schema.json`. The cross-field validator is `scripts/validate-release-evidence-packet.mjs`.

Each packet contains:

- `schema_version`, `packet_kind`, `packet_id`, and a UTC observation time;
- the repository, strict release tag and channel, one exact 40-character source commit repeated as the main and release target commit, and the exact public release URL;
- the release workflow file, run ID, attempt, and exact public run URL;
- the tested operating system, host architecture, package kind, package architecture, and the referenced asset name;
- exact asset-role names from `scripts/release-asset-contract.mjs`, byte sizes, matching SHA-256 digests from the evidence run and GitHub API, exact anonymous download URLs, source-bound attestations, and verified trust identities;
- sorted install, runtime, and cleanup check maps with an explicit status and sanitized outcome; and
- a sorted limitation map with an explicit disposition and short sanitized summary.

Asset arrays and keyed signature, check, and limitation maps use stable lexical ordering. A packet may include one or more assets from the exact release inventory, but every included role has a closed trust set. Unknown fields, names outside the release inventory, cross-role trust evidence, and Unicode- or case-colliding asset names are rejected. Failures and blocks stay explicit instead of being normalized into a pass.

## Platform trust paths

The validator applies package-specific trust rules:

| Exact package role     | Required public trust evidence                                                                                                                                                                                                                 |
| ---------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Windows GUI and CLI    | Authenticode verification bound to each executable's leaf certificate SHA-256 fingerprint                                                                                                                                                      |
| Windows NSIS installer | Authenticode verification bound to the leaf certificate SHA-256 fingerprint and Tauri updater verification bound to the public key embedded in `tauri.conf.json`                                                                               |
| Linux AppImage         | Tauri updater verification bound to the embedded public key                                                                                                                                                                                    |
| Linux Debian           | Matching public checksum and source-bound GitHub attestation, plus the `deb_checksum_attestation_only` limitation                                                                                                                              |
| macOS universal DMG    | DMG Developer ID, notarization submission, and stapled-ticket proof; plus separate Developer ID, notarization, and staple proof for the app mounted from the DMG. The container and contained app must report the same Developer ID authority. |
| macOS updater archive  | Tauri updater verification bound to the embedded public key, plus separate Developer ID, notarization, and staple proof for the app extracted from the exact verified archive bytes                                                            |
| Sidecars and metadata  | Updater-signature sidecars, updater and checksum manifests, and the provenance bundle carry no package-signature claim; integrity remains bound by the asset digest and source attestation                                                     |

Every asset also requires an attestation bound to `refs/heads/main`, the packet source commit, and `.github/workflows/release.yml` in this repository.

Real trust identities use closed formats: lowercase SHA-256 fingerprints for Authenticode and stapled tickets, the complete `Developer ID Application: Name (TEAMID)` authority, Apple notarization submission IDs, and the exact updater-key fingerprint derived from the configured public key. Synthetic identity strings are accepted only in schema fixtures.

## Validation

Validate one or more packets before publishing or indexing them:

```sh
node scripts/validate-release-evidence-packet.mjs docs/evidence/releases/<tag>/<platform>.json
```

The validator rejects malformed or mismatched commits, tags, digests, release URLs, workflow URLs, asset URLs, package-role mappings, trust identities, and check sets. It also rejects POSIX, Windows, UNC, home-relative, field-prefixed, and environment-expanded local paths; shell, colon, or JSON environment dumps; authorization headers and bearer tokens; credential assignments; private-key material; multiline output; and raw log fields. This scan traverses every packet string and recognizes sensitive values at the start of a string or after punctuation, including when wrapped in parentheses, brackets, braces, or quotes. Assignment discovery overlaps, so a benign outer observation such as `note=` cannot hide an inner credential or environment dump. Ordinary observations such as `theme=dark`, `interval=1s`, `mode=release`, and `status=failed` remain valid near the same punctuation because they do not use sensitive environment or credential keys and are not environment-style assignment clusters.

Keep diagnostic logs and screenshots in access-controlled workflow artifacts according to their retention policy. Commit only the small sanitized observation needed to understand the check result. A packet must never embed an environment dump or substitute raw output for a result summary.

## Publication layout

Real evidence packets use `packet_kind: release_evidence` and belong under `docs/evidence/releases/<tag>/`. A final release index may reference each platform packet by repository-relative file name and record its SHA-256 digest. The index must not rewrite packet contents or turn a failed or blocked packet into a passing release result.

Publishing a packet does not replace native execution or public-release verification. The packet records that evidence after it exists; it cannot manufacture it.

The parameterized install smoke harness is documented in [Public-artifact install smoke harness](public-artifact-install-smoke.md). Its current public surface cannot create native proof: fixture runs emit only a normalized `schema_fixture`, plans emit no packet, and an unsettled timeout emits no packet. `release_evidence` remains reserved for a future reviewed branded native executor.

## Synthetic fixtures

The Windows NSIS, Linux Debian, Linux AppImage, macOS DMG, and macOS updater examples under `docs/evidence/releases/fixtures/v1/` are schema fixtures only. They use the reserved `v0.0.0-evidence.*` tag, `packet_kind: schema_fixture`, synthetic identities and observations, and the required `synthetic_fixture_no_release_claim` limitation. Every fixture check and limitation disposition must remain `not_applicable`; a fixture that claims passed proof or an accepted release decision is invalid. These files exercise the contract and must not be cited as release evidence.
