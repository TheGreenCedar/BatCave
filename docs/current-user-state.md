# Current-user state ownership and retention

BatCave Monitor keeps its desktop runtime state on the local machine under one directory owned by the current user. This contract defines the files managed by the current-user persistence coordinator, their safety boundaries, and what cleanup operations may remove.

Windows collector-service state is a separate trust boundary. Service security identifiers (SIDs), discretionary access control lists (DACLs), `ProgramData` paths, and service uninstall behavior belong to [issue #69](https://github.com/TheGreenCedar/BatCave/issues/69) and are not inferred here.

## Filesystem roots

| Platform | Current-user root                                                                                            | Required boundary                                                                                                                                                                                                                                                                      |
| -------- | ------------------------------------------------------------------------------------------------------------ | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Windows  | `%LOCALAPPDATA%\BatCaveMonitor`                                                                              | `LOCALAPPDATA` must be an absolute drive-qualified local path without `.` or `..` traversal. Reads and writes fail closed when the current-user ownership or permission boundary cannot be verified. The coordinator does not fall back to `ProgramData` or a service-owned directory. |
| Linux    | `$XDG_DATA_HOME/BatCaveMonitor`, when `XDG_DATA_HOME` is absolute; otherwise `~/.local/share/BatCaveMonitor` | The directory must be a real directory owned by the current effective user. BatCave creates or resets its mode to `0700` and rejects symlink roots.                                                                                                                                    |
| macOS    | `~/Library/Application Support/BatCaveMonitor`                                                               | The directory must be a real directory owned by the current effective user. BatCave creates or resets its mode to `0700` and rejects symlink roots.                                                                                                                                    |

An invalid, relative, unavailable, or unverified root is a persistence failure. Monitoring must remain available with degraded persistence rather than switching to a broader or shared location.

## Owned entries

| Entry                              | Format and purpose                                                                                                                                  | Lifecycle                                                                                                                                                                                                                                                                   |
| ---------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `settings.json`                    | JSON settings owned by the desktop runtime, including query, cadence, pause, and durable UI preferences. Privilege activation remains session-only. | Loaded at startup and written through atomic replacement. A successful supported migration replaces the legacy payload. A corrupt, unknown, or failed-migration payload is preserved during ordinary startup and shutdown; an explicit user settings change may replace it. |
| `warm-cache.json`                  | JSON process rows used only to make a standard-access warm start less empty. It is a cache, not authoritative telemetry.                            | Missing or unreadable cache data does not prevent startup. The runtime may replace it after a successful standard-access sample and removes or suppresses it when privileged state makes reuse unsafe.                                                                      |
| `diagnostics.jsonl`                | Newline-delimited local diagnostic events.                                                                                                          | Appended and synchronized locally. It rotates before an append would exceed the file budget.                                                                                                                                                                                |
| `diagnostics.jsonl.1`              | The one retained rotated diagnostic file.                                                                                                           | Rotation replaces the older `.1` file; there is no `.2` generation.                                                                                                                                                                                                         |
| `<component>.<pid>.<sequence>.tmp` | Same-directory temporary file used while atomically replacing a JSON component.                                                                     | Removed after a handled write or replacement failure. A process or machine crash can leave a stale temporary file; it remains BatCave-owned cleanup residue and is safe to remove while the app is stopped.                                                                 |

On Unix, the expected component-file mode is `0600`: new component and atomic temporary files request that mode, and diagnostic appends reset their file to it. An existing component must be a regular file owned by the current user with no group or other access; BatCave rejects an unsafe file instead of following or overwriting it. Windows component reparse points and Unix component symlinks are rejected.

WebView local storage is outside this filesystem contract. The UI can temporarily retain theme or history preferences there while migrating them to `settings.json`; deleting the `BatCaveMonitor` directory does not prove those browser-managed values are gone. Transient elevated-helper pipes, tokens, and per-run artifacts are runtime IPC rather than coordinator-owned durable state.

## Writes, migration, and fallback

Settings and warm-cache writes use a private sibling temporary file, synchronize it, replace the destination atomically, and synchronize the parent directory on Unix. A failure before replacement preserves the previous destination. If replacement succeeds but the final directory synchronization fails, BatCave reports that the new value may be installed but its durability is uncertain.

A missing JSON component loads as no saved value. Malformed JSON and unsupported migrations return a typed failure without renaming, deleting, or rewriting the source bytes. The runtime starts with safe defaults and reports degraded persistence. After a failed settings load, automatic shutdown persistence stays blocked so defaults cannot erase the original file; an explicit user mutation is the recovery boundary that may replace it.

Warm-cache failure has a narrower consequence: the runtime starts without cached rows, reports the persistence failure, and may write a new cache after later standard-access collection. No saved file can make a live sample authoritative or bypass metric-quality checks.

## Diagnostic bounds and breaker

The production diagnostic policy is concrete:

- one current `diagnostics.jsonl` file, limited to 1 MiB;
- one `diagnostics.jsonl.1` backup, replaced at the next rotation;
- one event limited to 64 KiB, including its trailing newline;
- rotation occurs before an append that would cross the current-file limit.

The first create, permission, load, write, synchronization, replacement, or rotation failure is retained as the active diagnostic persistence failure. Later diagnostic events are counted as suppressed without another filesystem write, so a diagnostic write failure cannot recursively generate diagnostics. An explicit retry clears the breaker; ordinary serialization rejection does not open it.

## Upgrade, uninstall, and manual cleanup

Current packaging defines no hook that deletes the current-user root. The retention policy is therefore:

| Action       | Current-user state policy                                                                                                                                                                                                                                                                                      |
| ------------ | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Upgrade      | Preserve settings, warm cache, diagnostics, and any owned recovery residue. A supported settings migration runs after the upgraded app starts.                                                                                                                                                                 |
| Uninstall    | Preserve the current-user root. The repository does not claim that NSIS, Debian, AppImage, or macOS app removal deletes these files. A later reinstall can encounter the retained state.                                                                                                                       |
| Manual reset | Stop BatCave, then remove only the platform-specific `BatCaveMonitor` directory above. Do not remove its parent (`LOCALAPPDATA`, `XDG_DATA_HOME`, `.local/share`, or `Application Support`). The next start recreates filesystem state from defaults, subject to any separate WebView local-storage migration. |

These files are BatCave-owned even though the uninstall policy preserves them for the user. Future automated cleanup must stay within the exact current-user root and must not infer authority over service-owned Windows state.

## Windows service boundary

This contract grants no ownership of a Windows collector service directory and makes no claim about a service SID, DACL, ancestor reparse-point policy, `ProgramData` layout, service diagnostics, upgrade, rollback, or uninstall cleanup. [Issue #69](https://github.com/TheGreenCedar/BatCave/issues/69) must define and prove that boundary before service-owned files are documented or removed.

The implementation and focused contract tests live in [`persistence.rs`](../src/BatCave.App/src-tauri/src/persistence.rs); atomic replacement lives in [`atomic_json.rs`](../src/BatCave.App/src-tauri/src/atomic_json.rs).
