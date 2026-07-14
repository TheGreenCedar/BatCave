# Public-artifact install smoke harness

The install smoke harness turns one already-verified public package into an ordered native test plan and a sanitized result. It does not provide platform commands. A native lane supplies an explicit adapter that owns local paths and performs the package-specific work.

The harness has three source boundaries:

- `scripts/install-smoke-contract.mjs` validates the closed input and produces the versioned plan;
- `scripts/public-artifact-install-smoke.mjs` runs injected adapter actions in order; and
- `scripts/install-smoke-evidence.mjs` derives the result state and maps actual runs into the release-evidence packet contract.

## Public verification boundary

Native and plan modes accept only the frozen in-memory receipt returned by a successful `verifyPublicRelease` call. Copying, serializing, reconstructing, or editing that receipt removes its process-local identity and the harness rejects it before calling any adapter action.

This forces a native smoke driver to compose the two operations in one process:

```js
import { verifyPublicRelease } from "./scripts/verify-public-release.mjs";
import { runInstallSmoke } from "./scripts/public-artifact-install-smoke.mjs";

const verification = await verifyPublicRelease(candidate, publishedRelease, publicDownloadRoot);
const input = {
  schema_version: 1,
  execution_kind: "native",
  app_version: "0.3.0",
  evidence_template: blockedEvidenceTemplate,
  public_verification: verification.receipt,
  isolation,
};
const adapter = createNativeAdapter({ publicDownloadRoot, isolatedRoots });
const result = await runInstallSmoke(input, adapter);
```

`verifyPublicRelease` has already checked the immutable release state, anonymous URLs, exact asset inventory, byte sizes, SHA-256 digests, checksum manifest, and source-bound attestations. The native adapter must still rehash the selected local file immediately before mutation and verify its package-specific trust identities. A caller-authored `disposition: passed` object is never sufficient.

## Closed package profiles

| Platform path | Preparation action | Runtime install identity | Required trust basis |
| --- | --- | --- | --- |
| Windows NSIS | `install_nsis` | `nsis` | Authenticode and Tauri updater signatures |
| Linux deb | `install_deb` | `deb` | Public checksum and source-bound attestation |
| Linux AppImage | `stage_appimage` | `appimage` | Tauri updater signature |
| macOS DMG | `install_dmg_app` | `app_bundle` | Developer ID, notarization, and staple evidence for the DMG and contained app |
| macOS updater archive | `stage_updater_archive_app` | `app_bundle` | Contained-app Developer ID, notarization, staple, and Tauri updater evidence |

The updater archive path stages the app extracted from the exact verified archive. It is evidence for the archive contents and staged app; it is not a normal installer run and does not prove an A-to-B in-app update.

## Ordered gates

Every plan contains all of these gates in this order:

1. prior anonymous download and checksum verification;
2. package-specific trust verification;
3. an immediate local rehash, regular-file check, symlink rejection, and adapter-root containment;
4. install or preparation in an isolated install root;
5. launch;
6. exact app version, source commit, and install-kind identity;
7. same-version restart with settings preserved;
8. a platform-supported degradation observation;
9. telemetry with an explicit native or limited quality state; unavailable telemetry must block the gate;
10. application removal;
11. owned runtime residue check; and
12. the declared user-state preservation or removal policy.

Missing adapter actions, extra actions, unsafe executor settings, duplicate or reordered results, partial observations, identity drift, timeout, unsupported capability, unsafe output, and unexpected residue fail closed. Runtime gates stop after a failure, while bounded cleanup still runs after package mutation was attempted.

## Adapter contract

An adapter supplies one function for every action in its plan and declares the executor properties before the first action can run:

- tokenized argument vectors rather than command strings;
- `shell: false`;
- a minimal environment;
- bounded captured output;
- timeout cancellation that terminates the process tree; and
- adapter-owned mappings from the plan's opaque root IDs to isolated local paths.

Actions receive an `AbortSignal`, immutable release/platform/asset identity, the opaque isolation IDs, and the safety constraints. They return only a short sanitized outcome and the closed observations required by that gate. Local paths, raw output, environment dumps, and credentials are rejected from results and evidence.

The Windows profile always records `windows_service_behavior: not_assumed`. This harness does not assume the unfinished collector service, prove Event Tracing for Windows (ETW), or turn standard-access observations into elevated-service evidence.

Windows cannot produce `native_proven` evidence until the exact public NSIS bytes carry the Authenticode identity required by the release evidence template and the native trust adapter observes that same identity. The current unsigned prerelease path therefore remains blocked by #42.

## Result and evidence states

The version 1 result keeps lifecycle detail outside the fixed #98 packet statuses:

| Result disposition | Meaning | Evidence output |
| --- | --- | --- |
| `planned` | Adapter actions were not invoked | `null` |
| `fixture` | Synthetic adapter exercise only | `schema_fixture`; every check remains `not_applicable` |
| `skipped` | A required native action was unsupported, skipped, or blocked | `release_evidence` with affected checks `blocked` |
| `failed` | A required native action failed or timed out | `release_evidence` with affected checks `failed` or `blocked` |
| `native_proven` | Every required native gate passed | `release_evidence` with mapped checks `passed` |

The mapper never adds planned, fixture, timeout, unsupported, or skipped to the #98 status vocabulary. Fixtures cannot become release evidence, and plans emit no packet. A schema-valid packet can still record a failed or blocked run; schema validity is not release approval.

## Native-lane handoff

A later native lane should:

1. allocate fresh install and user-state roots and keep their real paths inside the adapter;
2. run `verifyPublicRelease` anonymously and retain its downloaded directory for the adapter closure;
3. build a blocked `release_evidence` template with the exact workflow, release, asset, attestation, and signature identities;
4. create the plan and review every required action before execution;
5. run the native adapter, validate the returned result, and attach only its sanitized evidence packet; and
6. retain raw logs and screenshots as access-controlled workflow artifacts, not packet fields.

The harness does not publish a release, sign packages, create platform adapters, execute installers in repository tests, prove accessibility, prove updater expiry or A-to-B behavior, or satisfy the final stable-release gate by itself.
