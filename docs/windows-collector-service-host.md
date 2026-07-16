# Windows collector service host

BatCave now has a dedicated `batcave-collector-service.exe` source target. The binary enters the Windows Service Control Manager (SCM) dispatcher as the `BatCaveCollector` own-process service, starts the shared immutable collector engine, and serves the frozen collector-service IPC v1 contract over a local named pipe.

The Windows NSIS bundle packages the service binary beside the desktop executable and uses installer hooks to own its SCM and `ProgramData` boundaries. The desktop remains `asInvoker`; the installer never starts the GUI as an elevated child. Installed behavior still requires exact-artifact native proof before it is treated as release evidence.

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

The first pipe instance uses `FILE_FLAG_FIRST_PIPE_INSTANCE` to fail if another process has already claimed the service name. Every instance sets `PIPE_REJECT_REMOTE_CLIENTS`. The protected discretionary access-control list grants full control to Local System and only `FILE_GENERIC_READ | FILE_WRITE_DATA` to local interactive users. It deliberately excludes `FILE_GENERIC_WRITE`, whose named-pipe mapping includes `FILE_CREATE_PIPE_INSTANCE`, and grants nothing to Everyone, Anonymous, Network, or the general Authenticated Users group. The desktop opens only this fixed pipe and requests the exact `FILE_READ_DATA | FILE_WRITE_DATA` rights.

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

The desktop independently authenticates the other direction. It confirms that the pipe server PID is the running `BatCaveCollector` SCM own-process service before and after inspection, opens that PID, requires a Local System token, canonicalizes the process image to `batcave-collector-service.exe` beside `batcave-monitor.exe`, binds its file identity, and reads its full `ProductVersion`. Only that transport evidence can create `VerifiedServicePeer`; JSON cannot supply or override it. A different service `ProductVersion` is reported as incompatible before negotiation. Before authenticated negotiation, that status includes the transport-verified service version but leaves the minimum desktop version unreported.

## Request and shutdown behavior

Each accepted connection gets an independent authorization session. Negotiation is mandatory; all later requests remain bound to the original verified peer. Framing, request batches, payload fields, process counts, strings, and snapshot sequences retain the bounds in [Collector service IPC v1](collector-service-ipc-v1.md). Any malformed or unauthorized request receives at most one request-bound structured failure and then the connection closes.

The desktop waits at most 250 ms for the fixed pipe and gives each request/response operation a two-second deadline. It accepts exactly one request-bound response, requires increasing sample sequences for the negotiated service instance, and reuses an `unchanged` response only when it names the exact cached sequence. Authenticated service timestamps drive only per-service rate deltas; public freshness is stamped when the desktop observes a new snapshot so independent service and desktop clock anchors cannot skew it. A disconnect, timeout, malformed frame, wrong request ID, sequence regression, identity drift, or peer-verification failure closes the client session.

Startup and recovery try the service first. Missing, stopped, incompatible, unauthorized, and failed service states are carried into runtime protocol v3 and the desktop immediately samples through a local standard-access collector. That fallback has process-network ETW disabled, retries the service on a bounded five-second cadence, and adds a visible collector warning instead of presenting protected fields as current. A service/fallback or service-instance transition clears rate baselines before publishing the new source. The desktop manifest remains `asInvoker`; the legacy helper remains available only as migration behavior and is stopped if the authenticated service becomes active.

SCM stop and shutdown controls set the service stop signal. The nonblocking listener and client loops observe that signal, client workers join, and the collector engine shuts down before the service reports `SERVICE_STOPPED`.

This host starts process-network ETW only after acquiring the protected root-bound owner guard and applying the exact recovery policy. A fresh start requires both lease and session absence. A trusted stale lease can be reclaimed only after PID plus process creation time proves the controller dead and the queried session matches every recorded field; the service writes `stopping`, stops the hard-coded session through the returned native handle, proves controller/session absence, and removes the exact lease before writing a new `intent`. Consumer startup advances the lease to `active`; shutdown writes `stopping`, stops the collector engine, requires a clean bounded consumer join, proves native session absence, and only then removes the exact lease. Stop, close, join-timeout, and consumer-panic failures propagate as service shutdown failures and retain the lease. The service binary digest identifies its generation, the protected root's volume/file identity identifies the installation, and Windows supplies the boot identifier. Corrupt, conflicting, mismatched, or unqueryable state stays untouched and keeps the service unavailable.

The monitor still fails closed until a supported event decodes, tracks `ProcessTrace` progress through buffer and event heartbeats, queries the exact session for loss and configuration drift, and requires a clean decoded interval after loss before returning to native quality. A consumer with no progress beyond the bounded heartbeat window is unavailable even when the session query still succeeds.

## Installer ownership

The per-machine NSIS installer is the only entry point allowed to create, reconfigure, stop, or remove `BatCaveCollector`. Its hooks first open the fixed SCM registry key read-only so an inaccessible or malformed registration cannot be mistaken for an absent service, then refuse a same-name service unless a private installer marker, exact quoted Program Files image path, LocalSystem account, and own-process type all match. Before any service mutation, the hooks also use Tauri's app-running guard and require the fixed per-machine install directory. The hooks then invoke only the namespaced fixed service-binary commands `--provision prepare-upgrade`, `--provision install`, and `--provision uninstall`. They do not run `sc.exe`, change ACLs, or recursively remove paths themselves.

The Windows CLI is published separately and is not part of the current NSIS payload. The native provisioner retires the one historical installed CLI only when its fixed name, exact size, and SHA-256 match the recorded product bytes. It opens the leaf without following reparses or sharing writes/deletes, verifies its final path and access control, hashes a bounded byte count, and marks that same handle for deletion. Missing residue is harmless; a directory, reparse, path mismatch, writable object, wrong size, wrong digest, or post-delete replacement aborts before SCM mutation and remains visible as untrusted residue.

The native service installer boundary owns `%ProgramData%\BatCaveMonitor\Service`. It opens path components without following reparse points, creates directories with their final security descriptors, verifies LocalSystem ownership and protected DACLs, and grants write access only to LocalSystem, Administrators, and the `NT SERVICE\BatCaveCollector` SID. The same native boundary owns rollback, SCM query-back and settlement, and bounded uninstall cleanup. Only after verification can the service convert the fixed path into a protected ETW lease capability; a missing or invalid root keeps the service unavailable and leaves the desktop on its visible standard-access fallback.

Upgrade denial occurs before the elevated installer runs and therefore leaves the prior installed desktop/service untouched. A fresh per-machine install denied at UAC installs neither component; proving a fresh denied install with a per-user desktop would require a different installer product decision.

## Remaining #69 work

- Prove installed happy-path, denied-install/upgrade, missing-service, stopped-service, unauthorized-client, and incompatible-version behavior on Windows.
- Capture fresh native Tauri evidence for the access and diagnostic states.

#70 still owns installed crash/restart proof, reboot/upgrade behavior, multi-user lifecycle proof, and legacy-helper removal.
