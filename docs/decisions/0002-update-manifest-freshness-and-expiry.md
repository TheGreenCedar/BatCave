# Update manifest freshness and expiry

- Status: accepted for the current GitHub Releases updater
- Date: 2026-07-14
- Related issues: [#47](https://github.com/TheGreenCedar/BatCave/issues/47), [#76](https://github.com/TheGreenCedar/BatCave/issues/76), [#105](https://github.com/TheGreenCedar/BatCave/issues/105)

## Decision

BatCave will not add an `expires_at` field or a custom signed manifest envelope to the current updater. Issue #47 should drop its requirement to reject “expired” and “mismatched-channel” update signatures. Tauri payload signatures have neither property: they authenticate exact payload bytes under the embedded updater key, but do not authenticate the manifest, expire, or bind a payload to a release channel.

For the current stable updater, the supported contract is:

1. Update checks happen only after the user selects **Check now**. Startup and monitoring do not depend on the network.
2. BatCave requests only `https://github.com/TheGreenCedar/BatCave/releases/latest/download/latest.json`. GitHub's latest-release routing supplies the stable, non-prerelease channel.
3. Tauri offers an update only when its SemVer is greater than the installed app version. BatCave does not enable downgrades.
4. Tauri downloads the selected payload and verifies the exact bytes against the manifest-provided Minisign signature and the public key embedded in the app before installation.
5. Manifest routing and publication integrity rely on GitHub HTTPS, release access controls, and immutable publication. BatCave makes no end-to-end cryptographic claim for manifest freshness, stable-channel binding, or replay resistance to an older release that is still newer than the installed app.
6. BatCave retains selected manifest and update data ephemerally in process, not only while the frontend considers an update pending. The frontend `Update` holds `rawJson` and a resource ID; Tauri's in-memory resource holds the selected URL, signature, and `raw_json`. Dropping or replacing the JavaScript object does not close that Rust resource, and `downloadAndInstall()` does not close it. BatCave therefore calls `Update.close()` before replacing a selected update and after download/install completion or failure. An unexpected cleanup failure retains the reference for Retry and blocks another check until cleanup succeeds. BatCave persists no manifest, update authorization, trusted clock, highest-seen version, or anti-rollback state. This resource lifetime is an in-memory cleanup concern, not persisted security state. A failed check or payload verification changes no installed files and leaves monitoring available.

This is a scope correction, not a claim that signed freshness metadata has no value. A future requirement for freeze-attack or cryptographic channel protection must use a renewable metadata design with explicit trusted-time, anti-rollback state, rotation, availability, and recovery semantics. A signed timestamp added to an immutable file is not that design.

## Current verification path

BatCave's `package.json` pins `@tauri-apps/plugin-updater` to 2.10.1. Rust's `Cargo.toml` uses the compatible-version requirement `2.10.1`, and `Cargo.lock` resolves `tauri-plugin-updater` 2.10.1. The repository generates the following static manifest shape, abridged to one platform target:

```json
{
  "version": "0.3.0",
  "notes": "BatCave v0.3.0",
  "platforms": {
    "windows-x86_64": {
      "signature": "<payload signature>",
      "url": "<immutable release asset>"
    }
  }
}
```

The source path is:

1. [`tauri.conf.json`](../../src/BatCave.App/src-tauri/tauri.conf.json) embeds the updater public key and the stable GitHub endpoint.
2. [`build-update-manifest.mjs`](../../scripts/build-update-manifest.mjs) binds the release version to exact platform asset names, URLs, and payload signatures. It emits no channel, issuance time, expiry, metadata version, or manifest signature.
3. [`App.svelte`](../../src/BatCave.App/src/App.svelte) calls `check({ timeout: 15_000 })` only on explicit user action. Before each check it closes the prior `pendingUpdate`; a successful check replaces that reference with the returned JavaScript `Update`. If cleanup fails for any reason other than Tauri's exact already-removed error, BatCave retains the prior reference and does not start another check.
4. The pinned Tauri source fetches and parses JSON before applying the default `remote version > installed version` comparison. On success, [`commands.rs` stores the Rust `Update` in the webview resource table](https://github.com/tauri-apps/plugins-workspace/blob/d6a3898001a4bcc659e045f9501498751b77dbe6/plugins/updater/src/commands.rs#L71-L94) and returns its resource ID plus a clone of `raw_json` to JavaScript. The resource retains the selected URL and signature, but interprets no channel or expiry policy.
5. `downloadAndInstall()` looks up that in-memory resource, clones it for the operation, downloads the selected URL, verifies the payload bytes, and installs only after verification. The plugin command does not close the stored `Update` resource itself, so BatCave closes the JavaScript `Update` in a `finally` path after completion or failure. [Tauri's `Resource` contract keeps the Rust resource until explicit close or application exit](https://github.com/tauri-apps/tauri/blob/6f6ab1207bb3923c2721fbc67d2fdb1c8deb0c7a/packages/api/src/core.ts#L293-L334). Neither side writes this update selection to disk.
6. After a download or install failure, [`App.svelte`](../../src/BatCave.App/src/App.svelte) closes the selected resource and sets the update status to `error`. A cleanup failure remains visible and retains the reference so Retry can attempt cleanup again before checking. [`SettingsDrawer.svelte`](../../src/BatCave.App/src/lib/components/shell/SettingsDrawer.svelte) routes **Retry** to `onCheckForUpdates`, which fetches metadata again only after cleanup. If an update is still available, the next **Download and install** action downloads it from the new selection; Retry does not reuse the previous JavaScript `Update` for installation.

Tauri's [official updater documentation](https://v2.tauri.app/plugin/updater/) describes the public key as validating updater artifacts before installation, the GitHub static-JSON endpoint, the required manifest fields, the default forward-only comparator, and runtime public-key replacement for rotation. The exact pinned upstream source shows [manifest parsing and the default comparator](https://github.com/tauri-apps/plugins-workspace/blob/d6a3898001a4bcc659e045f9501498751b77dbe6/plugins/updater/src/updater.rs#L474-L552), the [`raw_json` handoff](https://github.com/tauri-apps/plugins-workspace/blob/d6a3898001a4bcc659e045f9501498751b77dbe6/plugins/updater/src/updater.rs#L601-L624), and [payload-byte signature verification before installation](https://github.com/tauri-apps/plugins-workspace/blob/d6a3898001a4bcc659e045f9501498751b77dbe6/plugins/updater/src/updater.rs#L648-L728).

The Minisign signature's trusted comment commonly contains a timestamp, but the pinned verifier [authenticates that comment as data](https://github.com/jedisct1/rust-minisign-verify/blob/3a91d03f86a8462a1af953c2854687d3f953d541/src/lib.rs#L334-L370) and never compares it with a clock. It is not an expiry. Windows Authenticode and Apple Developer ID/notarization are separate OS package and publisher checks; neither authenticates `latest.json`.

## Required behavior

| Scenario | Required result |
| --- | --- |
| No valid metadata can be fetched, including a non-success response or missing `latest.json` | Report a generic update-check failure; do not download or install; monitoring continues. Pinned Tauri does not expose the HTTP status here, so BatCave cannot distinguish “no stable release” from “manifest missing.” |
| Manifest is malformed or lacks the current target | Report the same generic update-check failure; do not download or install; monitoring continues. |
| Endpoint returns 204 No Content | Report that BatCave is up to date; no update resource is created. |
| Endpoint is unreachable, TLS fails, or the device is offline | Report that the update service could not be reached; startup and monitoring continue. |
| Remote SemVer equals or is lower than the installed version | Report no update. Never opt into Tauri's downgrade comparator. |
| Downloaded bytes do not match the updater signature | Reject before installation and state that verification or installation failed. Leave the installed app unchanged. |
| A previous manifest is replayed and its version is at or below the installed version | Tauri's comparator rejects it. |
| A previous valid release is replayed and remains above the installed version | It may be offered and installed if its payload signature is valid. This is a documented unsupported freshness case, not an expiry failure. |
| A prerelease signed by the same updater key is substituted by a compromised metadata channel | The payload signature can validate it. Stable-channel separation is an operational GitHub endpoint/release-control guarantee, not a cryptographic payload-signature guarantee. |
| The system clock is early or late | BatCave applies no manifest-time rule. Clock-related HTTPS failures are ordinary check failures and do not affect monitoring. |
| The user retries after payload verification or installation fails | `App.svelte` closes and clears the failed selection, sets the status to `error`, and **Retry** invokes `onCheckForUpdates`. If cleanup itself failed, the reference is retained and Retry attempts that close again before any request. After cleanup, the path fetches metadata again and, if an update is still available, requires a new **Download and install** action that downloads from the new selection. It does not reuse the previous JavaScript `Update` for installation; no update authorization survives process exit. |

GitHub documents that prereleases and drafts cannot be selected as the latest release and that immutable release assets cannot be modified or deleted after publication. Those controls fit BatCave's stable routing and artifact-integrity model, but remain service-side controls rather than a client-verified manifest signature.

## Replay, rollback, and persisted state

The installed app version is the only monotonic floor. This prevents a normal rollback to the same or a lower version. It does not prove that a higher version is the newest version ever published or seen.

BatCave will not persist a highest-seen manifest version for the present unsigned metadata. Such state would let an unauthenticated manifest permanently fast-forward a client and deny future updates. It also would not solve first-contact replay. There is no updater-state migration for this decision.

If stronger replay resistance becomes necessary, the client must persist authenticated metadata versions atomically and reject rollback relative to that trusted state. It must also bound fast-forward values, define state-corruption recovery, and keep offline startup independent from update metadata.

## Expiry and clock semantics

The current contract has no metadata expiry and makes no expiry claim. Payload signatures remain valid as long as their key is trusted and the signed bytes match.

A future expiry design must define all of the following together:

- a separately renewable signed metadata endpoint rather than an immutable release asset;
- a fixed update-cycle start time and strict rejection when signed metadata is expired;
- maximum metadata lifetime and future-issued clock skew;
- persisted authenticated metadata versions and last trusted state;
- behavior for a bad local clock, including that update checks may fail closed while startup and monitoring continue;
- a renewal service, alerting, and an emergency recovery path.

The Update Framework's specification couples expiration checks with persisted metadata versions to address freeze and rollback attacks. BatCave should adopt a maintained TUF-style implementation or an equivalent reviewed system if this threat enters scope, rather than inventing a one-file envelope.

## Key rotation and recovery

The payload-key rotation contract remains the one in [`docs/releases.md`](../releases.md): publish a transition update signed by the old private key whose app embeds the new public key, then sign later updater payloads with the new private key.

This has a deliberate recovery limit. Once the channel moves to payloads signed only by the new key, an older client that missed the transition cannot update in-app. If the old private key is lost or compromised before a safe transition, there is no secure in-band recovery under the existing trust root. The user must install an independently verified release manually. A second manifest signature made by the same key would not improve that failure mode.

## Issue #47 acceptance correction

Replace:

> Reject invalid, expired, mismatched-channel, and tampered update signatures.

with:

> Reject malformed update metadata and any updater payload whose Tauri signature is invalid or does not match the exact downloaded bytes. Use only the configured stable GitHub latest-release endpoint, never enable downgrades, and document that Tauri payload signatures do not expire or cryptographically bind a release channel.

The corresponding hostile-case proof should cover unavailable, missing, and malformed metadata through the current generic fail-closed check UX; equal/lower SemVer; a tampered payload; a signature from the wrong key; offline check behavior; and unchanged startup/monitoring. It should not claim status-aware 404 handling, expiry, or cryptographic channel rejection without later implementation.

The frontend closes the current `Update` explicitly before replacing it on a new check and after download/install completion or failure. That cleanup bounds abandoned Rust resources; it does not add persisted authorization, freshness, or anti-rollback state.

## Follow-up scope

The deterministic [local hostile-case matrix](../updater-hostile-fixtures.md) proves the corrected non-mutating boundary without publishing a release. It covers explicit resource cleanup, generic fail-closed behavior for unavailable, missing, empty, malformed, and wrong-target metadata; equal/lower-version suppression; wrong-key, invalid-signature, and byte-tamper rejection; and request failure. The loopback server is a fixture, but metadata parsing, comparison, download, and signature verification execute through Tauri's pinned native updater path rather than a frontend signature mock.

The real public A-to-B install remains a later #47/#76 gate after signed stable-quality artifacts exist. That proof should add public endpoint routing, exact downloaded bytes, installed-version transition, preserved settings, and failure recovery. It should cite this decision so the evidence does not imply manifest expiry or cryptographic channel binding.

If product risk later requires freeze-attack or cross-channel protection, open a separate design and implementation child. Its scope must include the renewable metadata publisher, client verifier, persisted trusted state, clock policy, rotation ceremony, monitoring, and recovery; it must not be folded into payload-tamper proof.

## Alternatives rejected

- **Unsigned `expires_at`:** attacker-controlled metadata cannot authenticate its own expiry.
- **Signed envelope stored only in the immutable release:** it eventually expires but cannot be renewed. Publishing on a metadata-refresh cadence is release churn; choosing a long lifetime merely creates a long replay window.
- **Persist the highest unsigned version:** a forged high version can permanently block legitimate updates, and first-contact replay remains.
- **Use different payload keys for stable and prerelease:** this can bind a payload to a client flavor but does not provide freshness or expiry, and it adds another distributed app/key lifecycle when BatCave currently exposes only the stable channel.
- **Reuse the payload signature timestamp:** Minisign's trusted comment is authenticated but the verifier gives it no freshness semantics; treating build time as expiry would require a separate policy and trusted clock.
- **Add a custom mutable endpoint now:** it introduces an online signing/renewal system, availability obligation, persistent client state, and new recovery paths beyond #47's current GitHub Releases scope.
- **Treat Authenticode or notarization as manifest protection:** those mechanisms authenticate platform executables and publishers, not update routing metadata.

## Evidence and confidence

This decision is based on BatCave source at integration commit `c21f916575de9815ea59a181643634363f226d6a`, exact `tauri-plugin-updater` 2.10.1 source from upstream commit `d6a3898001a4bcc659e045f9501498751b77dbe6`, [Tauri's updater documentation](https://v2.tauri.app/plugin/updater/), [GitHub's immutable-release contract](https://docs.github.com/en/code-security/concepts/supply-chain-security/immutable-releases), [GitHub's release API semantics](https://docs.github.com/en/rest/releases/releases), and [The Update Framework specification](https://theupdateframework.github.io/specification/latest/).

Confidence is high for the current implementation boundary and the absence of Tauri expiry/channel enforcement. The residual uncertainty is product risk appetite: a future threat model may justify operating renewable signed metadata, but adding expiry without that system would be misleading and brittle.
