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

Real evidence packets use `packet_kind: release_evidence` and belong under `docs/evidence/releases/<tag>/`. The final review input for that tag is `docs/evidence/releases/<tag>/index.json`, using the structural schema at `docs/evidence/releases/release-evidence-index.schema.json`.

The index records one release identity and a sorted reference for each selected platform packet. Every reference contains:

- the exact packet ID, canonical repository-relative path, and SHA-256 of the complete packet file;
- the declared support profile and exact package role selected by that packet; and
- the selected public asset name, byte size, evidence SHA-256, GitHub API digest, and anonymous download URL.

Validate the assembled index from the repository root:

```sh
node scripts/validate-release-evidence-index.mjs docs/evidence/releases/<tag>/index.json
```

The validator reads every referenced file and runs the existing packet validator before comparing identities. All packets must agree on repository, tag, channel, source/main/release-target commit, release URL, workflow run and attempt, and support-contract version. The reference must reproduce the packet's profile, package role, and selected asset exactly. Paths must be canonical and remain directly under the tag directory; missing, linked, duplicate, reordered, digest-drifted, or cross-release packet files fail closed.

Coverage is derived from `platform-support-contract.v1.json`, rather than maintained as a second hand-authored list. Every declared support profile must appear, and each distinct public package role must appear exactly once. This binds Windows NSIS, Linux deb, Linux AppImage, macOS DMG, and macOS updater-archive evidence without turning one host packet into evidence for a different profile or package.

The index has no accepted, passed, or overall disposition field. Its mandatory `independent_review_and_live_publication_required` non-claim makes it a review input only. Publishing a packet or index does not replace native execution, public-release verification, signed updater proof, or #76's independent final review. These files record evidence after it exists; they cannot manufacture it.

The parameterized install smoke harness is documented in [Public-artifact install smoke harness](public-artifact-install-smoke.md). The [native executor boundary](native-install-smoke-executor.md) closes selected-byte ownership and validates the future packet mapping, but no platform adapter can mint its internal native execution receipt. Fixture runs emit only a normalized `schema_fixture`; plans and the source-slice executor emit no release packet. `release_evidence` remains reserved for a complete reviewed native run.

## Synthetic fixtures

The Windows NSIS, Linux Debian, Linux AppImage, macOS DMG, and macOS updater examples under `docs/evidence/releases/fixtures/v1/` are schema fixtures only. They use one reserved `v0.0.0-evidence.*` release identity so the synthetic index can exercise cross-packet binding, plus synthetic observations and the required `synthetic_fixture_no_release_claim` limitation. Every fixture check and limitation disposition remains `not_applicable`; a fixture that claims passed proof or an accepted release decision is invalid.

`docs/evidence/releases/fixtures/release-evidence-index.v1.json` references those five packet fixtures by their real repository file digests. It uses `index_kind: schema_fixture` and carries both mandatory non-claims. The validator rejects those packets if the index is relabeled as a real release index. The fixture set exercises schema and hostile validation only; it must never be cited as native, public-release, or acceptance evidence.
