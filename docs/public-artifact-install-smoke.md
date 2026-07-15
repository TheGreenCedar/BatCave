# Public-artifact install smoke harness

The install-smoke harness defines the closed platform profiles, gate order, isolation contract, and sanitized result shape for testing a public package. Its current executable surface supports planning and synthetic fixtures only. It cannot execute an installer in repository tests, produce native proof, or emit `release_evidence`.

The boundaries are:

- `scripts/install-smoke-contract.mjs` validates input and creates a versioned plan;
- `scripts/public-artifact-install-smoke.mjs` runs fixture actions in order; and
- `scripts/install-smoke-evidence.mjs` derives the result and builds a normalized `schema_fixture` packet.

## Proof boundary

Plan mode accepts only the frozen in-process receipt returned by `verifyPublicRelease`. The receipt is marked `proof_scope: contract_only`: it proves that the public-verification prerequisite ran, but it is never native-install authorization. Copying, reconstructing, or editing the receipt removes its process-local identity and fails before any action.

Fixture mode uses an explicit `fixture_only` receipt. Injected actions, executor booleans, adapter-authored observations, and caller-authored `execution_kind: native` values cannot mint native eligibility. The only accepted execution kinds are `plan` and `fixture`; `native_proven` and `release_evidence` are unreachable.

Plans bind the evidence template's observation time and exact workflow file, run ID, run attempt, and run URL. They retain only the selected receipt-bound package asset. Other template assets and signature claims are not copied into action context.

The harness also carries a frozen process-local selected-identity receipt through each plan and result. Standalone validation compares the exact plan ID, observation time, release, platform, asset name, size, digest, and URL against that receipt. Copying the receipt or consistently rewriting both a fixture packet and its top-level asset does not preserve the receipt's process identity and fails validation.

## Closed package profiles

| Platform path | Package operation | Fixture action | Runtime identity | Required limitation |
| --- | --- | --- | --- | --- |
| Windows NSIS | install | `install_nsis` | `nsis` | service and ETW behavior out of scope |
| Linux deb | install | `install_deb` | `deb` | checksum and source attestation are the trust basis |
| Linux AppImage | stage | `stage_appimage` | `appimage` | none beyond fixture-only status |
| macOS DMG | install | `install_dmg_app` | `app_bundle` | none beyond fixture-only status |
| macOS updater archive | stage | `stage_updater_archive_app` | `app_bundle` | staging only; no normal package install or A-to-B update claim |

Every fixture packet also carries `synthetic_fixture_no_release_claim`. The Windows limitation explicitly keeps service and Event Tracing for Windows behavior outside the contract.

## Ordered gates

Every plan contains these gates in order:

1. anonymous public download and checksum prerequisite;
2. package-trust contract;
3. selected-asset rehash, regular-file, symlink, and containment contract;
4. package installation or staging;
5. launch;
6. exact app version, source commit, and package identity;
7. restart with settings preserved;
8. supported degradation observation;
9. telemetry with a native or limited quality state;
10. application removal;
11. owned-runtime residue check; and
12. the declared user-state policy.

Fixture observations at the trust and rehash gates are self-attestations. They exercise validation and ordering but prove neither the file inspected nor the bytes later consumed. They always map to `not_applicable` fixture checks.

## Fixture executor and timeout settlement

A fixture adapter supplies one explicit own data function for every action and declares tokenized arguments, `shell: false`, a minimal environment, bounded output, process-tree cleanup, and a termination-confirmation function. Actions receive an abort signal and immutable context containing the workflow identity, observation time, selected asset, profile, opaque isolation identifiers, and constraints.

When an action times out, abort is only the first step. The harness waits for a separate bounded termination handshake confirming both that the action settled and that its process tree settled. Confirmed termination records `timeout` and permits bounded cleanup. Missing, invalid, or late confirmation records `partial`, blocks every later action including cleanup, and emits no evidence packet.

Local paths, raw output, environment dumps, and credentials are rejected from results.

## Result states

| Disposition | Meaning | Evidence output |
| --- | --- | --- |
| `planned` | The ordered native contract was produced; no actions ran | `null` |
| `fixture` | Synthetic actions ran or failed within the fixture contract | normalized `schema_fixture`; every mapped check is `not_applicable` |
| `partial` | A timeout did not produce a settled termination handshake | `null` |

Result validation rederives the disposition and the full packet. Packet ID, observation time, workflow and release identity, platform, selected asset, every check status and outcome, and the exact limitation set must match. Contradictory or extra evidence fails.

## Native executor boundary

The [native executor source slice](native-install-smoke-executor.md) adds process-local artifact ownership, hostile path and caller-seam tests, closed result validation, and the future #98 evidence derivation. Adding a caller-visible boolean, kind string, or injectable function remains insufficient. The current owned-byte verification receipt is artifact-only and cannot authorize a native disposition.

The built-in JavaScript Linux deb/AppImage source descriptor is registered, and its fixed process-group settlement probes are covered by hosted validation. The macOS profiles also have a reviewed source descriptor that binds verified asset identity and future tool/resource constraints. Those source contracts still have no package command and emit no evidence.

Separately, the production `batcave-install-smoke` Rust entry independently verifies the public release and selected bytes. Its Linux dispatch seals those bytes in a private descriptor and revalidates them in the Linux handler, which currently returns `skipped` without executing a package. Protected post-public release jobs do exercise public deb and AppImage artifacts through separate closed capture paths, but their sanitized outputs remain observations with `release_evidence_eligible: false`; they do not make this JavaScript harness native. Native proof still requires the reviewed adapter to consume owned bytes while it performs trust, runtime, timeout-settlement, removal, and residue gates. The harness does not publish releases, sign packages, run installers, prove accessibility, prove Windows service or ETW behavior, prove updater expiry or A-to-B updates, or satisfy the stable-release gate.
