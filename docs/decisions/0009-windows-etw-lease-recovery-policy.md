# Windows ETW lease recovery policy

- Status: accepted as a source contract; native ownership is not wired
- Date: 2026-07-14
- Issue: [#70](https://github.com/TheGreenCedar/BatCave/issues/70)
- Service boundary: [#69](https://github.com/TheGreenCedar/BatCave/issues/69)

## Decision

Recover a BatCave Event Tracing for Windows (ETW) process-network session only from one protected `EtwLeaseV1` whose complete identity agrees with the installed service and the observed native session. Never reclaim by a BatCave-looking name or provider alone.

This change adds the pure recovery decision before native storage and session control. The collector service still constructs `TelemetryCollector::for_collector_service()` with process-network ETW disabled. No code in this slice writes a lease, acquires a mutex, starts or stops a session, or changes runtime quality.

## Lease identity

The versioned lease records:

- `intent`, `active`, or `stopping` phase;
- install ID, service generation, and service instance ID;
- Windows boot identity;
- controller PID plus process creation time; and
- exact session name, provider identity, session flags, and configuration digest.

The future native owner must store the lease under service-owned protected storage with atomic replace and flush semantics. It must hold a machine-wide mutex restricted to the service SID and administrators before reading the lease or observing ETW state. Storage trust and atomic persistence are inputs to this policy, not claims made by the source-only model.

## Recovery decisions

| Decision | Required observation | Allowed next action |
| --- | --- | --- |
| `StartFresh` | no lease and no session; an exact trusted stale lease whose controller and session are both proven absent; or a trusted prior-boot lease after current-boot session absence is proven | create and flush a new `intent` lease before `StartTrace` |
| `ReclaimExact` | trusted, well-formed, version-matched lease; exact install, generation, boot, and session identity; old controller proven dead; exact session still present | advance to `stopping`, stop only that exact session, then prove absence |
| `Conflict` | corrupt or untrusted lease, identity drift, live exact controller, session without a trusted lease, or observed-session mismatch | leave the lease and session untouched; keep attribution unavailable |
| `Retain` | session/controller query unavailable, controller evidence is for the wrong PID, or a stop attempt failed | retain the lease, do not start a replacement, and exit or retry only through a later bounded recovery pass |

A PID match is insufficient. PID plus process creation time identifies the controller. The process observation must be for the recorded PID; an arbitrary process observation proves nothing. A reused recorded PID with a different creation time proves the old controller is gone, but it does not authorize reclaim unless every lease and session field also matches.

## Crash ordering

The native implementation must use this order:

1. acquire the protected machine-wide mutex;
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

The cross-platform unit matrix covers fresh start, every crash phase, exact reclaim, live controller, PID reuse, stale lease, corrupt and untrusted storage, schema and identity drift, session-query ambiguity, exact-session mismatch, prefix/provider-only matches, and stop failure retention.

Focused command:

```sh
cargo test --manifest-path src/BatCave.App/src-tauri/Cargo.toml collector_service::etw_lease::tests
```

All-target Clippy and the repository validation workflow must also pass. Repository search must continue to show that `TelemetryCollector::for_collector_service()` disables process-network ETW.

## Non-claims and follow-up

- No native lease file, ACL, atomic write, mutex, ETW query, start, stop, or consumer path exists in this slice.
- No Windows crash, restart, reboot, upgrade, uninstall, second-instance, or multi-user behavior is proven.
- No helper path is deleted, and #69 desktop cutover and installer provisioning remain open.
- No runtime snapshot or release evidence may describe process-network attribution as service-native from this decision alone.

#70 remains open for native wiring, bounded settlement, lifecycle proof, multi-user behavior, upgrade/uninstall cleanup, and legacy-helper removal.
