# Platform capabilities

**Updated**: 2026-07-13

This is the static support contract for BatCave Monitor 0.2. Runtime protocol observations remain the authority for what one sample actually contains: a supported collector can still report `held`, `partial`, or `unavailable` when its source is starting, denied, missing, or malformed.

## Telemetry sources and scope

| Surface | Windows | Linux | macOS |
| --- | --- | --- | --- |
| System CPU | Win32 system counters; native aggregate and kernel CPU | `/proc/stat`; native aggregate, kernel, and logical CPU deltas | sysinfo host CPU; estimated aggregate/logical CPU, kernel CPU unavailable |
| System memory | Win32 physical memory, commit, cache, and kernel pool counters | `/proc/meminfo`; native memory and swap | sysinfo host memory and swap; memory native, swap estimated |
| Host disk | PDH physical-disk totals/rates | Deduplicated `/sys/class/block` physical-device totals/rates | Deduplicated `IOBlockStorageDriver` byte counters; disk-image paths are excluded |
| Host network | Non-loopback interface aggregate | Non-loopback `/proc/net/dev` interface aggregate | sysinfo all-interface aggregate, including `lo0` |
| Process identity and resources | Win32 process APIs | `/proc/<pid>` | sysinfo row enriched by libproc |
| Process read/write I/O | Win32 cumulative transfer counters | `/proc/<pid>/io` cumulative counters | libproc `proc_pid_rusage` cumulative counters |
| Process network | ETW IP socket payload attribution | Optional bpftrace/eBPF IP socket payload attribution | Unavailable |
| Protected collection | Current elevated token or local elevated helper | No helper; normal host permissions apply | No helper; normal host permissions apply |

The macOS host-disk number is a physical block-driver aggregate, not a sum of mounted APFS volumes or visible processes. Registry entry IDs deduplicate the source. Attaching a DMG may add an `IOBlockStorageDriver`, but its `IOHDIXController`/DiskImages registry path is excluded. If any eligible physical driver lacks a complete byte-counter pair, the whole host-disk metric fails closed to `unavailable`; it is not published as a partial host total. Device-set changes require a fresh baseline before rates resume.

The macOS network source includes loopback because that is the scope exposed by the sysinfo source used here. Protocol v3 publishes `all_interface_aggregate` for those observations. Windows and Linux host-network observations publish `non_loopback_interface_aggregate`. Process-attributed observations, where supported, publish `ip_socket_payload`.

## Process failure semantics

macOS libproc probes classify each field independently:

| Probe outcome | Row behavior | Metric behavior |
| --- | --- | --- |
| Process exited (`ESRCH`) | Drop the stale row as ordinary churn | No denied count and no rate baseline |
| Access denied (`EPERM`/`EACCES`) | Keep the sysinfo row; `denied` only when every native probe is denied | Affected counters are unavailable with `access_denied` |
| Unsupported (`ENOSYS`/`ENOTSUP`) | Keep the row as partial | Affected field is explicitly unavailable with `unsupported_metric` |
| Other native failure | Keep the row as partial | Affected field is unavailable with `collector_failure` |
| Mixed success/failure | Keep independently successful fields | Row is partial; one failed probe cannot erase another probe's truth |

Process rates use PID plus start time and require compatible live cumulative-counter quality and source. A reused PID, denial, exit, counter reset, source change, or recovery from unavailable data establishes a new baseline and publishes zero rate until the next compatible sample. This prevents churn and permission changes from becoming synthetic I/O spikes.

## Distribution and CPU architecture

| Platform | x86_64 | ARM64 | Published package |
| --- | --- | --- | --- |
| Windows | Supported and validated | **Unsupported**: no CI, native collector proof, installer, or release artifact | x86_64 NSIS installer and executables |
| Linux | Supported and validated | **Unsupported**: no CI, native collector proof, `.deb`, AppImage, or release artifact | x86_64 `.deb` and AppImage |
| macOS 12+ | Supported and validated | Supported and validated | One universal `x86_64` + `arm64` app, DMG, CLI, and updater archive |

Benchmark scripts accepting an architecture label do not create platform support. Windows ARM64 and Linux ARM64 remain unsupported until their native collectors, validation jobs, packages, and release artifacts are all exercised. macOS is different: validation rejects warnings for both Apple targets, and bundle verification requires both Mach-O slices.

Ad-hoc artifacts from `main` are internal validation builds. Versioned macOS releases additionally require Developer ID signing, notarization, and stapling. See [Release channels and verification](releases.md) for promotion details and [Runtime telemetry](runtime-telemetry.md) for the live quality contract.
