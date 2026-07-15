# Windows ETW lease recovery policy

- Status: accepted as a source contract; durable storage and kernel ownership are dormant
- Date: 2026-07-14
- Issue: [#70](https://github.com/TheGreenCedar/BatCave/issues/70)
- Service boundary: [#69](https://github.com/TheGreenCedar/BatCave/issues/69)

## Decision

Recover a BatCave Event Tracing for Windows (ETW) process-network session only from one protected `EtwLeaseV1` whose complete identity agrees with the installed service and the observed native session. Never reclaim by a BatCave-looking name or provider alone.

The recovery decision is now paired with a dormant durable-store boundary and an exact Windows ownership guard. The collector service still constructs `TelemetryCollector::for_collector_service()` with process-network ETW disabled. No host code constructs the protected-root capability, opens the mutex, writes a lease, starts or stops a session, or changes runtime quality.

## Lease identity

The versioned lease records:

- `intent`, `active`, or `stopping` phase;
- install ID, service generation, and service instance ID;
- Windows boot identity;
- controller PID plus process creation time; and
- exact session name, provider identity, session flags, and configuration digest.

The dormant store uses the fixed `etw-lease.v1.json` leaf below a platform-verified service root. Its protected-root capability can be constructed only through an unsafe boundary whose caller must prove every mutable path component, reject links and reparse points, prove a service-owned directory, and exclude unprivileged writers through inherited access control. Any existing lease or owner-lock leaf must pass the same ownership, type, reparse, and write-access proof. The #69 installer/service-storage lane still owns that native verifier and root provisioning.

Reads are bounded to 16 KiB and classify missing, corrupt, and untrusted bytes separately. Unknown JSON fields remain rejected. Each read returns a root-bound snapshot containing the observed classification and the exact trusted bytes. A write or removal must present that snapshot with ownership authority from the same protected-root capability. Authority from root A cannot observe or mutate root B, and a snapshot from root A cannot authorize a mutation under root B.

Mutation is compare-before-write within the protected single-owner boundary. The store re-reads the leaf and rejects the operation if its state or exact trusted bytes changed after observation. Corrupt and untrusted observations never authorize replacement or removal. A trusted replacement must preserve the prior install ID, service generation, boot identity, and complete session identity. Replacements serialize only a well-formed schema-v1 lease, write a unique same-directory temporary file, flush it, and atomically replace the leaf. Windows uses `MoveFileExW` with replace and write-through flags; Unix source tests retain the existing rename plus parent-directory synchronization contract. Exact trusted removal synchronizes the parent on Unix; an exact still-absent snapshot is a no-op.

The dormant Windows guard opens the fixed `etw-owner.v1.lock` leaf in that protected root with file sharing disabled. Windows applies sharing checks machine-wide, across logon sessions. Exactly one service process can retain the handle; a second reports contention, and crash or orderly close releases ownership in the kernel while leaving the harmless protected leaf for reuse. The handle also denies delete sharing, so the lock cannot be replaced while held. This root-bound primitive avoids the first-creator squatting risk of a public fixed-name `Global\` mutex.

## Recovery decisions

| Decision | Required observation | Allowed next action |
| --- | --- | --- |
| `StartFresh` | no lease and no session; an exact trusted stale lease whose controller and session are both proven absent; or a trusted prior-boot lease after current-boot session absence is proven | create and flush a new `intent` lease before `StartTrace` |
| `ReclaimExact` | trusted, well-formed, version-matched lease; exact install, generation, boot, and session identity; old controller proven dead; exact session still present | advance to `stopping`, stop only that exact session, then prove absence |
| `Conflict` | corrupt or untrusted lease, identity drift, live exact controller, session without a trusted lease, or observed-session mismatch | leave the lease and session untouched; keep attribution unavailable |
| `Retain` | session/controller query unavailable, controller evidence is for the wrong PID, or a stop attempt failed | retain the lease, do not start a replacement, and exit or retry only through a later bounded recovery pass |

A PID match is insufficient. PID plus a nonzero process creation time identifies the controller. The process observation must be for the recorded PID; an arbitrary process observation or an unknown creation time proves nothing. A reused recorded PID with a different nonzero creation time proves the old controller is gone, but it does not authorize reclaim unless every lease and session field also matches.

## Crash ordering

The native implementation must use this order:

1. acquire the protected machine-wide ownership guard and retain it;
2. write and flush `intent`;
3. start the exact configured session;
4. open the consumer and prove it is running;
5. atomically replace the lease with `active`;
6. replace it with `stopping` before shutdown;
7. stop and query the exact session; and
8. remove the lease only after absence and cleanup are proven.

Crashes in all three phases converge through the same policy. A missing session may clear an exact stale lease only after the recorded controller is proven dead. A present exact session may be reclaimed only under the complete trusted identity. Failed stop keeps `stopping` durable and blocks replacement.

ETW sessions and controller processes do not survive a Windows reboot. A trusted lease from another boot may therefore be discarded only after the current boot proves the session name absent. A present or unqueryable session remains a conflict or retention state; the old boot identity never authorizes stopping a current-boot session.

## Relationship to ETW quality

Lease ownership and sample quality are separate contracts. The parallel #70 quality slice owns supported-event decode proof, loss/error counters, consumer health, and the rule that idle zero is native only while the session and decoder are healthy. Neither slice enables service ETW by itself.

## Verification

The cross-platform unit matrix covers fresh start, every crash phase, exact reclaim, live controller, PID reuse, stale lease, corrupt and untrusted storage, schema and identity drift, session-query ambiguity, exact-session mismatch, prefix/provider-only matches, stop failure retention, atomic phase replacement, invalid-write preservation, bounded corrupt input, untrusted leaf refusal, exact removal, cross-root authority and snapshot rejection, corrupt-state mutation refusal, identity-conflicting replacement refusal, and stale-observation refusal. Hosted Windows additionally proves first-owner acquisition, simultaneous second-owner rejection, kernel release, reacquisition, and lease access only through the retained root-bound guard.

Focused command:

```sh
cargo test --manifest-path src/BatCave.App/src-tauri/Cargo.toml collector_service::etw_lease::tests
```

All-target Clippy and the repository validation workflow must also pass. Repository search must continue to show that `TelemetryCollector::for_collector_service()` disables process-network ETW.

## Non-claims and follow-up

- No installed service-storage root, native root verifier, or installer ACL is claimed by this slice; the file store cannot be constructed by the service until #69 supplies that proof.
- The atomic store and Windows ownership guard exist as dormant source boundaries only. The SCM host does not call them.
- No ETW query, start, stop, or consumer path is connected to lease ownership.
- No Windows crash, restart, reboot, upgrade, uninstall, second-instance, or multi-user behavior is proven.
- No helper path is deleted, and #69 desktop cutover and installer provisioning remain open.
- No runtime snapshot or release evidence may describe process-network attribution as service-native from this decision alone.

#70 remains open for native wiring, bounded settlement, lifecycle proof, multi-user behavior, upgrade/uninstall cleanup, and legacy-helper removal.
