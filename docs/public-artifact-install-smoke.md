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

## Future native executor

Native proof requires a separately reviewed, process-local branded executor capability. Adding a caller-visible boolean, kind string, or injectable function is insufficient. That executor must perform file identity, `lstat`/symlink and containment checks, real-path resolution, hashing, package trust validation, and consumption of the verified bytes as one capability-bound operation. Its tests must swap or replace the file between verification and consumption and demonstrate fail-closed behavior.

Until that capability exists, this harness supplies a durable contract and fixture suite only. It does not publish releases, sign packages, run installers, prove accessibility, prove Windows service or ETW behavior, prove updater expiry or A-to-B updates, or satisfy the stable-release gate.
