# Windows collector service host

BatCave now has a dedicated `batcave-collector-service.exe` source target. The binary enters the Windows Service Control Manager (SCM) dispatcher as the `BatCaveCollector` own-process service, starts the shared immutable collector engine, and serves the frozen collector-service IPC v1 contract over a local named pipe.

This is the host and transport boundary. It does not install or start the service outside SCM, change the desktop runtime source, modify NSIS, or claim installed Windows behavior.

## Service and pipe identity

- SCM service name: `BatCaveCollector`
- Binary target: `batcave-collector-service.exe`
- Pipe: `\\.\pipe\BatCaveCollector.v1`
- Pipe instances: at most 8, matching the advertised protocol limit
- Kernel pipe buffers: 64 KiB in each direction
- Protocol frame: four-byte little-endian length plus at most 8 MiB JSON
- Client idle timeout: 30 seconds
- Response write timeout: 5 seconds
- Requests per connection: at most 4,096

The first pipe instance uses `FILE_FLAG_FIRST_PIPE_INSTANCE` to fail if another process has already claimed the service name. Every instance sets `PIPE_REJECT_REMOTE_CLIENTS`. The protected discretionary access-control list grants full control to Local System and read/write access to local interactive users; it grants nothing to Everyone, Anonymous, Network, or the general Authenticated Users group.

## Client verification

The pipe access-control list is only the first gate. Before reading a request, the service derives the client process and session from the connected pipe, then verifies all of these facts:

1. The pipe process ID is stable before and after inspection.
2. The process creation time is nonzero, so process-ID reuse changes the session binding.
3. The process token and impersonated pipe token have the same user SID and session as the pipe.
4. Both tokens are standard-user tokens, not elevated tokens.
5. The canonical executable is exactly `batcave-monitor.exe` beside the service binary.
6. The executable file identity is bound to its volume and file index.
7. The executable's fixed file version and full `ProductVersion` string match the service package version, including any prerelease suffix.

Only then does the transport create `VerifiedPeer`. The JSON negotiation release must still match that transport-derived peer. A changed PID, start time, session, principal, file identity, release, path, or elevation state fails closed. The release identity deliberately omits `source_commit_sha`: the current executable resource proves the full package version but does not embed independently readable commit metadata.

## Request and shutdown behavior

Each accepted connection gets an independent authorization session. Negotiation is mandatory; all later requests remain bound to the original verified peer. Framing, request batches, payload fields, process counts, strings, and snapshot sequences retain the bounds in [Collector service IPC v1](collector-service-ipc-v1.md). Any malformed or unauthorized request receives at most one request-bound structured failure and then the connection closes.

SCM stop and shutdown controls set the service stop signal. The nonblocking listener and client loops observe that signal, client workers join, and the collector engine shuts down before the service reports `SERVICE_STOPPED`.

## Remaining #69 work

- Connect the standard-user desktop to this pipe and preserve local standard-access fallback.
- Add NSIS provisioning, service ownership, account, start mode, and permissions.
- Prove installed happy-path, denied-install/upgrade, missing-service, stopped-service, unauthorized-client, and incompatible-version behavior on Windows.
- Capture fresh native Tauri evidence for the access and diagnostic states.

#70 still owns crash recovery, reboot/upgrade behavior, ETW lease cleanup, multi-user lifecycle proof, and legacy-helper removal.
