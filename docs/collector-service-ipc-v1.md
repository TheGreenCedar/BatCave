# Collector service IPC v1

BatCave's Windows collector service uses a separate local protocol from runtime protocol v3. This contract carries an immutable, unshaped collector snapshot to the standard-user desktop. The desktop remains responsible for query filtering, grouping, sorting, contributor selection, and presentation.

## Request surface

| Operation | Purpose | Mutable service state |
| --- | --- | --- |
| `negotiate` | Select protocol v1 and bind the session to a transport-verified desktop executable | No |
| `service_identity` | Read service, release, instance, protocol, minimum-desktop, and limit identity | No |
| `latest_snapshot` | Read the newest immutable raw collector snapshot, optionally after an observed sequence | No |
| `ping` | Prove that the authenticated session is responsive | No |
| `disconnect` | End the client session | No |

Unknown operations and fields fail as `malformed`. The protocol has no caller-supplied path, file, process, command, query, pause, cadence, installer, or service-control operation.

## Bounds

| Boundary | v1 limit |
| --- | ---: |
| Length-prefixed JSON frame | 8 MiB |
| Processes in one snapshot | 5,000 |
| Any string | 32 KiB UTF-8 |
| Concurrent clients advertised by the service | 8 |

The decoder reads a four-byte little-endian frame length and never allocates a claimed payload larger than the frame limit. It also limits one decoded batch to 64 frames. Snapshot validation separately bounds warnings, logical CPUs, kernel-pool tags, driver candidates, numeric percentages, and process-count consistency.

## Identity and authorization

Client JSON carries only the desktop release identity needed for negotiation. It cannot claim a process ID, logon session, token, executable path, or authorization result.

Before any request is authorized, the transport creates `VerifiedPeer` from operating-system evidence. Negotiation succeeds only when the release claimed in the request equals the independently verified executable release. Every later request stays bound to the same process ID and start time, logon session, principal, executable-file identity, and release. Process-ID reuse or executable replacement breaks the session binding.

The desktop side has the same fail-closed rule: it accepts a claimed service identity only alongside a `VerifiedServicePeer` supplied by SCM, pipe-server process, Local System token, canonical image, file-identity, and executable-version evidence. At startup, the service grants the local Interactive group only `PROCESS_QUERY_LIMITED_INFORMATION` on its process and `TOKEN_QUERY` on its primary token so an unelevated desktop can collect that evidence; System and Administrators retain full access, and no mutation, duplication, or impersonation right is exposed. The full service `ProductVersion` must match the desktop before negotiation. Neither peer can establish its own transport identity through JSON. `source_commit_sha` remains optional because neither executable exposes independently readable commit metadata in its Windows version resource.

The response carries the fixed service name, service and release versions, service instance ID, protocol version, minimum desktop version, and exact contract limits. Snapshot sequence numbers must increase within that service instance. Duplicate, regressing, or different-instance frames fail as `stale_sequence`; an `unchanged` response is valid only for the exact previously observed sequence.

## Structured failures

The v1 error codes are `incompatible`, `unauthorized`, `malformed`, `oversized`, and `stale_sequence`. Error detail is bounded by the same string limit.

The Windows host and both transport directions consume this contract in `windows_service.rs`, `windows_transport.rs`, and `windows_client.rs`; [Windows collector service host](windows-collector-service-host.md) records their security, fallback, and lifecycle boundaries. Installer provisioning remains separate work.
