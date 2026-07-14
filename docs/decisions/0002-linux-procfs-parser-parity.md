# Linux procfs parser parity

- Status: accepted for issue #71
- Date: 2026-07-13
- Scope: Linux process collection only

## Decision

Retain BatCave's manual `/proc` readers for issue #71. Do not add the `procfs` crate or replace a parser in this change.

The bounded comparison evaluated `procfs` 0.18.0 against every manual process surface BatCave currently publishes. The crate has API coverage for the raw inputs, but source-level coverage is not yet behavioral parity for BatCave's access-quality, process-identity, and failure contracts. The retained parsers now reject malformed required counters instead of converting them to zero and have hostile fixtures for names, field boundaries, numeric overflow, missing and duplicate I/O counters, units, and partial CPU totals.

## Comparison

| BatCave input | `procfs` 0.18.0 surface | Static parity | Remaining proof before replacement |
| --- | --- | --- | --- |
| `/proc/<pid>/stat` | `Process::stat()` / `Stat` | Required PID, name, state, parent, CPU, thread, start-time, virtual-memory, and RSS fields exist | Compare rows for short-lived processes and names containing spaces and parentheses; prove no PID-reuse mixing |
| `/proc/<pid>/status` `RssAnon` | `Process::status()` / `Status` | Anonymous RSS is represented | Preserve BatCave's explicit RSS estimate when the field is absent, malformed, or denied |
| `/proc/<pid>/io` | `Process::io()` / `Io` | `read_bytes` and `write_bytes` exist | Preserve per-field access failure and unavailable quality rather than zero |
| `/proc/<pid>/exe` | `Process::exe()` | Equivalent symlink surface | Compare permission-denied and deleted-executable behavior |
| `/proc/<pid>/fd` | `Process::fd_count()` | Equivalent count; the crate also uses the Linux 6.2 directory-size fast path | Prove zero descriptors remains a valid measured zero and denial remains partial access |
| clock ticks, page size, boot time | `ticks_per_second()`, `page_size()`, `boot_time_secs()` | Equivalent system inputs | Compare start-time rounding and fallback behavior on the oldest supported distribution |
| `/proc/stat` CPU total and logical CPUs | `KernelStats` plus CPU-time APIs | Data exists, but its aggregation does not directly match BatCave's current one-core-equivalent delta calculation | Compare deterministic deltas and hot-plug behavior before changing the collector |

`cargo info procfs@0.18.0 --verbose` shows that default features add `chrono` and `flate2`; a fair minimal candidate would disable defaults but still add `procfs-core`, `rustix`, and `bitflags` to the production graph. Dependency size is secondary to semantic parity, but there is no reason to pay it before the runtime comparison exists.

## Replacement gate

A future parser replacement must be a separate, reviewable change and must first run both implementations on native Linux against the same bounded fixture set:

1. the current user, a short-lived child, a process with spaces and parentheses in its name, and a process whose optional files are inaccessible;
2. exact `(pid, start_time)` identity and row-set comparison;
3. CPU, memory, I/O, thread, descriptor, executable, and access-quality comparison over at least two samples;
4. malformed fixture parity for required fields and units;
5. strict validation and benchmark evidence showing no hot-path regression.

Until that evidence exists, the manual parser remains the reference implementation. This decision does not claim native runtime parity from the macOS implementation host; native Linux package and permission-path proof remains an integration gate.

## Reproduction

From the repository root:

```bash
cargo info procfs@0.18.0 --verbose
cargo test --manifest-path src/BatCave.App/src-tauri/Cargo.toml linux_process::tests
```

The crate comparison used its published 0.18.0 source for `Process::stat`, `status`, `io`, `exe`, and `fd_count`, plus its tick, page-size, and boot-time helpers. No crate code or generated artifact is vendored into BatCave.
