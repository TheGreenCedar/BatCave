# Rust Runtime And Telemetry Status

**Updated**: 2026-04-28

## Current Architecture

```text
Svelte cockpit
  -> Tauri commands
    -> Rust RuntimeState
      -> Rust RuntimeStore: settings, warm cache, diagnostics, health, query shaping, byte-rate derivation
        -> Rust collectors and helper modes
          -> Win32 process telemetry
          -> Win32 system telemetry
          -> PDH disk-rate telemetry
          -> ETW per-process network attribution
          -> local elevated-helper snapshot mode
          -> Rust benchmark CLI
```

## Implemented

- Rust contracts are the production snake_case JSON surface.
- Runtime settings, warm cache, warnings, diagnostics, health budgets, query shaping, pause/resume, refresh, and admin-mode state live in the Rust store.
- Native process telemetry reports parent PID, start-time identity, working/private memory, process I/O totals, thread count, handle count, and access state.
- Native system telemetry reports aggregate CPU deltas, kernel CPU, logical CPU enrichment, physical memory, pagefile totals, interface-level network totals/rates, and PDH physical-disk rates.
- Per-process network attribution is collected through a Rust ETW consumer over the Windows kernel TCP/IP provider. If the kernel logger is unavailable or access is denied, process network quality is reported as `unavailable` with the ETW failure reason instead of fabricating rates.
- Elevated-helper snapshots use the same Rust telemetry collector, so admin mode can carry ETW-attributed process rows when standard access lacks kernel trace rights.
- Missing or delayed metrics use explicit quality metadata such as `native`, `held`, `partial`, or `unavailable`.
- Benchmark and elevated-helper command modes are Rust CLI surfaces on the Tauri binary.

## Remaining Product Enhancements

- Add signed installer and update-channel work before external distribution.
- Expand screenshot-based validation when changing visible cockpit layout or metric-quality messaging.

## Validation

- `cargo test --manifest-path src/BatCave.App/src-tauri/Cargo.toml`
- `powershell -NoProfile -ExecutionPolicy Bypass -File scripts/validate-tauri.ps1`
- `powershell -NoProfile -ExecutionPolicy Bypass -File scripts/run-benchmark.ps1 -BenchmarkHost core -Platform x64 -Ticks 120 -SleepMs 1000`
