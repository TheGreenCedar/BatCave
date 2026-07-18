# Platform capabilities

**Updated**: 2026-07-14

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
| Process network | ETW IP socket payload attribution | Optional bpftrace/eBPF IP socket payload attribution | NStat IP socket payload attribution |
| Protected collection | Current elevated token or installed collector service | Normal host permissions apply | Normal host permissions apply |

The macOS host-disk number is a physical block-driver aggregate, not a sum of mounted APFS volumes or visible processes. Registry entry IDs deduplicate the source. Attaching a DMG may add an `IOBlockStorageDriver`, but its `IOHDIXController`/DiskImages registry path is excluded. If any eligible physical driver lacks a complete byte-counter pair, the whole host-disk metric fails closed to `unavailable`; it is not published as a partial host total. Device-set changes require a fresh baseline before rates resume.

The macOS host-network source includes loopback because that is the scope exposed by sysinfo. Protocol v3 publishes `all_interface_aggregate` for those observations. Windows and Linux host-network observations publish `non_loopback_interface_aggregate`. macOS process rates come from XNU NStat TCP, UDP, and QUIC counters and publish `ip_socket_payload`.

NStat is a private XNU wire interface, not a private-framework dependency. BatCave qualifies the revision-9 layout on Darwin 21 through 25, opens one unprivileged nonblocking control socket, and fails closed on an unqualified OS layout, rejected provider, malformed message, truncation, or counter regression. The first complete query establishes a baseline; no historical bytes are emitted as a live rate.

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

The [versioned machine contract](evidence/releases/platform-support-contract.v1.json) is authoritative for release hosts, architectures, package kinds, and proof states. This is its canonical human-readable matrix.

| Profile | Minimum host | Host architecture/runtime | Contract release packages | Source proof | Oldest-host native proof |
| --- | --- | --- | --- | --- | --- |
| `windows-client-10-x86_64` | Windows 10 client `10.0.16299`+ | `x86_64` | NSIS | `source_enforced` | `pending` |
| `ubuntu-22.04-x86_64-glibc` | Ubuntu `22.04`+ | `x86_64`, glibc | deb, AppImage | `source_enforced` | `pending` |
| `debian-12-x86_64-glibc` | Debian `12`+ | `x86_64`, glibc | deb, AppImage | `source_enforced` | `pending` |
| `macos-12-arm64` | macOS `12.0`+ | Apple Silicon `arm64` | DMG, updater archive | `source_enforced` | `pending` |

`source_enforced` means repository, build, configuration, metadata, and extraction-only package checks agree with the contract. It is not native install or runtime proof. Linux builders are pinned to `ubuntu-22.04`; package inspection requires x86-64 ELF payloads, no required symbol newer than `GLIBC_2.35`, and deb dependencies on `libgtk-3-0` and `libwebkit2gtk-4.1-0`.

Windows Server, Windows ARM64, Linux ARM64, macOS Intel `x86_64`, musl, unlisted Linux distributions, and unlisted package/host combinations are explicit non-claims. Benchmark scripts accepting an architecture label do not create platform support. macOS validation rejects warnings for the `aarch64-apple-darwin` target, and bundle verification requires an `arm64` Mach-O slice without an Intel slice, but oldest-supported macOS native proof remains pending.

Ad-hoc artifacts from `main` are internal validation builds. Versioned macOS releases additionally require Developer ID signing, notarization, and stapling. See [Release channels and verification](releases.md) for promotion details and [Runtime telemetry](runtime-telemetry.md) for the live quality contract.
